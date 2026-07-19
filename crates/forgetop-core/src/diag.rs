//! Lightweight, bounded file logging for diagnostics. Writes are best-effort: diagnostics
//! must never turn a recoverable application error into a crash.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, TimeDelta, Utc};
use fs2::FileExt;
use regex::Regex;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// Retain at most 3 MiB across the active file and five rotated segments.
pub const MAX_LOG_BYTES: u64 = 3 * 1024 * 1024;
const SEGMENT_BYTES: u64 = 512 * 1024;
const MAX_LOG_AGE_HOURS: i64 = 24;

static LOG_LOCK: Mutex<()> = Mutex::new(());

/// `$XDG_CONFIG_HOME/forgetop/forgetop.log` (next to `config.json`), or `./forgetop.log`.
pub fn log_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("forgetop").join("forgetop.log")
}

/// Append a timestamped `context: message` line to the log file. Best-effort.
pub fn log(context: &str, message: &str) {
    if let Ok(_guard) = LOG_LOCK.try_lock() {
        let _ = LogStore::new(log_path()).append_at(Utc::now(), context, message);
    }
}

/// Physically prune expired diagnostics and repair their size and permissions. Best-effort.
///
/// Call this once during startup and periodically in long-running processes. If this process or
/// another forgetop process is already touching the logs, maintenance is skipped rather than
/// delaying the UI.
pub fn maintain() {
    if let Ok(_guard) = LOG_LOCK.try_lock() {
        let _ = LogStore::new(log_path()).maintain_at(Utc::now());
    }
}

/// An owned, immutable diagnostic attachment assembled from the retained segments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticSnapshot {
    pub bytes: Vec<u8>,
    pub size_bytes: u64,
    pub oldest_at: Option<DateTime<Utc>>,
    pub newest_at: Option<DateTime<Utc>>,
}

impl DiagnosticSnapshot {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

/// Read the retained log as a chronologically ordered, re-sanitized snapshot.
pub fn snapshot() -> io::Result<DiagnosticSnapshot> {
    let _guard = LOG_LOCK
        .lock()
        .map_err(|_| io::Error::other("diagnostic log lock poisoned"))?;
    LogStore::new(log_path()).snapshot_at(Utc::now())
}

#[derive(Clone, Debug)]
struct LogStore {
    path: PathBuf,
    max_bytes: u64,
    segment_bytes: u64,
    max_age: TimeDelta,
}

impl LogStore {
    fn new(path: PathBuf) -> Self {
        Self::with_limits(
            path,
            MAX_LOG_BYTES,
            SEGMENT_BYTES,
            TimeDelta::hours(MAX_LOG_AGE_HOURS),
        )
    }

    fn with_limits(path: PathBuf, max_bytes: u64, segment_bytes: u64, max_age: TimeDelta) -> Self {
        debug_assert!(max_bytes > 0);
        debug_assert!(segment_bytes > 0);
        Self {
            path,
            max_bytes,
            segment_bytes: segment_bytes.min(max_bytes),
            max_age,
        }
    }

    fn segment_count(&self) -> usize {
        (self.max_bytes / self.segment_bytes).max(1) as usize
    }

    fn segment_path(&self, index: usize) -> PathBuf {
        if index == 0 {
            self.path.clone()
        } else {
            suffixed_path(&self.path, &format!(".{index}"))
        }
    }

    fn lock_path(&self) -> PathBuf {
        suffixed_path(&self.path, ".lock")
    }

    fn segment_paths(&self) -> Vec<PathBuf> {
        (0..self.segment_count())
            .map(|index| self.segment_path(index))
            .collect()
    }

    fn append_at(&self, now: DateTime<Utc>, context: &str, message: &str) -> io::Result<()> {
        self.with_exclusive_lock(|store| store.append_unlocked(now, context, message))?;
        Ok(())
    }

