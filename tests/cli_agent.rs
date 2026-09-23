//! Runs the real `whk` binary the way an agent or CI job would: no TTY, stdin
//! closed, config directory isolated.

use std::process::{Output, Stdio};

use wiremock::MockServer;

async fn run_whk(server_url: &str, args: &[&str], secret: Option<&str>) -> Output {
    let config_home = tempfile::tempdir().unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_whk"));
    command
        .args(["--server", server_url, "--api-key", "whk_test"])
        .args(args)
        .env("HOME", config_home.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .env("APPDATA", config_home.path())
        .env_remove("WEBHOOKER_SERVER")
        .env_remove("WEBHOOKER_API_KEY")
        .env_remove("WHK_SECRET")
        .stdin(Stdio::null());
    if let Some(secret) = secret {
        command.env("WHK_SECRET", secret);
    }
    command.output().await.unwrap()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[tokio::test]
async fn a_preset_secret_without_a_tty_fails_fast_instead_of_blocking() {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        run_whk(
            "http://127.0.0.1:9",
            &["sources", "create", "stripe", "--verify", "stripe"],
            None,
        ),
    )
    .await
    .expect("whk must not wait for a prompt");
    assert!(!output.status.success());
    assert!(
        stderr_of(&output).contains("no TTY to prompt for the secret"),
        "{}",
        stderr_of(&output)
    );
}

#[tokio::test]
async fn whk_secret_supplies_the_preset_secret() {
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/sources/"))
        .and(body_json(serde_json::json!({
            "name": "stripe",
            "verification_config": {"provider": "stripe", "secret": "whsec_env"}
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "0198c9f0-0000-7000-8000-00000000000a",
            "name": "stripe"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let output = run_whk(
        &server.uri(),
        &["sources", "create", "stripe", "--verify", "stripe"],
        Some("whsec_env"),
    )
    .await;
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("stripe"));
}
