//! The TUI's background relay: one session at a time, built on
//! `relay::run`, and the ring buffer the inspector shows.

use std::collections::VecDeque;

use crate::relay::{body_bytes, RelayEvent, RelayOutcome, RelayRecord};
use crate::sse::StreamStatus;
use crate::tui::forms::input::TextInput;

/// The inspector keeps the last 500 records.
pub const RING_CAPACITY: usize = 500;

#[derive(Debug, Clone, PartialEq)]
pub enum RelayConnection {
    Connecting,
    Connected,
    Reconnecting {
        reason: String,
        server_message: Option<String>,
    },
    /// The stream gave up (revoked key, deleted source); `x` clears it.
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct RelayState {
    pub session: u64,
    pub source_id: String,
    pub source_name: String,
    pub target_url: String,
    pub extra_headers: Vec<(String, String)>,
    pub connection: RelayConnection,
    /// Newest first.
    pub records: VecDeque<RelayRecord>,
    pub forwarded: usize,
    pub errors: usize,
    /// Index into `visible()`.
    pub cursor: usize,
    /// Request and response fill the whole pane.
    pub expanded: bool,
    /// The search being typed; while set, keys go to it.
    pub search: Option<TextInput>,
    pub query: Option<String>,
}

impl RelayState {
    pub fn new(
        session: u64,
        source_id: String,
        source_name: String,
        target_url: String,
        extra_headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            session,
            source_id,
            source_name,
            target_url,
            extra_headers,
            connection: RelayConnection::Connecting,
            records: VecDeque::new(),
            forwarded: 0,
            errors: 0,
            cursor: 0,
            expanded: false,
            search: None,
            query: None,
        }
    }

    pub fn is_live(&self) -> bool {
        !matches!(self.connection, RelayConnection::Failed(_))
    }

    pub fn push(&mut self, record: RelayRecord) {
        match record.outcome {
            RelayOutcome::Forwarded(_) => self.forwarded += 1,
            RelayOutcome::Failed(_) | RelayOutcome::Dropped => self.errors += 1,
            RelayOutcome::SkippedUnverified => {}
        }
        self.records.push_front(record);
        self.records.truncate(RING_CAPACITY);
        // At the top the newest record stays selected; further down the
        // selection follows the record the user is reading.
        if self.cursor > 0 {
            self.cursor = (self.cursor + 1).min(self.records.len() - 1);
        }
    }

    pub fn visible(&self) -> Vec<&RelayRecord> {
        match self.query.as_deref() {
            None => self.records.iter().collect(),
            Some(query) => {
                let needle = query.to_lowercase();
                self.records
                    .iter()
                    .filter(|record| matches_query(record, &needle))
                    .collect()
            }
        }
    }

    pub fn selected(&self) -> Option<&RelayRecord> {
        self.visible().get(self.cursor).copied()
    }
}

/// What a relay task reports back to the app.
#[derive(Debug, Clone, PartialEq)]
pub enum RelayUpdate {
    Status(StreamStatus),
    Event(RelayEvent),
    /// The result of `p`: a request re-sent to localhost from memory.
    Replayed(RelayRecord),
    /// The task ended; `Some` carries the fatal stream error.
    Ended(Option<String>),
}

fn matches_query(record: &RelayRecord, needle: &str) -> bool {
    let outcome = match &record.outcome {
        RelayOutcome::Forwarded(response) => response.status.to_string(),
        RelayOutcome::Failed(error) => error.clone(),
        RelayOutcome::Dropped => "dropped".to_string(),
        RelayOutcome::SkippedUnverified => "skipped".to_string(),
    };
    [
        record.frame.public_id.as_str(),
        record.frame.method.as_str(),
        outcome.as_str(),
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(needle))
}

/// Size of the webhook body as sent, decoded from base64 when needed.
pub fn body_size(record: &RelayRecord) -> usize {
    body_bytes(&record.frame).map_or(record.frame.body.len(), |bytes| bytes.len())
}

