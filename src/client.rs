use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct MeResponse {
    pub auth: String,
    pub workspace: WorkspaceInfo,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    pub plan: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub token: String,
}

#[derive(Deserialize)]
struct SourceListResponse {
    items: Vec<SourceInfo>,
}

/// A destination or connection reduced to what a selector needs to match on.
#[derive(Debug, Clone, Deserialize)]
pub struct DestinationInfo {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
struct DestinationListResponse {
    items: Vec<DestinationInfo>,
}

pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>, api_key: String) -> Result<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent(concat!("whk/", env!("CARGO_PKG_VERSION")))
                .build()?,
            base_url,
            api_key,
        })
    }

    pub fn api_key_prefix(&self) -> String {
        // Mirrors the server's stored prefix (KEY_PREFIX_LEN = 12).
        self.api_key.chars().take(12).collect()
    }

    pub fn request(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(&self.api_key)
    }

    pub async fn me(&self) -> Result<MeResponse> {
        let response = self.request("/api/v1/me").send().await?;
        Ok(check_status(response)?.json().await?)
    }

    /// GET returning the parsed JSON body.
    pub async fn get_json(&self, path: &str) -> Result<serde_json::Value> {
        let response = self.request(path).send().await?;
        Ok(ensure_ok(response).await?.json().await?)
    }

    pub async fn post_json(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.send_json(reqwest::Method::POST, path, body).await
    }

    pub async fn patch_json(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.send_json(reqwest::Method::PATCH, path, body).await
    }

    /// DELETE endpoints answer 204 with no body, so nothing is parsed.
    pub async fn delete(&self, path: &str) -> Result<()> {
        let response = self
            .http
            .delete(format!("{}{}", self.base_url, path))
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        ensure_ok(response).await?;
        Ok(())
    }

    async fn send_json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let response = self
            .http
            .request(method, format!("{}{}", self.base_url, path))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        Ok(ensure_ok(response).await?.json().await?)
    }

    pub async fn list_sources(&self) -> Result<Vec<SourceInfo>> {
        let response = self.request("/api/v1/sources/?limit=200").send().await?;
        let list: SourceListResponse = check_status(response)?.json().await?;
        Ok(list.items)
    }

    /// Resolves a user-supplied selector against the workspace's sources by
    /// id, ingest token, or name (first match wins, in that order).
    pub async fn resolve_source(&self, selector: &str) -> Result<SourceInfo> {
        let sources = self.list_sources().await?;
        sources
            .iter()
            .find(|source| source.id == selector)
            .or_else(|| sources.iter().find(|source| source.token == selector))
            .or_else(|| sources.iter().find(|source| source.name == selector))
            .cloned()
            .with_context(|| format!("no source matches \"{selector}\" (by id, token or name)"))
    }

    pub async fn list_destinations(&self) -> Result<Vec<DestinationInfo>> {
        let response = self.request("/api/v1/destinations/").send().await?;
        let list: DestinationListResponse = ensure_ok(response).await?.json().await?;
        Ok(list.items)
    }

    /// Resolves a user-supplied selector against the workspace's destinations by
    /// id or name (first match wins, in that order).
    pub async fn resolve_destination(&self, selector: &str) -> Result<DestinationInfo> {
        let destinations = self.list_destinations().await?;
        destinations
            .iter()
            .find(|destination| destination.id == selector)
            .or_else(|| {
                destinations
                    .iter()
                    .find(|destination| destination.name == selector)
            })
            .cloned()
            .with_context(|| format!("no destination matches \"{selector}\" (by id or name)"))
    }
}

/// A non-2xx answer from the API. `Display` keeps the CLI's established
/// wording; the TUI reads the fields to decide what to do.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: u16,
    /// The envelope's `error.code`; `None` for plain-text or empty bodies.
    pub code: Option<String>,
    /// The envelope's `error.message`, or the raw body text when it is not JSON.
    pub message: String,
    /// Parsed from the `Retry-After` header (seconds), sent with 429.
    pub retry_after: Option<Duration>,
    rendered: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.rendered)
    }
}

