//! Stale-while-revalidate cache for fetched provider data.
//!
//! A JSON file on disk mirrored by an in-memory map. The TUI hydrates it once at startup and
//! paints the last known data immediately, then repaints when the network answers — the cache
//! exists to remove the blank screen you would otherwise stare at while the first fetch runs.
//! Serving stale data is therefore the point, not a compromise.
//!
//! Every failure mode here degrades to a miss or a no-op: a cache that can return an error is a
//! cache that can break the app it was added to speed up.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::diag;

/// Bumped when the on-disk shape changes. A file at any other version is discarded wholesale
/// rather than migrated — re-fetching is cheap, a half-migrated cache is not.
const CACHE_VERSION: u32 = 1;

/// Anything a week old is worth a round trip; keeping it only grows the file we parse at startup.
const MAX_AGE_DAYS: i64 = 7;

/// A hard ceiling so a long-lived install can't turn startup hydration into a visible pause.
const MAX_ENTRIES: usize = 500;

/// A cached value plus when it was fetched.
#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    pub value: T,
    pub fetched_at: DateTime<Utc>,
}

/// What a `put` did. `Stale` is the only outcome that means the caller is holding older data
/// than the store — `Disabled` is not a refusal, it is a cache that simply isn't storing
/// anything, so a caller that repaints from what it just tried to write is still correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePut {
    Stored,
    Stale,
    Disabled,
}

/// An entry as it lives on disk and in the map: already serialized, so one store can hold every
/// domain type without `CacheStore` itself being generic.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    fetched_at: DateTime<Utc>,
    value: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    /// Defaulted so a file written before versioning reads as 0 and is discarded, not rejected.
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: HashMap<String, StoredEntry>,
}

/// Resolves the on-disk cache path: `$XDG_CONFIG_HOME/forgetop/cache.json` (or the platform
/// config dir), alongside `config.json`.
fn default_cache_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("forgetop").join("cache.json")
}

/// Owner-only permissions, mirroring `diag`'s treatment of the log file. A no-op off unix,
/// where the config directory's own ACLs are the protection.
#[cfg(unix)]
async fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await
}

#[cfg(not(unix))]
async fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    tokio::fs::metadata(path).await.map(|_| ())
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    let mut p = path.to_path_buf();
    p.set_extension(ext);
    p
}

/// JSON-file cache with atomic (temp + rename) writes and an in-memory mirror.
pub struct CacheStore {
    /// `None` marks the store disabled: nothing is read and nothing is written.
    path: Option<PathBuf>,
    entries: RwLock<HashMap<String, StoredEntry>>,
    dirty: AtomicBool,
}

