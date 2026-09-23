use anyhow::Result;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::client::ApiClient;
use crate::output;
use crate::relay::parse_header_flag;

/// Outbound auth methods whose only input is a secret, so the CLI can accept
/// `--auth hmac` and read the secret off a prompt instead of argv.
const SECRET_ONLY_AUTH: [&str; 1] = ["hmac"];

/// Builds an `auth_config` payload from a preset name plus a prompted secret, a
/// full JSON config, or the literal `none` to disable outbound signing.
pub fn parse_auth_arg(raw: &str, read_secret: impl FnOnce() -> Result<String>) -> Result<Value> {
    if raw == "none" {
        return Ok(json!({"type": "none"}));
    }
    if SECRET_ONLY_AUTH.contains(&raw) {
        let secret = read_secret()?;
        if secret.trim().is_empty() {
            anyhow::bail!("the signing secret must not be empty");
        }
        return Ok(json!({"type": raw, "secret": secret}));
    }
    let config = crate::args::parse_json_arg(raw)?;
    if config.get("type").is_none() {
        anyhow::bail!("auth config needs a \"type\" field, or pass one of: hmac, none");
    }
    Ok(config)
}

fn headers_map(headers: &[String]) -> Result<BTreeMap<String, String>> {
    headers.iter().map(|raw| parse_header_flag(raw)).collect()
}

pub struct CreateFields<'a> {
    pub name: &'a str,
    pub url: &'a str,
    pub headers: &'a [String],
    pub auth: Option<Value>,
    pub timeout_ms: Option<i64>,
    pub retry_policy: Option<Value>,
}

fn create_body(fields: &CreateFields<'_>) -> Result<Value> {
    let mut body = json!({"name": fields.name, "url": fields.url});
    let headers = headers_map(fields.headers)?;
    if !headers.is_empty() {
        body["custom_headers"] = json!(headers);
    }
    if let Some(auth) = fields.auth.clone() {
        body["auth_config"] = auth;
    }
    if let Some(timeout) = fields.timeout_ms {
        body["timeout_ms"] = json!(timeout);
    }
    if let Some(policy) = fields.retry_policy.clone() {
        body["retry_policy"] = policy;
    }
    Ok(body)
}

pub struct UpdateFields<'a> {
    pub name: Option<&'a str>,
    pub url: Option<&'a str>,
    pub headers: &'a [String],
    pub auth: Option<Value>,
    pub timeout_ms: Option<i64>,
    pub retry_policy: Option<Value>,
    pub status: Option<&'a str>,
}

fn update_body(fields: &UpdateFields<'_>) -> Result<Value> {
    let mut body = serde_json::Map::new();
    if let Some(name) = fields.name {
        body.insert("name".into(), json!(name));
    }
    if let Some(url) = fields.url {
        body.insert("url".into(), json!(url));
    }
    if !fields.headers.is_empty() {
        // Custom headers replace the stored set rather than merging into it,
        // matching the API's PATCH contract.
        body.insert("custom_headers".into(), json!(headers_map(fields.headers)?));
    }
    if let Some(auth) = fields.auth.clone() {
        body.insert("auth_config".into(), auth);
    }
    if let Some(timeout) = fields.timeout_ms {
        body.insert("timeout_ms".into(), json!(timeout));
    }
    if let Some(policy) = fields.retry_policy.clone() {
        body.insert("retry_policy".into(), policy);
    }
    if let Some(status) = fields.status {
        body.insert("status".into(), json!(status));
    }
    if body.is_empty() {
        anyhow::bail!("nothing to update: pass at least one field");
    }
    Ok(Value::Object(body))
}

fn print_destination(destination: &Value, json_output: bool) {
    if json_output {
        output::print_json(destination);
        return;
    }
    output::print_record(&[
        ("id", output::cell(destination, "id")),
        ("name", output::cell(destination, "name")),
        ("url", output::cell(destination, "url")),
        ("status", output::cell(destination, "status")),
        ("auth", output::cell(destination, "auth_config")),
        ("headers", output::cell(destination, "custom_headers")),
        ("timeout ms", output::cell(destination, "timeout_ms")),
        ("retry policy", output::cell(destination, "retry_policy")),
        ("circuit", output::cell(destination, "circuit_state")),
    ]);
}

