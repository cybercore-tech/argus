//! Desktop notification on real drift, using the same
//! `omarchy-notification-send --exec` clickable-toast pattern already used
//! by `pkg-update-watch`/`snapshot-watch` on this box — click opens the
//! Argus TUI via `bin/argus-open` rather than a plain `notify-send` toast
//! with no action.

use crate::change::Change;
use std::process::Command;

fn label(change: &Change) -> &'static str {
    match change {
        Change::New => "new file",
        Change::Deleted => "deleted",
        Change::Modified {
            content_changed,
            mode_changed,
            owner_changed,
        } => match (content_changed, mode_changed, owner_changed) {
            (true, _, _) => "content changed",
            (_, true, _) => "permissions changed",
            (_, _, true) => "ownership changed",
            _ => "modified",
        },
        Change::Unchanged => "unchanged",
    }
}

pub fn notify(path: &str, change: &Change) {
    let open_script = crate::bin_dir().join("argus-open");
    let mut cmd = Command::new("omarchy-notification-send");
    cmd.arg("--app-name")
        .arg("argus")
        .arg("-u")
        .arg("critical")
        .arg(format!("Argus: {}", label(change)))
        .arg(path);
    if open_script.exists() {
        cmd.arg("--exec").arg(open_script);
    }
    if let Err(e) = cmd.status() {
        eprintln!("argus: warning: failed to send desktop notification: {e}");
    }
}
