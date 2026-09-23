use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::common;
use crate::tui::app::App;
use crate::tui::hints;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let mut lines = Vec::new();
    for (title, entries) in hints::help(&app.screen) {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(Span::styled(title, theme.title())));
        for (key, label) in entries {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<16}"), theme.fg(Tone::Accent)),
                Span::styled(label, theme.fg(Tone::Text)),
            ]));
        }
    }
    let popup = common::centered(area, 56, lines.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(common::pane(app, "Keys", false, true)),
        popup,
    );
}
