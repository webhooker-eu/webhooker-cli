//! The TUI's background relay: one session at a time, built on
//! `relay::run`, and the ring buffer the inspector shows.

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::client::ApiClient;
use crate::relay::{
    self, body_bytes, RelayEvent, RelayOptions, RelayOutcome, RelayRecord, WebhookFrame,
    FORWARD_QUEUE_CAPACITY,
};
use crate::sse::StreamStatus;
use crate::tui::action::Action;
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

/// Everything a relay task needs; built from the relay form.
#[derive(Debug, Clone, PartialEq)]
pub struct RelayStart {
    pub session: u64,
    pub source_id: String,
    pub source_name: String,
    pub target_url: String,
    pub extra_headers: Vec<(String, String)>,
}

/// `p`: one recorded request, sent again to the current target.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalReplay {
    pub session: u64,
    pub frame: Arc<WebhookFrame>,
    pub target_url: String,
    pub extra_headers: Vec<(String, String)>,
}

fn relay_action(session: u64, update: RelayUpdate) -> Action {
    Action::Relay { session, update }
}

/// A running relay. Dropping or stopping it aborts the task, which closes the
/// stream and frees the workspace's live stream slot.
pub struct RelayHandle {
    task: JoinHandle<()>,
    retarget: watch::Sender<String>,
}

