//! Sets up a debounced, inotify-backed watch (via `notify` +
//! `notify-debouncer-full`) over every `[[watch]]` entry, and turns each
//! debounced filesystem event into a classified `Change` against the
//! baseline loaded at startup — appending real drift to the event log.
//! Deliberately does NOT fire a desktop toast itself (see `toast.rs` for
//! why) — that's `argus notify-watch`'s job, a separate `--user` process.

use crate::baseline::Baseline;
use crate::change::{self, Change};
use crate::events::{self, EventRecord};
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, new_debouncer};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// Whether a raw event kind is worth re-hashing a path over. A live deploy
/// showed `Access(Open(Any))` firing repeatedly for every custom unit file
/// under `/etc/systemd/system` — something (most likely cyberdeck-hub's own
/// live-status polling) opens all of them on a short cycle, which is
/// completely legitimate read traffic, not tampering. A file-integrity tool
/// cares about writes (content/permissions/ownership actually changing),
/// not reads — reacting to every open wastes a full re-hash on each poll,
/// and for a path with no baseline entry yet (like a freshly-created,
/// not-yet-`sigilward update`d unit file) it meant logging "New" on every
/// single read forever, which is genuinely spammy, not just wasteful.
pub fn is_write_relevant(kind: &EventKind) -> bool {
    !matches!(kind, EventKind::Access(_))
}

pub struct WatchTarget {
    pub path: PathBuf,
    pub recursive: bool,
}

/// Runs the watch loop until the process is killed (this is the body of
/// `argus daemon`, meant to run forever under systemd). Blocking — the
/// debouncer's callback runs on its own thread and sends batches of
/// `DebouncedEvent`s over a channel to this loop, which does the actual
/// classify/log/notify work.
pub fn run(
    targets: &[WatchTarget],
    baseline: &Baseline,
    log_path: &std::path::Path,
) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();

    let mut debouncer = new_debouncer(
        Duration::from_secs(2),
        None,
        move |result: DebounceEventResult| {
            let _ = tx.send(result);
        },
    )?;

    for target in targets {
        let mode = if target.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if let Err(e) = debouncer.watch(&target.path, mode) {
            eprintln!(
                "argus: warning: failed to watch {}: {e}",
                target.path.display()
            );
        } else {
            println!(
                "argus: watching {} ({})",
                target.path.display(),
                if target.recursive {
                    "recursive"
                } else {
                    "flat"
                }
            );
        }
    }

    println!("argus: daemon running, {} watch root(s)", targets.len());

    for result in rx {
        match result {
            Ok(debounced_events) => {
                // A single save can surface as several `DebouncedEvent`s for
                // the same path (e.g. separate Create/Modify(Data)/
                // Modify(Metadata) kinds) even within one debounced batch —
                // dedupe to one classify-and-log per unique path per batch,
                // not per raw event, or one real edit logs several
                // identical lines. `Access` events (mere reads/opens — see
                // `is_write_relevant`) are filtered out before that, so
                // something merely reading a watched file never triggers a
                // re-hash at all.
                let mut seen = std::collections::HashSet::new();
                for debounced in &debounced_events {
                    if !is_write_relevant(&debounced.kind) {
                        continue;
                    }
                    for path in &debounced.paths {
                        if seen.insert(path.clone()) {
                            classify_and_log(path, baseline, log_path);
                        }
                    }
                }
            }
            Err(errors) => {
                for e in errors {
                    eprintln!("argus: watch error: {e}");
                }
            }
        }
    }

    Ok(())
}

