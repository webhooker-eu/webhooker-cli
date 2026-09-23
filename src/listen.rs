use anyhow::Result;
use tokio::sync::{mpsc, watch};

use crate::client::ApiClient;
use crate::relay::{
    self, body_bytes, parse_header_flag, ForwardedResponse, RelayEvent, RelayOptions, RelayOutcome,
    RelayRecord, WebhookFrame, FORWARD_QUEUE_CAPACITY,
};
use crate::sse;

fn body_size_label(frame: &WebhookFrame) -> String {
    let length = body_bytes(frame)
        .map(|bytes| bytes.len())
        .unwrap_or(frame.body.len());
    format!("{:.1} KB", length as f64 / 1024.0)
}

fn json_line(frame: &WebhookFrame, outcome: &RelayOutcome) -> String {
    let label = match outcome {
        RelayOutcome::Forwarded(_) => "forwarded",
        RelayOutcome::SkippedUnverified => "skipped",
        RelayOutcome::Failed(_) => "error",
        RelayOutcome::Dropped => "dropped",
    };
    let mut record = serde_json::json!({
        "event": frame.public_id,
        "method": frame.method,
        "outcome": label,
    });
    match outcome {
        RelayOutcome::Forwarded(response) => {
            record["status"] = response.status.into();
            record["latency_ms"] = (response.elapsed.as_millis() as u64).into();
        }
        RelayOutcome::SkippedUnverified => {
            record["reason"] = "signature verification failed".into();
        }
        RelayOutcome::Failed(error) => record["error"] = error.as_str().into(),
        RelayOutcome::Dropped => record["reason"] = "forward queue full".into(),
    }
    record.to_string()
}

fn forwarded_line(frame: &WebhookFrame, response: &ForwardedResponse) -> String {
    format!(
        "  {} {} ← {} ({}, {})\n           → {} in {}ms",
        frame.received_at,
        frame.method,
        frame.public_id,
        frame.content_type.as_deref().unwrap_or("-"),
        body_size_label(frame),
        response.status,
        response.elapsed.as_millis() as u64
    )
}

/// Machine-readable records go to stdout; human progress and failures are
/// diagnostics and go to stderr, so `whk listen --json | jq` stays clean.
/// Drops and malformed frames were always diagnostics, in both modes.
fn report(event: &RelayEvent, json_output: bool) {
    let record = match event {
        RelayEvent::MalformedFrame(error) => {
            eprintln!("  malformed frame skipped: {error}");
            return;
        }
        RelayEvent::Record(record) => record,
    };
    let RelayRecord { frame, outcome, .. } = record;
    if let RelayOutcome::Dropped = outcome {
        eprintln!(
            "  {} dropped ← {} ({FORWARD_QUEUE_CAPACITY} webhooks already queued; the local endpoint is too slow)",
            frame.received_at, frame.public_id
        );
        return;
    }
    if json_output {
        println!("{}", json_line(frame, outcome));
        return;
    }
    match outcome {
        RelayOutcome::Forwarded(response) => println!("{}", forwarded_line(frame, response)),
        RelayOutcome::SkippedUnverified => eprintln!(
            "  {} skipped ← {} (signature invalid; use --skip-verify)",
            frame.received_at, frame.public_id
        ),
        RelayOutcome::Failed(error) => eprintln!(
            "  {} ← {} forward failed: {error}",
            frame.received_at, frame.public_id
        ),
        RelayOutcome::Dropped => {}
    }
}

pub async fn run(
    client: &ApiClient,
    source_selector: &str,
    forward_url: &str,
    skip_verify: bool,
    raw_headers: &[String],
    json_output: bool,
) -> Result<()> {
    let extra_headers: Vec<(String, String)> = raw_headers
        .iter()
        .map(|raw| parse_header_flag(raw))
        .collect::<Result<_>>()?;
    let source = client.resolve_source(source_selector).await?;
    eprintln!(
        "  Forwarding \"{}\" → {forward_url}  (Ctrl-C to stop)",
        source.name
    );

    let (_retarget, target) = watch::channel(forward_url.to_string());
    let (events, mut received) = mpsc::unbounded_channel();
    let printer = tokio::spawn(async move {
        while let Some(event) = received.recv().await {
            report(&event, json_output);
        }
    });
    let options = RelayOptions {
        source_id: source.id,
        skip_verify,
        extra_headers,
        queue_capacity: FORWARD_QUEUE_CAPACITY,
    };
    let result = relay::run(client, options, target, events, sse::print_status).await;
    let _ = printer.await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::tests::frame;
    use base64::Engine as _;
    use std::time::Duration;

    fn forwarded(status: u16, latency_ms: u64) -> RelayOutcome {
        RelayOutcome::Forwarded(ForwardedResponse {
            status,
            headers: vec![],
            body: vec![],
            body_truncated: false,
            elapsed: Duration::from_millis(latency_ms),
        })
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
            forwarded(200, 12),
            RelayOutcome::SkippedUnverified,
            RelayOutcome::Failed("connection refused".to_string()),
        ];
        let labels = ["forwarded", "skipped", "error"];
        for (outcome, label) in outcomes.iter().zip(labels) {
            let record: serde_json::Value = serde_json::from_str(&json_line(&frame, outcome))
                .expect("every --json line must be valid JSON");
            assert_eq!(record["outcome"], label);
            assert_eq!(record["event"], "evt_testtesttest01");
            assert_eq!(record["method"], "POST");
        }
        let forwarded_record: serde_json::Value =
            serde_json::from_str(&json_line(&frame, &outcomes[0])).unwrap();
        assert_eq!(forwarded_record["status"], 200);
        assert_eq!(forwarded_record["latency_ms"], 12);
        let failed: serde_json::Value =
            serde_json::from_str(&json_line(&frame, &outcomes[2])).unwrap();
        assert_eq!(failed["error"], "connection refused");
    }

    #[test]
    fn forwarded_text_line_is_unchanged() {
        let RelayOutcome::Forwarded(response) = forwarded(200, 12) else {
            unreachable!()
        };
        assert_eq!(
            forwarded_line(&frame("{}"), &response),
            "  2026-07-16T12:04:31Z POST ← evt_testtesttest01 (application/json, 0.0 KB)\n           → 200 in 12ms"
        );
    }
}
