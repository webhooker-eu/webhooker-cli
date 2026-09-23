//! Create, edit, pause/resume, enable/disable, rotate and delete. Plugged into
//! the shell through one-line hooks; nothing here sends a request without a
//! form save or a confirmed modal.

pub mod destination_form;
pub mod source_form;
#[cfg(test)]
pub(crate) mod test_support;
pub mod validation;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::tui::action::Effect;
use crate::tui::app::App;
use crate::tui::editor::EditOutcome;
use crate::tui::forms::form::FieldKind;

/// `Enter` on a focused JSON field of the open modal opens `$EDITOR` with the
/// field's last text (which may be the invalid text of a failed edit).
pub fn open_json_editor(app: &App, key: &KeyEvent) -> Option<Vec<Effect>> {
    if key.code != KeyCode::Enter || !key.modifiers.is_empty() || app.confirm.is_some() {
        return None;
    }
    let form = &app.modal.as_ref()?.form;
    if form.submitting {
        return None;
    }
    let field = form.fields.get(form.focused)?;
    let FieldKind::Json { text, .. } = &field.kind else {
        return None;
    };
    Some(vec![Effect::EditJson {
        field_key: field.key,
        text: text.clone(),
    }])
}

pub fn on_json_edited(
    app: &mut App,
    field_key: &'static str,
    result: Result<EditOutcome, String>,
) -> Vec<Effect> {
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    let form = &mut modal.form;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(message) => {
            form.error = Some(message);
            return Vec::new();
        }
    };
    let Some(field) = form.fields.iter_mut().find(|field| field.key == field_key) else {
        return Vec::new();
    };
    let FieldKind::Json { text, value, error } = &mut field.kind else {
        return Vec::new();
    };
    match outcome {
        EditOutcome::Parsed(parsed) => {
            *text = crate::tui::editor::text_for(&parsed);
            *value = parsed;
            *error = None;
            field.error = None;
        }
        EditOutcome::Invalid {
            text: edited,
            message,
        } => {
            *text = edited;
            *error = Some(message.clone());
            // `Field.error` is what the form view draws under the field.
            field.error = Some(message);
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::action::Action;
    use crate::tui::app::{update, FormPurpose, ModalForm};
    use crate::tui::editor::EditOutcome;
    use crate::tui::fixtures;
    use crate::tui::forms::form::{Field, FieldKind, Form};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use serde_json::{json, Value};

    pub(crate) fn with_modal(fields: Vec<Field>, focused: usize) -> crate::tui::app::App {
        let mut app = fixtures::app();
        let mut form = Form::new("Test", fields);
        form.focused = focused;
        app.modal = Some(ModalForm {
            form,
            purpose: FormPurpose::EventFilters,
        });
        app
    }

    fn enter(app: &mut crate::tui::app::App) -> Vec<Effect> {
        update(
            app,
            Action::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        )
    }

    #[test]
    fn enter_on_a_json_field_asks_the_loop_to_open_the_editor() {
        let mut app = with_modal(
            vec![
                Field::text("name", "Name", ""),
                Field::json("response", "Response config", json!({"status": 200})),
            ],
            1,
        );
        assert_eq!(
            enter(&mut app),
            vec![Effect::EditJson {
                field_key: "response",
                text: "{\n  \"status\": 200\n}".into()
            }]
        );
    }

    #[test]
    fn enter_elsewhere_is_left_to_the_form() {
        let mut app = with_modal(vec![Field::text("name", "Name", "")], 0);
        assert!(!enter(&mut app)
            .iter()
            .any(|effect| matches!(effect, Effect::EditJson { .. })));
    }

    #[test]
    fn a_parsed_edit_updates_the_field() {
        let mut app = with_modal(vec![Field::json("response", "Response", Value::Null)], 0);
        update(
            &mut app,
            Action::JsonEdited {
                field_key: "response",
                result: Ok(EditOutcome::Parsed(json!({"status": 202}))),
            },
        );
        let form = &app.modal.as_ref().unwrap().form;
        assert_eq!(form.json("response"), Some(&json!({"status": 202})));
        assert_eq!(form.fields[0].error, None);
    }

    #[test]
    fn an_invalid_edit_keeps_the_text_to_reopen_and_shows_the_position() {
        let mut app = with_modal(
            vec![Field::json("response", "Response", json!({"a": 1}))],
            0,
        );
        update(
            &mut app,
            Action::JsonEdited {
                field_key: "response",
                result: Ok(EditOutcome::Invalid {
                    text: "{oops".into(),
                    message: "Invalid JSON at line 1, column 2 (enter to fix)".into(),
                }),
            },
        );
        let form = &app.modal.as_ref().unwrap().form;
        assert_eq!(
            form.json("response"),
            Some(&json!({"a": 1})),
            "last valid value kept"
        );
        assert_eq!(
            form.fields[0].error.as_deref(),
            Some("Invalid JSON at line 1, column 2 (enter to fix)")
        );
        assert_eq!(
            enter(&mut app),
            vec![Effect::EditJson {
                field_key: "response",
                text: "{oops".into()
            }],
            "enter reopens the same text"
        );
    }

    #[test]
    fn an_editor_that_fails_to_start_is_a_form_error() {
        let mut app = with_modal(vec![Field::json("response", "Response", Value::Null)], 0);
        update(
            &mut app,
            Action::JsonEdited {
                field_key: "response",
                result: Err("could not start vi: not found".into()),
            },
        );
        assert_eq!(
            app.modal.as_ref().unwrap().form.error.as_deref(),
            Some("could not start vi: not found")
        );
        assert!(matches!(
            app.modal.as_ref().unwrap().form.fields[0].kind,
            FieldKind::Json { .. }
        ));
    }
}
