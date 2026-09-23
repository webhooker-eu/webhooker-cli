use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row};
use ratatui::Frame;

use super::common;
use crate::tui::app::{App, Focus};
use crate::tui::events_state::{DlqPane, TimeWindow};
use crate::tui::status;
use crate::tui::theme::Tone;

/// The DLQ screen and a source's DLQ tab: the summary per connection on
/// top, the selected connection's dead-lettered deliveries below.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let separator = theme.glyphs.separator;
    let dlq = &app.event_screens.dlq;
    let focused = app.focus == Focus::Main;
    let summaries = app.data.dlq_summary.value.as_ref().map(|page| &page.items);
    let rows = summaries.map_or(0, Vec::len) as u16;
    let [summary_area, entries_area] = Layout::vertical([
        Constraint::Length((rows + 2).clamp(4, 9)),
        Constraint::Min(0),
    ])
    .areas(area);

    let source = app
        .dlq_source_id()
        .map_or_else(|| "no source".to_string(), |id| app.names.source(&id));
    let window = if dlq.window == TimeWindow::All {
        String::new()
    } else {
        format!(" {separator} {}", dlq.window.label())
    };
    let title = format!(
        "DLQ {separator} {source} {separator} {}{window}",
        dlq.statuses.join(",")
    );
    let block = common::pane(
        app,
        &title,
        app.data.dlq_summary.loading,
        focused && dlq.pane == DlqPane::Summary,
    );
    let inner = block.inner(summary_area);
    frame.render_widget(block, summary_area);
    match summaries {
        None => {}
        Some(items) if items.is_empty() => {
            common::empty(frame, inner, app, "Nothing in the dead-letter queue.")
        }
        Some(items) => {
            let table_rows: Vec<Row> = items
                .iter()
                .map(|summary| {
                    Row::new(vec![
                        Cell::from(Span::styled(
                            summary.destination_name.clone(),
                            theme.fg(Tone::Text),
                        )),
                        Cell::from(Span::styled(
                            format!("{} exhausted", summary.exhausted_count),
                            theme.fg(Tone::Danger),
                        )),
                        Cell::from(Span::styled(
                            format!("{} failed", summary.failed_count),
                            theme.fg(Tone::Warning),
                        )),
                    ])
                })
                .collect();
            let widths = [
                Constraint::Fill(1),
                Constraint::Length(14),
                Constraint::Length(12),
            ];
            common::render_table(
                frame,
                inner,
                common::table(app, table_rows, &widths),
                dlq.summary_cursor,
            );
        }
    }

    let selected = app.selected_dlq_summary();
    let entries_title = match selected {
        Some(summary) => format!("Deliveries {separator} {}", summary.destination_name),
        None => "Deliveries".to_string(),
    };
    let block = common::pane(
        app,
        &entries_title,
        app.data.dlq_entries.loading,
        focused && dlq.pane == DlqPane::Entries,
    );
    let inner = block.inner(entries_area);
    frame.render_widget(block, entries_area);
    if selected.is_none() || app.data.dlq_entries.value.is_none() {
        return;
    }
    let entries = app.selected_dlq_entries();
    if entries.is_empty() {
        common::empty(
            frame,
            inner,
            app,
            "Nothing dead-lettered for this connection with these filters.",
        );
        return;
    }
    let table_rows: Vec<Row> = entries
        .iter()
        .map(|entry| {
            Row::new(vec![
                Cell::from(Span::styled(
                    common::timestamp(&entry.updated_at, app.settings.time_format, app.wall_clock),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Line::from(vec![
                    common::badge_span(app, status::delivery(&entry.status, theme.glyphs, 0)),
                    Span::styled(format!(" {}", entry.status), theme.fg(Tone::Text)),
                ])),
                Cell::from(Span::styled(
                    format!("{} attempts", entry.attempt_count),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Span::styled(
                    entry
                        .last_response_status
                        .map_or_else(|| "-".to_string(), |code| format!("HTTP {code}")),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Span::styled(
                    entry.last_error.clone().unwrap_or_default(),
                    theme.fg(Tone::Danger),
                )),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(19),
        Constraint::Length(12),
        Constraint::Length(11),
        Constraint::Length(8),
        Constraint::Fill(1),
    ];
    common::render_table(
        frame,
        inner,
        common::table(app, table_rows, &widths),
        dlq.entries_cursor,
    );
}
