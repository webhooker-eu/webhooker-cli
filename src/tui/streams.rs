//! The live tail: at most one `/sources/{id}/tail` stream, for the visible
//! screen. Notices from an older subscription are dropped; connection changes
//! become `TailStatus` state instead of output.

use crate::sse::{self, SseEvent, StreamStatus};
use crate::tui::action::{Action, Effect};
use crate::tui::app::App;
use crate::tui::budget::{Decision, Priority};
use crate::tui::events_state::{TailStatus, TailSubscription, LIVE_CAPACITY};
use crate::tui::model::{EventSummary, TailNotice};
use crate::tui::screen::{Screen, SourceTab};
use crate::tui::worker::Worker;

pub fn tail_path(source_id: &str) -> String {
    format!("/api/v1/sources/{source_id}/tail")
}

pub fn parse_notice(event: &SseEvent) -> Option<TailNotice> {
    if event.event != "event" {
        return None;
    }
    serde_json::from_str(&event.data).ok()
}

/// The source whose tail the visible screen needs. The unfiltered Events
/// screen polls instead of opening one tail per source.
pub fn wanted_source(app: &App) -> Option<String> {
    match &app.screen {
        Screen::SourceDetail {
            id,
            tab: SourceTab::Live | SourceTab::Events,
        } => Some(id.clone()),
        Screen::Events => app.event_screens.global.filter.source_id.clone(),
        _ => None,
    }
}

/// Called at the end of `enter`: opens, switches or closes the tail.
pub fn sync(app: &mut App) -> Vec<Effect> {
    let wanted = wanted_source(app);
    let current = app
        .event_screens
        .tail
        .as_ref()
        .map(|tail| tail.source_id.clone());
    if wanted == current {
        return Vec::new();
    }
    match wanted {
        None => {
            app.event_screens.tail = None;
            vec![Effect::UnsubscribeTail]
        }
        Some(source_id) => {
            app.event_screens.next_subscription += 1;
            let subscription = app.event_screens.next_subscription;
            app.event_screens.tail = Some(TailSubscription {
                source_id: source_id.clone(),
                subscription,
                status: TailStatus::Connecting,
            });
            app.event_screens.live = Default::default();
            vec![Effect::SubscribeTail {
                source_id,
                subscription,
            }]
        }
    }
}

fn is_current(app: &App, subscription: u64) -> bool {
    app.event_screens
        .tail
        .as_ref()
        .is_some_and(|tail| tail.subscription == subscription)
}

pub fn on_notice(app: &mut App, subscription: u64, notice: TailNotice) -> Vec<Effect> {
    if !is_current(app, subscription) {
        return Vec::new();
    }
    let row = EventSummary::from_notice(&notice);
    let live = &mut app.event_screens.live;
    if !live.rows.iter().any(|existing| existing.id == row.id) {
        live.rows.insert(0, row.clone());
        live.rows.truncate(LIVE_CAPACITY);
        if !live.follow && live.rows.len() > 1 {
            live.cursor = (live.cursor + 1).min(live.rows.len() - 1);
        }
    }
    let accepts = app
        .visible_events_filter()
        .is_some_and(|filter| filter.accepts(&notice));
    if accepts && app.visible_events_page() == 1 {
        if let Some(page) = app.data.events.value.as_mut() {
            if !page.items.iter().any(|existing| existing.id == row.id) {
                page.items.insert(0, row);
                page.total = page.total.map(|total| total + 1);
            }
        }
    }
    Vec::new()
}

pub fn on_status(app: &mut App, subscription: u64, status: StreamStatus) -> Vec<Effect> {
    if !is_current(app, subscription) {
        return Vec::new();
    }
    if let Some(tail) = app.event_screens.tail.as_mut() {
        tail.status = match status {
            StreamStatus::Connected => TailStatus::Live,
            StreamStatus::Reconnecting { reason, .. } => TailStatus::Reconnecting { reason },
            StreamStatus::Fatal(message) => TailStatus::Failed(message),
        };
    }
    Vec::new()
}

