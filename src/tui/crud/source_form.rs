//! Source create/edit forms and their request bodies.

use serde_json::{json, Map, Value};

use super::validation::{
    contains_masked, finish, json_field, object_or_null, optional_color, required, selected,
    with_key, without_key, FieldError,
};
use crate::tui::forms::form::{Field, Form, SelectOption};
use crate::tui::model::Source;

pub const PRESET_PROVIDERS: [&str; 3] = ["stripe", "github", "shopify"];
pub const JSON_PROVIDERS: [&str; 3] = ["generic_hmac", "basic_auth", "api_key"];
const PROVIDERS: [&str; 7] = [
    "none",
    "stripe",
    "github",
    "shopify",
    "generic_hmac",
    "basic_auth",
    "api_key",
];
const STATUSES: [&str; 2] = ["active", "paused"];

fn options(values: &[&str]) -> Vec<SelectOption> {
    values
        .iter()
        .map(|value| SelectOption::new(*value, *value))
        .collect()
}

pub fn create_form() -> Form {
    Form::new(
        "New source",
        vec![
            Field::text("name", "Name", ""),
            Field::text("description", "Description", ""),
            Field::text("color", "Color (#rrggbb)", ""),
            Field::select("provider", "Verification", options(&PROVIDERS), None),
            Field::secret("secret", "Secret (stripe, github, shopify)"),
            Field::json(
                "verification",
                "Settings JSON (other providers)",
                Value::Null,
            ),
            Field::json("response", "Response config", Value::Null),
        ],
    )
}