impl CacheStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: Some(path.into()), entries: RwLock::new(HashMap::new()), dirty: AtomicBool::new(false) }
    }

    pub fn at_default_path() -> Self {
        Self::new(default_cache_path())
    }

    /// Non-persistent store: `get` always misses, `put` and `flush` are no-ops. Used for `--demo`
    /// and tests — demo data is canned, so caching it would only leak between runs.
    pub fn disabled() -> Self {
        Self { path: None, entries: RwLock::new(HashMap::new()), dirty: AtomicBool::new(false) }
    }

    /// A poisoned lock means some other caller panicked mid-update; for a cache the worst case is
    /// one odd entry, which is not worth taking the process down for.
    fn read_entries(&self) -> RwLockReadGuard<'_, HashMap<String, StoredEntry>> {
        self.entries.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_entries(&self) -> RwLockWriteGuard<'_, HashMap<String, StoredEntry>> {
        self.entries.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Hydrate the in-memory map from disk. Never fails the caller: a missing, unreadable, corrupt
    /// or version-mismatched file leaves the store empty and logs a line.
    pub async fn load(&self) {
        let Some(path) = self.path.as_ref() else { return };

        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(err) => {
                diag::log("cache", &format!("no cache to load from {}: {err}", path.display()));
                return;
            }
        };
        let file: CacheFile = match serde_json::from_slice(&bytes) {
            Ok(file) => file,
            Err(err) => {
                diag::log("cache", &format!("discarding unreadable cache {}: {err}", path.display()));
                return;
            }
        };
        if file.version != CACHE_VERSION {
            diag::log(
                "cache",
                &format!("discarding cache at version {} (want {CACHE_VERSION})", file.version),
            );
            return;
        }

        let entries = prune(file.entries, Utc::now());
        // The guard is taken after all the I/O and never held across an await.
        *self.write_entries() = entries;
    }

    /// Read a cached value. `None` on a miss, and on a deserialize failure — a domain type whose
    /// shape changed since the entry was written must degrade to a miss, not an error.
    ///
    /// There is deliberately **no read-time TTL**: an entry is returned however old it is. Callers
    /// paint stale data on purpose and surface its age in the UI. Expiring on read would hand the
    /// caller a blank screen again, which is the exact problem this cache exists to remove.
    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<CacheEntry<T>> {
        let stored = self.read_entries().get(key).cloned()?;
        match serde_json::from_value(stored.value) {
            Ok(value) => Some(CacheEntry { value, fetched_at: stored.fetched_at }),
            Err(err) => {
                diag::log("cache", &format!("dropping unusable entry {key}: {err}"));
                None
            }
        }
    }

    /// Write through, reporting what happened.
    ///
    /// Refuses ([`CachePut::Stale`], mutating nothing) when an entry already exists whose
    /// `fetched_at` is newer than or equal to this one. Responses do not come back in the order
    /// they were sent: a slow request started before a fast one can land after it, and without
    /// this guard it would quietly overwrite fresh data with stale.
    pub fn put<T: Serialize>(&self, key: &str, value: &T, fetched_at: DateTime<Utc>) -> CachePut {
        if self.path.is_none() {
            return CachePut::Disabled;
        }
        let value = match serde_json::to_value(value) {
            Ok(value) => value,
            Err(err) => {
                diag::log("cache", &format!("not caching {key}: {err}"));
                // Nothing was stored, but nothing newer is being held back either — a value this
                // store cannot serialize is, for this key, a store that isn't storing. Reporting
                // it as `Stale` would tell the caller its data is out of date, which it is not.
                return CachePut::Disabled;
            }
        };

        {
            let mut entries = self.write_entries();
            if entries.get(key).is_some_and(|existing| existing.fetched_at >= fetched_at) {
                return CachePut::Stale;
            }
            entries.insert(key.to_string(), StoredEntry { fetched_at, value });
        }
        self.dirty.store(true, Ordering::Relaxed);
        CachePut::Stored
    }

    /// Persist to disk if dirty, atomically. Errors are logged, never propagated; the dirty flag
    /// survives a failed write so the next flush retries rather than losing the entries.
    pub async fn flush(&self) {
        let Some(path) = self.path.as_ref() else { return };
        if !self.dirty.load(Ordering::Relaxed) {
            return;
        }

        // Serialize from a snapshot so no lock guard is alive across the writes below.
        let json = {
            let entries = self.read_entries().clone();
            match serde_json::to_vec(&CacheFile { version: CACHE_VERSION, entries }) {
                Ok(json) => json,
                Err(err) => {
                    diag::log("cache", &format!("cannot serialize cache: {err}"));
                    return;
                }
            }
        };

        if let Some(dir) = path.parent() {
            if let Err(err) = tokio::fs::create_dir_all(dir).await {
                diag::log("cache", &format!("cannot create {}: {err}", dir.display()));
                return;
            }
        }
        // Write to a temp file then rename so a crash mid-write can't corrupt the cache.
        let tmp = with_extension(path, "tmp");
        if let Err(err) = tokio::fs::write(&tmp, &json).await {
            diag::log("cache", &format!("cannot write {}: {err}", tmp.display()));
            return;
        }
        // Tighten before the rename, so the file is never world-readable even briefly. This
        // holds private repository data — PR titles, branch names, diff hunks — so it gets the
        // same 0600 the diagnostics log gets, not the 0644 a default umask would leave.
        if let Err(err) = restrict_to_owner(&tmp).await {
            diag::log("cache", &format!("cannot secure {}: {err}", tmp.display()));
            let _ = tokio::fs::remove_file(&tmp).await;
            return;
        }
        if let Err(err) = tokio::fs::rename(&tmp, path).await {
            diag::log("cache", &format!("cannot replace {}: {err}", path.display()));
            return;
        }
        self.dirty.store(false, Ordering::Relaxed);
    }
}

