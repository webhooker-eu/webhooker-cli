use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde::Deserialize;

use crate::client::ApiClient;
use crate::sse;

/// A frame of the server's live event stream.
#[derive(Debug, Deserialize)]
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

pub struct ForwardOutcome {
    pub status: u16,
    pub elapsed: Duration,
}

pub async fn forward(
    http: &reqwest::Client,
    url: &str,
    frame: &WebhookFrame,
    extra_headers: &[(String, String)],
) -> Result<ForwardOutcome> {
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
    let response = request.send().await?;
    Ok(ForwardOutcome {
        status: response.status().as_u16(),
        elapsed: started.elapsed(),
    })
}

fn body_size_label(frame: &WebhookFrame) -> String {
    let length = body_bytes(frame)
        .map(|bytes| bytes.len())
        .unwrap_or(frame.body.len());
    format!("{:.1} KB", length as f64 / 1024.0)
}

/// What happened to one frame, for reporting.
enum FrameOutcome {
    Forwarded { status: u16, latency_ms: u64 },
    SkippedUnverified,
    Failed(String),
}

fn json_line(frame: &WebhookFrame, outcome: &FrameOutcome) -> String {
    let label = match outcome {
        FrameOutcome::Forwarded { .. } => "forwarded",
        FrameOutcome::SkippedUnverified => "skipped",
        FrameOutcome::Failed(_) => "error",
    };
    let mut record = serde_json::json!({
        "event": frame.public_id,
        "method": frame.method,
        "outcome": label,
    });
    match outcome {
        FrameOutcome::Forwarded { status, latency_ms } => {
            record["status"] = (*status).into();
            record["latency_ms"] = (*latency_ms).into();
        }
        FrameOutcome::SkippedUnverified => {
            record["reason"] = "signature verification failed".into();
        }
        FrameOutcome::Failed(error) => record["error"] = error.as_str().into(),
    }
    record.to_string()
}

/// Machine-readable records go to stdout; human progress and failures are
/// diagnostics and go to stderr, so `whk listen --json | jq` stays clean.
fn report(frame: &WebhookFrame, outcome: &FrameOutcome, json_output: bool) {
    if json_output {
        println!("{}", json_line(frame, outcome));
        return;
    }
    match outcome {
        FrameOutcome::Forwarded { status, latency_ms } => println!(
            "  {} {} ← {} ({}, {})\n           → {} in {}ms",
            frame.received_at,
            frame.method,
            frame.public_id,
            frame.content_type.as_deref().unwrap_or("-"),
            body_size_label(frame),
            status,
            latency_ms
        ),
        FrameOutcome::SkippedUnverified => eprintln!(
            "  {} skipped ← {} (signature invalid; use --skip-verify)",
            frame.received_at, frame.public_id
        ),
        FrameOutcome::Failed(error) => eprintln!(
            "  {} ← {} forward failed: {error}",
            frame.received_at, frame.public_id
        ),
    }
}

/// Bounds how far the forwarder may fall behind: each request can block for up
/// to 30s, so an unbounded queue would grow without limit during a burst.
const FORWARD_QUEUE_CAPACITY: usize = 256;

pub async fn run(
    client: &ApiClient,
    source_selector: &str,
    forward_url: &str,
    skip_verify: bool,
    raw_headers: &[String],
    json_output: bool,
) -> Result<()> {
    let extra: Vec<(String, String)> = raw_headers
        .iter()
        .map(|raw| parse_header_flag(raw))
        .collect::<Result<_>>()?;
    let source = client.resolve_source(source_selector).await?;
    eprintln!(
        "  Forwarding \"{}\" → {forward_url}  (Ctrl-C to stop)",
        source.name
    );

    let http = reqwest::Client::new();
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<WebhookFrame>(FORWARD_QUEUE_CAPACITY);

    let path = format!("/api/v1/sources/{}/stream", source.id);
    let forward_task = tokio::spawn({
        let http = http.clone();
        let forward_url = forward_url.to_string();
        async move {
            while let Some(frame) = receiver.recv().await {
                if frame.verification_status == "failed" && !skip_verify {
                    report(&frame, &FrameOutcome::SkippedUnverified, json_output);
                    continue;
                }
                let outcome = match forward(&http, &forward_url, &frame, &extra).await {
                    Ok(forwarded) => FrameOutcome::Forwarded {
                        status: forwarded.status,
                        latency_ms: forwarded.elapsed.as_millis() as u64,
                    },
                    Err(error) => FrameOutcome::Failed(error.to_string()),
                };
                report(&frame, &outcome, json_output);
            }
        }
    });

    let result = sse::run_stream(client, &path, |event| {
        if event.event != "webhook" {
            return;
        }
        match serde_json::from_str::<WebhookFrame>(&event.data) {
            Ok(frame) => {
                if let Err(tokio::sync::mpsc::error::TrySendError::Full(frame)) =
                    sender.try_send(frame)
                {
                    eprintln!(
                        "  {} dropped ← {} ({FORWARD_QUEUE_CAPACITY} webhooks already queued; the local endpoint is too slow)",
                        frame.received_at, frame.public_id
                    );
                }
            }
            Err(error) => eprintln!("  malformed frame skipped: {error}"),
        }
    }, sse::print_status)
    .await;
    drop(sender);
    let _ = forward_task.await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn frame(body: &str) -> WebhookFrame {
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
    fn reported_size_uses_the_decoded_body_not_the_lossy_string() {
        let mut binary = frame("lossy");
        binary.body_base64 = Some(base64::engine::general_purpose::STANDARD.encode([0xFFu8; 2048]));
        assert_eq!(body_size_label(&binary), "2.0 KB");
        assert_eq!(body_size_label(&frame("plain")), "0.0 KB");
    }

    #[test]
    fn json_records_stay_parsable_on_every_outcome() {
        let frame = frame("{}");
        let outcomes = [
            FrameOutcome::Forwarded {
                status: 200,
                latency_ms: 12,
            },
            FrameOutcome::SkippedUnverified,
            FrameOutcome::Failed("connection refused".to_string()),
        ];
        let labels = ["forwarded", "skipped", "error"];
        for (outcome, label) in outcomes.iter().zip(labels) {
            let record: serde_json::Value = serde_json::from_str(&json_line(&frame, outcome))
                .expect("every --json line must be valid JSON");
            assert_eq!(record["outcome"], label);
            assert_eq!(record["event"], "evt_testtesttest01");
            assert_eq!(record["method"], "POST");
        }
        let forwarded: serde_json::Value =
            serde_json::from_str(&json_line(&frame, &outcomes[0])).unwrap();
        assert_eq!(forwarded["status"], 200);
        assert_eq!(forwarded["latency_ms"], 12);
        let failed: serde_json::Value =
            serde_json::from_str(&json_line(&frame, &outcomes[2])).unwrap();
        assert_eq!(failed["error"], "connection refused");
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
        let outcome = forward(&http, &url, &frame(r#"{"hello":"listen"}"#), &[])
            .await
            .unwrap();
        assert_eq!(outcome.status, 200);
    }
}
