use std::time::Duration;

use anyhow::{bail, Result};
use futures::StreamExt;

use crate::client::ApiClient;

#[derive(Debug, PartialEq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// A line terminator at a given offset: CRLF, a lone CR or a lone LF. A CR at
/// the very end of the buffer is undecidable until the next byte arrives.
enum Terminator {
    None,
    Incomplete,
    Length(usize),
}

fn terminator_at(buffer: &[u8], index: usize) -> Terminator {
    match buffer.get(index) {
        None => Terminator::Incomplete,
        Some(b'\n') => Terminator::Length(1),
        Some(b'\r') => match buffer.get(index + 1) {
            None => Terminator::Incomplete,
            Some(b'\n') => Terminator::Length(2),
            Some(_) => Terminator::Length(1),
        },
        Some(_) => Terminator::None,
    }
}

/// Locates the first blank line, returning (block length, separator length).
fn find_event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0;
    while index < buffer.len() {
        match terminator_at(buffer, index) {
            Terminator::Length(first) => match terminator_at(buffer, index + first) {
                Terminator::Length(second) => return Some((index, first + second)),
                Terminator::Incomplete => return None,
                Terminator::None => index += first,
            },
            Terminator::Incomplete => return None,
            Terminator::None => index += 1,
        }
    }
    None
}

/// Incremental SSE parser over raw byte chunks. Events are blocks separated by
/// a blank line; `data:` lines accumulate, `event:` names the type, comments
/// (leading ':') and other fields (id/retry) are ignored.
///
/// Buffering is done on bytes and each block is decoded only once it is
/// complete, so a multi-byte character or a CRLF separator straddling a network
/// chunk boundary survives intact.
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((block_length, separator_length)) = find_event_boundary(&self.buffer) {
            let block: Vec<u8> = self
                .buffer
                .drain(..block_length + separator_length)
                .take(block_length)
                .collect();
            if let Some(event) = parse_block(&String::from_utf8_lossy(&block)) {
                events.push(event);
            }
        }
        events
    }
}

fn parse_block(block: &str) -> Option<SseEvent> {
    let mut event_type = "message".to_string();
    let mut data_lines: Vec<&str> = Vec::new();
    for line in block.split(['\r', '\n']) {
        if let Some(value) = line.strip_prefix("event:") {
            event_type = value.trim_start().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.strip_prefix(' ').unwrap_or(value));
        }
        // Comments (":...") and id/retry fields are intentionally ignored.
    }
    if data_lines.is_empty() {
        return None;
    }
    Some(SseEvent {
        event: event_type,
        data: data_lines.join("\n"),
    })
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// What a response status means for the stream loop.
#[derive(Debug, PartialEq)]
pub enum StreamAction {
    Proceed,
    Fatal(String),
    Retry(String),
}

/// Only a revoked key or a missing source is hopeless; everything else,
/// including 403, can clear on its own.
pub fn action_for_status(status: u16) -> StreamAction {
    match status {
        200 => StreamAction::Proceed,
        401 => StreamAction::Fatal(
            "the API key was rejected (revoked?); run `whk login` again".to_string(),
        ),
        404 => StreamAction::Fatal("source not found on the server".to_string()),
        // The server frees a live stream slot only when it fails to write its
        // keep-alive, so after a network drop the CLI briefly competes with its
        // own stale slot.
        403 => StreamAction::Retry(
            "no live stream slot: a previous connection's slot may still be releasing, \
             or the plan's live stream limit is reached"
                .to_string(),
        ),
        other => StreamAction::Retry(format!("server returned HTTP {other}")),
    }
}

/// Connection state of a stream, reported instead of printed so the TUI can
/// show it in its status line.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamStatus {
    Connected,
    Reconnecting {
        reason: String,
        retry_in: Duration,
        /// The API's own `error.message` when the refusal carried one.
        server_message: Option<String>,
    },
    Fatal(String),
}

/// The CLI's reporting: reconnects go to stderr exactly as before, everything
/// else is already visible through the command's own output or error.
pub fn print_status(status: StreamStatus) {
    if let StreamStatus::Reconnecting {
        reason, retry_in, ..
    } = status
    {
        eprintln!("  reconnecting… ({reason})  retry in {retry_in:?}");
    }
}

async fn envelope_message(response: reqwest::Response) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        error: Detail,
    }
    #[derive(serde::Deserialize)]
    struct Detail {
        message: String,
    }
    let body = response.text().await.ok()?;
    serde_json::from_str::<Envelope>(&body)
        .ok()
        .map(|envelope| envelope.error.message)
}

