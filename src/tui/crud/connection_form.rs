//! Connection create/edit forms. Source and destination are chosen on create
//! only; the API cannot move a connection.

use serde_json::{json, Map, Value};

use super::validation::{finish, json_field, object_or_null, selected, FieldError};
use crate::tui::forms::form::{Field, Form, SelectOption};
use crate::tui::model::{Connection, Destination, Source};
use crate::tui::names::NameCache;

pub use super::source_form::CreateRequest;

pub fn create_form(
    sources: &[Source],
    destinations: &[Destination],
    preselected_source: Option<&str>,
) -> Form {
    let source_options: Vec<SelectOption> = sources
        .iter()
        .map(|source| SelectOption::new(source.id.as_str(), source.name.as_str()))
        .collect();
    let destination_options: Vec<SelectOption> = destinations
        .iter()
        .map(|destination| SelectOption::new(destination.id.as_str(), destination.name.as_str()))
        .collect();
    Form::new(
        "New connection",
        vec![
            Field::select("source", "Source", source_options, preselected_source),
            Field::select("destination", "Destination", destination_options, None),
            Field::toggle("enabled", "Enabled", true),
            Field::json("filter", "Filter rules", Value::Null),
            Field::json("transformation", "Transformation", Value::Null),
        ],
    )
}

pub fn edit_form(connection: &Connection, names: &NameCache) -> Form {
    Form::new(
        format!(
            "Edit {} → {}",
            names.source(&connection.source_id),
            names.destination(&connection.destination_id)
        ),
        vec![
            Field::toggle("enabled", "Enabled", connection.enabled),
            Field::json("filter", "Filter rules", connection.filter_rules.clone()),
            Field::json(
                "transformation",
                "Transformation",
                connection.transformation.clone(),
            ),
        ],
    )
}

pub fn create_request(form: &Form) -> Result<CreateRequest, Vec<FieldError>> {
    let mut errors = Vec::new();
    let filter = object_or_null(&json_field(form, "filter"), "filter", &mut errors);
    let transformation = object_or_null(
        &json_field(form, "transformation"),
        "transformation",
        &mut errors,
    );
    let mut body = json!({
        "source_id": selected(form, "source", ""),
        "destination_id": selected(form, "destination", ""),
    });
    if let Some(filter) = filter.filter(|value| !value.is_null()) {
        body["filter_rules"] = filter;
    }
    if let Some(transformation) = transformation.filter(|value| !value.is_null()) {
        body["transformation"] = transformation;
    }
    let follow_up = (!form.bool("enabled")).then(|| json!({"enabled": false}));
    finish(errors, CreateRequest { body, follow_up })
}

pub fn edit_body(form: &Form, connection: &Connection) -> Result<Option<Value>, Vec<FieldError>> {
    let mut errors = Vec::new();
    let mut body = Map::new();
    let enabled = form.bool("enabled");
    if enabled != connection.enabled {
        body.insert("enabled".into(), json!(enabled));
    }
    for (key, api_key, current) in [
        ("filter", "filter_rules", &connection.filter_rules),
        (
            "transformation",
            "transformation",
            &connection.transformation,
        ),
    ] {
        if let Some(value) = object_or_null(&json_field(form, key), key, &mut errors) {
            if &value != current {
                body.insert(api_key.into(), value);
            }
        }
    }
    finish(errors, (!body.is_empty()).then_some(Value::Object(body)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::crud::test_support::{select, set_json, set_toggle};
    use crate::tui::fixtures;
    use serde_json::json;

    fn lists() -> (Vec<Source>, Vec<Destination>) {
        let app = fixtures::app();
        (
            app.data.sources.value.clone().unwrap(),
            app.data.destinations.value.clone().unwrap(),
        )
    }

    #[test]
    fn create_preselects_the_source_and_sends_the_pair() {
        let (sources, destinations) = lists();
        let mut form = create_form(&sources, &destinations, Some(fixtures::GITHUB_ID));
        select(&mut form, "destination", fixtures::AUDIT_ID);
        let request = create_request(&form).unwrap();
        assert_eq!(
            request.body,
            json!({"source_id": fixtures::GITHUB_ID, "destination_id": fixtures::AUDIT_ID})
        );
        assert_eq!(request.follow_up, None);
    }

    #[test]
    fn a_disabled_new_connection_is_disabled_by_a_follow_up() {
        let (sources, destinations) = lists();
        let mut form = create_form(&sources, &destinations, None);
        set_toggle(&mut form, "enabled", false);
        set_json(&mut form, "filter", json!({"all": []}));
        let request = create_request(&form).unwrap();
        assert_eq!(request.body["filter_rules"], json!({"all": []}));
        assert_eq!(request.follow_up, Some(json!({"enabled": false})));
    }

    #[test]
    fn filters_and_transformations_must_be_objects() {
        let (sources, destinations) = lists();
        let mut form = create_form(&sources, &destinations, None);
        set_json(&mut form, "transformation", json!("upper"));
        assert_eq!(
            create_request(&form).unwrap_err(),
            vec![(
                "transformation",
                "Must be a JSON object, or empty".to_string()
            )]
        );
    }

    #[test]
    fn edit_sends_changes_and_null_clears() {
        let connection = fixtures::connection(
            fixtures::STRIPE_BILLING_ID,
            fixtures::STRIPE_ID,
            fixtures::BILLING_ID,
            true,
        );
        let app = fixtures::app();
        let mut form = edit_form(&connection, &app.names);
        assert_eq!(edit_body(&form, &connection).unwrap(), None);
        set_toggle(&mut form, "enabled", false);
        set_json(&mut form, "filter", Value::Null);
        assert_eq!(
            edit_body(&form, &connection).unwrap(),
            Some(json!({"enabled": false, "filter_rules": null}))
        );
        assert_eq!(form.title, "Edit stripe-prod → billing-worker");
    }
}
