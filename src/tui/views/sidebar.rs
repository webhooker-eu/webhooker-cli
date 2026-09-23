use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::tui::app::{App, Focus};
use crate::tui::screen::Section;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let current = app.screen.section();
    let lines: Vec<Line> = Section::ALL
        .iter()
        .map(|section| {
            if Some(*section) == current {
                Line::from(vec![
                    Span::styled(format!("{} ", theme.glyphs.pointer), theme.fg(Tone::Accent)),
                    Span::styled(section.label(), theme.title()),
                ])
            } else {
                Line::from(Span::styled(
                    format!("  {}", section.label()),
                    theme.fg(Tone::Text),
                ))
            }
        })
        .collect();
    let border = if app.focus == Focus::Sidebar {
        Tone::Accent
    } else {
        Tone::Border
    };
    let block = Block::bordered()
        .border_set(theme.border_set())
        .border_style(theme.fg(border));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Under 100 columns the sidebar becomes one line of short labels.
pub fn render_strip(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let current = app.screen.section();
    let mut spans = vec![Span::raw(" ")];
    for (index, section) in Section::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if Some(*section) == current {
            theme.title().add_modifier(Modifier::UNDERLINED)
        } else if app.focus == Focus::Sidebar {
            theme.fg(Tone::Text)
        } else {
            theme.fg(Tone::Muted)
        };
        spans.push(Span::styled(section.short_label(), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
