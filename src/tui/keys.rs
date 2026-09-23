//! Key handling. Lowercase keys and digits never change remote state; while a
//! text field has focus, printable keys are input, not hotkeys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::action::Effect;
use crate::tui::app::{App, Confirm, ConfirmAction, Focus, FormPurpose};
use crate::tui::forms::input::TextInput;
use crate::tui::forms::settings_form::{FormOutcome, SettingsForm};
use crate::tui::keys_events;
use crate::tui::screen::{Screen, Section, SourceTab};
use crate::tui::settings::is_http_url;

const PAGE: usize = 10;
const SCROLL_PAGE: u16 = 10;

pub fn handle(app: &mut App, key: KeyEvent) -> Vec<Effect> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    if control && key.code == KeyCode::Char('z') {
        return Vec::new();
    }
    if control && key.code == KeyCode::Char('c') {
        app.request_quit();
        return Vec::new();
    }
    if let Some(confirm) = app.confirm.take() {
        return on_confirm_key(app, confirm, key.code);
    }
    if app.modal.is_some() {
        return on_modal_key(app, key);
    }
    if app.help_open {
        if matches!(
            key.code,
            KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc
        ) {
            app.help_open = false;
        }
        return Vec::new();
    }
    if app.screen == Screen::Login {
        return on_login_key(app, key);
    }
    if app.screen == Screen::Settings && app.focus == Focus::Main {
        if let Some(effects) = on_settings_key(app, key) {
            return effects;
        }
    }
    if app.source_search.is_some() {
        return on_search_key(app, key);
    }
    if app.pending_jump {
        app.pending_jump = false;
        return match key.code {
            KeyCode::Char(letter) => Section::from_jump_key(letter)
                .map(|section| app.switch_to(section))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
    }
    match key.code {
        KeyCode::Char('q') => {
            app.request_quit();
            return Vec::new();
        }
        KeyCode::Char('?') => {
            app.help_open = true;
            return Vec::new();
        }
        KeyCode::Char('r') => return app.refresh_now(),
        KeyCode::Char('g') if app.focus == Focus::Sidebar || !app.in_scrollable_pane() => {
            app.pending_jump = true;
            return Vec::new();
        }
        _ => {}
    }
    match app.focus {
        Focus::Sidebar => on_sidebar_key(app, key.code),
        Focus::Main => on_main_key(app, key.code),
    }
}

/// The new cursor for a navigation key, or `None` for any other key.
pub(crate) fn moved(cursor: usize, length: usize, code: KeyCode) -> Option<usize> {
    let last = length.saturating_sub(1);
    match code {
        KeyCode::Up | KeyCode::Char('k') => Some(cursor.saturating_sub(1)),
        KeyCode::Down | KeyCode::Char('j') => Some((cursor + 1).min(last)),
        KeyCode::Home => Some(0),
        KeyCode::End => Some(last),
        KeyCode::PageUp => Some(cursor.saturating_sub(PAGE)),
        KeyCode::PageDown => Some((cursor + PAGE).min(last)),
        _ => None,
    }
}

pub(crate) fn to_sidebar(app: &mut App, code: KeyCode) -> Vec<Effect> {
    if matches!(
        code,
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') | KeyCode::Tab | KeyCode::BackTab
    ) {
        app.focus = Focus::Sidebar;
    }
    Vec::new()
}

fn on_sidebar_key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    let last = Section::ALL.len() - 1;
    let target = match code {
        KeyCode::Up | KeyCode::Char('k') => app.cursors.sidebar.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => (app.cursors.sidebar + 1).min(last),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
            app.focus = Focus::Main;
            return Vec::new();
        }
        _ => return Vec::new(),
    };
    if target == app.cursors.sidebar {
        return Vec::new();
    }
    let effects = app.switch_to(Section::ALL[target]);
    app.focus = Focus::Sidebar;
    effects
}

