//! The relay core shared by `whk listen` and the TUI: parse full webhooks off
//! a source's live stream, queue them, and forward each to a local URL.

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde::Deserialize;

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
}
