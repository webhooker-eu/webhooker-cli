//! The relay inspector and the header indicator.

use chrono::{DateTime, Utc};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Wrap};
use ratatui::Frame;

use super::{body, common};
use crate::relay::{body_bytes, forward_headers, RelayOutcome, RelayRecord, RESPONSE_BODY_LIMIT};
use crate::tui::app::{App, Focus};
use crate::tui::relay_control::{NO_SLOT_PREFIX, SLOT_HINT};
use crate::tui::relay_session::{body_size, size_label, RelayConnection, RelayState};
use crate::tui::settings::TimeFormat;
use crate::tui::theme::Tone;

const LOCAL_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "::1", "[::1]"];

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Main;
    let Some(relay) = &app.relay else {
        let block = common::pane(app, "Relay", false, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        common::empty(
            frame,
            inner,
            app,
            "No relay running. Press n to start one, or L on a source.",
        );
        return;
    };
    let theme = &app.theme;
    let separator = theme.glyphs.separator;
    let title = format!(
        "Relay {separator} {} {} {}",
        relay.source_name, theme.glyphs.arrow, relay.target_url
    );
    let counters = format!(" {} fwd {separator} {} err ", relay.forwarded, relay.errors);
    let block = common::pane(app, &title, false, focused)
        .title_top(Line::from(Span::styled(counters, theme.fg(Tone::Muted))).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [status_area, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    render_connection(frame, status_area, app, relay);
    if relay.expanded {
        render_exchange(frame, content, app, relay);
        return;
    }
    let [list_area, exchange_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Percentage(45)]).areas(content);
    render_records(frame, list_area, app, relay);
    render_exchange(frame, exchange_area, app, relay);
}

fn render_connection(frame: &mut Frame, area: Rect, app: &App, relay: &RelayState) {
    let glyphs = app.theme.glyphs;
    let spinner = glyphs.spinner_frame(app.tick_count);
    let (glyph, tone, text) = match &relay.connection {
        RelayConnection::Connecting => (spinner, Tone::Accent, "connecting".to_string()),
        RelayConnection::Connected => (glyphs.active, Tone::Success, "connected".to_string()),
        RelayConnection::Reconnecting {
            reason,
            server_message,
        } => {
            let text = match server_message {
                Some(message) if reason.starts_with(NO_SLOT_PREFIX) => {
                    format!("{message}. {SLOT_HINT}")
                }
                _ => format!("reconnecting: {reason}"),
            };
            (spinner, Tone::Warning, text)
        }
        RelayConnection::Failed(message) => {
            (glyphs.failure, Tone::Danger, format!("stopped: {message}"))
        }
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(glyph, app.theme.fg(tone)),
            Span::styled(format!(" {text}"), app.theme.fg(tone)),
        ])),
        area,
    );
}

