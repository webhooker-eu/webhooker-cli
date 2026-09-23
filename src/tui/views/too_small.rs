use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let line_area = Rect::new(area.x, area.y + area.height / 2, area.width, 1);
    frame.render_widget(
        Paragraph::new("Please enlarge the terminal window")
            .alignment(Alignment::Center)
            .style(app.theme.fg(Tone::Warning)),
        line_area,
    );
}
