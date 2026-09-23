use anyhow::Result;
use serde_json::Value;

use crate::args::query_string;
use crate::client::ApiClient;
use crate::output;

/// Time window shared by both stats endpoints; absent bounds mean "all time"
/// and "now" on the server.
pub struct Window<'a> {
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
}

fn window_params(window: &Window<'_>) -> [(&'static str, Option<String>); 2] {
    [
        ("received_after", window.since.map(str::to_string)),
        ("received_before", window.until.map(str::to_string)),
    ]
}

fn overview_path(source_ids: &[String], window: &Window<'_>) -> String {
    let source_ids = (!source_ids.is_empty()).then(|| source_ids.join(","));
    let mut params = vec![("source_ids", source_ids)];
    params.extend(window_params(window));
    format!("/api/v1/stats/overview{}", query_string(&params))
}

fn volume_path(window: &Window<'_>) -> String {
    format!(
        "/api/v1/stats/volume-by-source{}",
        query_string(&window_params(window))
    )
}

fn latency(body: &Value, percentile: &str) -> String {
    match body["e2e_latency_ms"][percentile].as_f64() {
        Some(milliseconds) => format!("{milliseconds:.0} ms"),
        None => "-".to_string(),
    }
}

fn table_or(empty_label: &str, headers: &[&str], rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        empty_label.to_string()
    } else {
        output::render_table(headers, rows)
    }
}

fn items_rows(body: &Value, key: &str, fields: &[&str]) -> Vec<Vec<String>> {
    body[key]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    fields
                        .iter()
                        .map(|field| output::cell(item, field))
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn render_overview(body: &Value) -> String {
    let summary = output::render_record(&[
        (
            "window",
            format!(
                "{} → {}",
                output::cell(body, "range_start"),
                output::cell(body, "range_end")
            ),
        ),
        ("events", output::cell(body, "total_events")),
        ("failed attempts", output::cell(body, "failed_attempts")),
        ("latency p50", latency(body, "p50_ms")),
        ("latency p95", latency(body, "p95_ms")),
        ("latency p99", latency(body, "p99_ms")),
    ]);
    let statuses = table_or(
        "(no deliveries)",
        &["STATUS", "DELIVERIES"],
        &items_rows(body, "deliveries_by_status", &["status", "count"]),
    );
    let bucket_header = output::cell(body, "bucket_unit").to_uppercase();
    let buckets = table_or(
        "(no events)",
        &[bucket_header.as_str(), "EVENTS"],
        &items_rows(body, "events_per_bucket", &["bucket", "count"]),
    );
    [summary, statuses, buckets].join("\n\n")
}

pub async fn overview(
    client: &ApiClient,
    source_selectors: &[String],
    window: Window<'_>,
    json_output: bool,
) -> Result<()> {
    let mut source_ids = Vec::new();
    for selector in source_selectors {
        source_ids.push(client.resolve_source(selector).await?.id);
    }
    let body = client
        .get_json(&overview_path(&source_ids, &window))
        .await?;
    if json_output {
        output::print_json(&body);
    } else {
        println!("{}", render_overview(&body));
    }
    Ok(())
}

pub async fn by_source(client: &ApiClient, window: Window<'_>, json_output: bool) -> Result<()> {
    let body = client.get_json(&volume_path(&window)).await?;
    if json_output {
        output::print_json(&body);
        return Ok(());
    }
    output::print_table(
        &["NAME", "EVENTS", "SOURCE ID"],
        &items_rows(&body, "items", &["name", "count", "source_id"]),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn overview_body() -> Value {
        json!({
            "total_events": 1204,
            "events_per_bucket": [
                {"bucket": "2026-09-23T10:00:00Z", "count": 700},
                {"bucket": "2026-09-23T11:00:00Z", "count": 504}
            ],
            "bucket_unit": "hour",
            "range_start": "2026-09-22T12:00:00Z",
            "range_end": "2026-09-23T12:00:00Z",
            "deliveries_by_status": [
                {"status": "succeeded", "count": 1180},
                {"status": "exhausted", "count": 3}
            ],
            "failed_attempts": 41,
            "e2e_latency_ms": {"p50_ms": 120.4, "p95_ms": 880.0, "p99_ms": null}
        })
    }

    #[test]
    fn overview_path_joins_source_ids_and_maps_the_window() {
        let window = Window {
            since: Some("2026-09-20T10:00:00Z"),
            until: None,
        };
        assert_eq!(
            overview_path(&["a".to_string(), "b".to_string()], &window),
            "/api/v1/stats/overview?source_ids=a%2Cb&received_after=2026-09-20T10%3A00%3A00Z"
        );
        let everything = Window {
            since: None,
            until: None,
        };
        assert_eq!(overview_path(&[], &everything), "/api/v1/stats/overview");
    }

    #[test]
    fn volume_path_maps_the_window() {
        let window = Window {
            since: None,
            until: Some("2026-09-23T00:00:00Z"),
        };
        assert_eq!(
            volume_path(&window),
            "/api/v1/stats/volume-by-source?received_before=2026-09-23T00%3A00%3A00Z"
        );
    }

    #[test]
    fn overview_renders_totals_latency_statuses_and_buckets() {
        let rendered = render_overview(&overview_body());
        assert_eq!(
            rendered,
            "window:          2026-09-22T12:00:00Z → 2026-09-23T12:00:00Z\n\
             events:          1204\n\
             failed attempts: 41\n\
             latency p50:     120 ms\n\
             latency p95:     880 ms\n\
             latency p99:     -\n\
             \n\
             STATUS     DELIVERIES\n\
             succeeded  1180\n\
             exhausted  3\n\
             \n\
             HOUR                  EVENTS\n\
             2026-09-23T10:00:00Z  700\n\
             2026-09-23T11:00:00Z  504"
        );
    }

    #[test]
    fn empty_tables_say_none() {
        let mut body = overview_body();
        body["deliveries_by_status"] = json!([]);
        body["events_per_bucket"] = json!([]);
        let rendered = render_overview(&body);
        assert!(
            rendered.ends_with("latency p99:     -\n\n(no deliveries)\n\n(no events)"),
            "{rendered}"
        );
    }
}
