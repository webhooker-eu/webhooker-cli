//! Render snapshots at 80×24 and 120×40 plus targeted text checks. Snapshots
//! live in `src/tui/views/snapshots/`; review every new or changed `.snap`.

use std::time::Duration;

use insta::assert_snapshot;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::render;
use crate::tui::app::{App, Confirm, ConfirmAction};
use crate::tui::fixtures;
use crate::tui::forms::input::TextInput;
use crate::tui::screen::Screen;
use crate::tui::screen::Section;
use crate::tui::screen::SourceTab;
use crate::tui::settings::ThemeChoice;
use crate::tui::theme::Tone;

pub(super) fn draw(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal
}

pub(super) fn screen_text(app: &App, width: u16, height: u16) -> String {
    let terminal = draw(app, width, height);
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|row| {
            (0..width)
                .map(|column| buffer.cell((column, row)).map_or(" ", |cell| cell.symbol()))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn snapshot_both_sizes(name: &str, app: &App) {
    for (width, height) in [(80, 24), (120, 40)] {
        assert_snapshot!(
            format!("{name}_{width}x{height}"),
            draw(app, width, height).backend()
        );
    }
}

pub(super) fn ascii(mut app: App) -> App {
    app.env.force_ascii = true;
    app.apply_settings(app.settings.clone());
    app
}

/// Stats stays a placeholder until Plan 3 and has nothing to poll, so it
/// shows the chrome alone.
fn on_stats() -> App {
    let mut app = fixtures::app();
    app.switch_to(Section::Stats);
    app
}

#[test]
fn chrome_around_a_placeholder_section() {
    snapshot_both_sizes("chrome_stats", &on_stats());
}

#[test]
fn the_header_collapses_below_twenty_rows() {
    let app = on_stats();
    assert_snapshot!("chrome_compact_100x18", draw(&app, 100, 18).backend());
    let text = screen_text(&app, 100, 18);
    assert!(text.starts_with(" Webhooker v"), "{text}");
    assert!(text.lines().next().unwrap().contains("acme · pro"));
}

#[test]
fn too_small_terminals_ask_to_enlarge() {
    let app = fixtures::app();
    for (width, height) in [(59, 20), (80, 14)] {
        assert!(screen_text(&app, width, height).contains("Please enlarge the terminal window"));
    }
    assert!(!screen_text(&app, 60, 15).contains("Please enlarge"));
    assert_snapshot!("too_small_59x20", draw(&app, 59, 20).backend());
}

#[test]
fn help_overlay() {
    let mut app = on_stats();
    app.help_open = true;
    snapshot_both_sizes("help_stats", &app);
}

#[test]
fn confirm_modal_names_the_target() {
    let mut app = on_stats();
    app.confirm = Some(Confirm::about(
        "Rotate the ingest token of ",
        "stripe-prod",
        "?",
        ConfirmAction::DiscardSettings,
    ));
    assert_snapshot!("confirm_80x24", draw(&app, 80, 24).backend());
    assert!(screen_text(&app, 80, 24).contains("Rotate the ingest token of stripe-prod?"));
}

#[test]
fn the_status_line_reports_age_rate_limits_offline_and_toasts() {
    let mut app = fixtures::app();
    assert!(screen_text(&app, 120, 40).contains("3s ago"));
    app.rate_limited_until = Some(app.now + Duration::from_secs(23));
    assert!(screen_text(&app, 120, 40).contains("rate limited · resumes in 23s"));
    app.rate_limited_until = None;
    app.poller.network_failed(app.now);
    assert!(screen_text(&app, 120, 40).contains("offline"));
    app.toast("Settings saved", Tone::Success);
    assert!(screen_text(&app, 120, 40).contains("Settings saved"));
}

#[test]
fn an_override_key_is_marked_in_the_header() {
    let mut app = on_stats();
    app.session.key_source = crate::tui::app::KeySource::Override;
    assert!(screen_text(&app, 120, 40).contains("acme · pro · app.webhooker.eu · key: env"));
}

#[test]
fn the_sidebar_becomes_a_strip_under_100_columns() {
    let app = on_stats();
    let narrow = screen_text(&app, 99, 30);
    assert!(narrow.contains("Sources Dests Conns Events DLQ Stats Relay Settings"));
    let wide = screen_text(&app, 100, 30);
    assert!(wide.contains("Destinations"));
}

#[test]
fn ascii_mode_draws_only_ascii() {
    let app = ascii(on_stats());
    let text = screen_text(&app, 120, 40);
    assert!(text.is_ascii(), "{text}");
    assert!(text.contains("|  | |_| |/"));
}
fn login_app() -> App {
    let mut app = App::new(fixtures::init(false));
    app.start();
    app
}

#[test]
fn login_screen() {
    let app = login_app();
    snapshot_both_sizes("login", &app);
    let text = screen_text(&app, 80, 24);
    assert!(text.contains("API key"));
    assert!(text.contains("https://app.webhooker.eu"));
    assert!(!text.contains("Sources"), "no sidebar before login");
}

#[test]
fn login_masks_the_key_and_shows_errors_inline() {
    let mut app = login_app();
    app.login.api_key.set("whk_secret");
    app.login.error = Some(crate::tui::app::KEY_REJECTED_MESSAGE.to_string());
    let text = screen_text(&app, 80, 24);
    assert!(text.contains("••••••••••"));
    assert!(!text.contains("whk_secret"));
    assert!(text.contains("API key is invalid or revoked"));
}

#[test]
fn settings_screen() {
    let mut app = fixtures::app();
    app.switch_to(Section::Settings);
    snapshot_both_sizes("settings", &app);
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("Request budget (%)"));
    assert!(text.contains("‹ auto ›"));
}

#[test]
fn settings_show_unsaved_changes_and_errors() {
    let mut app = fixtures::app();
    app.switch_to(Section::Settings);
    let form = app.settings_form.as_mut().unwrap();
    form.draft.compact_header = true;
    assert!(screen_text(&app, 120, 40).contains("Unsaved changes"));
    app.settings_form.as_mut().unwrap().error = Some("disk full".into());
    assert!(screen_text(&app, 120, 40).contains("disk full"));
}

#[test]
fn sources_list() {
    let app = fixtures::app();
    snapshot_both_sizes("sources", &app);
    let text = screen_text(&app, 120, 40);
    for expected in [
        "stripe-prod",
        "1 204 events",
        "2 conns",
        "1 conn ",
        "verify: stripe",
        "paused",
        "enter open · / filter",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
}

#[test]
fn sources_list_in_every_theme() {
    let dark = fixtures::app();
    insta::assert_debug_snapshot!(
        "sources_dark_styles",
        draw(&dark, 80, 24).backend().buffer()
    );
    let mut light = fixtures::app();
    light.apply_settings(crate::tui::settings::UiSettings {
        theme: ThemeChoice::Light,
        ..light.settings.clone()
    });
    insta::assert_debug_snapshot!(
        "sources_light_styles",
        draw(&light, 80, 24).backend().buffer()
    );
    let ascii_app = ascii(fixtures::app());
    assert_snapshot!("sources_ascii_80x24", draw(&ascii_app, 80, 24).backend());
    assert!(screen_text(&ascii_app, 80, 24).is_ascii());
}

#[test]
fn sources_empty_filtering_and_search() {
    let mut app = fixtures::app();
    let loaded = fixtures::loaded_at(&app);
    app.data.sources.finish(Vec::new(), loaded);
    assert!(screen_text(&app, 80, 24).contains("No sources yet. Press n to create one."));
    app.source_query = Some("zzz".into());
    assert!(screen_text(&app, 80, 24).contains("No sources match \"zzz\"."));
    app.source_search = Some(TextInput::new("str", false));
    let text = screen_text(&app, 80, 24);
    assert!(text.contains("/str"));
    assert!(text.contains("enter apply filter · esc cancel"));
}

#[test]
fn source_overview() {
    let app = fixtures::source_detail(SourceTab::Overview);
    snapshot_both_sizes("source_overview", &app);
    let text = screen_text(&app, 120, 40);
    for expected in [
        "1 Overview",
        "5 DLQ",
        "https://app.webhooker.eu/in/6b225n04u5kmyg",
        "stripe (secret ***)",
        "#3b82f6",
        "2026-09-01 10:00:00",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
}

#[test]
fn source_connections_tab() {
    let app = fixtures::source_detail(SourceTab::Connections);
    snapshot_both_sizes("source_connections", &app);
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("billing-worker"));
    assert!(text.contains("billing.internal"));
    // ⚡ is two columns wide, so the buffer holds a padding cell after it.
    assert!(text.contains('⚡') && text.contains(" open"));
}

fn on(screen: Screen) -> App {
    let mut app = fixtures::app();
    app.screen = screen;
    app
}

#[test]
fn destinations_list() {
    let app = on(Screen::Destinations);
    snapshot_both_sizes("destinations", &app);
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("billing-worker"));
    assert!(text.contains("billing.internal"));
    // ⚡ is two columns wide, so the buffer holds a padding cell after it.
    assert!(text.contains('⚡') && text.contains(" open"));
}

#[test]
fn destination_detail() {
    let app = fixtures::destination_detail();
    snapshot_both_sizes("destination_detail", &app);
    let text = screen_text(&app, 120, 40);
    for expected in [
        "https://billing.internal/hooks",
        "X-Env: prod",
        "hmac (secret ***)",
        "10000 ms",
        "30s · 2m · 10m · 1h · 4h",
        "Fed by",
        "stripe-prod",
        "github-ci",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
}

#[test]
fn connections_list() {
    let app = on(Screen::Connections);
    snapshot_both_sizes("connections", &app);
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("stripe-prod → billing-worker"));
    assert!(text.contains("filter"));
}

#[test]
fn connection_detail() {
    let app = fixtures::connection_detail();
    snapshot_both_sizes("connection_detail", &app);
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("\"invoice.paid\""));
    assert!(text.contains("Transformation"));
    assert!(text.contains("none"));
}