fn on_main_key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    match app.screen.clone() {
        Screen::Sources => on_sources_key(app, code),
        Screen::SourceDetail { tab, .. } => on_source_detail_key(app, code, tab),
        Screen::Destinations => {
            let length = app.data.destinations.value.as_ref().map_or(0, Vec::len);
            if let Some(cursor) = moved(app.cursors.destinations, length, code) {
                app.cursors.destinations = cursor;
                return Vec::new();
            }
            if code == KeyCode::Enter {
                let selected = app
                    .data
                    .destinations
                    .value
                    .as_ref()
                    .and_then(|destinations| destinations.get(app.cursors.destinations))
                    .map(|destination| destination.id.clone());
                if let Some(id) = selected {
                    return app.open(Screen::DestinationDetail { id });
                }
                return Vec::new();
            }
            to_sidebar(app, code)
        }
        Screen::Connections => {
            let length = app.data.connections.value.as_ref().map_or(0, Vec::len);
            if let Some(cursor) = moved(app.cursors.connections, length, code) {
                app.cursors.connections = cursor;
                return Vec::new();
            }
            if code == KeyCode::Enter {
                let selected = app
                    .data
                    .connections
                    .value
                    .as_ref()
                    .and_then(|connections| connections.get(app.cursors.connections))
                    .map(|connection| connection.id.clone());
                if let Some(id) = selected {
                    return app.open(Screen::ConnectionDetail { id });
                }
                return Vec::new();
            }
            to_sidebar(app, code)
        }
        Screen::DestinationDetail { .. } | Screen::ConnectionDetail { .. } => {
            on_text_pane_key(app, code)
        }
        Screen::Events
        | Screen::Dlq
        | Screen::Stats
        | Screen::Relay
        | Screen::Settings
        | Screen::EventDetail { .. } => to_sidebar(app, code),
        Screen::Login => Vec::new(),
    }
}

fn on_sources_key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    let length = app.data.sources.value.as_ref().map_or(0, Vec::len);
    if let Some(cursor) = moved(app.cursors.sources, length, code) {
        app.cursors.sources = cursor;
        return Vec::new();
    }
    match code {
        KeyCode::Enter => {
            let selected = app
                .data
                .sources
                .value
                .as_ref()
                .and_then(|sources| sources.get(app.cursors.sources))
                .map(|source| source.id.clone());
            match selected {
                Some(id) => app.open(Screen::SourceDetail {
                    id,
                    tab: SourceTab::Overview,
                }),
                None => Vec::new(),
            }
        }
        KeyCode::Char('/') => {
            app.source_search = Some(TextInput::new(
                app.source_query.clone().unwrap_or_default(),
                false,
            ));
            Vec::new()
        }
        other => to_sidebar(app, other),
    }
}

fn on_source_detail_key(app: &mut App, code: KeyCode, tab: SourceTab) -> Vec<Effect> {
    match code {
        KeyCode::Char(digit @ '1'..='5') => {
            let index = usize::from(digit as u8 - b'1');
            return app.set_source_tab(SourceTab::ALL[index]);
        }
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
            return app.set_source_tab(tab.next())
        }
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
            return app.set_source_tab(tab.previous())
        }
        KeyCode::Esc => return app.back(),
        _ => {}
    }
    if tab != SourceTab::Connections {
        return Vec::new();
    }
    let length = app
        .data
        .source_connections
        .value
        .as_ref()
        .map_or(0, Vec::len);
    if let Some(cursor) = moved(app.cursors.source_connections, length, code) {
        app.cursors.source_connections = cursor;
        return Vec::new();
    }
    if code == KeyCode::Enter {
        let selected = app
            .data
            .source_connections
            .value
            .as_ref()
            .and_then(|connections| connections.get(app.cursors.source_connections))
            .map(|connection| connection.id.clone());
        if let Some(id) = selected {
            return app.open(Screen::ConnectionDetail { id });
        }
    }
    Vec::new()
}

