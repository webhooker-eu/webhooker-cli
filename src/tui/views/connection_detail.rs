use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use serde_json::Value;

use super::common;
use crate::tui::app::App;
use crate::tui::model::Connection;
use crate::tui::status;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let block = common::pane(app, "Connection", app.data.connection.loading, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(connection) = &app.data.connection.value else {
        return;
    };
    let lines = lines(app, connection);
    let offset = common::scroll_offset(app, lines.len(), inner.height);
    frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
}

fn lines(app: &App, connection: &Connection) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let mut lines = vec![
        common::field(
            app,
            "Source",
            common::text(app, app.names.source(&connection.source_id)),
        ),
        common::field(
            app,
            "Destination",
            common::text(app, app.names.destination(&connection.destination_id)),
        ),
        common::field(
            app,
            "Enabled",
            vec![
                common::badge_span(app, status::enabled(connection.enabled, theme.glyphs)),
                Span::styled(
                    if connection.enabled { " yes" } else { " no" },
                    theme.fg(Tone::Text),
                ),
            ],
        ),
        common::field(
            app,
            "Created",
            common::text(
                app,
                common::timestamp(
                    &connection.created_at,
                    app.settings.time_format,
                    app.wall_clock,
                ),
            ),
        ),
        common::field(app, "ID", common::text(app, connection.id.clone())),
    ];
    for (title, value) in [
        ("Filter rules", &connection.filter_rules),
        ("Transformation", &connection.transformation),
    ] {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(title, theme.title())));
        lines.extend(json_or_none(app, value));
    }
    lines
}

fn json_or_none(app: &App, value: &Value) -> Vec<Line<'static>> {
    if value.is_null() {
        vec![Line::from(Span::styled(
            "  none",
            app.theme.fg(Tone::Muted),
        ))]
    } else {
        common::pretty_json(app, value)
    }
}
