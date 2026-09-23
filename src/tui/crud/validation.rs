//! Checks shared by the resource forms. Errors are `(field key, message)`
//! pairs, shown under the field by the form view.

use serde_json::{Map, Value};

use crate::tui::forms::form::Form;
use crate::tui::settings::{is_http_url, Rgb};

pub type FieldError = (&'static str, String);

pub fn required(form: &Form, key: &'static str, errors: &mut Vec<FieldError>) -> String {
    let value = form.text(key).trim().to_string();
    if value.is_empty() {
        errors.push((key, "Required".to_string()));
    }
    value
}

/// `None` when empty; an error unless `#rrggbb`.
pub fn optional_color(
    form: &Form,
    key: &'static str,
    errors: &mut Vec<FieldError>,
) -> Option<String> {
    let value = form.text(key).trim().to_string();
    if value.is_empty() {
        return None;
    }
    if Rgb::parse_hex(&value).is_none() {
        errors.push((key, "Use a #rrggbb color".to_string()));
        return None;
    }
    Some(value)
}

pub fn http_url(form: &Form, key: &'static str, errors: &mut Vec<FieldError>) -> String {
    let value = form.text(key).trim().to_string();
    if !is_http_url(&value) {
        errors.push((key, "Must be an absolute http(s) URL".to_string()));
    }
    value
}

/// The selected option's value, or `fallback` when the field is absent.
pub fn selected(form: &Form, key: &str, fallback: &str) -> String {
    form.selected(key)
        .map(|value| value.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

pub fn json_field(form: &Form, key: &str) -> Value {
    form.json(key).cloned().unwrap_or(Value::Null)
}

/// `Some` for an object or `null`, else an error on `key`.
pub fn object_or_null(
    value: &Value,
    key: &'static str,
    errors: &mut Vec<FieldError>,
) -> Option<Value> {
    if value.is_null() || value.is_object() {
        Some(value.clone())
    } else {
        errors.push((key, "Must be a JSON object, or empty".to_string()));
        None
    }
}

/// The API masks stored secrets as `***` and rejects them on write.
pub fn contains_masked(value: &Value) -> bool {
    match value {
        Value::String(text) => text == "***",
        Value::Array(items) => items.iter().any(contains_masked),
        Value::Object(map) => map.values().any(contains_masked),
        _ => false,
    }
}

pub fn without_key(value: &Value, key: &str) -> Value {
    match value {
        Value::Object(map) => {
            let mut copy = map.clone();
            copy.remove(key);
            Value::Object(copy)
        }
        other => other.clone(),
    }
}

pub fn with_key(value: &Value, key: &str, tag: &str) -> Value {
    let mut map = value.as_object().cloned().unwrap_or_else(Map::new);
    map.insert(key.to_string(), Value::String(tag.to_string()));
    Value::Object(map)
}

/// Turns collected errors into the builder result.
pub fn finish<T>(errors: Vec<FieldError>, value: T) -> Result<T, Vec<FieldError>> {
    if errors.is_empty() {
        Ok(value)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masked_secrets_are_found_at_any_depth() {
        assert!(contains_masked(&json!({"secret": "***"})));
        assert!(contains_masked(&json!({"nested": [{"key": "***"}]})));
        assert!(!contains_masked(&json!({"secret": "real"})));
    }

    #[test]
    fn provider_keys_are_stripped_and_restored() {
        let stored = json!({"provider": "api_key", "header": "X-Key", "key": "***"});
        assert_eq!(
            without_key(&stored, "provider"),
            json!({"header": "X-Key", "key": "***"})
        );
        assert_eq!(
            with_key(&json!({"header": "X-Key"}), "provider", "api_key"),
            json!({"header": "X-Key", "provider": "api_key"})
        );
    }

    #[test]
    fn object_or_null_only() {
        let mut errors = Vec::new();
        assert_eq!(
            object_or_null(&json!({"a": 1}), "filter", &mut errors),
            Some(json!({"a": 1}))
        );
        assert_eq!(
            object_or_null(&Value::Null, "filter", &mut errors),
            Some(Value::Null)
        );
        assert_eq!(object_or_null(&json!([1]), "filter", &mut errors), None);
        assert_eq!(
            errors,
            vec![("filter", "Must be a JSON object, or empty".to_string())]
        );
    }
}
