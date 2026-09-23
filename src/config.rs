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
    pub api_key: String,
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
    let Some(api_key) = api_key_flag.or_else(|| saved.map(|config| config.api_key)) else {
        bail!("no API key: run `whk login`, pass --api-key, or set WEBHOOKER_API_KEY");
    };
    Ok(Config { server, api_key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webhooker").join("config.toml");
        let config = Config {
            server: "https://webhooker.eu".to_string(),
            api_key: "whk_secret".to_string(),
        };
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
        save(
            &path,
            &Config {
                server: "s".into(),
                api_key: "k".into(),
            },
        )
        .unwrap();
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
        save(
            &path,
            &Config {
                server: "s".into(),
                api_key: "k".into(),
            },
        )
        .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn flag_beats_saved_config() {
        let saved = Some(Config {
            server: "https://saved.example".into(),
            api_key: "whk_saved".into(),
        });
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
        let saved = Some(Config {
            server: "https://saved.example".into(),
            api_key: "whk_saved".into(),
        });
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
        let saved = Config {
            server: "https://hooks.internal".into(),
            api_key: "whk_saved".into(),
        };
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
            let saved = Config {
                server: legacy.to_string(),
                api_key: "whk_saved".to_string(),
            };
            assert_eq!(resolve_server(None, Some(&saved)), DEFAULT_SERVER);
        }
    }
}
