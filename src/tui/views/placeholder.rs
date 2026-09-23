use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::common;
use crate::tui::app::{App, Focus};
use crate::tui::screen::Section;
use crate::tui::theme::Tone;

/// Sections that later plans fill in.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let title = app.screen.section().map_or("Webhooker", Section::label);
    let block = common::pane(app, title, false, app.focus == Focus::Main);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_inline(frame, inner, app, "This section is not available yet.");
}

pub fn render_inline(frame: &mut Frame, area: Rect, app: &App, message: &str) {
    frame.render_widget(
        Paragraph::new(message.to_string()).style(app.theme.fg(Tone::Muted)),
        area,
    );
}
