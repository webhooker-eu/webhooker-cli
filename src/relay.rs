//! The relay core shared by `whk listen` and the TUI: parse full webhooks off
//! a source's live stream, queue them, and forward each to a local URL.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde::Deserialize;
use tokio::sync::{mpsc, watch};

use crate::client::ApiClient;
use crate::sse::{self, StreamStatus};

/// A frame of the server's live event stream.
#[derive(Debug, Clone, Deserialize)]
pub struct WebhookFrame {
    pub id: String,
    pub public_id: String,
    pub source_id: String,
    pub method: String,
    pub headers: serde_json::Value,
    pub body: String,
    #[serde(default)]
    pub body_base64: Option<String>,
    pub content_type: Option<String>,
    pub verification_status: String,
    pub received_at: String,
}

/// Hop-by-hop and transport headers that must not be replayed to localhost:
/// reqwest recomputes them, and a stale Host/Content-Length breaks requests.
const SKIPPED_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "keep-alive",
    "upgrade",
    "proxy-authorization",
    "proxy-connection",
];

/// Response bodies are kept for the inspector up to this size.
pub const RESPONSE_BODY_LIMIT: usize = 64 * 1024;

pub fn forward_headers(frame: &WebhookFrame, extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = frame
        .headers
        .as_object()
        .map(|map| {
            map.iter()
                .filter(|(name, _)| !SKIPPED_HEADERS.contains(&name.to_lowercase().as_str()))
                .filter_map(|(name, value)| {
                    value
                        .as_str()
                        .map(|text| (name.to_lowercase(), text.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    headers.push(("x-webhooker-event-id".to_string(), frame.public_id.clone()));
    headers.extend(extra.iter().cloned());
    headers
}

pub fn body_bytes(frame: &WebhookFrame) -> Result<Vec<u8>> {
    match &frame.body_base64 {
        Some(encoded) => Ok(base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .context("invalid body_base64 in stream frame")?),
        None => Ok(frame.body.clone().into_bytes()),
    }
}

pub fn parse_header_flag(raw: &str) -> Result<(String, String)> {
    let Some((name, value)) = raw.split_once(':') else {
        bail!("--header must look like \"Name: Value\", got \"{raw}\"");
    };
    Ok((name.trim().to_string(), value.trim().to_string()))
}

/// What the local server answered.
#[derive(Debug, Clone, PartialEq)]
pub struct ForwardedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// At most [`RESPONSE_BODY_LIMIT`] bytes.
    pub body: Vec<u8>,
    pub body_truncated: bool,
    /// Time until the response headers arrived.
    pub elapsed: Duration,
}

pub async fn forward(
    http: &reqwest::Client,
    url: &str,
    frame: &WebhookFrame,
    extra_headers: &[(String, String)],
) -> Result<ForwardedResponse> {
    let method = reqwest::Method::from_bytes(frame.method.as_bytes())
        .with_context(|| format!("bad method {}", frame.method))?;
    let mut request = http
        .request(method, url)
        .timeout(Duration::from_secs(30))
        .body(body_bytes(frame)?);
    for (name, value) in forward_headers(frame, extra_headers) {
        request = request.header(&name, &value);
    }
    let started = Instant::now();
    let mut response = request.send().await?;
    let elapsed = started.elapsed();
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect();
    let (body, body_truncated) = read_capped(&mut response, RESPONSE_BODY_LIMIT).await;
    Ok(ForwardedResponse {
        status,
        headers,
        body,
        body_truncated,
        elapsed,
    })
}

/// Reads at most `limit` bytes of the body. A local server that fails
/// mid-body still produced a status, so a read error just ends the capture.
async fn read_capped(response: &mut reqwest::Response, limit: usize) -> (Vec<u8>, bool) {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let room = limit - body.len();
        if chunk.len() > room {
            body.extend_from_slice(&chunk[..room]);
            return (body, true);
        }
        body.extend_from_slice(&chunk);
    }
    (body, false)
}

/// Bounds how far the forwarder may fall behind: each request can block for up
/// to 30s, so an unbounded queue would grow without limit during a burst.
pub const FORWARD_QUEUE_CAPACITY: usize = 256;

/// What happened to one frame.
#[derive(Debug, Clone)]
pub enum RelayOutcome {
    Forwarded(ForwardedResponse),
    SkippedUnverified,
    Failed(String),
    /// The queue was full; the frame was never sent.
    Dropped,
}

#[derive(Debug, Clone)]
pub struct RelayRecord {
    pub frame: Arc<WebhookFrame>,
    /// The target the frame was sent to (or would have been, for drops).
    pub target_url: String,
    pub outcome: RelayOutcome,
}

#[derive(Debug, Clone)]
pub enum RelayEvent {
    Record(RelayRecord),
    MalformedFrame(String),
}

pub struct RelayOptions {
    pub source_id: String,
    pub skip_verify: bool,
    pub extra_headers: Vec<(String, String)>,
    pub queue_capacity: usize,
}

/// Streams a source's full webhooks and forwards each to the URL currently in
/// `target`. Returns only on a fatal stream error; dropping the future stops
/// the relay and frees the server's stream slot.
pub async fn run(
    client: &ApiClient,
    options: RelayOptions,
    target: watch::Receiver<String>,
    events: mpsc::UnboundedSender<RelayEvent>,
    on_status: impl FnMut(StreamStatus),
) -> Result<()> {
    let RelayOptions {
        source_id,
        skip_verify,
        extra_headers,
        queue_capacity,
    } = options;
    let (queue, mut queued) = mpsc::channel::<Arc<WebhookFrame>>(queue_capacity);

    let forward_task = tokio::spawn({
        let events = events.clone();
        let target = target.clone();
        async move {
            let http = reqwest::Client::new();
            while let Some(frame) = queued.recv().await {
                let target_url = target.borrow().clone();
                let outcome = if frame.verification_status == "failed" && !skip_verify {
                    RelayOutcome::SkippedUnverified
                } else {
                    match forward(&http, &target_url, &frame, &extra_headers).await {
                        Ok(response) => RelayOutcome::Forwarded(response),
                        Err(error) => RelayOutcome::Failed(error.to_string()),
                    }
                };
                let _ = events.send(RelayEvent::Record(RelayRecord {
                    frame,
                    target_url,
                    outcome,
                }));
            }
        }
    });

    let path = format!("/api/v1/sources/{source_id}/stream");
    let result = sse::run_stream(
        client,
        &path,
        |event| {
            if event.event != "webhook" {
                return;
            }
            match serde_json::from_str::<WebhookFrame>(&event.data) {
                Ok(frame) => {
                    if let Err(mpsc::error::TrySendError::Full(frame)) =
                        queue.try_send(Arc::new(frame))
                    {
                        let _ = events.send(RelayEvent::Record(RelayRecord {
                            frame,
                            target_url: target.borrow().clone(),
                            outcome: RelayOutcome::Dropped,
                        }));
                    }
                }
                Err(error) => {
                    let _ = events.send(RelayEvent::MalformedFrame(error.to_string()));
                }
            }
        },
        on_status,
    )
    .await;
    drop(queue);
    let _ = forward_task.await;
    result
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use wiremock::matchers::{body_bytes as body_bytes_matcher, body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    pub(crate) fn frame(body: &str) -> WebhookFrame {
        WebhookFrame {
            id: "0198c9f0-0000-7000-8000-0000000000ff".into(),
            public_id: "evt_testtesttest01".into(),
            source_id: "0198c9f0-0000-7000-8000-00000000000a".into(),
            method: "POST".into(),
            headers: serde_json::json!({
                "content-type": "application/json",
                "x-github-event": "push",
                "host": "webhooker.eu",
                "content-length": "18",
                "connection": "keep-alive"
            }),
            body: body.to_string(),
            body_base64: None,
            content_type: Some("application/json".into()),
            verification_status: "verified".into(),
            received_at: "2026-07-16T12:04:31Z".into(),
        }
    }

    #[test]
    fn forward_headers_strip_hop_by_hop_and_add_event_id() {
        let headers = forward_headers(&frame("{}"), &[("x-env".into(), "local".into())]);
        let names: Vec<&str> = headers.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"x-github-event"));
        assert!(names.contains(&"content-type"));
        assert!(names.contains(&"x-webhooker-event-id"));
        assert!(names.contains(&"x-env"));
        assert!(!names.contains(&"host"));
        assert!(!names.contains(&"content-length"));
        assert!(!names.contains(&"connection"));
    }

    #[test]
    fn body_bytes_prefers_base64_when_present() {
        let mut binary = frame("lossy");
        binary.body_base64 = Some("//4AAQ==".into());
        assert_eq!(body_bytes(&binary).unwrap(), vec![0xFF, 0xFE, 0x00, 0x01]);
        assert_eq!(body_bytes(&frame("plain")).unwrap(), b"plain".to_vec());
    }

    #[test]
    fn parse_header_flag_splits_on_first_colon() {
        assert_eq!(
            parse_header_flag("X-Env: local").unwrap(),
            ("X-Env".to_string(), "local".to_string())
        );
        assert!(parse_header_flag("no-colon").is_err());
    }

    #[tokio::test]
    async fn forward_replays_method_headers_and_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .and(header("x-github-event", "push"))
            .and(header("x-webhooker-event-id", "evt_testtesttest01"))
            .and(body_string(r#"{"hello":"listen"}"#))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let url = format!("{}/webhook", server.uri());
        let response = forward(&http, &url, &frame(r#"{"hello":"listen"}"#), &[])
            .await
            .unwrap();
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn forward_sends_the_decoded_base64_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .and(body_bytes_matcher(vec![0xFFu8, 0xFE, 0x00, 0x01]))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let mut binary = frame("lossy");
        binary.body_base64 = Some("//4AAQ==".into());
        let url = format!("{}/webhook", server.uri());
        let response = forward(&reqwest::Client::new(), &url, &binary, &[])
            .await
            .unwrap();
        assert_eq!(response.status, 204);
    }

    #[tokio::test]
    async fn forward_records_the_response_status_headers_and_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webhook"))
            .respond_with(
                ResponseTemplate::new(500)
                    .insert_header("x-trace", "abc")
                    .set_body_string(r#"{"error":"db timeout"}"#),
            )
            .mount(&server)
            .await;

        let url = format!("{}/webhook", server.uri());
        let response = forward(&reqwest::Client::new(), &url, &frame("{}"), &[])
            .await
            .unwrap();
        assert_eq!(response.status, 500);
        assert!(response
            .headers
            .contains(&("x-trace".to_string(), "abc".to_string())));
        assert_eq!(response.body, br#"{"error":"db timeout"}"#.to_vec());
        assert!(!response.body_truncated);
    }

    #[tokio::test]
    async fn forward_truncates_large_response_bodies() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 70 * 1024]))
            .mount(&server)
            .await;

        let response = forward(&reqwest::Client::new(), &server.uri(), &frame("{}"), &[])
            .await
            .unwrap();
        assert_eq!(response.body.len(), RESPONSE_BODY_LIMIT);
        assert!(response.body_truncated);
    }

    #[tokio::test]
    async fn forward_reports_connection_errors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed_url = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        let error = forward(&reqwest::Client::new(), &closed_url, &frame("{}"), &[])
            .await
            .unwrap_err();
        assert!(!error.to_string().is_empty());
    }

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
            "received_at": "2026-07-16T12:04:31Z"
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

    fn options(queue_capacity: usize) -> RelayOptions {
        RelayOptions {
            source_id: "src".into(),
            skip_verify: false,
            extra_headers: vec![],
            queue_capacity,
        }
    }

    /// Runs the relay until `count` records arrived or five seconds passed.
    async fn collect_records(
        api: &MockServer,
        relay_options: RelayOptions,
        target: watch::Receiver<String>,
        count: usize,
        mut on_record: impl FnMut(&RelayRecord),
    ) -> Vec<RelayRecord> {
        let client = ApiClient::new(api.uri(), "whk_testkey".to_string()).unwrap();
        let (events, mut received) = mpsc::unbounded_channel();
        let relay = run(&client, relay_options, target, events, |_| {});
        tokio::pin!(relay);
        let mut records = Vec::new();
        let deadline = tokio::time::sleep(Duration::from_secs(5));
        tokio::pin!(deadline);
        while records.len() < count {
            tokio::select! {
                result = &mut relay => panic!("relay ended early: {result:?}"),
                _ = &mut deadline => panic!("timed out with {} records", records.len()),
                Some(event) = received.recv() => {
                    if let RelayEvent::Record(record) = event {
                        on_record(&record);
                        records.push(record);
                    }
                }
            }
        }
        records
    }

    #[tokio::test]
    async fn run_forwards_frames_and_records_the_response() {
        let api = stream_server(&["evt_one"]).await;
        let local = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201).set_body_string("ok"))
            .mount(&local)
            .await;
        let (_retarget, target) = watch::channel(local.uri());

        let records = collect_records(&api, options(8), target, 1, |_| {}).await;
        assert_eq!(records[0].frame.public_id, "evt_one");
        assert_eq!(records[0].target_url, local.uri());
        let RelayOutcome::Forwarded(response) = &records[0].outcome else {
            panic!("expected a forwarded record, got {:?}", records[0].outcome);
        };
        assert_eq!(response.status, 201);
        assert_eq!(response.body, b"ok".to_vec());
    }

    #[tokio::test]
    async fn connection_errors_become_failed_records() {
        let api = stream_server(&["evt_one"]).await;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed_url = format!("http://{}/", listener.local_addr().unwrap());
        drop(listener);
        let (_retarget, target) = watch::channel(closed_url);

        let records = collect_records(&api, options(8), target, 1, |_| {}).await;
        assert!(matches!(records[0].outcome, RelayOutcome::Failed(_)));
    }

    #[tokio::test]
    async fn retargeting_applies_to_the_next_frame_without_reconnecting() {
        let api = stream_server(&["evt_one", "evt_two"]).await;
        let first = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
            .mount(&first)
            .await;
        let second = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&second)
            .await;
        let (retarget, target) = watch::channel(first.uri());

        // Retarget as soon as the first local server has the first request in
        // flight: the second frame is still queued and must go to `second`.
        let retarget_task = tokio::spawn({
            let second_uri = second.uri();
            async move {
                while first
                    .received_requests()
                    .await
                    .unwrap_or_default()
                    .is_empty()
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                retarget.send(second_uri).unwrap();
                first
            }
        });

        let records = collect_records(&api, options(8), target, 2, |_| {}).await;
        let first = retarget_task.await.unwrap();
        assert_eq!(records[0].frame.public_id, "evt_one");
        assert_eq!(records[0].target_url, first.uri());
        assert_eq!(records[1].frame.public_id, "evt_two");
        assert_eq!(records[1].target_url, second.uri());
        let stream_requests = api.received_requests().await.unwrap();
        assert_eq!(stream_requests.len(), 1, "retarget must not reconnect");
    }

    #[tokio::test]
    async fn a_full_queue_produces_dropped_records() {
        let api = stream_server(&["evt_one", "evt_two", "evt_three"]).await;
        let local = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&local)
            .await;
        let (_retarget, target) = watch::channel(local.uri());

        let mut dropped = 0;
        collect_records(&api, options(1), target, 1, |record| {
            if matches!(record.outcome, RelayOutcome::Dropped) {
                dropped += 1;
            }
        })
        .await;
        assert!(dropped >= 1);
    }

    #[tokio::test]
    async fn unverified_frames_are_skipped_unless_asked() {
        let server = MockServer::start().await;
        let unverified = frame_json("evt_bad").replace("\"verified\"", "\"failed\"");
        Mock::given(method("GET"))
            .and(path("/api/v1/sources/src/stream"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!("event: webhook\ndata: {unverified}\n\n")),
            )
            .mount(&server)
            .await;
        let (_retarget, target) = watch::channel("http://127.0.0.1:1/".to_string());

        let records = collect_records(&server, options(8), target, 1, |_| {}).await;
        assert!(matches!(
            records[0].outcome,
            RelayOutcome::SkippedUnverified
        ));
    }
}