#[test]
fn long_detail_panes_clamp_their_scroll() {
    let mut app = fixtures::connection_detail();
    app.scroll = 500;
    screen_text(&app, 80, 15);
    assert!(app.scroll_limit.get() < 500);
}

#[test]
fn every_screen_is_ascii_clean_in_ascii_mode() {
    let mut help = fixtures::app();
    help.help_open = true;
    let mut settings = fixtures::app();
    settings.switch_to(Section::Settings);
    let mut login = App::new(fixtures::init(false));
    login.start();
    login.login.api_key.set("whk_secret");
    let screens = [
        fixtures::app(),
        fixtures::source_detail(SourceTab::Overview),
        fixtures::source_detail(SourceTab::Connections),
        on(Screen::Destinations),
        fixtures::destination_detail(),
        on(Screen::Connections),
        fixtures::connection_detail(),
        settings,
        login,
        help,
    ];
    for app in screens {
        let screen = app.screen.clone();
        let text = screen_text(&ascii(app), 120, 40);
        assert!(text.is_ascii(), "{screen:?} is not ASCII:\n{text}");
    }
}

#[test]
fn a_modal_form_draws_over_the_screen() {
    let mut app = on_stats();
    crate::tui::keys_events::open_event_filters(
        &mut app,
        crate::tui::events_state::EventsScope::Global,
    );
    let text = screen_text(&app, 80, 24);
    for expected in [
        "Event filters",
        "Source",
        "Time range",
        "tab next field · ctrl+s save · esc cancel",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
}

#[test]
fn events_list() {
    let app = fixtures::events_app();
    snapshot_both_sizes("events", &app);
    let text = screen_text(&app, 120, 40);
    for expected in [
        "source: all sources · verification: any · window: all time",
        "page 1/3 · 120 events",
        "evt_8f2a1b",
        "stripe-prod",
        "github-ci",
        "1.2 KB",
        "✓2 ✕1",
        "…1",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
}

#[test]
fn events_from_the_tail_show_pending_counters() {
    let mut app = fixtures::events_app();
    let page = app.data.events.value.as_mut().unwrap();
    page.items[0].delivery_count = None;
    page.items[0].delivered_count = None;
    page.items[0].failed_count = None;
    page.items[0].pending_count = None;
    let text = screen_text(&app, 120, 40);
    let row = text
        .lines()
        .find(|line| line.contains("evt_8f2a1b"))
        .unwrap();
    assert!(row.contains('…') && !row.contains("✓2"), "{row}");
}

#[test]
fn events_tab_of_a_source() {
    let mut app = fixtures::source_detail(SourceTab::Events);
    let now = app.now;
    app.data.events.finish(fixtures::events_page(), now);
    assert_snapshot!("events_source_tab_120x40", draw(&app, 120, 40).backend());
    assert!(screen_text(&app, 120, 40).contains("source: stripe-prod"));
}

#[test]
fn event_filter_form() {
    let mut app = fixtures::events_app();
    crate::tui::keys_events::open_event_filters(
        &mut app,
        crate::tui::events_state::EventsScope::Global,
    );
    assert_snapshot!("form_event_filters_80x24", draw(&app, 80, 24).backend());
}

#[test]
fn live_tab() {
    let app = fixtures::live_app();
    snapshot_both_sizes("live", &app);
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("● live"));
    assert!(text.contains("follow on"));
    assert!(text.contains("evt_new"));
}

#[test]
fn live_tab_states() {
    let mut app = fixtures::live_app();
    app.event_screens.live.rows.clear();
    assert!(screen_text(&app, 120, 40)
        .contains("Waiting for webhooks… Send one to https://app.webhooker.eu/in/6b225n04u5kmyg"));
    app.event_screens.tail.as_mut().unwrap().status =
        crate::tui::events_state::TailStatus::Reconnecting {
            reason: "stream closed".into(),
        };
    assert!(screen_text(&app, 120, 40).contains("reconnecting (stream closed)"));
    app.event_screens.tail.as_mut().unwrap().status =
        crate::tui::events_state::TailStatus::Failed("source not found on the server".into());
    assert!(screen_text(&app, 120, 40).contains("✕ source not found on the server"));
}

#[test]
fn event_detail() {
    let app = fixtures::event_detail_app();
    snapshot_both_sizes("event_detail", &app);
    let text = screen_text(&app, 120, 40);
    for expected in [
        "POST evt_8f2a1b",
        "content-type: application/json",
        "user-agent: Stripe/1.0",
        "\"type\": \"invoice.paid\"",
        "billing-worker",
        "audit-log",
        "exhausted",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
}

#[test]
fn expanded_attempts_and_header_filters() {
    let mut app = fixtures::event_detail_app();
    app.event_screens.detail.expanded = Some("0198c9f0-0000-7000-8000-0000000000f2".into());
    app.event_screens.detail.header_filter = Some("signature".into());
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("→ 500"));
    assert!(text.contains("connection refused"));
    assert!(text.contains("{\"error\":\"db timeout\"}"));
    assert!(text.contains("stripe-signature"));
    assert!(!text.contains("user-agent"));
}

#[test]
fn binary_bodies_are_hex_dumped() {
    let mut app = fixtures::event_detail_app();
    app.data.event.value.as_mut().unwrap().body_base64 = Some("//4AAQ==".into());
    assert_snapshot!("event_detail_binary_120x40", draw(&app, 120, 40).backend());
}

#[test]
fn replay_form() {
    let mut app = fixtures::event_detail_app();
    crate::tui::app::update(
        &mut app,
        crate::tui::action::Action::Key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char('R'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        )),
    );
    assert_snapshot!("form_replay_80x24", draw(&app, 80, 24).backend());
    assert!(screen_text(&app, 80, 24).contains("[x] audit-log  disabled"));
}

