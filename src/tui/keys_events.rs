//! Keys of the Events, Live, event detail, DLQ and Stats screens, and the
//! submits of their forms.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::tui::action::Effect;
use crate::tui::app::{App, Confirm, ConfirmAction, FormPurpose, ModalForm};
use crate::tui::events_state::{
    is_rfc3339, DlqPane, EventFilter, EventPane, EventsScope, TimeWindow, DLQ_STATUSES,
    EVENTS_PAGE_SIZE,
};
use crate::tui::forms::form::{CheckItem, Field, Form, SelectOption};
use crate::tui::forms::input::TextInput;
use crate::tui::keys;
use crate::tui::screen::Screen;
use crate::tui::theme::Tone;

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

const SCROLL_PAGE: u16 = 10;

fn scroll(position: &mut u16, limit: u16, code: KeyCode) {
    *position = match code {
        KeyCode::Down | KeyCode::Char('j') => position.saturating_add(1).min(limit),
        KeyCode::Up | KeyCode::Char('k') => position.saturating_sub(1),
        KeyCode::PageDown => position.saturating_add(SCROLL_PAGE).min(limit),
        KeyCode::PageUp => position.saturating_sub(SCROLL_PAGE),
        KeyCode::Char('g') | KeyCode::Home => 0,
        KeyCode::Char('G') | KeyCode::End => limit,
        _ => *position,
    };
}

pub fn on_event_detail_key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    match code {
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => return app.back(),
        KeyCode::Char('R') => return open_replay(app),
        _ => {}
    }
    let deliveries: Vec<String> = app
        .data
        .event
        .value
        .as_ref()
        .map(|event| {
            event
                .deliveries
                .iter()
                .map(|delivery| delivery.id.clone())
                .collect()
        })
        .unwrap_or_default();
    let view = &mut app.event_screens.detail;
    match code {
        KeyCode::Tab => view.pane = view.pane.next(),
        KeyCode::BackTab => view.pane = view.pane.previous(),
        KeyCode::Char('w') => view.wrap = !view.wrap,
        KeyCode::Char('/') => {
            view.pane = EventPane::Headers;
            view.header_search = Some(TextInput::new(
                view.header_filter.clone().unwrap_or_default(),
                false,
            ));
        }
        KeyCode::Enter if view.pane == EventPane::Deliveries => {
            if let Some(id) = deliveries.get(view.delivery_cursor) {
                view.expanded = if view.expanded.as_ref() == Some(id) {
                    None
                } else {
                    Some(id.clone())
                };
            }
        }
        other => match view.pane {
            EventPane::Headers => scroll(&mut view.headers_scroll, view.headers_limit.get(), other),
            EventPane::Body => scroll(&mut view.body_scroll, view.body_limit.get(), other),
            EventPane::Deliveries => {
                if let Some(cursor) = keys::moved(view.delivery_cursor, deliveries.len(), other) {
                    view.delivery_cursor = cursor;
                }
            }
        },
    }
    Vec::new()
}

pub fn on_header_search_key(app: &mut App, key: KeyEvent) -> Vec<Effect> {
    let view = &mut app.event_screens.detail;
    match key.code {
        KeyCode::Esc => view.header_search = None,
        KeyCode::Enter => {
            let value = view
                .header_search
                .take()
                .map(|input| input.value().trim().to_string())
                .unwrap_or_default();
            view.header_filter = (!value.is_empty()).then_some(value);
            view.headers_scroll = 0;
        }
        _ => {
            if let Some(input) = view.header_search.as_mut() {
                input.handle(key);
            }
        }
    }
    Vec::new()
}

/// `R`: a checklist of the source's connections, all selected, disabled
/// ones marked.
fn open_replay(app: &mut App) -> Vec<Effect> {
    let Some(event) = app.data.event.value.as_ref() else {
        return Vec::new();
    };
    let (event_id, public_id, source_id) = (
        event.id.clone(),
        event.public_id.clone(),
        event.source_id.clone(),
    );
    let connections = app
        .data
        .source_connections
        .value
        .clone()
        .filter(|connections| {
            connections
                .iter()
                .all(|connection| connection.source_id == source_id)
        });
    let Some(connections) = connections else {
        app.toast(
            "Connections are still loading; try again in a moment",
            Tone::Warning,
        );
        return Vec::new();
    };
    if connections.is_empty() {
        app.toast("This source has no connections to replay to", Tone::Warning);
        return Vec::new();
    }
    let items = connections
        .iter()
        .map(|connection| {
            let item = CheckItem::new(
                connection.id.clone(),
                connection.destination.name.clone(),
                true,
            );
            if connection.enabled {
                item
            } else {
                item.with_note("disabled")
            }
        })
        .collect();
    app.modal = Some(ModalForm {
        form: Form::new(
            format!("Replay {public_id}"),
            vec![Field::checklist("connections", "Connections", items)],
        ),
        purpose: FormPurpose::Replay {
            event_id,
            public_id,
        },
    });
    Vec::new()
}

