use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::args::query_string;
use crate::client::ApiClient;
use crate::output;

pub struct ListFilters<'a> {
    pub source: Option<&'a str>,
    pub verification_status: Option<&'a str>,
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
    pub search: Option<&'a str>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

fn list_path(source_id: Option<&str>, filters: &ListFilters<'_>) -> String {
    format!(
        "/api/v1/events/{}",
        query_string(&[
            ("source_id", source_id.map(str::to_string)),
            (
                "verification_status",
                filters.verification_status.map(str::to_string)
            ),
            ("received_after", filters.since.map(str::to_string)),
            ("received_before", filters.until.map(str::to_string)),
            ("q", filters.search.map(str::to_string)),
            ("page", filters.page.map(|value| value.to_string())),
            ("limit", filters.limit.map(|value| value.to_string())),
        ])
    )
}

/// Deliveries condensed into one column: delivered / failed / pending.
fn delivery_summary(event: &Value) -> String {
    format!(
        "{}/{}/{}",
        output::cell(event, "delivered_count"),
        output::cell(event, "failed_count"),
        output::cell(event, "pending_count")
    )
}

fn event_row(event: &Value) -> Vec<String> {
    vec![
        output::cell(event, "public_id"),
        output::cell(event, "received_at"),
        output::cell(event, "method"),
        output::cell(event, "verification_status"),
        output::cell(event, "body_size"),
        delivery_summary(event),
    ]
}

fn bulk_body(
    connection_id: &str,
    statuses: &[String],
    since: Option<&str>,
    until: Option<&str>,
) -> Value {
    let mut body = json!({"connection_id": connection_id});
    if !statuses.is_empty() {
        body["statuses"] = json!(statuses);
    }
    if let Some(since) = since {
        body["since"] = json!(since);
    }
    if let Some(until) = until {
        body["until"] = json!(until);
    }
    body
}

fn resend_body(connection_ids: &[String]) -> Value {
    if connection_ids.is_empty() {
        json!({})
    } else {
        json!({"connection_ids": connection_ids})
    }
}

/// Accepts either the event's UUID or the short public id that `whk tail` and
/// the dashboard show. A public id is resolved through the search filter, which
/// matches on it.
async fn resolve_event_id(client: &ApiClient, selector: &str) -> Result<String> {
    if uuid_shaped(selector) {
        return Ok(selector.to_string());
    }
    let body = client
        .get_json(&format!(
            "/api/v1/events/{}",
            query_string(&[
                ("q", Some(selector.to_string())),
                ("limit", Some("50".into()))
            ])
        ))
        .await?;
    let matched = body["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|event| event["public_id"].as_str() == Some(selector))
        })
        .with_context(|| format!("no event matches \"{selector}\" (by id or public id)"))?;
    matched["id"]
        .as_str()
        .map(str::to_string)
        .context("event is missing its id")
}

fn uuid_shaped(value: &str) -> bool {
    value.len() == 36
        && value
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
}