/// Runs in the worker until aborted. Opening the stream counts as one
/// request; reconnects happen inside `sse::run_stream` with its backoff.
pub async fn run_tail(worker: Worker, source_id: String, subscription: u64) {
    let actions = worker.actions();
    loop {
        match worker.acquire(Priority::FirstLoad) {
            Decision::Go | Decision::Skip => break,
            Decision::Wait(delay) => tokio::time::sleep(delay).await,
        }
    }
    let Some(client) = worker.client() else {
        let _ = actions.send(Action::TailStatus {
            subscription,
            status: StreamStatus::Fatal("not logged in".to_string()),
        });
        return;
    };
    let notices = actions.clone();
    let statuses = actions;
    let _ = sse::run_stream(
        &client,
        &tail_path(&source_id),
        move |event| {
            if let Some(notice) = parse_notice(&event) {
                let _ = notices.send(Action::TailNotice {
                    subscription,
                    notice,
                });
            }
        },
        move |status| {
            let _ = statuses.send(Action::TailStatus {
                subscription,
                status,
            });
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::update;
    use crate::tui::events_state::EventsState;
    use crate::tui::fixtures;
    use crate::tui::model::Page;
    use crate::tui::screen::Section;
    use std::time::Duration;

    fn notice(public_id: &str) -> TailNotice {
        TailNotice {
            event_id: format!("id-{public_id}"),
            public_id: public_id.into(),
            source_id: fixtures::STRIPE_ID.into(),
            method: "POST".into(),
            received_at: "2026-09-23T12:05:00Z".into(),
            content_type: Some("application/json".into()),
            body_size: 10,
            verification_status: "verified".into(),
        }
    }

    fn subscription(app: &App) -> u64 {
        app.event_screens.tail.as_ref().unwrap().subscription
    }

    #[test]
    fn only_event_frames_are_notices() {
        let data = serde_json::to_string(&serde_json::json!({
            "event_id": "e1", "public_id": "evt_1", "source_id": "s1", "method": "POST",
            "received_at": "2026-09-23T12:05:00Z", "content_type": null, "body_size": 2,
            "verification_status": "verified"
        }))
        .unwrap();
        let event = SseEvent {
            event: "event".into(),
            data: data.clone(),
        };
        assert_eq!(parse_notice(&event).unwrap().public_id, "evt_1");
        let other = SseEvent {
            event: "message".into(),
            data,
        };
        assert!(parse_notice(&other).is_none());
        assert_eq!(tail_path("s1"), "/api/v1/sources/s1/tail");
    }

    #[test]
    fn the_live_tab_subscribes_and_leaving_unsubscribes() {
        let mut app = fixtures::app();
        let effects = app.open(Screen::SourceDetail {
            id: fixtures::STRIPE_ID.into(),
            tab: SourceTab::Live,
        });
        assert!(effects.contains(&Effect::SubscribeTail {
            source_id: fixtures::STRIPE_ID.into(),
            subscription: 1,
        }));
        let staying = app.set_source_tab(SourceTab::Events);
        assert!(!staying.iter().any(|effect| matches!(
            effect,
            Effect::SubscribeTail { .. } | Effect::UnsubscribeTail
        )));
        let leaving = app.back();
        assert!(leaving.contains(&Effect::UnsubscribeTail));
        assert!(app.event_screens.tail.is_none());
    }

    #[test]
    fn the_events_screen_tails_only_when_filtered_to_one_source() {
        let mut app = fixtures::app();
        let unfiltered = app.switch_to(Section::Events);
        assert!(!unfiltered
            .iter()
            .any(|effect| matches!(effect, Effect::SubscribeTail { .. })));
        app.event_screens.global.filter.source_id = Some(fixtures::GITHUB_ID.into());
        let filtered = app.enter();
        assert!(filtered.contains(&Effect::SubscribeTail {
            source_id: fixtures::GITHUB_ID.into(),
            subscription: 1,
        }));
    }

    #[test]
    fn notices_fill_the_live_feed_newest_first() {
        let mut app = fixtures::app();
        app.open(Screen::SourceDetail {
            id: fixtures::STRIPE_ID.into(),
            tab: SourceTab::Live,
        });
        let current = subscription(&app);
        update(
            &mut app,
            Action::TailNotice {
                subscription: current,
                notice: notice("evt_1"),
            },
        );
        update(
            &mut app,
            Action::TailNotice {
                subscription: current,
                notice: notice("evt_2"),
            },
        );
        update(
            &mut app,
            Action::TailNotice {
                subscription: current,
                notice: notice("evt_2"),
            },
        );
        let rows: Vec<&str> = app
            .event_screens
            .live
            .rows
            .iter()
            .map(|row| row.public_id.as_str())
            .collect();
        assert_eq!(rows, vec!["evt_2", "evt_1"]);
        assert_eq!(app.event_screens.live.cursor, 0);
    }

    #[test]
    fn without_follow_the_selection_stays_on_its_row() {
        let mut app = fixtures::app();
        app.open(Screen::SourceDetail {
            id: fixtures::STRIPE_ID.into(),
            tab: SourceTab::Live,
        });
        let current = subscription(&app);
        update(
            &mut app,
            Action::TailNotice {
                subscription: current,
                notice: notice("evt_1"),
            },
        );
        app.event_screens.live.follow = false;
        update(
            &mut app,
            Action::TailNotice {
                subscription: current,
                notice: notice("evt_2"),
            },
        );
        assert_eq!(app.event_screens.live.cursor, 1);
    }

    #[test]
    fn notices_from_an_old_subscription_are_dropped() {
        let mut app = fixtures::app();
        app.open(Screen::SourceDetail {
            id: fixtures::STRIPE_ID.into(),
            tab: SourceTab::Live,
        });
        let old = subscription(&app);
        app.back();
        app.open(Screen::SourceDetail {
            id: fixtures::GITHUB_ID.into(),
            tab: SourceTab::Live,
        });
        update(
            &mut app,
            Action::TailNotice {
                subscription: old,
                notice: notice("evt_1"),
            },
        );
        assert!(app.event_screens.live.rows.is_empty());
    }

    #[test]
    fn notices_join_page_one_of_a_matching_events_list() {
        let mut app = fixtures::app();
        app.open(Screen::SourceDetail {
            id: fixtures::STRIPE_ID.into(),
            tab: SourceTab::Events,
        });
        let now = app.now;
        app.data.events.finish(
            Page {
                items: Vec::new(),
                total: Some(0),
            },
            now,
        );
        let current = subscription(&app);
        update(
            &mut app,
            Action::TailNotice {
                subscription: current,
                notice: notice("evt_1"),
            },
        );
        let page = app.data.events.value.as_ref().unwrap();
        assert_eq!(page.items[0].public_id, "evt_1");
        assert_eq!(page.items[0].delivery_count, None);
        assert_eq!(page.total, Some(1));

        app.event_screens.source = EventsState {
            page: 2,
            ..EventsState::default()
        };
        update(
            &mut app,
            Action::TailNotice {
                subscription: current,
                notice: notice("evt_2"),
            },
        );
        assert_eq!(app.data.events.value.as_ref().unwrap().items.len(), 1);
    }

    #[test]
    fn stream_status_is_state_not_output() {
        let mut app = fixtures::app();
        app.open(Screen::SourceDetail {
            id: fixtures::STRIPE_ID.into(),
            tab: SourceTab::Live,
        });
        let current = subscription(&app);
        let status = |app: &App| app.event_screens.tail.as_ref().unwrap().status.clone();
        assert_eq!(status(&app), TailStatus::Connecting);
        update(
            &mut app,
            Action::TailStatus {
                subscription: current,
                status: StreamStatus::Connected,
            },
        );
        assert_eq!(status(&app), TailStatus::Live);
        update(
            &mut app,
            Action::TailStatus {
                subscription: current,
                status: StreamStatus::Reconnecting {
                    reason: "stream closed".into(),
                    retry_in: Duration::from_secs(1),
                    server_message: None,
                },
            },
        );
        assert_eq!(
            status(&app),
            TailStatus::Reconnecting {
                reason: "stream closed".into()
            }
        );
        update(
            &mut app,
            Action::TailStatus {
                subscription: current,
                status: StreamStatus::Fatal("source not found on the server".into()),
            },
        );
        assert_eq!(
            status(&app),
            TailStatus::Failed("source not found on the server".into())
        );
    }
}
