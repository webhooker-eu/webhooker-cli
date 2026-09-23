use ratatui::layout::{Constraint, Rect};
use ratatui::text::Span;
use ratatui::widgets::{Cell, Row};
use ratatui::Frame;

use super::common;
use crate::tui::app::{App, Focus};
use crate::tui::status;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let block = common::pane(
        app,
        "Connections",
        app.data.connections.loading,
        app.focus == Focus::Main,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(connections) = &app.data.connections.value else {
        return;
    };
    if connections.is_empty() {
        common::empty(
            frame,
            inner,
            app,
            "No connections yet. Press n to create one.",
        );
        return;
    }
    let theme = &app.theme;
    let rows: Vec<Row> = connections
        .iter()
        .map(|connection| {
            let mut flags = Vec::new();
            if connection.has_filter() {
                flags.push("filter");
            }
            if connection.has_transformation() {
                flags.push("transform");
            }
            Row::new(vec![
                Cell::from(common::badge_span(
                    app,
                    status::enabled(connection.enabled, theme.glyphs),
                )),
                Cell::from(Span::styled(
                    format!(
                        "{} {} {}",
                        app.names.source(&connection.source_id),
                        theme.glyphs.arrow,
                        app.names.destination(&connection.destination_id)
                    ),
                    theme.fg(Tone::Text),
                )),
                Cell::from(Span::styled(
                    flags.join(&format!(" {} ", theme.glyphs.separator)),
                    theme.fg(Tone::Muted),
                )),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Fill(3),
        Constraint::Length(20),
    ];
    common::render_table(
        frame,
        inner,
        common::table(app, rows, &widths),
        app.cursors.connections,
    );
}
