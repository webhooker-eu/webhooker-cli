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
    let mut lines = vec![
        Line::from(question),
        Line::raw(""),
        Line::from(vec![
            Span::styled("y", theme.fg(Tone::Accent)),
            Span::styled(" yes   ", theme.fg(Tone::Muted)),
            Span::styled("n", theme.fg(Tone::Accent)),
            Span::styled(" no", theme.fg(Tone::Muted)),
        ]),
    ];
    if let Some(typed) = &confirm.typed_name {
        let input_style = if typed.mismatch {
            theme.fg(Tone::Danger)
        } else {
            theme.fg(Tone::Text)
        };
        lines.truncate(2);
        lines.push(Line::from(vec![
            Span::styled("Type ", theme.fg(Tone::Muted)),
            Span::styled(
                typed.expected.clone(),
                theme.fg(Tone::Text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to confirm:", theme.fg(Tone::Muted)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("> {}", typed.input.value()),
            input_style,
        )));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("enter", theme.fg(Tone::Accent)),
            Span::styled(" confirm   ", theme.fg(Tone::Muted)),
            Span::styled("esc", theme.fg(Tone::Accent)),
            Span::styled(" cancel", theme.fg(Tone::Muted)),
        ]));
    }
    let width =
        (confirm.text().chars().count() as u16 + 6).clamp(30, area.width.saturating_sub(4).max(30));
    let popup = common::centered(area, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(common::pane(app, "Confirm", false, true)),
        popup,
    );
}