pub fn size_label(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fixtures::{forwarded, relay_record};

    fn state() -> RelayState {
        RelayState::new(
            1,
            "src".into(),
            "stripe-prod".into(),
            "http://localhost:3000".into(),
            vec![],
        )
    }

    #[test]
    fn a_new_session_is_connecting_and_empty() {
        let relay = state();
        assert_eq!(relay.connection, RelayConnection::Connecting);
        assert!(relay.records.is_empty());
        assert!(relay.is_live());
    }

    #[test]
    fn records_are_newest_first_and_capped_at_500() {
        let mut relay = state();
        for index in 0..=RING_CAPACITY {
            relay.push(relay_record(
                &format!("evt_{index}"),
                "2026-09-23T12:00:00Z",
                forwarded(200, 5, "ok"),
            ));
        }
        assert_eq!(relay.records.len(), RING_CAPACITY);
        assert_eq!(relay.records[0].frame.public_id, "evt_500");
        assert_eq!(relay.forwarded, RING_CAPACITY + 1);
        assert_eq!(relay.errors, 0);
    }

    #[test]
    fn failures_and_drops_count_as_errors_but_skips_do_not() {
        let mut relay = state();
        let received_at = "2026-09-23T12:00:00Z";
        relay.push(relay_record(
            "evt_a",
            received_at,
            RelayOutcome::Failed("refused".into()),
        ));
        relay.push(relay_record("evt_b", received_at, RelayOutcome::Dropped));
        relay.push(relay_record(
            "evt_c",
            received_at,
            RelayOutcome::SkippedUnverified,
        ));
        relay.push(relay_record(
            "evt_d",
            received_at,
            forwarded(500, 5, "boom"),
        ));
        assert_eq!(relay.errors, 2);
        assert_eq!(relay.forwarded, 1);
    }

    #[test]
    fn the_selection_stays_on_the_same_record_when_new_ones_arrive() {
        let mut relay = state();
        let received_at = "2026-09-23T12:00:00Z";
        relay.push(relay_record(
            "evt_old",
            received_at,
            forwarded(200, 5, "ok"),
        ));
        relay.push(relay_record(
            "evt_mid",
            received_at,
            forwarded(200, 5, "ok"),
        ));
        relay.cursor = 1;
        assert_eq!(relay.selected().unwrap().frame.public_id, "evt_old");
        relay.push(relay_record(
            "evt_new",
            received_at,
            forwarded(200, 5, "ok"),
        ));
        assert_eq!(relay.selected().unwrap().frame.public_id, "evt_old");
        assert_eq!(relay.cursor, 2);
        let mut following = state();
        following.push(relay_record("evt_1", received_at, forwarded(200, 5, "ok")));
        following.push(relay_record("evt_2", received_at, forwarded(200, 5, "ok")));
        assert_eq!(
            following.cursor, 0,
            "at the top, the newest record stays selected"
        );
    }

    #[test]
    fn the_query_filters_by_public_id_method_status_or_error() {
        let mut relay = state();
        let received_at = "2026-09-23T12:00:00Z";
        relay.push(relay_record("evt_a", received_at, forwarded(200, 5, "ok")));
        relay.push(relay_record(
            "evt_b",
            received_at,
            forwarded(500, 5, "boom"),
        ));
        relay.push(relay_record(
            "evt_c",
            received_at,
            RelayOutcome::Failed("connection refused".into()),
        ));
        relay.query = Some("500".into());
        assert_eq!(relay.visible().len(), 1);
        relay.query = Some("REFUSED".into());
        assert_eq!(relay.visible()[0].frame.public_id, "evt_c");
        relay.query = Some("evt_a".into());
        assert_eq!(relay.selected().unwrap().frame.public_id, "evt_a");
    }

    #[test]
    fn a_failed_session_is_not_live() {
        let mut relay = state();
        relay.connection = RelayConnection::Failed("source not found".into());
        assert!(!relay.is_live());
    }

    #[test]
    fn body_sizes_render_in_bytes_or_kilobytes() {
        assert_eq!(size_label(35), "35 B");
        assert_eq!(size_label(1_229), "1.2 KB");
    }
}
