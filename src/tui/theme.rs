//! Palettes, color capability detection, glyphs with ASCII fallbacks, and the
//! logo. Views ask the theme for styles by `Tone`, never for raw colors.

use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;

pub use crate::tui::settings::Rgb;
use crate::tui::settings::{ThemeChoice, UiSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSupport {
    None,
    Ansi256,
    TrueColor,
}

/// The environment variables that shape rendering, read once at startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalEnv {
    pub no_color: bool,
    pub truecolor: bool,
    pub force_ascii: bool,
    pub dumb: bool,
    pub light_background: Option<bool>,
}

impl TerminalEnv {
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            no_color: lookup("NO_COLOR").is_some_and(|value| !value.is_empty()),
            truecolor: matches!(lookup("COLORTERM").as_deref(), Some("truecolor" | "24bit")),
            force_ascii: lookup("WHK_ASCII").as_deref() == Some("1"),
            dumb: lookup("TERM").as_deref() == Some("dumb"),
            light_background: lookup("COLORFGBG").and_then(|value| background_is_light(&value)),
        }
    }

    pub fn color_support(&self) -> ColorSupport {
        if self.no_color || self.dumb {
            ColorSupport::None
        } else if self.truecolor {
            ColorSupport::TrueColor
        } else {
            ColorSupport::Ansi256
        }
    }
}

/// `COLORFGBG` is "foreground;background" in ANSI color numbers; 7 and 15
/// are the light grays and white.
fn background_is_light(colorfgbg: &str) -> Option<bool> {
    let background: u8 = colorfgbg.rsplit(';').next()?.trim().parse().ok()?;
    Some(matches!(background, 7 | 15))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Text,
    Muted,
    Border,
    Accent,
    Success,
    Warning,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub text: Rgb,
    pub muted: Rgb,
    pub border: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub danger: Rgb,
    pub selection: Rgb,
}

pub const DARK: Palette = Palette {
    text: Rgb(0xe6, 0xe6, 0xe6),
    muted: Rgb(0x8a, 0x8f, 0x98),
    border: Rgb(0x3a, 0x3f, 0x4b),
    success: Rgb(0x4a, 0xde, 0x80),
    warning: Rgb(0xfa, 0xcc, 0x15),
    danger: Rgb(0xf8, 0x71, 0x71),
    selection: Rgb(0x2a, 0x2f, 0x3a),
};

pub const LIGHT: Palette = Palette {
    text: Rgb(0x1f, 0x23, 0x28),
    muted: Rgb(0x6e, 0x77, 0x81),
    border: Rgb(0xd0, 0xd7, 0xde),
    success: Rgb(0x1a, 0x7f, 0x37),
    warning: Rgb(0x9a, 0x67, 0x00),
    danger: Rgb(0xcf, 0x22, 0x2e),
    selection: Rgb(0xea, 0xee, 0xf2),
};

pub const HIGH_CONTRAST: Palette = Palette {
    text: Rgb(0xff, 0xff, 0xff),
    muted: Rgb(0xd0, 0xd0, 0xd0),
    border: Rgb(0xff, 0xff, 0xff),
    success: Rgb(0x00, 0xff, 0x00),
    warning: Rgb(0xff, 0xff, 0x00),
    danger: Rgb(0xff, 0x40, 0x40),
    selection: Rgb(0x00, 0x00, 0x80),
};

#[derive(Debug)]
pub struct Glyphs {
    pub active: &'static str,
    pub inactive: &'static str,
    pub paused: &'static str,
    pub success: &'static str,
    pub failure: &'static str,
    pub skipped: &'static str,
    pub filtered: &'static str,
    pub circuit: &'static str,
    pub relay: &'static str,
    pub pointer: &'static str,
    pub arrow: &'static str,
    pub separator: &'static str,
    pub ellipsis: &'static str,
    pub unknown: &'static str,
    pub previous: &'static str,
    pub next: &'static str,
    pub mask: char,
    pub spinner: &'static [&'static str],
}

impl Glyphs {
    pub fn spinner_frame(&self, tick: usize) -> &'static str {
        self.spinner[tick % self.spinner.len()]
    }
}

pub static UNICODE_GLYPHS: Glyphs = Glyphs {
    active: "●",
    inactive: "○",
    paused: "◐",
    success: "✓",
    failure: "✕",
    skipped: "–",
    filtered: "⊘",
    circuit: "⚡",
    relay: "⇄",
    pointer: "▸",
    arrow: "→",
    separator: "·",
    ellipsis: "…",
    unknown: "?",
    previous: "‹",
    next: "›",
    mask: '•',
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
};

