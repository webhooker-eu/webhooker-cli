use std::io::IsTerminal;

use anyhow::{bail, Context, Result};

/// Reads a JSON argument supplied inline, as `@path/to/file.json`, or as `-`
/// (stdin). Files and stdin keep secrets out of the shell history.
pub fn parse_json_arg(raw: &str) -> Result<serde_json::Value> {
    let (source, text) = match raw {
        "-" => {
            let mut buffer = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)
                .context("failed to read JSON from stdin")?;
            ("stdin".to_string(), buffer)
        }
        _ => match raw.strip_prefix('@') {
            Some(path) => (
                path.to_string(),
                std::fs::read_to_string(path)
                    .with_context(|| format!("failed to read JSON from {path}"))?,
            ),
            None => ("the --flag value".to_string(), raw.to_string()),
        },
    };
    // The parse error carries a line and column but never the text itself: the
    // file or pipe this came from holds a secret, and an error message ends up
    // in terminal scrollback and CI logs.
    serde_json::from_str(&text).with_context(|| format!("invalid JSON in {source}"))
}

/// Preset providers that need nothing but a secret, so the CLI can accept
/// `--verify stripe` and ask for the secret instead of demanding full JSON.
const PRESET_PROVIDERS: [&str; 3] = ["stripe", "github", "shopify"];

/// Builds a `verification_config` payload from a bare preset provider name (the
/// secret comes from `read_secret`, never from argv), the literal `none` to turn
/// verification off, or a full JSON config for the variants that carry more than
/// a secret.
pub fn parse_verification_arg(
    raw: &str,
    read_secret: impl FnOnce() -> Result<String>,
) -> Result<serde_json::Value> {
    if raw == "none" {
        return Ok(serde_json::json!({"provider": "none"}));
    }
    if PRESET_PROVIDERS.contains(&raw) {
        let secret = read_secret()?;
        if secret.trim().is_empty() {
            bail!("the signing secret must not be empty");
        }
        return Ok(serde_json::json!({"provider": raw, "secret": secret}));
    }
    let config = parse_json_arg(raw)?;
    if config.get("provider").is_none() {
        bail!(
            "verification config needs a \"provider\" field, or pass one of: {}, none",
            PRESET_PROVIDERS.join(", ")
        );
    }
    Ok(config)
}

/// Reads a secret without ever blocking an agent: a terminal gets a hidden
/// prompt; otherwise the secret comes from `WHK_SECRET` or one line of stdin.
pub fn read_secret(prompt: &str) -> Result<String> {
    let stdin = std::io::stdin();
    resolve_secret(
        stdin.is_terminal(),
        std::env::var("WHK_SECRET").ok(),
        || rpassword::prompt_password(prompt),
        || {
            let mut line = String::new();
            std::io::BufRead::read_line(&mut stdin.lock(), &mut line)?;
            Ok(line)
        },
    )
}

fn resolve_secret(
    stdin_is_terminal: bool,
    env_secret: Option<String>,
    prompt: impl FnOnce() -> std::io::Result<String>,
    read_line: impl FnOnce() -> std::io::Result<String>,
) -> Result<String> {
    if stdin_is_terminal {
        return Ok(prompt()?);
    }
    if let Some(secret) = env_secret.filter(|secret| !secret.is_empty()) {
        return Ok(secret);
    }
    let line = read_line().context("failed to read the secret from stdin")?;
    let secret = line.trim_end_matches(['\r', '\n']).to_string();
    if secret.is_empty() {
        bail!("no TTY to prompt for the secret; set WHK_SECRET or pipe it on stdin");
    }
    Ok(secret)
}

