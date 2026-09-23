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

const SOURCE_ID: &str = "0198c9f0-0000-7000-8000-00000000000a";

async fn mount_sources(server: &MockServer) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};
    Mock::given(method("GET"))
        .and(path("/api/v1/sources/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [{"id": SOURCE_ID, "name": "stripe-prod", "token": "tok"}],
            "total": 1
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn dlq_summary_prints_a_table_and_raw_json() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    let server = MockServer::start().await;
    mount_sources(&server).await;
    let summary = serde_json::json!({"items": [{
        "connection_id": "c1",
        "destination_name": "billing",
        "exhausted_count": 4,
        "failed_count": 1
    }]});
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/sources/{SOURCE_ID}/dlq/summary")))
        .respond_with(ResponseTemplate::new(200).set_body_json(summary.clone()))
        .mount(&server)
        .await;

    let text = run_whk(&server.uri(), &["dlq", "summary", "stripe-prod"], None).await;
    assert!(text.status.success(), "{}", stderr_of(&text));
    assert_eq!(
        stdout_of(&text),
        "CONNECTION  DESTINATION  EXHAUSTED  FAILED\nc1          billing      4          1\n"
    );

    let json = run_whk(
        &server.uri(),
        &["--json", "dlq", "summary", "stripe-prod"],
        None,
    )
    .await;
    let parsed: serde_json::Value = serde_json::from_str(stdout_of(&json).trim()).unwrap();
    assert_eq!(parsed, summary);
}

#[tokio::test]
async fn dlq_ls_passes_the_status_filter() {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, ResponseTemplate};

    let server = MockServer::start().await;
    mount_sources(&server).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/sources/{SOURCE_ID}/dlq")))
        .and(query_param("status", "exhausted"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [], "total": 0
        })))
        .expect(1)
        .mount(&server)
        .await;

    let output = run_whk(
        &server.uri(),
        &["dlq", "ls", "stripe-prod", "--status", "exhausted"],
        None,
    )
    .await;
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "(none)\n");
}

#[tokio::test]
async fn dlq_resend_posts_a_bulk_resend() {
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/deliveries/resend-bulk"))
        .and(body_json(serde_json::json!({
            "connection_id": "c1",
            "statuses": ["exhausted", "failed"]
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({"created": 3})))
        .expect(1)
        .mount(&server)
        .await;

    let output = run_whk(
        &server.uri(),
        &[
            "dlq",
            "resend",
            "--connection",
            "c1",
            "--status",
            "exhausted,failed",
        ],
        None,
    )
    .await;
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(stdout_of(&output), "Queued 3 delivery(ies)\n");
}

#[tokio::test]
async fn stats_overview_resolves_sources_to_ids() {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, ResponseTemplate};

    let server = MockServer::start().await;
    mount_sources(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/stats/overview"))
        .and(query_param("source_ids", SOURCE_ID))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_events": 2,
            "events_per_bucket": [],
            "bucket_unit": "hour",
            "range_start": "2026-09-22T12:00:00Z",
            "range_end": "2026-09-23T12:00:00Z",
            "deliveries_by_status": [],
            "failed_attempts": 0,
            "e2e_latency_ms": {"p50_ms": null, "p95_ms": null, "p99_ms": null}
        })))
        .expect(2)
        .mount(&server)
        .await;

    let text = run_whk(
        &server.uri(),
        &["stats", "overview", "--source", "stripe-prod"],
        None,
    )
    .await;
    assert!(text.status.success(), "{}", stderr_of(&text));
    assert!(
        stdout_of(&text).contains("events:          2"),
        "{}",
        stdout_of(&text)
    );

    let json = run_whk(
        &server.uri(),
        &["--json", "stats", "overview", "--source", "stripe-prod"],
        None,
    )
    .await;
    let parsed: serde_json::Value = serde_json::from_str(stdout_of(&json).trim()).unwrap();
    assert_eq!(parsed["total_events"], 2);
}

#[tokio::test]
async fn stats_by_source_prints_the_volume_table() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/stats/volume-by-source"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [{"source_id": "s1", "name": "stripe-prod", "color": null, "count": 12}]
        })))
        .mount(&server)
        .await;

    let output = run_whk(&server.uri(), &["stats", "by-source"], None).await;
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output),
        "NAME         EVENTS  SOURCE ID\nstripe-prod  12      s1\n"
    );
}