pub fn submit_replay(app: &mut App, event_id: String, public_id: String) -> Vec<Effect> {
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    let connection_ids = modal.form.checked("connections");
    if connection_ids.is_empty() {
        modal
            .form
            .set_field_error("connections", "Pick at least one connection");
        return Vec::new();
    }
    let count = connection_ids.len();
    app.modal = None;
    app.confirm = Some(Confirm::about(
        "Replay ",
        public_id,
        format!(
            " to {count} connection{}?",
            if count == 1 { "" } else { "s" }
        ),
        ConfirmAction::ReplayEvent {
            event_id,
            connection_ids,
        },
    ));
    Vec::new()
}

pub fn on_dlq_key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    let in_source_tab = matches!(app.screen, Screen::SourceDetail { .. });
    let summaries = app
        .data
        .dlq_summary
        .value
        .as_ref()
        .map_or(0, |page| page.items.len());
    let entries = app.selected_dlq_entries().len();
    let pane = app.event_screens.dlq.pane;
    let dlq = &mut app.event_screens.dlq;
    match pane {
        DlqPane::Summary => {
            if let Some(cursor) = keys::moved(dlq.summary_cursor, summaries, code) {
                dlq.summary_cursor = cursor;
                dlq.entries_cursor = 0;
                return Vec::new();
            }
        }
        DlqPane::Entries => {
            if let Some(cursor) = keys::moved(dlq.entries_cursor, entries, code) {
                dlq.entries_cursor = cursor;
                return Vec::new();
            }
        }
    }
    match code {
        KeyCode::Enter if pane == DlqPane::Summary => {
            if summaries > 0 {
                app.event_screens.dlq.pane = DlqPane::Entries;
            }
            Vec::new()
        }
        KeyCode::Enter => {
            let selected = app
                .selected_dlq_entries()
                .get(app.event_screens.dlq.entries_cursor)
                .map(|entry| entry.event_id.clone());
            match selected {
                Some(id) => app.open(Screen::EventDetail { id }),
                None => Vec::new(),
            }
        }
        KeyCode::Backspace => {
            app.event_screens.dlq.pane = DlqPane::Summary;
            Vec::new()
        }
        KeyCode::Esc if !in_source_tab && pane == DlqPane::Entries => {
            app.event_screens.dlq.pane = DlqPane::Summary;
            Vec::new()
        }
        KeyCode::Char('F') => {
            open_dlq_filters(app);
            Vec::new()
        }
        KeyCode::Char('R') => {
            open_bulk_resend(app);
            Vec::new()
        }
        other if !in_source_tab => keys::to_sidebar(app, other),
        _ => Vec::new(),
    }
}

fn status_items(checked: &[String]) -> Vec<CheckItem> {
    DLQ_STATUSES
        .iter()
        .map(|status| {
            CheckItem::new(
                *status,
                *status,
                checked.iter().any(|value| value == status),
            )
        })
        .collect()
}

/// `F` on the DLQ: source (DLQ screen only), statuses and failure window.
pub fn open_dlq_filters(app: &mut App) {
    let dlq = app.event_screens.dlq.clone();
    let mut fields = Vec::new();
    if !matches!(app.screen, Screen::SourceDetail { .. }) {
        let options = app
            .data
            .sources
            .value
            .iter()
            .flatten()
            .map(|source| SelectOption::new(source.id.clone(), source.name.clone()))
            .collect();
        fields.push(Field::select(
            "source",
            "Source",
            options,
            app.dlq_source_id().as_deref(),
        ));
    }
    fields.push(Field::checklist(
        "statuses",
        "Statuses",
        status_items(&dlq.statuses),
    ));
    fields.push(Field::select(
        "window",
        "Failed within",
        [
            TimeWindow::All,
            TimeWindow::LastHour,
            TimeWindow::LastDay,
            TimeWindow::LastWeek,
        ]
        .iter()
        .map(|window| SelectOption::new(window.value(), window.label()))
        .collect(),
        Some(dlq.window.value()),
    ));
    app.modal = Some(ModalForm {
        form: Form::new("DLQ filters", fields),
        purpose: FormPurpose::DlqFilters,
    });
}

