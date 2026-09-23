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
use serde_json::Value;

use crate::tui::action::{Effect, FetchError, Mutation, Request};
use crate::tui::app::{App, Confirm, ConfirmAction, FormPurpose, ModalForm, TypedName};
use crate::tui::budget::Priority;
use crate::tui::editor::EditOutcome;
use crate::tui::forms::form::{FieldKind, Form};
use crate::tui::model::{Connection, Destination, Source};
use crate::tui::screen::{Screen, SourceTab};
use crate::tui::status;
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
        KeyCode::Char('P') => ask_toggle_status(app),
        KeyCode::Char('E') => ask_toggle_connection(app),
        KeyCode::Char('T') => ask_rotate(app),
        KeyCode::Char('D') => ask_delete(app),
        _ => return None,
    };
    Some(effects)
}

fn ask_toggle_status(app: &mut App) -> Vec<Effect> {
    let (id, name, current, is_source) = match target(app) {
        Some(Target::Source(source)) => (source.id, source.name, source.status, true),
        Some(Target::Destination(destination)) => {
            (destination.id, destination.name, destination.status, false)
        }
        _ => return Vec::new(),
    };
    // An unknown status offers no action.
    let Some(next) = status::toggled_status(&current) else {
        return Vec::new();
    };
    let verb = if next == "paused" {
        "Pause "
    } else {
        "Resume "
    };
    let action = if is_source {
        ConfirmAction::SetSourceStatus {
            id,
            status: next.to_string(),
        }
    } else {
        ConfirmAction::SetDestinationStatus {
            id,
            status: next.to_string(),
        }
    };
    app.confirm = Some(Confirm::about(verb, name, "?", action));
    Vec::new()
}

fn ask_toggle_connection(app: &mut App) -> Vec<Effect> {
    let Some(Target::Connection { id, enabled, .. }) = target(app) else {
        return Vec::new();
    };
    let verb = if enabled {
        "Disable the connection "
    } else {
        "Enable the connection "
    };
    let label = connection_label(app, &id);
    app.confirm = Some(Confirm::about(
        verb,
        label,
        "?",
        ConfirmAction::SetConnectionEnabled {
            id,
            enabled: !enabled,
        },
    ));
    Vec::new()
}

fn ask_rotate(app: &mut App) -> Vec<Effect> {
    let Some(Target::Source(source)) = target(app) else {
        return Vec::new();
    };
    app.confirm = Some(Confirm::about(
        "Rotate the ingest token of ",
        source.name,
        "? The old URL stops working within ~30 s.",
        ConfirmAction::RotateSourceToken { id: source.id },
    ));
    Vec::new()
}

fn ask_delete(app: &mut App) -> Vec<Effect> {
    let confirm = match target(app) {
        Some(Target::Source(source)) => Confirm {
            typed_name: Some(TypedName::new(source.name.clone())),
            ..Confirm::about(
                "Move ",
                source.name,
                " to the trash?",
                ConfirmAction::DeleteSource { id: source.id },
            )
        },
        Some(Target::Destination(destination)) => Confirm {
            typed_name: Some(TypedName::new(destination.name.clone())),
            ..Confirm::about(
                "Delete ",
                destination.name,
                "?",
                ConfirmAction::DeleteDestination { id: destination.id },
            )
        },
        Some(Target::Connection { id, .. }) => {
            let label = connection_label(app, &id);
            Confirm::about(
                "Delete the connection ",
                label,
                "?",
                ConfirmAction::DeleteConnection { id },
            )
        }
        None => return Vec::new(),
    };
    app.confirm = Some(confirm);
    Vec::new()
}

