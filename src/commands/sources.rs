use anyhow::Result;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::output;

/// Query string for the paginated listings (`ls` and `trash`).
fn list_query(search: Option<&str>, page: Option<i64>, limit: Option<i64>) -> String {
    crate::args::query_string(&[
        ("q", search.map(str::to_string)),
        ("page", page.map(|value| value.to_string())),
        ("limit", limit.map(|value| value.to_string())),
    ])
}

fn create_body(name: &str, color: Option<&str>, verification: Option<Value>) -> Value {
    let mut body = json!({"name": name});
    if let Some(color) = color {
        body["color"] = json!(color);
    }
    if let Some(config) = verification {
        body["verification_config"] = config;
    }
    body
}

pub struct UpdateFields<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub color: Option<&'a str>,
    pub status: Option<&'a str>,
    pub verification: Option<Value>,
}

fn update_body(fields: &UpdateFields<'_>) -> Result<Value> {
    let mut body = serde_json::Map::new();
    if let Some(name) = fields.name {
        body.insert("name".into(), json!(name));
    }
    if let Some(description) = fields.description {
        body.insert("description".into(), json!(description));
    }
    if let Some(color) = fields.color {
        body.insert("color".into(), json!(color));
    }
    if let Some(status) = fields.status {
        body.insert("status".into(), json!(status));
    }
    if let Some(config) = fields.verification.clone() {
        body.insert("verification_config".into(), config);
    }
    if body.is_empty() {
        anyhow::bail!("nothing to update: pass at least one field");
    }
    Ok(Value::Object(body))
}

fn source_row(source: &Value) -> Vec<String> {
    vec![
        output::cell(source, "name"),
        output::cell(source, "status"),
        output::cell(source, "event_count"),
        output::cell(source, "ingest_url"),
    ]
}

fn print_source(source: &Value, json_output: bool) {
    if json_output {
        output::print_json(source);
        return;
    }
    output::print_record(&[
        ("id", output::cell(source, "id")),
        ("name", output::cell(source, "name")),
        ("status", output::cell(source, "status")),
        ("verification", output::cell(source, "verification_config")),
        ("ingest url", output::cell(source, "ingest_url")),
        ("created at", output::cell(source, "created_at")),
    ]);
}

pub async fn list(
    client: &ApiClient,
    search: Option<&str>,
    page: Option<i64>,
    limit: Option<i64>,
    json_output: bool,
) -> Result<()> {
    let path = format!("/api/v1/sources/{}", list_query(search, page, limit));
    let body = client.get_json(&path).await?;
    if json_output {
        output::print_json(&body);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = body["items"]
        .as_array()
        .map(|items| items.iter().map(source_row).collect())
        .unwrap_or_default();
    output::print_table(&["NAME", "STATUS", "EVENTS", "INGEST URL"], &rows);
    Ok(())
}

pub async fn create(
    client: &ApiClient,
    name: &str,
    color: Option<&str>,
    verification: Option<Value>,
    json_output: bool,
) -> Result<()> {
    let created = client
        .post_json("/api/v1/sources/", create_body(name, color, verification))
        .await?;
    print_source(&created, json_output);
    Ok(())
}

pub async fn get(client: &ApiClient, selector: &str, json_output: bool) -> Result<()> {
    let source = client.resolve_source(selector).await?;
    let detail = client
        .get_json(&format!("/api/v1/sources/{}", source.id))
        .await?;
    print_source(&detail, json_output);
    Ok(())
}

pub async fn update(
    client: &ApiClient,
    selector: &str,
    fields: UpdateFields<'_>,
    json_output: bool,
) -> Result<()> {
    let body = update_body(&fields)?;
    let source = client.resolve_source(selector).await?;
    let updated = client
        .patch_json(&format!("/api/v1/sources/{}", source.id), body)
        .await?;
    print_source(&updated, json_output);
    Ok(())
}

pub async fn remove(client: &ApiClient, selector: &str, json_output: bool) -> Result<()> {
    let source = client.resolve_source(selector).await?;
    client
        .delete(&format!("/api/v1/sources/{}", source.id))
        .await?;
    output::print_deleted("source", &source.id, &source.name, json_output);
    Ok(())
}

pub async fn trash(
    client: &ApiClient,
    search: Option<&str>,
    page: Option<i64>,
    limit: Option<i64>,
    json_output: bool,
) -> Result<()> {
    let path = format!("/api/v1/sources/trash{}", list_query(search, page, limit));
    let body = client.get_json(&path).await?;
    if json_output {
        output::print_json(&body);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|source| {
                    vec![
                        output::cell(source, "id"),
                        output::cell(source, "name"),
                        output::cell(source, "deleted_at"),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    output::print_table(&["ID", "NAME", "DELETED AT"], &rows);
    Ok(())
}

/// Restore takes the trashed source's id: a trashed source is invisible to the
/// name/token resolver, which only sees live sources.
pub async fn restore(client: &ApiClient, source_id: &str, json_output: bool) -> Result<()> {
    let restored = client
        .post_json(&format!("/api/v1/sources/{source_id}/restore"), json!({}))
        .await?;
    print_source(&restored, json_output);
    Ok(())
}

pub async fn rotate_token(client: &ApiClient, selector: &str, json_output: bool) -> Result<()> {
    let source = client.resolve_source(selector).await?;
    let rotated = client
        .post_json(
            &format!("/api/v1/sources/{}/rotate-token", source.id),
            json!({}),
        )
        .await?;
    if !json_output {
        eprintln!("The previous ingest URL stops accepting webhooks within 30 seconds.");
    }
    print_source(&rotated, json_output);
    Ok(())
}

/// Prints the ingest URL alone, so it can be piped straight into curl.
pub async fn url(client: &ApiClient, selector: &str) -> Result<()> {
    let source = client.resolve_source(selector).await?;
    let detail = client
        .get_json(&format!("/api/v1/sources/{}", source.id))
        .await?;
    println!("{}", output::cell(&detail, "ingest_url"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_query_omits_absent_filters() {
        assert_eq!(list_query(None, None, None), "");
        assert_eq!(list_query(None, Some(2), Some(50)), "?page=2&limit=50");
    }

    #[test]
    fn list_query_encodes_the_search_term() {
        assert_eq!(list_query(Some("a b&c"), None, None), "?q=a%20b%26c");
    }

    #[test]
    fn create_body_carries_only_what_was_given() {
        assert_eq!(create_body("stripe", None, None), json!({"name": "stripe"}));
        assert_eq!(
            create_body(
                "stripe",
                Some("#3b82f6"),
                Some(json!({"provider": "stripe"}))
            ),
            json!({
                "name": "stripe",
                "color": "#3b82f6",
                "verification_config": {"provider": "stripe"}
            })
        );
    }

    #[test]
    fn update_body_rejects_an_empty_patch() {
        let fields = UpdateFields {
            name: None,
            description: None,
            color: None,
            status: None,
            verification: None,
        };
        assert!(update_body(&fields).is_err());
    }

    #[test]
    fn update_body_sends_only_the_supplied_fields() {
        let fields = UpdateFields {
            name: None,
            description: None,
            color: None,
            status: Some("paused"),
            verification: None,
        };
        assert_eq!(update_body(&fields).unwrap(), json!({"status": "paused"}));
    }
}
