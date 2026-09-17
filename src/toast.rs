//! Desktop notification on real drift, using the same
//! `omarchy-notification-send --exec` clickable-toast pattern already used
//! by `pkg-update-watch`/`snapshot-watch` on this box — click opens the
//! Argus TUI via `bin/argus-open` rather than a plain `notify-send` toast
//! with no action.
//!
//! Deliberately never called from the root daemon (`argus daemon`) — a
//! root system-service process reaching a user's desktop session was a
//! real dead end, confirmed live: exporting `DBUS_SESSION_BUS_ADDRESS`/
//! `WAYLAND_DISPLAY` on root's own environment failed outright (`busctl:
//! Failed to connect to user scope bus... Broken pipe`), and dropping to
//! `runuser -u raven` for just the notification subprocess got past that
//! error but still never produced a visible toast — most likely because a
//! process spawned via `runuser` from a *system* unit isn't a member of
//! the real login session's cgroup/scope (`user@1000.service`), which some
//! notification-daemon/compositor security policy silently rejects even
//! after the D-Bus handshake itself succeeds. Rather than keep guessing at
//! a workaround for that session-boundary problem, `argus notify-watch`
//! (see `main.rs`) runs this from a completely separate `--user` systemd
//! unit instead — genuinely inside the user session, no bridging needed.

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
