use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::common;
use crate::tui::app::App;
use crate::tui::hints;
use crate::tui::theme::Tone;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let mut spans = vec![Span::raw(" ")];
    for (index, (key, label)) in hints::all_hints(app).into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                format!(" {} ", theme.glyphs.separator),
                theme.fg(Tone::Muted),
            ));
        }
        spans.push(Span::styled(key, theme.fg(Tone::Accent)));
        spans.push(Span::styled(format!(" {label}"), theme.fg(Tone::Muted)));
    }
    let status = status_text(app);
    let status_width = status
        .as_ref()
        .map_or(0, |(text, _)| text.chars().count() as u16 + 2);
    let [hints_area, status_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(status_width)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(spans)), hints_area);
    if let Some((text, tone)) = status {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(format!("{text} "), theme.fg(tone))))
                .alignment(Alignment::Right),
            status_area,
        );
    }
}

/// Toast, else rate limit, else offline, else stale, else the data's age.
pub fn status_text(app: &App) -> Option<(String, Tone)> {
    let separator = app.theme.glyphs.separator;
    if let Some(toast) = app.toasts.last() {
        return Some((toast.text.clone(), toast.tone));
    }
    if let Some(until) = app.rate_limited_until {
        let seconds = until.saturating_duration_since(app.now).as_secs().max(1);
        return Some((
            format!("rate limited {separator} resumes in {seconds}s"),
            Tone::Warning,
        ));
    }
    if app.poller.is_offline() {
        return Some(("offline".to_string(), Tone::Danger));
    }
    let (loaded_at, stale) = app.primary_status()?;
    if stale {
        return Some(("stale".to_string(), Tone::Warning));
    }
    loaded_at.map(|loaded_at| {
        (
            common::age(app.now.saturating_duration_since(loaded_at)),
            Tone::Muted,
        )
    })
}
