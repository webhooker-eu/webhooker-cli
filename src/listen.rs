use anyhow::Result;

use crate::client::ApiClient;
use crate::relay::{body_bytes, forward, parse_header_flag, WebhookFrame};
use crate::sse;

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
                    Ok(response) => FrameOutcome::Forwarded {
                        status: response.status,
                        latency_ms: response.elapsed.as_millis() as u64,
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
    use crate::relay::tests::frame;
    use base64::Engine as _;

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
}