/// Builds a query string from the params that were actually supplied, so an
/// absent filter never reaches the API as an empty value.
pub fn query_string(params: &[(&str, Option<String>)]) -> String {
    let parts: Vec<String> = params
        .iter()
        .filter_map(|(key, value)| {
            value
                .as_ref()
                .map(|value| format!("{key}={}", urlencode(value)))
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// Percent-encodes the characters that would break a query string. Filters and
/// search terms are the only user input that reaches a URL, so a full encoder
/// is overkill.
fn urlencode(raw: &str) -> String {
    raw.chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => character.to_string(),
            other => other
                .to_string()
                .bytes()
                .map(|byte| format!("%{byte:02X}"))
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_json_is_parsed_as_is() {
        let value = parse_json_arg(r#"{"provider":"stripe","secret":"whsec"}"#).unwrap();
        assert_eq!(value["provider"], "stripe");
    }

    #[test]
    fn at_prefix_reads_the_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), r#"{"max_attempts":3}"#).unwrap();
        let value = parse_json_arg(&format!("@{}", file.path().display())).unwrap();
        assert_eq!(value["max_attempts"], 3);
    }

    #[test]
    fn malformed_json_never_echoes_the_input() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), r#"{"secret": "whsec_live",}"#).unwrap();
        let error = parse_json_arg(&format!("@{}", file.path().display())).unwrap_err();
        let rendered = format!("{error:#}");
        assert!(!rendered.contains("whsec_live"), "{rendered}");
        assert!(
            rendered.contains(&file.path().display().to_string()),
            "{rendered}"
        );
    }

    #[test]
    fn verify_none_turns_verification_off_without_a_prompt() {
        let value =
            parse_verification_arg("none", || unreachable!("secret must not be read")).unwrap();
        assert_eq!(value, serde_json::json!({"provider": "none"}));
    }

    #[test]
    fn preset_provider_takes_the_secret_from_the_reader_not_argv() {
        let value = parse_verification_arg("stripe", || Ok("whsec_live".to_string())).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"provider": "stripe", "secret": "whsec_live"})
        );
    }

    #[test]
    fn preset_provider_rejects_a_blank_secret() {
        assert!(parse_verification_arg("github", || Ok("  ".to_string())).is_err());
    }

    #[test]
    fn full_json_config_passes_through_untouched() {
        let raw = r#"{"provider":"generic_hmac","secret":"s","header":"X-Sig","algorithm":"sha256","encoding":"hex"}"#;
        let value =
            parse_verification_arg(raw, || unreachable!("secret must not be read")).unwrap();
        assert_eq!(value["header"], "X-Sig");
    }

    #[test]
    fn json_config_without_a_provider_is_rejected() {
        let error = parse_verification_arg(r#"{"secret":"s"}"#, || unreachable!()).unwrap_err();
        assert!(error.to_string().contains("provider"), "{error}");
    }

    #[test]
    fn query_string_skips_absent_params_and_encodes_values() {
        assert_eq!(query_string(&[("page", None), ("limit", None)]), "");
        assert_eq!(
            query_string(&[
                ("q", Some("a b&c".to_string())),
                ("page", Some("2".to_string()))
            ]),
            "?q=a%20b%26c&page=2"
        );
        // Timestamps carry colons and a plus sign, all of which must be encoded.
        assert_eq!(
            query_string(&[("received_after", Some("2026-09-20T10:00:00Z".to_string()))]),
            "?received_after=2026-09-20T10%3A00%3A00Z"
        );
    }

    fn no_prompt() -> std::io::Result<String> {
        unreachable!("the terminal prompt must not be used")
    }

    fn no_stdin() -> std::io::Result<String> {
        unreachable!("stdin must not be read")
    }

    #[test]
    fn a_terminal_gets_the_hidden_prompt() {
        let secret = resolve_secret(
            true,
            Some("ignored".into()),
            || Ok("typed".into()),
            no_stdin,
        )
        .unwrap();
        assert_eq!(secret, "typed");
    }

    #[test]
    fn without_a_terminal_the_environment_wins() {
        let secret = resolve_secret(false, Some("from-env".into()), no_prompt, no_stdin).unwrap();
        assert_eq!(secret, "from-env");
    }

    #[test]
    fn without_a_terminal_one_stdin_line_is_read() {
        let secret =
            resolve_secret(false, None, no_prompt, || Ok("piped-secret\r\n".into())).unwrap();
        assert_eq!(secret, "piped-secret");
    }

    #[test]
    fn without_a_terminal_env_or_stdin_it_fails_fast() {
        let error = resolve_secret(false, Some(String::new()), no_prompt, || Ok(String::new()))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "no TTY to prompt for the secret; set WHK_SECRET or pipe it on stdin"
        );
    }
}
