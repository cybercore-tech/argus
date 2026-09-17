mod app;
mod baseline;
mod change;
mod config;
mod events;
mod notify_watch;
mod theme;
mod toast;
mod ui;
mod watcher;

use anyhow::Result;
use app::{App, Mode};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::path::PathBuf;
use theme::Theme;

pub fn bin_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("tools/security/argus/bin")
}

enum ArgMode {
    Tui,
    Daemon,
    NotifyWatch,
    AcceptOne { path: String, at: String },
    Events { since: Option<String> },
}

fn parse_args() -> std::result::Result<ArgMode, String> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => Ok(ArgMode::Tui),
        Some("daemon") => Ok(ArgMode::Daemon),
        Some("notify-watch") => Ok(ArgMode::NotifyWatch),
        Some("accept-one") => {
            let path = args
                .next()
                .ok_or_else(|| "argus accept-one: missing <path>".to_string())?;
            let at = args
                .next()
                .ok_or_else(|| "argus accept-one: missing <at> (RFC3339 timestamp)".to_string())?;
            Ok(ArgMode::AcceptOne { path, at })
        }
        Some("events") => {
            let mut since = None;
            while let Some(arg) = args.next() {
                if arg == "--since" {
                    since = args.next();
                }
            }
            Ok(ArgMode::Events { since })
        }
        Some("-h") | Some("--help") => {
            print_usage();
            std::process::exit(0);
        }
        Some(other) => Err(format!(
            "argus: unrecognized argument '{other}'\nRun `argus --help` for usage."
        )),
    }
}

fn print_usage() {
    println!(
        "argus — real-time file-integrity watcher, extends SigilWard's baseline live\n\
         \n\
         USAGE:\n\
         \x20   argus              Open the TUI event viewer\n\
         \x20   argus daemon       Run the real-time watcher in the foreground (for systemd)\n\
         \x20   argus notify-watch Tail the event log and fire desktop toasts (for a --user unit)\n\
         \x20   argus accept-one <path> <at>   Re-hash <path> and write it into SigilWard's\n\
         \x20                      baseline, marking the <at>-timestamped event accepted.\n\
         \x20                      Meant to be run via sudo — this is what the TUI's `a`\n\
         \x20                      key actually shells out to, not usually run by hand.\n\
         \x20   argus events [--since <RFC3339>]   Print the event log as JSON\n\
         \x20   -h, --help         Print this help and exit\n\
         \n\
         Watches the same paths as SigilWard (~/.config/sigilward/config.toml)\n\
         and shares its baseline (~/.local/state/sigilward/baseline.json).\n\
         Events log to ~/.local/state/argus/events.jsonl.\n\
         \n\
         `daemon` only detects and logs — it deliberately never sends a\n\
         desktop notification itself (a root system service reaching a\n\
         user's desktop session doesn't work reliably). Run `notify-watch`\n\
         separately, as yourself, for toasts."
    );
}

fn resolve_config() -> Result<config::AppConfig> {
    let path = config::default_config_path().ok_or_else(|| {
        anyhow::anyhow!("no sigilward config found — run sigilward's own setup first")
    })?;
    config::load(&path)
}

fn run_daemon() -> Result<()> {
    let cfg = resolve_config()?;
    let baseline_path = config::expand_home(&cfg.baseline_path);
    let baseline = baseline::load(&baseline_path).unwrap_or_default();
    if baseline.files.is_empty() {
        eprintln!(
            "argus: warning: baseline at {} is empty or missing — every watched file will read as New until you run `sigilward init`",
            baseline_path.display()
        );
    }

    let targets: Vec<watcher::WatchTarget> = cfg
        .watch
        .iter()
        .map(|w| watcher::WatchTarget {
            path: config::expand_home(&w.path),
            recursive: w.recursive,
        })
        .collect();

    let log_path = events::default_log_path();
    watcher::run(&targets, &baseline, &log_path)
}

fn run_notify_watch() -> Result<()> {
    let log_path = events::default_log_path();
    notify_watch::run(&log_path)
}

/// Re-hashes `path` right now and writes it into SigilWard's real
/// baseline, then marks the matching (by path + timestamp) event in
/// Argus's own log accepted. Deliberately a *separate* CLI mode, not
/// something `run_tui` does directly — some watched paths (`/etc/sudoers`,
/// or any unit file that happens to be root-only) can only be read by
/// root, and the TUI itself runs unprivileged. The TUI's `a` key shells
/// out to `sudo argus accept-one <path> <at>` with a real inherited
/// terminal for the password prompt (same pattern as cyberwatch's own
/// sudo-in-terminal actions), rather than the TUI process itself trying
/// to gain privilege.
fn run_accept_one(path: &str, at: &str) -> Result<()> {
    // Hardcoded, not resolved via $HOME/$XDG_*: this runs under `sudo`,
    // and whether `sudo` preserves or resets HOME depends on this box's
    // sudoers policy (`always_set_home` and friends) — exactly the same
    // class of bug that bit argus.service before `Environment=HOME=...`
    // was added there. Sidestep it entirely rather than risk silently
    // resolving to /root/.config here too.
    let config_path = std::path::Path::new("/home/raven/.config/sigilward/config.toml");
    let cfg = config::load(config_path)?;
    // Not config::expand_home — it reads $HOME too, same risk as above.
    let baseline_path = std::path::PathBuf::from(cfg.baseline_path.replacen("~", "/home/raven", 1));
    let log_path = std::path::PathBuf::from("/home/raven/.local/state/argus/events.jsonl");
    let at = chrono::DateTime::parse_from_rfc3339(at)
        .map_err(|e| anyhow::anyhow!("invalid <at> timestamp: {e}"))?
        .with_timezone(&chrono::Utc);

    baseline::accept(&baseline_path, path)?;
    events::mark_accepted(&log_path, path, at)?;
    println!("argus: accepted {path} into the baseline");
    Ok(())
}