#[test]
fn dlq_screen() {
    let app = fixtures::dlq_app();
    snapshot_both_sizes("dlq", &app);
    let text = screen_text(&app, 120, 40);
    for expected in [
        "DLQ · stripe-prod · exhausted,failed",
        "billing-worker",
        "4 exhausted",
        "1 failed",
        "Deliveries · billing-worker",
        "HTTP 410 Gone",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    assert!(
        !text.contains("connection refused"),
        "only the selected connection's rows"
    );
}

#[test]
fn dlq_tab_of_a_source_and_empty_states() {
    let mut app = fixtures::source_detail(SourceTab::Dlq);
    let now = app.now;
    app.data.dlq_summary.finish(
        crate::tui::model::Page {
            items: Vec::new(),
            total: None,
        },
        now,
    );
    assert!(screen_text(&app, 120, 40).contains("Nothing in the dead-letter queue."));
}

#[test]
fn bulk_resend_form() {
    let mut app = fixtures::dlq_app();
    crate::tui::keys_events::open_bulk_resend(&mut app);
    assert_snapshot!("form_bulk_resend_80x24", draw(&app, 80, 24).backend());
}

#[test]
fn stats_screen() {
    let app = fixtures::stats_app();
    snapshot_both_sizes("stats", &app);
    let text = screen_text(&app, 120, 40);
    for expected in [
        "Events per hour",
        "1 204",
        "Failed attempts 41",
        "p50 120 ms",
        "p95 880 ms",
        "p99 -",
        "✓ succeeded 1180",
        "✕ exhausted 3",
        "github-ci",
        "75%",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    let stripe = text
        .lines()
        .position(|line| line.contains("stripe-prod"))
        .unwrap();
    let github = text
        .lines()
        .position(|line| line.contains("github-ci"))
        .unwrap();
    assert!(stripe < github, "sources are sorted by volume");
}

#[test]
fn event_screens_are_ascii_clean_in_ascii_mode() {
    let mut with_form = fixtures::dlq_app();
    crate::tui::keys_events::open_bulk_resend(&mut with_form);
    let mut expanded = fixtures::event_detail_app();
    expanded.event_screens.detail.expanded = Some("0198c9f0-0000-7000-8000-0000000000f2".into());
    let mut binary = fixtures::event_detail_app();
    binary.data.event.value.as_mut().unwrap().body_base64 = Some("//4AAQ==".into());
    let mut events_tab = fixtures::source_detail(SourceTab::Events);
    let now = events_tab.now;
    events_tab.data.events.finish(fixtures::events_page(), now);
    let screens = [
        fixtures::events_app(),
        events_tab,
        fixtures::live_app(),
        fixtures::event_detail_app(),
        expanded,
        binary,
        fixtures::dlq_app(),
        with_form,
        fixtures::stats_app(),
    ];
    for app in screens {
        let screen = app.screen.clone();
        let app = ascii(app);
        for (width, height) in [(80, 24), (120, 40)] {
            let text = screen_text(&app, width, height);
            assert!(
                text.is_ascii(),
                "{screen:?} at {width}x{height} is not ASCII:\n{text}"
            );
        }
    }
}

use crate::tui::relay_session::RelayConnection;

fn relay_on_screen() -> App {
    fixtures::relay_app()
}

#[test]
fn relay_screen_without_a_session() {
    let mut app = fixtures::app();
    app.switch_to(Section::Relay);
    snapshot_both_sizes("relay_empty", &app);
    assert!(screen_text(&app, 120, 40)
        .contains("No relay running. Press n to start one, or L on a source."));
}

#[test]
fn relay_inspector() {
    let app = relay_on_screen();
    snapshot_both_sizes("relay_inspector", &app);
    let text = screen_text(&app, 120, 40);
    for expected in [
        "Relay · stripe-prod → http://localhost:3000",
        "2 fwd · 1 err",
        "connected",
        "12:04:11",
        "evt_8f2a…",
        "→ 200",
        "34ms",
        "→ 500",
        "✕ connection refused",
        "Request",
        "Response",
        "500 Internal Server Error",
        "p replay locally · u change URL · x stop · enter expand · / search",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
}

#[test]
fn the_expanded_exchange_shows_headers_and_bodies() {
    let mut app = relay_on_screen();
    app.relay.as_mut().unwrap().expanded = true;
    assert_snapshot!("relay_expanded_120x40", draw(&app, 120, 40).backend());
    let text = screen_text(&app, 120, 40);
    for expected in [
        "POST http://localhost:3000",
        "stripe-signature: t=1,v1=abc",
        "x-webhooker-event-id: evt_77c1d05e9a",
        "\"type\": \"invoice.paid\"",
        "\"error\": \"db timeout\"",
        "120ms",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    assert!(
        !text.contains("host: app.webhooker.eu"),
        "hop-by-hop headers are not sent"
    );
}

#[test]
fn the_header_shows_the_relay_on_every_screen() {
    let mut app = relay_on_screen();
    app.switch_to(Section::Stats);
    assert!(screen_text(&app, 120, 40).contains("⇄ relay stripe-prod → :3000 ●"));
    assert!(screen_text(&app, 100, 18).contains("⇄ relay stripe-prod → :3000 ●"));
    assert!(!screen_text(&fixtures::app(), 120, 40).contains("relay stripe-prod"));
}

#[test]
fn a_refused_stream_slot_is_explained_in_the_inspector() {
    let mut app = relay_on_screen();
    app.relay.as_mut().unwrap().connection = RelayConnection::Reconnecting {
        reason: "no live stream slot: the plan's live stream limit is reached".into(),
        server_message: Some("live stream limit reached for your plan (3)".into()),
    };
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("live stream limit reached for your plan (3)"));
    assert!(text.contains("⇄ relay stripe-prod → :3000 ⠋"));
}

#[test]
fn a_failed_relay_is_marked_in_the_header() {
    let mut app = relay_on_screen();
    app.relay.as_mut().unwrap().connection = RelayConnection::Failed("source not found".into());
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("⇄ relay stripe-prod → :3000 ✕"));
    assert!(text.contains("stopped: source not found"));
}

#[test]
fn the_relay_views_are_ascii_clean_in_ascii_mode() {
    let app = ascii(relay_on_screen());
    let text = screen_text(&app, 120, 40);
    assert!(text.is_ascii(), "{text}");
    assert!(text.contains("<> relay stripe-prod -> :3000 *"));
    let mut expanded = relay_on_screen();
    expanded.relay.as_mut().unwrap().expanded = true;
    assert!(screen_text(&ascii(expanded), 120, 40).is_ascii());
}

#[test]
fn targets_are_shortened_for_the_header() {
    use super::relay::short_target;
    assert_eq!(short_target("http://localhost:3000"), ":3000");
    assert_eq!(short_target("http://127.0.0.1:8080/hooks"), ":8080");
    assert_eq!(
        short_target("https://tunnel.example.com/in"),
        "tunnel.example.com"
    );
    assert_eq!(short_target("http://localhost"), ":80");
}
