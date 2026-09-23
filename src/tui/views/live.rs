use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row};
use ratatui::Frame;

use super::{common, events};
use crate::tui::app::App;
use crate::tui::events_state::TailStatus;
use crate::tui::status;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let glyphs = theme.glyphs;
    let live = &app.event_screens.live;
    let [status_area, list_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);

    let spinner = glyphs.spinner_frame(app.tick_count);
    let (state, tone) = match app.event_screens.tail.as_ref().map(|tail| &tail.status) {
        Some(TailStatus::Live) => (format!("{} live", glyphs.active), Tone::Success),
        Some(TailStatus::Connecting) | None => (
            format!("{spinner} connecting{}", glyphs.ellipsis),
            Tone::Muted,
        ),
        Some(TailStatus::Reconnecting { reason }) => {
            (format!("{spinner} reconnecting ({reason})"), Tone::Warning)
        }
        Some(TailStatus::Failed(message)) => {
            (format!("{} {message}", glyphs.failure), Tone::Danger)
        }
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(state, theme.fg(tone)),
            Span::styled(
                format!(
                    "   {} follow {}",
                    glyphs.separator,
                    if live.follow { "on" } else { "off" }
                ),
                theme.fg(Tone::Muted),
            ),
        ])),
        status_area,
    );

    if live.rows.is_empty() {
        let target = app
            .data
            .source
            .value
            .as_ref()
            .map_or_else(String::new, |source| {
                format!(" Send one to {}", source.ingest_url)
            });
        common::empty(
            frame,
            list_area,
            app,
            &format!("Waiting for webhooks{}{target}", glyphs.ellipsis),
        );
        return;
    }
    let rows: Vec<Row> = live
        .rows
        .iter()
        .map(|event| {
            Row::new(vec![
                Cell::from(Span::styled(
                    common::timestamp(&event.received_at, app.settings.time_format, app.wall_clock),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Span::styled(event.method.clone(), theme.fg(Tone::Accent))),
                Cell::from(Span::styled(event.public_id.clone(), theme.fg(Tone::Text))),
                Cell::from(Span::styled(
                    event
                        .content_type
                        .clone()
                        .unwrap_or_else(|| "-".to_string()),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Span::styled(
                    events::size_label(event.body_size),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(common::badge_span(
                    app,
                    status::verification(&event.verification_status, glyphs),
                )),
                Cell::from(events::counters(app, event)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(19),
        Constraint::Length(6),
        Constraint::Fill(2),
        Constraint::Fill(2),
        Constraint::Length(8),
        Constraint::Length(1),
        Constraint::Length(12),
    ];
    common::render_table(
        frame,
        list_area,
        common::table(app, rows, &widths),
        live.cursor,
    );
}