fn connection_label(app: &App, id: &str) -> String {
    let ends = app
        .data
        .connections
        .value
        .iter()
        .flatten()
        .chain(app.data.connection.value.iter())
        .find(|connection| connection.id == id)
        .map(|connection| {
            (
                connection.source_id.clone(),
                connection.destination_id.clone(),
            )
        })
        .or_else(|| {
            app.data
                .source_connections
                .value
                .iter()
                .flatten()
                .find(|connection| connection.id == id)
                .map(|connection| {
                    (
                        connection.source_id.clone(),
                        connection.destination_id.clone(),
                    )
                })
        });
    match ends {
        Some((source_id, destination_id)) => format!(
            "{} {} {}",
            app.names.source(&source_id),
            app.theme.glyphs.arrow,
            app.names.destination(&destination_id)
        ),
        None => crate::tui::names::short_id(id),
    }
}

/// Runs a confirmed CRUD action; called from `keys::confirmed`.
pub fn confirmed(_app: &mut App, action: ConfirmAction) -> Vec<Effect> {
    let mutation = match action {
        ConfirmAction::SetSourceStatus { id, status } => Mutation::UpdateSource {
            id,
            body: serde_json::json!({"status": status}),
        },
        ConfirmAction::SetDestinationStatus { id, status } => Mutation::UpdateDestination {
            id,
            body: serde_json::json!({"status": status}),
        },
        ConfirmAction::SetConnectionEnabled { id, enabled } => Mutation::UpdateConnection {
            id,
            body: serde_json::json!({"enabled": enabled}),
        },
        ConfirmAction::RotateSourceToken { id } => Mutation::RotateSourceToken { id },
        ConfirmAction::DeleteSource { id } => Mutation::DeleteSource { id },
        ConfirmAction::DeleteDestination { id } => Mutation::DeleteDestination { id },
        ConfirmAction::DeleteConnection { id } => Mutation::DeleteConnection { id },
        _ => return Vec::new(),
    };
    vec![Effect::Mutate { mutation }]
}

/// Keys while a typed-name confirm is open. See the rules at the top of
/// this task.
pub fn on_typed_confirm_key(app: &mut App, key: KeyEvent) -> Vec<Effect> {
    let Some(mut confirm) = app.confirm.take() else {
        return Vec::new();
    };
    let Some(typed) = confirm.typed_name.as_mut() else {
        app.confirm = Some(confirm);
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => Vec::new(),
        KeyCode::Enter if typed.matches() => confirmed(app, confirm.action),
        KeyCode::Enter => {
            typed.mismatch = true;
            app.confirm = Some(confirm);
            Vec::new()
        }
        _ => {
            if typed.input.handle(key) {
                typed.mismatch = false;
            }
            app.confirm = Some(confirm);
            Vec::new()
        }
    }
}

/// Which modal form belongs to a mutation, for inline errors.
fn form_is_for(purpose: &FormPurpose, mutation: &Mutation) -> bool {
    match (purpose, mutation) {
        (FormPurpose::CreateSource, Mutation::CreateSource { .. })
        | (FormPurpose::CreateDestination, Mutation::CreateDestination { .. })
        | (FormPurpose::CreateConnection, Mutation::CreateConnection { .. }) => true,
        (FormPurpose::EditSource { id }, Mutation::UpdateSource { id: target, .. })
        | (FormPurpose::EditDestination { id }, Mutation::UpdateDestination { id: target, .. })
        | (FormPurpose::EditConnection { id }, Mutation::UpdateConnection { id: target, .. }) => {
            id == target
        }
        _ => false,
    }
}

fn is_crud(mutation: &Mutation) -> bool {
    matches!(
        mutation,
        Mutation::CreateSource { .. }
            | Mutation::UpdateSource { .. }
            | Mutation::DeleteSource { .. }
            | Mutation::RotateSourceToken { .. }
            | Mutation::CreateDestination { .. }
            | Mutation::UpdateDestination { .. }
            | Mutation::DeleteDestination { .. }
            | Mutation::CreateConnection { .. }
            | Mutation::UpdateConnection { .. }
            | Mutation::DeleteConnection { .. }
    )
}

fn close_modal_for(app: &mut App, mutation: &Mutation) {
    if app
        .modal
        .as_ref()
        .is_some_and(|modal| form_is_for(&modal.purpose, mutation))
    {
        app.modal = None;
    }
}

