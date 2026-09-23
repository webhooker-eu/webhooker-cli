//! A modal form of typed fields. Tab and Shift+Tab move between fields and
//! wrap, Ctrl+S saves, Esc cancels. Keys a field does not use are reported as
//! `Ignored`, never handed to the screen behind the form.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::forms::input::TextInput;
use crate::tui::forms::settings_form::FormOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

impl SelectOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckItem {
    pub value: String,
    pub label: String,
    pub checked: bool,
    /// Shown next to the label, e.g. "disabled".
    pub note: Option<String>,
}

impl CheckItem {
    pub fn new(value: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            checked,
            note: None,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// Rows of `Name: Value`. In list mode `a` adds a row, `x` removes the focused
/// row and `enter` edits it; while editing, printable keys are input and
/// `enter` or `esc` finish editing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeadersEditor {
    pub rows: Vec<TextInput>,
    pub cursor: usize,
    pub editing: bool,
}

impl HeadersEditor {
    pub fn new(pairs: &[(String, String)]) -> Self {
        Self {
            rows: pairs
                .iter()
                .map(|(name, value)| TextInput::new(format!("{name}: {value}"), false))
                .collect(),
            cursor: 0,
            editing: false,
        }
    }

    fn handle(&mut self, key: KeyEvent) -> bool {
        if self.editing {
            return match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.editing = false;
                    true
                }
                _ => self
                    .rows
                    .get_mut(self.cursor)
                    .is_some_and(|row| row.handle(key)),
            };
        }
        match key.code {
            KeyCode::Char('a') => {
                self.rows.push(TextInput::default());
                self.cursor = self.rows.len() - 1;
                self.editing = true;
                true
            }
            KeyCode::Char('x') if !self.rows.is_empty() => {
                self.rows.remove(self.cursor);
                self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
                true
            }
            KeyCode::Enter if !self.rows.is_empty() => {
                self.editing = true;
                true
            }
            KeyCode::Up | KeyCode::Char('k') if self.cursor > 0 => {
                self.cursor -= 1;
                true
            }
            KeyCode::Down | KeyCode::Char('j') if self.cursor + 1 < self.rows.len() => {
                self.cursor += 1;
                true
            }
            _ => false,
        }
    }

    fn non_empty_rows(&self) -> impl Iterator<Item = &str> {
        self.rows
            .iter()
            .map(|row| row.value().trim())
            .filter(|raw| !raw.is_empty())
    }

    pub fn pairs(&self) -> Result<Vec<(String, String)>, String> {
        self.non_empty_rows()
            .map(|raw| {
                let (name, value) = raw
                    .split_once(':')
                    .ok_or_else(|| format!("\"{raw}\" must look like Name: Value"))?;
                let name = name.trim();
                if name.is_empty() {
                    return Err(format!("\"{raw}\" has no header name"));
                }
                Ok((name.to_string(), value.trim().to_string()))
            })
            .collect()
    }

    /// Rows read leniently, for change tracking only.
    fn loose_pairs(&self) -> Vec<(String, String)> {
        self.non_empty_rows()
            .map(|raw| match raw.split_once(':') {
                Some((name, value)) => (name.trim().to_string(), value.trim().to_string()),
                None => (raw.to_string(), String::new()),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Text(TextInput),
    /// Masked; on edit the caller treats "" as "keep the current one".
    Secret(TextInput),
    Select {
        options: Vec<SelectOption>,
        selected: usize,
    },
    Toggle(bool),
    Checklist {
        items: Vec<CheckItem>,
        cursor: usize,
    },
    Headers(HeadersEditor),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Text(String),
    Bool(bool),
    Selected(String),
    Checked(Vec<String>),
    Headers(Vec<(String, String)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub key: &'static str,
    pub label: String,
    pub kind: FieldKind,
    pub error: Option<String>,
}

impl Field {
    fn with_kind(key: &'static str, label: impl Into<String>, kind: FieldKind) -> Self {
        Self {
            key,
            label: label.into(),
            kind,
            error: None,
        }
    }

    pub fn text(key: &'static str, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::with_kind(key, label, FieldKind::Text(TextInput::new(value, false)))
    }

    pub fn secret(key: &'static str, label: impl Into<String>) -> Self {
        Self::with_kind(key, label, FieldKind::Secret(TextInput::new("", true)))
    }

    pub fn select(
        key: &'static str,
        label: impl Into<String>,
        options: Vec<SelectOption>,
        selected_value: Option<&str>,
    ) -> Self {
        let selected = selected_value
            .and_then(|value| options.iter().position(|option| option.value == value))
            .unwrap_or(0);
        Self::with_kind(key, label, FieldKind::Select { options, selected })
    }

    pub fn toggle(key: &'static str, label: impl Into<String>, value: bool) -> Self {
        Self::with_kind(key, label, FieldKind::Toggle(value))
    }

    pub fn checklist(key: &'static str, label: impl Into<String>, items: Vec<CheckItem>) -> Self {
        Self::with_kind(key, label, FieldKind::Checklist { items, cursor: 0 })
    }

    pub fn headers(
        key: &'static str,
        label: impl Into<String>,
        pairs: &[(String, String)],
    ) -> Self {
        Self::with_kind(key, label, FieldKind::Headers(HeadersEditor::new(pairs)))
    }

    pub fn value(&self) -> FieldValue {
        match &self.kind {
            FieldKind::Text(input) | FieldKind::Secret(input) => {
                FieldValue::Text(input.value().to_string())
            }
            FieldKind::Select { options, selected } => FieldValue::Selected(
                options
                    .get(*selected)
                    .map(|option| option.value.clone())
                    .unwrap_or_default(),
            ),
            FieldKind::Toggle(value) => FieldValue::Bool(*value),
            FieldKind::Checklist { items, .. } => FieldValue::Checked(
                items
                    .iter()
                    .filter(|item| item.checked)
                    .map(|item| item.value.clone())
                    .collect(),
            ),
            FieldKind::Headers(editor) => FieldValue::Headers(editor.loose_pairs()),
        }
    }

    /// The text input that has the cursor: a text or secret field, or the
    /// header row being edited.
    pub fn text_input(&self) -> Option<&TextInput> {
        match &self.kind {
            FieldKind::Text(input) | FieldKind::Secret(input) => Some(input),
            FieldKind::Headers(editor) if editor.editing => editor.rows.get(editor.cursor),
            _ => None,
        }
    }

    fn is_editing_headers(&self) -> bool {
        matches!(&self.kind, FieldKind::Headers(editor) if editor.editing)
    }

    fn stop_editing(&mut self) {
        if let FieldKind::Headers(editor) = &mut self.kind {
            editor.editing = false;
        }
    }

    fn handle(&mut self, key: KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match &mut self.kind {
            FieldKind::Text(input) | FieldKind::Secret(input) => input.handle(key),
            FieldKind::Select { options, selected } => {
                if options.is_empty() {
                    return false;
                }
                let count = options.len();
                match key.code {
                    KeyCode::Left | KeyCode::Char('h') => {
                        *selected = (*selected + count - 1) % count;
                        true
                    }
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Enter => {
                        *selected = (*selected + 1) % count;
                        true
                    }
                    KeyCode::Char(letter) if !control => {
                        let wanted = letter.to_ascii_lowercase();
                        let found =
                            (1..=count)
                                .map(|step| (*selected + step) % count)
                                .find(|index| {
                                    options[*index]
                                        .label
                                        .chars()
                                        .next()
                                        .is_some_and(|first| first.to_ascii_lowercase() == wanted)
                                });
                        match found {
                            Some(index) => {
                                *selected = index;
                                true
                            }
                            None => false,
                        }
                    }
                    _ => false,
                }
            }
            FieldKind::Toggle(value) => match key.code {
                KeyCode::Char(' ')
                | KeyCode::Enter
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Char('h')
                | KeyCode::Char('l') => {
                    *value = !*value;
                    true
                }
                _ => false,
            },
            FieldKind::Checklist { items, cursor } => match key.code {
                KeyCode::Up | KeyCode::Char('k') if *cursor > 0 => {
                    *cursor -= 1;
                    true
                }
                KeyCode::Down | KeyCode::Char('j') if *cursor + 1 < items.len() => {
                    *cursor += 1;
                    true
                }
                KeyCode::Char(' ') => match items.get_mut(*cursor) {
                    Some(item) => {
                        item.checked = !item.checked;
                        true
                    }
                    None => false,
                },
                _ => false,
            },
            FieldKind::Headers(editor) => editor.handle(key),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Form {
    pub title: String,
    pub fields: Vec<Field>,
    pub focused: usize,
    /// Shown at the bottom, e.g. a server message.
    pub error: Option<String>,
    /// A save is in flight: the title spins and keys are ignored.
    pub submitting: bool,
    initial: Vec<FieldValue>,
}

impl Form {
    pub fn new(title: impl Into<String>, fields: Vec<Field>) -> Self {
        let initial = fields.iter().map(Field::value).collect();
        Self {
            title: title.into(),
            fields,
            focused: 0,
            error: None,
            submitting: false,
            initial,
        }
    }

    pub fn focused_field(&self) -> &Field {
        &self.fields[self.focused]
    }

    pub fn handle(&mut self, key: KeyEvent) -> FormOutcome {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && key.code == KeyCode::Char('s') {
            for field in &mut self.fields {
                field.stop_editing();
            }
            return FormOutcome::Save;
        }
        if self.fields.is_empty() {
            return match key.code {
                KeyCode::Esc => FormOutcome::Cancel,
                _ => FormOutcome::Ignored,
            };
        }
        let count = self.fields.len();
        match key.code {
            KeyCode::Esc if !self.fields[self.focused].is_editing_headers() => {
                return FormOutcome::Cancel
            }
            KeyCode::Tab => {
                self.fields[self.focused].stop_editing();
                self.focused = (self.focused + 1) % count;
                return FormOutcome::Consumed;
            }
            KeyCode::BackTab => {
                self.fields[self.focused].stop_editing();
                self.focused = (self.focused + count - 1) % count;
                return FormOutcome::Consumed;
            }
            _ => {}
        }
        if self.fields[self.focused].handle(key) {
            return FormOutcome::Consumed;
        }
        match key.code {
            KeyCode::Down if self.focused + 1 < count => {
                self.focused += 1;
                FormOutcome::Consumed
            }
            KeyCode::Up if self.focused > 0 => {
                self.focused -= 1;
                FormOutcome::Consumed
            }
            _ => FormOutcome::Ignored,
        }
    }

    fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.key == key)
    }

    pub fn value(&self, key: &str) -> Option<FieldValue> {
        self.field(key).map(Field::value)
    }

    pub fn text(&self, key: &str) -> String {
        match self.value(key) {
            Some(FieldValue::Text(text)) => text,
            _ => String::new(),
        }
    }

    pub fn bool(&self, key: &str) -> bool {
        matches!(self.value(key), Some(FieldValue::Bool(true)))
    }

    pub fn selected(&self, key: &str) -> Option<String> {
        match self.value(key) {
            Some(FieldValue::Selected(value)) => Some(value),
            _ => None,
        }
    }

    pub fn checked(&self, key: &str) -> Vec<String> {
        match self.value(key) {
            Some(FieldValue::Checked(values)) => values,
            _ => Vec::new(),
        }
    }

    pub fn headers(&self, key: &str) -> Result<Vec<(String, String)>, String> {
        match self.field(key).map(|field| &field.kind) {
            Some(FieldKind::Headers(editor)) => editor.pairs(),
            _ => Ok(Vec::new()),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.fields
            .iter()
            .map(Field::value)
            .ne(self.initial.iter().cloned())
    }

    pub fn set_field_error(&mut self, key: &str, message: impl Into<String>) {
        if let Some(field) = self.fields.iter_mut().find(|field| field.key == key) {
            field.error = Some(message.into());
        }
    }

    pub fn clear_errors(&mut self) {
        self.error = None;
        for field in &mut self.fields {
            field.error = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press(form: &mut Form, code: KeyCode) -> FormOutcome {
        form.handle(key(code))
    }

    fn type_text(form: &mut Form, text: &str) {
        for character in text.chars() {
            assert_eq!(press(form, KeyCode::Char(character)), FormOutcome::Consumed);
        }
    }

    fn sample() -> Form {
        Form::new(
            "Sample",
            vec![
                Field::text("name", "Name", "billing"),
                Field::secret("secret", "Secret"),
                Field::select(
                    "provider",
                    "Provider",
                    vec![
                        SelectOption::new("none", "none"),
                        SelectOption::new("stripe", "stripe"),
                        SelectOption::new("github", "github"),
                        SelectOption::new("shopify", "shopify"),
                    ],
                    Some("stripe"),
                ),
                Field::toggle("enabled", "Enabled", true),
                Field::checklist(
                    "connections",
                    "Connections",
                    vec![
                        CheckItem::new("c1", "billing-worker", true),
                        CheckItem::new("c2", "audit-log", true).with_note("disabled"),
                    ],
                ),
                Field::headers(
                    "headers",
                    "Headers",
                    &[("X-Env".to_string(), "prod".to_string())],
                ),
            ],
        )
    }

    #[test]
    fn text_fields_take_every_printable_key() {
        let mut form = sample();
        type_text(&mut form, "-2qj");
        assert_eq!(form.text("name"), "billing-2qj");
        assert_eq!(press(&mut form, KeyCode::Down), FormOutcome::Consumed);
        type_text(&mut form, "whsec");
        assert_eq!(form.text("secret"), "whsec");
        assert_eq!(form.value("secret"), Some(FieldValue::Text("whsec".into())));
    }

    #[test]
    fn tab_and_backtab_wrap_around() {
        let mut form = sample();
        press(&mut form, KeyCode::BackTab);
        assert_eq!(form.focused_field().key, "headers");
        press(&mut form, KeyCode::Tab);
        assert_eq!(form.focused_field().key, "name");
    }

    #[test]
    fn selects_cycle_and_jump_by_first_letter() {
        let mut form = sample();
        form.focused = 2;
        press(&mut form, KeyCode::Right);
        assert_eq!(form.selected("provider").as_deref(), Some("github"));
        press(&mut form, KeyCode::Char('s'));
        assert_eq!(form.selected("provider").as_deref(), Some("shopify"));
        press(&mut form, KeyCode::Char('s'));
        assert_eq!(form.selected("provider").as_deref(), Some("stripe"));
        assert_eq!(press(&mut form, KeyCode::Char('q')), FormOutcome::Ignored);
        press(&mut form, KeyCode::Left);
        assert_eq!(form.selected("provider").as_deref(), Some("none"));
    }

    #[test]
    fn toggles_flip() {
        let mut form = sample();
        form.focused = 3;
        press(&mut form, KeyCode::Char(' '));
        assert!(!form.bool("enabled"));
        press(&mut form, KeyCode::Enter);
        assert!(form.bool("enabled"));
    }

    #[test]
    fn checklists_move_inside_and_hand_off_at_the_edges() {
        let mut form = sample();
        form.focused = 4;
        press(&mut form, KeyCode::Char(' '));
        assert_eq!(form.checked("connections"), vec!["c2".to_string()]);
        press(&mut form, KeyCode::Char('j'));
        press(&mut form, KeyCode::Char(' '));
        assert!(form.checked("connections").is_empty());
        press(&mut form, KeyCode::Down);
        assert_eq!(
            form.focused_field().key,
            "headers",
            "down past the last item moves on"
        );
    }

    #[test]
    fn headers_add_edit_and_remove_rows() {
        let mut form = sample();
        form.focused = 5;
        press(&mut form, KeyCode::Char('a'));
        type_text(&mut form, "X-Team: billing");
        assert_eq!(
            press(&mut form, KeyCode::Esc),
            FormOutcome::Consumed,
            "esc ends editing"
        );
        assert_eq!(
            form.headers("headers").unwrap(),
            vec![
                ("X-Env".to_string(), "prod".to_string()),
                ("X-Team".to_string(), "billing".to_string())
            ]
        );
        press(&mut form, KeyCode::Char('k'));
        press(&mut form, KeyCode::Char('x'));
        assert_eq!(
            form.headers("headers").unwrap(),
            vec![("X-Team".to_string(), "billing".to_string())]
        );
        press(&mut form, KeyCode::Char('a'));
        type_text(&mut form, "broken");
        press(&mut form, KeyCode::Enter);
        assert_eq!(
            form.headers("headers").unwrap_err(),
            "\"broken\" must look like Name: Value"
        );
        assert_eq!(press(&mut form, KeyCode::Esc), FormOutcome::Cancel);
    }

    #[test]
    fn save_cancel_dirty_and_errors() {
        let mut form = sample();
        assert!(!form.is_dirty());
        assert_eq!(
            form.handle(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            FormOutcome::Save
        );
        type_text(&mut form, "x");
        assert!(form.is_dirty());
        form.set_field_error("name", "taken");
        form.error = Some("server said no".into());
        assert_eq!(form.fields[0].error.as_deref(), Some("taken"));
        form.clear_errors();
        assert!(form.fields[0].error.is_none());
        assert!(form.error.is_none());
        assert_eq!(press(&mut form, KeyCode::Esc), FormOutcome::Cancel);
    }

    #[test]
    fn missing_keys_read_as_empty() {
        let form = sample();
        assert_eq!(form.value("nope"), None);
        assert_eq!(form.text("nope"), "");
        assert!(!form.bool("nope"));
        assert_eq!(form.selected("nope"), None);
        assert!(form.checked("nope").is_empty());
        assert_eq!(form.headers("nope").unwrap(), Vec::new());
    }
}
