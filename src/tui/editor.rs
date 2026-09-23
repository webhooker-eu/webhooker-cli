//! The `$EDITOR` round trip for JSON fields. The event loop suspends the
//! terminal around `edit`; this module never touches the screen.

use std::io::Write;
use std::process::Command;

use serde_json::Value;
use tempfile::NamedTempFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    pub program: String,
    pub arguments: Vec<String>,
}

impl EditorCommand {
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// `$VISUAL`, then `$EDITOR`, then `vi` (Unix) or `notepad` (Windows).
    /// Values like `code --wait` are split on whitespace.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let raw = ["VISUAL", "EDITOR"]
            .iter()
            .filter_map(|name| lookup(name))
            .find(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_editor().to_string());
        let mut words = raw.split_whitespace().map(str::to_string);
        let program = words.next().unwrap_or_else(|| default_editor().to_string());
        Self {
            program,
            arguments: words.collect(),
        }
    }
}

fn default_editor() -> &'static str {
    if cfg!(windows) {
        "notepad"
    } else {
        "vi"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditOutcome {
    /// An empty file is `Value::Null`, which clears the field.
    Parsed(Value),
    /// The text is kept so `Enter` reopens exactly what the user wrote.
    Invalid { text: String, message: String },
}

/// The file content for a value: `null` becomes an empty file.
pub fn text_for(value: &Value) -> String {
    if value.is_null() {
        String::new()
    } else {
        serde_json::to_string_pretty(value).unwrap_or_default()
    }
}

pub fn parse(text: &str) -> EditOutcome {
    if text.trim().is_empty() {
        return EditOutcome::Parsed(Value::Null);
    }
    match serde_json::from_str(text) {
        Ok(value) => EditOutcome::Parsed(value),
        Err(error) => EditOutcome::Invalid {
            text: text.to_string(),
            message: format!(
                "Invalid JSON at line {}, column {} (enter to fix)",
                error.line(),
                error.column()
            ),
        },
    }
}

/// `tempfile` creates the file with mode 0600 on Unix; the `.json` suffix
/// gives editors syntax highlighting.
fn write_temp(text: &str) -> std::io::Result<NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("whk-")
        .suffix(".json")
        .tempfile()?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    Ok(file)
}

/// Blocks until the editor exits. The caller must have left raw mode and the
/// alternate screen first.
pub fn edit(command: &EditorCommand, text: &str) -> Result<EditOutcome, String> {
    let file =
        write_temp(text).map_err(|error| format!("could not create a temp file: {error}"))?;
    let status = Command::new(&command.program)
        .args(&command.arguments)
        .arg(file.path())
        .status()
        .map_err(|error| format!("could not start {}: {error}", command.program))?;
    if !status.success() {
        return Err(format!("{} exited with {status}", command.program));
    }
    let edited = std::fs::read_to_string(file.path())
        .map_err(|error| format!("could not read the edited file: {error}"))?;
    Ok(parse(&edited))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lookup_from(
        pairs: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn visual_beats_editor_beats_the_platform_default() {
        let both = EditorCommand::from_lookup(lookup_from(&[
            ("VISUAL", "code --wait"),
            ("EDITOR", "nano"),
        ]));
        assert_eq!(both.program, "code");
        assert_eq!(both.arguments, vec!["--wait".to_string()]);
        let editor_only =
            EditorCommand::from_lookup(lookup_from(&[("VISUAL", " "), ("EDITOR", "nano")]));
        assert_eq!(editor_only.program, "nano");
        let neither = EditorCommand::from_lookup(lookup_from(&[]));
        assert_eq!(
            neither.program,
            if cfg!(windows) { "notepad" } else { "vi" }
        );
    }

    #[test]
    fn parse_accepts_json_and_treats_empty_as_null() {
        assert_eq!(parse("{\"a\": 1}"), EditOutcome::Parsed(json!({"a": 1})));
        assert_eq!(parse("  \n"), EditOutcome::Parsed(Value::Null));
    }

    #[test]
    fn parse_errors_carry_line_and_column_and_keep_the_text() {
        let EditOutcome::Invalid { text, message } = parse("{\"a\": }") else {
            panic!("expected an invalid outcome");
        };
        assert_eq!(text, "{\"a\": }");
        assert!(
            message.starts_with("Invalid JSON at line 1, column 7"),
            "{message}"
        );
    }

    #[test]
    fn text_for_shows_null_as_an_empty_file() {
        assert_eq!(text_for(&Value::Null), "");
        assert_eq!(text_for(&json!({"a": 1})), "{\n  \"a\": 1\n}");
    }

    #[cfg(unix)]
    #[test]
    fn the_temp_file_is_private_and_ends_in_json() {
        use std::os::unix::fs::PermissionsExt;
        let file = write_temp("{}").unwrap();
        let mode = std::fs::metadata(file.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(file.path().to_string_lossy().ends_with(".json"));
    }

    /// Runs the script through `sh` so the test never executes a file it just
    /// wrote (which can fail with ETXTBSY while other tests fork).
    #[cfg(unix)]
    fn stub_editor(script: &str) -> (tempfile::TempDir, EditorCommand) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("editor.sh");
        std::fs::write(&path, script).unwrap();
        let command = EditorCommand {
            program: "sh".to_string(),
            arguments: vec![path.to_string_lossy().into_owned()],
        };
        (directory, command)
    }

    #[cfg(unix)]
    #[test]
    fn a_stub_editor_round_trip() {
        let (_directory, command) = stub_editor("printf '%s' '{\"edited\": true}' > \"$1\"\n");
        assert_eq!(
            edit(&command, "{}"),
            Ok(EditOutcome::Parsed(json!({"edited": true})))
        );

        let (_directory, command) = stub_editor("printf '%s' '{oops' > \"$1\"\n");
        assert!(matches!(
            edit(&command, "{}"),
            Ok(EditOutcome::Invalid { ref text, .. }) if text == "{oops"
        ));

        let (_directory, command) = stub_editor(": > \"$1\"\n");
        assert_eq!(
            edit(&command, "{\"a\": 1}"),
            Ok(EditOutcome::Parsed(Value::Null))
        );

        let (_directory, command) = stub_editor("exit 3\n");
        assert!(edit(&command, "{}").unwrap_err().contains("exited"));

        let missing = EditorCommand {
            program: "/nonexistent/whk-editor".to_string(),
            arguments: vec![],
        };
        assert!(edit(&missing, "{}")
            .unwrap_err()
            .contains("could not start"));
    }
}
