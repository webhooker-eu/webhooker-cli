//! Typed views of the API responses the TUI renders. Every field has a
//! default, so a missing or newly added field never fails a whole screen.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use base64::Engine as _;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub plan: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Me {
    pub workspace: Workspace,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PlanList {
    pub items: Vec<Plan>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub id: String,
    pub limits: PlanLimits,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PlanLimits {
    pub api_per_minute: Option<u32>,
    pub max_live_streams: Option<u32>,
}

impl PlanList {
    pub fn limits_for(&self, plan_id: &str) -> PlanLimits {
        self.items
            .iter()
            .find(|plan| plan.id == plan_id)
            .map(|plan| plan.limits.clone())
            .unwrap_or_default()
    }
}

/// `{items, total}`; `total` is absent on the unpaged listings.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Page<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<T>,
    #[serde(default)]
    pub total: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub token: String,
    pub ingest_url: String,
    pub verification_config: Value,
    pub response_config: Value,
    pub status: String,
    pub created_at: String,
    /// Only the list endpoint returns it.
    pub event_count: Option<i64>,
}

impl Source {
    pub fn verification_provider(&self) -> &str {
        self.verification_config
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("none")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Destination {
    pub id: String,
    pub name: String,
    pub url: String,
    pub custom_headers: BTreeMap<String, String>,
    pub auth_config: Value,
    pub timeout_ms: Option<i64>,
    pub retry_policy: Value,
    pub status: String,
    pub circuit_state: String,
    pub circuit_reopen_at: Option<String>,
    pub created_at: String,
}

impl Destination {
    pub fn auth_type(&self) -> &str {
        self.auth_config
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("none")
    }

    /// `None` means the server's default policy.
    pub fn retry_intervals(&self) -> Option<Vec<u64>> {
        self.retry_policy
            .get("intervals_seconds")?
            .as_array()?
            .iter()
            .map(Value::as_u64)
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Connection {
    pub id: String,
    pub source_id: String,
    pub destination_id: String,
    pub filter_rules: Value,
    pub transformation: Value,
    pub enabled: bool,
    pub created_at: String,
}

impl Connection {
    pub fn has_filter(&self) -> bool {
        !self.filter_rules.is_null()
    }

    pub fn has_transformation(&self) -> bool {
        !self.transformation.is_null()
    }
}

/// A row of `GET /sources/{id}/connections`, which embeds the destination.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SourceConnection {
    pub id: String,
    pub source_id: String,
    pub destination_id: String,
    pub enabled: bool,
    pub destination: DestinationSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DestinationSummary {
    pub id: String,
    pub name: String,
    pub url: String,
    pub circuit_state: String,
}

/// `host[:port]` of a URL, or the input unchanged when it does not parse.
pub fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed.host_str().map(|host| match parsed.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            })
        })
        .unwrap_or_else(|| url.to_string())
}

/// `30s · 2m · 10m · 1h · 4h`
pub fn format_intervals(intervals: &[u64], separator: &str) -> String {
    intervals
        .iter()
        .map(|seconds| human_seconds(*seconds))
        .collect::<Vec<_>>()
        .join(&format!(" {separator} "))
}

fn human_seconds(seconds: u64) -> String {
    match seconds {
        0 => "0s".to_string(),
        whole_days if whole_days % 86_400 == 0 => format!("{}d", whole_days / 86_400),
        whole_hours if whole_hours % 3_600 == 0 => format!("{}h", whole_hours / 3_600),
        whole_minutes if whole_minutes % 60 == 0 => format!("{}m", whole_minutes / 60),
        other => format!("{other}s"),
    }
}

/// A row of `GET /events/`. Rows that arrive over the live tail have no
/// delivery counters yet (`None`, drawn as `…`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct EventSummary {
    pub id: String,
    pub public_id: String,
    pub source_id: String,
    pub method: String,
    pub content_type: Option<String>,
    pub verification_status: String,
    pub body_size: i64,
    pub received_at: String,
    pub delivery_count: Option<i64>,
    pub delivered_count: Option<i64>,
    pub failed_count: Option<i64>,
    pub pending_count: Option<i64>,
}

