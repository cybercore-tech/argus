# Argus

Real-time file-integrity watcher. Extends [SigilWard](https://github.com/darkstardevx/sigilward)'s
point-in-time baseline with a live inotify daemon — the same watched paths and
the same baseline, but change detection the moment it happens instead of at
the next daily timer run.

## Why

SigilWard bases drift detection on `sigilward-check.timer` (daily). Argus
adds the real-time half: `notify` + `notify-debouncer-full` watch the exact
paths in `~/.config/sigilward/config.toml`, classify every real change
against SigilWard's own baseline, and log it. Only actual writes trigger a
re-check — pure reads (someone opening the file, e.g. `sudo` reading
`/etc/sudoers` on every invocation, or anything else polling unit files)
are filtered out before they ever get re-hashed.

A TUI (`argus`, no args) reviews the event log and can accept a reviewed
change directly into SigilWard's baseline with one key — so `sigilward
check` stops re-flagging something you already looked at in Argus, without
a blanket `sigilward update` silently re-baselining everything else too.

## Usage

```
argus              # TUI event viewer
argus daemon       # foreground watch loop (run via argus.service)
argus notify-watch # tail the event log and fire desktop toasts (run via argus-notify.service)
argus events [--since <RFC3339>]   # JSON dump for scripting
```

`daemon` only detects and logs — it never sends a desktop notification
itself. A root system service reaching your actual desktop session doesn't
work reliably (tried two ways, both confirmed broken live: exporting
`DBUS_SESSION_BUS_ADDRESS` on root's own environment, then dropping to
`runuser -u <you>` for just the notification — neither produced a visible
toast, most likely a session/cgroup-scope problem beyond just env vars).
`notify-watch` is a separate, unprivileged process for exactly this: run it
as yourself via a `systemd --user` unit, genuinely inside your session.

## Install

```
cargo build --release
sudo cp target/release/argus /usr/local/bin/   # or use ~/.cargo-target/release/argus directly
mkdir -p ~/.local/state/argus   # required once, before first start — see argus.service's own comment for why

# the privileged watcher (detect + log only, no notifications)
sudo cp argus.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now argus.service

# the unprivileged notifier (desktop toasts, runs as you)
mkdir -p ~/.config/systemd/user
cp argus-notify.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now argus-notify.service
```

`argus.service` runs as root (unlike `sigilward-check.service`, which runs
as your own user and has silently never been able to read `/etc/sudoers` —
mode `0440 root:root`). Argus needs real read access to the paths it's
meant to catch tampering in. `argus-notify.service` runs as you, with no
special privileges — it only reads the event log `argus.service` writes to
and shells out to `omarchy-notification-send`.
