use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A single-line text field. The cursor is a character index, so multi-byte
/// input never splits a character.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    cursor: usize,
    masked: bool,
}

impl TextInput {
    pub fn new(value: impl Into<String>, masked: bool) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self {
            value,
            cursor,
            masked,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.value.chars().count();
    }

    /// The value as drawn: masked fields show one `mask` per character.
    pub fn display(&self, mask: char) -> String {
        if self.masked {
            mask.to_string().repeat(self.value.chars().count())
        } else {
            self.value.clone()
        }
    }

    /// Applies an editing key. Returns false for keys a field does not use
    /// (Enter, Tab, Esc, other control chords), so the caller can act on them.
    pub fn handle(&mut self, key: KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('u') if control => {
                self.value.clear();
                self.cursor = 0;
            }
            KeyCode::Char(character) if !control && !alt => {
                let index = self.byte_index();
                self.value.insert(index, character);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let index = self.byte_index();
                    self.value.remove(index);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.length() {
                    let index = self.byte_index();
                    self.value.remove(index);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.length()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.length(),
            _ => return false,
        }
        true
    }

    fn length(&self) -> usize {
        self.value.chars().count()
    }

    fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map(|(index, _)| index)
            .unwrap_or(self.value.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn typed(text: &str) -> TextInput {
        let mut input = TextInput::default();
        for character in text.chars() {
            assert!(input.handle(key(KeyCode::Char(character))));
        }
        input
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut input = typed("héllo");
        input.handle(key(KeyCode::Left));
        input.handle(key(KeyCode::Left));
        input.handle(key(KeyCode::Char('X')));
        assert_eq!(input.value(), "hélXlo");
        assert_eq!(input.cursor(), 4);
    }

    #[test]
    fn backspace_and_delete_respect_multibyte_characters() {
        let mut input = typed("aéb");
        input.handle(key(KeyCode::Left));
        input.handle(key(KeyCode::Backspace));
        assert_eq!(input.value(), "ab");
        input.handle(key(KeyCode::Home));
        input.handle(key(KeyCode::Delete));
        assert_eq!(input.value(), "b");
        input.handle(key(KeyCode::Home));
        assert!(input.handle(key(KeyCode::Backspace)));
        assert_eq!(input.value(), "b");
    }

    #[test]
    fn control_u_clears_and_masking_hides_the_value() {
        let mut input = TextInput::new("whk_secret", true);
        assert_eq!(input.display('•'), "••••••••••");
        input.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn navigation_keys_it_does_not_use_are_left_to_the_caller() {
        let mut input = typed("x");
        assert!(!input.handle(key(KeyCode::Enter)));
        assert!(!input.handle(key(KeyCode::Tab)));
        assert!(!input.handle(key(KeyCode::Esc)));
        assert!(!input.handle(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)));
        assert_eq!(input.value(), "x");
    }
}