impl EventSummary {
    pub fn from_notice(notice: &TailNotice) -> Self {
        Self {
            id: notice.event_id.clone(),
            public_id: notice.public_id.clone(),
            source_id: notice.source_id.clone(),
            method: notice.method.clone(),
            content_type: notice.content_type.clone(),
            verification_status: notice.verification_status.clone(),
            body_size: notice.body_size,
            received_at: notice.received_at.clone(),
            ..Self::default()
        }
    }
}

/// `event: event` frames of `GET /sources/{id}/tail`: metadata only.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TailNotice {
    pub event_id: String,
    pub public_id: String,
    pub source_id: String,
    pub method: String,
    pub received_at: String,
    pub content_type: Option<String>,
    pub body_size: i64,
    pub verification_status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct EventDetail {
    pub id: String,
    pub public_id: String,
    pub source_id: String,
    pub method: String,
    pub headers: Value,
    pub body: String,
    pub body_base64: Option<String>,
    pub body_size: i64,
    pub content_type: Option<String>,
    pub verification_status: String,
    pub received_at: String,
    pub expires_at: String,
    pub deliveries: Vec<Delivery>,
}

impl EventDetail {
    pub fn has_transitional_deliveries(&self) -> bool {
        self.deliveries
            .iter()
            .any(|delivery| crate::tui::status::is_transitional(&delivery.status))
    }

    /// Header names and values sorted by name; non-string values as JSON.
    pub fn sorted_headers(&self) -> Vec<(String, String)> {
        let mut headers: Vec<(String, String)> = self
            .headers
            .as_object()
            .map(|map| {
                map.iter()
                    .map(|(name, value)| {
                        let text = value
                            .as_str()
                            .map_or_else(|| value.to_string(), str::to_string);
                        (name.clone(), text)
                    })
                    .collect()
            })
            .unwrap_or_default();
        headers.sort();
        headers
    }

