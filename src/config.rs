use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_SERVER: &str = "https://app.webhooker.eu";
/// Earlier releases defaulted to the landing host, which does not serve the API.
const LEGACY_DEFAULT_SERVER: &str = "https://webhooker.eu";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub server: String,
    /// Empty when only `[ui]` settings were saved; treated as "no key".
    #[serde(default)]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "UiSection::is_empty")]
    pub ui: UiSection,
}

impl Config {
    pub fn new(server: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            api_key: api_key.into(),
            ui: UiSection::default(),
        }
    }
}

/// The `[ui]` table as written on disk. Values stay raw here; the TUI applies
/// defaults and range checks, so an out-of-range value never makes the config
/// unreadable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_on_bare_command: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ascii: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_screen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_budget_percent: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_default_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_header: Option<bool>,
    #[serde(skip_serializing_if = "UiState::is_empty")]
    pub state: UiState,
}

impl UiSection {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Values the TUI remembers between runs, written on exit and on relay start.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_relay_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_screen: Option<String>,
}

impl UiState {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Platform config file: ~/.config/webhooker/config.toml on Linux,
/// ~/Library/Application Support/webhooker/config.toml on macOS,
/// %APPDATA%\webhooker\config.toml on Windows.
pub fn default_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("cannot resolve the user config directory")?;
    Ok(base.join("webhooker").join("config.toml"))
}

