//! The app side of the relay: applying updates from the relay task, the
//! start and retarget forms, the Relay screen keys, and stopping.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::relay::{RelayEvent, RelayOutcome, RelayRecord};
use crate::sse::StreamStatus;
use crate::tui::action::Effect;
use crate::tui::app::{App, Focus, FormPurpose, ModalForm};
use crate::tui::forms::form::{Field, Form, SelectOption};
use crate::tui::forms::input::TextInput;
use crate::tui::relay_session::{
    LocalReplay, RelayConnection, RelayStart, RelayState, RelayUpdate,
};
use crate::tui::screen::{Screen, Section};
use crate::tui::settings::is_http_url;
use crate::tui::theme::{Glyphs, Tone};

/// How `sse::action_for_status` words a refused stream slot (HTTP 403).
pub const NO_SLOT_PREFIX: &str = "no live stream slot";
pub const SLOT_HINT: &str = "Close other whk listen sessions or dashboard live tabs";

pub fn on_update(app: &mut App, session: u64, update: RelayUpdate) -> Vec<Effect> {
    let glyphs = app.theme.glyphs;
    let Some(relay) = app.relay.as_mut().filter(|relay| relay.session == session) else {
        return Vec::new();
    };
    let toast = match update {
        RelayUpdate::Status(StreamStatus::Connected) => {
            relay.connection = RelayConnection::Connected;
            None
        }
        RelayUpdate::Status(StreamStatus::Reconnecting {
            reason,
            server_message,
            ..
        }) => {
            let repeated = matches!(
                &relay.connection,
                RelayConnection::Reconnecting { server_message: previous, .. }
                    if *previous == server_message
            );
            let toast = match &server_message {
                Some(message) if reason.starts_with(NO_SLOT_PREFIX) && !repeated => {
                    Some((format!("{message}. {SLOT_HINT}"), Tone::Danger))
                }
                _ => None,
            };
            relay.connection = RelayConnection::Reconnecting {
                reason,
                server_message,
            };
            toast
        }
        RelayUpdate::Status(StreamStatus::Fatal(message)) | RelayUpdate::Ended(Some(message)) => {
            let first = !matches!(relay.connection, RelayConnection::Failed(_));
            relay.connection = RelayConnection::Failed(message.clone());
            first.then(|| (format!("Relay stopped: {message}"), Tone::Danger))
        }
        RelayUpdate::Ended(None) => {
            relay.connection = RelayConnection::Failed("the stream ended".to_string());
            None
        }
        RelayUpdate::Event(RelayEvent::MalformedFrame(error)) => Some((
            format!("Relay skipped a malformed webhook: {error}"),
            Tone::Warning,
        )),
        RelayUpdate::Event(RelayEvent::Record(record)) => {
            let toast = record_toast(&record);
            relay.push(record);
            toast
        }
        RelayUpdate::Replayed(record) => {
            let toast = replay_toast(&record, glyphs);
            relay.push(record);
            toast
        }
    };
    if let Some((text, tone)) = toast {
        app.toast(text, tone);
    }
    Vec::new()
}

/// Failed forwards and drops show on any screen.
fn record_toast(record: &RelayRecord) -> Option<(String, Tone)> {
    let public_id = &record.frame.public_id;
    match &record.outcome {
        RelayOutcome::Failed(error) => {
            Some((format!("Relay: {public_id} failed: {error}"), Tone::Danger))
        }
        RelayOutcome::Dropped => Some((
            format!("Relay dropped {public_id}: the local endpoint is too slow"),
            Tone::Warning,
        )),
        RelayOutcome::Forwarded(_) | RelayOutcome::SkippedUnverified => None,
    }
}

fn replay_toast(record: &RelayRecord, glyphs: &Glyphs) -> Option<(String, Tone)> {
    let public_id = &record.frame.public_id;
    match &record.outcome {
        RelayOutcome::Forwarded(response) => {
            let tone = if response.status < 400 {
                Tone::Success
            } else {
                Tone::Warning
            };
            Some((
                format!("Replayed {public_id} {} {}", glyphs.arrow, response.status),
                tone,
            ))
        }
        RelayOutcome::Failed(error) => Some((
            format!("Replay of {public_id} failed: {error}"),
            Tone::Danger,
        )),
        RelayOutcome::Dropped | RelayOutcome::SkippedUnverified => None,
    }
}