    fn append_unlocked(&self, now: DateTime<Utc>, context: &str, message: &str) -> io::Result<()> {
        self.prune_expired(now)?;

        let line = bounded_entry(now, context, message, self.segment_bytes as usize);
        let current_len = fs::metadata(&self.path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if current_len > 0 && current_len.saturating_add(line.len() as u64) > self.segment_bytes {
            self.rotate()?;
        }

        let mut file = open_private(&self.path, true, false)?;
        file.write_all(line.as_bytes())?;
        self.enforce_total_bound()
    }

    fn maintain_at(&self, now: DateTime<Utc>) -> io::Result<()> {
        self.with_exclusive_lock(|store| {
            store.prune_expired(now)?;
            store.enforce_total_bound()
        })?;
        Ok(())
    }

    fn with_exclusive_lock<T>(
        &self,
        operation: impl FnOnce(&Self) -> io::Result<T>,
    ) -> io::Result<Option<T>> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let lock = open_private(&self.lock_path(), false, false)?;
        match lock.try_lock_exclusive() {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        }
        self.repair_segment_permissions()?;
        operation(self).map(Some)
    }

    fn repair_segment_permissions(&self) -> io::Result<()> {
        for path in self.segment_paths() {
            match repair_private_permissions(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn prune_expired(&self, now: DateTime<Utc>) -> io::Result<()> {
        let cutoff = now - self.max_age;
        for path in self.segment_paths() {
            let contents = match fs::read_to_string(&path) {
                Ok(contents) => contents,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    fs::remove_file(path)?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let retained: Vec<String> = contents
                .lines()
                .filter(|line| parse_timestamp(line).is_some_and(|timestamp| timestamp >= cutoff))
                .map(sanitize)
                .collect();
            let retained = newest_lines_within(retained, self.segment_bytes as usize);
            if retained.is_empty() {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            } else {
                write_private(&path, &join_lines(&retained))?;
            }
        }
        Ok(())
    }

    fn rotate(&self) -> io::Result<()> {
        let count = self.segment_count();
        if count == 1 {
            return write_private(&self.path, &[]);
        }

        let oldest = self.segment_path(count - 1);
        match fs::remove_file(oldest) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        for index in (1..count).rev() {
            let from = self.segment_path(index - 1);
            let to = self.segment_path(index);
            if from.exists() {
                fs::rename(from, to)?;
            }
        }
        Ok(())
    }

    fn enforce_total_bound(&self) -> io::Result<()> {
        let paths = self.segment_paths();
        let mut total: u64 = paths
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum();
        for path in paths.into_iter().rev() {
            if total <= self.max_bytes {
                break;
            }
            let size = match fs::metadata(&path) {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let contents = match fs::read_to_string(&path) {
                Ok(contents) => contents,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => {
                    fs::remove_file(&path)?;
                    total = total.saturating_sub(size);
                    continue;
                }
            };
            let allowed = size.saturating_sub(total - self.max_bytes) as usize;
            let lines = newest_lines_within(contents.lines().map(str::to_owned).collect(), allowed);
            if lines.is_empty() {
                fs::remove_file(path)?;
                total = total.saturating_sub(size);
            } else {
                let retained = join_lines(&lines);
                write_private(&path, &retained)?;
                total = total
                    .saturating_sub(size)
                    .saturating_add(retained.len() as u64);
            }
        }
        Ok(())
    }

    fn snapshot_at(&self, now: DateTime<Utc>) -> io::Result<DiagnosticSnapshot> {
        self.with_exclusive_lock(|store| store.snapshot_unlocked(now))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "diagnostic logs are busy"))
    }

    fn snapshot_unlocked(&self, now: DateTime<Utc>) -> io::Result<DiagnosticSnapshot> {
        let cutoff = now - self.max_age;
        let mut lines = Vec::new();
        for index in (0..self.segment_count()).rev() {
            let path = self.segment_path(index);
            let Ok(contents) = fs::read_to_string(path) else {
                continue;
            };
            lines.extend(
                contents
                    .lines()
                    .filter(|line| {
                        parse_timestamp(line).is_some_and(|timestamp| timestamp >= cutoff)
                    })
                    .map(sanitize),
            );
        }
        let lines = newest_lines_within(lines, self.max_bytes as usize);
        let oldest_at = lines.first().and_then(|line| parse_timestamp(line));
        let newest_at = lines.last().and_then(|line| parse_timestamp(line));
        let bytes = join_lines(&lines);
        Ok(DiagnosticSnapshot {
            size_bytes: bytes.len() as u64,
            bytes,
            oldest_at,
            newest_at,
        })
    }
}

fn suffixed_path(path: &std::path::Path, suffix: &str) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn open_private(path: &std::path::Path, append: bool, truncate: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(!append)
        .write(true)
        .create(true)
        .append(append)
        .truncate(truncate);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path)?;
    repair_private_permissions(path)?;
    Ok(file)
}

fn write_private(path: &std::path::Path, contents: &[u8]) -> io::Result<()> {
    let mut file = open_private(path, false, true)?;
    file.write_all(contents)
}

#[cfg(unix)]
fn repair_private_permissions(path: &std::path::Path) -> io::Result<()> {
    let permissions = fs::metadata(path)?.permissions();
    if permissions.mode() & 0o777 != 0o600 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn repair_private_permissions(path: &std::path::Path) -> io::Result<()> {
    fs::metadata(path).map(|_| ())
}

fn bounded_entry(now: DateTime<Utc>, context: &str, message: &str, max_bytes: usize) -> String {
    let context = sanitize(single_line(context));
    let message = sanitize(single_line(message));
    let prefix = format!("{} [{}] ", now.to_rfc3339(), context);
    let available = max_bytes.saturating_sub(prefix.len() + 1);
    let message = truncate_utf8(&message, available);
    format!("{prefix}{message}\n")
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn parse_timestamp(line: &str) -> Option<DateTime<Utc>> {
    let timestamp = line.split_once(' ')?.0;
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|value| value.to_utc())
}

fn newest_lines_within(lines: Vec<String>, max_bytes: usize) -> Vec<String> {
    let mut used = 0;
    let mut retained = Vec::new();
    for line in lines.into_iter().rev() {
        let line_bytes = line.len() + 1;
        if line_bytes > max_bytes.saturating_sub(used) {
            continue;
        }
        used += line_bytes;
        retained.push(line);
    }
    retained.reverse();
    retained
}

fn join_lines(lines: &[String]) -> Vec<u8> {
    if lines.is_empty() {
        return Vec::new();
    }
    let mut joined = lines.join("\n").into_bytes();
    joined.push(b'\n');
    joined
}

fn sanitize(value: impl AsRef<str>) -> String {
    redaction_rules().iter().fold(
        value.as_ref().to_owned(),
        |sanitized, (regex, replacement)| regex.replace_all(&sanitized, *replacement).into_owned(),
    )
}

fn redaction_rules() -> &'static [(Regex, &'static str)] {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            (
                r#"(?i)\b(authorization|proxy-authorization|private[-_]?token|x[-_]?(?:api[-_]?key|auth[-_]?token|forgetop[-_]?token)|api[-_]?key|apikey|access[-_]?token|refresh[-_]?token|client[-_]?secret|app[-_]?password|password|passwd|secret|token|pat)(\s*[:=]\s*)(?:(?:Bearer|Basic)\s+[^\s,;&]+|"[^"]*"|'[^']*'|[^\s,;&]+)"#,
                "$1$2[REDACTED]",
            ),
            (r"(?i)\b(FORGETOP_PAT_[A-Z0-9_]+)(\s*=\s*)[^\s,;&]+", "$1$2[REDACTED]"),
            (r"(?i)(://)[^\s/@]+@", "$1[REDACTED]@"),
            (r"(?i)([?&#]t=)[^&#\s]+", "$1[REDACTED]"),
            (r"(?i)\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+", "$1 [REDACTED]"),
            (
                r"(?i)\b(?:github_pat_|gh[pousr]_|glpat-|lin_api_)[A-Za-z0-9_-]+",
                "[REDACTED]",
            ),
        ]
        .into_iter()
        .map(|(pattern, replacement)| (Regex::new(pattern).expect("valid diagnostic redaction regex"), replacement))
        .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeDelta};
    use tempfile::tempdir;

