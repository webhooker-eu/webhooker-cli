use anyhow::Result;
use serde::Deserialize;

use crate::client::ApiClient;
use crate::sse;

/// Metadata of an event received by the server; the body is not included.
#[derive(Debug, Deserialize)]
struct TailNotice {
    public_id: String,
    method: String,
    received_at: String,
    content_type: Option<String>,
    body_size: i64,
    verification_status: String,
}

pub async fn run(client: &ApiClient, source_selector: &str, json_output: bool) -> Result<()> {
    let source = client.resolve_source(source_selector).await?;
    eprintln!("  Tailing \"{}\"  (Ctrl-C to stop)", source.name);
    let path = format!("/api/v1/sources/{}/tail", source.id);
    sse::run_stream(
        client,
        &path,
        |event| {
            if json_output {
                println!("{}", event.data);
                return;
            }
            match serde_json::from_str::<TailNotice>(&event.data) {
                Ok(notice) => println!(
                    "  {} {} ← {} ({}, {} B, {})",
                    notice.received_at,
                    notice.method,
                    notice.public_id,
                    notice.content_type.as_deref().unwrap_or("-"),
                    notice.body_size,
                    notice.verification_status
                ),
                Err(error) => eprintln!("  malformed notice skipped: {error}"),
            }
        },
        sse::print_status,
    )
    .await
}
