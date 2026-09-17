//! Sets up a debounced, inotify-backed watch (via `notify` +
//! `notify-debouncer-full`) over every `[[watch]]` entry, and turns each
//! debounced filesystem event into a classified `Change` against the
//! baseline loaded at startup — appending real drift to the event log and
//! firing a desktop toast.

use crate::baseline::Baseline;
use crate::change::{self, Change};
use crate::events::{self, EventRecord};
use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, new_debouncer};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

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
                // TEMPORARY diagnostic: a real deploy showed /etc/sudoers
                // and argus.service repeatedly logged as "New" roughly
                // every 2s (== the debounce timeout) with no real change —
                // a self-sustaining loop of some kind. Logging the raw
                // notify::EventKind for every debounced event (before any
                // of our own dedup/classify logic runs) to see what's
                // actually arriving, rather than guess further.
                for debounced in &debounced_events {
                    eprintln!(
                        "argus: DIAG raw event: kind={:?} paths={:?}",
                        debounced.kind, debounced.paths
                    );
                }

                // A single save can surface as several `DebouncedEvent`s for
                // the same path (e.g. separate Create/Modify(Data)/
                // Modify(Metadata) kinds) even within one debounced batch —
                // dedupe to one classify-and-log per unique path per batch,
                // not per raw event, or one real edit logs several
                // identical lines.
                let mut seen = std::collections::HashSet::new();
                for debounced in &debounced_events {
                    for path in &debounced.paths {
                        if seen.insert(path.clone()) {
                            handle_path(path, baseline, log_path);
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

fn handle_path(path: &std::path::Path, baseline: &Baseline, log_path: &std::path::Path) {
    let Some((key, change)) = classify_and_log(path, baseline, log_path) else {
        return;
    };
    crate::toast::notify(&key, &change);
}

/// The testable core: classify `path` against `baseline` and, if it's a
/// real change (not `Unchanged`/untracked), append it to the event log and
/// return it. Split out from `handle_path` so tests can exercise the real
/// classify-and-log logic without also firing a desktop notification.
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
    use std::collections::BTreeMap;

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
