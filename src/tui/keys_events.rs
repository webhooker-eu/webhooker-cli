//! Keys of the Events, Live, event detail, DLQ and Stats screens, and the
//! submits of their forms.

use ratatui::crossterm::event::KeyCode;

use crate::tui::action::Effect;
use crate::tui::app::{App, FormPurpose, ModalForm};
use crate::tui::events_state::{
    is_rfc3339, EventFilter, EventsScope, TimeWindow, EVENTS_PAGE_SIZE,
};
use crate::tui::forms::form::{Field, Form, SelectOption};
use crate::tui::keys;
use crate::tui::screen::Screen;

const TIME_HINT: &str = "Use RFC 3339, e.g. 2026-09-20T10:00:00Z";

fn non_empty(raw: String) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn scope_of(app: &App) -> EventsScope {
    if app.screen == Screen::Events {
        EventsScope::Global
    } else {
        EventsScope::Source
    }
}

fn verification_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("", "any"),
        SelectOption::new("verified", "verified"),
        SelectOption::new("failed", "failed"),
        SelectOption::new("skipped", "skipped"),
    ]
}

/// `F`: source (Events screen only), verification, time range, custom
/// bounds and public-id search.
pub fn open_event_filters(app: &mut App, scope: EventsScope) {
    let filter = app.event_screens.events(scope).filter.clone();
    let mut fields = Vec::new();
    if scope == EventsScope::Global {
        let mut options = vec![SelectOption::new("", "all sources")];
        options.extend(
            app.data
                .sources
                .value
                .iter()
                .flatten()
                .map(|source| SelectOption::new(source.id.clone(), source.name.clone())),
        );
        fields.push(Field::select(
            "source",
            "Source",
            options,
            Some(filter.source_id.as_deref().unwrap_or("")),
        ));
    }
    fields.push(Field::select(
        "verification",
        "Verification",
        verification_options(),
        Some(filter.verification.as_deref().unwrap_or("")),
    ));
    fields.push(Field::select(
        "window",
        "Time range",
        TimeWindow::ALL
            .iter()
            .map(|window| SelectOption::new(window.value(), window.label()))
            .collect(),
        Some(filter.window.value()),
    ));
    fields.push(Field::text(
        "since",
        "Since (custom)",
        filter.since.clone().unwrap_or_default(),
    ));
    fields.push(Field::text(
        "until",
        "Until (custom)",
        filter.until.clone().unwrap_or_default(),
    ));
    fields.push(Field::text(
        "search",
        "Public id",
        filter.search.clone().unwrap_or_default(),
    ));
    app.modal = Some(ModalForm {
        form: Form::new("Event filters", fields),
        purpose: FormPurpose::EventFilters,
    });
}

pub fn submit_event_filters(app: &mut App) -> Vec<Effect> {
    let scope = scope_of(app);
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    let form = &mut modal.form;
    let window = form
        .selected("window")
        .and_then(|value| TimeWindow::from_value(&value))
        .unwrap_or_default();
    let since = non_empty(form.text("since"));
    let until = non_empty(form.text("until"));
    if window == TimeWindow::Custom {
        let mut valid = true;
        if since.is_none() && until.is_none() {
            form.set_field_error("since", "Give a start, an end, or both");
            valid = false;
        }
        for (key, value) in [("since", &since), ("until", &until)] {
            if value.as_deref().is_some_and(|raw| !is_rfc3339(raw)) {
                form.set_field_error(key, TIME_HINT);
                valid = false;
            }
        }
        if !valid {
            return Vec::new();
        }
    }
    let custom = window == TimeWindow::Custom;
    let filter = EventFilter {
        source_id: match scope {
            EventsScope::Global => form.selected("source").and_then(non_empty),
            EventsScope::Source => None,
        },
        verification: form.selected("verification").and_then(non_empty),
        window,
        since: since.filter(|_| custom),
        until: until.filter(|_| custom),
        search: non_empty(form.text("search")),
    };
    app.modal = None;
    let state = app.event_screens.events_mut(scope);
    state.filter = filter;
    state.page = 1;
    state.cursor = 0;
    app.enter()
}

