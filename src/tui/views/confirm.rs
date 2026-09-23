use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::common;
use crate::tui::app::{App, Confirm};
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App, confirm: &Confirm) {
    let theme = &app.theme;
    let mut question = vec![Span::styled(confirm.before.clone(), theme.fg(Tone::Text))];
    if let Some(target) = &confirm.target {
        question.push(Span::styled(
            target.clone(),
            theme.fg(Tone::Text).add_modifier(Modifier::BOLD),
        ));
    }
    question.push(Span::styled(confirm.after.clone(), theme.fg(Tone::Text)));
    let lines = vec![
        Line::from(question),
        Line::raw(""),
        Line::from(vec![
            Span::styled("y", theme.fg(Tone::Accent)),
            Span::styled(" yes   ", theme.fg(Tone::Muted)),
            Span::styled("n", theme.fg(Tone::Accent)),
            Span::styled(" no", theme.fg(Tone::Muted)),
        ]),
    ];
    let width =
        (confirm.text().chars().count() as u16 + 6).clamp(30, area.width.saturating_sub(4).max(30));
    let popup = common::centered(area, width, 5);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(common::pane(app, "Confirm", false, true)),
        popup,
    );
}
