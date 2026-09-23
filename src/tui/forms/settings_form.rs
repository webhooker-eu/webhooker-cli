//! The Settings screen: a form over `[ui]`. Choices cycle with h/l, space or
//! enter; text fields take every printable key.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::forms::input::TextInput;
use crate::tui::settings::{is_http_url, Choice, Rgb, UiSettings, BUDGET_PERCENT_RANGE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    OpenOnBareCommand,
    Theme,
    Accent,
    Ascii,
    StartScreen,
    TimeFormat,
    RequestBudgetPercent,
    RelayDefaultUrl,
    Clipboard,
    CompactHeader,
}

impl SettingsField {
    pub const ALL: [SettingsField; 10] = [
        SettingsField::OpenOnBareCommand,
        SettingsField::Theme,
        SettingsField::Accent,
        SettingsField::Ascii,
        SettingsField::StartScreen,
        SettingsField::TimeFormat,
        SettingsField::RequestBudgetPercent,
        SettingsField::RelayDefaultUrl,
        SettingsField::Clipboard,
        SettingsField::CompactHeader,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsField::OpenOnBareCommand => "Open on bare `whk`",
            SettingsField::Theme => "Theme",
            SettingsField::Accent => "Accent color",
            SettingsField::Ascii => "ASCII only",
            SettingsField::StartScreen => "Start screen",
            SettingsField::TimeFormat => "Time format",
            SettingsField::RequestBudgetPercent => "Request budget (%)",
            SettingsField::RelayDefaultUrl => "Relay default URL",
            SettingsField::Clipboard => "Clipboard",
            SettingsField::CompactHeader => "Compact header",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormOutcome {
    /// Not a key the form uses; the app may treat it as a hotkey.
    Ignored,
    Consumed,
    Save,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct SettingsForm {
    pub draft: UiSettings,
    pub accent: TextInput,
    pub budget: TextInput,
    pub relay_url: TextInput,
    pub focused: usize,
    pub error: Option<String>,
    original: UiSettings,
}

impl SettingsForm {
    pub fn new(settings: &UiSettings) -> Self {
        Self {
            draft: settings.clone(),
            accent: TextInput::new(settings.accent.to_hex(), false),
            budget: TextInput::new(settings.request_budget_percent.to_string(), false),
            relay_url: TextInput::new(settings.relay_default_url.clone(), false),
            focused: 0,
            error: None,
            original: settings.clone(),
        }
    }

    pub fn focused_field(&self) -> SettingsField {
        SettingsField::ALL[self.focused]
    }

    pub fn text_input(&self, field: SettingsField) -> Option<&TextInput> {
        match field {
            SettingsField::Accent => Some(&self.accent),
            SettingsField::RequestBudgetPercent => Some(&self.budget),
            SettingsField::RelayDefaultUrl => Some(&self.relay_url),
            _ => None,
        }
    }

    fn text_input_mut(&mut self, field: SettingsField) -> Option<&mut TextInput> {
        match field {
            SettingsField::Accent => Some(&mut self.accent),
            SettingsField::RequestBudgetPercent => Some(&mut self.budget),
            SettingsField::RelayDefaultUrl => Some(&mut self.relay_url),
            _ => None,
        }
    }

    pub fn value_label(&self, field: SettingsField) -> String {
        let on_off = |value: bool| if value { "on" } else { "off" }.to_string();
        match field {
            SettingsField::OpenOnBareCommand => on_off(self.draft.open_on_bare_command),
            SettingsField::Theme => self.draft.theme.label().to_string(),
            SettingsField::Ascii => on_off(self.draft.ascii),
            SettingsField::StartScreen => self.draft.start_screen.label().to_string(),
            SettingsField::TimeFormat => self.draft.time_format.label().to_string(),
            SettingsField::Clipboard => self.draft.clipboard.label().to_string(),
            SettingsField::CompactHeader => on_off(self.draft.compact_header),
            SettingsField::Accent
            | SettingsField::RequestBudgetPercent
            | SettingsField::RelayDefaultUrl => self
                .text_input(field)
                .map(|input| input.value().to_string())
                .unwrap_or_default(),
        }
    }

    /// An invalid draft counts as changed, so leaving still asks first.
    pub fn is_dirty(&self) -> bool {
        self.collect()
            .map_or(true, |settings| settings != self.original)
    }

    pub fn collect(&self) -> Result<UiSettings, String> {
        let accent = Rgb::parse_hex(self.accent.value().trim())
            .ok_or_else(|| "Accent color must look like #7c5cff".to_string())?;
        let request_budget_percent = self
            .budget
            .value()
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|percent| BUDGET_PERCENT_RANGE.contains(percent))
            .ok_or_else(|| "Request budget must be a whole number from 10 to 90".to_string())?;
        let relay_default_url = self.relay_url.value().trim();
        if !is_http_url(relay_default_url) {
            return Err("Relay default URL must be an absolute http(s) URL".to_string());
        }
        Ok(UiSettings {
            accent,
            request_budget_percent,
            relay_default_url: relay_default_url.to_string(),
            ..self.draft.clone()
        })
    }

    pub fn handle(&mut self, key: KeyEvent) -> FormOutcome {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            return FormOutcome::Save;
        }
        let last = SettingsField::ALL.len() - 1;
        match key.code {
            KeyCode::Esc => return FormOutcome::Cancel,
            KeyCode::Tab | KeyCode::Down => {
                self.focused = (self.focused + 1).min(last);
                return FormOutcome::Consumed;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focused = self.focused.saturating_sub(1);
                return FormOutcome::Consumed;
            }
            _ => {}
        }
        let field = self.focused_field();
        if let Some(input) = self.text_input_mut(field) {
            return if input.handle(key) {
                FormOutcome::Consumed
            } else {
                FormOutcome::Ignored
            };
        }
        let forward = match key.code {
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') | KeyCode::Enter => true,
            KeyCode::Left | KeyCode::Char('h') => false,
            _ => return FormOutcome::Ignored,
        };
        self.step(field, forward);
        FormOutcome::Consumed
    }

    fn step(&mut self, field: SettingsField, forward: bool) {
        fn cycle<T: Choice>(value: T, forward: bool) -> T {
            if forward {
                value.next()
            } else {
                value.previous()
            }
        }
        let draft = &mut self.draft;
        match field {
            SettingsField::OpenOnBareCommand => {
                draft.open_on_bare_command = !draft.open_on_bare_command
            }
            SettingsField::Theme => draft.theme = cycle(draft.theme, forward),
            SettingsField::Ascii => draft.ascii = !draft.ascii,
            SettingsField::StartScreen => draft.start_screen = cycle(draft.start_screen, forward),
            SettingsField::TimeFormat => draft.time_format = cycle(draft.time_format, forward),
            SettingsField::Clipboard => draft.clipboard = cycle(draft.clipboard, forward),
            SettingsField::CompactHeader => draft.compact_header = !draft.compact_header,
            SettingsField::Accent
            | SettingsField::RequestBudgetPercent
            | SettingsField::RelayDefaultUrl => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::settings::{ThemeChoice, TimeFormat};

    fn press(form: &mut SettingsForm, code: KeyCode) -> FormOutcome {
        form.handle(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn focus(form: &mut SettingsForm, field: SettingsField) {
        while form.focused_field() != field {
            press(form, KeyCode::Tab);
        }
    }

    #[test]
    fn choices_cycle_and_toggles_flip() {
        let mut form = SettingsForm::new(&UiSettings::default());
        focus(&mut form, SettingsField::Theme);
        assert_eq!(press(&mut form, KeyCode::Right), FormOutcome::Consumed);
        assert_eq!(form.draft.theme, ThemeChoice::Dark);
        press(&mut form, KeyCode::Left);
        press(&mut form, KeyCode::Left);
        assert_eq!(form.draft.theme, ThemeChoice::HighContrast);
        focus(&mut form, SettingsField::TimeFormat);
        press(&mut form, KeyCode::Char(' '));
        assert_eq!(form.draft.time_format, TimeFormat::Utc);
        focus(&mut form, SettingsField::CompactHeader);
        press(&mut form, KeyCode::Enter);
        assert!(form.draft.compact_header);
        assert!(form.is_dirty());
    }

    #[test]
    fn text_fields_capture_letters_and_validate_on_collect() {
        let mut form = SettingsForm::new(&UiSettings::default());
        focus(&mut form, SettingsField::RequestBudgetPercent);
        form.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for character in "95".chars() {
            assert_eq!(
                press(&mut form, KeyCode::Char(character)),
                FormOutcome::Consumed
            );
        }
        assert_eq!(
            form.collect().unwrap_err(),
            "Request budget must be a whole number from 10 to 90"
        );
        press(&mut form, KeyCode::Backspace);
        press(&mut form, KeyCode::Backspace);
        press(&mut form, KeyCode::Char('4'));
        press(&mut form, KeyCode::Char('0'));
        assert_eq!(form.collect().unwrap().request_budget_percent, 40);
    }

    #[test]
    fn letters_on_a_choice_field_are_left_to_the_app() {
        let mut form = SettingsForm::new(&UiSettings::default());
        focus(&mut form, SettingsField::Theme);
        assert_eq!(press(&mut form, KeyCode::Char('q')), FormOutcome::Ignored);
        assert_eq!(press(&mut form, KeyCode::Char('g')), FormOutcome::Ignored);
    }

    #[test]
    fn save_cancel_and_dirty_tracking() {
        let mut form = SettingsForm::new(&UiSettings::default());
        assert!(!form.is_dirty());
        assert_eq!(
            form.handle(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            FormOutcome::Save
        );
        assert_eq!(press(&mut form, KeyCode::Esc), FormOutcome::Cancel);
        focus(&mut form, SettingsField::Accent);
        press(&mut form, KeyCode::Backspace);
        assert!(form.is_dirty(), "an invalid draft counts as a change");
        assert_eq!(
            form.collect().unwrap_err(),
            "Accent color must look like #7c5cff"
        );
    }

    #[test]
    fn value_labels_describe_every_field() {
        let form = SettingsForm::new(&UiSettings::default());
        assert_eq!(form.value_label(SettingsField::OpenOnBareCommand), "on");
        assert_eq!(form.value_label(SettingsField::Theme), "auto");
        assert_eq!(form.value_label(SettingsField::Accent), "#7c5cff");
        assert_eq!(form.value_label(SettingsField::RequestBudgetPercent), "50");
        assert_eq!(form.value_label(SettingsField::Clipboard), "osc52");
        assert!(form.text_input(SettingsField::RelayDefaultUrl).is_some());
        assert!(form.text_input(SettingsField::Theme).is_none());
    }
}
