use crate::change::Change;
use crate::events::{self, EventRecord};
use anyhow::Result;
use std::path::PathBuf;

pub enum Mode {
    Normal,
    Filter,
    Detail,
    Help,
}

pub struct App {
    pub events: Vec<EventRecord>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub filter_text: String,
    pub mode: Mode,
    pub status: Option<String>,
    pub should_quit: bool,
    log_path: PathBuf,
    baseline_path: PathBuf,
}

impl App {
    pub fn new(log_path: PathBuf, baseline_path: PathBuf) -> Result<Self> {
        let mut app = Self {
            events: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            filter_text: String::new(),
            mode: Mode::Normal,
            status: None,
            should_quit: false,
            log_path,
            baseline_path,
        };
        app.reload();
        Ok(app)
    }

    /// Re-reads the event log from disk — the daemon may have appended
    /// more events since the TUI opened.
    pub fn reload(&mut self) {
        self.events = events::read_all(&self.log_path).unwrap_or_default();
        self.apply_sort_and_filter();
    }

    /// Most recent first, unreviewed events before already-accepted ones.
    pub fn apply_sort_and_filter(&mut self) {
        self.events
            .sort_by(|a, b| a.accepted.cmp(&b.accepted).then_with(|| b.at.cmp(&a.at)));
        let needle = self.filter_text.to_lowercase();
        self.filtered = self
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| needle.is_empty() || e.path.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn selected_event(&self) -> Option<&EventRecord> {
        self.filtered
            .get(self.selected)
            .and_then(|&i| self.events.get(i))
    }

    pub fn next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    pub fn previous(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = if self.selected == 0 {
                self.filtered.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    /// Re-hashes the selected event's path fresh and writes it into
    /// SigilWard's real baseline, then flags the event reviewed — "I saw
    /// this and it's fine" as a single key, without touching any other
    /// unreviewed drift the way a blanket `sigilward update` would.
    pub fn accept_selected(&mut self) {
        let Some(event) = self.selected_event().cloned() else {
            return;
        };
        match crate::baseline::accept(&self.baseline_path, &event.path) {
            Ok(()) => {
                if let Err(e) = events::mark_accepted(&self.log_path, &event.path, event.at) {
                    self.status = Some(format!("accepted, but failed to update log: {e}"));
                } else {
                    self.status = Some(format!("accepted: {}", event.path));
                }
                self.reload();
            }
            Err(e) => self.status = Some(format!("accept failed: {e}")),
        }
    }
}

pub fn detail_lines(record: &EventRecord) -> String {
    let mut lines = vec![
        format!("path:   {}", record.path),
        format!("when:   {}", record.at.to_rfc3339()),
    ];
    match &record.change {
        Change::New => lines.push("kind:   new file (not previously baselined)".to_string()),
        Change::Deleted => lines.push("kind:   deleted (was in baseline, now gone)".to_string()),
        Change::Modified {
            content_changed,
            mode_changed,
            owner_changed,
        } => {
            lines.push("kind:   modified".to_string());
            lines.push(format!("  content changed:   {content_changed}"));
            lines.push(format!("  permissions changed: {mode_changed}"));
            lines.push(format!("  ownership changed:   {owner_changed}"));
        }
        Change::Unchanged => lines.push("kind:   unchanged".to_string()),
    }
    lines.push(format!(
        "reviewed: {}",
        if record.accepted {
            "yes (accepted into baseline)"
        } else {
            "no"
        }
    ));
    lines.join("\n")
}
