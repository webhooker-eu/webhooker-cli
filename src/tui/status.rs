//! Status values from the API mapped to a glyph and a tone. Any value the TUI
//! does not know gets a neutral `?` and no actions, never a crash.

use crate::tui::theme::{Glyphs, Tone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    pub glyph: &'static str,
    pub tone: Tone,
}

fn badge(glyph: &'static str, tone: Tone) -> Badge {
    Badge { glyph, tone }
}

pub fn unknown(glyphs: &Glyphs) -> Badge {
    badge(glyphs.unknown, Tone::Muted)
}

/// Sources and destinations share `active` / `paused`.
pub fn resource_status(status: &str, glyphs: &Glyphs) -> Badge {
    match status {
        "active" => badge(glyphs.active, Tone::Success),
        "paused" => badge(glyphs.paused, Tone::Muted),
        _ => unknown(glyphs),
    }
}

/// A closed circuit is the normal state and shows nothing.
pub fn circuit(state: &str, glyphs: &Glyphs) -> Option<Badge> {
    match state {
        "closed" => None,
        "open" => Some(badge(glyphs.circuit, Tone::Danger)),
        "half_open" => Some(badge(glyphs.circuit, Tone::Warning)),
        _ => Some(unknown(glyphs)),
    }
}

pub fn enabled(enabled: bool, glyphs: &Glyphs) -> Badge {
    if enabled {
        badge(glyphs.active, Tone::Success)
    } else {
        badge(glyphs.inactive, Tone::Muted)
    }
}

pub fn verification(status: &str, glyphs: &Glyphs) -> Badge {
    match status {
        "verified" => badge(glyphs.success, Tone::Success),
        "failed" => badge(glyphs.failure, Tone::Danger),
        "skipped" => badge(glyphs.skipped, Tone::Muted),
        _ => unknown(glyphs),
    }
}

pub fn delivery(status: &str, glyphs: &Glyphs, tick: usize) -> Badge {
    match status {
        "pending" | "delivering" => badge(glyphs.spinner_frame(tick), Tone::Accent),
        "succeeded" => badge(glyphs.success, Tone::Success),
        "failed" | "exhausted" => badge(glyphs.failure, Tone::Danger),
        "filtered" => badge(glyphs.filtered, Tone::Muted),
        _ => unknown(glyphs),
    }
}

pub fn is_transitional(status: &str) -> bool {
    matches!(status, "pending" | "delivering")
}

/// The status a pause/resume toggle would set; an unknown status offers none.
pub fn toggled_status(status: &str) -> Option<&'static str> {
    match status {
        "active" => Some("paused"),
        "paused" => Some("active"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::{ASCII_GLYPHS, UNICODE_GLYPHS};

    static GLYPHS: &Glyphs = &UNICODE_GLYPHS;

    #[test]
    fn resource_statuses() {
        assert_eq!(
            resource_status("active", GLYPHS),
            Badge {
                glyph: "●",
                tone: Tone::Success
            }
        );
        assert_eq!(
            resource_status("paused", GLYPHS),
            Badge {
                glyph: "◐",
                tone: Tone::Muted
            }
        );
        assert_eq!(
            resource_status("archived", GLYPHS),
            Badge {
                glyph: "?",
                tone: Tone::Muted
            }
        );
    }

    #[test]
    fn circuit_states() {
        assert_eq!(circuit("closed", GLYPHS), None);
        assert_eq!(
            circuit("open", GLYPHS),
            Some(Badge {
                glyph: "⚡",
                tone: Tone::Danger
            })
        );
        assert_eq!(
            circuit("half_open", GLYPHS),
            Some(Badge {
                glyph: "⚡",
                tone: Tone::Warning
            })
        );
        assert_eq!(
            circuit("melted", GLYPHS),
            Some(Badge {
                glyph: "?",
                tone: Tone::Muted
            })
        );
    }

    #[test]
    fn verification_and_enabled() {
        assert_eq!(verification("verified", GLYPHS).tone, Tone::Success);
        assert_eq!(verification("failed", GLYPHS).glyph, "✕");
        assert_eq!(verification("skipped", GLYPHS).glyph, "–");
        assert_eq!(verification("new", GLYPHS).glyph, "?");
        assert_eq!(
            enabled(true, GLYPHS),
            Badge {
                glyph: "●",
                tone: Tone::Success
            }
        );
        assert_eq!(
            enabled(false, GLYPHS),
            Badge {
                glyph: "○",
                tone: Tone::Muted
            }
        );
    }

    #[test]
    fn deliveries_spin_while_transitional() {
        assert_eq!(delivery("pending", GLYPHS, 0).glyph, "⠋");
        assert_eq!(delivery("delivering", GLYPHS, 1).glyph, "⠙");
        assert_eq!(delivery("succeeded", GLYPHS, 0).tone, Tone::Success);
        assert_eq!(delivery("exhausted", GLYPHS, 0).tone, Tone::Danger);
        assert_eq!(delivery("filtered", &ASCII_GLYPHS, 0).glyph, "/");
        assert_eq!(delivery("teleported", GLYPHS, 0).glyph, "?");
        assert!(is_transitional("pending"));
        assert!(!is_transitional("failed"));
    }

    #[test]
    fn unknown_statuses_offer_no_toggle() {
        assert_eq!(toggled_status("active"), Some("paused"));
        assert_eq!(toggled_status("paused"), Some("active"));
        assert_eq!(toggled_status("archived"), None);
    }
}
