//! Classifies what happened to one path, reusing SigilWard's exact
//! `Change`/`compare` model from `sigilward/src/diff.rs` — but applied live,
//! one path at a time (against the in-memory baseline the daemon loaded at
//! startup), not as a full two-tree walk. inotify's own event kind
//! (Create/Modify/Remove) is only a "something happened here, go look"
//! signal — a rename-based atomic save fires Remove+Create for what's
//! really one content change, so the real classification always comes from
//! comparing fresh state against the baseline record, same as SigilWard.

use crate::baseline::{FileRecord, record_for};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    New,
    Deleted,
    Modified {
        content_changed: bool,
        mode_changed: bool,
        owner_changed: bool,
    },
    /// The path fired an inotify event but nothing about its recorded
    /// state actually differs — e.g. a read-only `Access` event, or a
    /// write that rewrote identical bytes. Not worth logging as drift.
    Unchanged,
}

fn compare(old: &FileRecord, new: &FileRecord) -> Change {
    let content_changed = old.sha256 != new.sha256 || old.size != new.size;
    let mode_changed = old.mode != new.mode;
    let owner_changed = old.uid != new.uid || old.gid != new.gid;

    if content_changed || mode_changed || owner_changed {
        Change::Modified {
            content_changed,
            mode_changed,
            owner_changed,
        }
    } else {
        Change::Unchanged
    }
}

/// Re-stats/re-hashes `path` right now and classifies it against
/// `baseline_record` (the path's last-known-good record, if any). Returns
/// `Ok(None)` for a path that no longer exists and was never in the
/// baseline either (e.g. a transient temp file inside a watched directory
/// that isn't itself tracked).
pub fn classify(
    path: &Path,
    baseline_record: Option<&FileRecord>,
) -> std::io::Result<Option<Change>> {
    // Directories are never tracked (same convention as SigilWard's own
    // baseline walk, which skips them entirely — only files/symlinks get
    // recorded). Every file create/delete under a recursively-watched
    // directory also fires its own separate inotify event for the
    // *directory itself* (its mtime just changed too), so this is hit
    // constantly in normal operation, not an edge case — without this
    // check, record_for's hash_file() tries to open() a directory and
    // fails with EISDIR on every single one of those.
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.file_type().is_dir()
    {
        return Ok(baseline_record.map(|_| Change::Modified {
            content_changed: true,
            mode_changed: false,
            owner_changed: false,
        }));
    }

    let current = record_for(path);
    match (baseline_record, current) {
        (None, Ok(_)) => Ok(Some(Change::New)),
        (Some(_), Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(Some(Change::Deleted)),
        (Some(old), Ok(new)) => Ok(Some(compare(old, &new))),
        (None, Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        (_, Err(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(hash: &str, mode: u32, uid: u32, gid: u32, size: u64) -> FileRecord {
        FileRecord {
            sha256: hash.to_string(),
            mode,
            uid,
            gid,
            size,
        }
    }

    #[test]
    fn detects_content_change() {
        let old = record("abc", 0o644, 0, 0, 10);
        let new = record("xyz", 0o644, 0, 0, 12);
        match compare(&old, &new) {
            Change::Modified {
                content_changed,
                mode_changed,
                owner_changed,
            } => {
                assert!(content_changed);
                assert!(!mode_changed);
                assert!(!owner_changed);
            }
            other => panic!("expected Modified, got {other:?}"),
        }
    }

    #[test]
    fn detects_permission_change_with_same_content() {
        let old = record("abc", 0o644, 0, 0, 10);
        let new = record("abc", 0o777, 0, 0, 10);
        match compare(&old, &new) {
            Change::Modified {
                content_changed,
                mode_changed,
                ..
            } => {
                assert!(!content_changed);
                assert!(mode_changed);
            }
            other => panic!("expected Modified, got {other:?}"),
        }
    }

    #[test]
    fn detects_ownership_change() {
        let old = record("abc", 0o644, 0, 0, 10);
        let new = record("abc", 0o644, 1000, 1000, 10);
        match compare(&old, &new) {
            Change::Modified { owner_changed, .. } => assert!(owner_changed),
            other => panic!("expected Modified, got {other:?}"),
        }
    }

    #[test]
    fn identical_records_are_unchanged() {
        let old = record("abc", 0o644, 0, 0, 10);
        let new = record("abc", 0o644, 0, 0, 10);
        assert_eq!(compare(&old, &new), Change::Unchanged);
    }

    #[test]
    fn classify_detects_new_file_not_in_baseline() {
        let dir = std::env::temp_dir().join(format!("argus-change-test1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("new.txt");
        std::fs::write(&path, b"fresh").unwrap();

        let result = classify(&path, None).unwrap();
        assert_eq!(result, Some(Change::New));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn classify_detects_deleted_file_still_in_baseline() {
        let dir = std::env::temp_dir().join(format!("argus-change-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gone.txt");
        let old = record("abc", 0o644, 0, 0, 10);

        let result = classify(&path, Some(&old)).unwrap();
        assert_eq!(result, Some(Change::Deleted));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn classify_returns_none_for_untracked_transient_path() {
        let dir = std::env::temp_dir().join(format!("argus-change-test3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("never-existed.tmp");

        let result = classify(&path, None).unwrap();
        assert_eq!(result, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn classify_ignores_an_untracked_directorys_own_inotify_event() {
        // The real-world case that broke the first live deploy: every file
        // create/delete under a recursively-watched directory also fires
        // its own separate inotify event for the *directory itself* (its
        // mtime changed too). Directories are never tracked, so this must
        // not attempt to hash the directory and must not error.
        let dir = std::env::temp_dir().join(format!("argus-change-test5-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("subdir")).unwrap();

        let result = classify(&dir.join("subdir"), None).unwrap();
        assert_eq!(result, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn classify_flags_a_tracked_file_path_replaced_by_a_directory() {
        // Rare, but a real type change worth surfacing rather than
        // silently dropping like the untracked case above.
        let dir = std::env::temp_dir().join(format!("argus-change-test6-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("was-a-file")).unwrap();

        let old = record("abc", 0o644, 0, 0, 10);
        let result = classify(&dir.join("was-a-file"), Some(&old)).unwrap();
        assert!(matches!(
            result,
            Some(Change::Modified {
                content_changed: true,
                ..
            })
        ));

        std::fs::remove_dir_all(&dir).ok();
    }
}
