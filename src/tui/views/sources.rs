use std::collections::HashMap;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row};
use ratatui::Frame;

use super::common;
use crate::tui::app::{App, Focus};
use crate::tui::forms::input::TextInput;
use crate::tui::status;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let title = match &app.source_query {
        Some(query) => format!("Sources {} /{query}", theme.glyphs.separator),
        None => "Sources".to_string(),
    };
    let block = common::pane(
        app,
        &title,
        app.data.sources.loading,
        app.focus == Focus::Main,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let list_area = match &app.source_search {
        Some(input) => {
            let [list, search] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
            render_search(frame, search, app, input);
            list
        }
        None => inner,
    };
    let Some(sources) = &app.data.sources.value else {
        return;
    };
    if sources.is_empty() {
        let message = match &app.source_query {
            Some(query) => format!("No sources match \"{query}\"."),
            None => "No sources yet. Press n to create one.".to_string(),
        };
        common::empty(frame, list_area, app, &message);
        return;
    }
    let counts = connection_counts(app);
    let rows: Vec<Row> = sources
        .iter()
        .map(|source| {
            let connections = counts.get(source.id.as_str()).copied().unwrap_or(0);
            let connections_label = if connections == 1 {
                "1 conn".to_string()
            } else {
                format!("{connections} conns")
            };
            let detail = if source.status == "paused" {
                "paused".to_string()
            } else {
                format!("verify: {}", source.verification_provider())
            };
            Row::new(vec![
                Cell::from(common::badge_span(
                    app,
                    status::resource_status(&source.status, theme.glyphs),
                )),
                Cell::from(Span::styled(source.name.clone(), theme.fg(Tone::Text))),
                Cell::from(
                    Line::from(Span::styled(
                        format!(
                            "{} events",
                            common::group_thousands(source.event_count.unwrap_or(0))
                        ),
                        theme.fg(Tone::Muted),
                    ))
                    .alignment(Alignment::Right),
                ),
                Cell::from(Span::styled(connections_label, theme.fg(Tone::Muted))),
                Cell::from(Span::styled(detail, theme.fg(Tone::Muted))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Fill(2),
        Constraint::Length(14),
        Constraint::Length(8),
        Constraint::Fill(1),
    ];
    common::render_table(
        frame,
        list_area,
        common::table(app, rows, &widths),
        app.cursors.sources,
    );
}

fn connection_counts(app: &App) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for connection in app.data.connections.value.iter().flatten() {
        *counts.entry(connection.source_id.as_str()).or_insert(0) += 1;
    }
    counts
}

fn render_search(frame: &mut Frame, area: Rect, app: &App, input: &TextInput) {
    let theme = &app.theme;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("/", theme.fg(Tone::Accent)),
            Span::styled(input.value().to_string(), theme.fg(Tone::Text)),
        ])),
        area,
    );
    let column = area.x + 1 + input.cursor() as u16;
    if column < area.x + area.width {
        frame.set_cursor_position((column, area.y));
    }
}