pub fn submit_dlq_filters(app: &mut App) -> Vec<Effect> {
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    let statuses = modal.form.checked("statuses");
    if statuses.is_empty() {
        modal
            .form
            .set_field_error("statuses", "Pick at least one status");
        return Vec::new();
    }
    let window = modal
        .form
        .selected("window")
        .and_then(|value| TimeWindow::from_value(&value))
        .unwrap_or_default();
    let source = modal.form.selected("source").and_then(non_empty);
    app.modal = None;
    let dlq = &mut app.event_screens.dlq;
    dlq.statuses = statuses;
    dlq.window = window;
    if source.is_some() {
        dlq.source_id = source;
    }
    dlq.pane = DlqPane::Summary;
    dlq.summary_cursor = 0;
    dlq.entries_cursor = 0;
    app.enter()
}

/// `R` on the DLQ: resend the selected connection's dead-lettered deliveries.
pub fn open_bulk_resend(app: &mut App) {
    let Some(summary) = app.selected_dlq_summary().cloned() else {
        app.toast("Nothing to resend", Tone::Warning);
        return;
    };
    let items = vec![
        CheckItem::new("exhausted", "exhausted", true)
            .with_note(format!("{} in the DLQ", summary.exhausted_count)),
        CheckItem::new("failed", "failed", false)
            .with_note(format!("{} in the DLQ", summary.failed_count)),
    ];
    app.modal = Some(ModalForm {
        form: Form::new(
            format!("Resend {}", summary.destination_name),
            vec![
                Field::checklist("statuses", "Statuses", items),
                Field::text("since", "Since (optional)", ""),
                Field::text("until", "Until (optional)", ""),
            ],
        ),
        purpose: FormPurpose::BulkResend {
            connection_id: summary.connection_id,
            destination_name: summary.destination_name,
            exhausted: summary.exhausted_count,
            failed: summary.failed_count,
        },
    });
}

