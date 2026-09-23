use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::client::ApiClient;
use crate::output;

fn create_body(
    source_id: &str,
    destination_id: &str,
    filter_rules: Option<Value>,
    transformation: Option<Value>,
) -> Value {
    let mut body = json!({"source_id": source_id, "destination_id": destination_id});
    if let Some(rules) = filter_rules {
        body["filter_rules"] = rules;
    }
    if let Some(transformation) = transformation {
        body["transformation"] = transformation;
    }
    body
}

pub struct UpdateFields {
    pub enabled: Option<bool>,
    /// `Some(Value::Null)` clears the rules, which is what `--filter null` sends.
    pub filter_rules: Option<Value>,
    pub transformation: Option<Value>,
}

fn update_body(fields: &UpdateFields) -> Result<Value> {
    let mut body = serde_json::Map::new();
    if let Some(enabled) = fields.enabled {
        body.insert("enabled".into(), json!(enabled));
    }
    if let Some(rules) = fields.filter_rules.clone() {
        body.insert("filter_rules".into(), rules);
    }
    if let Some(transformation) = fields.transformation.clone() {
        body.insert("transformation".into(), transformation);
    }
    if body.is_empty() {
        anyhow::bail!("nothing to update: pass at least one field");
    }
    Ok(Value::Object(body))
}

fn print_connection(connection: &Value, json_output: bool) {
    if json_output {
        output::print_json(connection);
        return;
    }
    output::print_record(&[
        ("id", output::cell(connection, "id")),
        ("source id", output::cell(connection, "source_id")),
        ("destination id", output::cell(connection, "destination_id")),
        ("enabled", output::cell(connection, "enabled")),
        ("filter rules", output::cell(connection, "filter_rules")),
        ("transformation", output::cell(connection, "transformation")),
    ]);
}

/// Connections carry ids, not names, so the listing resolves both ends through
/// the sources and destinations the workspace already exposes.
async fn name_lookup(
    client: &ApiClient,
) -> Result<(HashMap<String, String>, HashMap<String, String>)> {
    let sources = client
        .list_sources()
        .await?
        .into_iter()
        .map(|source| (source.id, source.name))
        .collect();
    let destinations = client
        .list_destinations()
        .await?
        .into_iter()
        .map(|destination| (destination.id, destination.name))
        .collect();
    Ok((sources, destinations))
}

fn named(lookup: &HashMap<String, String>, id: &str) -> String {
    lookup.get(id).cloned().unwrap_or_else(|| id.to_string())
}

pub async fn list(client: &ApiClient, source: Option<&str>, json_output: bool) -> Result<()> {
    let path = match source {
        Some(selector) => {
            let source = client.resolve_source(selector).await?;
            format!("/api/v1/sources/{}/connections", source.id)
        }
        None => "/api/v1/connections/".to_string(),
    };
    let body = client.get_json(&path).await?;
    if json_output {
        output::print_json(&body);
        return Ok(());
    }
    let (sources, destinations) = name_lookup(client).await?;
    let rows: Vec<Vec<String>> = body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|connection| {
                    vec![
                        output::cell(connection, "id"),
                        named(&sources, &output::cell(connection, "source_id")),
                        named(&destinations, &output::cell(connection, "destination_id")),
                        output::cell(connection, "enabled"),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    output::print_table(&["ID", "SOURCE", "DESTINATION", "ENABLED"], &rows);
    Ok(())
}

pub async fn create(
    client: &ApiClient,
    source_selector: &str,
    destination_selector: &str,
    filter_rules: Option<Value>,
    transformation: Option<Value>,
    json_output: bool,
) -> Result<()> {
    let source = client.resolve_source(source_selector).await?;
    let destination = client.resolve_destination(destination_selector).await?;
    let created = client
        .post_json(
            "/api/v1/connections/",
            create_body(&source.id, &destination.id, filter_rules, transformation),
        )
        .await?;
    if !json_output {
        eprintln!("Connected \"{}\" → \"{}\"", source.name, destination.name);
    }
    print_connection(&created, json_output);
    Ok(())
}

pub async fn update(
    client: &ApiClient,
    connection_id: &str,
    fields: UpdateFields,
    json_output: bool,
) -> Result<()> {
    let updated = client
        .patch_json(
            &format!("/api/v1/connections/{connection_id}"),
            update_body(&fields)?,
        )
        .await?;
    print_connection(&updated, json_output);
    Ok(())
}

pub async fn remove(client: &ApiClient, connection_id: &str, json_output: bool) -> Result<()> {
    client
        .delete(&format!("/api/v1/connections/{connection_id}"))
        .await?;
    output::print_deleted("connection", connection_id, connection_id, json_output);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_body_carries_only_what_was_given() {
        assert_eq!(
            create_body("src", "dst", None, None),
            json!({"source_id": "src", "destination_id": "dst"})
        );
        assert_eq!(
            create_body("src", "dst", Some(json!({"all": []})), None)["filter_rules"],
            json!({"all": []})
        );
    }

    #[test]
    fn update_body_distinguishes_clearing_from_leaving_unchanged() {
        // An explicit null clears the rules; an absent field leaves them alone.
        let cleared = update_body(&UpdateFields {
            enabled: None,
            filter_rules: Some(Value::Null),
            transformation: None,
        })
        .unwrap();
        assert_eq!(cleared, json!({"filter_rules": null}));
        assert!(cleared.as_object().unwrap().contains_key("filter_rules"));

        let untouched = update_body(&UpdateFields {
            enabled: Some(false),
            filter_rules: None,
            transformation: None,
        })
        .unwrap();
        assert_eq!(untouched, json!({"enabled": false}));
    }

    #[test]
    fn update_body_rejects_an_empty_patch() {
        assert!(update_body(&UpdateFields {
            enabled: None,
            filter_rules: None,
            transformation: None,
        })
        .is_err());
    }

    #[test]
    fn named_falls_back_to_the_id_when_unknown() {
        let mut lookup = HashMap::new();
        lookup.insert("known".to_string(), "stripe".to_string());
        assert_eq!(named(&lookup, "known"), "stripe");
        assert_eq!(named(&lookup, "other"), "other");
    }
}
