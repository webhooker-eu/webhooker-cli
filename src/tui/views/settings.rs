use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::common;
use crate::tui::app::{App, Focus};
use crate::tui::forms::settings_form::SettingsField;
use crate::tui::theme::Tone;

const LABEL_WIDTH: usize = 22;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let active = app.focus == Focus::Main;
    let block = common::pane(app, "Settings", false, active);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(form) = &app.settings_form else {
        return;
    };
    let focused_field = form.focused_field();
    let mut lines = Vec::new();
    for field in SettingsField::ALL {
        let focused = active && field == focused_field;
        let marker = if focused {
            format!("{} ", theme.glyphs.pointer)
        } else {
            "  ".to_string()
        };
        let value = if form.text_input(field).is_some() {
            form.value_label(field)
        } else {
            format!(
                "{} {} {}",
                theme.glyphs.previous,
                form.value_label(field),
                theme.glyphs.next
            )
        };
        let label_style = if focused {
            theme.title()
        } else {
            theme.fg(Tone::Muted)
        };
        lines.push(Line::from(vec![
            Span::styled(marker, theme.fg(Tone::Accent)),
            Span::styled(format!("{:<LABEL_WIDTH$}", field.label()), label_style),
            Span::styled(value, theme.fg(Tone::Text)),
        ]));
    }
    lines.push(Line::raw(""));
    if let Some(error) = &form.error {
        lines.push(Line::from(Span::styled(
            error.clone(),
            theme.fg(Tone::Danger),
        )));
    } else if form.is_dirty() {
        lines.push(Line::from(Span::styled(
            format!("Unsaved changes {} ctrl+s to save", theme.glyphs.separator),
            theme.fg(Tone::Warning),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    if let (true, Some(input)) = (active, form.text_input(focused_field)) {
        let row = SettingsField::ALL
            .iter()
            .position(|field| *field == focused_field)
            .unwrap_or(0) as u16;
        let column = inner.x + 2 + LABEL_WIDTH as u16 + input.cursor() as u16;
        if column < inner.x + inner.width && row < inner.height {
            frame.set_cursor_position((column, inner.y + row));
        }
    }
}
