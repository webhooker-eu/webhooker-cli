//! Render snapshots at 80×24 and 120×40 plus targeted text checks. Snapshots
//! live in `src/tui/views/snapshots/`; review every new or changed `.snap`.

use std::time::Duration;

use insta::assert_snapshot;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::render;
use crate::tui::app::{App, Confirm, ConfirmAction};
use crate::tui::fixtures;
use crate::tui::screen::Section;
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
