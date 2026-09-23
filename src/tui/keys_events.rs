//! Keys of the Events, Live, event detail, DLQ and Stats screens, and the
//! submits of their forms.

use crate::tui::action::Effect;
use crate::tui::app::{App, FormPurpose, ModalForm};
use crate::tui::events_state::{is_rfc3339, EventFilter, EventsScope, TimeWindow};
use crate::tui::forms::form::{Field, Form, SelectOption};
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
}