/// Connects to an SSE endpoint and invokes `on_event` per parsed event,
/// reconnecting with exponential backoff on stream drops. Connection changes
/// go to `on_status`; the CLI passes [`print_status`].
pub async fn run_stream(
    client: &ApiClient,
    path: &str,
    mut on_event: impl FnMut(SseEvent),
    mut on_status: impl FnMut(StreamStatus),
) -> Result<()> {
    let mut backoff = INITIAL_BACKOFF;
    loop {
        let (reason, server_message) = match client.request(path).send().await {
            Err(error) => (format!("connection failed ({error})"), None),
            Ok(response) => match action_for_status(response.status().as_u16()) {
                StreamAction::Fatal(message) => {
                    on_status(StreamStatus::Fatal(message.clone()));
                    bail!("{message}")
                }
                StreamAction::Retry(message) => (message, envelope_message(response).await),
                StreamAction::Proceed => {
                    on_status(StreamStatus::Connected);
                    (
                        consume_stream(response, &mut on_event, &mut backoff).await,
                        None,
                    )
                }
            },
        };
        on_status(StreamStatus::Reconnecting {
            reason,
            retry_in: backoff,
            server_message,
        });
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Reads one connection to completion and returns why it ended. The backoff is
/// reset only once an event has actually been delivered: a server that accepts
/// and immediately closes must not be retried at a flat one second forever.
async fn consume_stream(
    response: reqwest::Response,
    on_event: &mut impl FnMut(SseEvent),
    backoff: &mut Duration,
) -> String {
    let mut parser = SseParser::new();
    let mut bytes = response.bytes_stream();
    while let Some(chunk) = bytes.next().await {
        match chunk {
            Ok(chunk) => {
                for event in parser.feed(&chunk) {
                    on_event(event);
                    *backoff = INITIAL_BACKOFF;
                }
            }
            Err(error) => return format!("stream error: {error}"),
        }
    }
    "stream closed".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn mount_stream(server: &MockServer, response: ResponseTemplate) -> ApiClient {
        Mock::given(method("GET"))
            .and(path("/stream"))
            .respond_with(response)
            .mount(server)
            .await;
        ApiClient::new(server.uri(), "whk_testkey".to_string()).unwrap()
    }

    fn event_stream(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body)
    }

    #[tokio::test]
    async fn run_stream_delivers_every_frame_with_intact_utf8() {
        let server = MockServer::start().await;
        let client = mount_stream(
            &server,
            event_stream(
                "event: webhook\ndata: {\"body\":\"héllo\"}\n\nevent: webhook\ndata: {\"body\":\"second\"}\n\n",
            ),
        )
        .await;

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let stream = run_stream(
            &client,
            "/stream",
            move |event| {
                let _ = sender.send(event);
            },
            |_| {},
        );
        tokio::pin!(stream);

        let mut delivered = Vec::new();
        while delivered.len() < 2 {
            tokio::select! {
                result = &mut stream => panic!("stream ended early: {result:?}"),
                Some(event) = receiver.recv() => delivered.push(event),
            }
        }
        assert_eq!(delivered[0].event, "webhook");
        assert_eq!(delivered[0].data, "{\"body\":\"héllo\"}");
        assert_eq!(delivered[1].data, "{\"body\":\"second\"}");
    }

    #[tokio::test]
    async fn run_stream_aborts_on_401_and_404() {
        let unauthorized = MockServer::start().await;
        let client = mount_stream(&unauthorized, ResponseTemplate::new(401)).await;
        let error = run_stream(&client, "/stream", |_| {}, |_| {})
            .await
            .unwrap_err();
        assert!(error.to_string().contains("rejected"), "{error}");

        let missing = MockServer::start().await;
        let client = mount_stream(&missing, ResponseTemplate::new(404)).await;
        let error = run_stream(&client, "/stream", |_| {}, |_| {})
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not found"), "{error}");
    }

    #[tokio::test]
    async fn run_stream_retries_403_instead_of_exiting() {
        // 403 is the plan-limit status, and a slot held by a half-dead previous
        // connection frees itself shortly — exiting would strand the CLI.
        let server = MockServer::start().await;
        let client = mount_stream(&server, ResponseTemplate::new(403)).await;
        let outcome = tokio::time::timeout(
            Duration::from_millis(200),
            run_stream(&client, "/stream", |_| {}, |_| {}),
        )
        .await;
        assert!(outcome.is_err(), "run_stream returned instead of retrying");
    }

    #[test]
    fn status_action_table() {
        assert_eq!(action_for_status(200), StreamAction::Proceed);
        assert!(matches!(action_for_status(401), StreamAction::Fatal(_)));
        assert!(matches!(action_for_status(404), StreamAction::Fatal(_)));
        let StreamAction::Retry(message) = action_for_status(403) else {
            panic!("403 must be retryable: it is the transient plan-limit status");
        };
        assert!(message.contains("releasing"), "{message}");
        assert!(message.contains("limit is reached"), "{message}");
        assert!(matches!(action_for_status(502), StreamAction::Retry(_)));
    }

    #[tokio::test]
    async fn backoff_is_kept_when_a_connection_delivers_nothing() {
        let server = MockServer::start().await;
        let client = mount_stream(&server, event_stream("")).await;
        let response = client.request("/stream").send().await.unwrap();

        let mut backoff = Duration::from_secs(8);
        let reason = consume_stream(response, &mut |_| {}, &mut backoff).await;
        assert_eq!(backoff, Duration::from_secs(8));
        assert_eq!(reason, "stream closed");
    }

    #[tokio::test]
    async fn backoff_resets_once_an_event_is_delivered() {
        let server = MockServer::start().await;
        let client = mount_stream(&server, event_stream("data: x\n\n")).await;
        let response = client.request("/stream").send().await.unwrap();

        let mut delivered = 0;
        let mut backoff = Duration::from_secs(8);
        consume_stream(response, &mut |_| delivered += 1, &mut backoff).await;
        assert_eq!(delivered, 1);
        assert_eq!(backoff, INITIAL_BACKOFF);
    }

    #[test]
    fn parses_a_single_event() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"event: webhook\ndata: {\"a\":1}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "webhook");
        assert_eq!(events[0].data, "{\"a\":1}");
    }

    #[test]
    fn buffers_partial_chunks_across_feeds() {
        let mut parser = SseParser::new();
        assert!(parser.feed(b"event: webhook\nda").is_empty());
        let events = parser.feed(b"ta: {\"a\":1}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"a\":1}");
    }

    #[test]
    fn joins_multi_line_data_with_newlines() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: line1\ndata: line2\n\n");
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn ignores_comments_and_unknown_fields() {
        let mut parser = SseParser::new();
        // Keep-alive comments and id/retry fields must not produce events.
        assert!(parser.feed(b": keep-alive\n\n").is_empty());
        let events = parser.feed(b"id: 7\nretry: 100\ndata: x\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "x");
    }

    #[test]
    fn handles_crlf_line_endings() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"event: webhook\r\ndata: x\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "webhook");
        assert_eq!(events[0].data, "x");
    }

    #[test]
    fn event_defaults_to_message_when_missing() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: x\n\n");
        assert_eq!(events[0].event, "message");
    }

    #[test]
    fn keeps_multibyte_characters_split_across_chunks_intact() {
        let mut parser = SseParser::new();
        let block = "data: héllo\n\n".as_bytes();
        let split = block.iter().position(|byte| *byte == 0xC3).unwrap() + 1;
        assert!(parser.feed(&block[..split]).is_empty());
        let events = parser.feed(&block[split..]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "héllo");
    }

    #[test]
    fn keeps_events_separate_when_crlf_separator_is_split_across_chunks() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: x\r\n\r");
        assert!(events.is_empty());
        let events = parser.feed(b"\ndata: y\r\n\r\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "x");
        assert_eq!(events[1].data, "y");
    }

    #[test]
    fn accepts_lone_carriage_return_separators() {
        let mut parser = SseParser::new();
        let events = parser.feed(b"data: x\r\rdata: y\r\r");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "x");
        // The trailing lone CR may still turn out to be a CRLF, so the second
        // event is only released once the next byte disambiguates it.
        let events = parser.feed(b"data: z\r\r");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "y");
    }

    async fn collect_statuses(client: &ApiClient, window: Duration) -> Vec<StreamStatus> {
        let statuses = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = statuses.clone();
        let _ = tokio::time::timeout(
            window,
            run_stream(
                client,
                "/stream",
                |_| {},
                move |status| recorded.lock().unwrap().push(status),
            ),
        )
        .await;
        let collected = statuses.lock().unwrap().clone();
        collected
    }

    #[tokio::test]
    async fn connected_is_reported_before_the_stream_closes() {
        let server = MockServer::start().await;
        let client = mount_stream(&server, event_stream("data: x\n\n")).await;
        let statuses = collect_statuses(&client, Duration::from_millis(300)).await;
        assert_eq!(statuses[0], StreamStatus::Connected);
        assert!(matches!(
            &statuses[1],
            StreamStatus::Reconnecting { reason, retry_in, .. }
                if reason == "stream closed" && *retry_in == INITIAL_BACKOFF
        ));
    }

    #[tokio::test]
    async fn retryable_statuses_carry_the_server_message() {
        let server = MockServer::start().await;
        let client = mount_stream(
            &server,
            ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "error": {
                    "code": "plan_limit_exceeded",
                    "message": "live stream limit reached for your plan (3)"
                }
            })),
        )
        .await;
        let statuses = collect_statuses(&client, Duration::from_millis(300)).await;
        let StreamStatus::Reconnecting {
            reason,
            server_message,
            ..
        } = &statuses[0]
        else {
            panic!("expected a reconnect, got {statuses:?}");
        };
        assert!(reason.contains("no live stream slot"), "{reason}");
        assert_eq!(
            server_message.as_deref(),
            Some("live stream limit reached for your plan (3)")
        );
    }

    #[tokio::test]
    async fn fatal_statuses_are_reported_before_returning() {
        let server = MockServer::start().await;
        let client = mount_stream(&server, ResponseTemplate::new(401)).await;
        let mut statuses = Vec::new();
        let error = run_stream(&client, "/stream", |_| {}, |status| statuses.push(status))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("rejected"), "{error}");
        assert!(
            matches!(&statuses[..], [StreamStatus::Fatal(message)] if message.contains("rejected"))
        );
    }
}
