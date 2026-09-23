//! Copy through OSC 52: a few bytes written to the terminal, which puts them
//! on the system clipboard. No clipboard crate, works over SSH.

use std::io::Write;

use base64::Engine as _;

pub use crate::tui::settings::ClipboardMode;
use crate::tui::theme::TerminalEnv;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMethod {
    Osc52,
    /// Show the value in a modal for manual selection.
    Modal,
}

/// A value shown in the copy modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyValue {
    pub label: &'static str,
    pub text: String,
}

pub fn method(mode: ClipboardMode, env: &TerminalEnv) -> CopyMethod {
    if mode == ClipboardMode::Osc52 && !env.osc52_unsupported {
        CopyMethod::Osc52
    } else {
        CopyMethod::Modal
    }
}

pub fn osc52_sequence(text: &str) -> String {
    format!(
        "\u{1b}]52;c;{}\u{7}",
        base64::engine::general_purpose::STANDARD.encode(text)
    )
}

/// Called by the event loop between frames, never from a worker task.
pub fn write_osc52(text: &str) -> std::io::Result<()> {
    let mut stdout = std::io::stdout();
    stdout.write_all(osc52_sequence(text).as_bytes())?;
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::TerminalEnv;

    #[test]
    fn the_sequence_is_base64_between_osc_52_and_bel() {
        assert_eq!(osc52_sequence("hi"), "\u{1b}]52;c;aGk=\u{7}");
    }

    #[test]
    fn copies_go_to_a_modal_when_off_or_unsupported() {
        let supported = TerminalEnv::default();
        let unsupported = TerminalEnv {
            osc52_unsupported: true,
            ..TerminalEnv::default()
        };
        assert_eq!(method(ClipboardMode::Osc52, &supported), CopyMethod::Osc52);
        assert_eq!(method(ClipboardMode::Off, &supported), CopyMethod::Modal);
        assert_eq!(
            method(ClipboardMode::Osc52, &unsupported),
            CopyMethod::Modal
        );
    }

    #[test]
    fn terminals_known_without_osc_52() {
        let lookup = |pairs: &'static [(&'static str, &'static str)]| {
            TerminalEnv::from_lookup(move |name| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            })
        };
        assert!(lookup(&[("TERM_PROGRAM", "Apple_Terminal")]).osc52_unsupported);
        assert!(lookup(&[("TERM", "linux")]).osc52_unsupported);
        assert!(lookup(&[("TERM", "dumb")]).osc52_unsupported);
        assert!(!lookup(&[("TERM_PROGRAM", "iTerm.app")]).osc52_unsupported);
    }
}
