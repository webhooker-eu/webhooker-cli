//! Client-side request budget. The workspace's rate limit is shared with the
//! user's scripts and CI, so the TUI spends at most a share of it: at most
//! `limit` requests in any 60-second window.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const WINDOW: Duration = Duration::from_secs(60);
/// Free plan's `api_per_minute`, used until `/plans` answers or when it fails.
pub const FREE_API_PER_MINUTE: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Periodic refreshes: skipped when no budget is left.
    Background,
    /// The first load of the visible screen: waits for budget.
    FirstLoad,
    /// Mutations, `Enter`, `r`: always sent, even into debt.
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Go,
    Wait(Duration),
    Skip,
}

/// `percent` of `api_per_minute`, at least one request.
pub fn share_of(api_per_minute: u32, percent: u8) -> u32 {
    (u64::from(api_per_minute) * u64::from(percent) / 100).max(1) as u32
}

#[derive(Debug)]
pub struct Budget {
    limit: u32,
    sent: VecDeque<Instant>,
    paused_until: Option<Instant>,
}

impl Budget {
    pub fn new(limit: u32) -> Self {
        Self {
            limit: limit.max(1),
            sent: VecDeque::new(),
            paused_until: None,
        }
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    pub fn set_limit(&mut self, limit: u32) {
        self.limit = limit.max(1);
    }

    pub fn acquire(&mut self, priority: Priority, now: Instant) -> Decision {
        self.forget_expired(now);
        if priority == Priority::User {
            self.sent.push_back(now);
            return Decision::Go;
        }
        if let Some(remaining) = self.paused_for(now) {
            return match priority {
                Priority::FirstLoad => Decision::Wait(remaining),
                _ => Decision::Skip,
            };
        }
        if (self.sent.len() as u32) < self.limit {
            self.sent.push_back(now);
            return Decision::Go;
        }
        match priority {
            Priority::FirstLoad => Decision::Wait(
                self.sent
                    .front()
                    .map(|oldest| (*oldest + WINDOW).saturating_duration_since(now))
                    .unwrap_or_default(),
            ),
            _ => Decision::Skip,
        }
    }

    /// After a 429: nothing but user actions until `until`.
    pub fn pause_until(&mut self, until: Instant) {
        self.paused_until = Some(match self.paused_until {
            Some(existing) if existing > until => existing,
            _ => until,
        });
    }

    pub fn paused_for(&self, now: Instant) -> Option<Duration> {
        self.paused_until
            .filter(|until| *until > now)
            .map(|until| until - now)
    }

    fn forget_expired(&mut self, now: Instant) {
        while self
            .sent
            .front()
            .is_some_and(|sent_at| *sent_at + WINDOW <= now)
        {
            self.sent.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    #[test]
    fn background_requests_stop_at_the_limit() {
        let start = Instant::now();
        let mut budget = Budget::new(2);
        assert_eq!(budget.acquire(Priority::Background, start), Decision::Go);
        assert_eq!(budget.acquire(Priority::Background, start), Decision::Go);
        assert_eq!(budget.acquire(Priority::Background, start), Decision::Skip);
    }

    #[test]
    fn a_first_load_waits_for_the_oldest_request_to_leave_the_window() {
        let start = Instant::now();
        let mut budget = Budget::new(1);
        assert_eq!(budget.acquire(Priority::Background, start), Decision::Go);
        assert_eq!(
            budget.acquire(Priority::FirstLoad, at(start, 20)),
            Decision::Wait(Duration::from_secs(40))
        );
    }

    #[test]
    fn user_actions_always_go_even_into_debt() {
        let start = Instant::now();
        let mut budget = Budget::new(1);
        assert_eq!(budget.acquire(Priority::User, start), Decision::Go);
        assert_eq!(budget.acquire(Priority::User, start), Decision::Go);
        assert_eq!(budget.acquire(Priority::Background, start), Decision::Skip);
        assert_eq!(
            budget.acquire(Priority::Background, at(start, 60)),
            Decision::Go
        );
    }

    #[test]
    fn the_window_slides() {
        let start = Instant::now();
        let mut budget = Budget::new(1);
        assert_eq!(budget.acquire(Priority::Background, start), Decision::Go);
        assert_eq!(
            budget.acquire(Priority::Background, at(start, 59)),
            Decision::Skip
        );
        assert_eq!(
            budget.acquire(Priority::Background, at(start, 60)),
            Decision::Go
        );
    }

    #[test]
    fn a_pause_skips_background_and_delays_first_loads() {
        let start = Instant::now();
        let mut budget = Budget::new(10);
        budget.pause_until(at(start, 23));
        assert_eq!(budget.paused_for(start), Some(Duration::from_secs(23)));
        assert_eq!(budget.acquire(Priority::Background, start), Decision::Skip);
        assert_eq!(
            budget.acquire(Priority::FirstLoad, start),
            Decision::Wait(Duration::from_secs(23))
        );
        assert_eq!(budget.acquire(Priority::User, start), Decision::Go);
        assert_eq!(
            budget.acquire(Priority::Background, at(start, 23)),
            Decision::Go
        );
        assert_eq!(budget.paused_for(at(start, 23)), None);
    }

    #[test]
    fn a_shorter_pause_never_shortens_a_longer_one() {
        let start = Instant::now();
        let mut budget = Budget::new(10);
        budget.pause_until(at(start, 30));
        budget.pause_until(at(start, 10));
        assert_eq!(budget.paused_for(start), Some(Duration::from_secs(30)));
    }

    #[test]
    fn the_share_of_the_plan_limit() {
        assert_eq!(share_of(60, 50), 30);
        assert_eq!(share_of(600, 50), 300);
        assert_eq!(share_of(1200, 50), 600);
        assert_eq!(share_of(60, 10), 6);
        assert_eq!(share_of(1, 10), 1);
    }
}