pub const START_TITLE: &str = "Start relay";
pub const RETARGET_TITLE: &str = "Change relay target";
pub const INVALID_URL: &str = "Must be an absolute http(s) URL";

/// The source `L` refers to: the open source, or the selected row.
pub fn source_in_view(app: &App) -> Option<String> {
    match &app.screen {
        Screen::SourceDetail { id, .. } => Some(id.clone()),
        Screen::Sources => app
            .data
            .sources
            .value
            .as_ref()
            .and_then(|sources| sources.get(app.cursors.sources))
            .map(|source| source.id.clone()),
        _ => None,
    }
}

pub fn open_start_form(app: &mut App, source_id: Option<String>) -> Vec<Effect> {
    let options: Vec<SelectOption> = app
        .data
        .sources
        .value
        .iter()
        .flatten()
        .map(|source| SelectOption::new(source.id.clone(), source.name.clone()))
        .collect();
    let target = app
        .last_relay_url
        .clone()
        .unwrap_or_else(|| app.settings.relay_default_url.clone());
    let form = Form::new(
        START_TITLE,
        vec![
            Field::select("source", "Source", options, source_id.as_deref()),
            Field::text("target", "Target URL", target),
            Field::headers("headers", "Extra headers", &[]),
        ],
    );
    app.modal = Some(ModalForm {
        form,
        purpose: FormPurpose::Relay,
    });
    Vec::new()
}

pub fn submit_start(app: &mut App) -> Vec<Effect> {
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    let form = &mut modal.form;
    form.clear_errors();
    // An empty option list still reports `Some("")`.
    let source_id = form.selected("source").filter(|id| !id.is_empty());
    let target = form.text("target").trim().to_string();
    let headers = form.headers("headers");
    let mut valid = true;
    if source_id.is_none() {
        form.set_field_error("source", "Pick a source");
        valid = false;
    }
    if !is_http_url(&target) {
        form.set_field_error("target", INVALID_URL);
        valid = false;
    }
    if let Err(message) = &headers {
        form.set_field_error("headers", message.clone());
        valid = false;
    }
    let (Some(source_id), Ok(extra_headers), true) = (source_id, headers, valid) else {
        return Vec::new();
    };
    app.modal = None;
    start(app, source_id, target, extra_headers)
}

/// One session at a time: a new start replaces the running one.
fn start(
    app: &mut App,
    source_id: String,
    target_url: String,
    extra_headers: Vec<(String, String)>,
) -> Vec<Effect> {
    app.next_relay_session += 1;
    let session = app.next_relay_session;
    let source_name = app.names.source(&source_id);
    app.relay = Some(RelayState::new(
        session,
        source_id.clone(),
        source_name.clone(),
        target_url.clone(),
        extra_headers.clone(),
    ));
    app.last_relay_url = Some(target_url.clone());
    let mut effects = vec![
        Effect::StartRelay(RelayStart {
            session,
            source_id,
            source_name,
            target_url: target_url.clone(),
            extra_headers,
        }),
        Effect::RememberRelayUrl(target_url),
    ];
    effects.extend(app.switch_to(Section::Relay));
    effects
}

pub fn open_target_form(app: &mut App) -> Vec<Effect> {
    let Some(relay) = &app.relay else {
        return Vec::new();
    };
    let form = Form::new(
        RETARGET_TITLE,
        vec![Field::text(
            "target",
            "Target URL",
            relay.target_url.clone(),
        )],
    );
    app.modal = Some(ModalForm {
        form,
        purpose: FormPurpose::RelayTarget,
    });
    Vec::new()
}

pub fn submit_retarget(app: &mut App) -> Vec<Effect> {
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    let form = &mut modal.form;
    form.clear_errors();
    let target = form.text("target").trim().to_string();
    if !is_http_url(&target) {
        form.set_field_error("target", INVALID_URL);
        return Vec::new();
    }
    app.modal = None;
    let Some(relay) = app.relay.as_mut() else {
        return Vec::new();
    };
    relay.target_url = target.clone();
    app.last_relay_url = Some(target.clone());
    app.toast(format!("Relay now forwards to {target}"), Tone::Success);
    vec![
        Effect::RetargetRelay(target.clone()),
        Effect::RememberRelayUrl(target),
    ]
}

