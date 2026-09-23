use anyhow::Result;
use serde_json::Value;

use crate::args::query_string;
use crate::client::ApiClient;
use crate::commands::events;
use crate::output;

pub struct ListFilters<'a> {
    pub statuses: &'a [String],
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
    pub search: Option<&'a str>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

fn list_path(source_id: &str, filters: &ListFilters<'_>) -> String {
    let statuses = (!filters.statuses.is_empty()).then(|| filters.statuses.join(","));
    format!(
        "/api/v1/sources/{source_id}/dlq{}",
        query_string(&[
            ("status", statuses),
            ("received_after", filters.since.map(str::to_string)),
            ("received_before", filters.until.map(str::to_string)),
            ("q", filters.search.map(str::to_string)),
            ("page", filters.page.map(|value| value.to_string())),
            ("limit", filters.limit.map(|value| value.to_string())),
        ])
    )
}

fn summary_row(summary: &Value) -> Vec<String> {
    vec![
        output::cell(summary, "connection_id"),
        output::cell(summary, "destination_name"),
        output::cell(summary, "exhausted_count"),
        output::cell(summary, "failed_count"),
    ]
}

fn entry_row(entry: &Value) -> Vec<String> {
    vec![
        output::cell(entry, "event_id"),
        output::cell(entry, "destination_name"),
        output::cell(entry, "status"),
        output::cell(entry, "attempt_count"),
        output::cell(entry, "last_response_status"),
        output::cell(entry, "updated_at"),
        output::cell(entry, "last_error"),
    ]
}

fn rows(body: &Value, row: fn(&Value) -> Vec<String>) -> Vec<Vec<String>> {
    body["items"]
        .as_array()
        .map(|items| items.iter().map(row).collect())
        .unwrap_or_default()
}

pub async fn summary(client: &ApiClient, source_selector: &str, json_output: bool) -> Result<()> {
    let source = client.resolve_source(source_selector).await?;
    let body = client
        .get_json(&format!("/api/v1/sources/{}/dlq/summary", source.id))
        .await?;
    if json_output {
        output::print_json(&body);
        return Ok(());
    }
    output::print_table(
        &["CONNECTION", "DESTINATION", "EXHAUSTED", "FAILED"],
        &rows(&body, summary_row),
    );
    Ok(())
}

pub async fn list(
    client: &ApiClient,
    source_selector: &str,
    filters: ListFilters<'_>,
    json_output: bool,
) -> Result<()> {
    let source = client.resolve_source(source_selector).await?;
    let body = client.get_json(&list_path(&source.id, &filters)).await?;
    if json_output {
        output::print_json(&body);
        return Ok(());
    }
    output::print_table(
        &[
            "EVENT ID",
            "DESTINATION",
            "STATUS",
            "ATTEMPTS",
            "HTTP",
            "UPDATED AT",
            "LAST ERROR",
        ],
        &rows(&body, entry_row),
    );
    Ok(())
}

/// Same endpoint as `events replay-bulk`; `dlq resend` is the name agents look
/// for next to `dlq ls`.
pub async fn resend(
    client: &ApiClient,
    connection_id: &str,
    statuses: &[String],
    since: Option<&str>,
    until: Option<&str>,
    json_output: bool,
) -> Result<()> {
    events::replay_bulk(client, connection_id, statuses, since, until, json_output).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn no_filters<'a>() -> ListFilters<'a> {
        ListFilters {
            statuses: &[],
            since: None,
            until: None,
            search: None,
            page: None,
            limit: None,
        }
    }

    #[test]
    fn list_path_without_filters_has_no_query() {
        assert_eq!(list_path("src", &no_filters()), "/api/v1/sources/src/dlq");
    }

    #[test]
    fn list_path_joins_statuses_and_maps_the_window() {
        let statuses = vec!["exhausted".to_string(), "failed".to_string()];
        let filters = ListFilters {
            statuses: &statuses,
            since: Some("2026-09-20T10:00:00Z"),
            search: Some("evt_1"),
            limit: Some(20),
            ..no_filters()
        };
        assert_eq!(
            list_path("src", &filters),
            "/api/v1/sources/src/dlq?status=exhausted%2Cfailed&received_after=2026-09-20T10%3A00%3A00Z&q=evt_1&limit=20"
        );
    }

    #[test]
    fn summary_rows_read_the_counts() {
        let row = summary_row(&json!({
            "connection_id": "c1",
            "destination_name": "billing",
            "exhausted_count": 4,
            "failed_count": 1
        }));
        assert_eq!(row, vec!["c1", "billing", "4", "1"]);
    }

    #[test]
    fn entry_rows_put_the_error_last() {
        let row = entry_row(&json!({
            "event_id": "e1",
            "destination_name": "billing",
            "status": "exhausted",
            "attempt_count": 8,
            "last_response_status": 500,
            "last_error": "HTTP 500",
            "updated_at": "2026-09-20T10:00:00Z"
        }));
        assert_eq!(
            row,
            vec![
                "e1",
                "billing",
                "exhausted",
                "8",
                "500",
                "2026-09-20T10:00:00Z",
                "HTTP 500"
            ]
        );
    }
}
