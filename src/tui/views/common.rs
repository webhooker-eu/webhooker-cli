use std::time::Duration;

use chrono::{DateTime, Utc};
use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Row, Table, TableState};
use ratatui::Frame;
use serde_json::Value;

use crate::tui::app::App;
use crate::tui::settings::TimeFormat;
use crate::tui::status::{self, Badge};
use crate::tui::theme::Tone;

pub const LABEL_WIDTH: usize = 14;

/// A bordered pane; the title carries a spinner while `loading`.
pub fn pane(app: &App, title: &str, loading: bool, focused: bool) -> Block<'static> {
    let theme = &app.theme;
    let mut spans = vec![Span::styled(format!(" {title} "), theme.title())];
    if loading {
        spans.push(Span::styled(
            format!("{} ", theme.glyphs.spinner_frame(app.tick_count)),
            theme.fg(Tone::Accent),
        ));
    }
    let border = if focused { Tone::Accent } else { Tone::Border };
    Block::bordered()
        .border_set(theme.border_set())
        .border_style(theme.fg(border))
        .title(Line::from(spans))
}

pub fn badge_span(app: &App, badge: Badge) -> Span<'static> {
    Span::styled(badge.glyph, app.theme.fg(badge.tone))
}

pub fn text(app: &App, value: impl Into<String>) -> Vec<Span<'static>> {
    vec![Span::styled(value.into(), app.theme.fg(Tone::Text))]
}

/// `label` in a fixed-width muted column, then the value.
pub fn field(app: &App, label: &str, value: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{label:<LABEL_WIDTH$}"),
        app.theme.fg(Tone::Muted),
    )];
    spans.extend(value);
    Line::from(spans)
}

/// "closed" in muted text, or the circuit glyph and state in its tone.
pub fn circuit_spans(app: &App, state: &str) -> Vec<Span<'static>> {
    let label = state.replace('_', " ");
    match status::circuit(state, app.theme.glyphs) {
        None => vec![Span::styled(label, app.theme.fg(Tone::Muted))],
        Some(badge) => vec![
            badge_span(app, badge),
            Span::styled(format!(" {label}"), app.theme.fg(badge.tone)),
        ],
    }
}

pub fn empty(frame: &mut Frame, area: Rect, app: &App, message: &str) {
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            message.to_string(),
            app.theme.fg(Tone::Muted),
        ))),
        area,
    );
}

/// A selectable table with the theme's selection style and pointer.
pub fn table<'a>(app: &App, rows: Vec<Row<'a>>, widths: &[Constraint]) -> Table<'a> {
    Table::new(rows, widths.to_vec())
        .column_spacing(2)
        .highlight_symbol(format!("{} ", app.theme.glyphs.pointer))
        .row_highlight_style(app.theme.selected())
}

pub fn render_table(frame: &mut Frame, area: Rect, table: Table<'_>, selected: usize) {
    let mut state = TableState::default().with_selected(Some(selected));
    frame.render_stateful_widget(table, area, &mut state);
}

/// The scroll offset to draw with, clamped to the content; also tells the
/// key handler how far `G` and `j` may go.
pub fn scroll_offset(app: &App, content_lines: usize, visible: u16) -> u16 {
    let limit = u16::try_from(content_lines)
        .unwrap_or(u16::MAX)
        .saturating_sub(visible);
    app.scroll_limit.set(limit);
    app.scroll.min(limit)
}

pub fn age(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..=59 => format!("{seconds}s ago"),
        60..=3_599 => format!("{}m ago", seconds / 60),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

pub fn timestamp(raw: &str, format: TimeFormat, now: DateTime<Utc>) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(raw) else {
        return raw.to_string();
    };
    let pattern = "%Y-%m-%d %H:%M:%S";
    match format {
        TimeFormat::Utc => parsed.with_timezone(&Utc).format(pattern).to_string(),
        TimeFormat::Local => parsed
            .with_timezone(&chrono::Local)
            .format(pattern)
            .to_string(),
        TimeFormat::Relative => age((now - parsed.with_timezone(&Utc))
            .to_std()
            .unwrap_or_default()),
    }
}

/// Pretty-printed JSON, indented by two spaces. Plan 3 adds syntax colors.
pub fn pretty_json(app: &App, value: &Value) -> Vec<Line<'static>> {
    serde_json::to_string_pretty(value)
        .unwrap_or_default()
        .lines()
        .map(|line| Line::from(Span::styled(format!("  {line}"), app.theme.fg(Tone::Text))))
        .collect()
}

/// `1 204`
pub fn group_thousands(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    if value < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn ages_and_timestamps() {
        assert_eq!(age(Duration::from_secs(3)), "3s ago");
        assert_eq!(age(Duration::from_secs(125)), "2m ago");
        assert_eq!(age(Duration::from_secs(7_200)), "2h ago");
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
        assert_eq!(
            timestamp("2026-09-23T11:58:00Z", TimeFormat::Utc, now),
            "2026-09-23 11:58:00"
        );
        assert_eq!(
            timestamp("2026-09-23T11:58:00Z", TimeFormat::Relative, now),
            "2m ago"
        );
        assert_eq!(timestamp("yesterday", TimeFormat::Utc, now), "yesterday");
    }

    #[test]
    fn thousands_are_grouped_with_spaces() {
        assert_eq!(group_thousands(12), "12");
        assert_eq!(group_thousands(1_204), "1 204");
        assert_eq!(group_thousands(1_234_567), "1 234 567");
        assert_eq!(group_thousands(-4_000), "-4 000");
    }

    #[test]
    fn centered_rects_fit_inside() {
        assert_eq!(
            centered(Rect::new(0, 0, 80, 24), 40, 10),
            Rect::new(20, 7, 40, 10)
        );
        assert_eq!(
            centered(Rect::new(0, 0, 30, 5), 40, 10),
            Rect::new(0, 0, 30, 5)
        );
    }
}
