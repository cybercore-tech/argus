//! Desktop notification on real drift, using the same
//! `omarchy-notification-send --exec` clickable-toast pattern already used
//! by `pkg-update-watch`/`snapshot-watch` on this box — click opens the
//! Argus TUI via `bin/argus-open` rather than a plain `notify-send` toast
//! with no action.

use crate::change::Change;
use std::process::Command;

/// This is only ever called from the daemon (`toast::notify` is invoked by
/// `watcher::handle_path`, which only runs under `argus daemon` — the
/// interactive TUI never watches files live), which runs as root under
/// systemd. A root process has no desktop session of its own, and simply
/// exporting `DBUS_SESSION_BUS_ADDRESS`/`WAYLAND_DISPLAY` on the daemon's
/// own environment (the first attempt) wasn't enough — D-Bus's session bus
/// authenticates by the *connecting process's* UID, not just the socket
/// path, and rejected root: `busctl: Failed to connect to user scope bus
/// via local transport: Broken pipe` / `Transport endpoint is not
/// connected`, confirmed live. `runuser -u raven` drops to the real user
/// for just this one subprocess before it opens the D-Bus connection —
/// the standard pattern for a root daemon reaching a specific user's
/// session. Env vars are passed explicitly on the command rather than
/// relying on inheritance through runuser, which isn't guaranteed.
const RAVEN_USER: &str = "raven";
const RAVEN_RUNTIME_DIR: &str = "/run/user/1000";

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
    let mut cmd = Command::new("runuser");
    cmd.arg("-u").arg(RAVEN_USER).arg("--");
    cmd.arg("omarchy-notification-send")
        .arg("--app-name")
        .arg("argus")
        .arg("-u")
        .arg("critical")
        .arg(format!("Argus: {}", label(change)))
        .arg(path);
    if open_script.exists() {
        cmd.arg("--exec").arg(open_script);
    }
    cmd.env("XDG_RUNTIME_DIR", RAVEN_RUNTIME_DIR)
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={RAVEN_RUNTIME_DIR}/bus"),
        )
        .env("WAYLAND_DISPLAY", "wayland-1");
    if let Err(e) = cmd.status() {
        eprintln!("argus: warning: failed to send desktop notification: {e}");
    }
}