    /// The raw bytes of a body that is not valid UTF-8.
    pub fn binary_body(&self) -> Option<Vec<u8>> {
        let encoded = self.body_base64.as_deref()?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Delivery {
    pub id: String,
    pub connection_id: String,
    pub destination_name: String,
    pub status: String,
    pub attempt_count: i64,
    pub next_attempt_at: String,
    pub created_at: String,
    pub attempts: Vec<Attempt>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Attempt {
    pub attempt_number: i64,
    pub request_url: String,
    pub response_status: Option<i64>,
    pub response_body: Option<String>,
    pub error_message: Option<String>,
    pub latency_ms: i64,
    pub attempted_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DlqSummary {
    pub connection_id: String,
    pub destination_name: String,
    pub exhausted_count: i64,
    pub failed_count: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DlqEntry {
    pub id: String,
    pub event_id: String,
    pub connection_id: String,
    pub destination_name: String,
    pub status: String,
    pub attempt_count: i64,
    pub last_error: Option<String>,
    pub last_response_status: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct StatsOverview {
    pub total_events: i64,
    pub events_per_bucket: Vec<Bucket>,
    pub bucket_unit: String,
    pub range_start: String,
    pub range_end: String,
    pub deliveries_by_status: Vec<StatusCount>,
    pub failed_attempts: i64,
    pub e2e_latency_ms: Latency,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Bucket {
    pub bucket: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct StatusCount {
    pub status: String,
    pub count: i64,
}

/// End-to-end latency percentiles; `None` without successful deliveries.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Latency {
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SourceVolume {
    pub source_id: String,
    pub name: String,
    pub color: Option<String>,
    pub count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_source_list_item_from_the_api_decodes() {
        let page: Page<Source> = serde_json::from_value(json!({
            "items": [{
                "id": "0198c9f0-0000-7000-8000-00000000000a",
                "name": "stripe-prod",
                "description": null,
                "color": "#3b82f6",
                "token": "6b225n04u5kmyg",
                "ingest_path": "/in/6b225n04u5kmyg",
                "ingest_url": "https://app.webhooker.eu/in/6b225n04u5kmyg",
                "verification_config": {"provider": "stripe", "secret": "***"},
                "response_config": null,
                "status": "active",
                "created_at": "2026-09-01T10:00:00Z",
                "event_count": 1204,
                "field_added_later": true
            }],
            "total": 1
        }))
        .unwrap();
        let source = &page.items[0];
        assert_eq!(source.name, "stripe-prod");
        assert_eq!(source.event_count, Some(1204));
        assert_eq!(source.verification_provider(), "stripe");
        assert_eq!(page.total, Some(1));
    }

    #[test]
    fn missing_fields_default_instead_of_failing() {
        let source: Source = serde_json::from_value(json!({"id": "x"})).unwrap();
        assert_eq!(source.name, "");
        assert_eq!(source.verification_provider(), "none");
        let destinations: Page<Destination> =
            serde_json::from_value(json!({"items": [{"id": "d"}]})).unwrap();
        assert_eq!(destinations.total, None);
        assert_eq!(destinations.items[0].auth_type(), "none");
        assert_eq!(destinations.items[0].retry_intervals(), None);
    }

    #[test]
    fn retry_intervals_render_compactly() {
        let destination: Destination = serde_json::from_value(json!({
            "retry_policy": {"intervals_seconds": [30, 120, 600, 3600, 14400, 90, 86400]}
        }))
        .unwrap();
        let intervals = destination.retry_intervals().unwrap();
        assert_eq!(
            format_intervals(&intervals, "·"),
            "30s · 2m · 10m · 1h · 4h · 90s · 1d"
        );
    }

    #[test]
    fn host_of_keeps_the_port_and_survives_garbage() {
        assert_eq!(
            host_of("https://hooks.example.com:8443/in"),
            "hooks.example.com:8443"
        );
        assert_eq!(host_of("https://app.webhooker.eu"), "app.webhooker.eu");
        assert_eq!(host_of("not a url"), "not a url");
    }

    #[test]
    fn plan_limits_fall_back_to_empty_for_unknown_plans() {
        let plans: PlanList = serde_json::from_value(json!({
            "items": [{"id": "pro", "limits": {"api_per_minute": 600, "max_live_streams": 10}}]
        }))
        .unwrap();
        assert_eq!(plans.limits_for("pro").api_per_minute, Some(600));
        assert_eq!(plans.limits_for("team"), PlanLimits::default());
    }

    #[test]
    fn connections_report_filters_and_transformations() {
        let connection: Connection = serde_json::from_value(json!({
            "id": "c", "source_id": "s", "destination_id": "d",
            "filter_rules": {"all": []}, "transformation": null, "enabled": true
        }))
        .unwrap();
        assert!(connection.has_filter());
        assert!(!connection.has_transformation());
    }

    #[test]
    fn an_event_list_item_decodes_with_its_counters() {
        let page: Page<EventSummary> = serde_json::from_value(json!({
            "items": [{
                "id": "0198c9f0-0000-7000-8000-0000000000e1",
                "public_id": "evt_8f2a1b",
                "source_id": "0198c9f0-0000-7000-8000-00000000000a",
                "method": "POST",
                "content_type": "application/json",
                "verification_status": "verified",
                "body_size": 1234,
                "received_at": "2026-09-23T12:04:11Z",
                "delivery_count": 3,
                "delivered_count": 2,
                "failed_count": 1,
                "pending_count": 0
            }],
            "total": 120
        }))
        .unwrap();
        let event = &page.items[0];
        assert_eq!(event.public_id, "evt_8f2a1b");
        assert_eq!(event.delivered_count, Some(2));
        assert_eq!(page.total, Some(120));
    }

    #[test]
    fn a_tail_notice_becomes_a_row_without_counters() {
        let notice: TailNotice = serde_json::from_value(json!({
            "event_id": "e1",
            "public_id": "evt_1",
            "source_id": "s1",
            "workspace_id": "w1",
            "method": "POST",
            "received_at": "2026-09-23T12:04:11Z",
            "content_type": null,
            "body_size": 18,
            "verification_status": "verified"
        }))
        .unwrap();
        let row = EventSummary::from_notice(&notice);
        assert_eq!(row.id, "e1");
        assert_eq!(row.body_size, 18);
        assert_eq!(row.delivery_count, None);
        assert_eq!(row.pending_count, None);
    }

    fn detail() -> EventDetail {
        serde_json::from_value(json!({
            "id": "e1",
            "public_id": "evt_1",
            "source_id": "s1",
            "method": "POST",
            "headers": {"user-agent": "Stripe/1.0", "content-type": "application/json", "x-count": 3},
            "body": "{\"type\":\"invoice.paid\"}",
            "body_size": 23,
            "content_type": "application/json",
            "verification_status": "verified",
            "received_at": "2026-09-23T12:04:11Z",
            "expires_at": "2026-10-23T12:04:11Z",
            "deliveries": [{
                "id": "d1",
                "connection_id": "c1",
                "destination_name": "billing",
                "status": "delivering",
                "attempt_count": 1,
                "next_attempt_at": "2026-09-23T12:05:00Z",
                "created_at": "2026-09-23T12:04:11Z",
                "attempts": [{
                    "attempt_number": 1,
                    "request_url": "https://billing.internal/hooks",
                    "response_status": 500,
                    "response_body": "{\"error\":\"db timeout\"}",
                    "error_message": null,
                    "latency_ms": 120,
                    "attempted_at": "2026-09-23T12:04:12Z"
                }]
            }]
        }))
        .unwrap()
    }

    #[test]
    fn an_event_detail_decodes_deliveries_and_attempts() {
        let event = detail();
        assert_eq!(event.deliveries[0].attempts[0].response_status, Some(500));
        assert_eq!(event.deliveries[0].attempts[0].latency_ms, 120);
        assert!(event.has_transitional_deliveries());
        assert_eq!(
            event.sorted_headers(),
            vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("user-agent".to_string(), "Stripe/1.0".to_string()),
                ("x-count".to_string(), "3".to_string()),
            ]
        );
        assert_eq!(event.binary_body(), None);
    }

    #[test]
    fn a_binary_body_is_decoded_from_base64() {
        let mut event = detail();
        event.body_base64 = Some("//4AAQ==".into());
        assert_eq!(event.binary_body(), Some(vec![0xFF, 0xFE, 0x00, 0x01]));
        event.deliveries[0].status = "succeeded".into();
        assert!(!event.has_transitional_deliveries());
    }

    #[test]
    fn dlq_and_stats_responses_decode() {
        let summary: Page<DlqSummary> = serde_json::from_value(json!({"items": [{
            "connection_id": "c1", "destination_name": "billing", "exhausted_count": 4, "failed_count": 1
        }]}))
        .unwrap();
        assert_eq!(summary.items[0].exhausted_count, 4);
        let entries: Page<DlqEntry> = serde_json::from_value(json!({"items": [{
            "id": "d1", "event_id": "e1", "connection_id": "c1", "destination_name": "billing",
            "status": "exhausted", "attempt_count": 8, "last_error": "HTTP 500",
            "last_response_status": 500, "created_at": "2026-09-22T10:00:00Z",
            "updated_at": "2026-09-23T10:00:00Z"
        }], "total": 1}))
        .unwrap();
        assert_eq!(entries.items[0].last_response_status, Some(500));
        let overview: StatsOverview = serde_json::from_value(json!({
            "total_events": 1204,
            "events_per_bucket": [{"bucket": "2026-09-23T10:00:00Z", "count": 700}],
            "bucket_unit": "hour",
            "range_start": "2026-09-22T12:00:00Z",
            "range_end": "2026-09-23T12:00:00Z",
            "deliveries_by_status": [{"status": "succeeded", "count": 1180}],
            "failed_attempts": 41,
            "e2e_latency_ms": {"p50_ms": 120.4, "p95_ms": null, "p99_ms": null}
        }))
        .unwrap();
        assert_eq!(overview.events_per_bucket[0].count, 700);
        assert_eq!(overview.e2e_latency_ms.p50_ms, Some(120.4));
        let volume: Page<SourceVolume> = serde_json::from_value(json!({"items": [
            {"source_id": "s1", "name": "stripe-prod", "color": null, "count": 12}
        ]}))
        .unwrap();
        assert_eq!(volume.items[0].count, 12);
    }
}
