//! What the visible screen keeps fresh and when. Only the visible screen
//! polls; while offline, one probe per backoff step replaces the schedule.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::tui::action::Request;
use crate::tui::screen::{Screen, SourceTab};

pub const FIRST_OFFLINE_DELAY: Duration = Duration::from_secs(2);
pub const MAX_OFFLINE_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanTier {
    Free,
    /// Pro and Team poll faster.
    Paid,
}

impl PlanTier {
    pub fn from_plan(plan_id: &str) -> Self {
        match plan_id {
            "" | "free" => PlanTier::Free,
            _ => PlanTier::Paid,
        }
    }
}

pub fn list_interval(tier: PlanTier) -> Duration {
    match tier {
        PlanTier::Free => Duration::from_secs(60),
        PlanTier::Paid => Duration::from_secs(20),
    }
}

pub fn schedule(
    screen: &Screen,
    source_query: Option<&str>,
    tier: PlanTier,
) -> Vec<(Request, Duration)> {
    let every = list_interval(tier);
    let all_sources = Request::Sources { search: None };
    let requests = match screen {
        Screen::Sources => vec![
            Request::Sources {
                search: source_query.map(str::to_string),
            },
            Request::Connections,
        ],
        Screen::SourceDetail {
            id,
            tab: SourceTab::Connections,
        } => vec![
            Request::Source { id: id.clone() },
            Request::SourceConnections {
                source_id: id.clone(),
            },
        ],
        Screen::SourceDetail { id, .. } => vec![Request::Source { id: id.clone() }],
        Screen::Destinations => vec![Request::Destinations],
        Screen::DestinationDetail { id } => vec![
            Request::Destination { id: id.clone() },
            Request::Connections,
            all_sources,
        ],
        Screen::Connections => vec![Request::Connections, all_sources, Request::Destinations],
        Screen::ConnectionDetail { id } => vec![Request::Connection { id: id.clone() }],
        Screen::Login
        | Screen::Settings
        | Screen::Events
        | Screen::Dlq
        | Screen::Stats
        | Screen::Relay => Vec::new(),
    };
    requests
        .into_iter()
        .map(|request| (request, every))
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct Offline {
    delay: Duration,
    retry_at: Instant,
    /// A probe is out; its failure (and only one) doubles the delay.
    probing: bool,
}

#[derive(Debug, Default)]
pub struct Poller {
    last_requested: HashMap<Request, Instant>,
    offline: Option<Offline>,
}

impl Poller {
    pub fn mark(&mut self, request: &Request, now: Instant) {
        self.last_requested.insert(request.clone(), now);
    }

    /// Requests to refresh now; each returned request is marked as fetched.
    pub fn due(&mut self, schedule: &[(Request, Duration)], now: Instant) -> Vec<Request> {
        if let Some(offline) = &mut self.offline {
            if now < offline.retry_at {
                return Vec::new();
            }
            offline.retry_at = now + offline.delay;
            offline.probing = true;
            let probe: Vec<Request> = schedule
                .iter()
                .map(|(request, _)| request.clone())
                .collect();
            for request in &probe {
                self.last_requested.insert(request.clone(), now);
            }
            return probe;
        }
        let due: Vec<Request> = schedule
            .iter()
            .filter(|(request, every)| {
                self.last_requested
                    .get(request)
                    .is_none_or(|last| now.duration_since(*last) >= *every)
            })
            .map(|(request, _)| request.clone())
            .collect();
        for request in &due {
            self.mark(request, now);
        }
        due
    }

    pub fn network_failed(&mut self, now: Instant) {
        match &mut self.offline {
            None => {
                self.offline = Some(Offline {
                    delay: FIRST_OFFLINE_DELAY,
                    retry_at: now + FIRST_OFFLINE_DELAY,
                    probing: false,
                })
            }
            Some(offline) if offline.probing => {
                offline.delay = (offline.delay * 2).min(MAX_OFFLINE_DELAY);
                offline.retry_at = now + offline.delay;
                offline.probing = false;
            }
            Some(_) => {}
        }
    }

    pub fn network_recovered(&mut self) {
        self.offline = None;
    }

    pub fn is_offline(&self) -> bool {
        self.offline.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    #[test]
    fn schedules_follow_the_screen_and_the_plan() {
        let free = schedule(&Screen::Sources, None, PlanTier::Free);
        assert_eq!(
            free,
            vec![
                (Request::Sources { search: None }, seconds(60)),
                (Request::Connections, seconds(60)),
            ]
        );
        let paid = schedule(&Screen::Sources, Some("str"), PlanTier::Paid);
        assert_eq!(
            paid[0],
            (
                Request::Sources {
                    search: Some("str".into())
                },
                seconds(20)
            )
        );
        let connections_tab = schedule(
            &Screen::SourceDetail {
                id: "s1".into(),
                tab: SourceTab::Connections,
            },
            None,
            PlanTier::Free,
        );
        assert_eq!(connections_tab.len(), 2);
        assert!(schedule(&Screen::Settings, None, PlanTier::Paid).is_empty());
    }

    #[test]
    fn plan_tiers() {
        assert_eq!(PlanTier::from_plan("free"), PlanTier::Free);
        assert_eq!(PlanTier::from_plan(""), PlanTier::Free);
        assert_eq!(PlanTier::from_plan("pro"), PlanTier::Paid);
        assert_eq!(PlanTier::from_plan("team"), PlanTier::Paid);
    }

    #[test]
    fn a_request_is_due_once_its_interval_elapsed() {
        let start = Instant::now();
        let plan = vec![(Request::Destinations, seconds(60))];
        let mut poller = Poller::default();
        assert_eq!(poller.due(&plan, start), vec![Request::Destinations]);
        assert!(poller.due(&plan, start + seconds(30)).is_empty());
        assert_eq!(
            poller.due(&plan, start + seconds(60)),
            vec![Request::Destinations]
        );
    }

    #[test]
    fn a_marked_request_counts_as_fetched() {
        let start = Instant::now();
        let plan = vec![(Request::Destinations, seconds(60))];
        let mut poller = Poller::default();
        poller.mark(&Request::Destinations, start);
        assert!(poller.due(&plan, start + seconds(10)).is_empty());
    }

    #[test]
    fn offline_probes_back_off_from_2_to_30_seconds() {
        let start = Instant::now();
        let plan = vec![(Request::Destinations, seconds(60))];
        let mut poller = Poller::default();
        poller.network_failed(start);
        assert!(poller.is_offline());
        let mut now = start;
        for expected_delay in [2, 4, 8, 16, 30, 30] {
            assert!(poller
                .due(&plan, now + seconds(expected_delay - 1))
                .is_empty());
            now += seconds(expected_delay);
            assert_eq!(poller.due(&plan, now), vec![Request::Destinations]);
            poller.network_failed(now);
        }
    }

    #[test]
    fn failures_from_one_round_back_off_once_and_success_resets() {
        let start = Instant::now();
        let plan = vec![(Request::Destinations, seconds(60))];
        let mut poller = Poller::default();
        poller.network_failed(start);
        poller.network_failed(start);
        assert_eq!(
            poller.due(&plan, start + seconds(2)),
            vec![Request::Destinations]
        );
        poller.network_recovered();
        assert!(!poller.is_offline());
    }
}