pub fn on_events_list_key(app: &mut App, code: KeyCode, scope: EventsScope) -> Vec<Effect> {
    let rows = app
        .data
        .events
        .value
        .as_ref()
        .map_or(0, |page| page.items.len());
    let cursor = app.event_screens.events(scope).cursor;
    if let Some(moved) = keys::moved(cursor, rows, code) {
        app.event_screens.events_mut(scope).cursor = moved;
        return Vec::new();
    }
    match code {
        KeyCode::Enter => {
            let selected = app
                .data
                .events
                .value
                .as_ref()
                .and_then(|page| page.items.get(cursor))
                .map(|event| event.id.clone());
            match selected {
                Some(id) => app.open(Screen::EventDetail { id }),
                None => Vec::new(),
            }
        }
        KeyCode::Char('F') => {
            open_event_filters(app, scope);
            Vec::new()
        }
        KeyCode::Char(']') => turn_page(app, scope, 1),
        KeyCode::Char('[') => turn_page(app, scope, -1),
        other if scope == EventsScope::Global => keys::to_sidebar(app, other),
        _ => Vec::new(),
    }
}

fn turn_page(app: &mut App, scope: EventsScope, step: i64) -> Vec<Effect> {
    let Some(total) = app.data.events.value.as_ref().and_then(|page| page.total) else {
        return Vec::new();
    };
    let last_page = ((total + EVENTS_PAGE_SIZE - 1) / EVENTS_PAGE_SIZE).max(1);
    let state = app.event_screens.events_mut(scope);
    let target = (state.page + step).clamp(1, last_page);
    if target == state.page {
        return Vec::new();
    }
    state.page = target;
    state.cursor = 0;
    app.enter()
}