fn render_records(frame: &mut Frame, area: Rect, app: &App, relay: &RelayState) {
    let theme = &app.theme;
    let list_area = match &relay.search {
        Some(input) => {
            let [list, search] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("/", theme.fg(Tone::Accent)),
                    Span::styled(input.value().to_string(), theme.fg(Tone::Text)),
                ])),
                search,
            );
            let column = search.x + 1 + input.cursor() as u16;
            if column < search.x + search.width {
                frame.set_cursor_position((column, search.y));
            }
            list
        }
        None => area,
    };
    let visible = relay.visible();
    if visible.is_empty() {
        let message = match &relay.query {
            Some(query) => format!("No records match /{query}"),
            None => format!("Waiting for webhooks{}", theme.glyphs.ellipsis),
        };
        common::empty(frame, list_area, app, &message);
        return;
    }
    let rows: Vec<Row> = visible
        .iter()
        .map(|record| {
            Row::new(vec![
                Cell::from(Span::styled(
                    clock(&record.frame.received_at, app),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Span::styled(
                    record.frame.method.clone(),
                    theme.fg(Tone::Text),
                )),
                Cell::from(Span::styled(
                    short_public_id(&record.frame.public_id, theme.glyphs.ellipsis),
                    theme.fg(Tone::Text),
                )),
                Cell::from(Line::from(outcome_spans(app, record))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Fill(1),
    ];
    common::render_table(
        frame,
        list_area,
        common::table(app, rows, &widths),
        relay.cursor,
    );
}

fn outcome_spans(app: &App, record: &RelayRecord) -> Vec<Span<'static>> {
    let theme = &app.theme;
    let glyphs = theme.glyphs;
    match &record.outcome {
        RelayOutcome::Forwarded(response) => {
            vec![
                Span::styled(
                    format!("{} {}", glyphs.arrow, response.status),
                    theme.fg(status_tone(response.status)),
                ),
                Span::styled(
                    format!(
                        "  {:>5}ms  {:>8}",
                        response.elapsed.as_millis(),
                        size_label(body_size(record))
                    ),
                    theme.fg(Tone::Muted),
                ),
            ]
        }
        RelayOutcome::Failed(error) => vec![Span::styled(
            format!("{} {error}", glyphs.failure),
            theme.fg(Tone::Danger),
        )],
        RelayOutcome::Dropped => vec![Span::styled(
            format!("{} dropped (queue full)", glyphs.failure),
            theme.fg(Tone::Warning),
        )],
        RelayOutcome::SkippedUnverified => vec![Span::styled(
            format!("{} skipped (signature invalid)", glyphs.skipped),
            theme.fg(Tone::Muted),
        )],
    }
}

fn status_tone(status: u16) -> Tone {
    if status < 400 {
        Tone::Success
    } else {
        Tone::Danger
    }
}

fn render_exchange(frame: &mut Frame, area: Rect, app: &App, relay: &RelayState) {
    let Some(record) = relay.selected() else {
        common::empty(
            frame,
            area,
            app,
            "Select a record to see its request and response.",
        );
        return;
    };
    let [request_area, response_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    let border = app.theme.border_set();
    let block = |title: &'static str| {
        Block::bordered()
            .border_set(border)
            .border_style(app.theme.fg(Tone::Border))
            .title(Span::styled(format!(" {title} "), app.theme.title()))
    };
    frame.render_widget(
        Paragraph::new(request_lines(app, relay, record))
            .wrap(Wrap { trim: false })
            .block(block("Request")),
        request_area,
    );
    frame.render_widget(
        Paragraph::new(response_lines(app, record))
            .wrap(Wrap { trim: false })
            .block(block("Response")),
        response_area,
    );
}

fn request_lines(app: &App, relay: &RelayState, record: &RelayRecord) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let mut lines = vec![Line::from(Span::styled(
        format!("{} {}", record.frame.method, record.target_url),
        theme.fg(Tone::Accent),
    ))];
    let mut headers = forward_headers(&record.frame, &relay.extra_headers);
    headers.sort();
    lines.extend(header_lines(app, &headers));
    lines.push(Line::raw(""));
    let body = body_bytes(&record.frame).unwrap_or_else(|_| record.frame.body.clone().into_bytes());
    lines.extend(body_lines(app, &body, false));
    lines
}

fn response_lines(app: &App, record: &RelayRecord) -> Vec<Line<'static>> {
    let theme = &app.theme;
    match &record.outcome {
        RelayOutcome::Forwarded(response) => {
            let reason = reqwest::StatusCode::from_u16(response.status)
                .ok()
                .and_then(|status| status.canonical_reason())
                .unwrap_or("");
            let mut lines = vec![Line::from(vec![
                Span::styled(
                    format!("{} {reason}", response.status),
                    theme.fg(status_tone(response.status)),
                ),
                Span::styled(
                    format!("  {}ms", response.elapsed.as_millis()),
                    theme.fg(Tone::Muted),
                ),
            ])];
            let mut headers = response.headers.clone();
            headers.sort();
            lines.extend(header_lines(app, &headers));
            lines.push(Line::raw(""));
            lines.extend(body_lines(app, &response.body, response.body_truncated));
            lines
        }
        RelayOutcome::Failed(error) => {
            vec![Line::from(Span::styled(
                error.clone(),
                theme.fg(Tone::Danger),
            ))]
        }
        RelayOutcome::Dropped => vec![Line::from(Span::styled(
            "Not sent: the forward queue was full.",
            theme.fg(Tone::Warning),
        ))],
        RelayOutcome::SkippedUnverified => vec![Line::from(Span::styled(
            "Not sent: signature verification failed.",
            theme.fg(Tone::Muted),
        ))],
    }
}

fn header_lines(app: &App, headers: &[(String, String)]) -> Vec<Line<'static>> {
    headers
        .iter()
        .map(|(name, value)| {
            Line::from(vec![
                Span::styled(format!("{name}: "), app.theme.fg(Tone::Muted)),
                Span::styled(value.clone(), app.theme.fg(Tone::Text)),
            ])
        })
        .collect()
}