/// Apply the storage bounds at hydration time, where the cost is paid once per run.
fn prune(entries: HashMap<String, StoredEntry>, now: DateTime<Utc>) -> HashMap<String, StoredEntry> {
    let cutoff = now - TimeDelta::days(MAX_AGE_DAYS);
    // Future-dated entries are dropped as well as ancient ones. `put` refuses anything not
    // strictly newer than what is stored, so an entry written under a skewed clock would
    // otherwise refuse every honest write for that key for as long as the file survives —
    // a detail view that silently stops updating, with nothing to age it out.
    let mut kept: Vec<(String, StoredEntry)> = entries
        .into_iter()
        .filter(|(_, entry)| entry.fetched_at > cutoff && entry.fetched_at <= now)
        .collect();
    if kept.len() > MAX_ENTRIES {
        kept.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.fetched_at));
        kept.truncate(MAX_ENTRIES);
    }
    kept.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Item {
        id: u32,
        title: String,
    }

    fn item(id: u32) -> Item {
        Item { id, title: format!("item {id}") }
    }

    #[test]
    fn a_put_value_comes_back_with_its_timestamp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CacheStore::new(dir.path().join("cache.json"));
        let at = Utc::now();

        assert_eq!(store.put("prs", &item(1), at), CachePut::Stored);

        let entry = store.get::<Item>("prs").expect("hit");
        assert_eq!(entry.value, item(1));
        assert_eq!(entry.fetched_at, at);
        assert!(store.get::<Item>("absent").is_none());
    }

    /// The invariant the whole store exists to protect: a slow request that started first must not
    /// be able to land on top of a fresher response.
    #[test]
    fn an_older_put_cannot_overwrite_a_newer_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CacheStore::new(dir.path().join("cache.json"));
        let newer = Utc::now();
        let older = newer - TimeDelta::seconds(30);

        assert_eq!(store.put("prs", &item(2), newer), CachePut::Stored);
        assert_eq!(store.put("prs", &item(1), older), CachePut::Stale, "stale write must be refused");

        let entry = store.get::<Item>("prs").expect("hit");
        assert_eq!(entry.value, item(2), "the newer value must survive");
        assert_eq!(entry.fetched_at, newer);
    }

    #[test]
    fn an_equal_timestamp_put_is_refused_too() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CacheStore::new(dir.path().join("cache.json"));
        let at = Utc::now();

        assert_eq!(store.put("prs", &item(2), at), CachePut::Stored);
        assert_eq!(store.put("prs", &item(9), at), CachePut::Stale);
        assert_eq!(store.get::<Item>("prs").expect("hit").value, item(2));
    }

    #[test]
    fn a_shape_change_reads_as_a_miss_rather_than_a_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CacheStore::new(dir.path().join("cache.json"));
        store.put("prs", &item(1), Utc::now());

        assert!(store.get::<Vec<String>>("prs").is_none());
        // The entry is untouched for a caller that does know its type.
        assert!(store.get::<Item>("prs").is_some());
    }

    /// A value that cannot be serialized stores nothing, but the store isn't holding anything
    /// newer either — so it must not read as `Stale`, which is what tells a caller to stop.
    #[test]
    fn a_value_that_cannot_be_serialized_is_not_reported_as_stale() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CacheStore::new(dir.path().join("cache.json"));
        // serde_json cannot represent a map with non-string keys.
        let unserializable: HashMap<(u8, u8), u8> = HashMap::from([((1, 2), 3)]);

        assert_eq!(store.put("prs", &unserializable, Utc::now()), CachePut::Disabled);
        assert!(store.get::<Item>("prs").is_none(), "nothing was stored");
    }

    #[tokio::test]
    async fn a_corrupt_file_loads_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cache.json");
        std::fs::write(&path, b"{not json").expect("seed");

        let store = CacheStore::new(&path);
        store.load().await;

        assert!(store.get::<Item>("prs").is_none());
    }

    #[tokio::test]
    async fn a_file_from_another_version_is_discarded_wholesale() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cache.json");
        let seeded = format!(
            r#"{{"version":{},"entries":{{"prs":{{"fetched_at":"{}","value":{{"id":1,"title":"item 1"}}}}}}}}"#,
            CACHE_VERSION + 1,
            Utc::now().to_rfc3339()
        );
        std::fs::write(&path, seeded).expect("seed");

        let store = CacheStore::new(&path);
        store.load().await;

        assert!(store.get::<Item>("prs").is_none());
    }

    #[tokio::test]
    async fn a_missing_file_loads_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CacheStore::new(dir.path().join("cache.json"));

        store.load().await;

        assert!(store.get::<Item>("prs").is_none());
    }

    #[tokio::test]
    async fn entries_survive_a_flush_into_a_fresh_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cache.json");
        let at = Utc::now();

        let store = CacheStore::new(&path);
        store.put("prs", &item(7), at);
        store.flush().await;

        let reopened = CacheStore::new(&path);
        reopened.load().await;

        let entry = reopened.get::<Item>("prs").expect("hit");
        assert_eq!(entry.value, item(7));
        assert_eq!(entry.fetched_at, at);
        // Nothing is left behind by the temp + rename.
        assert!(!with_extension(&path, "tmp").exists());
    }

    /// The cache holds private repository data, so it must not be world-readable — the same
    /// standard `diag` holds the log file to.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_flushed_cache_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cache.json");
        let store = CacheStore::new(&path);
        store.put("prs", &item(7), Utc::now());
        store.flush().await;

        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "cache.json had mode {mode:o}");
    }

    #[tokio::test]
    async fn load_drops_entries_past_the_age_bound() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cache.json");
        let now = Utc::now();

        let store = CacheStore::new(&path);
        store.put("fresh", &item(1), now - TimeDelta::days(MAX_AGE_DAYS - 1));
        store.put("stale", &item(2), now - TimeDelta::days(MAX_AGE_DAYS + 1));
        store.flush().await;

        let reopened = CacheStore::new(&path);
        reopened.load().await;

        assert!(reopened.get::<Item>("fresh").is_some());
        assert!(reopened.get::<Item>("stale").is_none(), "expired entry must not hydrate");
    }

    /// A clock-skewed entry would refuse every honest write for its key forever, so ageing it
    /// out is the only thing that can unstick it.
    #[test]
    fn prune_drops_future_dated_entries() {
        let now = Utc::now();
        let entries: HashMap<String, StoredEntry> = [
            ("sane".to_string(), StoredEntry { fetched_at: now - TimeDelta::hours(1), value: serde_json::json!(1) }),
            ("skewed".to_string(), StoredEntry { fetched_at: now + TimeDelta::days(400), value: serde_json::json!(2) }),
        ]
        .into_iter()
        .collect();

        let kept = prune(entries, now);

        assert!(kept.contains_key("sane"));
        assert!(!kept.contains_key("skewed"), "a future-dated entry would wedge its key forever");
    }

    #[test]
    fn prune_keeps_the_most_recent_entries_within_the_count_bound() {
        let now = Utc::now();
        let entries: HashMap<String, StoredEntry> = (0..MAX_ENTRIES + 10)
            .map(|i| {
                let fetched_at = now - TimeDelta::seconds(i as i64);
                let value = serde_json::json!({ "id": i });
                (format!("k{i}"), StoredEntry { fetched_at, value })
            })
            .collect();

        let kept = prune(entries, now);

        assert_eq!(kept.len(), MAX_ENTRIES);
        assert!(kept.contains_key("k0"), "newest must survive");
        assert!(!kept.contains_key(&format!("k{}", MAX_ENTRIES + 9)), "oldest must be dropped");
    }

    #[tokio::test]
    async fn a_disabled_store_never_hits_and_never_writes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = CacheStore::disabled();

        assert_eq!(
            store.put("prs", &item(1), Utc::now()),
            CachePut::Disabled,
            "a disabled store is not refusing the write as stale"
        );
        assert!(store.get::<Item>("prs").is_none());
        store.load().await;
        store.flush().await;

        assert!(store.get::<Item>("prs").is_none());
        let written = std::fs::read_dir(dir.path()).expect("read dir").count();
        assert_eq!(written, 0, "a disabled store must not touch the filesystem");
    }
}