pub fn load(path: &Path) -> Result<Option<Config>> {
    match fs::read_to_string(path) {
        Ok(text) => {
            Ok(Some(toml::from_str(&text).with_context(|| {
                format!("invalid config at {}", path.display())
            })?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// Writes the config, never exposing the API key: on unix the file is created
/// with mode 0600 rather than being widened by the umask and narrowed after.
pub fn save(path: &Path, config: &Config) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    file.write_all(toml::to_string_pretty(config)?.as_bytes())?;
    #[cfg(unix)]
    {
        // `mode` applies to creation only, so tighten a file left loose by an
        // earlier version of the CLI.
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn delete(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Re-reads the file, applies `change` and writes it back, so a value another
/// process saved in the meantime survives. A missing file starts empty.
pub fn update(path: &Path, change: impl FnOnce(&mut Config)) -> Result<Config> {
    let mut config = load(path)?.unwrap_or_else(|| Config::new(DEFAULT_SERVER, ""));
    change(&mut config);
    save(path, &config)?;
    Ok(config)
}

/// Effective server: CLI flag > saved config > compiled-in default. A
/// self-hosted install must not be repointed at the default just because the
/// flag was omitted.
pub fn resolve_server(server_flag: Option<String>, saved: Option<&Config>) -> String {
    let saved_server = saved
        .map(|config| config.server.clone())
        .filter(|server| server.trim_end_matches('/') != LEGACY_DEFAULT_SERVER);
    server_flag
        .or(saved_server)
        .unwrap_or_else(|| DEFAULT_SERVER.to_string())
}

/// Effective credentials: CLI flag (clap already applied env vars, so
/// `flag > env` is handled upstream) > saved config > default server.
pub fn resolve(
    server_flag: Option<String>,
    api_key_flag: Option<String>,
    saved: Option<Config>,
) -> Result<Config> {
    let server = resolve_server(server_flag, saved.as_ref());
    let saved_key = saved
        .as_ref()
        .map(|config| config.api_key.clone())
        .filter(|key| !key.is_empty());
    let Some(api_key) = api_key_flag.or(saved_key) else {
        bail!("no API key: run `whk login`, pass --api-key, or set WEBHOOKER_API_KEY");
    };
    Ok(Config {
        server,
        api_key,
        ui: saved.map(|config| config.ui).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webhooker").join("config.toml");
        let config = Config::new("https://webhooker.eu", "whk_secret");
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), Some(config));
    }

    #[test]
    fn load_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&dir.path().join("nope.toml")).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_readable_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&path, &Config::new("s", "k")).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn saving_tightens_a_pre_existing_world_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "server = \"s\"\napi_key = \"old\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        save(&path, &Config::new("s", "k")).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn flag_beats_saved_config() {
        let saved = Some(Config::new("https://saved.example", "whk_saved"));
        let resolved = resolve(
            Some("https://flag.example".into()),
            Some("whk_flag".into()),
            saved,
        )
        .unwrap();
        assert_eq!(resolved.server, "https://flag.example");
        assert_eq!(resolved.api_key, "whk_flag");
    }

    #[test]
    fn saved_config_fills_missing_pieces() {
        let saved = Some(Config::new("https://saved.example", "whk_saved"));
        let resolved = resolve(None, None, saved).unwrap();
        assert_eq!(resolved.server, "https://saved.example");
        assert_eq!(resolved.api_key, "whk_saved");
    }

    #[test]
    fn missing_key_everywhere_is_an_error() {
        assert!(resolve(None, None, None).is_err());
    }

    #[test]
    fn server_falls_back_to_the_saved_one_before_the_default() {
        let saved = Config::new("https://hooks.internal", "whk_saved");
        assert_eq!(resolve_server(None, Some(&saved)), "https://hooks.internal");
        assert_eq!(
            resolve_server(Some("https://flag.example".into()), Some(&saved)),
            "https://flag.example"
        );
        assert_eq!(resolve_server(None, None), DEFAULT_SERVER);
    }

    #[test]
    fn saved_legacy_default_server_is_replaced_by_the_current_default() {
        for legacy in ["https://webhooker.eu", "https://webhooker.eu/"] {
            let saved = Config::new(legacy, "whk_saved");
            assert_eq!(resolve_server(None, Some(&saved)), DEFAULT_SERVER);
        }
    }

    #[test]
    fn a_file_without_a_ui_section_loads_with_an_empty_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "server = \"s\"\napi_key = \"k\"\n").unwrap();
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.ui, UiSection::default());
    }

    #[test]
    fn an_empty_ui_section_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&path, &Config::new("s", "k")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("[ui"), "{text}");
    }

    #[test]
    fn the_ui_section_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::new("s", "k");
        config.ui.theme = Some("light".into());
        config.ui.request_budget_percent = Some(40);
        config.ui.state.last_relay_url = Some("http://localhost:4000".into());
        save(&path, &config).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[ui]"), "{text}");
        assert!(text.contains("[ui.state]"), "{text}");
        assert_eq!(load(&path).unwrap(), Some(config));
    }

    #[test]
    fn update_keeps_a_value_another_process_saved_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&path, &Config::new("s", "k")).unwrap();

        // Another process changes the theme after this one loaded the file.
        let mut other = load(&path).unwrap().unwrap();
        other.ui.theme = Some("light".into());
        save(&path, &other).unwrap();

        update(&path, |config| {
            config.ui.state.last_relay_url = Some("http://localhost:4000".into())
        })
        .unwrap();
        let merged = load(&path).unwrap().unwrap();
        assert_eq!(merged.ui.theme.as_deref(), Some("light"));
        assert_eq!(
            merged.ui.state.last_relay_url.as_deref(),
            Some("http://localhost:4000")
        );
        assert_eq!(merged.api_key, "k");
    }

    #[test]
    fn update_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webhooker").join("config.toml");
        update(&path, |config| config.ui.theme = Some("dark".into())).unwrap();
        let created = load(&path).unwrap().unwrap();
        assert_eq!(created.server, DEFAULT_SERVER);
        assert_eq!(created.api_key, "");
        assert_eq!(created.ui.theme.as_deref(), Some("dark"));
    }

    #[test]
    fn an_empty_saved_key_counts_as_missing() {
        assert!(resolve(None, None, Some(Config::new("s", ""))).is_err());
    }

    #[test]
    fn resolve_carries_the_saved_ui_section() {
        let mut saved = Config::new("s", "k");
        saved.ui.ascii = Some(true);
        let resolved = resolve(None, Some("whk_flag".into()), Some(saved)).unwrap();
        assert_eq!(resolved.ui.ascii, Some(true));
    }
}
