//! Create, edit, pause/resume, enable/disable, rotate and delete. Plugged into
//! the shell through one-line hooks; nothing here sends a request without a
//! form save or a confirmed modal.

pub mod connection_form;
pub mod destination_form;
pub mod source_form;
#[cfg(test)]
pub(crate) mod test_support;
pub mod validation;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::tui::action::Effect;
use crate::tui::action::{Mutation, Request};
use crate::tui::app::App;
use crate::tui::app::{FormPurpose, ModalForm};
use crate::tui::budget::Priority;
use crate::tui::editor::EditOutcome;
use crate::tui::forms::form::FieldKind;
use crate::tui::forms::form::Form;
use crate::tui::model::{Connection, Destination, Source};
use crate::tui::screen::{Screen, SourceTab};
use crate::tui::theme::Tone;
use validation::FieldError;

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

/// Screens where the CRUD keys act. Source tabs other than Overview and
/// Connections belong to Plans 3-4.
fn on_crud_screen(app: &App) -> bool {
    matches!(
        app.screen,
        Screen::Sources
            | Screen::SourceDetail {
                tab: SourceTab::Overview | SourceTab::Connections,
                ..
            }
            | Screen::Destinations
            | Screen::DestinationDetail { .. }
            | Screen::Connections
            | Screen::ConnectionDetail { .. }
    )
}

/// What the lowercase and uppercase CRUD keys act on.
enum Target {
    Source(Source),
    Destination(Destination),
    /// A connection; `full` is `None` on the source's Connections tab, which
    /// only has the embedded summary.
    #[allow(dead_code)]
    Connection {
        id: String,
        enabled: bool,
        full: Option<Connection>,
    },
}

fn target(app: &App) -> Option<Target> {
    let data = &app.data;
    match &app.screen {
        Screen::Sources => data
            .sources
            .value
            .as_ref()?
            .get(app.cursors.sources)
            .cloned()
            .map(Target::Source),
        Screen::SourceDetail {
            tab: SourceTab::Connections,
            ..
        } => data
            .source_connections
            .value
            .as_ref()?
            .get(app.cursors.source_connections)
            .map(|connection| Target::Connection {
                id: connection.id.clone(),
                enabled: connection.enabled,
                full: None,
            }),
        Screen::SourceDetail { .. } => data.source.value.clone().map(Target::Source),
        Screen::Destinations => data
            .destinations
            .value
            .as_ref()?
            .get(app.cursors.destinations)
            .cloned()
            .map(Target::Destination),
        Screen::DestinationDetail { .. } => data.destination.value.clone().map(Target::Destination),
        Screen::Connections => data
            .connections
            .value
            .as_ref()?
            .get(app.cursors.connections)
            .map(|connection| Target::Connection {
                id: connection.id.clone(),
                enabled: connection.enabled,
                full: Some(connection.clone()),
            }),
        Screen::ConnectionDetail { .. } => {
            data.connection
                .value
                .clone()
                .map(|connection| Target::Connection {
                    id: connection.id.clone(),
                    enabled: connection.enabled,
                    full: Some(connection),
                })
        }
        _ => None,
    }
}

fn open_modal(app: &mut App, form: Form, purpose: FormPurpose) -> Vec<Effect> {
    app.modal = Some(ModalForm { form, purpose });
    Vec::new()
}

fn open_create(app: &mut App) -> Vec<Effect> {
    match &app.screen {
        Screen::Sources => open_modal(app, source_form::create_form(), FormPurpose::CreateSource),
        Screen::Destinations => open_modal(
            app,
            destination_form::create_form(),
            FormPurpose::CreateDestination,
        ),
        Screen::Connections => open_connection_create(app, None),
        Screen::SourceDetail {
            id,
            tab: SourceTab::Connections,
        } => {
            let id = id.clone();
            open_connection_create(app, Some(id))
        }
        _ => Vec::new(),
    }
}

