//! `argus notify-watch` — a separate, unprivileged process (meant for a
//! `systemd --user` unit, running genuinely as you, in your real login
//! session) that tails `events.jsonl` for new entries and fires a desktop
//! toast for each. Exists specifically because the root daemon (`argus
//! daemon`) trying to reach a desktop session itself was a dead end — see
//! `toast.rs`'s doc comment for the full story. This process needs no
//! privileges at all: it only reads a file the daemon writes to and shells
//! out to `omarchy-notification-send`, both things your own user can
//! already do.

use crate::events;
use crate::watcher::is_write_relevant;
use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, new_debouncer};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

pub fn run(log_path: &Path) -> Result<()> {
    // Skip whatever's already logged at startup — only toast for genuinely
    // new events from here on, not the historical backlog (which the TUI
    // is for reviewing, not a flood of toasts on every restart).
    let mut known = events::read_all(log_path).unwrap_or_default().len();
    println!(
        "argus-notify-watch: starting, {known} existing event(s) already in the log (not renotifying for these)"
    );

    // Watch the *directory*, not the file directly — sidesteps both "the
    // file doesn't exist yet" (the daemon may not have logged anything
    // yet) and any inode-swap weirdness if the file is ever recreated.
    // It's a small state directory holding only this one file, so any
    // event in it means "go re-check the log."
    let watch_dir = log_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| log_path.to_path_buf());
    std::fs::create_dir_all(&watch_dir)?;

    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    // Short — this is just watching one small append-only file for a new
    // line, not collapsing multi-event write bursts the way the daemon's
    // own debounce needs to. 200ms adds negligible extra latency on top of
    // the daemon's own 750ms.
    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        None,
        move |result: DebounceEventResult| {
            let _ = tx.send(result);
        },
    )?;
    debouncer.watch(&watch_dir, RecursiveMode::NonRecursive)?;
    println!("argus-notify-watch: watching {}", log_path.display());

    for result in rx {
        match result {
            Ok(debounced_events) => {
                let relevant = debounced_events.iter().any(|e| is_write_relevant(&e.kind));
                if !relevant {
                    continue;
                }
                let all = events::read_all(log_path).unwrap_or_default();
                if all.len() > known {
                    println!(
                        "argus-notify-watch: {} new event(s), notifying",
                        all.len() - known
                    );
                    for record in &all[known..] {
                        crate::toast::notify(&record.path, &record.change);
                    }
                    known = all.len();
                } else if all.len() < known {
                    // The log was replaced/truncated (e.g. someone cleared
                    // it out by hand) — reset the baseline rather than
                    // erroring or replaying history that no longer exists.
                    known = all.len();
                }
            }
            Err(errors) => {
                for e in errors {
                    eprintln!("argus-notify-watch: watch error: {e}");
                }
            }
        }
    }

    Ok(())
}
