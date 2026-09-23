//! Destination create/edit forms and their request bodies.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::validation::{finish, http_url, json_field, required, selected, FieldError};
use crate::tui::forms::form::{Field, Form, SelectOption};
use crate::tui::model::Destination;

const AUTH_TYPES: [&str; 4] = ["none", "hmac", "api_key", "basic_auth"];
const STATUSES: [&str; 2] = ["active", "paused"];
const TIMEOUT_RANGE: std::ops::RangeInclusive<i64> = 1..=300_000;
const RETRY_COUNT: std::ops::RangeInclusive<usize> = 1..=10;
const RETRY_SECONDS: std::ops::RangeInclusive<u64> = 1..=86_400;

fn options(values: &[&str]) -> Vec<SelectOption> {
    values
        .iter()
        .map(|value| SelectOption::new(*value, *value))
        .collect()
}

fn auth_setting<'a>(destination: &'a Destination, key: &str) -> &'a str {
    destination
        .auth_config
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn fields(destination: Option<&Destination>) -> Vec<Field> {
    let text = |value: Option<&str>| value.unwrap_or("").to_string();
    let auth_type = destination.map_or("none", Destination::auth_type);
    let mut fields = vec![
        Field::text(
            "name",
            "Name",
            text(destination.map(|item| item.name.as_str())),
        ),
        Field::text(
            "url",
            "URL",
            text(destination.map(|item| item.url.as_str())),
        ),
        Field::headers(
            "headers",
            "Headers",
            &destination
                .map(|item| {
                    item.custom_headers
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        ),
        Field::select("auth_type", "Auth", options(&AUTH_TYPES), Some(auth_type)),
        Field::secret("auth_secret", "HMAC secret"),
        Field::text(
            "api_key_header",
            "API key header",
            destination.map_or("", |item| auth_setting(item, "header")),
        ),
        Field::secret("api_key", "API key"),
        Field::text(
            "basic_username",
            "Basic auth user",
            destination.map_or("", |item| auth_setting(item, "username")),
        ),
        Field::secret("basic_password", "Basic auth password"),
        Field::text(
            "timeout_ms",
            "Timeout ms (empty = default)",
            destination
                .and_then(|item| item.timeout_ms)
                .map(|milliseconds| milliseconds.to_string())
                .unwrap_or_default(),
        ),
        Field::json(
            "retry",
            "Retry intervals, seconds (empty = default)",
            destination
                .and_then(Destination::retry_intervals)
                .map_or(Value::Null, |intervals| json!(intervals)),
        ),
    ];
    if let Some(destination) = destination {
        if STATUSES.contains(&destination.status.as_str()) {
            fields.push(Field::select(
                "status",
                "Status",
                options(&STATUSES),
                Some(destination.status.as_str()),
            ));
        }
    }
    fields
}

pub fn create_form() -> Form {
    Form::new("New destination", fields(None))
}

pub fn edit_form(destination: &Destination) -> Form {
    Form::new(
        format!("Edit {}", destination.name),
        fields(Some(destination)),
    )
}

pub fn create_body(form: &Form) -> Result<Value, Vec<FieldError>> {
    let mut errors = Vec::new();
    let name = required(form, "name", &mut errors);
    let url = http_url(form, "url", &mut errors);
    let headers = headers(form, &mut errors);
    let auth = auth(form, None, &mut errors);
    let timeout = timeout(form, &mut errors);
    let retry = json_field(form, "retry");
    let retry = if retry.is_null() {
        None
    } else {
        retry_policy(&retry, &mut errors)
    };

    let mut body = json!({"name": name, "url": url});
    if let Some(headers) = headers.filter(|headers| !headers.is_empty()) {
        body["custom_headers"] = json!(headers);
    }
    if let Some(auth) = auth {
        body["auth_config"] = auth;
    }
    if let Some(timeout) = timeout {
        body["timeout_ms"] = json!(timeout);
    }
    if let Some(retry) = retry {
        body["retry_policy"] = retry;
    }
    finish(errors, body)
}

/// Only changed fields; `Ok(None)` when nothing changed. An empty timeout
/// keeps the current one (the API cannot unset it).
pub fn edit_body(form: &Form, destination: &Destination) -> Result<Option<Value>, Vec<FieldError>> {
    let mut errors = Vec::new();
    let mut body = Map::new();
    let name = required(form, "name", &mut errors);
    if name != destination.name {
        body.insert("name".into(), json!(name));
    }
    let url = http_url(form, "url", &mut errors);
    if url != destination.url {
        body.insert("url".into(), json!(url));
    }
    if let Some(headers) = headers(form, &mut errors) {
        if headers != destination.custom_headers {
            body.insert("custom_headers".into(), json!(headers));
        }
    }
    if let Some(auth) = auth(form, Some(destination), &mut errors) {
        body.insert("auth_config".into(), auth);
    }
    if let Some(timeout) = timeout(form, &mut errors) {
        if Some(timeout) != destination.timeout_ms {
            body.insert("timeout_ms".into(), json!(timeout));
        }
    }
    let retry = json_field(form, "retry");
    let current = destination
        .retry_intervals()
        .map(|intervals| json!(intervals));
    if retry.is_null() {
        if current.is_some() {
            body.insert("retry_policy".into(), Value::Null);
        }
    } else if Some(&retry) != current.as_ref() {
        if let Some(policy) = retry_policy(&retry, &mut errors) {
            body.insert("retry_policy".into(), policy);
        }
    }
    let status = selected(form, "status", &destination.status);
    if status != destination.status {
        body.insert("status".into(), json!(status));
    }
    finish(errors, (!body.is_empty()).then_some(Value::Object(body)))
}

fn headers(form: &Form, errors: &mut Vec<FieldError>) -> Option<BTreeMap<String, String>> {
    match form.headers("headers") {
        Ok(pairs) => Some(pairs.into_iter().collect()),
        Err(message) => {
            errors.push(("headers", message));
            None
        }
    }
}

fn timeout(form: &Form, errors: &mut Vec<FieldError>) -> Option<i64> {
    let raw = form.text("timeout_ms").trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let parsed = raw
        .parse::<i64>()
        .ok()
        .filter(|value| TIMEOUT_RANGE.contains(value));
    if parsed.is_none() {
        errors.push((
            "timeout_ms",
            "Use a whole number from 1 to 300000".to_string(),
        ));
    }
    parsed
}

/// A JSON array of 1-10 whole seconds, each 1..86400.
pub fn retry_policy(value: &Value, errors: &mut Vec<FieldError>) -> Option<Value> {
    let intervals: Option<Vec<u64>> = value
        .as_array()
        .filter(|items| RETRY_COUNT.contains(&items.len()))
        .and_then(|items| items.iter().map(Value::as_u64).collect())
        .filter(|seconds: &Vec<u64>| seconds.iter().all(|second| RETRY_SECONDS.contains(second)));
    match intervals {
        Some(intervals) => Some(json!({"intervals_seconds": intervals})),
        None => {
            errors.push((
                "retry",
                "Use a JSON array of 1 to 10 whole seconds, each 1 to 86400, e.g. [30, 120, 600]"
                    .to_string(),
            ));
            None
        }
    }
}

/// `None` = unchanged (edit) or default `none` (create). With the same type
/// and empty secrets the stored auth is kept; changing the header or user
/// requires re-entering the secret, because the API replaces auth wholesale.
fn auth(
    form: &Form,
    original: Option<&Destination>,
    errors: &mut Vec<FieldError>,
) -> Option<Value> {
    let auth_type = selected(form, "auth_type", "none");
    let same_type = original.map(Destination::auth_type) == Some(auth_type.as_str());
    let text = |key: &str| form.text(key).trim().to_string();
    match auth_type.as_str() {
        "none" => match original {
            Some(_) if !same_type => Some(json!({"type": "none"})),
            _ => None,
        },
        "hmac" => {
            let secret = text("auth_secret");
            if !secret.is_empty() {
                return Some(json!({"type": "hmac", "secret": secret}));
            }
            if !same_type {
                errors.push(("auth_secret", "Required for hmac".to_string()));
            }
            None
        }
        "api_key" => credentials(
            ("api_key_header", text("api_key_header"), "header"),
            ("api_key", text("api_key"), "key"),
            "api_key",
            original.filter(|_| same_type),
            errors,
        ),
        "basic_auth" => credentials(
            ("basic_username", text("basic_username"), "username"),
            ("basic_password", text("basic_password"), "password"),
            "basic_auth",
            original.filter(|_| same_type),
            errors,
        ),
        _ => None,
    }
}

/// A (visible, secret) pair such as header + key or username + password.
fn credentials(
    (visible_key, visible, visible_name): (&'static str, String, &str),
    (secret_key, secret, secret_name): (&'static str, String, &str),
    auth_type: &str,
    same_type_original: Option<&Destination>,
    errors: &mut Vec<FieldError>,
) -> Option<Value> {
    if visible.is_empty() {
        errors.push((visible_key, "Required".to_string()));
        return None;
    }
    if !secret.is_empty() {
        return Some(json!({"type": auth_type, visible_name: visible, secret_name: secret}));
    }
    match same_type_original {
        Some(original) if auth_setting(original, visible_name) == visible => None,
        Some(_) => {
            errors.push((secret_key, "Enter it again to change this auth".to_string()));
            None
        }
        None => {
            errors.push((secret_key, "Required".to_string()));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::crud::test_support::{replace_field, select, set_json, set_text};
    use crate::tui::fixtures;
    use serde_json::json;

    fn billing() -> Destination {
        fixtures::destination(
            fixtures::BILLING_ID,
            "billing-worker",
            "https://billing.internal/hooks",
            "closed",
        )
    }

    #[test]
    fn create_sends_only_what_was_given() {
        let mut form = create_form();
        set_text(&mut form, "name", "worker");
        set_text(&mut form, "url", "https://example.com/hook");
        assert_eq!(
            create_body(&form).unwrap(),
            json!({"name": "worker", "url": "https://example.com/hook"})
        );
    }

    #[test]
    fn create_with_headers_auth_timeout_and_retries() {
        let mut form = create_form();
        set_text(&mut form, "name", "worker");
        set_text(&mut form, "url", "https://example.com/hook");
        replace_field(
            &mut form,
            Field::headers("headers", "Headers", &[("X-Env".into(), "prod".into())]),
        );
        select(&mut form, "auth_type", "api_key");
        set_text(&mut form, "api_key_header", "X-Key");
        set_text(&mut form, "api_key", "k-123");
        set_text(&mut form, "timeout_ms", "5000");
        set_json(&mut form, "retry", json!([30, 120]));
        assert_eq!(
            create_body(&form).unwrap(),
            json!({
                "name": "worker",
                "url": "https://example.com/hook",
                "custom_headers": {"X-Env": "prod"},
                "auth_config": {"type": "api_key", "header": "X-Key", "key": "k-123"},
                "timeout_ms": 5000,
                "retry_policy": {"intervals_seconds": [30, 120]}
            })
        );
    }

    #[test]
    fn create_validation() {
        let mut form = create_form();
        set_text(&mut form, "url", "ftp://example.com");
        select(&mut form, "auth_type", "hmac");
        set_text(&mut form, "timeout_ms", "300001");
        set_json(&mut form, "retry", json!([0, 90000]));
        let keys: Vec<&str> = create_body(&form)
            .unwrap_err()
            .iter()
            .map(|(key, _)| *key)
            .collect();
        assert_eq!(
            keys,
            vec!["name", "url", "auth_secret", "timeout_ms", "retry"]
        );
    }

    #[test]
    fn retry_lists_are_bounded() {
        let mut errors = Vec::new();
        assert_eq!(retry_policy(&json!([]), &mut errors), None);
        assert_eq!(retry_policy(&json!(vec![60; 11]), &mut errors), None);
        assert_eq!(retry_policy(&json!(["30"]), &mut errors), None);
        assert_eq!(errors.len(), 3);
        assert_eq!(
            retry_policy(&json!([1, 86400]), &mut Vec::new()),
            Some(json!({"intervals_seconds": [1, 86400]}))
        );
    }

    #[test]
    fn an_untouched_edit_changes_nothing() {
        let destination = billing();
        assert_eq!(
            edit_body(&edit_form(&destination), &destination).unwrap(),
            None
        );
    }

    #[test]
    fn an_empty_secret_keeps_the_auth_and_a_new_one_replaces_it() {
        let destination = billing();
        let mut form = edit_form(&destination);
        set_text(&mut form, "timeout_ms", "2000");
        assert_eq!(
            edit_body(&form, &destination).unwrap(),
            Some(json!({"timeout_ms": 2000}))
        );
        set_text(&mut form, "auth_secret", "new-secret");
        assert_eq!(
            edit_body(&form, &destination).unwrap().unwrap()["auth_config"],
            json!({"type": "hmac", "secret": "new-secret"})
        );
    }

    #[test]
    fn switching_auth_off_and_resetting_retries() {
        let destination = billing();
        let mut form = edit_form(&destination);
        select(&mut form, "auth_type", "none");
        set_json(&mut form, "retry", Value::Null);
        assert_eq!(
            edit_body(&form, &destination).unwrap(),
            Some(json!({"auth_config": {"type": "none"}, "retry_policy": null}))
        );
    }

    #[test]
    fn changed_headers_replace_the_whole_set() {
        let destination = billing();
        let mut form = edit_form(&destination);
        replace_field(
            &mut form,
            Field::headers(
                "headers",
                "Headers",
                &[
                    ("X-Env".into(), "prod".into()),
                    ("X-Team".into(), "billing".into()),
                ],
            ),
        );
        assert_eq!(
            edit_body(&form, &destination).unwrap(),
            Some(json!({"custom_headers": {"X-Env": "prod", "X-Team": "billing"}}))
        );
    }
}