pub fn stop(app: &mut App) -> Vec<Effect> {
    if app.relay.take().is_none() {
        return Vec::new();
    }
    app.toast("Relay stopped", Tone::Muted);
    vec![Effect::StopRelay]
}

fn replay_selected(app: &mut App) -> Vec<Effect> {
    let Some(relay) = &app.relay else {
        return Vec::new();
    };
    let Some(record) = relay.selected() else {
        return Vec::new();
    };
    vec![Effect::ReplayLocally(LocalReplay {
        session: relay.session,
        frame: record.frame.clone(),
        target_url: relay.target_url.clone(),
        extra_headers: relay.extra_headers.clone(),
    })]
}

pub fn search_active(app: &App) -> bool {
    app.screen == Screen::Relay
        && app
            .relay
            .as_ref()
            .is_some_and(|relay| relay.search.is_some())
}

pub fn on_search_key(app: &mut App, key: KeyEvent) -> Vec<Effect> {
    let Some(relay) = app.relay.as_mut() else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => relay.search = None,
        KeyCode::Enter => {
            let query = relay
                .search
                .take()
                .map(|input| input.value().trim().to_string())
                .unwrap_or_default();
            relay.query = (!query.is_empty()).then_some(query);
            relay.cursor = 0;
        }
        _ => {
            if let Some(input) = relay.search.as_mut() {
                input.handle(key);
            }
        }
    }
    Vec::new()
}

/// Relay screen keys while the main pane has focus.
pub fn on_key(app: &mut App, code: KeyCode) -> Vec<Effect> {
    if code == KeyCode::Char('n') {
        return open_start_form(app, None);
    }
    let Some(relay) = app.relay.as_mut() else {
        return leave(app, code);
    };
    let last = relay.visible().len().saturating_sub(1);
    match code {
        KeyCode::Up | KeyCode::Char('k') => relay.cursor = relay.cursor.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => relay.cursor = (relay.cursor + 1).min(last),
        KeyCode::Home => relay.cursor = 0,
        KeyCode::End => relay.cursor = last,
        KeyCode::Enter => relay.expanded = !relay.expanded,
        KeyCode::Esc if relay.expanded => relay.expanded = false,
        KeyCode::Char('/') => {
            relay.search = Some(TextInput::new(
                relay.query.clone().unwrap_or_default(),
                false,
            ))
        }
        KeyCode::Char('p') => return replay_selected(app),
        KeyCode::Char('u') => return open_target_form(app),
        KeyCode::Char('x') => return stop(app),
        other => return leave(app, other),
    }
    Vec::new()
}

