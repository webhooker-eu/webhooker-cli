//! State of the Events, Live, event detail, DLQ and Stats screens, the API
//! paths they read, and what they keep fresh.

use std::cell::Cell;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

use crate::args::query_string;
use crate::tui::action::Request;
use crate::tui::app::App;
use crate::tui::forms::input::TextInput;
use crate::tui::model::{DlqEntry, DlqSummary, TailNotice};
use crate::tui::names::NameCache;
use crate::tui::poller::{list_interval, PlanTier};
use crate::tui::screen::{Screen, SourceTab};

pub const EVENTS_PAGE_SIZE: i64 = 50;
pub const LIVE_CAPACITY: usize = 500;
pub const DLQ_STATUSES: [&str; 2] = ["exhausted", "failed"];
/// Terminal events stop polling; `r` still refreshes them.
pub const SETTLED_EVENT_INTERVAL: Duration = Duration::from_secs(3_600);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TimeWindow {
    #[default]
    All,
    LastHour,
    LastDay,
    LastWeek,
    Custom,
}

impl TimeWindow {
    pub const ALL: [TimeWindow; 5] = [
        TimeWindow::All,
        TimeWindow::LastHour,
        TimeWindow::LastDay,
        TimeWindow::LastWeek,
        TimeWindow::Custom,
    ];

    pub fn value(self) -> &'static str {
        match self {
            TimeWindow::All => "all",
            TimeWindow::LastHour => "1h",
            TimeWindow::LastDay => "24h",
            TimeWindow::LastWeek => "7d",
            TimeWindow::Custom => "custom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TimeWindow::All => "all time",
            TimeWindow::LastHour => "last hour",
            TimeWindow::LastDay => "last 24h",
            TimeWindow::LastWeek => "last 7 days",
            TimeWindow::Custom => "custom",
        }
    }

    pub fn from_value(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|window| window.value() == value)
    }

    pub fn span(self) -> Option<TimeDelta> {
        match self {
            TimeWindow::LastHour => Some(TimeDelta::hours(1)),
            TimeWindow::LastDay => Some(TimeDelta::days(1)),
            TimeWindow::LastWeek => Some(TimeDelta::days(7)),
            TimeWindow::All | TimeWindow::Custom => None,
        }
    }
}

