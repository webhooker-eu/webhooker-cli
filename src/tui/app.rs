//! TUI state and `update`: every input becomes an `Action`; `update` mutates
//! the state and returns the `Effect`s for the worker. Nothing here blocks.

use std::cell::Cell;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::client::ApiError;
use crate::tui::action::{
    Action, Effect, FetchError, LoginSuccess, Mutation, Request, GLOBAL_GENERATION,
};
use crate::tui::budget::{self, Priority, FREE_API_PER_MINUTE};
use crate::tui::events_state::EventScreens;
use crate::tui::forms::input::TextInput;
use crate::tui::forms::settings_form::SettingsForm;
use crate::tui::model::{
    Connection, Destination, DlqEntry, DlqSummary, EventDetail, EventSummary, Me, Page, PlanList,
    Source, SourceConnection, SourceVolume, StatsOverview, Workspace,
};
use crate::tui::names::NameCache;
use crate::tui::poller::{self, PlanTier, Poller};
use crate::tui::screen::{Screen, Section, SourceTab};
use crate::tui::settings::{StartScreen, UiSettings};
use crate::tui::theme::{TerminalEnv, Theme, Tone};

pub const TOAST_LIFETIME: Duration = Duration::from_secs(4);
/// Used when a 429 carries no `Retry-After`: one full rate-limit window.
pub const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);
pub const KEY_REJECTED_MESSAGE: &str = "API key is invalid or revoked";
pub const OVERRIDE_KEY_REJECTED_MESSAGE: &str =
    "the API key from --api-key/WEBHOOKER_API_KEY was rejected";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// Saved by `whk login` or the TUI's login screen.
    Config,
    /// `--api-key` or `WEBHOOKER_API_KEY`; the TUI never replaces it.
    Override,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Main,
}

/// Data the screen shows plus its loading state. Failed refreshes keep the
/// last value and mark it stale.
#[derive(Debug, Clone, PartialEq)]
pub struct Loadable<T> {
    pub value: Option<T>,
    pub loading: bool,
    pub stale: bool,
    pub loaded_at: Option<Instant>,
}

impl<T> Default for Loadable<T> {
    fn default() -> Self {
        Self {
            value: None,
            loading: false,
            stale: false,
            loaded_at: None,
        }
    }
}

impl<T> Loadable<T> {
    pub fn finish(&mut self, value: T, now: Instant) {
        self.value = Some(value);
        self.loading = false;
        self.stale = false;
        self.loaded_at = Some(now);
    }
}

/// Loading bookkeeping shared by every `Loadable`, whatever it holds.
pub trait Slot {
    fn begin(&mut self);
    fn fail(&mut self);
    fn cancel(&mut self);
    fn status(&self) -> (Option<Instant>, bool);
}

impl<T> Slot for Loadable<T> {
    fn begin(&mut self) {
        self.loading = true;
    }

    fn fail(&mut self) {
        self.loading = false;
        self.stale = self.value.is_some();
    }

    fn cancel(&mut self) {
        self.loading = false;
    }

    fn status(&self) -> (Option<Instant>, bool) {
        (self.loaded_at, self.stale)
    }
}

#[derive(Debug, Default, Clone)]
pub struct Data {
    pub sources: Loadable<Vec<Source>>,
    pub source: Loadable<Source>,
    pub source_connections: Loadable<Vec<SourceConnection>>,
    pub destinations: Loadable<Vec<Destination>>,
    pub destination: Loadable<Destination>,
    pub connections: Loadable<Vec<Connection>>,
    pub connection: Loadable<Connection>,
    pub events: Loadable<Page<EventSummary>>,
    pub event: Loadable<EventDetail>,
    pub dlq_summary: Loadable<Page<DlqSummary>>,
    pub dlq_entries: Loadable<Page<DlqEntry>>,
    pub stats_overview: Loadable<StatsOverview>,
    pub source_volume: Loadable<Page<SourceVolume>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cursors {
    pub sidebar: usize,
    pub sources: usize,
    pub destinations: usize,
    pub connections: usize,
    pub source_connections: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub text: String,
    pub tone: Tone,
    pub expires_at: Instant,
}

/// What a confirmed modal does. Plans 4 and 5 add variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    DiscardSettings,
}

/// A yes/no question; only `y` confirms. The target is drawn in bold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub before: String,
    pub target: Option<String>,
    pub after: String,
    pub action: ConfirmAction,
}

impl Confirm {
    pub fn plain(message: impl Into<String>, action: ConfirmAction) -> Self {
        Self {
            before: message.into(),
            target: None,
            after: String::new(),
            action,
        }
    }

    /// "Pause " + **stripe-prod** + "?"
    pub fn about(
        before: impl Into<String>,
        target: impl Into<String>,
        after: impl Into<String>,
        action: ConfirmAction,
    ) -> Self {
        Self {
            before: before.into(),
            target: Some(target.into()),
            after: after.into(),
            action,
        }
    }