pub async fn list(client: &ApiClient, json_output: bool) -> Result<()> {
    let body = client.get_json("/api/v1/destinations/").await?;
    if json_output {
        output::print_json(&body);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|destination| {
                    vec![
                        output::cell(destination, "name"),
                        output::cell(destination, "status"),
                        output::cell(destination, "circuit_state"),
                        output::cell(destination, "url"),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    output::print_table(&["NAME", "STATUS", "CIRCUIT", "URL"], &rows);
    Ok(())
}

pub async fn create(client: &ApiClient, fields: CreateFields<'_>, json_output: bool) -> Result<()> {
    let created = client
        .post_json("/api/v1/destinations/", create_body(&fields)?)
        .await?;
    print_destination(&created, json_output);
    Ok(())
}

pub async fn get(client: &ApiClient, selector: &str, json_output: bool) -> Result<()> {
    let destination = client.resolve_destination(selector).await?;
    let detail = client
        .get_json(&format!("/api/v1/destinations/{}", destination.id))
        .await?;
    print_destination(&detail, json_output);
    Ok(())
}

pub async fn update(
    client: &ApiClient,
    selector: &str,
    fields: UpdateFields<'_>,
    json_output: bool,
) -> Result<()> {
    let body = update_body(&fields)?;
    let destination = client.resolve_destination(selector).await?;
    let updated = client
        .patch_json(&format!("/api/v1/destinations/{}", destination.id), body)
        .await?;
    print_destination(&updated, json_output);
    Ok(())
}

pub async fn remove(client: &ApiClient, selector: &str, json_output: bool) -> Result<()> {
    let destination = client.resolve_destination(selector).await?;
    client
        .delete(&format!("/api/v1/destinations/{}", destination.id))
        .await?;
    output::print_deleted(
        "destination",
        &destination.id,
        &destination.name,
        json_output,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_preset_reads_the_secret_off_the_prompt() {
        let config = parse_auth_arg("hmac", || Ok("outbound-secret".to_string())).unwrap();
        assert_eq!(config, json!({"type": "hmac", "secret": "outbound-secret"}));
    }

    #[test]
    fn auth_none_disables_signing_without_a_prompt() {
        let config = parse_auth_arg("none", || unreachable!("secret must not be read")).unwrap();
        assert_eq!(config, json!({"type": "none"}));
    }

    #[test]
    fn auth_json_passes_through_and_needs_a_type() {
        let config = parse_auth_arg(
            r#"{"type":"api_key","header":"X-Key","key":"k"}"#,
            || unreachable!(),
        )
        .unwrap();
        assert_eq!(config["header"], "X-Key");
        assert!(parse_auth_arg(r#"{"key":"k"}"#, || unreachable!()).is_err());
    }

    #[test]
    fn create_body_maps_repeated_header_flags() {
        let headers = vec!["X-Env: prod".to_string(), "X-Team: billing".to_string()];
        let body = create_body(&CreateFields {
            name: "worker",
            url: "https://example.com/hook",
            headers: &headers,
            auth: None,
            timeout_ms: None,
            retry_policy: None,
        })
        .unwrap();
        assert_eq!(
            body,
            json!({
                "name": "worker",
                "url": "https://example.com/hook",
                "custom_headers": {"X-Env": "prod", "X-Team": "billing"}
            })
        );
    }

    #[test]
    fn create_body_rejects_a_malformed_header() {
        let headers = vec!["no-colon".to_string()];
        assert!(create_body(&CreateFields {
            name: "worker",
            url: "https://example.com/hook",
            headers: &headers,
            auth: None,
            timeout_ms: None,
            retry_policy: None,
        })
        .is_err());
    }

    #[test]
    fn update_body_sends_only_the_supplied_fields() {
        let body = update_body(&UpdateFields {
            name: None,
            url: None,
            headers: &[],
            auth: None,
            timeout_ms: Some(5000),
            retry_policy: None,
            status: Some("paused"),
        })
        .unwrap();
        assert_eq!(body, json!({"timeout_ms": 5000, "status": "paused"}));
    }

    #[test]
    fn update_body_rejects_an_empty_patch() {
        assert!(update_body(&UpdateFields {
            name: None,
            url: None,
            headers: &[],
            auth: None,
            timeout_ms: None,
            retry_policy: None,
            status: None,
        })
        .is_err());
    }
}
