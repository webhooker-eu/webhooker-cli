//! Contents of the hint bar and the help overlay. ASCII-safe on purpose.

use crate::tui::app::{App, Focus};
use crate::tui::screen::{Screen, SourceTab};

pub type Hint = (&'static str, &'static str);

pub fn hints(app: &App) -> Vec<Hint> {
    if let Some(confirm) = &app.confirm {
        if confirm.typed_name.is_some() {
            return vec![("enter", "confirm"), ("esc", "cancel")];
        }
        return vec![("y", "confirm"), ("n", "cancel")];
    }
    if app.help_open {
        return vec![("?", "close help")];
    }
    if app.copy_value.is_some() {
        return vec![("esc", "close")];
    }
    if app.modal.is_some() {
        return vec![("tab", "next field"), ("ctrl+s", "save"), ("esc", "cancel")];
    }
    if app.source_search.is_some() {
        return vec![("enter", "apply filter"), ("esc", "cancel")];
    }
    if app.event_screens.detail.header_search.is_some() {
        return vec![("enter", "apply filter"), ("esc", "cancel")];
    }
    if crate::tui::relay_control::search_active(app) {
        return vec![("enter", "apply search"), ("esc", "cancel")];
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
            ("L", "relay"),
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
        Screen::SourceDetail {
            tab: SourceTab::Live,
            ..
        } => vec![
            ("enter", "open"),
            ("f", "follow"),
            ("1-5", "tabs"),
            ("esc", "back"),
        ],
        Screen::SourceDetail {
            tab: SourceTab::Dlq,
            ..
        } => vec![
            ("enter", "open"),
            ("R", "resend"),
            ("F", "filters"),
            ("bksp", "summary"),
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
        Screen::EventDetail { .. } => vec![
            ("tab", "pane"),
            ("enter", "attempts"),
            ("R", "replay"),
            ("/", "headers"),
            ("w", "wrap"),
            ("esc", "back"),
        ],
        Screen::Dlq => vec![
            ("enter", "open"),
            ("R", "resend"),
            ("F", "filters"),
            ("esc", "summary"),
            ("?", "help"),
        ],
        Screen::Stats => vec![
            ("h/l", "range"),
            ("1-3", "24h 7d 30d"),
            ("r", "refresh"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Screen::Relay if app.relay.is_some() => vec![
            ("p", "replay locally"),
            ("u", "change URL"),
            ("x", "stop"),
            ("enter", "expand"),
            ("/", "search"),
        ],
        Screen::Relay => vec![
            ("n", "new relay"),
            ("g", "jump"),
            ("?", "help"),
            ("q", "quit"),
        ],
    }
}

/// The hint bar: the screen's hints with the CRUD keys after `enter open`.
pub fn all_hints(app: &App) -> Vec<Hint> {
    let mut hints = hints(app);
    let extra = crate::tui::crud::hints(app);
    let position = usize::from(hints.first() == Some(&("enter", "open")));
    hints.splice(position..position, extra);
    hints
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
        Screen::Sources => vec![("/", "filter by name"), ("L", "relay this source")],
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
        Screen::EventDetail { .. } => vec![
            ("tab", "headers, body, deliveries"),
            ("j/k, g/G", "scroll, or move between deliveries"),
            ("enter", "show a delivery's attempts"),
            ("/", "filter headers"),
            ("w", "wrap the body"),
            ("R", "replay to connections"),
        ],
        Screen::Dlq => vec![
            ("enter", "a connection's deliveries, then the event"),
            ("bksp, esc", "back to the summary"),
            ("F", "source, statuses, time range"),
            ("R", "resend the selected connection"),
        ],
        Screen::Stats => vec![("h/l, 1-3", "range: 24h, 7d, 30d")],
        Screen::Relay => vec![
            ("n", "start a relay"),
            ("p", "replay the selected request locally"),
            ("u", "change the target URL"),
            ("x", "stop the relay"),
            ("enter", "expand request and response"),
            ("/", "search"),
        ],
        _ => Vec::new(),
    };
    if !specific.is_empty() {
        sections.push(("This screen", specific));
    }
    if matches!(
        screen,
        Screen::Sources
            | Screen::SourceDetail { .. }
            | Screen::Destinations
            | Screen::DestinationDetail { .. }
            | Screen::Connections
            | Screen::ConnectionDetail { .. }
    ) {
        sections.push(crate::tui::crud::help());
    }
    sections
}
