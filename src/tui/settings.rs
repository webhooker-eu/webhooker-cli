//! The `[ui]` settings with defaults and range checks applied. Invalid values
//! fall back to the default and produce a warning shown as a toast.

use crate::config::{UiSection, UiState};

/// A setting with a fixed set of values, stored in the config as its label.
pub trait Choice: Copy + PartialEq + 'static {
    const ALL: &'static [Self];
    fn label(self) -> &'static str;

    fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|choice| choice.label() == raw)
    }

    fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|choice| *choice == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|choice| *choice == self)
            .unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

macro_rules! choice_enum {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant),+
        }

        impl Choice for $name {
            const ALL: &'static [Self] = &[$(Self::$variant),+];

            fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => $label),+
                }
            }
        }
    };
}

choice_enum!(ThemeChoice {
    Auto => "auto",
    Dark => "dark",
    Light => "light",
    HighContrast => "high-contrast",
});

choice_enum!(StartScreen {
    Sources => "sources",
    Events => "events",
    Relay => "relay",
    Stats => "stats",
    Last => "last",
});

choice_enum!(TimeFormat {
    Local => "local",
    Utc => "utc",
    Relative => "relative",
});

choice_enum!(ClipboardMode {
    Osc52 => "osc52",
    Off => "off",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Accepts exactly `#rrggbb`.
    pub fn parse_hex(raw: &str) -> Option<Self> {
        let digits = raw.strip_prefix('#')?;
        if digits.len() != 6
            || !digits
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return None;
        }
        let channel = |offset: usize| u8::from_str_radix(&digits[offset..offset + 2], 16).ok();
        Some(Self(channel(0)?, channel(2)?, channel(4)?))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

pub const DEFAULT_ACCENT: Rgb = Rgb(0x7c, 0x5c, 0xff);
pub const DEFAULT_RELAY_URL: &str = "http://localhost:3000";
pub const DEFAULT_BUDGET_PERCENT: u8 = 50;
pub const BUDGET_PERCENT_RANGE: std::ops::RangeInclusive<u8> = 10..=90;

/// An absolute http(s) URL, as required for the relay target and the server.
pub fn is_http_url(raw: &str) -> bool {
    (raw.starts_with("http://") || raw.starts_with("https://")) && reqwest::Url::parse(raw).is_ok()
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiSettings {
    pub open_on_bare_command: bool,
    pub theme: ThemeChoice,
    pub accent: Rgb,
    pub ascii: bool,
    pub start_screen: StartScreen,
    pub time_format: TimeFormat,
    pub request_budget_percent: u8,
    pub relay_default_url: String,
    pub clipboard: ClipboardMode,
    pub compact_header: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            open_on_bare_command: true,
            theme: ThemeChoice::Auto,
            accent: DEFAULT_ACCENT,
            ascii: false,
            start_screen: StartScreen::Sources,
            time_format: TimeFormat::Local,
            request_budget_percent: DEFAULT_BUDGET_PERCENT,
            relay_default_url: DEFAULT_RELAY_URL.to_string(),
            clipboard: ClipboardMode::Osc52,
            compact_header: false,
        }
    }
}

impl UiSettings {
    pub fn from_section(section: &UiSection) -> (Self, Vec<String>) {
        let defaults = Self::default();
        let mut warnings = Vec::new();

        let accent = match section.accent.as_deref() {
            None => defaults.accent,
            Some(raw) => Rgb::parse_hex(raw).unwrap_or_else(|| {
                warnings.push(format!(
                    "ui.accent = \"{raw}\" is not a #rrggbb color; using {}",
                    DEFAULT_ACCENT.to_hex()
                ));
                defaults.accent
            }),
        };
        let request_budget_percent = match section.request_budget_percent {
            None => defaults.request_budget_percent,
            Some(percent) => u8::try_from(percent)
                .ok()
                .filter(|percent| BUDGET_PERCENT_RANGE.contains(percent))
                .unwrap_or_else(|| {
                    warnings.push(format!(
                        "ui.request_budget_percent = {percent} is outside 10..=90; using {DEFAULT_BUDGET_PERCENT}"
                    ));
                    DEFAULT_BUDGET_PERCENT
                }),
        };
        let relay_default_url = match section.relay_default_url.as_deref() {
            None => defaults.relay_default_url.clone(),
            Some(raw) if is_http_url(raw) => raw.to_string(),
            Some(raw) => {
                warnings.push(format!(
                    "ui.relay_default_url = \"{raw}\" is not an http(s) URL; using {DEFAULT_RELAY_URL}"
                ));
                defaults.relay_default_url.clone()
            }
        };

        let settings = Self {
            open_on_bare_command: section
                .open_on_bare_command
                .unwrap_or(defaults.open_on_bare_command),
            theme: pick(
                section.theme.as_deref(),
                "theme",
                defaults.theme,
                &mut warnings,
            ),
            accent,
            ascii: section.ascii.unwrap_or(defaults.ascii),
            start_screen: pick(
                section.start_screen.as_deref(),
                "start_screen",
                defaults.start_screen,
                &mut warnings,
            ),
            time_format: pick(
                section.time_format.as_deref(),
                "time_format",
                defaults.time_format,
                &mut warnings,
            ),
            request_budget_percent,
            relay_default_url,
            clipboard: pick(
                section.clipboard.as_deref(),
                "clipboard",
                defaults.clipboard,
                &mut warnings,
            ),
            compact_header: section.compact_header.unwrap_or(defaults.compact_header),
        };
        (settings, warnings)
    }

    /// Every value is written explicitly, so the file documents itself.
    pub fn to_section(&self, state: UiState) -> UiSection {
        UiSection {
            open_on_bare_command: Some(self.open_on_bare_command),
            theme: Some(self.theme.label().to_string()),
            accent: Some(self.accent.to_hex()),
            ascii: Some(self.ascii),
            start_screen: Some(self.start_screen.label().to_string()),
            time_format: Some(self.time_format.label().to_string()),
            request_budget_percent: Some(i64::from(self.request_budget_percent)),
            relay_default_url: Some(self.relay_default_url.clone()),
            clipboard: Some(self.clipboard.label().to_string()),
            compact_header: Some(self.compact_header),
            state,
        }
    }
}

fn pick<T: Choice>(raw: Option<&str>, key: &str, default: T, warnings: &mut Vec<String>) -> T {
    let Some(raw) = raw else {
        return default;
    };
    T::parse(raw).unwrap_or_else(|| {
        let allowed: Vec<&str> = T::ALL.iter().map(|choice| choice.label()).collect();
        warnings.push(format!(
            "ui.{key} = \"{raw}\" is not one of {}; using {}",
            allowed.join(", "),
            default.label()
        ));
        default
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_section_means_defaults_without_warnings() {
        let (settings, warnings) = UiSettings::from_section(&UiSection::default());
        assert_eq!(settings, UiSettings::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn valid_values_are_applied() {
        let section = UiSection {
            open_on_bare_command: Some(false),
            theme: Some("light".into()),
            accent: Some("#112233".into()),
            ascii: Some(true),
            start_screen: Some("last".into()),
            time_format: Some("utc".into()),
            request_budget_percent: Some(20),
            relay_default_url: Some("http://localhost:4000".into()),
            clipboard: Some("off".into()),
            compact_header: Some(true),
            state: UiState::default(),
        };
        let (settings, warnings) = UiSettings::from_section(&section);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!settings.open_on_bare_command);
        assert_eq!(settings.theme, ThemeChoice::Light);
        assert_eq!(settings.accent, Rgb(0x11, 0x22, 0x33));
        assert!(settings.ascii);
        assert_eq!(settings.start_screen, StartScreen::Last);
        assert_eq!(settings.time_format, TimeFormat::Utc);
        assert_eq!(settings.request_budget_percent, 20);
        assert_eq!(settings.relay_default_url, "http://localhost:4000");
        assert_eq!(settings.clipboard, ClipboardMode::Off);
        assert!(settings.compact_header);
    }

    #[test]
    fn invalid_values_fall_back_with_one_warning_each() {
        let section = UiSection {
            theme: Some("neon".into()),
            accent: Some("purple".into()),
            request_budget_percent: Some(95),
            relay_default_url: Some("localhost:3000".into()),
            time_format: Some("iso".into()),
            ..UiSection::default()
        };
        let (settings, warnings) = UiSettings::from_section(&section);
        assert_eq!(settings, UiSettings::default());
        assert_eq!(warnings.len(), 5, "{warnings:?}");
        assert!(warnings.iter().any(|warning| warning.contains("ui.theme")));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("ui.request_budget_percent = 95")));
    }

    #[test]
    fn to_section_round_trips_and_keeps_the_state() {
        let settings = UiSettings {
            theme: ThemeChoice::HighContrast,
            request_budget_percent: 30,
            ..UiSettings::default()
        };
        let state = UiState {
            last_relay_url: Some("http://localhost:4000".into()),
            last_screen: Some("stats".into()),
        };
        let section = settings.to_section(state.clone());
        assert_eq!(section.state, state);
        assert_eq!(UiSettings::from_section(&section), (settings, vec![]));
    }

    #[test]
    fn choices_cycle_in_both_directions() {
        assert_eq!(ThemeChoice::HighContrast.next(), ThemeChoice::Auto);
        assert_eq!(ThemeChoice::Auto.previous(), ThemeChoice::HighContrast);
        assert_eq!(TimeFormat::parse("relative"), Some(TimeFormat::Relative));
        assert_eq!(TimeFormat::parse("Relative"), None);
    }

    #[test]
    fn hex_colors_parse_strictly() {
        assert_eq!(Rgb::parse_hex("#7c5cff"), Some(DEFAULT_ACCENT));
        assert_eq!(Rgb::parse_hex("7c5cff"), None);
        assert_eq!(Rgb::parse_hex("#7c5cf"), None);
        assert_eq!(Rgb::parse_hex("#gggggg"), None);
        assert_eq!(DEFAULT_ACCENT.to_hex(), "#7c5cff");
    }
}
