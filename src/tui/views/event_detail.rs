use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;

use super::{body, common, events};
use crate::tui::app::{App, Focus};
use crate::tui::events_state::EventPane;
use crate::tui::model::{Attempt, EventDetail};
use crate::tui::status;
use crate::tui::theme::Tone;

const WIDE_LAYOUT: u16 = 100;
const ATTEMPT_BODY_PREVIEW: usize = 120;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let title = app
        .data
        .event
        .value
        .as_ref()
        .map_or_else(|| "Event".to_string(), |event| event.public_id.clone());
    let block = common::pane(
        app,
        &title,
        app.data.event.loading,
        app.focus == Focus::Main,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(event) = &app.data.event.value else {
        return;
    };
    let [summary_area, panes_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(inner);
    render_summary(frame, summary_area, app, event);
    let (headers_area, body_area, deliveries_area) = if frame.area().width >= WIDE_LAYOUT {
        let [top, bottom] =
            Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)])
                .areas(panes_area);
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(top);
        (left, right, bottom)
    } else {
        let [headers, body_part, deliveries] = Layout::vertical([
            Constraint::Percentage(30),
            Constraint::Percentage(40),
            Constraint::Percentage(30),
        ])
        .areas(panes_area);
        (headers, body_part, deliveries)
    };
    render_headers(frame, headers_area, app, event);
    render_body(frame, body_area, app, event);
    render_deliveries(frame, deliveries_area, app, event);
}

fn render_summary(frame: &mut Frame, area: Rect, app: &App, event: &EventDetail) {
    let theme = &app.theme;
    let separator = format!(" {} ", theme.glyphs.separator);
    let time = |raw: &str| common::timestamp(raw, app.settings.time_format, app.wall_clock);
    let lines = vec![
        Line::from(vec![
            Span::styled(event.method.clone(), theme.fg(Tone::Accent)),
            Span::raw(" "),
            Span::styled(event.public_id.clone(), theme.title()),
            Span::styled(
                format!(
                    "{separator}{}{separator}{}",
                    app.names.source(&event.source_id),
                    time(&event.received_at)
                ),
                theme.fg(Tone::Muted),
            ),
        ]),
        Line::from(vec![
            common::badge_span(
                app,
                status::verification(&event.verification_status, theme.glyphs),
            ),
            Span::styled(
                format!(
                    " {}{separator}{}{separator}{}{separator}expires {}",
                    event.verification_status,
                    event.content_type.as_deref().unwrap_or("-"),
                    events::size_label(event.body_size),
                    time(&event.expires_at)
                ),
                theme.fg(Tone::Muted),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn pane_block(app: &App, title: &str, pane: EventPane) -> Block<'static> {
    let focused = app.focus == Focus::Main && app.event_screens.detail.pane == pane;
    common::pane(app, title, false, focused)
}

fn render_headers(frame: &mut Frame, area: Rect, app: &App, event: &EventDetail) {
    let theme = &app.theme;
    let view = &app.event_screens.detail;
    let title = match &view.header_filter {
        Some(filter) => format!("Headers {} /{filter}", theme.glyphs.separator),
        None => "Headers".to_string(),
    };
    let block = pane_block(app, &title, EventPane::Headers);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let (list_area, search_area) = match &view.header_search {
        Some(_) => {
            let [list, search] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
            (list, Some(search))
        }
        None => (inner, None),
    };
    let term = view.header_filter.as_deref().map(str::to_lowercase);
    let lines: Vec<Line> = event
        .sorted_headers()
        .into_iter()
        .filter(|(name, value)| {
            term.as_deref().is_none_or(|term| {
                name.to_lowercase().contains(term) || value.to_lowercase().contains(term)
            })
        })
        .map(|(name, value)| {
            Line::from(vec![
                Span::styled(name, theme.fg(Tone::Accent)),
                Span::styled(": ", theme.fg(Tone::Muted)),
                Span::styled(value, theme.fg(Tone::Text)),
            ])
        })
        .collect();
    let limit = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_sub(list_area.height);
    view.headers_limit.set(limit);
    frame.render_widget(
        Paragraph::new(lines).scroll((view.headers_scroll.min(limit), 0)),
        list_area,
    );
    if let (Some(search_area), Some(input)) = (search_area, &view.header_search) {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("/", theme.fg(Tone::Accent)),
                Span::styled(input.value().to_string(), theme.fg(Tone::Text)),
            ])),
            search_area,
        );
        let column = search_area.x + 1 + input.cursor() as u16;
        if column < search_area.x + search_area.width {
            frame.set_cursor_position((column, search_area.y));
        }
    }
}

