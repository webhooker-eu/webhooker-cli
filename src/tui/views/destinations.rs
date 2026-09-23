use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row};
use ratatui::Frame;

use super::common;
use crate::tui::app::{App, Focus};
use crate::tui::model::host_of;
use crate::tui::status;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let block = common::pane(
        app,
        "Destinations",
        app.data.destinations.loading,
        app.focus == Focus::Main,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(destinations) = &app.data.destinations.value else {
        return;
    };
    if destinations.is_empty() {
        common::empty(
            frame,
            inner,
            app,
            "No destinations yet. Press n to create one.",
        );
        return;
    }
    let theme = &app.theme;
    let rows: Vec<Row> = destinations
        .iter()
        .map(|destination| {
            Row::new(vec![
                Cell::from(common::badge_span(
                    app,
                    status::resource_status(&destination.status, theme.glyphs),
                )),
                Cell::from(Span::styled(destination.name.clone(), theme.fg(Tone::Text))),
                Cell::from(Span::styled(
                    host_of(&destination.url),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Line::from(common::circuit_spans(
                    app,
                    &destination.circuit_state,
                ))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Fill(2),
        Constraint::Fill(2),
        Constraint::Length(14),
    ];
    common::render_table(
        frame,
        inner,
        common::table(app, rows, &widths),
        app.cursors.destinations,
    );
}
