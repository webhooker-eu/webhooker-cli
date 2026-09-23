use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::common;
use crate::tui::app::App;
use crate::tui::theme::Tone;

const LABEL_WIDTH: u16 = 10;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let form = &app.login;
    let popup = common::centered(area, 64, 10);
    let block = common::pane(app, "Log in", form.submitting, true);
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    let status = match (&form.error, form.submitting) {
        (Some(error), _) => Line::from(Span::styled(error.clone(), theme.fg(Tone::Danger))),
        (None, true) => Line::from(Span::styled(
            format!("Checking the key{}", theme.glyphs.ellipsis),
            theme.fg(Tone::Muted),
        )),
        (None, false) => Line::raw(""),
    };
    let lines = vec![
        Line::from(Span::styled(
            format!(
                "Paste an API key (whk_{}) to connect this terminal.",
                theme.glyphs.ellipsis
            ),
            theme.fg(Tone::Muted),
        )),
        Line::raw(""),
        input_line(
            app,
            "API key",
            form.api_key.display(theme.glyphs.mask),
            !form.server_focused,
        ),
        input_line(
            app,
            "Server",
            form.server.value().to_string(),
            form.server_focused,
        ),
        Line::raw(""),
        status,
    ];
    frame.render_widget(Paragraph::new(lines), inner);

    let (input, row) = if form.server_focused {
        (&form.server, 3)
    } else {
        (&form.api_key, 2)
    };
    let column = inner.x + LABEL_WIDTH + input.cursor() as u16;
    if column < inner.x + inner.width {
        frame.set_cursor_position((column, inner.y + row));
    }
}

fn input_line(app: &App, label: &str, value: String, focused: bool) -> Line<'static> {
    let theme = &app.theme;
    let label_style = if focused {
        theme.title()
    } else {
        theme.fg(Tone::Muted)
    };
    Line::from(vec![
        Span::styled(
            format!("{label:<width$}", width = LABEL_WIDTH as usize),
            label_style,
        ),
        Span::styled(value, theme.fg(Tone::Text)),
    ])
}
