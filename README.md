# Argus

Real-time file-integrity watcher. Extends [SigilWard](https://github.com/darkstardevx/sigilward)'s
point-in-time baseline with a live inotify daemon — the same watched paths and
the same baseline, but change detection the moment it happens instead of at
the next daily timer run.

## Why

SigilWard bases drift detection on `sigilward-check.timer` (daily). Argus
adds the real-time half: `notify` + `notify-debouncer-full` watch the exact
paths in `~/.config/sigilward/config.toml`, classify every real change
against SigilWard's own baseline, log it, and fire a desktop toast.

A TUI (`argus`, no args) reviews the event log and can accept a reviewed
change directly into SigilWard's baseline with one key — so `sigilward
check` stops re-flagging something you already looked at in Argus, without
a blanket `sigilward update` silently re-baselining everything else too.

## Usage

```
argus            # TUI event viewer
argus daemon     # foreground watch loop (run via argus.service)
argus events      [--since <RFC3339>]   # JSON dump for scripting
```

## Install

```
cargo build --release
sudo cp target/release/argus /usr/local/bin/   # or use ~/.cargo-target/release/argus directly
mkdir -p ~/.local/state/argus   # required once, before first start — see argus.service's own comment for why
sudo cp argus.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now argus.service
```

Runs as root (unlike `sigilward-check.service`, which runs as your own user
and has silently never been able to read `/etc/sudoers` — mode `0440
root:root`). Argus needs real read access to the paths it's meant to catch
tampering in.
