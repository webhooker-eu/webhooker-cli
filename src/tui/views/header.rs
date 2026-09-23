use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::app::{App, KeySource};
use crate::tui::model::host_of;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App, compact: bool) {
    let theme = &app.theme;
    let version = env!("CARGO_PKG_VERSION");
    let separator = format!(" {} ", theme.glyphs.separator);
    let (name, plan) = match &app.session.workspace {
        Some(workspace) => (workspace.name.clone(), workspace.plan.clone()),
        None => (
            theme.glyphs.ellipsis.to_string(),
            theme.glyphs.ellipsis.to_string(),
        ),
    };
    let lines = if compact {
        vec![Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("Webhooker v{version}"), theme.title()),
            Span::styled(
                format!("{separator}{name}{separator}{plan}"),
                theme.fg(Tone::Muted),
            ),
        ])]
    } else {
        let mut context = format!(
            "{name}{separator}{plan}{separator}{}",
            host_of(&app.session.server)
        );
        if app.session.key_source == KeySource::Override {
            context.push_str(&format!("{separator}key: env"));
        }
        vec![
            Line::from(vec![
                Span::raw("  "),
                Span::styled(theme.logo[0], theme.fg(Tone::Accent)),
                Span::raw("   "),
                Span::styled(
                    format!("Webhooker CLI v{version}"),
                    theme.fg(Tone::Text).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::raw("  "),
                Span::styled(theme.logo[1], theme.fg(Tone::Accent)),
                Span::raw("   "),
                Span::styled(context, theme.fg(Tone::Muted)),
            ]),
        ]
    };
    frame.render_widget(Paragraph::new(lines), area);
    // The relay indicator sits on the header's last row, right-aligned.
    if let Some(indicator) = super::relay::indicator(app) {
        let width = (indicator.width() as u16).min(area.width);
        let row = area.y + area.height.saturating_sub(1);
        let column = (area.x + area.width).saturating_sub(width + 1).max(area.x);
        frame.render_widget(Paragraph::new(indicator), Rect::new(column, row, width, 1));
    }
}