pub fn submit_bulk_resend(
    app: &mut App,
    connection_id: String,
    destination_name: String,
    exhausted: i64,
    failed: i64,
) -> Vec<Effect> {
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    let form = &mut modal.form;
    let statuses = form.checked("statuses");
    let since = non_empty(form.text("since"));
    let until = non_empty(form.text("until"));
    let mut valid = true;
    if statuses.is_empty() {
        form.set_field_error("statuses", "Pick at least one status");
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
    let count: i64 = statuses
        .iter()
        .map(|status| match status.as_str() {
            "exhausted" => exhausted,
            "failed" => failed,
            _ => 0,
        })
        .sum();
    app.modal = None;
    app.confirm = Some(Confirm::about(
        "Resend ",
        format!(
            "{count} dead-lettered deliver{}",
            if count == 1 { "y" } else { "ies" }
        ),
        format!(" to {destination_name}?"),
        ConfirmAction::ResendBulk {
            connection_id,
            statuses,
            since,
            until,
        },
    ));
    Vec::new()
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

    use crate::tui::action::Mutation;
    use crate::tui::events_state::EventPane;

    #[test]
    fn tab_cycles_the_panes_and_keys_act_on_the_focused_one() {
        let mut app = fixtures::event_detail_app();
        app.event_screens.detail.headers_limit.set(5);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.event_screens.detail.headers_scroll, 5);
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.event_screens.detail.headers_scroll, 0);
        assert!(!app.pending_jump, "g means top inside a text pane");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.event_screens.detail.pane, EventPane::Deliveries);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.event_screens.detail.expanded.as_deref(),
            Some("0198c9f0-0000-7000-8000-0000000000f2")
        );
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.event_screens.detail.expanded, None);
        press(&mut app, KeyCode::Char('w'));
        assert!(app.event_screens.detail.wrap);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Events);
    }

    #[test]
    fn slash_filters_headers_without_triggering_hotkeys() {
        let mut app = fixtures::event_detail_app();
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('/'));
        assert_eq!(app.event_screens.detail.pane, EventPane::Headers);
        type_text(&mut app, "quser");
        assert!(!app.quit);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.event_screens.detail.header_filter.as_deref(),
            Some("quser")
        );
        press(&mut app, KeyCode::Char('/'));
        chord(&mut app, 'u');
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.event_screens.detail.header_filter, None);
    }

    #[test]
    fn replay_asks_for_connections_then_confirmation() {
        let mut app = fixtures::event_detail_app();
        press(&mut app, KeyCode::Char('R'));
        let modal = app.modal.as_ref().expect("the replay form opens");
        assert_eq!(modal.form.title, "Replay evt_8f2a1b");
        assert_eq!(
            modal.form.checked("connections"),
            vec![
                fixtures::STRIPE_BILLING_ID.to_string(),
                fixtures::STRIPE_AUDIT_ID.to_string()
            ]
        );
        chord(&mut app, 's');
        assert!(app.modal.is_none());
        assert_eq!(
            app.confirm.as_ref().unwrap().text(),
            "Replay evt_8f2a1b to 2 connections?"
        );
        let effects = press(&mut app, KeyCode::Char('y'));
        assert_eq!(
            effects,
            vec![Effect::Mutate {
                mutation: Mutation::ReplayEvent {
                    event_id: fixtures::EVENT_ID.into(),
                    connection_ids: vec![
                        fixtures::STRIPE_BILLING_ID.into(),
                        fixtures::STRIPE_AUDIT_ID.into()
                    ],
                }
            }]
        );
    }

    #[test]
    fn replay_needs_a_connection_and_loaded_connections() {
        let mut app = fixtures::event_detail_app();
        press(&mut app, KeyCode::Char('R'));
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' '));
        chord(&mut app, 's');
        assert_eq!(
            app.modal.as_ref().unwrap().form.fields[0].error.as_deref(),
            Some("Pick at least one connection")
        );

        let mut loading = fixtures::event_detail_app();
        loading.data.source_connections = Default::default();
        press(&mut loading, KeyCode::Char('R'));
        assert!(loading.modal.is_none());
        assert_eq!(
            loading.toasts.last().unwrap().text,
            "Connections are still loading; try again in a moment"
        );
    }

    use crate::tui::events_state::DlqPane;

    #[test]
    fn dlq_panes_move_and_open_events() {
        let mut app = fixtures::dlq_app();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.event_screens.dlq.pane, DlqPane::Entries);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.event_screens.dlq.entries_cursor, 1);
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.event_screens.dlq.pane, DlqPane::Summary);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.event_screens.dlq.summary_cursor, 1);
        assert_eq!(app.event_screens.dlq.entries_cursor, 0);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.screen,
            Screen::EventDetail {
                id: "0198c9f0-0000-7000-8000-0000000000e3".into()
            }
        );
    }

    #[test]
    fn esc_on_the_dlq_screen_returns_to_the_summary_then_the_sidebar() {
        let mut app = fixtures::dlq_app();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.event_screens.dlq.pane, DlqPane::Summary);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, crate::tui::app::Focus::Sidebar);
    }

    #[test]
    fn dlq_filters_pick_the_source_statuses_and_window() {
        let mut app = fixtures::dlq_app();
        press(&mut app, KeyCode::Char('F'));
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Right);
        let effects = chord(&mut app, 's');
        assert!(app.modal.is_none());
        let dlq = &app.event_screens.dlq;
        assert_eq!(dlq.source_id.as_deref(), Some(fixtures::GITHUB_ID));
        assert_eq!(dlq.statuses, vec!["failed".to_string()]);
        assert_eq!(dlq.window, TimeWindow::LastHour);
        assert!(fetched_requests(&effects).contains(&Request::DlqEntries {
            source_id: fixtures::GITHUB_ID.into(),
            statuses: vec!["failed".into()],
            window: TimeWindow::LastHour,
        }));
    }

    #[test]
    fn dlq_filters_need_a_status() {
        let mut app = fixtures::dlq_app();
        press(&mut app, KeyCode::Char('F'));
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' '));
        chord(&mut app, 's');
        assert_eq!(
            app.modal.as_ref().unwrap().form.fields[1].error.as_deref(),
            Some("Pick at least one status")
        );
    }

    #[test]
    fn bulk_resend_confirms_the_count_from_the_summary() {
        let mut app = fixtures::dlq_app();
        press(&mut app, KeyCode::Char('R'));
        let form = &app.modal.as_ref().unwrap().form;
        assert_eq!(form.title, "Resend billing-worker");
        assert_eq!(form.checked("statuses"), vec!["exhausted".to_string()]);
        chord(&mut app, 's');
        assert_eq!(
            app.confirm.as_ref().unwrap().text(),
            "Resend 4 dead-lettered deliveries to billing-worker?"
        );
        let effects = press(&mut app, KeyCode::Char('y'));
        assert_eq!(
            effects,
            vec![Effect::Mutate {
                mutation: Mutation::ResendBulk {
                    connection_id: fixtures::STRIPE_BILLING_ID.into(),
                    statuses: vec!["exhausted".into()],
                    since: None,
                    until: None,
                }
            }]
        );
    }

    #[test]
    fn bulk_resend_checks_its_time_bounds() {
        let mut app = fixtures::dlq_app();
        press(&mut app, KeyCode::Char('R'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "last week");
        chord(&mut app, 's');
        let form = &app.modal.as_ref().unwrap().form;
        assert_eq!(form.fields[1].error.as_deref(), Some(TIME_HINT));
        assert_eq!(
            form.checked("statuses"),
            vec!["exhausted".to_string(), "failed".to_string()]
        );
    }

    #[test]
    fn esc_in_a_source_dlq_tab_goes_back_to_the_list() {
        let mut app = fixtures::source_detail(crate::tui::screen::SourceTab::Dlq);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Sources);
    }
}
