//! Pure rendering: `render` draws the whole frame from `&App`.

#[allow(dead_code)]
mod common;
mod confirm;
mod header;
mod help;
mod login;
mod placeholder;
mod settings;
mod sidebar;
mod status_line;
mod too_small;

#[cfg(test)]
mod snapshot_tests;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::screen::Screen;

pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 15;
pub const WIDE_LAYOUT: u16 = 100;
pub const TALL_HEADER: u16 = 20;
pub const SIDEBAR_WIDTH: u16 = 16;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        too_small::render(frame, area, app);
        return;
    }
    let compact = area.height < TALL_HEADER || app.settings.compact_header;
    let header_height = if compact { 1 } else { 2 };
    let [header_area, body, status_area] = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    header::render(frame, header_area, app, compact);
    render_body(frame, body, app);
    status_line::render(frame, status_area, app);
    if app.help_open {
        help::render(frame, area, app);
    }
    if let Some(confirm) = &app.confirm {
        confirm::render(frame, area, app, confirm);
    }
}

fn render_body(frame: &mut Frame, body: Rect, app: &App) {
    if app.screen == Screen::Login {
        login::render(frame, body, app);
        return;
    }
    let main = if body.width >= WIDE_LAYOUT {
        let [sidebar_area, main] =
            Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)]).areas(body);
        sidebar::render(frame, sidebar_area, app);
        main
    } else {
        let [strip, main] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
        sidebar::render_strip(frame, strip, app);
        main
    };
    render_screen(frame, main, app);
}

fn render_screen(frame: &mut Frame, area: Rect, app: &App) {
    match &app.screen {
        Screen::Settings => settings::render(frame, area, app),
        _ => placeholder::render(frame, area, app),
    }
}