pub static ASCII_GLYPHS: Glyphs = Glyphs {
    active: "*",
    inactive: "o",
    paused: "-",
    success: "v",
    failure: "x",
    skipped: "-",
    filtered: "/",
    circuit: "!",
    relay: "<>",
    pointer: ">",
    arrow: "->",
    separator: "|",
    ellipsis: "...",
    unknown: "?",
    previous: "<",
    next: ">",
    mask: '*',
    spinner: &["-", "\\", "|", "/"],
};

pub const UNICODE_LOGO: [&str; 2] = ["▄▀▀▄ █▄ ▄█", "▀▄▄▀ █ ▀ █"];
pub const ASCII_LOGO: [&str; 2] = ["|  | |_| |/", "|/\\| | | |\\"];

pub const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

#[derive(Debug, Clone)]
pub struct Theme {
    pub palette: Palette,
    pub accent: Rgb,
    pub support: ColorSupport,
    pub ascii: bool,
    pub glyphs: &'static Glyphs,
    pub logo: [&'static str; 2],
}

impl Theme {
    /// Environment wins over settings: `NO_COLOR`, `WHK_ASCII=1` and
    /// `TERM=dumb` apply even when the config says otherwise.
    pub fn new(settings: &UiSettings, env: &TerminalEnv) -> Self {
        let ascii = settings.ascii || env.force_ascii || env.dumb;
        let palette = match settings.theme {
            ThemeChoice::Dark => DARK,
            ThemeChoice::Light => LIGHT,
            ThemeChoice::HighContrast => HIGH_CONTRAST,
            ThemeChoice::Auto if env.light_background == Some(true) => LIGHT,
            ThemeChoice::Auto => DARK,
        };
        Self {
            palette,
            accent: settings.accent,
            support: env.color_support(),
            ascii,
            glyphs: if ascii {
                &ASCII_GLYPHS
            } else {
                &UNICODE_GLYPHS
            },
            logo: if ascii { ASCII_LOGO } else { UNICODE_LOGO },
        }
    }

    pub fn color(&self, rgb: Rgb) -> Color {
        match self.support {
            ColorSupport::None => Color::Reset,
            ColorSupport::TrueColor => Color::Rgb(rgb.0, rgb.1, rgb.2),
            ColorSupport::Ansi256 => Color::Indexed(nearest_ansi256(rgb)),
        }
    }

    fn rgb(&self, tone: Tone) -> Rgb {
        match tone {
            Tone::Text => self.palette.text,
            Tone::Muted => self.palette.muted,
            Tone::Border => self.palette.border,
            Tone::Accent => self.accent,
            Tone::Success => self.palette.success,
            Tone::Warning => self.palette.warning,
            Tone::Danger => self.palette.danger,
        }
    }

    /// Without colors, emphasis falls back to bold and dim.
    pub fn fg(&self, tone: Tone) -> Style {
        match self.support {
            ColorSupport::None => match tone {
                Tone::Accent | Tone::Danger => Style::default().add_modifier(Modifier::BOLD),
                Tone::Muted | Tone::Border => Style::default().add_modifier(Modifier::DIM),
                _ => Style::default(),
            },
            _ => Style::default().fg(self.color(self.rgb(tone))),
        }
    }

    pub fn title(&self) -> Style {
        self.fg(Tone::Accent).add_modifier(Modifier::BOLD)
    }

    pub fn selected(&self) -> Style {
        match self.support {
            ColorSupport::None => Style::default().add_modifier(Modifier::REVERSED),
            _ => Style::default()
                .bg(self.color(self.palette.selection))
                .add_modifier(Modifier::BOLD),
        }
    }

    pub fn border_set(&self) -> border::Set<'static> {
        if self.ascii {
            ASCII_BORDER
        } else {
            border::PLAIN
        }
    }
}

/// Nearest xterm-256 index: the closer of the 6×6×6 cube and the gray ramp.
pub fn nearest_ansi256(color: Rgb) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    fn level(channel: u8) -> usize {
        match channel {
            0..=47 => 0,
            48..=114 => 1,
            _ => usize::from((channel - 35) / 40),
        }
    }
    let (red_level, green_level, blue_level) = (level(color.0), level(color.1), level(color.2));
    let cube = Rgb(LEVELS[red_level], LEVELS[green_level], LEVELS[blue_level]);
    let cube_index = 16 + 36 * red_level + 6 * green_level + blue_level;

    let average = (u16::from(color.0) + u16::from(color.1) + u16::from(color.2)) / 3;
    let gray_step = if average < 8 {
        0
    } else {
        ((average - 8) / 10).min(23)
    };
    let gray_value = (8 + 10 * gray_step) as u8;
    let gray = Rgb(gray_value, gray_value, gray_value);

    if distance(color, gray) < distance(color, cube) {
        (232 + gray_step) as u8
    } else {
        cube_index as u8
    }
}