fn on_text_pane_key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    let limit = app.scroll_limit.get();
    match code {
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => return app.back(),
        KeyCode::Down | KeyCode::Char('j') => app.scroll = app.scroll.saturating_add(1).min(limit),
        KeyCode::Up | KeyCode::Char('k') => app.scroll = app.scroll.saturating_sub(1),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_add(SCROLL_PAGE).min(limit),
        KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(SCROLL_PAGE),
        KeyCode::Char('g') | KeyCode::Home => app.scroll = 0,
        KeyCode::Char('G') | KeyCode::End => app.scroll = limit,
        _ => {}
    }
    Vec::new()
}

fn on_search_key(app: &mut App, key: KeyEvent) -> Vec<Effect> {
    match key.code {
        KeyCode::Esc => {
            app.source_search = None;
            Vec::new()
        }
        KeyCode::Enter => {
            let query = app
                .source_search
                .take()
                .map(|input| input.value().trim().to_string())
                .unwrap_or_default();
            app.source_query = (!query.is_empty()).then_some(query);
            app.cursors.sources = 0;
            app.enter()
        }
        _ => {
            if let Some(input) = app.source_search.as_mut() {
                input.handle(key);
            }
            Vec::new()
        }
    }
}

fn on_login_key(app: &mut App, key: KeyEvent) -> Vec<Effect> {
    match key.code {
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
            app.login.server_focused = !app.login.server_focused;
            Vec::new()
        }
        KeyCode::Enter => submit_login(app),
        KeyCode::Esc => {
            app.login.error = None;
            Vec::new()
        }
        _ => {
            let input = if app.login.server_focused {
                &mut app.login.server
            } else {
                &mut app.login.api_key
            };
            input.handle(key);
            Vec::new()
        }
    }
}

fn submit_login(app: &mut App) -> Vec<Effect> {
    if app.login.submitting {
        return Vec::new();
    }
    let api_key = app.login.api_key.value().trim().to_string();
    let server = app
        .login
        .server
        .value()
        .trim()
        .trim_end_matches('/')
        .to_string();
    let problem = if !api_key.starts_with("whk_") {
        Some("An API key starts with whk_")
    } else if !is_http_url(&server) {
        Some("The server must be an http(s) URL")
    } else {
        None
    };
    if let Some(problem) = problem {
        app.login.error = Some(problem.to_string());
        return Vec::new();
    }
    app.login.error = None;
    app.login.submitting = true;
    vec![Effect::Login { server, api_key }]
}

/// `None` when the form did not use the key, so global keys still work.
fn on_settings_key(app: &mut App, key: KeyEvent) -> Option<Vec<Effect>> {
    let form = app.settings_form.as_mut()?;
    match form.handle(key) {
        FormOutcome::Ignored => None,
        FormOutcome::Consumed => Some(Vec::new()),
        FormOutcome::Save => Some(match form.collect() {
            Ok(settings) => {
                form.error = None;
                let section = settings.to_section(Default::default());
                app.pending_settings = Some(settings);
                vec![Effect::SaveSettings(Box::new(section))]
            }
            Err(message) => {
                form.error = Some(message);
                Vec::new()
            }
        }),
        FormOutcome::Cancel => {
            if form.is_dirty() {
                app.confirm = Some(Confirm::plain(
                    "Discard changes?",
                    ConfirmAction::DiscardSettings,
                ));
            } else {
                app.focus = Focus::Sidebar;
            }
            Some(Vec::new())
        }
    }
}

fn on_modal_key(app: &mut App, key: KeyEvent) -> Vec<Effect> {
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    if modal.form.submitting {
        return Vec::new();
    }
    match modal.form.handle(key) {
        FormOutcome::Save => submit_modal(app),
        FormOutcome::Cancel => {
            if modal.form.is_dirty() {
                app.confirm = Some(Confirm::plain(
                    "Discard changes?",
                    ConfirmAction::DiscardForm,
                ));
            } else {
                app.modal = None;
            }
            Vec::new()
        }
        FormOutcome::Consumed | FormOutcome::Ignored => Vec::new(),
    }
}

/// Ctrl+S in a modal form: validate, then act or ask for confirmation.
pub fn submit_modal(app: &mut App) -> Vec<Effect> {
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    modal.form.clear_errors();
    let purpose = modal.purpose.clone();
    match purpose {
        FormPurpose::EventFilters => keys_events::submit_event_filters(app),
    }
}