pub fn rfc3339(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn is_rfc3339(raw: &str) -> bool {
    DateTime::parse_from_rfc3339(raw).is_ok()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct EventFilter {
    pub source_id: Option<String>,
    pub verification: Option<String>,
    pub window: TimeWindow,
    /// Only used with `TimeWindow::Custom`.
    pub since: Option<String>,
    pub until: Option<String>,
    /// Public id substring.
    pub search: Option<String>,
}

impl EventFilter {
    pub fn bounds(&self, now: DateTime<Utc>) -> (Option<String>, Option<String>) {
        match self.window {
            TimeWindow::All => (None, None),
            TimeWindow::Custom => (self.since.clone(), self.until.clone()),
            window => (window.span().map(|span| rfc3339(now - span)), None),
        }
    }

    /// Whether a live notice belongs on page 1 of this filter. A custom
    /// window may end in the past, so it never takes live rows.
    pub fn accepts(&self, notice: &TailNotice) -> bool {
        self.window != TimeWindow::Custom
            && self
                .source_id
                .as_deref()
                .is_none_or(|source_id| source_id == notice.source_id)
            && self
                .verification
                .as_deref()
                .is_none_or(|status| status == notice.verification_status)
            && self
                .search
                .as_deref()
                .is_none_or(|term| notice.public_id.contains(term))
    }

    pub fn describe(&self, names: &NameCache, separator: &str) -> String {
        let source = self
            .source_id
            .as_deref()
            .map_or_else(|| "all sources".to_string(), |id| names.source(id));
        let window = match self.window {
            TimeWindow::Custom => format!(
                "{} .. {}",
                self.since.as_deref().unwrap_or("start"),
                self.until.as_deref().unwrap_or("now")
            ),
            other => other.label().to_string(),
        };
        let mut parts = vec![
            format!("source: {source}"),
            format!(
                "verification: {}",
                self.verification.as_deref().unwrap_or("any")
            ),
            format!("window: {window}"),
        ];
        if let Some(term) = &self.search {
            parts.push(format!("search: {term}"));
        }
        parts.join(&format!(" {separator} "))
    }
}

pub fn events_path(filter: &EventFilter, page: i64, now: DateTime<Utc>) -> String {
    let (after, before) = filter.bounds(now);
    format!(
        "/api/v1/events/{}",
        query_string(&[
            ("source_id", filter.source_id.clone()),
            ("verification_status", filter.verification.clone()),
            ("received_after", after),
            ("received_before", before),
            ("q", filter.search.clone()),
            ("page", Some(page.to_string())),
            ("limit", Some(EVENTS_PAGE_SIZE.to_string())),
        ])
    )
}

pub fn dlq_entries_path(
    source_id: &str,
    statuses: &[String],
    window: TimeWindow,
    now: DateTime<Utc>,
) -> String {
    let statuses = (!statuses.is_empty()).then(|| statuses.join(","));
    let after = window.span().map(|span| rfc3339(now - span));
    format!(
        "/api/v1/sources/{source_id}/dlq{}",
        query_string(&[
            ("status", statuses),
            ("received_after", after),
            ("limit", Some("200".to_string())),
        ])
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StatsRange {
    #[default]
    Day,
    Week,
    Month,
}

impl StatsRange {
    pub const ALL: [StatsRange; 3] = [StatsRange::Day, StatsRange::Week, StatsRange::Month];

    pub fn label(self) -> &'static str {
        match self {
            StatsRange::Day => "24h",
            StatsRange::Week => "7d",
            StatsRange::Month => "30d",
        }
    }

    pub fn span(self) -> TimeDelta {
        match self {
            StatsRange::Day => TimeDelta::days(1),
            StatsRange::Week => TimeDelta::days(7),
            StatsRange::Month => TimeDelta::days(30),
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|range| *range == self)
            .unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

pub fn stats_overview_path(range: StatsRange, now: DateTime<Utc>) -> String {
    format!(
        "/api/v1/stats/overview{}",
        query_string(&[("received_after", Some(rfc3339(now - range.span())))])
    )
}

pub fn source_volume_path(range: StatsRange, now: DateTime<Utc>) -> String {
    format!(
        "/api/v1/stats/volume-by-source{}",
        query_string(&[("received_after", Some(rfc3339(now - range.span())))])
    )
}

/// The Events screen, or the Events tab of a source (source forced).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventsScope {
    Global,
    Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsState {
    pub filter: EventFilter,
    pub page: i64,
    pub cursor: usize,
}

impl Default for EventsState {
    fn default() -> Self {
        Self {
            filter: EventFilter::default(),
            page: 1,
            cursor: 0,
        }
    }
}

/// Rows from the tail, newest first. While following, the selection stays on
/// the newest row; otherwise it stays on the row the user picked.
#[derive(Debug, Clone)]
pub struct LiveFeed {
    pub rows: Vec<crate::tui::model::EventSummary>,
    pub cursor: usize,
    pub follow: bool,
}

impl Default for LiveFeed {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            cursor: 0,
            follow: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailStatus {
    Connecting,
    Live,
    Reconnecting { reason: String },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailSubscription {
    pub source_id: String,
    pub subscription: u64,
    pub status: TailStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventPane {
    #[default]
    Headers,
    Body,
    Deliveries,
}

impl EventPane {
    pub fn next(self) -> Self {
        match self {
            EventPane::Headers => EventPane::Body,
            EventPane::Body => EventPane::Deliveries,
            EventPane::Deliveries => EventPane::Headers,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            EventPane::Headers => EventPane::Deliveries,
            EventPane::Body => EventPane::Headers,
            EventPane::Deliveries => EventPane::Body,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EventView {
    pub pane: EventPane,
    pub headers_scroll: u16,
    pub body_scroll: u16,
    /// Largest useful scroll of each pane, written by the view.
    pub headers_limit: Cell<u16>,
    pub body_limit: Cell<u16>,
    pub delivery_cursor: usize,
    /// Delivery whose attempts are expanded.
    pub expanded: Option<String>,
    pub wrap: bool,
    pub header_filter: Option<String>,
    /// The header filter being typed; while set, keys go to it.
    pub header_search: Option<TextInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DlqPane {
    #[default]
    Summary,
    Entries,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlqState {
    /// Picked on the DLQ screen; the source tab uses its own source.
    pub source_id: Option<String>,
    pub pane: DlqPane,
    pub summary_cursor: usize,
    pub entries_cursor: usize,
    pub statuses: Vec<String>,
    pub window: TimeWindow,
    /// Source whose DLQ the cursors belong to.
    pub viewed: Option<String>,
}

impl Default for DlqState {
    fn default() -> Self {
        Self {
            source_id: None,
            pane: DlqPane::Summary,
            summary_cursor: 0,
            entries_cursor: 0,
            statuses: DLQ_STATUSES
                .iter()
                .map(|status| status.to_string())
                .collect(),
            window: TimeWindow::All,
            viewed: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatsState {
    pub range: StatsRange,
}

/// Everything the event screens remember; filters persist for the session.
#[derive(Debug, Default)]
pub struct EventScreens {
    pub global: EventsState,
    pub source: EventsState,
    /// Source the `source` state belongs to.
    pub source_for: Option<String>,
    /// Filter and page whose rows `data.events` holds.
    pub shown: Option<(EventFilter, i64)>,
    pub detail: EventView,
    pub detail_for: Option<String>,
    pub live: LiveFeed,
    pub tail: Option<TailSubscription>,
    pub next_subscription: u64,
    pub dlq: DlqState,
    pub stats: StatsState,
    /// Last source opened; the DLQ screen starts on it.
    pub last_source_id: Option<String>,
}

impl EventScreens {
    pub fn events(&self, scope: EventsScope) -> &EventsState {
        match scope {
            EventsScope::Global => &self.global,
            EventsScope::Source => &self.source,
        }
    }

    pub fn events_mut(&mut self, scope: EventsScope) -> &mut EventsState {
        match scope {
            EventsScope::Global => &mut self.global,
            EventsScope::Source => &mut self.source,
        }
    }
}

impl App {
    pub fn visible_events_scope(&self) -> Option<EventsScope> {
        match &self.screen {
            Screen::Events => Some(EventsScope::Global),
            Screen::SourceDetail {
                tab: SourceTab::Events,
                ..
            } => Some(EventsScope::Source),
            _ => None,
        }
    }

    /// The filter of the visible events list, with the source forced on a
    /// source's Events tab.
    pub fn visible_events_filter(&self) -> Option<EventFilter> {
        match &self.screen {
            Screen::Events => Some(self.event_screens.global.filter.clone()),
            Screen::SourceDetail {
                id,
                tab: SourceTab::Events,
            } => Some(EventFilter {
                source_id: Some(id.clone()),
                ..self.event_screens.source.filter.clone()
            }),
            _ => None,
        }
    }

    pub fn visible_events_page(&self) -> i64 {
        self.visible_events_scope()
            .map_or(1, |scope| self.event_screens.events(scope).page)
    }

    /// The source whose DLQ is shown: the tab's source, else the picked one,
    /// else the last source opened, else the first source.
    pub fn dlq_source_id(&self) -> Option<String> {
        if let Screen::SourceDetail { id, .. } = &self.screen {
            return Some(id.clone());
        }
        self.event_screens
            .dlq
            .source_id
            .clone()
            .or_else(|| self.event_screens.last_source_id.clone())
            .or_else(|| {
                self.data
                    .sources
                    .value
                    .as_ref()
                    .and_then(|sources| sources.first())
                    .map(|source| source.id.clone())
            })
    }

    pub fn selected_dlq_summary(&self) -> Option<&DlqSummary> {
        self.data
            .dlq_summary
            .value
            .as_ref()?
            .items
            .get(self.event_screens.dlq.summary_cursor)
    }

    pub fn selected_dlq_entries(&self) -> Vec<&DlqEntry> {
        let Some(summary) = self.selected_dlq_summary() else {
            return Vec::new();
        };
        self.data
            .dlq_entries
            .value
            .iter()
            .flat_map(|page| page.items.iter())
            .filter(|entry| entry.connection_id == summary.connection_id)
            .collect()
    }
}

fn interval(tier: PlanTier, free_seconds: u64, paid_seconds: u64) -> Duration {
    Duration::from_secs(match tier {
        PlanTier::Free => free_seconds,
        PlanTier::Paid => paid_seconds,
    })
}

fn events_request(filter: EventFilter, page: i64, tier: PlanTier) -> (Request, Duration) {
    let every = if filter.source_id.is_some() {
        interval(tier, 60, 15)
    } else {
        interval(tier, 30, 10)
    };
    (Request::Events { filter, page }, every)
}

fn dlq_requests(app: &App, source_id: &str) -> Vec<(Request, Duration)> {
    let dlq = &app.event_screens.dlq;
    let every = Duration::from_secs(60);
    vec![
        (
            Request::DlqSummary {
                source_id: source_id.to_string(),
            },
            every,
        ),
        (
            Request::DlqEntries {
                source_id: source_id.to_string(),
                statuses: dlq.statuses.clone(),
                window: dlq.window,
            },
            every,
        ),
    ]
}

/// Requests of the event screens; `poller::schedule` covers the others.
pub fn schedule(app: &App) -> Vec<(Request, Duration)> {
    let tier = app.tier();
    let names = (Request::Sources { search: None }, list_interval(tier));
    match &app.screen {
        Screen::Events => {
            let state = &app.event_screens.global;
            vec![
                events_request(state.filter.clone(), state.page, tier),
                names,
            ]
        }
        Screen::SourceDetail {
            tab: SourceTab::Events,
            ..
        } => app
            .visible_events_filter()
            .map(|filter| vec![events_request(filter, app.event_screens.source.page, tier)])
            .unwrap_or_default(),
        Screen::SourceDetail {
            id,
            tab: SourceTab::Dlq,
        } => dlq_requests(app, id),
        Screen::Dlq => {
            let mut requests = vec![names];
            if let Some(source_id) = app.dlq_source_id() {
                requests.extend(dlq_requests(app, &source_id));
            }
            requests
        }
        Screen::Stats => {
            let range = app.event_screens.stats.range;
            let every = Duration::from_secs(120);
            vec![
                (Request::StatsOverview { range }, every),
                (Request::SourceVolume { range }, every),
            ]
        }
        Screen::EventDetail { id } => {
            let loaded = app
                .data
                .event
                .value
                .as_ref()
                .filter(|event| event.id == *id);
            let settled = loaded.is_some_and(|event| !event.has_transitional_deliveries());
            let every = if settled {
                SETTLED_EVENT_INTERVAL
            } else {
                interval(tier, 5, 2)
            };
            let mut requests = vec![(Request::Event { id: id.clone() }, every)];
            if let Some(event) = loaded {
                requests.push((
                    Request::SourceConnections {
                        source_id: event.source_id.clone(),
                    },
                    list_interval(tier),
                ));
            }
            requests
        }
        _ => Vec::new(),
    }
}

impl App {
    /// Called by `enter`: resets state that belongs to another resource.
    pub fn prepare_screen(&mut self) {
        match self.screen.clone() {
            Screen::SourceDetail { id, .. } => {
                if self.event_screens.source_for.as_deref() != Some(id.as_str()) {
                    self.event_screens.source = EventsState::default();
                    self.event_screens.source_for = Some(id.clone());
                }
                self.event_screens.last_source_id = Some(id);
            }
            Screen::EventDetail { id } => {
                if self.event_screens.detail_for.as_deref() != Some(id.as_str()) {
                    self.event_screens.detail = EventView::default();
                    self.event_screens.detail_for = Some(id);
                }
            }
            _ => {}
        }
        if let Some(filter) = self.visible_events_filter() {
            let shown = Some((filter, self.visible_events_page()));
            if self.event_screens.shown != shown {
                self.data.events = Default::default();
                self.event_screens.shown = shown;
            }
        }
        let on_dlq = matches!(
            self.screen,
            Screen::Dlq
                | Screen::SourceDetail {
                    tab: SourceTab::Dlq,
                    ..
                }
        );
        if on_dlq {
            let source = self.dlq_source_id();
            if self.event_screens.dlq.viewed != source {
                let dlq = &mut self.event_screens.dlq;
                dlq.pane = DlqPane::Summary;
                dlq.summary_cursor = 0;
                dlq.entries_cursor = 0;
                dlq.viewed = source;
                self.data.dlq_summary = Default::default();
                self.data.dlq_entries = Default::default();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn noon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap()
    }

    fn notice(source_id: &str, verification: &str, public_id: &str) -> TailNotice {
        TailNotice {
            event_id: "e1".into(),
            public_id: public_id.into(),
            source_id: source_id.into(),
            method: "POST".into(),
            received_at: "2026-09-23T12:00:00Z".into(),
            content_type: None,
            body_size: 2,
            verification_status: verification.into(),
        }
    }

    #[test]
    fn unfiltered_events_ask_for_page_one() {
        assert_eq!(
            events_path(&EventFilter::default(), 1, noon()),
            "/api/v1/events/?page=1&limit=50"
        );
    }

    #[test]
    fn relative_windows_become_received_after() {
        let filter = EventFilter {
            source_id: Some("s1".into()),
            verification: Some("failed".into()),
            window: TimeWindow::LastHour,
            search: Some("evt_1".into()),
            ..EventFilter::default()
        };
        assert_eq!(
            events_path(&filter, 3, noon()),
            "/api/v1/events/?source_id=s1&verification_status=failed&received_after=2026-09-23T11%3A00%3A00Z&q=evt_1&page=3&limit=50"
        );
    }

    #[test]
    fn a_custom_window_sends_both_bounds() {
        let filter = EventFilter {
            window: TimeWindow::Custom,
            since: Some("2026-09-20T00:00:00Z".into()),
            until: Some("2026-09-21T00:00:00Z".into()),
            ..EventFilter::default()
        };
        let path = events_path(&filter, 1, noon());
        assert!(
            path.contains("received_after=2026-09-20T00%3A00%3A00Z"),
            "{path}"
        );
        assert!(
            path.contains("received_before=2026-09-21T00%3A00%3A00Z"),
            "{path}"
        );
    }

    #[test]
    fn dlq_and_stats_paths() {
        assert_eq!(
            dlq_entries_path(
                "s1",
                &["exhausted".to_string(), "failed".to_string()],
                TimeWindow::LastDay,
                noon()
            ),
            "/api/v1/sources/s1/dlq?status=exhausted%2Cfailed&received_after=2026-09-22T12%3A00%3A00Z&limit=200"
        );
        assert_eq!(
            stats_overview_path(StatsRange::Week, noon()),
            "/api/v1/stats/overview?received_after=2026-09-16T12%3A00%3A00Z"
        );
        assert_eq!(
            source_volume_path(StatsRange::Month, noon()),
            "/api/v1/stats/volume-by-source?received_after=2026-08-24T12%3A00%3A00Z"
        );
    }

    #[test]
    fn a_filter_accepts_matching_notices_only() {
        let filter = EventFilter {
            source_id: Some("s1".into()),
            verification: Some("verified".into()),
            search: Some("8f2a".into()),
            ..EventFilter::default()
        };
        assert!(filter.accepts(&notice("s1", "verified", "evt_8f2a1b")));
        assert!(!filter.accepts(&notice("s2", "verified", "evt_8f2a1b")));
        assert!(!filter.accepts(&notice("s1", "failed", "evt_8f2a1b")));
        assert!(!filter.accepts(&notice("s1", "verified", "evt_0000")));
        let custom = EventFilter {
            window: TimeWindow::Custom,
            ..EventFilter::default()
        };
        assert!(!custom.accepts(&notice("s1", "verified", "evt_1")));
    }

    #[test]
    fn a_filter_describes_itself() {
        let mut names = NameCache::default();
        names.remember_sources([("s1", "stripe-prod")]);
        let filter = EventFilter {
            source_id: Some("s1".into()),
            window: TimeWindow::LastDay,
            ..EventFilter::default()
        };
        assert_eq!(
            filter.describe(&names, "·"),
            "source: stripe-prod · verification: any · window: last 24h"
        );
    }

    #[test]
    fn windows_and_ranges_round_trip_their_values() {
        for window in TimeWindow::ALL {
            assert_eq!(TimeWindow::from_value(window.value()), Some(window));
        }
        assert_eq!(StatsRange::Month.next(), StatsRange::Day);
        assert_eq!(StatsRange::Day.previous(), StatsRange::Month);
        assert!(is_rfc3339("2026-09-20T10:00:00Z"));
        assert!(!is_rfc3339("yesterday"));
    }

    #[test]
    fn defaults_start_on_page_one_following_the_feed() {
        assert_eq!(EventsState::default().page, 1);
        assert!(LiveFeed::default().follow);
        assert_eq!(
            DlqState::default().statuses,
            vec!["exhausted".to_string(), "failed".to_string()]
        );
    }

    use crate::tui::fixtures;

    fn requests(app: &App) -> Vec<(Request, u64)> {
        schedule(app)
            .into_iter()
            .map(|(request, every)| (request, every.as_secs()))
            .collect()
    }

    #[test]
    fn unfiltered_events_poll_faster_than_filtered_ones() {
        let mut app = fixtures::app();
        app.screen = Screen::Events;
        let unfiltered = requests(&app);
        assert!(matches!(
            unfiltered[0],
            (Request::Events { page: 1, .. }, 10)
        ));
        app.event_screens.global.filter.source_id = Some(fixtures::STRIPE_ID.into());
        assert!(matches!(requests(&app)[0], (Request::Events { .. }, 15)));
        app.session.workspace.as_mut().unwrap().plan = "free".into();
        assert!(matches!(requests(&app)[0], (Request::Events { .. }, 60)));
    }

    #[test]
    fn the_source_events_tab_forces_its_source() {
        let mut app = fixtures::source_detail(SourceTab::Events);
        app.event_screens.source.filter.verification = Some("failed".into());
        let Request::Events { filter, .. } = &requests(&app)[0].0 else {
            panic!("expected events");
        };
        assert_eq!(filter.source_id.as_deref(), Some(fixtures::STRIPE_ID));
        assert_eq!(filter.verification.as_deref(), Some("failed"));
    }

    #[test]
    fn dlq_and_stats_intervals() {
        let mut app = fixtures::app();
        app.screen = Screen::Dlq;
        let dlq = requests(&app);
        assert!(dlq.contains(&(
            Request::DlqSummary {
                source_id: fixtures::STRIPE_ID.into()
            },
            60
        )));
        app.screen = Screen::Stats;
        let stats = requests(&app);
        assert_eq!(
            stats,
            vec![
                (
                    Request::StatsOverview {
                        range: StatsRange::Day
                    },
                    120
                ),
                (
                    Request::SourceVolume {
                        range: StatsRange::Day
                    },
                    120
                ),
            ]
        );
    }

    #[test]
    fn the_dlq_starts_on_the_last_source_opened() {
        let mut app = fixtures::app();
        app.screen = Screen::Dlq;
        assert_eq!(app.dlq_source_id().as_deref(), Some(fixtures::STRIPE_ID));
        app.event_screens.last_source_id = Some(fixtures::GITHUB_ID.into());
        assert_eq!(app.dlq_source_id().as_deref(), Some(fixtures::GITHUB_ID));
        app.event_screens.dlq.source_id = Some(fixtures::SHOPIFY_ID.into());
        assert_eq!(app.dlq_source_id().as_deref(), Some(fixtures::SHOPIFY_ID));
    }

    #[test]
    fn an_event_polls_quickly_until_its_deliveries_settle() {
        let mut app = fixtures::app();
        app.screen = Screen::EventDetail {
            id: fixtures::EVENT_ID.into(),
        };
        assert!(matches!(requests(&app)[0], (Request::Event { .. }, 2)));
        let now = app.now;
        let mut event = fixtures::event_detail();
        event.deliveries[0].status = "pending".into();
        app.data.event.finish(event.clone(), now);
        let pending = requests(&app);
        assert!(matches!(pending[0], (Request::Event { .. }, 2)));
        assert!(pending.contains(&(
            Request::SourceConnections {
                source_id: fixtures::STRIPE_ID.into()
            },
            20
        )));
        event.deliveries[0].status = "succeeded".into();
        app.data.event.finish(event, now);
        assert!(matches!(requests(&app)[0], (Request::Event { .. }, 3_600)));
    }
}