fn distance(first: Rgb, second: Rgb) -> u32 {
    let channel = |one: u8, other: u8| (i32::from(one) - i32::from(other)).pow(2) as u32;
    channel(first.0, second.0) + channel(first.1, second.1) + channel(first.2, second.2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::settings::{ThemeChoice, UiSettings};

    fn env_from(pairs: &[(&str, &str)]) -> TerminalEnv {
        TerminalEnv::from_lookup(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
    }

    #[test]
    fn color_support_follows_colorterm_no_color_and_term() {
        assert_eq!(env_from(&[]).color_support(), ColorSupport::Ansi256);
        assert_eq!(
            env_from(&[("COLORTERM", "truecolor")]).color_support(),
            ColorSupport::TrueColor
        );
        assert_eq!(
            env_from(&[("COLORTERM", "24bit")]).color_support(),
            ColorSupport::TrueColor
        );
        assert_eq!(
            env_from(&[("COLORTERM", "truecolor"), ("NO_COLOR", "1")]).color_support(),
            ColorSupport::None
        );
        assert_eq!(
            env_from(&[("NO_COLOR", "")]).color_support(),
            ColorSupport::Ansi256
        );
        assert_eq!(
            env_from(&[("TERM", "dumb")]).color_support(),
            ColorSupport::None
        );
    }

    #[test]
    fn colorfgbg_picks_the_auto_palette() {
        assert_eq!(
            env_from(&[("COLORFGBG", "15;0")]).light_background,
            Some(false)
        );
        assert_eq!(
            env_from(&[("COLORFGBG", "0;15")]).light_background,
            Some(true)
        );
        assert_eq!(env_from(&[("COLORFGBG", "garbage")]).light_background, None);
        let light = Theme::new(&UiSettings::default(), &env_from(&[("COLORFGBG", "0;15")]));
        assert_eq!(light.palette, LIGHT);
        let dark = Theme::new(&UiSettings::default(), &env_from(&[]));
        assert_eq!(dark.palette, DARK);
        let forced = Theme::new(
            &UiSettings {
                theme: ThemeChoice::HighContrast,
                ..UiSettings::default()
            },
            &env_from(&[("COLORFGBG", "0;15")]),
        );
        assert_eq!(forced.palette, HIGH_CONTRAST);
    }

    #[test]
    fn ascii_is_forced_by_the_environment_or_a_dumb_terminal() {
        let settings = UiSettings::default();
        assert!(!Theme::new(&settings, &env_from(&[])).ascii);
        let forced = Theme::new(&settings, &env_from(&[("WHK_ASCII", "1")]));
        assert!(forced.ascii);
        assert_eq!(forced.logo, ASCII_LOGO);
        assert_eq!(forced.glyphs.active, "*");
        assert!(Theme::new(&settings, &env_from(&[("TERM", "dumb")])).ascii);
        let chosen = Theme::new(
            &UiSettings {
                ascii: true,
                ..UiSettings::default()
            },
            &env_from(&[]),
        );
        assert!(chosen.ascii);
    }

    #[test]
    fn nearest_ansi256_maps_the_cube_and_the_grays() {
        assert_eq!(nearest_ansi256(Rgb(0, 0, 0)), 16);
        assert_eq!(nearest_ansi256(Rgb(255, 255, 255)), 231);
        assert_eq!(nearest_ansi256(Rgb(128, 128, 128)), 244);
        assert_eq!(nearest_ansi256(Rgb(0x7c, 0x5c, 0xff)), 99);
    }

    #[test]
    fn without_colors_styles_carry_no_color() {
        let theme = Theme::new(&UiSettings::default(), &env_from(&[("NO_COLOR", "1")]));
        assert_eq!(theme.fg(Tone::Danger).fg, None);
        assert!(theme.selected().add_modifier.contains(Modifier::REVERSED));
        let colored = Theme::new(
            &UiSettings::default(),
            &env_from(&[("COLORTERM", "truecolor")]),
        );
        assert_eq!(
            colored.fg(Tone::Accent).fg,
            Some(Color::Rgb(0x7c, 0x5c, 0xff))
        );
    }
}
