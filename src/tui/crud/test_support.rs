//! Test helpers that set field values the way a user would.
#![allow(dead_code)]

use serde_json::Value;

use crate::tui::forms::form::{Field, FieldKind, Form};

fn field_mut<'a>(form: &'a mut Form, key: &str) -> &'a mut Field {
    form.fields
        .iter_mut()
        .find(|field| field.key == key)
        .unwrap_or_else(|| panic!("no field {key}"))
}

pub fn set_text(form: &mut Form, key: &str, value: &str) {
    match &mut field_mut(form, key).kind {
        FieldKind::Text(input) | FieldKind::Secret(input) => input.set(value),
        other => panic!("{key} is not a text field: {other:?}"),
    }
}

pub fn select(form: &mut Form, key: &str, value: &str) {
    match &mut field_mut(form, key).kind {
        FieldKind::Select { options, selected } => {
            *selected = options
                .iter()
                .position(|option| option.value == value)
                .unwrap_or_else(|| panic!("{key} has no option {value}"));
        }
        other => panic!("{key} is not a select: {other:?}"),
    }
}

pub fn set_toggle(form: &mut Form, key: &str, on: bool) {
    match &mut field_mut(form, key).kind {
        FieldKind::Toggle(current) => *current = on,
        other => panic!("{key} is not a toggle: {other:?}"),
    }
}

pub fn set_json(form: &mut Form, key: &str, value: Value) {
    match &mut field_mut(form, key).kind {
        FieldKind::Json {
            text,
            value: current,
            error,
        } => {
            *text = crate::tui::editor::text_for(&value);
            *current = value;
            *error = None;
        }
        other => panic!("{key} is not a JSON field: {other:?}"),
    }
}

pub fn replace_field(form: &mut Form, field: Field) {
    let key = field.key;
    *field_mut(form, key) = field;
}

pub fn field_error<'a>(form: &'a Form, key: &str) -> Option<&'a str> {
    form.fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| field.error.as_deref())
}
