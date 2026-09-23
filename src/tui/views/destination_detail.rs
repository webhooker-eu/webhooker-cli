use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use serde_json::Value;

use super::common;
use crate::tui::app::App;
use crate::tui::model::{format_intervals, Destination};
use crate::tui::status;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let title = app.data.destination.value.as_ref().map_or_else(
        || "Destination".to_string(),
        |destination| destination.name.clone(),
    );
    let block = common::pane(app, &title, app.data.destination.loading, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(destination) = &app.data.destination.value else {
        return;
    };
    let lines = lines(app, destination);
    let offset = common::scroll_offset(app, lines.len(), inner.height);
    frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
}

fn lines(app: &App, destination: &Destination) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let separator = theme.glyphs.separator;
    let mut lines = vec![
        common::field(
            app,
            "URL",
            vec![Span::styled(
                destination.url.clone(),
                theme.fg(Tone::Accent),
            )],
        ),
        common::field(
            app,
            "Status",
            vec![
                common::badge_span(
                    app,
                    status::resource_status(&destination.status, theme.glyphs),
                ),
                Span::styled(format!(" {}", destination.status), theme.fg(Tone::Text)),
            ],
        ),
    ];
    if destination.custom_headers.is_empty() {
        lines.push(common::field(app, "Headers", common::text(app, "-")));
    }
    for (index, (name, value)) in destination.custom_headers.iter().enumerate() {
        let label = if index == 0 { "Headers" } else { "" };
        lines.push(common::field(
            app,
            label,
            common::text(app, format!("{name}: {value}")),
        ));
    }
    lines.push(common::field(
        app,
        "Auth",
        common::text(app, auth_summary(destination)),
    ));
    lines.push(common::field(
        app,
        "Timeout",
        common::text(
            app,
            destination.timeout_ms.map_or_else(
                || "default".to_string(),
                |milliseconds| format!("{milliseconds} ms"),
            ),
        ),
    ));
    lines.push(common::field(
        app,
        "Retries",
        common::text(
            app,
            destination.retry_intervals().map_or_else(
                || "default".to_string(),
                |intervals| format_intervals(&intervals, separator),
            ),
        ),
    ));
    let mut circuit = common::circuit_spans(app, &destination.circuit_state);
    if let Some(reopen_at) = &destination.circuit_reopen_at {
        circuit.push(Span::styled(
            format!(
                " {separator} reopens {}",
                common::timestamp(reopen_at, app.settings.time_format, app.wall_clock)
            ),
            theme.fg(Tone::Muted),
        ));
    }
    lines.push(common::field(app, "Circuit", circuit));
    lines.push(common::field(
        app,
        "Created",
        common::text(
            app,
            common::timestamp(
                &destination.created_at,
                app.settings.time_format,
                app.wall_clock,
            ),
        ),
    ));
    lines.push(common::field(
        app,
        "ID",
        common::text(app, destination.id.clone()),
    ));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled("Fed by", theme.title())));
    let feeding: Vec<_> = app
        .data
        .connections
        .value
        .iter()
        .flatten()
        .filter(|connection| connection.destination_id == destination.id)
        .collect();
    if feeding.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No connections.",
            theme.fg(Tone::Muted),
        )));
    }
    for connection in feeding {
        lines.push(Line::from(vec![
            Span::raw("  "),
            common::badge_span(app, status::enabled(connection.enabled, theme.glyphs)),
            Span::raw(" "),
            Span::styled(
                app.names.source(&connection.source_id),
                theme.fg(Tone::Text),
            ),
        ]));
    }
    lines
}

/// Secrets come back from the API as `***`; they are never shown anyway.
fn auth_summary(destination: &Destination) -> String {
    let setting = |name: &str| {
        destination
            .auth_config
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or("-")
            .to_string()
    };
    match destination.auth_type() {
        "none" => "none".to_string(),
        "hmac" => "hmac (secret ***)".to_string(),
        "api_key" => format!("api_key (header {}, key ***)", setting("header")),
        "basic_auth" => format!("basic_auth (user {}, password ***)", setting("username")),
        other => other.to_string(),
    }
}
