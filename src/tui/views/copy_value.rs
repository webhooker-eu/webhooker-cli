use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::common;
use crate::tui::app::App;
use crate::tui::clipboard::CopyValue;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App, copy: &CopyValue) {
    let theme = &app.theme;
    let instruction = format!("Select the {} with the mouse and copy it:", copy.label);
    let widest = instruction.chars().count().max(copy.text.chars().count()) as u16;
    let width = (widest + 4).clamp(40, area.width.saturating_sub(4).max(40));
    let lines = vec![
        Line::from(Span::styled(instruction, theme.fg(Tone::Muted))),
        Line::raw(""),
        Line::from(Span::styled(copy.text.clone(), theme.fg(Tone::Accent))),
        Line::raw(""),
        Line::from(vec![
            Span::styled("esc", theme.fg(Tone::Accent)),
            Span::styled(" close", theme.fg(Tone::Muted)),
        ]),
    ];
    let popup = common::centered(area, width, lines.len() as u16 + 3);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(common::pane(app, "Copy", false, true)),
        popup,
    );
}
