//! Contents of the hint bar and the help overlay. ASCII-safe on purpose.

use crate::tui::app::{App, Focus};
use crate::tui::screen::{Screen, SourceTab};

pub type Hint = (&'static str, &'static str);

pub fn hints(app: &App) -> Vec<Hint> {
    if app.confirm.is_some() {
        return vec![("y", "confirm"), ("n", "cancel")];
    }
    if app.help_open {
        return vec![("?", "close help")];
    }
    if app.modal.is_some() {
        return vec![("tab", "next field"), ("ctrl+s", "save"), ("esc", "cancel")];
    }
    if app.source_search.is_some() {
        return vec![("enter", "apply filter"), ("esc", "cancel")];
    }
    if app.focus == Focus::Sidebar && app.screen != Screen::Login {
        return vec![
            ("j/k", "section"),
            ("enter", "open"),
            ("g", "jump"),
            ("?", "help"),
            ("q", "quit"),
        ];
    }
    match &app.screen {
        Screen::Login => vec![
            ("enter", "log in"),
            ("tab", "next field"),
            ("ctrl+c", "quit"),
        ],
        Screen::Sources => vec![
            ("enter", "open"),
            ("/", "filter"),
            ("r", "refresh"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Screen::SourceDetail {
            tab: SourceTab::Connections,
            ..
        } => vec![
            ("enter", "open"),
            ("1-5", "tabs"),
            ("esc", "back"),
            ("?", "help"),
        ],
        Screen::SourceDetail {
            tab: SourceTab::Events,
            ..
        } => vec![
            ("enter", "open"),
            ("F", "filters"),
            ("[ ]", "pages"),
            ("1-5", "tabs"),
            ("esc", "back"),
        ],
        Screen::SourceDetail { .. } => vec![
            ("1-5", "tabs"),
            ("esc", "back"),
            ("r", "refresh"),
            ("?", "help"),
        ],
        Screen::Destinations | Screen::Connections => vec![
            ("enter", "open"),
            ("r", "refresh"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Screen::DestinationDetail { .. } | Screen::ConnectionDetail { .. } => vec![
            ("j/k", "scroll"),
            ("esc", "back"),
            ("r", "refresh"),
            ("?", "help"),
        ],
        Screen::Settings => vec![
            ("tab", "next field"),
            ("h/l", "change"),
            ("ctrl+s", "save"),
            ("esc", "back"),
        ],
        Screen::Events => vec![
            ("enter", "open"),
            ("F", "filters"),
            ("[ ]", "pages"),
            ("r", "refresh"),
            ("?", "help"),
        ],
        Screen::Dlq | Screen::Stats | Screen::Relay | Screen::EventDetail { .. } => {
            vec![("g", "jump"), ("?", "help"), ("q", "quit")]
        }
    }
}

pub fn help(screen: &Screen) -> Vec<(&'static str, Vec<Hint>)> {
    let mut sections = vec![(
        "Global",
        vec![
            ("j/k, arrows", "move"),
            ("enter", "open"),
            ("esc", "back"),
            ("tab", "next pane or tab"),
            ("r", "refresh now"),
            ("g + letter", "jump: s d c e q t r ,"),
            ("?", "this help"),
            ("q, ctrl+c", "quit"),
        ],
    )];
    let specific: Vec<Hint> = match screen {
        Screen::Sources => vec![("/", "filter by name")],
        Screen::SourceDetail { .. } => vec![("1-5", "switch tab"), ("h/l", "previous / next tab")],
        Screen::DestinationDetail { .. } | Screen::ConnectionDetail { .. } => vec![
            ("j/k", "scroll"),
            ("pgup/pgdn", "page"),
            ("g / G", "top / bottom"),
        ],
        Screen::Settings => vec![
            ("tab, shift+tab", "move between fields"),
            ("h/l, space", "change a choice"),
            ("ctrl+s", "save"),
            ("esc", "back"),
        ],
        Screen::Events => vec![
            ("F", "filters: source, verification, time, id"),
            ("[ / ]", "previous / next page"),
        ],
        _ => Vec::new(),
    };
    if !specific.is_empty() {
        sections.push(("This screen", specific));
    }
    sections
}