/// Classifies `path` against `baseline` and, if it's a real change (not
/// `Unchanged`/untracked), appends it to the event log and returns it —
/// this is the daemon's entire job now (see `toast.rs` for why it no
/// longer also fires a notification directly).
fn classify_and_log(
    path: &std::path::Path,
    baseline: &Baseline,
    log_path: &std::path::Path,
) -> Option<(String, Change)> {
    let key = path.to_string_lossy().into_owned();
    let baseline_record = baseline.files.get(&key);

    let outcome = match change::classify(path, baseline_record) {
        Ok(outcome) => outcome,
        Err(e) => {
            eprintln!("argus: warning: could not classify {}: {e}", path.display());
            return None;
        }
    };

    let change = match outcome {
        Some(Change::Unchanged) | None => return None,
        Some(change) => change,
    };

    let record = EventRecord {
        at: chrono::Utc::now(),
        path: key.clone(),
        change: change.clone(),
        accepted: false,
    };
    if let Err(e) = events::append(log_path, &record) {
        eprintln!("argus: warning: failed to log event for {key}: {e}");
    }

    Some((key, change))
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind};
    use std::collections::BTreeMap;

    #[test]
    fn access_events_are_never_write_relevant() {
        assert!(!is_write_relevant(&EventKind::Access(AccessKind::Open(
            notify::event::AccessMode::Any
        ))));
        assert!(!is_write_relevant(&EventKind::Access(AccessKind::Read)));
        assert!(!is_write_relevant(&EventKind::Access(AccessKind::Close(
            AccessMode::Any
        ))));
    }

    #[test]
    fn create_modify_remove_are_write_relevant() {
        assert!(is_write_relevant(&EventKind::Create(CreateKind::File)));
        assert!(is_write_relevant(&EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Any
        ))));
        assert!(is_write_relevant(&EventKind::Remove(RemoveKind::File)));
    }

    #[test]
    fn detects_and_logs_a_real_permission_change_against_baseline() {
        let dir = std::env::temp_dir().join(format!("argus-watcher-test1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let watched = dir.join("watched.conf");
        std::fs::write(&watched, b"original content").unwrap();

        // Baseline as it was when trusted: original content, mode 0o644.
        let original_record = crate::baseline::record_for(&watched).unwrap();
        let mut baseline = Baseline::default();
        baseline
            .files
            .insert(watched.to_string_lossy().into_owned(), original_record);

        // Now the file actually changes on disk (simulating what a real
        // inotify event would have fired for) — this is the same effect a
        // live edit under a watched directory would have.
        std::fs::write(&watched, b"tampered content").unwrap();

        let log_path = dir.join("events.jsonl");
        let result = classify_and_log(&watched, &baseline, &log_path);

        assert!(result.is_some());
        let (_, change) = result.unwrap();
        assert!(matches!(
            change,
            Change::Modified {
                content_changed: true,
                ..
            }
        ));

        let logged = events::read_all(&log_path).unwrap();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].path, watched.to_string_lossy());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_path_matching_its_baseline_exactly_logs_nothing() {
        let dir = std::env::temp_dir().join(format!("argus-watcher-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let watched = dir.join("stable.conf");
        std::fs::write(&watched, b"never changes").unwrap();

        let record = crate::baseline::record_for(&watched).unwrap();
        let mut baseline = Baseline::default();
        baseline
            .files
            .insert(watched.to_string_lossy().into_owned(), record);

        let log_path = dir.join("events.jsonl");
        // No modification happened — a spurious inotify Access-type event
        // firing on a read shouldn't produce a logged change.
        let result = classify_and_log(&watched, &baseline, &log_path);

        assert!(result.is_none());
        assert!(events::read_all(&log_path).unwrap().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_new_untracked_file_is_detected_as_new() {
        let dir = std::env::temp_dir().join(format!("argus-watcher-test3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let watched = dir.join("brand-new.conf");
        std::fs::write(&watched, b"wasn't here before").unwrap();

        let baseline = Baseline {
            files: BTreeMap::new(),
        };
        let log_path = dir.join("events.jsonl");
        let result = classify_and_log(&watched, &baseline, &log_path);

        assert_eq!(result.map(|(_, c)| c), Some(Change::New));

        std::fs::remove_dir_all(&dir).ok();
    }
}