pub async fn list(client: &ApiClient, filters: ListFilters<'_>, json_output: bool) -> Result<()> {
    let source_id = match filters.source {
        Some(selector) => Some(client.resolve_source(selector).await?.id),
        None => None,
    };
    let body = client
        .get_json(&list_path(source_id.as_deref(), &filters))
        .await?;
    if json_output {
        output::print_json(&body);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = body["items"]
        .as_array()
        .map(|items| items.iter().map(event_row).collect())
        .unwrap_or_default();
    output::print_table(
        &[
            "PUBLIC ID",
            "RECEIVED AT",
            "METHOD",
            "VERIFY",
            "BYTES",
            "DLV/FAIL/PEND",
        ],
        &rows,
    );
    Ok(())
}

pub async fn get(client: &ApiClient, selector: &str, json_output: bool) -> Result<()> {
    let event_id = resolve_event_id(client, selector).await?;
    let event = client
        .get_json(&format!("/api/v1/events/{event_id}"))
        .await?;
    if json_output {
        output::print_json(&event);
        return Ok(());
    }
    output::print_record(&[
        ("id", output::cell(&event, "id")),
        ("public id", output::cell(&event, "public_id")),
        ("source id", output::cell(&event, "source_id")),
        ("method", output::cell(&event, "method")),
        ("content type", output::cell(&event, "content_type")),
        ("verification", output::cell(&event, "verification_status")),
        ("received at", output::cell(&event, "received_at")),
        ("body size", output::cell(&event, "body_size")),
        ("headers", output::cell(&event, "headers")),
        ("body", output::cell(&event, "body")),
        ("deliveries", output::cell(&event, "deliveries")),
    ]);
    Ok(())
}

pub async fn replay(
    client: &ApiClient,
    selector: &str,
    connection_ids: &[String],
    json_output: bool,
) -> Result<()> {
    let event_id = resolve_event_id(client, selector).await?;
    let result = client
        .post_json(
            &format!("/api/v1/events/{event_id}/resend"),
            resend_body(connection_ids),
        )
        .await?;
    if json_output {
        output::print_json(&result);
    } else {
        println!("Queued {} delivery(ies)", output::cell(&result, "created"));
    }
    Ok(())
}

pub async fn replay_bulk(
    client: &ApiClient,
    connection_id: &str,
    statuses: &[String],
    since: Option<&str>,
    until: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let result = client
        .post_json(
            "/api/v1/deliveries/resend-bulk",
            bulk_body(connection_id, statuses, since, until),
        )
        .await?;
    if json_output {
        output::print_json(&result);
    } else {
        println!("Queued {} delivery(ies)", output::cell(&result, "created"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_filters<'a>() -> ListFilters<'a> {
        ListFilters {
            source: None,
            verification_status: None,
            since: None,
            until: None,
            search: None,
            page: None,
            limit: None,
        }
    }

    #[test]
    fn list_path_has_no_query_when_nothing_is_filtered() {
        assert_eq!(list_path(None, &no_filters()), "/api/v1/events/");
    }

    #[test]
    fn list_path_maps_flags_to_the_api_query_names() {
        let filters = ListFilters {
            since: Some("2026-09-20T10:00:00Z"),
            verification_status: Some("failed"),
            limit: Some(10),
            ..no_filters()
        };
        let path = list_path(Some("src-id"), &filters);
        assert!(path.contains("source_id=src-id"), "{path}");
        assert!(path.contains("verification_status=failed"), "{path}");
        assert!(
            path.contains("received_after=2026-09-20T10%3A00%3A00Z"),
            "{path}"
        );
        assert!(path.contains("limit=10"), "{path}");
    }

    #[test]
    fn resend_body_defaults_to_every_connection() {
        assert_eq!(resend_body(&[]), json!({}));
        assert_eq!(
            resend_body(&["c1".to_string()]),
            json!({"connection_ids": ["c1"]})
        );
    }

    #[test]
    fn bulk_body_carries_only_what_was_given() {
        assert_eq!(
            bulk_body("c1", &[], None, None),
            json!({"connection_id": "c1"})
        );
        assert_eq!(
            bulk_body(
                "c1",
                &["exhausted".to_string()],
                Some("2026-09-01T00:00:00Z"),
                None
            ),
            json!({
                "connection_id": "c1",
                "statuses": ["exhausted"],
                "since": "2026-09-01T00:00:00Z"
            })
        );
    }

    #[test]
    fn uuid_shaped_tells_ids_from_public_ids() {
        assert!(uuid_shaped("0198c9f0-0000-7000-8000-00000000000a"));
        assert!(!uuid_shaped("evt_2f8a"));
    }

    #[test]
    fn delivery_summary_reads_the_three_counters() {
        let event = json!({"delivered_count": 2, "failed_count": 1, "pending_count": 0});
        assert_eq!(delivery_summary(&event), "2/1/0");
    }
}