fn body_lines(app: &App, body_content: &[u8], truncated: bool) -> Vec<Line<'static>> {
    let muted = app.theme.fg(Tone::Muted);
    let mut lines = if body_content.is_empty() {
        vec![Line::from(Span::styled("(empty body)", muted))]
    } else {
        match std::str::from_utf8(body_content) {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(text) {
                Ok(json) if json.is_object() || json.is_array() => body::json_lines(app, &json),
                _ => text
                    .lines()
                    .map(|line| {
                        Line::from(Span::styled(line.to_string(), app.theme.fg(Tone::Text)))
                    })
                    .collect(),
            },
            Err(_) => vec![Line::from(Span::styled(
                format!("binary body, {} bytes", body_content.len()),
                muted,
            ))],
        }
    };
    if truncated {
        lines.push(Line::from(Span::styled(
            format!("(truncated at {} KB)", RESPONSE_BODY_LIMIT / 1024),
            muted,
        )));
    }
    lines
}

fn clock(raw: &str, app: &App) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(raw) else {
        return raw.to_string();
    };
    match app.settings.time_format {
        TimeFormat::Utc => parsed.with_timezone(&Utc).format("%H:%M:%S").to_string(),
        TimeFormat::Local => parsed
            .with_timezone(&chrono::Local)
            .format("%H:%M:%S")
            .to_string(),
        TimeFormat::Relative => common::age(
            (app.wall_clock - parsed.with_timezone(&Utc))
                .to_std()
                .unwrap_or_default(),
        ),
    }
}

/// `evt_8f2a…`
fn short_public_id(public_id: &str, ellipsis: &str) -> String {
    if public_id.chars().count() <= 9 {
        return public_id.to_string();
    }
    let prefix: String = public_id.chars().take(8).collect();
    format!("{prefix}{ellipsis}")
}

/// `:3000` for a local target, the host otherwise.
pub fn short_target(url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    let host = parsed.host_str().unwrap_or_default();
    if LOCAL_HOSTS.contains(&host) {
        return parsed
            .port_or_known_default()
            .map_or_else(|| host.to_string(), |port| format!(":{port}"));
    }
    crate::tui::model::host_of(url)
}

/// `⇄ relay stripe-prod → :3000 ●`, or `None` without a session.
pub fn indicator(app: &App) -> Option<Line<'static>> {
    let relay = app.relay.as_ref()?;
    let theme = &app.theme;
    let glyphs = theme.glyphs;
    let (glyph, tone) = match relay.connection {
        RelayConnection::Connected => (glyphs.active, Tone::Success),
        RelayConnection::Connecting | RelayConnection::Reconnecting { .. } => {
            (glyphs.spinner_frame(app.tick_count), Tone::Accent)
        }
        RelayConnection::Failed(_) => (glyphs.failure, Tone::Danger),
    };
    Some(Line::from(vec![
        Span::styled(
            format!(
                "{} relay {} {} {} ",
                glyphs.relay,
                relay.source_name,
                glyphs.arrow,
                short_target(&relay.target_url)
            ),
            theme.fg(Tone::Muted),
        ),
        Span::styled(glyph, theme.fg(tone)),
    ]))
}