impl std::error::Error for ApiError {}

/// Error body the API returns for every non-2xx response.
#[derive(Deserialize)]
struct ApiErrorBody {
    error: ApiErrorDetail,
}

#[derive(Deserialize)]
struct ApiErrorDetail {
    #[serde(default)]
    code: Option<String>,
    message: String,
}

/// Like [`check_status`], but reads the response body so the server's own
/// message ("name must not be empty") reaches the user instead of a bare status.
/// The statuses whose cause the body cannot explain — a rejected key, a spent
/// rate budget — keep the actionable wording of [`status_message`].
pub async fn ensure_ok(response: reqwest::Response) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    let server = server_origin(&response);
    let retry_after = retry_after_of(&response);
    let body = response.text().await.unwrap_or_default();
    Err(error_from_body(status, &server, retry_after, &body).into())
}

fn error_from_body(
    status: u16,
    server: &str,
    retry_after: Option<Duration>,
    body: &str,
) -> ApiError {
    let parsed = serde_json::from_str::<ApiErrorBody>(body).ok();
    let rendered = match &parsed {
        Some(parsed) if !matches!(status, 401 | 429) => {
            format!("{} (HTTP {status})", parsed.error.message)
        }
        // No parsable body, or a status whose body cannot explain the cause.
        _ => status_message(status, server),
    };
    let (code, message) = match parsed {
        Some(parsed) => (parsed.error.code, parsed.error.message),
        None => (None, body.trim().to_string()),
    };
    ApiError {
        status,
        code,
        message,
        retry_after,
        rendered,
    }
}

/// Translates the API's error contract into actionable CLI messages.
pub fn check_status(response: reqwest::Response) -> Result<reqwest::Response> {
    match response.status().as_u16() {
        200..=299 => Ok(response),
        status => {
            let rendered = status_message(status, &server_origin(&response));
            Err(ApiError {
                status,
                code: None,
                message: rendered.clone(),
                retry_after: retry_after_of(&response),
                rendered,
            }
            .into())
        }
    }
}

fn server_origin(response: &reqwest::Response) -> String {
    response.url().origin().ascii_serialization()
}

