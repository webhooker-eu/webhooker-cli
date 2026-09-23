//! Deterministic app states for update, key and render tests.

use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use serde_json::json;

use crate::tui::app::{App, AppInit, Focus, KeySource};
use crate::tui::model::{Connection, Destination, Source, SourceConnection, Workspace};
use crate::tui::screen::{Screen, SourceTab};
use crate::tui::settings::{TimeFormat, UiSettings};
use crate::tui::theme::TerminalEnv;

pub const STRIPE_ID: &str = "0198c9f0-0000-7000-8000-00000000000a";
pub const GITHUB_ID: &str = "0198c9f0-0000-7000-8000-00000000000b";
pub const SHOPIFY_ID: &str = "0198c9f0-0000-7000-8000-00000000000c";
pub const BILLING_ID: &str = "0198c9f0-0000-7000-8000-0000000000d1";
pub const AUDIT_ID: &str = "0198c9f0-0000-7000-8000-0000000000d2";
pub const STRIPE_BILLING_ID: &str = "0198c9f0-0000-7000-8000-0000000000c1";
pub const STRIPE_AUDIT_ID: &str = "0198c9f0-0000-7000-8000-0000000000c2";
pub const GITHUB_BILLING_ID: &str = "0198c9f0-0000-7000-8000-0000000000c3";

pub fn settings() -> UiSettings {
    UiSettings {
        time_format: TimeFormat::Utc,
        ..UiSettings::default()
    }
}

pub fn truecolor() -> TerminalEnv {
    TerminalEnv {
        truecolor: true,
        ..TerminalEnv::default()
    }
}

pub fn init(has_key: bool) -> AppInit {
    AppInit {
        settings: settings(),
        env: truecolor(),
        server: "https://app.webhooker.eu".to_string(),
        key_source: KeySource::Config,
        has_key,
        last_screen: None,
        warnings: Vec::new(),
        size: (120, 40),
        now: Instant::now(),
        wall_clock: Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap(),
    }
}

pub fn source(id: &str, name: &str, events: i64, provider: &str, status: &str) -> Source {
    let verification = if provider == "none" {
        json!({"provider": "none"})
    } else {
        json!({"provider": provider, "secret": "***"})
    };
    serde_json::from_value(json!({
        "id": id,
        "name": name,
        "description": "Payments from Stripe",
        "color": "#3b82f6",
        "token": "6b225n04u5kmyg",
        "ingest_url": "https://app.webhooker.eu/in/6b225n04u5kmyg",
        "verification_config": verification,
        "response_config": null,
        "status": status,
        "created_at": "2026-09-01T10:00:00Z",
        "event_count": events
    }))
    .unwrap()
}

pub fn destination(id: &str, name: &str, url: &str, circuit_state: &str) -> Destination {
    let reopen_at = if circuit_state == "open" {
        json!("2026-09-23T12:05:00Z")
    } else {
        json!(null)
    };
    serde_json::from_value(json!({
        "id": id,
        "name": name,
        "url": url,
        "custom_headers": {"X-Env": "prod"},
        "auth_config": {"type": "hmac", "secret": "***"},
        "timeout_ms": 10000,
        "retry_policy": {"intervals_seconds": [30, 120, 600, 3600, 14400]},
        "status": "active",
        "circuit_state": circuit_state,
        "circuit_reopen_at": reopen_at,
        "created_at": "2026-09-01T10:00:00Z"
    }))
    .unwrap()
}

pub fn connection(id: &str, source_id: &str, destination_id: &str, enabled: bool) -> Connection {
    let filter_rules = if id == STRIPE_BILLING_ID {
        json!({"all": [{"path": "type", "op": "eq", "value": "invoice.paid"}]})
    } else {
        json!(null)
    };
    serde_json::from_value(json!({
        "id": id,
        "source_id": source_id,
        "destination_id": destination_id,
        "filter_rules": filter_rules,
        "transformation": null,
        "enabled": enabled,
        "created_at": "2026-09-01T10:00:00Z"
    }))
    .unwrap()
}

pub fn source_connection(
    id: &str,
    destination_id: &str,
    name: &str,
    url: &str,
    circuit_state: &str,
) -> SourceConnection {
    serde_json::from_value(json!({
        "id": id,
        "source_id": STRIPE_ID,
        "destination_id": destination_id,
        "enabled": true,
        "destination": {"id": destination_id, "name": name, "url": url, "circuit_state": circuit_state}
    }))
    .unwrap()
}

pub fn loaded_at(app: &App) -> Instant {
    app.now
        .checked_sub(Duration::from_secs(3))
        .unwrap_or(app.now)
}

