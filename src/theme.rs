//! Map cybercore CYBERGRID palette hex values into ratatui colors. Same
//! pattern as cybermeta's and cyberwatch's `src/theme.rs`.

use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub white: Color,
    pub acid_green: Color,
    pub hot_pink: Color,
    pub purple: Color,
    pub cyan: Color,
    pub orange: Color,
    pub red: Color,
    pub panel: Color,
    pub line: Color,
    pub muted: Color,
}

impl Theme {
    pub fn from_cybercore() -> Self {
        let schema = cybercore::schema::load();
        let p = &schema.palette;
        Self {
            bg: hex_color(&p.bg),
            white: hex_color(&p.white),
            acid_green: hex_color(&p.acid_green),
            hot_pink: hex_color(&p.hot_pink),
            purple: hex_color(&p.purple),
            cyan: hex_color(&p.cyan),
            orange: hex_color(&p.orange),
            red: hex_color(&p.red),
            panel: hex_color(&p.panel),
            line: hex_color(&p.line),
            muted: hex_color(&p.muted),
        }
    }
}

fn hex_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return Color::White;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
    Color::Rgb(r, g, b)
}