fn retry_after_of(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

fn status_message(status: u16, server: &str) -> String {
    match status {
        401 => "the API key was rejected (revoked, or wrong --server?)".to_string(),
        403 => "forbidden: the plan limit was reached or access is denied".to_string(),
        // The API answers its own 404s with a JSON body, so a bare one means
        // the request never reached it.
        404 => format!(
            "{server} returned HTTP 404: it does not look like the Webhooker API; \
             fix it with --server, WEBHOOKER_SERVER or `whk login --server <url>`"
        ),
        429 => "rate limited: the workspace hit its plan's API budget; retry later".to_string(),
        other => format!("server returned HTTP {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn client_for(server: &MockServer) -> ApiClient {
        ApiClient::new(server.uri(), "whk_testkey".to_string()).unwrap()
    }

    #[test]
    fn api_key_prefix_never_splits_a_character() {
        let ascii =
            ApiClient::new("https://webhooker.eu", "whk_0123456789abcdef".to_string()).unwrap();
        assert_eq!(ascii.api_key_prefix(), "whk_01234567");
        // Byte 12 falls inside the last character of this key.
        let accented = ApiClient::new("https://webhooker.eu", "whk_aééééé".to_string()).unwrap();
        assert_eq!(accented.api_key_prefix(), "whk_aééééé");
    }

    #[tokio::test]
    async fn me_sends_bearer_and_parses_workspace() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .and(header("authorization", "Bearer whk_testkey"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "auth": "api_key",
                "workspace": {"id": "0198c9f0-0000-7000-8000-000000000001", "name": "Personal", "plan": "free"}
            })))
            .mount(&server)
            .await;

        let me = client_for(&server).await.me().await.unwrap();
        assert_eq!(me.workspace.name, "Personal");
        assert_eq!(me.workspace.plan, "free");
    }

    #[tokio::test]
    async fn me_maps_401_to_a_clear_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let error = client_for(&server).await.me().await.unwrap_err();
        assert!(error.to_string().contains("rejected"), "{error}");
    }

    #[tokio::test]
    async fn me_maps_bare_404_to_a_wrong_server_hint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(ResponseTemplate::new(404).set_body_string("<html>landing</html>"))
            .mount(&server)
            .await;

        let error = client_for(&server).await.me().await.unwrap_err();
        assert!(error.to_string().contains(&server.uri()), "{error}");
        assert!(error.to_string().contains("--server"), "{error}");
    }

    #[tokio::test]
    async fn get_json_maps_bare_404_to_a_wrong_server_hint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/events/"))
            .respond_with(ResponseTemplate::new(404).set_body_string("<html>landing</html>"))
            .mount(&server)
            .await;

        let error = client_for(&server)
            .await
            .get_json("/api/v1/events/")
            .await
            .unwrap_err();
        assert!(error.to_string().contains(&server.uri()), "{error}");
    }

    #[tokio::test]
    async fn get_json_keeps_the_api_message_for_a_json_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/events/evt_missing"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "error": {"code": "not_found", "message": "resource not found"}
            })))
            .mount(&server)
            .await;

        let error = client_for(&server)
            .await
            .get_json("/api/v1/events/evt_missing")
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "resource not found (HTTP 404)");
    }

    #[tokio::test]
    async fn resolve_source_matches_by_name_token_or_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sources/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "0198c9f0-0000-7000-8000-00000000000a", "name": "github-prod", "token": "6b225n04u5kmyg"},
                    {"id": "0198c9f0-0000-7000-8000-00000000000b", "name": "stripe", "token": "zzz25n04u5kmyg"}
                ],
                "total": 2
            })))
            .mount(&server)
            .await;
        let client = client_for(&server).await;

        assert_eq!(
            client.resolve_source("github-prod").await.unwrap().token,
            "6b225n04u5kmyg"
        );
        assert_eq!(
            client.resolve_source("zzz25n04u5kmyg").await.unwrap().name,
            "stripe"
        );
        assert_eq!(
            client
                .resolve_source("0198c9f0-0000-7000-8000-00000000000a")
                .await
                .unwrap()
                .name,
            "github-prod"
        );
        assert!(client.resolve_source("nope").await.is_err());
    }

    #[tokio::test]
    async fn post_json_sends_the_body_and_parses_the_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/sources/"))
            .and(header("authorization", "Bearer whk_testkey"))
            .and(body_json(serde_json::json!({"name": "stripe"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "0198c9f0-0000-7000-8000-00000000000a",
                "name": "stripe"
            })))
            .mount(&server)
            .await;

        let created = client_for(&server)
            .await
            .post_json("/api/v1/sources/", serde_json::json!({"name": "stripe"}))
            .await
            .unwrap();
        assert_eq!(created["name"], "stripe");
    }

    #[tokio::test]
    async fn patch_json_sends_only_the_supplied_fields() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/sources/0198c9f0-0000-7000-8000-00000000000a"))
            .and(body_json(serde_json::json!({"name": "renamed"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "0198c9f0-0000-7000-8000-00000000000a",
                "name": "renamed"
            })))
            .mount(&server)
            .await;

        let updated = client_for(&server)
            .await
            .patch_json(
                "/api/v1/sources/0198c9f0-0000-7000-8000-00000000000a",
                serde_json::json!({"name": "renamed"}),
            )
            .await
            .unwrap();
        assert_eq!(updated["name"], "renamed");
    }

    #[tokio::test]
    async fn delete_accepts_an_empty_204_response() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/sources/0198c9f0-0000-7000-8000-00000000000a"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        client_for(&server)
            .await
            .delete("/api/v1/sources/0198c9f0-0000-7000-8000-00000000000a")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn validation_errors_surface_the_server_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/sources/"))
            .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
                "error": {"code": "validation_error", "message": "name must not be empty"}
            })))
            .mount(&server)
            .await;

        let error = client_for(&server)
            .await
            .post_json("/api/v1/sources/", serde_json::json!({"name": " "}))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("name must not be empty"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn auth_and_rate_limit_failures_keep_the_actionable_wording() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sources/"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": {"code": "unauthorized", "message": "unauthorized"}
            })))
            .mount(&server)
            .await;

        let error = client_for(&server)
            .await
            .get_json("/api/v1/sources/")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("revoked"), "{error}");
    }

    #[tokio::test]
    async fn plan_limits_surface_the_server_message_not_the_generic_one() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/sources/"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "error": {
                    "code": "plan_limit_exceeded",
                    "message": "source limit reached for your plan (3)"
                }
            })))
            .mount(&server)
            .await;

        let error = client_for(&server)
            .await
            .post_json("/api/v1/sources/", serde_json::json!({"name": "stripe"}))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("source limit reached"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn resolve_destination_matches_by_id_or_name() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/destinations/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"id": "0198c9f0-0000-7000-8000-00000000000c", "name": "billing-worker"},
                    {"id": "0198c9f0-0000-7000-8000-00000000000d", "name": "audit-log"}
                ]
            })))
            .mount(&server)
            .await;
        let client = client_for(&server).await;

        assert_eq!(
            client.resolve_destination("audit-log").await.unwrap().id,
            "0198c9f0-0000-7000-8000-00000000000d"
        );
        assert_eq!(
            client
                .resolve_destination("0198c9f0-0000-7000-8000-00000000000c")
                .await
                .unwrap()
                .name,
            "billing-worker"
        );
        assert!(client.resolve_destination("nope").await.is_err());
    }

    fn api_error(error: &anyhow::Error) -> &ApiError {
        error
            .downcast_ref::<ApiError>()
            .expect("the error must be a client::ApiError")
    }

    #[tokio::test]
    async fn ensure_ok_parses_the_error_envelope() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/sources/"))
            .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
                "error": {"code": "validation_error", "message": "name must not be empty"}
            })))
            .mount(&server)
            .await;

        let error = client_for(&server)
            .await
            .post_json("/api/v1/sources/", serde_json::json!({"name": " "}))
            .await
            .unwrap_err();
        let parsed = api_error(&error);
        assert_eq!(parsed.status, 422);
        assert_eq!(parsed.code.as_deref(), Some("validation_error"));
        assert_eq!(parsed.message, "name must not be empty");
        assert_eq!(parsed.retry_after, None);
        assert_eq!(error.to_string(), "name must not be empty (HTTP 422)");
    }

    #[tokio::test]
    async fn plain_text_bodies_become_the_message_and_keep_the_old_wording() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/events/"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string("Failed to deserialize query string"),
            )
            .mount(&server)
            .await;

        let error = client_for(&server)
            .await
            .get_json("/api/v1/events/")
            .await
            .unwrap_err();
        let parsed = api_error(&error);
        assert_eq!(parsed.status, 400);
        assert_eq!(parsed.code, None);
        assert_eq!(parsed.message, "Failed to deserialize query string");
        assert_eq!(error.to_string(), "server returned HTTP 400");
    }

    #[tokio::test]
    async fn rate_limit_carries_retry_after_and_code() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sources/"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "23")
                    .set_body_json(serde_json::json!({
                        "error": {"code": "rate_limited", "message": "rate limit exceeded"}
                    })),
            )
            .mount(&server)
            .await;

        let error = client_for(&server)
            .await
            .get_json("/api/v1/sources/")
            .await
            .unwrap_err();
        let parsed = api_error(&error);
        assert_eq!(parsed.status, 429);
        assert_eq!(parsed.code.as_deref(), Some("rate_limited"));
        assert_eq!(parsed.retry_after, Some(std::time::Duration::from_secs(23)));
        assert!(error.to_string().contains("rate limited"), "{error}");
    }

    #[tokio::test]
    async fn check_status_errors_carry_the_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let error = client_for(&server).await.me().await.unwrap_err();
        assert_eq!(api_error(&error).status, 401);
        assert!(error.to_string().contains("rejected"), "{error}");
    }
}