/// Both lists feed the selects. When one is missing it is fetched and the
/// user presses `n` again, rather than seeing an empty select.
fn open_connection_create(app: &mut App, source_id: Option<String>) -> Vec<Effect> {
    let (Some(sources), Some(destinations)) =
        (&app.data.sources.value, &app.data.destinations.value)
    else {
        let generation = app.generation;
        let mut effects = Vec::new();
        if app.data.sources.value.is_none() {
            effects.push(Effect::Fetch {
                request: Request::Sources { search: None },
                generation,
                priority: Priority::User,
            });
        }
        if app.data.destinations.value.is_none() {
            effects.push(Effect::Fetch {
                request: Request::Destinations,
                generation,
                priority: Priority::User,
            });
        }
        app.toast(
            "Loading sources and destinations; press n again",
            Tone::Muted,
        );
        return effects;
    };
    if sources.is_empty() || destinations.is_empty() {
        app.toast("Create a source and a destination first", Tone::Warning);
        return Vec::new();
    }
    let form = connection_form::create_form(sources, destinations, source_id.as_deref());
    open_modal(app, form, FormPurpose::CreateConnection)
}

fn open_edit(app: &mut App) -> Vec<Effect> {
    match target(app) {
        Some(Target::Source(source)) => {
            let purpose = FormPurpose::EditSource {
                id: source.id.clone(),
            };
            open_modal(app, source_form::edit_form(&source), purpose)
        }
        Some(Target::Destination(destination)) => {
            let purpose = FormPurpose::EditDestination {
                id: destination.id.clone(),
            };
            open_modal(app, destination_form::edit_form(&destination), purpose)
        }
        Some(Target::Connection {
            full: Some(connection),
            ..
        }) => {
            let purpose = FormPurpose::EditConnection {
                id: connection.id.clone(),
            };
            let form = connection_form::edit_form(&connection, &app.names);
            open_modal(app, form, purpose)
        }
        _ => Vec::new(),
    }
}

/// Called by `keys::submit_modal` for the CRUD purposes.
pub fn submit(app: &mut App) -> Vec<Effect> {
    let Some(modal) = app.modal.as_ref() else {
        return Vec::new();
    };
    let form = &modal.form;
    let built: Result<Option<Mutation>, Vec<FieldError>> = match &modal.purpose {
        FormPurpose::CreateSource => source_form::create_request(form).map(|request| {
            Some(Mutation::CreateSource {
                body: request.body,
                follow_up: request.follow_up,
            })
        }),
        FormPurpose::EditSource { id } => find_source(app, id).map_or(Ok(None), |source| {
            source_form::edit_body(form, &source).map(|body| {
                body.map(|body| Mutation::UpdateSource {
                    id: id.clone(),
                    body,
                })
            })
        }),
        FormPurpose::CreateDestination => destination_form::create_body(form)
            .map(|body| Some(Mutation::CreateDestination { body })),
        FormPurpose::EditDestination { id } => {
            find_destination(app, id).map_or(Ok(None), |destination| {
                destination_form::edit_body(form, &destination).map(|body| {
                    body.map(|body| Mutation::UpdateDestination {
                        id: id.clone(),
                        body,
                    })
                })
            })
        }
        FormPurpose::CreateConnection => connection_form::create_request(form).map(|request| {
            Some(Mutation::CreateConnection {
                body: request.body,
                follow_up: request.follow_up,
            })
        }),
        FormPurpose::EditConnection { id } => {
            find_connection(app, id).map_or(Ok(None), |connection| {
                connection_form::edit_body(form, &connection).map(|body| {
                    body.map(|body| Mutation::UpdateConnection {
                        id: id.clone(),
                        body,
                    })
                })
            })
        }
        _ => return Vec::new(),
    };
    let Some(modal) = app.modal.as_mut() else {
        return Vec::new();
    };
    modal.form.clear_errors();
    match built {
        Ok(Some(mutation)) => {
            modal.form.submitting = true;
            vec![Effect::Mutate { mutation }]
        }
        Ok(None) => {
            app.modal = None;
            app.toast("No changes to save", Tone::Muted);
            Vec::new()
        }
        Err(errors) => {
            modal.form.submitting = false;
            for (key, message) in errors {
                modal.form.set_field_error(key, message);
            }
            Vec::new()
        }
    }
}

fn find_source(app: &App, id: &str) -> Option<Source> {
    app.data
        .source
        .value
        .iter()
        .chain(app.data.sources.value.iter().flatten())
        .find(|source| source.id == id)
        .cloned()
}

fn find_destination(app: &App, id: &str) -> Option<Destination> {
    app.data
        .destination
        .value
        .iter()
        .chain(app.data.destinations.value.iter().flatten())
        .find(|destination| destination.id == id)
        .cloned()
}

