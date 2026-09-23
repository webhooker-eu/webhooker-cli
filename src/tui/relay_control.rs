//! The app side of the relay: applying updates from the relay task, the
//! start and retarget forms, the Relay screen keys, and stopping.

use crate::relay::{RelayEvent, RelayOutcome, RelayRecord};
use crate::sse::StreamStatus;
use crate::tui::action::Effect;
use crate::tui::app::App;
use crate::tui::relay_session::{RelayConnection, RelayUpdate};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::RelayOutcome;
    use crate::tui::fixtures::{self, forwarded, relay_record};
    use crate::tui::relay_session::RelayState;
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
}