    pub fn text(&self) -> String {
        format!(
            "{}{}{}",
            self.before,
            self.target.as_deref().unwrap_or(""),
            self.after
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoginForm {
    pub api_key: TextInput,
    pub server: TextInput,
    pub server_focused: bool,
    pub error: Option<String>,
    pub submitting: bool,
}

impl LoginForm {
    pub fn for_server(server: &str) -> Self {
        Self {
            api_key: TextInput::new("", true),
            server: TextInput::new(server, false),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub server: String,
    pub key_source: KeySource,
    pub workspace: Option<Workspace>,
    pub plans: Option<PlanList>,
}

pub struct AppInit {
    pub settings: UiSettings,
    pub env: TerminalEnv,
    pub server: String,
    pub key_source: KeySource,
    pub has_key: bool,
    pub last_screen: Option<String>,
    pub warnings: Vec<String>,
    pub size: (u16, u16),
    pub now: Instant,
    pub wall_clock: DateTime<Utc>,
}

pub struct App {
    pub settings: UiSettings,
    pub env: TerminalEnv,
    pub theme: Theme,
    pub session: Session,
    pub screen: Screen,
    pub history: Vec<Screen>,
    pub focus: Focus,
    pub generation: u64,
    pub data: Data,
    pub names: NameCache,
    pub cursors: Cursors,
    /// The applied `/` filter of the Sources list.
    pub source_query: Option<String>,
    /// The filter being typed; while set, keys go to it.
    pub source_search: Option<TextInput>,
    /// `g` was pressed; the next letter picks a section.
    pub pending_jump: bool,
    pub help_open: bool,
    pub confirm: Option<Confirm>,
    pub toasts: Vec<Toast>,
    pub rate_limited_until: Option<Instant>,
    pub poller: Poller,
    pub login: LoginForm,
    pub settings_form: Option<SettingsForm>,
    /// Settings waiting for their write to succeed before they apply.
    pub pending_settings: Option<UiSettings>,
    /// State of the Events, Live, event detail, DLQ and Stats screens.
    pub event_screens: EventScreens,
    /// Scroll offset of the text pane on detail screens.
    pub scroll: u16,
    /// Largest useful `scroll`, written by the view that draws the pane.
    pub scroll_limit: Cell<u16>,
    pub size: (u16, u16),
    pub now: Instant,
    pub wall_clock: DateTime<Utc>,
    pub tick_count: usize,
    pub quit: bool,
    /// Printed after the terminal is restored; makes `whk` exit with 1.
    pub exit_error: Option<String>,
    has_key: bool,
    last_screen: Option<String>,
}

impl App {
    pub fn new(init: AppInit) -> Self {
        let theme = Theme::new(&init.settings, &init.env);
        let mut app = Self {
            theme,
            settings: init.settings,
            env: init.env,
            session: Session {
                server: init.server.clone(),
                key_source: init.key_source,
                workspace: None,
                plans: None,
            },
            screen: Screen::Login,
            history: Vec::new(),
            focus: Focus::Main,
            generation: GLOBAL_GENERATION + 1,
            data: Data::default(),
            names: NameCache::default(),
            cursors: Cursors::default(),
            source_query: None,
            source_search: None,
            pending_jump: false,
            help_open: false,
            confirm: None,
            toasts: Vec::new(),
            rate_limited_until: None,
            poller: Poller::default(),
            login: LoginForm::for_server(&init.server),
            settings_form: None,
            pending_settings: None,
            event_screens: EventScreens::default(),
            scroll: 0,
            scroll_limit: Cell::new(u16::MAX),
            size: init.size,
            now: init.now,
            wall_clock: init.wall_clock,
            tick_count: 0,
            quit: false,
            exit_error: None,
            has_key: init.has_key,
            last_screen: init.last_screen,
        };
        for warning in init.warnings {
            app.toast(warning, Tone::Warning);
        }
        app
    }

    /// The first effects: workspace, plan limits and the start screen, or
    /// the login screen when there is no key.
    pub fn start(&mut self) -> Vec<Effect> {
        if !self.has_key {
            self.screen = Screen::Login;
            return Vec::new();
        }
        let mut effects = vec![
            self.fetch(Request::Me, GLOBAL_GENERATION, Priority::FirstLoad),
            self.fetch(Request::Plans, GLOBAL_GENERATION, Priority::FirstLoad),
        ];
        let section = self.start_section();
        effects.extend(self.switch_to(section));
        effects
    }

    pub fn is_logged_in(&self) -> bool {
        self.session.workspace.is_some()
    }

    pub fn tier(&self) -> PlanTier {
        self.session
            .workspace
            .as_ref()
            .map_or(PlanTier::Free, |workspace| {
                PlanTier::from_plan(&workspace.plan)
            })
    }

    /// `ui.request_budget_percent` of the plan's `api_per_minute`; the Free
    /// limit until `/me` and `/plans` have both answered, or if `/plans` fails.
    pub fn budget_limit(&self) -> u32 {
        let api_per_minute = match (&self.session.workspace, &self.session.plans) {
            (Some(workspace), Some(plans)) => plans
                .limits_for(&workspace.plan)
                .api_per_minute
                .unwrap_or(FREE_API_PER_MINUTE),
            _ => FREE_API_PER_MINUTE,
        };
        budget::share_of(api_per_minute, self.settings.request_budget_percent)
    }

    pub fn budget_effect(&self) -> Effect {
        Effect::ConfigureBudget {
            limit: self.budget_limit(),
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, tone: Tone) {
        self.toasts.push(Toast {
            text: text.into(),
            tone,
            expires_at: self.now + TOAST_LIFETIME,
        });
    }

    /// Plan 4 asks for confirmation here while a relay runs.
    pub fn request_quit(&mut self) {
        self.quit = true;
    }

    pub fn last_screen_slug(&self) -> Option<&'static str> {
        self.screen.section().map(Section::slug)
    }

    /// Inside these panes `g` means "top" instead of a section jump.
    pub fn in_scrollable_pane(&self) -> bool {
        matches!(
            self.screen,
            Screen::DestinationDetail { .. }
                | Screen::ConnectionDetail { .. }
                | Screen::EventDetail { .. }
        )
    }

    pub fn apply_settings(&mut self, settings: UiSettings) {
        self.theme = Theme::new(&settings, &self.env);
        self.settings = settings;
    }

    pub fn switch_to(&mut self, section: Section) -> Vec<Effect> {
        self.history.clear();
        self.screen = section.root();
        self.focus = Focus::Main;
        self.cursors.sidebar = Section::ALL
            .iter()
            .position(|candidate| *candidate == section)
            .unwrap_or(0);
        self.enter()
    }

    /// Opens a detail screen on top of the current one.
    pub fn open(&mut self, screen: Screen) -> Vec<Effect> {
        match &screen {
            Screen::SourceDetail { .. } => {
                self.data.source = Loadable::default();
                self.data.source_connections = Loadable::default();
                self.cursors.source_connections = 0;
            }
            Screen::DestinationDetail { .. } => self.data.destination = Loadable::default(),
            Screen::ConnectionDetail { .. } => self.data.connection = Loadable::default(),
            Screen::EventDetail { .. } => self.data.event = Loadable::default(),
            _ => {}
        }
        let previous = std::mem::replace(&mut self.screen, screen);
        self.history.push(previous);
        self.focus = Focus::Main;
        self.enter()
    }

    /// Back to the previous screen, or to the sidebar from a section root.
    pub fn back(&mut self) -> Vec<Effect> {
        match self.history.pop() {
            Some(previous) => {
                self.screen = previous;
                self.enter()
            }
            None => {
                self.focus = Focus::Sidebar;
                Vec::new()
            }
        }
    }

    pub fn set_source_tab(&mut self, tab: SourceTab) -> Vec<Effect> {
        if let Screen::SourceDetail { tab: current, .. } = &mut self.screen {
            if *current == tab {
                return Vec::new();
            }
            *current = tab;
        }
        self.enter()
    }

    /// A new screen is visible: new generation, first loads for its schedule.
    pub fn enter(&mut self) -> Vec<Effect> {
        self.generation += 1;
        self.scroll = 0;
        self.pending_jump = false;
        self.prepare_screen();
        if self.screen == Screen::Settings && self.settings_form.is_none() {
            self.settings_form = Some(SettingsForm::new(&self.settings));
        }
        let generation = self.generation;
        let mut effects: Vec<Effect> = self
            .schedule()
            .into_iter()
            .map(|(request, _)| self.fetch(request, generation, Priority::FirstLoad))
            .collect();
        effects.extend(crate::tui::streams::sync(self));
        effects
    }

    /// `r`: refetch everything the screen shows, as a user action.
    pub fn refresh_now(&mut self) -> Vec<Effect> {
        let generation = self.generation;
        self.schedule()
            .into_iter()
            .map(|(request, _)| self.fetch(request, generation, Priority::User))
            .collect()
    }

    /// (loaded at, stale) of the data the current screen is about.
    pub fn primary_status(&self) -> Option<(Option<Instant>, bool)> {
        let slot: &dyn Slot = match &self.screen {
            Screen::Sources => &self.data.sources,
            Screen::SourceDetail {
                tab: SourceTab::Connections,
                ..
            } => &self.data.source_connections,
            Screen::SourceDetail {
                tab: SourceTab::Events,
                ..
            } => &self.data.events,
            Screen::SourceDetail {
                tab: SourceTab::Dlq,
                ..
            } => &self.data.dlq_summary,
            Screen::SourceDetail { .. } => &self.data.source,
            Screen::Destinations => &self.data.destinations,
            Screen::DestinationDetail { .. } => &self.data.destination,
            Screen::Connections => &self.data.connections,
            Screen::ConnectionDetail { .. } => &self.data.connection,
            Screen::Events => &self.data.events,
            Screen::EventDetail { .. } => &self.data.event,
            Screen::Dlq => &self.data.dlq_summary,
            Screen::Stats => &self.data.stats_overview,
            _ => return None,
        };
        Some(slot.status())
    }

    fn start_section(&self) -> Section {
        match self.settings.start_screen {
            StartScreen::Sources => Section::Sources,
            StartScreen::Events => Section::Events,
            StartScreen::Relay => Section::Relay,
            StartScreen::Stats => Section::Stats,
            StartScreen::Last => self
                .last_screen
                .as_deref()
                .and_then(Section::from_slug)
                .unwrap_or(Section::Sources),
        }
    }

    fn schedule(&self) -> Vec<(Request, Duration)> {
        let mut schedule =
            poller::schedule(&self.screen, self.source_query.as_deref(), self.tier());
        schedule.extend(crate::tui::events_state::schedule(self));
        schedule
    }

    fn fetch(&mut self, request: Request, generation: u64, priority: Priority) -> Effect {
        self.poller.mark(&request, self.now);
        if let Some(slot) = self.slot(&request) {
            slot.begin();
        }
        Effect::Fetch {
            request,
            generation,
            priority,
        }
    }

    fn slot(&mut self, request: &Request) -> Option<&mut dyn Slot> {
        let slot: &mut dyn Slot = match request {
            Request::Me | Request::Plans => return None,
            Request::Sources { .. } => &mut self.data.sources,
            Request::Source { .. } => &mut self.data.source,
            Request::SourceConnections { .. } => &mut self.data.source_connections,
            Request::Destinations => &mut self.data.destinations,
            Request::Destination { .. } => &mut self.data.destination,
            Request::Connections => &mut self.data.connections,
            Request::Connection { .. } => &mut self.data.connection,
            Request::Events { .. } => &mut self.data.events,
            Request::Event { .. } => &mut self.data.event,
            Request::DlqSummary { .. } => &mut self.data.dlq_summary,
            Request::DlqEntries { .. } => &mut self.data.dlq_entries,
            Request::StatsOverview { .. } => &mut self.data.stats_overview,
            Request::SourceVolume { .. } => &mut self.data.source_volume,
        };
        Some(slot)
    }

    fn slot_fail(&mut self, request: &Request) {
        if let Some(slot) = self.slot(request) {
            slot.fail();
        }
    }

    fn slot_cancel(&mut self, request: &Request) {
        if let Some(slot) = self.slot(request) {
            slot.cancel();
        }
    }

    fn is_open_resource(&self, request: &Request) -> bool {
        match (request, &self.screen) {
            (Request::Source { id }, Screen::SourceDetail { id: open, .. })
            | (
                Request::SourceConnections { source_id: id },
                Screen::SourceDetail { id: open, .. },
            )
            | (Request::Destination { id }, Screen::DestinationDetail { id: open })
            | (Request::Connection { id }, Screen::ConnectionDetail { id: open })
            | (Request::Event { id }, Screen::EventDetail { id: open }) => id == open,
            _ => false,
        }
    }

    fn key_rejected(&mut self) -> Vec<Effect> {
        match self.session.key_source {
            KeySource::Config => {
                self.screen = Screen::Login;
                self.history.clear();
                self.generation += 1;
                self.login = LoginForm::for_server(&self.session.server);
                self.login.error = Some(KEY_REJECTED_MESSAGE.to_string());
            }
            KeySource::Override => {
                self.exit_error = Some(OVERRIDE_KEY_REJECTED_MESSAGE.to_string());
                self.quit = true;
            }
        }
        Vec::new()
    }

    fn on_api_error(&mut self, request: &Request, error: ApiError) -> Vec<Effect> {
        self.slot_fail(request);
        if *request == Request::Plans {
            // The Free limits stay in force; nothing to tell the user.
            return Vec::new();
        }
        match error.status {
            401 => self.key_rejected(),
            429 => {
                self.rate_limited_until =
                    Some(self.now + error.retry_after.unwrap_or(DEFAULT_RETRY_AFTER));
                Vec::new()
            }
            404 if self.is_open_resource(request) => {
                self.toast("No longer exists", Tone::Warning);
                self.back()
            }
            500..=599 => {
                self.poller.network_failed(self.now);
                Vec::new()
            }
            _ => {
                self.toast(FetchError::Api(error).message(), Tone::Danger);
                Vec::new()
            }
        }
    }

    fn apply(&mut self, request: &Request, value: Value) -> Result<Vec<Effect>, serde_json::Error> {
        let now = self.now;
        match request {
            Request::Me => {
                let me: Me = decode(value)?;
                self.session.workspace = Some(me.workspace);
                return Ok(vec![self.budget_effect()]);
            }
            Request::Plans => {
                self.session.plans = Some(decode(value)?);
                return Ok(vec![self.budget_effect()]);
            }
            Request::Sources { .. } => {
                let page: Page<Source> = decode(value)?;
                self.names.remember_sources(
                    page.items
                        .iter()
                        .map(|source| (source.id.as_str(), source.name.as_str())),
                );
                self.cursors.sources = clamp_cursor(self.cursors.sources, page.items.len());
                self.data.sources.finish(page.items, now);
            }
            Request::Source { .. } => {
                let source: Source = decode(value)?;
                self.names
                    .remember_sources([(source.id.as_str(), source.name.as_str())]);
                self.data.source.finish(source, now);
            }
            Request::SourceConnections { .. } => {
                let page: Page<SourceConnection> = decode(value)?;
                self.names
                    .remember_destinations(page.items.iter().map(|connection| {
                        (
                            connection.destination.id.as_str(),
                            connection.destination.name.as_str(),
                        )
                    }));
                self.cursors.source_connections =
                    clamp_cursor(self.cursors.source_connections, page.items.len());
                self.data.source_connections.finish(page.items, now);
            }
            Request::Destinations => {
                let page: Page<Destination> = decode(value)?;
                self.names.remember_destinations(
                    page.items
                        .iter()
                        .map(|destination| (destination.id.as_str(), destination.name.as_str())),
                );
                self.cursors.destinations =
                    clamp_cursor(self.cursors.destinations, page.items.len());
                self.data.destinations.finish(page.items, now);
            }
            Request::Destination { .. } => {
                let destination: Destination = decode(value)?;
                self.names
                    .remember_destinations([(destination.id.as_str(), destination.name.as_str())]);
                self.data.destination.finish(destination, now);
            }
            Request::Connections => {
                let page: Page<Connection> = decode(value)?;
                self.cursors.connections = clamp_cursor(self.cursors.connections, page.items.len());
                self.data.connections.finish(page.items, now);
            }
            Request::Connection { .. } => {
                self.data.connection.finish(decode(value)?, now);
            }
            Request::Events { .. } => {
                let page: Page<EventSummary> = decode(value)?;
                if let Some(scope) = self.visible_events_scope() {
                    let state = self.event_screens.events_mut(scope);
                    state.cursor = clamp_cursor(state.cursor, page.items.len());
                }
                self.data.events.finish(page, now);
            }
            Request::Event { .. } => {
                let event: EventDetail = decode(value)?;
                let deliveries = event.deliveries.len();
                let view = &mut self.event_screens.detail;
                view.delivery_cursor = clamp_cursor(view.delivery_cursor, deliveries);
                self.data.event.finish(event, now);
            }
            Request::DlqSummary { .. } => {
                let page: Page<DlqSummary> = decode(value)?;
                let dlq = &mut self.event_screens.dlq;
                dlq.summary_cursor = clamp_cursor(dlq.summary_cursor, page.items.len());
                self.data.dlq_summary.finish(page, now);
            }
            Request::DlqEntries { .. } => {
                self.data.dlq_entries.finish(decode(value)?, now);
            }
            Request::StatsOverview { .. } => {
                self.data.stats_overview.finish(decode(value)?, now);
            }
            Request::SourceVolume { .. } => {
                let page: Page<SourceVolume> = decode(value)?;
                self.names.remember_sources(
                    page.items
                        .iter()
                        .map(|volume| (volume.source_id.as_str(), volume.name.as_str())),
                );
                self.data.source_volume.finish(page, now);
            }
        }
        Ok(Vec::new())
    }
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, serde_json::Error> {
    serde_json::from_value(value)
}

fn clamp_cursor(cursor: usize, length: usize) -> usize {
    cursor.min(length.saturating_sub(1))
}

pub fn update(app: &mut App, action: Action) -> Vec<Effect> {
    match action {
        Action::Key(key) => crate::tui::keys::handle(app, key),
        Action::Resize { width, height } => {
            app.size = (width, height);
            Vec::new()
        }
        Action::Tick { now, wall_clock } => on_tick(app, now, wall_clock),
        Action::Fetched {
            request,
            generation,
            result,
        } => on_fetched(app, request, generation, result),
        Action::FetchSkipped {
            request,
            generation,
        } => {
            if generation == GLOBAL_GENERATION || generation == app.generation {
                app.slot_cancel(&request);
            }
            Vec::new()
        }
        Action::LoginFinished(result) => on_login_finished(app, result),
        Action::SettingsSaved(result) => on_settings_saved(app, result),
        Action::Mutated { mutation, result } => on_mutated(app, mutation, result),
        Action::TailNotice {
            subscription,
            notice,
        } => crate::tui::streams::on_notice(app, subscription, notice),
        Action::TailStatus {
            subscription,
            status,
        } => crate::tui::streams::on_status(app, subscription, status),
        Action::Terminate => {
            app.quit = true;
            Vec::new()
        }
    }
}

fn on_tick(app: &mut App, now: Instant, wall_clock: DateTime<Utc>) -> Vec<Effect> {
    app.now = now;
    app.wall_clock = wall_clock;
    app.tick_count = app.tick_count.wrapping_add(1);
    app.toasts.retain(|toast| toast.expires_at > now);
    if app.rate_limited_until.is_some_and(|until| until <= now) {
        app.rate_limited_until = None;
    }
    if app.screen == Screen::Login || app.rate_limited_until.is_some() {
        return Vec::new();
    }
    let schedule = app.schedule();
    let generation = app.generation;
    app.poller
        .due(&schedule, now)
        .into_iter()
        .map(|request| app.fetch(request, generation, Priority::Background))
        .collect()
}

fn on_fetched(
    app: &mut App,
    request: Request,
    generation: u64,
    result: Result<Value, FetchError>,
) -> Vec<Effect> {
    if generation != GLOBAL_GENERATION && generation != app.generation {
        return Vec::new();
    }
    match result {
        Ok(value) => {
            app.poller.network_recovered();
            match app.apply(&request, value) {
                Ok(effects) => effects,
                Err(error) => {
                    app.slot_fail(&request);
                    app.toast(
                        format!("Unexpected response from the server: {error}"),
                        Tone::Danger,
                    );
                    Vec::new()
                }
            }
        }
        Err(FetchError::Network(_)) => {
            app.slot_fail(&request);
            app.poller.network_failed(app.now);
            Vec::new()
        }
        Err(FetchError::Api(error)) => app.on_api_error(&request, error),
    }
}

fn on_login_finished(app: &mut App, result: Result<LoginSuccess, String>) -> Vec<Effect> {
    app.login.submitting = false;
    match result {
        Ok(success) => {
            app.session.server = success.server;
            app.session.key_source = KeySource::Config;
            app.session.workspace = Some(success.me.workspace);
            app.has_key = true;
            app.login = LoginForm::for_server(&app.session.server);
            let mut effects = vec![
                app.fetch(Request::Plans, GLOBAL_GENERATION, Priority::FirstLoad),
                app.budget_effect(),
            ];
            effects.extend(app.switch_to(Section::Sources));
            effects
        }
        Err(message) => {
            app.login.error = Some(message);
            Vec::new()
        }
    }
}

fn on_settings_saved(app: &mut App, result: Result<(), String>) -> Vec<Effect> {
    match result {
        Ok(()) => {
            if let Some(settings) = app.pending_settings.take() {
                app.apply_settings(settings);
            }
            app.settings_form = Some(SettingsForm::new(&app.settings));
            app.toast("Settings saved", Tone::Success);
            vec![app.budget_effect()]
        }
        Err(message) => {
            app.pending_settings = None;
            if let Some(form) = app.settings_form.as_mut() {
                form.error = Some(message);
            }
            Vec::new()
        }
    }
}

/// A finished write: toast the outcome, then refetch what it touched.
fn on_mutated(app: &mut App, mutation: Mutation, result: Result<Value, FetchError>) -> Vec<Effect> {
    let mut effects = match result {
        Ok(value) => mutation_succeeded(app, &mutation, &value),
        Err(error) => {
            if let FetchError::Api(api) = &error {
                match api.status {
                    401 => return app.key_rejected(),
                    429 => {
                        app.rate_limited_until =
                            Some(app.now + api.retry_after.unwrap_or(DEFAULT_RETRY_AFTER));
                    }
                    _ => {}
                }
            }
            app.toast(error.message(), Tone::Danger);
            Vec::new()
        }
    };
    let generation = app.generation;
    for request in mutation.affected() {
        effects.push(app.fetch(request, generation, Priority::User));
    }
    effects
}

fn mutation_succeeded(app: &mut App, mutation: &Mutation, value: &Value) -> Vec<Effect> {
    match mutation {
        Mutation::ReplayEvent { .. } | Mutation::ResendBulk { .. } => {
            let created = value.get("created").and_then(Value::as_i64).unwrap_or(0);
            let noun = if created == 1 {
                "delivery"
            } else {
                "deliveries"
            };
            app.toast(format!("{created} {noun} queued"), Tone::Success);
            if matches!(mutation, Mutation::ResendBulk { .. }) {
                return app.refresh_now();
            }
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fixtures;
    use crate::tui::settings::ThemeChoice;
    use serde_json::json;

    fn fetched_requests(effects: &[Effect]) -> Vec<Request> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Fetch { request, .. } => Some(request.clone()),
                _ => None,
            })
            .collect()
    }

    fn failure(status: u16) -> FetchError {
        FetchError::Api(ApiError::synthetic(status, None, "boom"))
    }

    fn fail_sources(app: &mut App, error: FetchError) -> Vec<Effect> {
        let generation = app.generation;
        update(
            app,
            Action::Fetched {
                request: Request::Sources { search: None },
                generation,
                result: Err(error),
            },
        )
    }

    fn tick(app: &mut App, after: Duration) -> Vec<Effect> {
        let now = app.now + after;
        let wall_clock = app.wall_clock;
        update(app, Action::Tick { now, wall_clock })
    }

    #[test]
    fn without_a_key_the_app_starts_on_login() {
        let mut app = App::new(fixtures::init(false));
        assert!(app.start().is_empty());
        assert_eq!(app.screen, Screen::Login);
    }

    #[test]
    fn with_a_key_it_loads_the_workspace_and_the_start_screen() {
        let mut app = App::new(fixtures::init(true));
        let effects = app.start();
        assert_eq!(app.screen, Screen::Sources);
        assert_eq!(
            fetched_requests(&effects),
            vec![
                Request::Me,
                Request::Plans,
                Request::Sources { search: None },
                Request::Connections
            ]
        );
        assert!(matches!(
            effects[0],
            Effect::Fetch {
                generation: GLOBAL_GENERATION,
                priority: Priority::FirstLoad,
                ..
            }
        ));
        assert!(app.data.sources.loading);
    }

    #[test]
    fn the_last_screen_is_restored_when_asked() {
        let mut init = fixtures::init(true);
        init.settings.start_screen = StartScreen::Last;
        init.last_screen = Some("destinations".into());
        let mut app = App::new(init);
        app.start();
        assert_eq!(app.screen, Screen::Destinations);
    }

    #[test]
    fn startup_warnings_become_toasts() {
        let mut init = fixtures::init(true);
        init.warnings = vec!["ui.theme = \"neon\" is not valid".into()];
        let app = App::new(init);
        assert_eq!(app.toasts[0].tone, Tone::Warning);
    }

    #[test]
    fn results_for_a_screen_that_is_gone_are_dropped() {
        let mut app = fixtures::app();
        let old_generation = app.generation;
        app.switch_to(Section::Destinations);
        let before = app.data.sources.clone();
        update(
            &mut app,
            Action::Fetched {
                request: Request::Sources { search: None },
                generation: old_generation,
                result: Ok(json!({"items": [], "total": 0})),
            },
        );
        assert_eq!(app.data.sources, before);
    }

    #[test]
    fn global_results_are_never_dropped() {
        let mut app = fixtures::app();
        app.switch_to(Section::Destinations);
        update(
            &mut app,
            Action::Fetched {
                request: Request::Me,
                generation: GLOBAL_GENERATION,
                result: Ok(json!({"workspace": {"id": "w", "name": "other", "plan": "team"}})),
            },
        );
        assert_eq!(app.session.workspace.as_ref().unwrap().name, "other");
    }

    #[test]
    fn a_rejected_config_key_returns_to_login() {
        let mut app = fixtures::app();
        fail_sources(&mut app, failure(401));
        assert_eq!(app.screen, Screen::Login);
        assert_eq!(app.login.error.as_deref(), Some(KEY_REJECTED_MESSAGE));
        assert!(!app.quit);
    }

    #[test]
    fn a_rejected_override_key_exits_with_a_message() {
        let mut app = fixtures::app();
        app.session.key_source = KeySource::Override;
        fail_sources(&mut app, failure(401));
        assert!(app.quit);
        assert_eq!(
            app.exit_error.as_deref(),
            Some(OVERRIDE_KEY_REJECTED_MESSAGE)
        );
    }

    #[test]
    fn a_404_on_the_open_resource_goes_back_with_a_toast() {
        let mut app = fixtures::app();
        app.open(Screen::SourceDetail {
            id: fixtures::STRIPE_ID.into(),
            tab: SourceTab::Overview,
        });
        let generation = app.generation;
        update(
            &mut app,
            Action::Fetched {
                request: Request::Source {
                    id: fixtures::STRIPE_ID.into(),
                },
                generation,
                result: Err(failure(404)),
            },
        );
        assert_eq!(app.screen, Screen::Sources);
        assert_eq!(app.toasts.last().unwrap().text, "No longer exists");
    }

    #[test]
    fn other_client_errors_become_a_toast_with_the_server_message() {
        let mut app = fixtures::app();
        fail_sources(
            &mut app,
            FetchError::Api(ApiError::synthetic(
                403,
                Some("forbidden"),
                "not in this workspace",
            )),
        );
        let toast = app.toasts.last().unwrap();
        assert_eq!(toast.text, "not in this workspace");
        assert_eq!(toast.tone, Tone::Danger);
        assert!(app.data.sources.stale);
    }

    #[test]
    fn a_429_pauses_polling_until_retry_after() {
        let mut app = fixtures::app();
        let limited = FetchError::Api(
            ApiError::synthetic(429, Some("rate_limited"), "slow down")
                .with_retry_after(Duration::from_secs(23)),
        );
        fail_sources(&mut app, limited);
        assert_eq!(
            app.rate_limited_until,
            Some(app.now + Duration::from_secs(23))
        );
        assert!(tick(&mut app, Duration::from_secs(10)).is_empty());
        let resumed = tick(&mut app, Duration::from_secs(15));
        assert!(app.rate_limited_until.is_none());
        assert!(!resumed.is_empty());
    }

    #[test]
    fn a_network_error_keeps_the_data_and_marks_it_stale() {
        let mut app = fixtures::app();
        fail_sources(&mut app, FetchError::Network("connection refused".into()));
        assert!(app.data.sources.value.is_some());
        assert!(app.data.sources.stale);
        assert!(app.poller.is_offline());
        let generation = app.generation;
        update(
            &mut app,
            Action::Fetched {
                request: Request::Sources { search: None },
                generation,
                result: Ok(json!({"items": [], "total": 0})),
            },
        );
        assert!(!app.poller.is_offline());
        assert!(!app.data.sources.stale);
    }

    #[test]
    fn server_errors_count_as_offline() {
        let mut app = fixtures::app();
        fail_sources(&mut app, failure(503));
        assert!(app.poller.is_offline());
        assert!(app.toasts.is_empty());
    }

    #[test]
    fn background_refreshes_follow_the_plan_interval() {
        let mut app = fixtures::app();
        let first = tick(&mut app, Duration::ZERO);
        assert_eq!(
            fetched_requests(&first),
            vec![Request::Sources { search: None }, Request::Connections]
        );
        assert!(matches!(
            first[0],
            Effect::Fetch {
                priority: Priority::Background,
                ..
            }
        ));
        assert!(fetched_requests(&tick(&mut app, Duration::from_secs(10))).is_empty());
        assert_eq!(
            fetched_requests(&tick(&mut app, Duration::from_secs(10))).len(),
            2
        );
    }

    #[test]
    fn a_skipped_fetch_clears_the_spinner() {
        let mut app = fixtures::app();
        tick(&mut app, Duration::ZERO);
        assert!(app.data.sources.loading);
        let generation = app.generation;
        update(
            &mut app,
            Action::FetchSkipped {
                request: Request::Sources { search: None },
                generation,
            },
        );
        assert!(!app.data.sources.loading);
    }

    #[test]
    fn toasts_expire_after_four_seconds() {
        let mut app = fixtures::app();
        app.toast("hello", Tone::Success);
        tick(&mut app, Duration::from_secs(3));
        assert_eq!(app.toasts.len(), 1);
        tick(&mut app, Duration::from_secs(1));
        assert!(app.toasts.is_empty());
    }

    #[test]
    fn lists_fill_the_name_cache_and_clamp_the_cursor() {
        let mut app = fixtures::app();
        app.cursors.sources = 2;
        let generation = app.generation;
        update(
            &mut app,
            Action::Fetched {
                request: Request::Sources { search: None },
                generation,
                result: Ok(json!({"items": [{"id": "new-id", "name": "renamed"}], "total": 1})),
            },
        );
        assert_eq!(app.cursors.sources, 0);
        assert_eq!(app.names.source("new-id"), "renamed");
    }

    #[test]
    fn the_budget_follows_the_plan_and_the_setting() {
        let mut app = fixtures::app();
        assert_eq!(app.budget_limit(), 30, "Free limits until /plans answers");
        let effects = update(
            &mut app,
            Action::Fetched {
                request: Request::Plans,
                generation: GLOBAL_GENERATION,
                result: Ok(json!({"items": [
                    {"id": "free", "limits": {"api_per_minute": 60}},
                    {"id": "pro", "limits": {"api_per_minute": 600}}
                ]})),
            },
        );
        assert_eq!(effects, vec![Effect::ConfigureBudget { limit: 300 }]);
        app.apply_settings(UiSettings {
            request_budget_percent: 20,
            ..app.settings.clone()
        });
        assert_eq!(app.budget_limit(), 120);
    }

    #[test]
    fn a_failed_plans_request_keeps_the_free_limits_quietly() {
        let mut app = fixtures::app();
        update(
            &mut app,
            Action::Fetched {
                request: Request::Plans,
                generation: GLOBAL_GENERATION,
                result: Err(failure(500)),
            },
        );
        assert_eq!(app.budget_limit(), 30);
        assert!(app.toasts.is_empty());
        assert!(!app.poller.is_offline());
    }

    #[test]
    fn a_successful_login_opens_sources() {
        let mut app = App::new(fixtures::init(false));
        app.start();
        let effects = update(
            &mut app,
            Action::LoginFinished(Ok(LoginSuccess {
                server: "https://hooks.internal".into(),
                me: Me {
                    workspace: Workspace {
                        id: "w".into(),
                        name: "acme".into(),
                        plan: "free".into(),
                    },
                },
            })),
        );
        assert_eq!(app.screen, Screen::Sources);
        assert_eq!(app.session.server, "https://hooks.internal");
        assert!(fetched_requests(&effects).contains(&Request::Plans));
        assert!(effects.contains(&Effect::ConfigureBudget { limit: 30 }));
    }

    #[test]
    fn a_failed_login_shows_the_message_inline() {
        let mut app = App::new(fixtures::init(false));
        app.start();
        app.login.submitting = true;
        update(
            &mut app,
            Action::LoginFinished(Err(KEY_REJECTED_MESSAGE.into())),
        );
        assert_eq!(app.screen, Screen::Login);
        assert!(!app.login.submitting);
        assert_eq!(app.login.error.as_deref(), Some(KEY_REJECTED_MESSAGE));
    }

    #[test]
    fn saved_settings_apply_only_after_the_write_succeeds() {
        let mut app = fixtures::app();
        app.switch_to(Section::Settings);
        app.pending_settings = Some(UiSettings {
            theme: ThemeChoice::Light,
            ..app.settings.clone()
        });
        let effects = update(&mut app, Action::SettingsSaved(Ok(())));
        assert_eq!(app.settings.theme, ThemeChoice::Light);
        assert_eq!(app.theme.palette, crate::tui::theme::LIGHT);
        assert_eq!(effects, vec![Effect::ConfigureBudget { limit: 30 }]);

        app.pending_settings = Some(app.settings.clone());
        update(&mut app, Action::SettingsSaved(Err("disk full".into())));
        assert_eq!(
            app.settings_form.as_ref().unwrap().error.as_deref(),
            Some("disk full")
        );
    }

    #[test]
    fn terminate_quits() {
        let mut app = fixtures::app();
        update(&mut app, Action::Terminate);
        assert!(app.quit);
    }

    #[test]
    fn opening_an_event_clears_the_previous_one_and_resets_its_view() {
        let mut app = fixtures::app();
        let now = app.now;
        app.data.event.finish(fixtures::event_detail(), now);
        app.event_screens.detail.wrap = true;
        let effects = app.open(Screen::EventDetail {
            id: "other-event".into(),
        });
        assert!(app.data.event.value.is_none());
        assert!(!app.event_screens.detail.wrap);
        assert_eq!(
            fetched_requests(&effects)[0],
            Request::Event {
                id: "other-event".into()
            }
        );
        assert!(app.in_scrollable_pane());
    }

    #[test]
    fn a_404_on_the_open_event_goes_back() {
        let mut app = fixtures::app();
        app.switch_to(Section::Events);
        app.open(Screen::EventDetail {
            id: fixtures::EVENT_ID.into(),
        });
        let generation = app.generation;
        update(
            &mut app,
            Action::Fetched {
                request: Request::Event {
                    id: fixtures::EVENT_ID.into(),
                },
                generation,
                result: Err(failure(404)),
            },
        );
        assert_eq!(app.screen, Screen::Events);
    }

    #[test]
    fn switching_sources_resets_the_tab_state_and_remembers_the_source() {
        let mut app = fixtures::source_detail(SourceTab::Events);
        app.enter();
        app.event_screens.source.page = 3;
        app.back();
        app.open(Screen::SourceDetail {
            id: fixtures::GITHUB_ID.into(),
            tab: SourceTab::Events,
        });
        assert_eq!(app.event_screens.source.page, 1);
        assert_eq!(
            app.event_screens.last_source_id.as_deref(),
            Some(fixtures::GITHUB_ID)
        );
    }

    #[test]
    fn event_screens_report_their_data_age() {
        let mut app = fixtures::app();
        app.screen = Screen::Stats;
        assert_eq!(app.primary_status(), Some((None, false)));
        app.screen = Screen::SourceDetail {
            id: fixtures::STRIPE_ID.into(),
            tab: SourceTab::Dlq,
        };
        assert_eq!(app.primary_status(), Some((None, false)));
    }

    use crate::tui::action::Mutation;

    fn replay() -> Mutation {
        Mutation::ReplayEvent {
            event_id: fixtures::EVENT_ID.into(),
            connection_ids: vec![fixtures::STRIPE_BILLING_ID.into()],
        }
    }

    #[test]
    fn a_successful_replay_toasts_the_count_and_refetches_the_event() {
        let mut app = fixtures::app();
        let effects = update(
            &mut app,
            Action::Mutated {
                mutation: replay(),
                result: Ok(json!({"created": 2})),
            },
        );
        let toast = app.toasts.last().unwrap();
        assert_eq!(toast.text, "2 deliveries queued");
        assert_eq!(toast.tone, Tone::Success);
        assert!(effects.contains(&Effect::Fetch {
            request: Request::Event {
                id: fixtures::EVENT_ID.into()
            },
            generation: app.generation,
            priority: Priority::User,
        }));
    }

    #[test]
    fn a_failed_mutation_closes_forms_and_shows_the_server_message() {
        let mut app = fixtures::app();
        let effects = update(
            &mut app,
            Action::Mutated {
                mutation: replay(),
                result: Err(FetchError::Api(ApiError::synthetic(
                    403,
                    Some("forbidden"),
                    "not allowed",
                ))),
            },
        );
        let toast = app.toasts.last().unwrap();
        assert_eq!(toast.text, "not allowed");
        assert_eq!(toast.tone, Tone::Danger);
        assert_eq!(
            fetched_requests(&effects).len(),
            1,
            "the event is refetched"
        );
    }

    #[test]
    fn a_rate_limited_mutation_pauses_polling() {
        let mut app = fixtures::app();
        update(
            &mut app,
            Action::Mutated {
                mutation: replay(),
                result: Err(FetchError::Api(
                    ApiError::synthetic(429, Some("rate_limited"), "slow down")
                        .with_retry_after(Duration::from_secs(9)),
                )),
            },
        );
        assert_eq!(
            app.rate_limited_until,
            Some(app.now + Duration::from_secs(9))
        );
    }

    #[test]
    fn a_bulk_resend_refreshes_the_visible_screen() {
        let mut app = fixtures::app();
        app.switch_to(Section::Dlq);
        let effects = update(
            &mut app,
            Action::Mutated {
                mutation: Mutation::ResendBulk {
                    connection_id: fixtures::STRIPE_BILLING_ID.into(),
                    statuses: vec!["exhausted".into()],
                    since: None,
                    until: None,
                },
                result: Ok(json!({"created": 1})),
            },
        );
        assert_eq!(app.toasts.last().unwrap().text, "1 delivery queued");
        assert!(fetched_requests(&effects).contains(&Request::DlqSummary {
            source_id: fixtures::STRIPE_ID.into()
        }));
    }
}