/// Logged in to "acme" on the Pro plan, on the Sources list, every list
/// loaded 3 s ago.
pub fn app() -> App {
    let mut app = App::new(init(true));
    let loaded = loaded_at(&app);
    app.session.workspace = Some(Workspace {
        id: "0198c9f0-0000-7000-8000-0000000000aa".into(),
        name: "acme".into(),
        plan: "pro".into(),
    });
    app.screen = Screen::Sources;
    app.focus = Focus::Main;
    app.data.sources.finish(
        vec![
            source(STRIPE_ID, "stripe-prod", 1204, "stripe", "active"),
            source(GITHUB_ID, "github-ci", 318, "github", "active"),
            source(SHOPIFY_ID, "shopify-old", 12, "none", "paused"),
        ],
        loaded,
    );
    app.data.destinations.finish(
        vec![
            destination(
                BILLING_ID,
                "billing-worker",
                "https://billing.internal/hooks",
                "closed",
            ),
            destination(
                AUDIT_ID,
                "audit-log",
                "https://audit.example.com/in",
                "open",
            ),
        ],
        loaded,
    );
    app.data.connections.finish(
        vec![
            connection(STRIPE_BILLING_ID, STRIPE_ID, BILLING_ID, true),
            connection(STRIPE_AUDIT_ID, STRIPE_ID, AUDIT_ID, true),
            connection(GITHUB_BILLING_ID, GITHUB_ID, BILLING_ID, false),
        ],
        loaded,
    );
    app.names.remember_sources([
        (STRIPE_ID, "stripe-prod"),
        (GITHUB_ID, "github-ci"),
        (SHOPIFY_ID, "shopify-old"),
    ]);
    app.names
        .remember_destinations([(BILLING_ID, "billing-worker"), (AUDIT_ID, "audit-log")]);
    app
}

pub fn source_detail(tab: SourceTab) -> App {
    let mut app = app();
    let loaded = loaded_at(&app);
    app.history.push(Screen::Sources);
    app.screen = Screen::SourceDetail {
        id: STRIPE_ID.into(),
        tab,
    };
    app.data.source.finish(
        source(STRIPE_ID, "stripe-prod", 1204, "stripe", "active"),
        loaded,
    );
    app.data.source_connections.finish(
        vec![
            source_connection(
                STRIPE_BILLING_ID,
                BILLING_ID,
                "billing-worker",
                "https://billing.internal/hooks",
                "closed",
            ),
            source_connection(
                STRIPE_AUDIT_ID,
                AUDIT_ID,
                "audit-log",
                "https://audit.example.com/in",
                "open",
            ),
        ],
        loaded,
    );
    app
}

pub fn destination_detail() -> App {
    let mut app = app();
    let loaded = loaded_at(&app);
    app.history.push(Screen::Destinations);
    app.screen = Screen::DestinationDetail {
        id: BILLING_ID.into(),
    };
    app.data.destination.finish(
        destination(
            BILLING_ID,
            "billing-worker",
            "https://billing.internal/hooks",
            "closed",
        ),
        loaded,
    );
    app
}

pub fn connection_detail() -> App {
    let mut app = app();
    let loaded = loaded_at(&app);
    app.history.push(Screen::Connections);
    app.screen = Screen::ConnectionDetail {
        id: STRIPE_BILLING_ID.into(),
    };
    app.data.connection.finish(
        connection(STRIPE_BILLING_ID, STRIPE_ID, BILLING_ID, true),
        loaded,
    );
    app
}

pub const EVENT_ID: &str = "0198c9f0-0000-7000-8000-0000000000e1";

pub fn event_detail() -> crate::tui::model::EventDetail {
    serde_json::from_value(json!({
        "id": EVENT_ID,
        "public_id": "evt_8f2a1b",
        "source_id": STRIPE_ID,
        "method": "POST",
        "headers": {
            "content-type": "application/json",
            "stripe-signature": "t=1726000000,v1=5257a869",
            "user-agent": "Stripe/1.0"
        },
        "body": "{\"type\":\"invoice.paid\",\"amount\":1200,\"paid\":true,\"note\":null}",
        "body_size": 62,
        "content_type": "application/json",
        "verification_status": "verified",
        "received_at": "2026-09-23T12:04:11Z",
        "expires_at": "2026-10-23T12:04:11Z",
        "deliveries": [
            {
                "id": "0198c9f0-0000-7000-8000-0000000000f1",
                "connection_id": STRIPE_BILLING_ID,
                "destination_name": "billing-worker",
                "status": "succeeded",
                "attempt_count": 1,
                "next_attempt_at": "2026-09-23T12:04:11Z",
                "created_at": "2026-09-23T12:04:11Z",
                "attempts": [{
                    "attempt_number": 1,
                    "request_url": "https://billing.internal/hooks",
                    "response_status": 200,
                    "response_body": "ok",
                    "error_message": null,
                    "latency_ms": 34,
                    "attempted_at": "2026-09-23T12:04:12Z"
                }]
            },
            {
                "id": "0198c9f0-0000-7000-8000-0000000000f2",
                "connection_id": STRIPE_AUDIT_ID,
                "destination_name": "audit-log",
                "status": "exhausted",
                "attempt_count": 2,
                "next_attempt_at": "2026-09-23T12:10:00Z",
                "created_at": "2026-09-23T12:04:11Z",
                "attempts": [
                    {
                        "attempt_number": 1,
                        "request_url": "https://audit.example.com/in",
                        "response_status": 500,
                        "response_body": "{\"error\":\"db timeout\"}",
                        "error_message": "HTTP 500",
                        "latency_ms": 120,
                        "attempted_at": "2026-09-23T12:04:12Z"
                    },
                    {
                        "attempt_number": 2,
                        "request_url": "https://audit.example.com/in",
                        "response_status": null,
                        "response_body": null,
                        "error_message": "connection refused",
                        "latency_ms": 3,
                        "attempted_at": "2026-09-23T12:06:12Z"
                    }
                ]
            }
        ]
    }))
    .unwrap()
}
