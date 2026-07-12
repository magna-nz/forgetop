//! Lightweight file logging for diagnostics — a crash log and runtime fetch errors, so a
//! user (or we) can review what went wrong after the fact. Best-effort by design: it never
//! panics and never blocks the UI on IO — a failed write is silently dropped.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;

/// `$XDG_CONFIG_HOME/forgetop/forgetop.log` (next to `config.json`), or `./forgetop.log`.
pub fn log_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("forgetop").join("forgetop.log")
}

/// Append a timestamped `context: message` line to the log file. Best-effort.
pub fn log(context: &str, message: &str) {
    let path = log_path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let line = format!("{} [{}] {}\n", Utc::now().to_rfc3339(), context, message.trim());
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_is_the_forgetop_log_file() {
        let p = log_path();
        assert_eq!(p.file_name().unwrap(), "forgetop.log");
        assert!(p.to_string_lossy().contains("forgetop"));
    }
}