impl RelayHandle {
    pub fn spawn(
        client: Arc<ApiClient>,
        start: &RelayStart,
        actions: mpsc::UnboundedSender<Action>,
    ) -> Self {
        let (retarget, target) = watch::channel(start.target_url.clone());
        let session = start.session;
        let options = RelayOptions {
            source_id: start.source_id.clone(),
            // Same default as `whk listen`.
            skip_verify: false,
            extra_headers: start.extra_headers.clone(),
            queue_capacity: FORWARD_QUEUE_CAPACITY,
        };
        let task = tokio::spawn(async move {
            let (events, mut received) = mpsc::unbounded_channel::<RelayEvent>();
            let event_actions = actions.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(event) = received.recv().await {
                    if event_actions
                        .send(relay_action(session, RelayUpdate::Event(event)))
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let status_actions = actions.clone();
            let result = relay::run(&client, options, target, events, move |status| {
                let _ = status_actions.send(relay_action(session, RelayUpdate::Status(status)));
            })
            .await;
            let _ = forwarder.await;
            let error = result.err().map(|error| format!("{error:#}"));
            let _ = actions.send(relay_action(session, RelayUpdate::Ended(error)));
        });
        Self { task, retarget }
    }

    /// The next frame goes to `url`; the stream stays connected.
    pub fn retarget(&self, url: String) {
        self.retarget.send_replace(url);
    }

    pub fn target(&self) -> String {
        self.retarget.borrow().clone()
    }

    pub fn stop(&self) {
        self.task.abort();
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for RelayHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Sends a recorded request again, from memory; the server is not involved.
pub async fn replay_locally(replay: LocalReplay) -> RelayRecord {
    let http = reqwest::Client::new();
    let outcome = match relay::forward(
        &http,
        &replay.target_url,
        &replay.frame,
        &replay.extra_headers,
    )
    .await
    {
        Ok(response) => RelayOutcome::Forwarded(response),
        Err(error) => RelayOutcome::Failed(error.to_string()),
    };
    RelayRecord {
        frame: replay.frame,
        target_url: replay.target_url,
        outcome,
    }
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

    use crate::client::ApiClient;
    use crate::tui::action::Action;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn frame_json(public_id: &str) -> String {
        serde_json::json!({
            "id": "0198c9f0-0000-7000-8000-0000000000ff",
            "public_id": public_id,
            "source_id": "src",
            "method": "POST",
            "headers": {"content-type": "application/json"},
            "body": "{}",
            "content_type": "application/json",
            "verification_status": "verified",
            "received_at": "2026-09-23T12:04:11Z"
        })
        .to_string()
    }

    async fn stream_server(public_ids: &[&str]) -> MockServer {
        let server = MockServer::start().await;
        let body: String = public_ids
            .iter()
            .map(|public_id| format!("event: webhook\ndata: {}\n\n", frame_json(public_id)))
            .collect();
        Mock::given(method("GET"))
            .and(path("/api/v1/sources/src/stream"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        server
    }

    fn start(session: u64, target_url: String) -> RelayStart {
        RelayStart {
            session,
            source_id: "src".into(),
            source_name: "stripe-prod".into(),
            target_url,
            extra_headers: vec![("x-env".into(), "local".into())],
        }
    }

    /// Collects relay updates until one matches or five seconds pass.
    async fn wait_for(
        receiver: &mut mpsc::UnboundedReceiver<Action>,
        wanted: impl Fn(&RelayUpdate) -> bool,
    ) -> (u64, RelayUpdate) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let action = tokio::time::timeout_at(deadline, receiver.recv())
                .await
                .expect("timed out waiting for a relay update")
                .expect("the channel must stay open");
            if let Action::Relay { session, update } = action {
                if wanted(&update) {
                    return (session, update);
                }
            }
        }
    }

    #[tokio::test]
    async fn a_session_reports_its_connection_and_forwarded_records() {
        let api = stream_server(&["evt_one"]).await;
        let local = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("x-env", "local"))
            .respond_with(ResponseTemplate::new(201).set_body_string("ok"))
            .mount(&local)
            .await;
        let client = Arc::new(ApiClient::new(api.uri(), "whk_test".into()).unwrap());
        let (actions, mut receiver) = mpsc::unbounded_channel();
        let handle = RelayHandle::spawn(client, &start(7, local.uri()), actions);

        let (session, _) = wait_for(&mut receiver, |update| {
            *update == RelayUpdate::Status(StreamStatus::Connected)
        })
        .await;
        assert_eq!(session, 7);
        let (_, update) = wait_for(&mut receiver, |update| {
            matches!(update, RelayUpdate::Event(RelayEvent::Record(_)))
        })
        .await;
        let RelayUpdate::Event(RelayEvent::Record(record)) = update else {
            unreachable!()
        };
        assert_eq!(record.frame.public_id, "evt_one");
        assert!(matches!(
            record.outcome,
            RelayOutcome::Forwarded(ref response) if response.status == 201
        ));
        handle.stop();
    }

    #[tokio::test]
    async fn retargeting_replaces_the_target_in_place() {
        let api = stream_server(&[]).await;
        let client = Arc::new(ApiClient::new(api.uri(), "whk_test".into()).unwrap());
        let (actions, _receiver) = mpsc::unbounded_channel();
        let handle = RelayHandle::spawn(client, &start(1, "http://localhost:3000".into()), actions);
        handle.retarget("http://localhost:4000".into());
        assert_eq!(handle.target(), "http://localhost:4000");
        handle.stop();
    }

    #[tokio::test]
    async fn stopping_aborts_the_task() {
        let api = stream_server(&[]).await;
        let client = Arc::new(ApiClient::new(api.uri(), "whk_test".into()).unwrap());
        let (actions, _receiver) = mpsc::unbounded_channel();
        let handle = RelayHandle::spawn(client, &start(1, "http://localhost:3000".into()), actions);
        assert!(!handle.is_finished());
        handle.stop();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(handle.is_finished());
    }

    #[tokio::test]
    async fn a_revoked_key_ends_the_session_with_the_error() {
        let api = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&api)
            .await;
        let client = Arc::new(ApiClient::new(api.uri(), "whk_test".into()).unwrap());
        let (actions, mut receiver) = mpsc::unbounded_channel();
        let _handle =
            RelayHandle::spawn(client, &start(2, "http://localhost:3000".into()), actions);
        let (_, update) = wait_for(&mut receiver, |update| {
            matches!(update, RelayUpdate::Ended(_))
        })
        .await;
        let RelayUpdate::Ended(Some(message)) = update else {
            panic!("expected an error");
        };
        assert!(message.contains("rejected"), "{message}");
    }

    #[tokio::test]
    async fn local_replay_records_the_response_or_the_error() {
        let local = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("x-env", "local"))
            .respond_with(ResponseTemplate::new(202).set_body_string("again"))
            .mount(&local)
            .await;
        let frame = Arc::new(crate::tui::fixtures::relay_frame(
            "evt_one",
            "2026-09-23T12:04:11Z",
        ));
        let replay = LocalReplay {
            session: 1,
            frame: frame.clone(),
            target_url: local.uri(),
            extra_headers: vec![("x-env".into(), "local".into())],
        };
        let record = replay_locally(replay).await;
        assert_eq!(record.target_url, local.uri());
        assert!(matches!(
            record.outcome,
            RelayOutcome::Forwarded(ref response) if response.status == 202
        ));

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed_url = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        let failed = replay_locally(LocalReplay {
            session: 1,
            frame,
            target_url: closed_url,
            extra_headers: vec![],
        })
        .await;
        assert!(matches!(failed.outcome, RelayOutcome::Failed(_)));
    }
}
