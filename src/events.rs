//! Append-only JSON-lines event log at `~/.local/state/argus/events.jsonl`.
//! The daemon appends; the TUI and `argus events` just read the same file —
//! no IPC needed between them.

use crate::change::Change;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EventRecord {
    pub at: DateTime<Utc>,
    pub path: String,
    pub change: Change,
    #[serde(default)]
    pub accepted: bool,
}

pub fn default_log_path() -> PathBuf {
    let base = dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("argus").join("events.jsonl")
}

pub fn append(log_path: &Path, record: &EventRecord) -> Result<()> {
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    let line = serde_json::to_string(record)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Reads every record currently in the log, oldest first. Malformed lines
/// (a partial write caught mid-flush, e.g.) are skipped rather than
/// aborting the whole read — one bad line shouldn't hide the rest of the
/// history.
pub fn read_all(log_path: &Path) -> Result<Vec<EventRecord>> {
    if !log_path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(log_path)?;
    Ok(raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

/// Marks the most recent unaccepted record for `path` as accepted, by
/// rewriting the log with that one line updated. The log is small (one
/// line per real drift event, not per file) so a full rewrite on accept is
/// cheap and far simpler than an index.
///
/// Writes to a temp file in the same directory and renames it over the
/// original, rather than truncating the existing file in place — not just
/// for atomicity, but because it's the only thing that actually works
/// here: `events.jsonl` is created by the root daemon (`argus daemon`),
/// so it's `root`-owned, and the TUI's `a` (accept) action runs as a
/// normal user. Opening the *existing* file for write needs permission on
/// the file itself (which a non-owner doesn't have — confirmed live,
/// `EACCES`/os error 13). A rename-based replace only needs write
/// permission on the *directory*, which the user does have (they created
/// `~/.local/state/argus/` themselves, per the one-time setup step).
pub fn mark_accepted(log_path: &Path, path: &str, at: DateTime<Utc>) -> Result<()> {
    let mut records = read_all(log_path)?;
    if let Some(record) = records
        .iter_mut()
        .rev()
        .find(|r| r.path == path && r.at == at)
    {
        record.accepted = true;
    }
    let tmp_path = log_path.with_extension("jsonl.tmp");
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        for record in &records {
            writeln!(file, "{}", serde_json::to_string(record)?)?;
        }
    }
    std::fs::rename(&tmp_path, log_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_round_trip() {
        let dir = std::env::temp_dir().join(format!("argus-events-test1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("events.jsonl");

        let record = EventRecord {
            at: Utc::now(),
            path: "/etc/foo".to_string(),
            change: Change::New,
            accepted: false,
        };
        append(&log_path, &record).unwrap();

        let all = read_all(&log_path).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].path, "/etc/foo");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_all_skips_malformed_lines() {
        let dir = std::env::temp_dir().join(format!("argus-events-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("events.jsonl");

        std::fs::write(&log_path, "not json\n{\"broken\":\n").unwrap();
        let all = read_all(&log_path).unwrap();
        assert!(all.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mark_accepted_flags_the_matching_record() {
        let dir = std::env::temp_dir().join(format!("argus-events-test3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("events.jsonl");

        let at = Utc::now();
        let record = EventRecord {
            at,
            path: "/etc/foo".to_string(),
            change: Change::New,
            accepted: false,
        };
        append(&log_path, &record).unwrap();

        mark_accepted(&log_path, "/etc/foo", at).unwrap();

        let all = read_all(&log_path).unwrap();
        assert!(all[0].accepted);

        std::fs::remove_dir_all(&dir).ok();
    }
}