fn find_connection(app: &App, id: &str) -> Option<Connection> {
    app.data
        .connection
        .value
        .iter()
        .chain(app.data.connections.value.iter().flatten())
        .find(|connection| connection.id == id)
        .cloned()
}

/// `n e y P E T D` on the CRUD screens. `None` leaves the key to the screen.
/// Nothing here returns `Effect::Mutate`: uppercase keys only open a confirm.
pub fn handle_key(app: &mut App, code: KeyCode) -> Option<Vec<Effect>> {
    if !on_crud_screen(app) {
        return None;
    }
    let effects = match code {
        KeyCode::Char('n') => open_create(app),
        KeyCode::Char('e') => open_edit(app),
        _ => return None,
    };
    Some(effects)
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
            purpose: FormPurpose::CreateSource,
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

    use crate::tui::action::{Mutation, Request};
    use crate::tui::crud::test_support::{select, set_text};
    use crate::tui::screen::{Screen, SourceTab};

    fn press(app: &mut crate::tui::app::App, code: KeyCode) -> Vec<Effect> {
        update(app, Action::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn mutations(effects: &[Effect]) -> Vec<Mutation> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Mutate { mutation } => Some(mutation.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn n_opens_the_right_create_form_per_screen() {
        let mut app = fixtures::app();
        assert!(press(&mut app, KeyCode::Char('n')).is_empty());
        assert_eq!(
            app.modal.as_ref().unwrap().purpose,
            FormPurpose::CreateSource
        );

        let mut app = fixtures::app();
        app.screen = Screen::Destinations;
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(
            app.modal.as_ref().unwrap().purpose,
            FormPurpose::CreateDestination
        );

        let mut app = fixtures::source_detail(SourceTab::Connections);
        press(&mut app, KeyCode::Char('n'));
        let modal = app.modal.as_ref().unwrap();
        assert_eq!(modal.purpose, FormPurpose::CreateConnection);
        assert_eq!(
            modal.form.selected("source").map(|value| value.to_string()),
            Some(fixtures::STRIPE_ID.to_string())
        );
    }

    #[test]
    fn a_connection_form_needs_both_lists_loaded() {
        let mut app = fixtures::source_detail(SourceTab::Connections);
        app.data.destinations = Default::default();
        let effects = press(&mut app, KeyCode::Char('n'));
        assert!(app.modal.is_none());
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::Fetch {
                request: Request::Destinations,
                ..
            }
        )));
        assert!(app.toasts.last().unwrap().text.contains("press n again"));
    }

    #[test]
    fn e_opens_the_edit_form_for_the_selection() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(
            app.modal.as_ref().unwrap().purpose,
            FormPurpose::EditSource {
                id: fixtures::GITHUB_ID.into()
            }
        );
        let mut app = fixtures::connection_detail();
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(
            app.modal.as_ref().unwrap().purpose,
            FormPurpose::EditConnection {
                id: fixtures::STRIPE_BILLING_ID.into()
            }
        );
    }

    #[test]
    fn saving_a_valid_create_form_sends_one_mutation() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('n'));
        let form = &mut app.modal.as_mut().unwrap().form;
        set_text(form, "name", "hooks");
        let effects = submit(&mut app);
        assert_eq!(
            mutations(&effects),
            vec![Mutation::CreateSource {
                body: json!({"name": "hooks"}),
                follow_up: None
            }]
        );
        assert!(app.modal.as_ref().unwrap().form.submitting);
    }

    #[test]
    fn an_invalid_form_stays_open_with_field_errors() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('n'));
        select(&mut app.modal.as_mut().unwrap().form, "provider", "stripe");
        assert!(submit(&mut app).is_empty());
        let form = &app.modal.as_ref().unwrap().form;
        assert!(!form.submitting);
        assert_eq!(
            crate::tui::crud::test_support::field_error(form, "name"),
            Some("Required")
        );
        assert_eq!(
            crate::tui::crud::test_support::field_error(form, "secret"),
            Some("Required for this provider")
        );
    }

    #[test]
    fn saving_an_unchanged_edit_sends_nothing() {
        let mut app = fixtures::destination_detail();
        press(&mut app, KeyCode::Char('e'));
        assert!(submit(&mut app).is_empty());
        assert!(app.modal.is_none());
        assert_eq!(app.toasts.last().unwrap().text, "No changes to save");
    }
}
