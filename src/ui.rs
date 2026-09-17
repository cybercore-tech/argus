use crate::app::{self, App, Mode};
use crate::change::Change;
use crate::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row, Table, TableState, Wrap};

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    draw_table(frame, app, theme, chunks[0]);
    draw_status(frame, app, theme, chunks[1]);
    draw_footer(frame, theme, chunks[2]);

    match app.mode {
        Mode::Detail => draw_detail_popup(frame, area, app, theme),
        Mode::Help => draw_help_popup(frame, area, theme),
        _ => {}
    }
}

fn kind_label_and_color(change: &Change, theme: &Theme) -> (&'static str, ratatui::style::Color) {
    match change {
        Change::New => ("new", theme.cyan),
        Change::Deleted => ("deleted", theme.red),
        Change::Modified {
            content_changed, ..
        } if *content_changed => ("modified", theme.orange),
        Change::Modified { .. } => ("modified", theme.purple),
        Change::Unchanged => ("unchanged", theme.muted),
    }
}

fn draw_table(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let header = Row::new(vec!["", "When", "Path", "Change", "Reviewed"]).style(
        Style::default()
            .fg(theme.orange)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = app
        .filtered
        .iter()
        .map(|&i| {
            let e = &app.events[i];
            let flag = if e.accepted {
                Span::styled("●", Style::default().fg(theme.acid_green))
            } else {
                Span::styled("●", Style::default().fg(theme.red))
            };
            let (kind, kind_color) = kind_label_and_color(&e.change, theme);
            let reviewed = if e.accepted {
                Span::styled("yes", Style::default().fg(theme.acid_green))
            } else {
                Span::styled("no", Style::default().fg(theme.muted))
            };

            Row::new(vec![
                Line::from(flag),
                Line::from(Span::styled(
                    e.at.format("%Y-%m-%d %H:%M:%S").to_string(),
                    Style::default().fg(theme.muted),
                )),
                Line::from(Span::styled(
                    e.path.clone(),
                    Style::default().fg(theme.white),
                )),
                Line::from(Span::styled(kind, Style::default().fg(kind_color))),
                Line::from(reviewed),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Percentage(18),
        Constraint::Percentage(48),
        Constraint::Percentage(16),
        Constraint::Min(10),
    ];

    let title = format!(" argus — {} events ", app.events.len());
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(theme.hot_pink)
                .fg(theme.white)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.orange))
                .title(Span::styled(title, Style::default().fg(theme.orange))),
        );

    let mut state = TableState::default();
    state.select(if app.filtered.is_empty() {
        None
    } else {
        Some(app.selected)
    });
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_status(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let text = match &app.mode {
        Mode::Filter => format!("filter: {}_", app.filter_text),
        _ => app.status.clone().unwrap_or_default(),
    };
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(theme.cyan)),
        area,
    );
}

fn draw_footer(frame: &mut Frame, theme: &Theme, area: Rect) {
    let text =
        "j/k nav  enter/l details  a accept into baseline  /  filter  R reload  ? help  q quit";
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(theme.muted)),
        area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn draw_detail_popup(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let popup = centered_rect(80, 60, area);
    frame.render_widget(Clear, popup);
    let text = app
        .selected_event()
        .map(app::detail_lines)
        .unwrap_or_default();
    let block = Block::default()
        .style(Style::default().bg(theme.panel).fg(theme.white))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.orange))
        .title(Span::styled(
            " event detail (Esc to close) ",
            Style::default().fg(theme.orange),
        ));
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
        popup,
    );
}

fn draw_help_popup(frame: &mut Frame, area: Rect, theme: &Theme) {
    let popup = centered_rect(60, 55, area);
    frame.render_widget(Clear, popup);
    let lines = [
        "j/k, ↑/↓   move",
        "/          filter by path",
        "enter, l   show event detail",
        "a          accept: re-hash and write into SigilWard's baseline",
        "R          reload the event log from disk",
        "?          toggle this help",
        "q, Esc     quit (Esc closes a popup first)",
    ]
    .join("\n");
    let block = Block::default()
        .style(Style::default().bg(theme.panel).fg(theme.white))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.line))
        .title(Span::styled(
            " argus — keys ",
            Style::default().fg(theme.line),
        ));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}