    #[cfg(unix)]
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn log_path_is_the_forgetop_log_file() {
        let p = log_path();
        assert_eq!(p.file_name().unwrap(), "forgetop.log");
        assert!(p.to_string_lossy().contains("forgetop"));
    }

    #[test]
    fn append_redacts_credentials_and_keeps_one_entry_per_line() {
        let dir = tempdir().unwrap();
        let store = LogStore::with_limits(
            dir.path().join("forgetop.log"),
            1024,
            512,
            TimeDelta::hours(24),
        );
        let now = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .unwrap()
            .to_utc();

        store
            .append_at(
                now,
                "fetch token=context-secret",
                "Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz\n\
                 Proxy-Authorization: Basic ZGFuaWVsOnBhc3N3b3Jk\n\
                 url=https://daniel:password@example.com/api?t=session-secret&access_token=glpat-secret-value",
            )
            .unwrap();

        let snapshot = store.snapshot_at(now).unwrap();
        let text = snapshot.text();
        assert!(!text.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
        assert!(!text.contains("ZGFuaWVsOnBhc3N3b3Jk"));
        assert!(!text.contains("context-secret"));
        assert!(!text.contains("password"));
        assert!(!text.contains("session-secret"));
        assert!(!text.contains("glpat-secret-value"));
        assert!(text.contains("[REDACTED]"));
        assert_eq!(text.lines().count(), 1);
    }

    #[test]
    fn append_rotates_segments_and_never_exceeds_the_size_limit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("forgetop.log");
        let store = LogStore::with_limits(path.clone(), 360, 120, TimeDelta::hours(24));
        let now = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .unwrap()
            .to_utc();

        for index in 0..20 {
            store
                .append_at(
                    now + TimeDelta::seconds(index),
                    "fetch",
                    &format!("entry-{index:02} {}", "x".repeat(28)),
                )
                .unwrap();
        }

        let total: u64 = store
            .segment_paths()
            .iter()
            .filter_map(|p| fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        assert!(total <= 360, "stored {total} bytes");

        let text = store
            .snapshot_at(now + TimeDelta::seconds(20))
            .unwrap()
            .text();
        assert!(text.contains("entry-19"));
        assert!(!text.contains("entry-00"));
        assert!(store.segment_paths().iter().filter(|p| p.exists()).count() > 1);
    }

    #[test]
    fn append_prunes_entries_older_than_the_retention_window() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("forgetop.log");
        let store = LogStore::with_limits(path.clone(), 1024, 512, TimeDelta::hours(24));
        let now = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .unwrap()
            .to_utc();

        store
            .append_at(now - TimeDelta::hours(25), "fetch", "expired")
            .unwrap();
        store.append_at(now, "fetch", "retained").unwrap();

        let snapshot = store.snapshot_at(now).unwrap();
        assert!(!snapshot.text().contains("expired"));
        assert!(snapshot.text().contains("retained"));
        assert!(!fs::read_to_string(path).unwrap().contains("expired"));
    }