pub fn on_live_key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    let live = &mut app.event_screens.live;
    if let Some(cursor) = keys::moved(live.cursor, live.rows.len(), code) {
        live.cursor = cursor;
        return Vec::new();
    }
    match code {
        KeyCode::Char('f') => {
            live.follow = !live.follow;
            if live.follow {
                live.cursor = 0;
            }
            Vec::new()
        }
        KeyCode::Enter => {
            let selected = live.rows.get(live.cursor).map(|row| row.id.clone());
            match selected {
                Some(id) => app.open(Screen::EventDetail { id }),
                None => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::action::{Action, Request};
    use crate::tui::app::update;
    use crate::tui::fixtures;
    use crate::tui::screen::Section;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    fn on_events() -> App {
        let mut app = fixtures::app();
        app.switch_to(Section::Events);
        app
    }

    #[test]
    fn the_filter_form_applies_on_ctrl_s() {
        let mut app = on_events();
        open_event_filters(&mut app, EventsScope::Global);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "8f2a");
        let effects = chord(&mut app, 's');
        assert!(app.modal.is_none());
        let expected = EventFilter {
            source_id: Some(fixtures::STRIPE_ID.into()),
            window: TimeWindow::LastHour,
            search: Some("8f2a".into()),
            ..EventFilter::default()
        };
        assert_eq!(app.event_screens.global.filter, expected);
        assert!(fetched_requests(&effects).contains(&Request::Events {
            filter: expected,
            page: 1
        }));
        assert!(effects.contains(&Effect::SubscribeTail {
            source_id: fixtures::STRIPE_ID.into(),
            subscription: 1
        }));
    }

    #[test]
    fn a_custom_range_needs_valid_timestamps() {
        let mut app = on_events();
        open_event_filters(&mut app, EventsScope::Global);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "yesterday");
        chord(&mut app, 's');
        let form = &app.modal.as_ref().expect("the form stays open").form;
        assert_eq!(
            form.fields[3].error.as_deref(),
            Some("Use RFC 3339, e.g. 2026-09-20T10:00:00Z")
        );
    }

    #[test]
    fn the_source_tab_form_has_no_source_field() {
        let mut app = fixtures::source_detail(crate::tui::screen::SourceTab::Events);
        open_event_filters(&mut app, EventsScope::Source);
        let keys: Vec<&str> = app
            .modal
            .as_ref()
            .unwrap()
            .form
            .fields
            .iter()
            .map(|field| field.key)
            .collect();
        assert_eq!(
            keys,
            vec!["verification", "window", "since", "until", "search"]
        );
    }

    #[test]
    fn cancelling_asks_only_when_something_changed() {
        let mut app = on_events();
        open_event_filters(&mut app, EventsScope::Global);
        press(&mut app, KeyCode::Esc);
        assert!(app.modal.is_none());
        open_event_filters(&mut app, EventsScope::Global);
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Esc);
        assert!(app.modal.is_some());
        assert_eq!(app.confirm.as_ref().unwrap().text(), "Discard changes?");
        press(&mut app, KeyCode::Char('y'));
        assert!(app.modal.is_none());
    }

    #[test]
    fn keys_never_leak_out_of_a_modal() {
        let mut app = on_events();
        open_event_filters(&mut app, EventsScope::Global);
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Char('?'));
        assert!(!app.quit);
        assert!(!app.help_open);
        assert!(app.modal.is_some());
    }

    #[test]
    fn enter_opens_the_selected_event() {
        let mut app = fixtures::events_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.screen,
            Screen::EventDetail {
                id: "0198c9f0-0000-7000-8000-0000000000e2".into()
            }
        );
    }

    #[test]
    fn brackets_turn_pages_within_the_total() {
        let mut app = fixtures::events_app();
        let effects = press(&mut app, KeyCode::Char(']'));
        assert_eq!(app.event_screens.global.page, 2);
        assert!(fetched_requests(&effects).contains(&Request::Events {
            filter: EventFilter::default(),
            page: 2
        }));
        // Turning a page reloads the list; put page data back each time.
        let now = app.now;
        app.data.events.finish(fixtures::events_page(), now);
        press(&mut app, KeyCode::Char(']'));
        app.data.events.finish(fixtures::events_page(), now);
        assert!(
            press(&mut app, KeyCode::Char(']')).is_empty(),
            "120 events are 3 pages"
        );
        assert_eq!(app.event_screens.global.page, 3);
        app.data.events.finish(fixtures::events_page(), now);
        press(&mut app, KeyCode::Char('['));
        assert_eq!(app.event_screens.global.page, 2);
        app.data.events = Default::default();
        assert!(
            press(&mut app, KeyCode::Char(']')).is_empty(),
            "no paging before the list loads"
        );
    }

    #[test]
    fn capital_f_opens_the_filters_and_esc_leaves_to_the_sidebar() {
        let mut app = fixtures::events_app();
        press(&mut app, KeyCode::Char('F'));
        assert_eq!(
            app.modal.as_ref().unwrap().purpose,
            FormPurpose::EventFilters
        );
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, crate::tui::app::Focus::Sidebar);
    }

    #[test]
    fn the_source_events_tab_pages_its_own_state() {
        let mut app = fixtures::source_detail(crate::tui::screen::SourceTab::Events);
        let now = app.now;
        app.data.events.finish(fixtures::events_page(), now);
        press(&mut app, KeyCode::Char(']'));
        assert_eq!(app.event_screens.source.page, 2);
        assert_eq!(app.event_screens.global.page, 1);
    }

    #[test]
    fn the_live_feed_moves_follows_and_opens() {
        let mut app = fixtures::live_app();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.event_screens.live.cursor, 1);
        press(&mut app, KeyCode::Char('f'));
        assert!(!app.event_screens.live.follow);
        press(&mut app, KeyCode::Char('f'));
        assert!(app.event_screens.live.follow);
        assert_eq!(
            app.event_screens.live.cursor, 0,
            "following jumps to the newest"
        );
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.screen,
            Screen::EventDetail {
                id: "id-evt_new".into()
            }
        );
    }
}