/// Hooked at the top of `app::on_mutated`; `None` for mutations it does not own.
pub fn on_mutated(
    app: &mut App,
    mutation: &Mutation,
    result: &Result<Value, FetchError>,
) -> Option<Vec<Effect>> {
    if !is_crud(mutation) {
        return None;
    }
    // A rejected key and rate limits keep Plan 3's handling.
    if let Err(FetchError::Api(api)) = result {
        if matches!(api.status, 401 | 429) {
            return None;
        }
    }
    Some(match result {
        Ok(value) => on_success(app, mutation, value),
        Err(error) => on_failure(app, mutation, error),
    })
}

fn on_success(app: &mut App, mutation: &Mutation, value: &Value) -> Vec<Effect> {
    close_modal_for(app, mutation);
    let name = value["name"].as_str().unwrap_or("").to_string();
    let separator = app.theme.glyphs.separator;
    let mut follow_up = None;
    let leaving = match mutation {
        Mutation::CreateSource {
            follow_up: body, ..
        }
        | Mutation::CreateConnection {
            follow_up: body, ..
        } => {
            let id = value["id"].as_str().unwrap_or("").to_string();
            if let (Some(body), false) = (body, id.is_empty()) {
                follow_up = Some(match mutation {
                    Mutation::CreateSource { .. } => Mutation::UpdateSource {
                        id,
                        body: body.clone(),
                    },
                    _ => Mutation::UpdateConnection {
                        id,
                        body: body.clone(),
                    },
                });
            }
            let message = if matches!(mutation, Mutation::CreateConnection { .. }) {
                "Connected".to_string()
            } else {
                format!("Created {name}")
            };
            app.toast(message, Tone::Success);
            false
        }
        Mutation::CreateDestination { .. } => {
            app.toast(format!("Created {name}"), Tone::Success);
            false
        }
        Mutation::UpdateSource { body, .. } | Mutation::UpdateDestination { body, .. } => {
            let message = match body.get("status").and_then(Value::as_str) {
                Some("paused") if body.as_object().is_some_and(|map| map.len() == 1) => {
                    format!("Paused {name}")
                }
                Some("active") if body.as_object().is_some_and(|map| map.len() == 1) => {
                    format!("Resumed {name}")
                }
                _ => "Saved".to_string(),
            };
            app.toast(message, Tone::Success);
            false
        }
        Mutation::UpdateConnection { body, .. } => {
            let message = match (body.get("enabled"), body.as_object().map(|map| map.len())) {
                (Some(Value::Bool(true)), Some(1)) => "Connection enabled",
                (Some(Value::Bool(false)), Some(1)) => "Connection disabled",
                _ => "Saved",
            };
            app.toast(message, Tone::Success);
            false
        }
        Mutation::RotateSourceToken { .. } => {
            app.toast(
                format!("Ingest token rotated {separator} the old URL stops working within ~30 s"),
                Tone::Success,
            );
            false
        }
        Mutation::DeleteSource { id } => {
            app.toast(
                format!("Moved to trash {separator} restore with whk sources restore {id}"),
                Tone::Success,
            );
            matches!(&app.screen, Screen::SourceDetail { id: open, .. } if open == id)
        }
        Mutation::DeleteDestination { id } => {
            app.toast("Destination deleted", Tone::Success);
            matches!(&app.screen, Screen::DestinationDetail { id: open } if open == id)
        }
        Mutation::DeleteConnection { id } => {
            app.toast("Connection deleted", Tone::Success);
            matches!(&app.screen, Screen::ConnectionDetail { id: open } if open == id)
        }
        _ => false,
    };
    // `back` enters the previous screen, which fetches it anyway.
    let mut effects = if leaving {
        app.back()
    } else {
        app.refresh_now()
    };
    if let Some(mutation) = follow_up {
        effects.push(Effect::Mutate { mutation });
    }
    effects
}