fn leave(app: &mut App, code: KeyCode) -> Vec<Effect> {
    if matches!(
        code,
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') | KeyCode::Tab | KeyCode::BackTab
    ) {
        app.focus = Focus::Sidebar;
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::RelayOutcome;
    use crate::tui::fixtures::{self, forwarded, relay_record};
    use std::time::Duration;

    const RECEIVED_AT: &str = "2026-09-23T12:05:00Z";

    fn relay(app: &App) -> &RelayState {
        app.relay.as_ref().unwrap()
    }

    fn record_update(outcome: RelayOutcome) -> RelayUpdate {
        RelayUpdate::Event(RelayEvent::Record(relay_record(
            "evt_new",
            RECEIVED_AT,
            outcome,
        )))
    }

    fn slot_refused() -> RelayUpdate {
        RelayUpdate::Status(StreamStatus::Reconnecting {
            reason: "no live stream slot: a previous connection's slot may still be releasing, \
                     or the plan's live stream limit is reached"
                .into(),
            retry_in: Duration::from_secs(1),
            server_message: Some("live stream limit reached for your plan (3)".into()),
        })
    }

    #[test]
    fn records_are_added_and_failures_become_toasts() {
        let mut app = fixtures::relay_app();
        on_update(
            &mut app,
            1,
            record_update(RelayOutcome::Failed("connection refused".into())),
        );
        assert_eq!(relay(&app).records.len(), 4);
        assert_eq!(relay(&app).errors, 2);
        let toast = app.toasts.last().unwrap();
        assert_eq!(toast.text, "Relay: evt_new failed: connection refused");
        assert_eq!(toast.tone, Tone::Danger);
    }

    #[test]
    fn drops_warn_and_successes_stay_quiet() {
        let mut app = fixtures::relay_app();
        on_update(&mut app, 1, record_update(forwarded(200, 3, "ok")));
        assert!(app.toasts.is_empty());
        on_update(&mut app, 1, record_update(RelayOutcome::Dropped));
        let toast = app.toasts.last().unwrap();
        assert_eq!(
            toast.text,
            "Relay dropped evt_new: the local endpoint is too slow"
        );
        assert_eq!(toast.tone, Tone::Warning);
    }

    #[test]
    fn updates_from_an_older_session_are_ignored() {
        let mut app = fixtures::relay_app();
        on_update(&mut app, 99, record_update(RelayOutcome::Dropped));
        assert_eq!(relay(&app).records.len(), 3);
        assert!(app.toasts.is_empty());
    }

    #[test]
    fn a_refused_stream_slot_shows_the_server_message_once() {
        let mut app = fixtures::relay_app();
        on_update(&mut app, 1, slot_refused());
        assert!(matches!(
            relay(&app).connection,
            RelayConnection::Reconnecting {
                server_message: Some(_),
                ..
            }
        ));
        assert_eq!(
            app.toasts.last().unwrap().text,
            "live stream limit reached for your plan (3). \
             Close other whk listen sessions or dashboard live tabs"
        );
        on_update(&mut app, 1, slot_refused());
        assert_eq!(
            app.toasts.len(),
            1,
            "a repeated refusal is not toasted again"
        );
    }

    #[test]
    fn ordinary_reconnects_are_silent_and_connected_clears_them() {
        let mut app = fixtures::relay_app();
        on_update(
            &mut app,
            1,
            RelayUpdate::Status(StreamStatus::Reconnecting {
                reason: "stream closed".into(),
                retry_in: Duration::from_secs(1),
                server_message: None,
            }),
        );
        assert!(app.toasts.is_empty());
        on_update(&mut app, 1, RelayUpdate::Status(StreamStatus::Connected));
        assert_eq!(relay(&app).connection, RelayConnection::Connected);
    }

    #[test]
    fn a_fatal_stream_error_is_reported_once_and_keeps_the_records() {
        let mut app = fixtures::relay_app();
        on_update(
            &mut app,
            1,
            RelayUpdate::Status(StreamStatus::Fatal("source not found on the server".into())),
        );
        on_update(
            &mut app,
            1,
            RelayUpdate::Ended(Some("source not found on the server".into())),
        );
        assert_eq!(
            relay(&app).connection,
            RelayConnection::Failed("source not found on the server".into())
        );
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(relay(&app).records.len(), 3);
    }

    #[test]
    fn a_local_replay_is_recorded_with_a_toast() {
        let mut app = fixtures::relay_app();
        on_update(
            &mut app,
            1,
            RelayUpdate::Replayed(relay_record(
                "evt_77c1d05e9a",
                RECEIVED_AT,
                forwarded(200, 8, "ok"),
            )),
        );
        assert_eq!(relay(&app).records[0].frame.public_id, "evt_77c1d05e9a");
        assert_eq!(
            app.toasts.last().unwrap().text,
            "Replayed evt_77c1d05e9a → 200"
        );
        assert_eq!(app.toasts.last().unwrap().tone, Tone::Success);
    }

    #[test]
    fn malformed_frames_warn() {
        let mut app = fixtures::relay_app();
        on_update(
            &mut app,
            1,
            RelayUpdate::Event(RelayEvent::MalformedFrame("missing field `id`".into())),
        );
        assert_eq!(app.toasts.last().unwrap().tone, Tone::Warning);
    }

    #[test]
    fn the_app_routes_relay_actions() {
        let mut app = fixtures::relay_app();
        crate::tui::app::update(
            &mut app,
            crate::tui::action::Action::Relay {
                session: 1,
                update: RelayUpdate::Status(StreamStatus::Connected),
            },
        );
        assert_eq!(relay(&app).connection, RelayConnection::Connected);
    }

    use crate::tui::action::Action;
    use crate::tui::app::update;
    use crate::tui::forms::form::FieldKind;
    use crate::tui::screen::SourceTab;
    use ratatui::crossterm::event::KeyModifiers;

    fn press(app: &mut App, code: KeyCode) -> Vec<Effect> {
        update(app, Action::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn set_text(app: &mut App, key: &str, value: &str) {
        let form = &mut app.modal.as_mut().unwrap().form;
        for field in &mut form.fields {
            if field.key == key {
                if let FieldKind::Text(input) = &mut field.kind {
                    input.set(value);
                }
            }
        }
    }

    fn field_error(app: &App, key: &str) -> Option<String> {
        app.modal
            .as_ref()
            .unwrap()
            .form
            .fields
            .iter()
            .find(|field| field.key == key)
            .and_then(|field| field.error.clone())
    }

    #[test]
    fn l_on_the_sources_list_opens_the_form_for_the_selected_source() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('L'));
        let modal = app.modal.as_ref().expect("the relay form");
        assert_eq!(modal.purpose, FormPurpose::Relay);
        assert_eq!(
            modal.form.selected("source").as_deref(),
            Some(fixtures::GITHUB_ID)
        );
        assert_eq!(modal.form.text("target"), "http://localhost:3000");
    }

    #[test]
    fn l_on_a_source_detail_preselects_that_source_and_the_last_url_wins() {
        let mut app = fixtures::source_detail(SourceTab::Overview);
        app.last_relay_url = Some("http://localhost:4000".into());
        press(&mut app, KeyCode::Char('L'));
        let form = &app.modal.as_ref().unwrap().form;
        assert_eq!(
            form.selected("source").as_deref(),
            Some(fixtures::STRIPE_ID)
        );
        assert_eq!(form.text("target"), "http://localhost:4000");
    }

    #[test]
    fn submitting_starts_the_relay_and_opens_the_inspector() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('L'));
        let effects = submit_start(&mut app);
        assert!(app.modal.is_none());
        assert_eq!(app.screen, Screen::Relay);
        assert_eq!(app.last_relay_url.as_deref(), Some("http://localhost:3000"));
        let relay = relay(&app);
        assert_eq!(relay.session, 1);
        assert_eq!(relay.source_name, "stripe-prod");
        assert!(effects.contains(&Effect::StartRelay(RelayStart {
            session: 1,
            source_id: fixtures::STRIPE_ID.into(),
            source_name: "stripe-prod".into(),
            target_url: "http://localhost:3000".into(),
            extra_headers: vec![],
        })));
        assert!(effects.contains(&Effect::RememberRelayUrl("http://localhost:3000".into())));
    }

    #[test]
    fn ctrl_s_in_the_form_submits_through_submit_modal() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('L'));
        let effects = update(
            &mut app,
            Action::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartRelay(_))));
    }

    #[test]
    fn an_invalid_target_keeps_the_form_open_with_the_error() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('L'));
        set_text(&mut app, "target", "localhost:3000");
        assert!(submit_start(&mut app).is_empty());
        assert!(app.relay.is_none());
        assert_eq!(
            field_error(&app, "target").as_deref(),
            Some("Must be an absolute http(s) URL")
        );
    }

    #[test]
    fn a_second_start_replaces_the_session() {
        let mut app = fixtures::relay_app();
        open_start_form(&mut app, Some(fixtures::GITHUB_ID.into()));
        submit_start(&mut app);
        assert_eq!(relay(&app).session, 2);
        assert_eq!(relay(&app).source_name, "github-ci");
        assert!(relay(&app).records.is_empty());
    }

    #[test]
    fn inspector_keys_move_expand_replay_and_stop() {
        let mut app = fixtures::relay_app();
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(relay(&app).cursor, 0);
        press(&mut app, KeyCode::Enter);
        assert!(relay(&app).expanded);
        press(&mut app, KeyCode::Esc);
        assert!(!relay(&app).expanded);
        press(&mut app, KeyCode::Char('j'));
        let effects = press(&mut app, KeyCode::Char('p'));
        let [Effect::ReplayLocally(LocalReplay {
            session,
            frame,
            target_url,
            ..
        })] = effects.as_slice()
        else {
            panic!("{effects:?}");
        };
        assert_eq!(*session, 1);
        assert_eq!(frame.public_id, "evt_77c1d05e9a");
        assert_eq!(target_url, "http://localhost:3000");
        let effects = press(&mut app, KeyCode::Char('x'));
        assert_eq!(effects, vec![Effect::StopRelay]);
        assert!(app.relay.is_none());
        assert_eq!(app.toasts.last().unwrap().text, "Relay stopped");
    }

    #[test]
    fn u_changes_the_target_without_restarting() {
        let mut app = fixtures::relay_app();
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(
            app.modal.as_ref().unwrap().purpose,
            FormPurpose::RelayTarget
        );
        set_text(&mut app, "target", "http://localhost:4000/hooks");
        let effects = submit_retarget(&mut app);
        assert_eq!(
            effects,
            vec![
                Effect::RetargetRelay("http://localhost:4000/hooks".into()),
                Effect::RememberRelayUrl("http://localhost:4000/hooks".into()),
            ]
        );
        assert_eq!(relay(&app).target_url, "http://localhost:4000/hooks");
        assert_eq!(relay(&app).session, 1);
        assert!(app.modal.is_none());
    }

    #[test]
    fn slash_searches_and_captures_hotkeys() {
        let mut app = fixtures::relay_app();
        press(&mut app, KeyCode::Char('/'));
        for character in "500x".chars() {
            press(&mut app, KeyCode::Char(character));
        }
        assert!(
            app.relay.is_some(),
            "x typed into the search must not stop the relay"
        );
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Enter);
        assert_eq!(relay(&app).query.as_deref(), Some("500"));
        assert_eq!(relay(&app).visible().len(), 1);
    }

    #[test]
    fn n_on_an_empty_relay_screen_opens_the_form() {
        let mut app = fixtures::app();
        app.switch_to(Section::Relay);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.modal.as_ref().unwrap().purpose, FormPurpose::Relay);
    }

    #[test]
    fn quitting_with_a_live_relay_asks_first() {
        let mut app = fixtures::relay_app();
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.quit);
        assert_eq!(
            app.confirm.as_ref().unwrap().text(),
            "Relay is running. Quit and stop it?"
        );
        let effects = press(&mut app, KeyCode::Char('y'));
        assert!(app.quit);
        assert_eq!(effects, vec![Effect::StopRelay]);
    }

    #[test]
    fn a_second_ctrl_c_quits_without_asking_again() {
        let mut app = fixtures::relay_app();
        let ctrl_c = || Action::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        update(&mut app, ctrl_c());
        assert!(!app.quit);
        update(&mut app, ctrl_c());
        assert!(app.quit);
    }

    #[test]
    fn a_failed_relay_does_not_block_quitting() {
        let mut app = fixtures::relay_app();
        app.relay.as_mut().unwrap().connection = RelayConnection::Failed("gone".into());
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }

    #[test]
    fn a_termination_signal_stops_the_relay_and_quits() {
        let mut app = fixtures::relay_app();
        let effects = update(&mut app, Action::Terminate);
        assert!(app.quit);
        assert!(app.relay.is_none());
        assert_eq!(effects, vec![Effect::StopRelay]);
    }

    #[test]
    fn the_relay_screen_polls_the_sources_for_the_form() {
        let schedule =
            crate::tui::poller::schedule(&Screen::Relay, None, crate::tui::poller::PlanTier::Free);
        assert_eq!(
            schedule,
            vec![(
                crate::tui::action::Request::Sources { search: None },
                Duration::from_secs(60)
            )]
        );
    }

    #[test]
    fn hints_follow_the_session() {
        let mut app = fixtures::app();
        app.switch_to(Section::Relay);
        assert_eq!(crate::tui::hints::hints(&app)[0], ("n", "new relay"));
        let app = fixtures::relay_app();
        assert_eq!(
            crate::tui::hints::hints(&app),
            vec![
                ("p", "replay locally"),
                ("u", "change URL"),
                ("x", "stop"),
                ("enter", "expand"),
                ("/", "search"),
            ]
        );
        assert!(crate::tui::hints::hints(&fixtures::app()).contains(&("L", "relay")));
    }
}
