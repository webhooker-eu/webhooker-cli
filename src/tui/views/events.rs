use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row};
use ratatui::Frame;

use super::common;
use crate::tui::app::{App, Focus};
use crate::tui::events_state::{EventsScope, EVENTS_PAGE_SIZE};
use crate::tui::model::EventSummary;
use crate::tui::status;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let block = common::pane(
        app,
        "Events",
        app.data.events.loading,
        app.focus == Focus::Main,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_list(frame, inner, app, EventsScope::Global);
}

/// Filters bar and rows; the source tab draws this inside its own pane.
pub fn render_list(frame: &mut Frame, area: Rect, app: &App, scope: EventsScope) {
    let theme = &app.theme;
    let separator = theme.glyphs.separator;
    let Some(filter) = app.visible_events_filter() else {
        return;
    };
    let state = app.event_screens.events(scope);
    let [bar_area, list_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);
    let total = app.data.events.value.as_ref().and_then(|page| page.total);
    let position = match total {
        Some(total) => format!(
            "page {}/{} {separator} {} events",
            state.page,
            ((total + EVENTS_PAGE_SIZE - 1) / EVENTS_PAGE_SIZE).max(1),
            common::group_thousands(total)
        ),
        None => format!("page {}", state.page),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                filter.describe(&app.names, separator),
                theme.fg(Tone::Muted),
            )),
            Line::from(Span::styled(position, theme.fg(Tone::Muted))),
        ]),
        bar_area,
    );

    let Some(page) = &app.data.events.value else {
        return;
    };
    if page.items.is_empty() {
        let message = if state.filter == Default::default() && state.page == 1 {
            "No events yet."
        } else {
            "No events match these filters."
        };
        common::empty(frame, list_area, app, message);
        return;
    }
    let with_source = scope == EventsScope::Global;
    let rows: Vec<Row> = page
        .items
        .iter()
        .map(|event| {
            let mut cells = vec![
                Cell::from(Span::styled(
                    common::timestamp(&event.received_at, app.settings.time_format, app.wall_clock),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Span::styled(event.method.clone(), theme.fg(Tone::Accent))),
                Cell::from(Span::styled(event.public_id.clone(), theme.fg(Tone::Text))),
            ];
            if with_source {
                cells.push(Cell::from(Span::styled(
                    app.names.source(&event.source_id),
                    theme.fg(Tone::Text),
                )));
            }
            cells.extend([
                Cell::from(Span::styled(
                    event
                        .content_type
                        .clone()
                        .unwrap_or_else(|| "-".to_string()),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Span::styled(
                    size_label(event.body_size),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(common::badge_span(
                    app,
                    status::verification(&event.verification_status, theme.glyphs),
                )),
                Cell::from(counters(app, event)),
            ]);
            Row::new(cells)
        })
        .collect();
    let mut widths = vec![
        Constraint::Length(19),
        Constraint::Length(6),
        Constraint::Fill(2),
    ];
    if with_source {
        widths.push(Constraint::Fill(2));
    }
    widths.extend([
        Constraint::Fill(2),
        Constraint::Length(8),
        Constraint::Length(1),
        Constraint::Length(12),
    ]);
    common::render_table(
        frame,
        list_area,
        common::table(app, rows, &widths),
        state.cursor,
    );
}

/// `✓2 ✕1 …1`; `…` alone while a live row has no counters yet.
pub fn counters(app: &App, event: &EventSummary) -> Line<'static> {
    let theme = &app.theme;
    let glyphs = theme.glyphs;
    let (Some(total), Some(delivered), Some(failed), Some(pending)) = (
        event.delivery_count,
        event.delivered_count,
        event.failed_count,
        event.pending_count,
    ) else {
        return Line::from(Span::styled(glyphs.ellipsis, theme.fg(Tone::Muted)));
    };
    if total == 0 {
        return Line::from(Span::styled("-", theme.fg(Tone::Muted)));
    }
    let mut spans = Vec::new();
    for (count, glyph, tone) in [
        (delivered, glyphs.success, Tone::Success),
        (failed, glyphs.failure, Tone::Danger),
        (pending, glyphs.ellipsis, Tone::Accent),
    ] {
        if count > 0 {
            if !spans.is_empty() {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(format!("{glyph}{count}"), theme.fg(tone)));
        }
    }
    Line::from(spans)
}

pub fn size_label(bytes: i64) -> String {
    match bytes {
        0..=1_023 => format!("{bytes} B"),
        1_024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1_024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(size_label(18), "18 B");
        assert_eq!(size_label(1_234), "1.2 KB");
        assert_eq!(size_label(3_145_728), "3.0 MB");
    }
}
