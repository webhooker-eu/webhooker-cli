use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Wrap};
use ratatui::Frame;

use super::{common, placeholder};
use crate::tui::app::App;
use crate::tui::model::host_of;
use crate::tui::screen::SourceTab;
use crate::tui::settings::Rgb;
use crate::tui::status;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App, tab: SourceTab) {
    let loading = app.data.source.loading || app.data.source_connections.loading;
    let title = app
        .data
        .source
        .value
        .as_ref()
        .map_or_else(|| "Source".to_string(), |source| source.name.clone());
    let block = common::pane(app, &title, loading, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [tabs_area, body] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(inner);
    render_tabs(frame, tabs_area, app, tab);
    match tab {
        SourceTab::Overview => render_overview(frame, body, app),
        SourceTab::Connections => render_connections(frame, body, app),
        SourceTab::Live | SourceTab::Events | SourceTab::Dlq => {
            placeholder::render_inline(frame, body, app, "This tab is not available yet.")
        }
    }
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App, active: SourceTab) {
    let theme = &app.theme;
    let mut spans = Vec::new();
    for (index, tab) in SourceTab::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("   "));
        }
        let label = format!("{} {}", index + 1, tab.label());
        spans.push(if *tab == active {
            Span::styled(label, theme.title().add_modifier(Modifier::UNDERLINED))
        } else {
            Span::styled(label, theme.fg(Tone::Muted))
        });
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_overview(frame: &mut Frame, area: Rect, app: &App) {
    let Some(source) = &app.data.source.value else {
        return;
    };
    let theme = &app.theme;
    let badge = status::resource_status(&source.status, theme.glyphs);
    let color = match (
        source.color.as_deref().and_then(Rgb::parse_hex),
        &source.color,
    ) {
        (Some(rgb), Some(raw)) => vec![
            Span::styled(
                if theme.ascii { "##" } else { "██" },
                Style::default().fg(theme.color(rgb)),
            ),
            Span::styled(format!(" {raw}"), theme.fg(Tone::Text)),
        ],
        _ => common::text(app, "-"),
    };
    let verification = match source.verification_provider() {
        "none" => "none".to_string(),
        provider if source.verification_config.get("secret").is_some() => {
            format!("{provider} (secret ***)")
        }
        provider => provider.to_string(),
    };
    let response = if source.response_config.is_null() {
        "default".to_string()
    } else {
        source.response_config.to_string()
    };
    let description = source
        .description
        .clone()
        .filter(|description| !description.is_empty())
        .unwrap_or_else(|| "-".to_string());
    let lines = vec![
        common::field(
            app,
            "Ingest URL",
            vec![Span::styled(
                source.ingest_url.clone(),
                theme.fg(Tone::Accent),
            )],
        ),
        common::field(
            app,
            "Status",
            vec![
                common::badge_span(app, badge),
                Span::styled(format!(" {}", source.status), theme.fg(Tone::Text)),
            ],
        ),
        common::field(app, "Description", common::text(app, description)),
        common::field(app, "Color", color),
        common::field(app, "Verification", common::text(app, verification)),
        common::field(app, "Response", common::text(app, response)),
        common::field(
            app,
            "Created",
            common::text(
                app,
                common::timestamp(&source.created_at, app.settings.time_format, app.wall_clock),
            ),
        ),
        common::field(app, "ID", common::text(app, source.id.clone())),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn render_connections(frame: &mut Frame, area: Rect, app: &App) {
    let Some(connections) = &app.data.source_connections.value else {
        return;
    };
    if connections.is_empty() {
        common::empty(frame, area, app, "Not connected to any destination.");
        return;
    }
    let theme = &app.theme;
    let rows: Vec<Row> = connections
        .iter()
        .map(|connection| {
            Row::new(vec![
                Cell::from(common::badge_span(
                    app,
                    status::enabled(connection.enabled, theme.glyphs),
                )),
                Cell::from(Span::styled(
                    connection.destination.name.clone(),
                    theme.fg(Tone::Text),
                )),
                Cell::from(Span::styled(
                    host_of(&connection.destination.url),
                    theme.fg(Tone::Muted),
                )),
                Cell::from(Line::from(common::circuit_spans(
                    app,
                    &connection.destination.circuit_state,
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
        area,
        common::table(app, rows, &widths),
        app.cursors.source_connections,
    );
}