    #[test]
    fn maintenance_physically_prunes_expired_entries_without_an_append() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("forgetop.log");
        let store = LogStore::with_limits(path.clone(), 1024, 512, TimeDelta::hours(24));
        let now = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .unwrap()
            .to_utc();
        fs::write(
            &path,
            format!(
                "{} [fetch] expired\n{} [fetch] retained\n",
                (now - TimeDelta::hours(25)).to_rfc3339(),
                now.to_rfc3339()
            ),
        )
        .unwrap();

        store.maintain_at(now).unwrap();

        let contents = fs::read_to_string(path).unwrap();
        assert!(!contents.contains("expired"));
        assert!(contents.contains("retained"));
    }

    #[test]
    fn contended_interprocess_lock_drops_append_without_mutating_logs() {
        use fs2::FileExt;

        let dir = tempdir().unwrap();
        let path = dir.path().join("forgetop.log");
        let store = LogStore::with_limits(path.clone(), 1024, 512, TimeDelta::hours(24));
        let now = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .unwrap()
            .to_utc();
        store.append_at(now, "fetch", "before-contention").unwrap();

        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(store.lock_path())
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        store
            .append_at(now + TimeDelta::seconds(1), "fetch", "must-be-dropped")
            .unwrap();
        store.maintain_at(now + TimeDelta::hours(25)).unwrap();
        FileExt::unlock(&lock).unwrap();

        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains("before-contention"));
        assert!(!contents.contains("must-be-dropped"));
    }

    #[cfg(unix)]
    #[test]
    fn segment_paths_preserve_non_utf8_log_paths() {
        let dir = tempdir().unwrap();
        let mut name = b"forgetop-".to_vec();
        name.push(0xff);
        name.extend_from_slice(b".log");
        let path = dir.path().join(std::ffi::OsString::from_vec(name));
        let store = LogStore::with_limits(path.clone(), 1024, 512, TimeDelta::hours(24));

        let mut expected = path.as_os_str().as_bytes().to_vec();
        expected.extend_from_slice(b".1");
        assert_eq!(store.segment_path(1).as_os_str().as_bytes(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn diagnostics_files_are_created_and_repaired_as_owner_only() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("forgetop.log");
        let store = LogStore::with_limits(path, 360, 120, TimeDelta::hours(24));
        let now = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .unwrap()
            .to_utc();
        for index in 0..4 {
            store
                .append_at(
                    now + TimeDelta::seconds(index),
                    "fetch",
                    &format!("entry-{index} {}", "x".repeat(36)),
                )
                .unwrap();
        }

        let mut paths: Vec<_> = store
            .segment_paths()
            .into_iter()
            .filter(|path| path.exists())
            .collect();
        paths.push(store.lock_path());
        assert!(paths.len() >= 3, "expected active, rotated, and lock files");
        for path in &paths {
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }

        store.maintain_at(now + TimeDelta::minutes(1)).unwrap();

        for path in paths {
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{} had mode {mode:o}", path.display());
        }
    }

    #[test]
    fn snapshot_is_chronological_and_immutable() {
        let dir = tempdir().unwrap();
        let store = LogStore::with_limits(
            dir.path().join("forgetop.log"),
            360,
            120,
            TimeDelta::hours(24),
        );
        let now = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .unwrap()
            .to_utc();

        for index in 0..6 {
            store
                .append_at(
                    now + TimeDelta::seconds(index),
                    "action",
                    &format!("ordered-{index} {}", "x".repeat(24)),
                )
                .unwrap();
        }
        let before = store.snapshot_at(now + TimeDelta::seconds(6)).unwrap();
        store
            .append_at(now + TimeDelta::seconds(7), "action", "later-entry")
            .unwrap();

        let text = before.text();
        let positions: Vec<_> = (1..6)
            .filter_map(|index| text.find(&format!("ordered-{index}")))
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(!text.contains("later-entry"));
        assert_eq!(before.size_bytes, before.bytes.len() as u64);
        assert!(before.oldest_at <= before.newest_at);
    }

    #[test]
    fn snapshot_sanitizes_legacy_entries_again() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("forgetop.log");
        let store = LogStore::with_limits(path.clone(), 1024, 512, TimeDelta::hours(24));
        let now = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .unwrap()
            .to_utc();
        fs::write(
            path,
            format!(
                "{} [legacy] api_key=should-not-leave-disk\n",
                now.to_rfc3339()
            ),
        )
        .unwrap();

        let text = store.snapshot_at(now).unwrap().text();
        assert!(!text.contains("should-not-leave-disk"));
        assert!(text.contains("api_key=[REDACTED]"));
    }

    #[test]
    fn sanitizes_single_url_userinfo_without_redacting_plain_email_addresses() {
        let sanitized =
            sanitize("url=https://single-credential@example.com/api contact=user@example.com");

        assert_eq!(
            sanitized,
            "url=https://[REDACTED]@example.com/api contact=user@example.com"
        );
    }

    #[test]
    fn failed_io_is_returned_to_the_best_effort_wrapper() {
        let dir = tempdir().unwrap();
        let blocked_parent = dir.path().join("not-a-directory");
        fs::write(&blocked_parent, "file").unwrap();
        let store = LogStore::with_limits(
            blocked_parent.join("forgetop.log"),
            1024,
            512,
            TimeDelta::hours(24),
        );
        let now = DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
            .unwrap()
            .to_utc();

        assert!(store
            .append_at(now, "fetch", "recoverable failure")
            .is_err());
    }
}