fn run_events(since: Option<String>) -> Result<()> {
    let log_path = events::default_log_path();
    let mut all = events::read_all(&log_path)?;
    if let Some(since) = since {
        let cutoff = chrono::DateTime::parse_from_rfc3339(&since)
            .map_err(|e| anyhow::anyhow!("invalid --since timestamp: {e}"))?
            .with_timezone(&chrono::Utc);
        all.retain(|e| e.at >= cutoff);
    }
    println!("{}", serde_json::to_string(&all)?);
    Ok(())
}

fn run_tui() -> Result<()> {
    let log_path = events::default_log_path();
    let theme = Theme::from_cybercore();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(log_path)?;
    let result = event_loop(&mut terminal, &mut app, &theme);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

fn main() -> Result<()> {
    match parse_args() {
        Ok(ArgMode::Daemon) => run_daemon(),
        Ok(ArgMode::NotifyWatch) => run_notify_watch(),
        Ok(ArgMode::AcceptOne { path, at }) => run_accept_one(&path, &at),
        Ok(ArgMode::Events { since }) => run_events(since),
        Ok(ArgMode::Tui) => run_tui(),
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    }
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    theme: &Theme,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app, theme))?;

        if let Some(job) = app.accept_job.take() {
            suspend_and_accept(terminal, app, job)?;
            continue;
        }

        if app.should_quit {
            return Ok(());
        }

        if let Event::Key(key) = event::read()? {
            match app.mode {
                Mode::Normal => handle_normal(app, key.code, key.modifiers),
                Mode::Filter => handle_filter(app, key.code),
                Mode::Detail => handle_detail(app, key.code),
                Mode::Help => handle_help(app, key.code),
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

/// Leaves the alternate screen/raw mode, runs `sudo argus accept-one
/// <path> <at>` with the parent's real stdio inherited (so `sudo`'s
/// password prompt has a real terminal to write to — same reasoning as
/// cyberwatch's `suspend_and_act`), then resumes the TUI. `at` goes over
/// as RFC3339 so the subprocess can find the exact matching log entry.
fn suspend_and_accept(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    job: app::AcceptJob,
) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;

    let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("argus"));
    let at = job.at.to_rfc3339();
    println!("\n=== sudo argus accept-one {} ===", job.path);
    match std::process::Command::new("sudo")
        .arg(&current_exe)
        .arg("accept-one")
        .arg(&job.path)
        .arg(&at)
        .status()
    {
        Ok(status) if status.success() => {
            app.status = Some(format!("accepted: {}", job.path));
        }
        Ok(status) => {
            println!("accept-one exited with {status}");
            app.status = Some(format!("accept failed (see above): {}", job.path));
        }
        Err(e) => {
            println!("failed to run accept-one: {e}");
            app.status = Some(format!("accept failed: {e}"));
        }
    }
    println!("\nPress Enter to return to argus.");
    let mut discard = String::new();
    let _ = io::stdin().read_line(&mut discard);

    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    )?;
    terminal.clear()?;
    app.reload();
    Ok(())
}

fn handle_normal(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('j') | KeyCode::Down => app.next(),
        KeyCode::Char('k') | KeyCode::Up => app.previous(),
        KeyCode::Char('/') => app.mode = Mode::Filter,
        KeyCode::Char('R') => {
            app.reload();
            app.status = Some("reloaded.".to_string());
        }
        KeyCode::Char('a') => app.request_accept_selected(),
        KeyCode::Enter | KeyCode::Char('l') => {
            if app.selected_event().is_some() {
                app.mode = Mode::Detail;
            }
        }
        KeyCode::Char('?') => app.mode = Mode::Help,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.should_quit = true,
        _ => {}
    }
}

fn handle_filter(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.filter_text.clear();
            app.apply_sort_and_filter();
            app.mode = Mode::Normal;
        }
        KeyCode::Enter => app.mode = Mode::Normal,
        KeyCode::Backspace => {
            app.filter_text.pop();
            app.apply_sort_and_filter();
        }
        KeyCode::Char(c) => {
            app.filter_text.push(c);
            app.apply_sort_and_filter();
        }
        _ => {}
    }
}

fn handle_detail(app: &mut App, code: KeyCode) {
    if matches!(code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('l')) {
        app.mode = Mode::Normal;
    }
}

fn handle_help(app: &mut App, code: KeyCode) {
    if matches!(code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')) {
        app.mode = Mode::Normal;
    }
}
