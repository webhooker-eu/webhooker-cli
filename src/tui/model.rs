//! Typed views of the API responses the TUI renders. Every field has a
//! default, so a missing or newly added field never fails a whole screen.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

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
}