fn render_body(frame: &mut Frame, area: Rect, app: &App, event: &EventDetail) {
    let view = &app.event_screens.detail;
    let title = if view.wrap {
        format!("Body {} wrap", app.theme.glyphs.separator)
    } else {
        "Body".to_string()
    };
    let block = pane_block(app, &title, EventPane::Body);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = body::body_lines(app, event);
    let limit = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_sub(inner.height);
    view.body_limit.set(limit);
    let mut paragraph = Paragraph::new(lines).scroll((view.body_scroll.min(limit), 0));
    if view.wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    frame.render_widget(paragraph, inner);
}

fn render_deliveries(frame: &mut Frame, area: Rect, app: &App, event: &EventDetail) {
    let theme = &app.theme;
    let glyphs = theme.glyphs;
    let view = &app.event_screens.detail;
    let block = pane_block(app, "Deliveries", EventPane::Deliveries);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if event.deliveries.is_empty() {
        common::empty(frame, inner, app, "No deliveries.");
        return;
    }
    let focused = app.focus == Focus::Main && view.pane == EventPane::Deliveries;
    let mut lines = Vec::new();
    let mut cursor_line = 0;
    for (index, delivery) in event.deliveries.iter().enumerate() {
        let selected = index == view.delivery_cursor;
        if selected {
            cursor_line = lines.len();
        }
        let pointer = if selected && focused {
            format!("{} ", glyphs.pointer)
        } else {
            "  ".to_string()
        };
        let attempts = if delivery.attempt_count == 1 {
            "1 attempt".to_string()
        } else {
            format!("{} attempts", delivery.attempt_count)
        };
        lines.push(Line::from(vec![
            Span::styled(pointer, theme.fg(Tone::Accent)),
            common::badge_span(
                app,
                status::delivery(&delivery.status, glyphs, app.tick_count),
            ),
            Span::raw(" "),
            Span::styled(delivery.destination_name.clone(), theme.fg(Tone::Text)),
            Span::styled(
                format!("  {}  {attempts}", delivery.status),
                theme.fg(Tone::Muted),
            ),
        ]));
        if view.expanded.as_deref() == Some(delivery.id.as_str()) {
            for attempt in &delivery.attempts {
                lines.extend(attempt_lines(app, attempt));
            }
        }
    }
    let offset = u16::try_from(cursor_line)
        .unwrap_or(u16::MAX)
        .saturating_sub(inner.height.saturating_sub(1));
    frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
}

fn attempt_lines(app: &App, attempt: &Attempt) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let glyphs = theme.glyphs;
    let outcome = match attempt.response_status {
        Some(code) => {
            let tone = if (200..300).contains(&code) {
                Tone::Success
            } else {
                Tone::Danger
            };
            Span::styled(format!("{} {code}", glyphs.arrow), theme.fg(tone))
        }
        None => Span::styled(glyphs.failure, theme.fg(Tone::Danger)),
    };
    let mut first = vec![
        Span::styled(
            format!("      #{}  ", attempt.attempt_number),
            theme.fg(Tone::Muted),
        ),
        outcome,
        Span::styled(
            format!(
                "  {}ms  {}",
                attempt.latency_ms,
                common::timestamp(
                    &attempt.attempted_at,
                    app.settings.time_format,
                    app.wall_clock
                )
            ),
            theme.fg(Tone::Muted),
        ),
    ];
    if let Some(error) = &attempt.error_message {
        first.push(Span::styled(format!("  {error}"), theme.fg(Tone::Danger)));
    }
    let mut lines = vec![
        Line::from(first),
        Line::from(Span::styled(
            format!("        {}", attempt.request_url),
            theme.fg(Tone::Muted),
        )),
    ];
    if let Some(response) = attempt
        .response_body
        .as_deref()
        .filter(|body| !body.is_empty())
    {
        let preview: String = response
            .chars()
            .take(ATTEMPT_BODY_PREVIEW)
            .map(|character| {
                if character == '\n' || character == '\r' {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        lines.push(Line::from(Span::styled(
            format!("        {preview}"),
            theme.fg(Tone::Text),
        )));
    }
    lines
}