fn on_confirm_key(app: &mut App, confirm: Confirm, code: KeyCode) -> Vec<Effect> {
    match code {
        KeyCode::Char('y') => confirmed(app, confirm.action),
        KeyCode::Char('n') | KeyCode::Esc | KeyCode::Enter => Vec::new(),
        _ => {
            app.confirm = Some(confirm);
            Vec::new()
        }
    }
}

fn confirmed(app: &mut App, action: ConfirmAction) -> Vec<Effect> {
    match action {
        ConfirmAction::DiscardSettings => {
            app.settings_form = Some(SettingsForm::new(&app.settings));
            app.focus = Focus::Sidebar;
            Vec::new()
        }
        ConfirmAction::DiscardForm => {
            app.modal = None;
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::action::{Action, Request};
    use crate::tui::app::update;
    use crate::tui::fixtures;
    use crate::tui::settings::ThemeChoice;

    fn press(app: &mut App, code: KeyCode) -> Vec<Effect> {
        update(app, Action::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn chord(app: &mut App, letter: char) -> Vec<Effect> {
        update(
            app,
            Action::Key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::CONTROL)),
        )
    }

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    fn fetched_requests(effects: &[Effect]) -> Vec<Request> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Fetch { request, .. } => Some(request.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn g_then_a_letter_jumps_to_a_section() {
        let mut app = fixtures::app();
        assert!(press(&mut app, KeyCode::Char('g')).is_empty());
        let effects = press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.screen, Screen::Destinations);
        assert_eq!(fetched_requests(&effects), vec![Request::Destinations]);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char(','));
        assert_eq!(app.screen, Screen::Settings);
    }

    #[test]
    fn the_sidebar_switches_sections_and_keeps_focus() {
        let mut app = fixtures::app();
        app.focus = Focus::Sidebar;
        press(&mut app, KeyCode::Down);
        assert_eq!(app.screen, Screen::Destinations);
        assert_eq!(app.focus, Focus::Sidebar);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Main);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Sidebar);
    }

    #[test]
    fn enter_opens_the_selected_source_and_esc_returns() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('j'));
        let effects = press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.screen,
            Screen::SourceDetail {
                id: fixtures::GITHUB_ID.into(),
                tab: SourceTab::Overview
            }
        );
        assert_eq!(
            fetched_requests(&effects),
            vec![Request::Source {
                id: fixtures::GITHUB_ID.into()
            }]
        );
        let effects = press(&mut app, KeyCode::Char('4'));
        assert!(
            fetched_requests(&effects).contains(&Request::SourceConnections {
                source_id: fixtures::GITHUB_ID.into()
            })
        );
        press(&mut app, KeyCode::Tab);
        assert!(matches!(
            app.screen,
            Screen::SourceDetail {
                tab: SourceTab::Dlq,
                ..
            }
        ));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Sources);
        assert_eq!(app.cursors.sources, 1);
    }

    #[test]
    fn enter_on_the_connections_tab_opens_the_connection() {
        let mut app = fixtures::source_detail(SourceTab::Connections);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.screen,
            Screen::ConnectionDetail {
                id: fixtures::STRIPE_AUDIT_ID.into()
            }
        );
    }

    #[test]
    fn text_panes_scroll_and_g_means_top() {
        let mut app = fixtures::connection_detail();
        app.scroll_limit.set(30);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.scroll, 11);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.scroll, 30);
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.scroll, 0);
        assert!(!app.pending_jump);
    }

    #[test]
    fn typing_in_the_filter_never_triggers_hotkeys() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "qr?g");
        assert!(!app.quit);
        assert!(!app.help_open);
        let effects = press(&mut app, KeyCode::Enter);
        assert_eq!(app.source_query.as_deref(), Some("qr?g"));
        assert!(fetched_requests(&effects).contains(&Request::Sources {
            search: Some("qr?g".into())
        }));
        press(&mut app, KeyCode::Char('/'));
        chord(&mut app, 'u');
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.source_query, None);
    }

    #[test]
    fn login_checks_the_key_before_sending() {
        let mut app = App::new(fixtures::init(false));
        app.start();
        type_text(&mut app, "nope");
        assert!(press(&mut app, KeyCode::Enter).is_empty());
        assert!(app.login.error.as_deref().unwrap().contains("whk_"));
        chord(&mut app, 'u');
        type_text(&mut app, "whk_new");
        let effects = press(&mut app, KeyCode::Enter);
        assert_eq!(
            effects,
            vec![Effect::Login {
                server: "https://app.webhooker.eu".into(),
                api_key: "whk_new".into()
            }]
        );
        assert!(app.login.submitting);
        assert!(
            press(&mut app, KeyCode::Enter).is_empty(),
            "no double submit"
        );
    }

    #[test]
    fn login_switches_to_the_server_field() {
        let mut app = App::new(fixtures::init(false));
        app.start();
        press(&mut app, KeyCode::Tab);
        chord(&mut app, 'u');
        type_text(&mut app, "ftp://x");
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "whk_new");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.login.error.as_deref(),
            Some("The server must be an http(s) URL")
        );
    }

    #[test]
    fn settings_save_through_ctrl_s() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char(','));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        let effects = chord(&mut app, 's');
        let [Effect::SaveSettings(section)] = effects.as_slice() else {
            panic!("{effects:?}");
        };
        assert_eq!(section.theme.as_deref(), Some("dark"));
        assert_eq!(app.settings.theme, ThemeChoice::Auto, "applied once saved");
        assert_eq!(
            app.pending_settings.as_ref().unwrap().theme,
            ThemeChoice::Dark
        );
    }

    #[test]
    fn leaving_changed_settings_asks_first() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char(','));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.confirm.as_ref().unwrap().text(), "Discard changes?");
        press(&mut app, KeyCode::Char('x'));
        assert!(app.confirm.is_some(), "other keys keep the question open");
        press(&mut app, KeyCode::Char('n'));
        assert!(app.confirm.is_none());
        assert!(app.settings_form.as_ref().unwrap().is_dirty());
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('y'));
        assert!(!app.settings_form.as_ref().unwrap().is_dirty());
        assert_eq!(app.focus, Focus::Sidebar);
    }

    #[test]
    fn enter_cancels_a_confirmation() {
        let mut app = fixtures::app();
        app.confirm = Some(Confirm::plain(
            "Discard changes?",
            ConfirmAction::DiscardSettings,
        ));
        press(&mut app, KeyCode::Enter);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn ctrl_z_is_ignored_and_q_or_ctrl_c_quit() {
        let mut app = fixtures::app();
        assert!(chord(&mut app, 'z').is_empty());
        assert!(!app.quit);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
        let mut app = fixtures::app();
        chord(&mut app, 'c');
        assert!(app.quit);
    }

    #[test]
    fn help_swallows_keys_until_closed() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('?'));
        assert!(app.help_open);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursors.sources, 0);
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.help_open);
        assert!(!app.quit);
    }

    #[test]
    fn r_refreshes_as_a_user_action() {
        let mut app = fixtures::app();
        let effects = press(&mut app, KeyCode::Char('r'));
        assert!(effects.iter().all(|effect| matches!(
            effect,
            Effect::Fetch {
                priority: crate::tui::budget::Priority::User,
                ..
            }
        )));
        assert_eq!(effects.len(), 2);
    }

    #[test]
    fn hints_follow_the_focus_and_the_screen() {
        let mut app = fixtures::app();
        assert_eq!(crate::tui::hints::hints(&app)[0], ("enter", "open"));
        app.focus = Focus::Sidebar;
        assert_eq!(crate::tui::hints::hints(&app)[0], ("j/k", "section"));
        let help = crate::tui::hints::help(&Screen::Sources);
        assert_eq!(help[0].0, "Global");
        assert!(help[1].1.contains(&("/", "filter by name")));
    }
}