/// 422, 409 and plan-limit 403 answers to a form go under the form, which
/// stays open. Everything else closes it with a red toast and refreshes.
fn on_failure(app: &mut App, mutation: &Mutation, error: &FetchError) -> Vec<Effect> {
    if let (Some(modal), FetchError::Api(api)) = (app.modal.as_mut(), error) {
        let inline = matches!(
            (api.status, api.code.as_deref()),
            (422 | 409, _) | (403, Some("plan_limit_exceeded"))
        );
        if inline && form_is_for(&modal.purpose, mutation) {
            modal.form.submitting = false;
            modal.form.error = Some(
                if api.status == 409 && matches!(mutation, Mutation::CreateConnection { .. }) {
                    "This source is already connected to that destination".to_string()
                } else {
                    error.message()
                },
            );
            return Vec::new();
        }
    }
    close_modal_for(app, mutation);
    app.toast(error.message(), Tone::Danger);
    app.refresh_now()
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

    fn chars(app: &mut crate::tui::app::App, text: &str) {
        for character in text.chars() {
            press(app, KeyCode::Char(character));
        }
    }

    #[test]
    fn p_asks_before_pausing_and_y_sends_the_patch() {
        let mut app = fixtures::app();
        assert!(press(&mut app, KeyCode::Char('P')).is_empty());
        assert_eq!(app.confirm.as_ref().unwrap().text(), "Pause stripe-prod?");
        let effects = press(&mut app, KeyCode::Char('y'));
        assert_eq!(
            mutations(&effects),
            vec![Mutation::UpdateSource {
                id: fixtures::STRIPE_ID.into(),
                body: json!({"status": "paused"})
            }]
        );
    }

    #[test]
    fn p_resumes_a_paused_source_and_ignores_unknown_statuses() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::End);
        press(&mut app, KeyCode::Char('P'));
        assert_eq!(app.confirm.as_ref().unwrap().text(), "Resume shopify-old?");
        let mut app = fixtures::app();
        app.data.sources.value.as_mut().unwrap()[0].status = "archived".into();
        press(&mut app, KeyCode::Char('P'));
        assert!(app.confirm.is_none());
    }

    #[test]
    fn e_toggles_a_connection_and_t_rotates_a_token() {
        let mut app = fixtures::source_detail(SourceTab::Connections);
        press(&mut app, KeyCode::Char('E'));
        assert_eq!(
            app.confirm.as_ref().unwrap().text(),
            "Disable the connection stripe-prod → billing-worker?"
        );
        assert_eq!(
            mutations(&press(&mut app, KeyCode::Char('y'))),
            vec![Mutation::UpdateConnection {
                id: fixtures::STRIPE_BILLING_ID.into(),
                body: json!({"enabled": false})
            }]
        );
        let mut app = fixtures::source_detail(SourceTab::Overview);
        press(&mut app, KeyCode::Char('T'));
        assert_eq!(
            app.confirm.as_ref().unwrap().text(),
            "Rotate the ingest token of stripe-prod? The old URL stops working within ~30 s."
        );
        assert_eq!(
            mutations(&press(&mut app, KeyCode::Char('y'))),
            vec![Mutation::RotateSourceToken {
                id: fixtures::STRIPE_ID.into()
            }]
        );
    }

    #[test]
    fn deleting_a_source_requires_typing_its_name() {
        let mut app = fixtures::app();
        press(&mut app, KeyCode::Char('D'));
        let confirm = app.confirm.as_ref().unwrap();
        assert_eq!(confirm.text(), "Move stripe-prod to the trash?");
        assert!(confirm.typed_name.is_some());
        assert!(
            press(&mut app, KeyCode::Char('y')).is_empty(),
            "y is input here"
        );
        assert!(
            press(&mut app, KeyCode::Enter).is_empty(),
            "\"y\" is not the name"
        );
        assert!(
            app.confirm
                .as_ref()
                .unwrap()
                .typed_name
                .as_ref()
                .unwrap()
                .mismatch
        );
        update(
            &mut app,
            Action::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        );
        chars(&mut app, "Stripe-prod");
        assert!(press(&mut app, KeyCode::Enter).is_empty(), "case matters");
        update(
            &mut app,
            Action::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        );
        chars(&mut app, "stripe-prod ");
        assert_eq!(
            mutations(&press(&mut app, KeyCode::Enter)),
            vec![Mutation::DeleteSource {
                id: fixtures::STRIPE_ID.into()
            }]
        );
        assert!(app.confirm.is_none());
    }

    #[test]
    fn esc_cancels_a_typed_confirm() {
        let mut app = fixtures::destination_detail();
        press(&mut app, KeyCode::Char('D'));
        assert_eq!(
            app.confirm.as_ref().unwrap().text(),
            "Delete billing-worker?"
        );
        press(&mut app, KeyCode::Esc);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn deleting_a_connection_is_a_plain_confirm() {
        let mut app = fixtures::connection_detail();
        press(&mut app, KeyCode::Char('D'));
        assert!(app.confirm.as_ref().unwrap().typed_name.is_none());
        assert!(press(&mut app, KeyCode::Enter).is_empty());
        assert!(app.confirm.is_none(), "enter cancels a plain confirm");
    }

    #[test]
    fn no_key_sends_a_mutation_without_a_confirmation_or_a_save() {
        let screens = [
            fixtures::app(),
            fixtures::source_detail(SourceTab::Overview),
            fixtures::source_detail(SourceTab::Connections),
            {
                let mut app = fixtures::app();
                app.screen = Screen::Destinations;
                app
            },
            fixtures::destination_detail(),
            {
                let mut app = fixtures::app();
                app.screen = Screen::Connections;
                app
            },
            fixtures::connection_detail(),
        ];
        for base in screens {
            for letter in ('A'..='Z').chain('a'..='z').chain(['/', '?', ' ']) {
                let mut app = clone_app(&base);
                let effects = press(&mut app, KeyCode::Char(letter));
                assert!(
                    mutations(&effects).is_empty(),
                    "{letter:?} on {:?} sent a mutation",
                    base.screen
                );
                for follow in [KeyCode::Char('n'), KeyCode::Esc, KeyCode::Enter] {
                    let mut probe = clone_app(&app);
                    assert!(mutations(&press(&mut probe, follow)).is_empty());
                }
            }
        }
    }

    /// Fixtures are cheap to rebuild; this re-creates the same screen state.
    fn clone_app(app: &crate::tui::app::App) -> crate::tui::app::App {
        let mut copy = match &app.screen {
            Screen::SourceDetail { tab, .. } => fixtures::source_detail(*tab),
            Screen::DestinationDetail { .. } => fixtures::destination_detail(),
            Screen::ConnectionDetail { .. } => fixtures::connection_detail(),
            _ => fixtures::app(),
        };
        copy.screen = app.screen.clone();
        copy.confirm = app.confirm.clone();
        copy.modal = app.modal.clone();
        copy.focus = app.focus;
        copy
    }

    use crate::client::ApiError;
    use crate::tui::action::FetchError;

    fn mutated(
        app: &mut crate::tui::app::App,
        mutation: Mutation,
        result: Result<Value, FetchError>,
    ) -> Vec<Effect> {
        update(app, Action::Mutated { mutation, result })
    }

    fn api(status: u16, code: &str, message: &str) -> FetchError {
        FetchError::Api(ApiError::synthetic(status, Some(code), message))
    }

    fn open_create_source(app: &mut crate::tui::app::App) -> Mutation {
        press(app, KeyCode::Char('n'));
        set_text(&mut app.modal.as_mut().unwrap().form, "name", "hooks");
        mutations(&submit(app)).remove(0)
    }

    #[test]
    fn a_created_source_closes_the_form_refreshes_and_patches_the_rest() {
        let mut app = fixtures::app();
        let mutation = Mutation::CreateSource {
            body: json!({"name": "hooks"}),
            follow_up: Some(json!({"description": "From the shop"})),
        };
        app.modal = Some(ModalForm {
            form: source_form::create_form(),
            purpose: FormPurpose::CreateSource,
        });
        let effects = mutated(
            &mut app,
            mutation,
            Ok(json!({"id": "new-id", "name": "hooks"})),
        );
        assert!(app.modal.is_none());
        assert_eq!(app.toasts.last().unwrap().text, "Created hooks");
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::Fetch { .. })));
        assert_eq!(
            mutations(&effects),
            vec![Mutation::UpdateSource {
                id: "new-id".into(),
                body: json!({"description": "From the shop"})
            }]
        );
    }

    #[test]
    fn validation_errors_stay_inline_and_keep_the_form() {
        let mut app = fixtures::app();
        let mutation = open_create_source(&mut app);
        mutated(
            &mut app,
            mutation,
            Err(api(422, "validation_error", "name is taken")),
        );
        let form = &app.modal.as_ref().unwrap().form;
        assert_eq!(form.error.as_deref(), Some("name is taken"));
        assert!(!form.submitting);
        assert!(app.toasts.is_empty());
    }

    #[test]
    fn a_duplicate_connection_explains_itself() {
        let mut app = fixtures::app();
        app.screen = Screen::Connections;
        press(&mut app, KeyCode::Char('n'));
        let mutation = mutations(&submit(&mut app)).remove(0);
        mutated(
            &mut app,
            mutation,
            Err(api(409, "conflict", "connection already exists")),
        );
        assert_eq!(
            app.modal.as_ref().unwrap().form.error.as_deref(),
            Some("This source is already connected to that destination")
        );
    }

    #[test]
    fn plan_limits_are_inline_but_forbidden_is_a_toast() {
        let mut app = fixtures::app();
        let mutation = open_create_source(&mut app);
        mutated(
            &mut app,
            mutation.clone(),
            Err(api(
                403,
                "plan_limit_exceeded",
                "source limit reached for your plan (3)",
            )),
        );
        assert_eq!(
            app.modal.as_ref().unwrap().form.error.as_deref(),
            Some("source limit reached for your plan (3)")
        );
        mutated(
            &mut app,
            mutation,
            Err(api(403, "forbidden", "not allowed")),
        );
        assert!(app.modal.is_none());
        assert_eq!(app.toasts.last().unwrap().text, "not allowed");
        assert_eq!(app.toasts.last().unwrap().tone, Tone::Danger);
    }

    #[test]
    fn a_failed_confirmed_action_toasts_and_refreshes() {
        let mut app = fixtures::app();
        let effects = mutated(
            &mut app,
            Mutation::UpdateSource {
                id: fixtures::STRIPE_ID.into(),
                body: json!({"status": "paused"}),
            },
            Err(FetchError::Network("connection refused".into())),
        );
        assert_eq!(app.toasts.last().unwrap().text, "connection refused");
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::Fetch { .. })));
    }

    #[test]
    fn a_paused_source_is_announced() {
        let mut app = fixtures::app();
        mutated(
            &mut app,
            Mutation::UpdateSource {
                id: fixtures::STRIPE_ID.into(),
                body: json!({"status": "paused"}),
            },
            Ok(json!({"id": fixtures::STRIPE_ID, "name": "stripe-prod", "status": "paused"})),
        );
        assert_eq!(app.toasts.last().unwrap().text, "Paused stripe-prod");
    }

    #[test]
    fn a_trashed_source_says_how_to_restore_it_and_leaves_its_detail() {
        let mut app = fixtures::source_detail(SourceTab::Overview);
        mutated(
            &mut app,
            Mutation::DeleteSource {
                id: fixtures::STRIPE_ID.into(),
            },
            Ok(Value::Null),
        );
        assert_eq!(
            app.toasts.last().unwrap().text,
            format!(
                "Moved to trash · restore with whk sources restore {}",
                fixtures::STRIPE_ID
            )
        );
        assert_eq!(app.screen, Screen::Sources);
    }

    #[test]
    fn other_mutations_are_left_to_plan_3() {
        let mut app = fixtures::app();
        let replay = Mutation::ReplayEvent {
            event_id: "e1".into(),
            connection_ids: vec![],
        };
        assert!(on_mutated(&mut app, &replay, &Ok(json!({"created": 1}))).is_none());
    }
}
