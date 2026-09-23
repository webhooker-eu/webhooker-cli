//! Full-screen terminal UI opened by a bare `whk` on a terminal or by `whk ui`.

pub mod action;
pub mod app;
pub mod budget;
pub mod events_state;
pub mod forms;
pub mod hints;
pub mod keys;
pub mod keys_events;
pub mod model;
pub mod names;
pub mod poller;
pub mod relay_session;
pub mod screen;
pub mod settings;
pub mod status;
pub mod streams;
pub mod theme;
pub mod views;
pub mod worker;

#[cfg(test)]
pub(crate) mod fixtures;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use futures::StreamExt;
use ratatui::crossterm::event::{Event, EventStream, KeyEventKind};
use tokio::sync::mpsc;

use crate::client::ApiClient;
use crate::config::{self, UiSection};
use action::Action;
use app::{App, AppInit, KeySource};
use settings::UiSettings;
use theme::TerminalEnv;
use worker::Worker;

const TICK: Duration = Duration::from_millis(250);

pub struct LaunchOptions {
    pub server: String,
    pub api_key: Option<String>,
    pub key_source: KeySource,
    pub config_path: PathBuf,
    pub ui: UiSection,
}

/// Everything that decides whether a bare `whk` opens the TUI.
pub struct BareCommandContext {
    pub stdin_is_terminal: bool,
    pub stdout_is_terminal: bool,
    pub json: bool,
    pub no_tui_env: Option<String>,
    pub open_on_bare_command: Option<bool>,
}

pub fn bare_command_opens_tui(context: &BareCommandContext) -> bool {
    context.stdin_is_terminal
        && context.stdout_is_terminal
        && !context.json
        && context.no_tui_env.as_deref().is_none_or(str::is_empty)
        && context.open_on_bare_command != Some(false)
}

pub fn attached_to_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Runs the TUI until the user quits or a termination signal arrives. The
/// terminal is restored on every path; `ratatui::init` also installs a panic
/// hook that restores it before the panic message prints.
pub async fn run(options: LaunchOptions) -> Result<()> {
    let env = TerminalEnv::from_env();
    let (settings, warnings) = UiSettings::from_section(&options.ui);
    let size = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
    let mut app = App::new(AppInit {
        settings,
        env,
        server: options.server.clone(),
        key_source: options.key_source,
        has_key: options.api_key.is_some(),
        last_screen: options.ui.state.last_screen.clone(),
        warnings,
        size,
        now: Instant::now(),
        wall_clock: chrono::Utc::now(),
    });
    let client = options
        .api_key
        .map(|key| ApiClient::new(options.server.clone(), key))
        .transpose()?;
    let (actions, mut received) = mpsc::unbounded_channel();
    let worker = Worker::new(
        client,
        app.budget_limit(),
        options.config_path.clone(),
        actions,
    );

    let mut terminal = ratatui::init();
    let outcome = event_loop(&mut terminal, &mut app, &worker, &mut received).await;
    ratatui::restore();

    remember_last_screen(&app, &options.config_path);
    outcome?;
    if let Some(message) = app.exit_error.take() {
        bail!("{message}");
    }
    Ok(())
}

/// Only for a logged-in session with a config file, so an env-only key never
/// makes the TUI create a config file.
fn remember_last_screen(app: &App, config_path: &std::path::Path) {
    let Some(slug) = app.last_screen_slug() else {
        return;
    };
    if app.is_logged_in() && config_path.exists() {
        let _ = config::update(config_path, |config| {
            config.ui.state.last_screen = Some(slug.to_string())
        });
    }
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    worker: &Worker,
    received: &mut mpsc::UnboundedReceiver<Action>,
) -> Result<()> {
    let mut input = EventStream::new();
    let mut ticker = tokio::time::interval(TICK);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    for effect in app.start() {
        worker.run(effect);
    }
    while !app.quit {
        terminal.draw(|frame| views::render(frame, app))?;
        let action = tokio::select! {
            event = input.next() => match event {
                // Windows also reports key releases; only presses count.
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => Action::Key(key),
                Some(Ok(Event::Resize(width, height))) => Action::Resize { width, height },
                Some(Ok(_)) => continue,
                Some(Err(error)) => return Err(error.into()),
                None => Action::Terminate,
            },
            _ = ticker.tick() => Action::Tick { now: Instant::now(), wall_clock: chrono::Utc::now() },
            Some(action) = received.recv() => action,
            _ = &mut shutdown => Action::Terminate,
        };
        for effect in app::update(app, action) {
            worker.run(effect);
        }
    }
    Ok(())
}

/// SIGTERM and SIGHUP on Unix, the console window closing on Windows.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) {
            (Ok(mut terminate), Ok(mut hangup)) => {
                tokio::select! {
                    _ = terminate.recv() => {}
                    _ = hangup.recv() => {}
                }
            }
            _ => std::future::pending::<()>().await,
        }
    }
    #[cfg(windows)]
    {
        match tokio::signal::windows::ctrl_close() {
            Ok(mut close) => {
                close.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal_context() -> BareCommandContext {
        BareCommandContext {
            stdin_is_terminal: true,
            stdout_is_terminal: true,
            json: false,
            no_tui_env: None,
            open_on_bare_command: None,
        }
    }

    #[test]
    fn a_bare_command_on_a_terminal_opens_the_tui() {
        assert!(bare_command_opens_tui(&terminal_context()));
        assert!(bare_command_opens_tui(&BareCommandContext {
            no_tui_env: Some(String::new()),
            open_on_bare_command: Some(true),
            ..terminal_context()
        }));
    }

    #[test]
    fn anything_but_an_interactive_terminal_prints_help() {
        for context in [
            BareCommandContext {
                stdin_is_terminal: false,
                ..terminal_context()
            },
            BareCommandContext {
                stdout_is_terminal: false,
                ..terminal_context()
            },
            BareCommandContext {
                json: true,
                ..terminal_context()
            },
            BareCommandContext {
                no_tui_env: Some("1".into()),
                ..terminal_context()
            },
            BareCommandContext {
                open_on_bare_command: Some(false),
                ..terminal_context()
            },
        ] {
            assert!(!bare_command_opens_tui(&context));
        }
    }
}