/// Secrets are never shown: the secret field starts empty, and JSON settings
/// keep the API's `***` placeholders until the user replaces them.
pub fn edit_form(source: &Source) -> Form {
    let provider = source.verification_provider();
    let settings = if JSON_PROVIDERS.contains(&provider) {
        without_key(&source.verification_config, "provider")
    } else {
        Value::Null
    };
    let mut fields = vec![
        Field::text("name", "Name", &source.name),
        Field::text(
            "description",
            "Description",
            source.description.as_deref().unwrap_or(""),
        ),
        Field::text(
            "color",
            "Color (#rrggbb)",
            source.color.as_deref().unwrap_or(""),
        ),
    ];
    if STATUSES.contains(&source.status.as_str()) {
        fields.push(Field::select(
            "status",
            "Status",
            options(&STATUSES),
            Some(source.status.as_str()),
        ));
    }
    fields.extend([
        Field::select(
            "provider",
            "Verification",
            options(&PROVIDERS),
            Some(provider),
        ),
        Field::secret("secret", "Secret (empty = keep)"),
        Field::json("verification", "Settings JSON (other providers)", settings),
        Field::json(
            "response",
            "Response config",
            source.response_config.clone(),
        ),
    ]);
    Form::new(format!("Edit {}", source.name), fields)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateRequest {
    pub body: Value,
    pub follow_up: Option<Value>,
}

pub fn create_request(form: &Form) -> Result<CreateRequest, Vec<FieldError>> {
    let mut errors = Vec::new();
    let name = required(form, "name", &mut errors);
    let color = optional_color(form, "color", &mut errors);
    let verification = verification(form, None, &mut errors);
    let response = object_or_null(&json_field(form, "response"), "response", &mut errors);

    let mut body = json!({"name": name});
    if let Some(color) = color {
        body["color"] = json!(color);
    }
    if let Some(config) = verification {
        body["verification_config"] = config;
    }
    let mut follow_up = Map::new();
    let description = form.text("description").trim().to_string();
    if !description.is_empty() {
        follow_up.insert("description".into(), json!(description));
    }
    if let Some(response) = response.filter(|response| !response.is_null()) {
        follow_up.insert("response_config".into(), response);
    }
    finish(
        errors,
        CreateRequest {
            body,
            follow_up: (!follow_up.is_empty()).then_some(Value::Object(follow_up)),
        },
    )
}

/// Only changed fields; `Ok(None)` when nothing changed.
pub fn edit_body(form: &Form, source: &Source) -> Result<Option<Value>, Vec<FieldError>> {
    let mut errors = Vec::new();
    let mut body = Map::new();
    let name = required(form, "name", &mut errors);
    if name != source.name {
        body.insert("name".into(), json!(name));
    }
    let description = form.text("description").trim().to_string();
    if description != source.description.clone().unwrap_or_default() {
        body.insert("description".into(), json!(description));
    }
    match optional_color(form, "color", &mut errors) {
        Some(color) if Some(&color) != source.color.as_ref() => {
            body.insert("color".into(), json!(color));
        }
        None if source.color.is_some() && form.text("color").trim().is_empty() => {
            errors.push((
                "color",
                "A color cannot be removed; pick another one".to_string(),
            ));
        }
        _ => {}
    }
    let status = selected(form, "status", &source.status);
    if status != source.status {
        body.insert("status".into(), json!(status));
    }
    if let Some(config) = verification(form, Some(source), &mut errors) {
        body.insert("verification_config".into(), config);
    }
    if let Some(response) = object_or_null(&json_field(form, "response"), "response", &mut errors) {
        if response != source.response_config {
            body.insert("response_config".into(), response);
        }
    }
    finish(errors, (!body.is_empty()).then_some(Value::Object(body)))
}

/// `None` means "leave unchanged" (edit) or "no verification" (create).
fn verification(
    form: &Form,
    original: Option<&Source>,
    errors: &mut Vec<FieldError>,
) -> Option<Value> {
    let provider = selected(form, "provider", "none");
    let same_provider = original.map(Source::verification_provider) == Some(provider.as_str());
    let provider = provider.as_str();
    if provider == "none" {
        return match original {
            Some(_) if !same_provider => Some(json!({"provider": "none"})),
            _ => None,
        };
    }
    if PRESET_PROVIDERS.contains(&provider) {
        let secret = form.text("secret").trim().to_string();
        if !secret.is_empty() {
            return Some(json!({"provider": provider, "secret": secret}));
        }
        if !same_provider {
            errors.push(("secret", "Required for this provider".to_string()));
        }
        return None;
    }
    let settings = json_field(form, "verification");
    if same_provider {
        let stored = original.map(|source| without_key(&source.verification_config, "provider"));
        if stored.as_ref() == Some(&settings) {
            return None;
        }
    }
    if !settings.is_object() {
        errors.push((
            "verification",
            "Enter this provider's settings as a JSON object".to_string(),
        ));
        return None;
    }
    if contains_masked(&settings) {
        errors.push((
            "verification",
            "Replace *** with the real secret".to_string(),
        ));
        return None;
    }
    Some(with_key(&settings, "provider", provider))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::crud::test_support::{field_error, select, set_json, set_text};
    use crate::tui::fixtures;
    use serde_json::json;

    #[test]
    fn create_sends_name_color_and_a_preset_secret() {
        let mut form = create_form();
        set_text(&mut form, "name", " stripe-prod ");
        set_text(&mut form, "color", "#3b82f6");
        select(&mut form, "provider", "stripe");
        set_text(&mut form, "secret", "whsec_live");
        let request = create_request(&form).unwrap();
        assert_eq!(
            request.body,
            json!({
                "name": "stripe-prod",
                "color": "#3b82f6",
                "verification_config": {"provider": "stripe", "secret": "whsec_live"}
            })
        );
        assert_eq!(request.follow_up, None);
    }

    #[test]
    fn description_and_response_go_into_a_follow_up_patch() {
        let mut form = create_form();
        set_text(&mut form, "name", "hooks");
        set_text(&mut form, "description", "From the shop");
        set_json(
            &mut form,
            "response",
            json!({"status": 202, "content_type": "text", "body": "ok"}),
        );
        let request = create_request(&form).unwrap();
        assert_eq!(request.body, json!({"name": "hooks"}));
        assert_eq!(
            request.follow_up,
            Some(json!({
                "description": "From the shop",
                "response_config": {"status": 202, "content_type": "text", "body": "ok"}
            }))
        );
    }

    #[test]
    fn create_validation_reports_every_field() {
        let mut form = create_form();
        set_text(&mut form, "color", "blue");
        select(&mut form, "provider", "github");
        set_json(&mut form, "response", json!([1]));
        let errors = create_request(&form).unwrap_err();
        let keys: Vec<&str> = errors.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, vec!["name", "color", "secret", "response"]);
    }

    #[test]
    fn json_providers_take_their_settings_from_the_json_field() {
        let mut form = create_form();
        set_text(&mut form, "name", "custom");
        select(&mut form, "provider", "api_key");
        set_json(
            &mut form,
            "verification",
            json!({"header": "X-Key", "key": "k"}),
        );
        assert_eq!(
            create_request(&form).unwrap().body["verification_config"],
            json!({"header": "X-Key", "key": "k", "provider": "api_key"})
        );
        set_json(
            &mut form,
            "verification",
            json!({"header": "X-Key", "key": "***"}),
        );
        assert_eq!(
            create_request(&form).unwrap_err(),
            vec![(
                "verification",
                "Replace *** with the real secret".to_string()
            )]
        );
    }

    #[test]
    fn an_untouched_edit_changes_nothing() {
        let source = fixtures::source(fixtures::STRIPE_ID, "stripe-prod", 1204, "stripe", "active");
        let form = edit_form(&source);
        assert_eq!(edit_body(&form, &source).unwrap(), None);
    }

    #[test]
    fn an_empty_secret_on_edit_keeps_the_current_one() {
        let source = fixtures::source(fixtures::STRIPE_ID, "stripe-prod", 1204, "stripe", "active");
        let mut form = edit_form(&source);
        set_text(&mut form, "name", "stripe-live");
        assert_eq!(
            edit_body(&form, &source).unwrap(),
            Some(json!({"name": "stripe-live"}))
        );
        set_text(&mut form, "secret", "whsec_new");
        assert_eq!(
            edit_body(&form, &source).unwrap().unwrap()["verification_config"],
            json!({"provider": "stripe", "secret": "whsec_new"})
        );
    }

    #[test]
    fn changing_the_provider_needs_a_new_secret() {
        let source = fixtures::source(fixtures::STRIPE_ID, "stripe-prod", 1204, "stripe", "active");
        let mut form = edit_form(&source);
        select(&mut form, "provider", "github");
        assert_eq!(
            edit_body(&form, &source).unwrap_err(),
            vec![("secret", "Required for this provider".to_string())]
        );
        select(&mut form, "provider", "none");
        assert_eq!(
            edit_body(&form, &source).unwrap(),
            Some(json!({"verification_config": {"provider": "none"}}))
        );
    }

    #[test]
    fn status_and_response_changes_on_edit() {
        let source = fixtures::source(fixtures::STRIPE_ID, "stripe-prod", 1204, "stripe", "active");
        let mut form = edit_form(&source);
        select(&mut form, "status", "paused");
        set_json(
            &mut form,
            "response",
            json!({"status": 200, "content_type": "json", "body": "{}"}),
        );
        assert_eq!(
            edit_body(&form, &source).unwrap(),
            Some(json!({
                "status": "paused",
                "response_config": {"status": 200, "content_type": "json", "body": "{}"}
            }))
        );
    }

    #[test]
    fn clearing_the_response_sends_null() {
        let mut source =
            fixtures::source(fixtures::STRIPE_ID, "stripe-prod", 1204, "stripe", "active");
        source.response_config = json!({"status": 200, "content_type": "json", "body": "{}"});
        let mut form = edit_form(&source);
        set_json(&mut form, "response", Value::Null);
        assert_eq!(
            edit_body(&form, &source).unwrap(),
            Some(json!({"response_config": null}))
        );
    }

    #[test]
    fn an_unknown_status_has_no_status_field() {
        let source = fixtures::source(fixtures::STRIPE_ID, "stripe-prod", 1, "stripe", "archived");
        let form = edit_form(&source);
        assert!(form.fields.iter().all(|field| field.key != "status"));
        assert_eq!(field_error(&form, "name"), None);
    }
}
