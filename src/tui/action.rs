//! The messages of the TUI: `Action`s flow into `update`, `Effect`s flow out
//! to the worker.

use std::time::Instant;

use chrono::{DateTime, Utc};
use ratatui::crossterm::event::KeyEvent;
use serde_json::Value;

use crate::args::query_string;
use crate::client::ApiError;
use crate::config::UiSection;
use crate::tui::budget::Priority;
use crate::tui::model::Me;

/// Generation of fetches that belong to no screen (workspace, plans); their
/// results are never dropped. Screen generations start at 1.
pub const GLOBAL_GENERATION: u64 = 0;

/// A read the worker performs against the API.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Request {
    Me,
    Plans,
    Sources { search: Option<String> },
    Source { id: String },
    SourceConnections { source_id: String },
    Destinations,
    Destination { id: String },
    Connections,
    Connection { id: String },
}

impl Request {
    pub fn path(&self) -> String {
        match self {
            Request::Me => "/api/v1/me".to_string(),
            Request::Plans => "/api/v1/plans".to_string(),
            Request::Sources { search } => format!(
                "/api/v1/sources/{}",
                query_string(&[("q", search.clone()), ("limit", Some("200".to_string()))])
            ),
            Request::Source { id } => format!("/api/v1/sources/{id}"),
            Request::SourceConnections { source_id } => {
                format!("/api/v1/sources/{source_id}/connections?limit=200")
            }
            Request::Destinations => "/api/v1/destinations/".to_string(),
            Request::Destination { id } => format!("/api/v1/destinations/{id}"),
            Request::Connections => "/api/v1/connections/".to_string(),
            Request::Connection { id } => format!("/api/v1/connections/{id}"),
        }
    }

    /// `/plans` is public and not rate limited, so it never spends budget.
    pub fn is_metered(&self) -> bool {
        !matches!(self, Request::Plans)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FetchError {
    /// The API answered with a non-2xx status.
    Api(ApiError),
    /// No answer: connection, TLS or decoding failure.
    Network(String),
}

impl FetchError {
    pub fn from_anyhow(error: anyhow::Error) -> Self {
        match error.downcast::<ApiError>() {
            Ok(api) => Self::Api(api),
            Err(other) => Self::Network(format!("{other:#}")),
        }
    }

    /// The text to show the user: the server's own message when it sent one.
    pub fn message(&self) -> String {
        match self {
            Self::Api(error) if !error.message.is_empty() => error.message.clone(),
            Self::Api(error) => error.to_string(),
            Self::Network(message) => message.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoginSuccess {
    pub server: String,
    pub me: Me,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Key(KeyEvent),
    Resize {
        width: u16,
        height: u16,
    },
    Tick {
        now: Instant,
        wall_clock: DateTime<Utc>,
    },
    Fetched {
        request: Request,
        generation: u64,
        result: Result<Value, FetchError>,
    },
    /// The budget had nothing left for a background refresh.
    FetchSkipped {
        request: Request,
        generation: u64,
    },
    LoginFinished(Result<LoginSuccess, String>),
    SettingsSaved(Result<(), String>),
    /// SIGTERM, SIGHUP or the console window closing.
    Terminate,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Fetch {
        request: Request,
        generation: u64,
        priority: Priority,
    },
    ConfigureBudget {
        limit: u32,
    },
    Login {
        server: String,
        api_key: String,
    },
    /// The whole `[ui]` section; the worker keeps `[ui.state]` from disk.
    SaveSettings(Box<UiSection>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_paths_match_the_api() {
        assert_eq!(Request::Me.path(), "/api/v1/me");
        assert_eq!(
            Request::Sources { search: None }.path(),
            "/api/v1/sources/?limit=200"
        );
        assert_eq!(
            Request::Sources {
                search: Some("stripe prod".into())
            }
            .path(),
            "/api/v1/sources/?q=stripe%20prod&limit=200"
        );
        assert_eq!(
            Request::SourceConnections {
                source_id: "s1".into()
            }
            .path(),
            "/api/v1/sources/s1/connections?limit=200"
        );
        assert_eq!(Request::Destinations.path(), "/api/v1/destinations/");
        assert_eq!(
            Request::Connection { id: "c1".into() }.path(),
            "/api/v1/connections/c1"
        );
    }

    #[test]
    fn only_plans_is_free_of_the_rate_limit() {
        assert!(!Request::Plans.is_metered());
        assert!(Request::Me.is_metered());
        assert!(Request::Connections.is_metered());
    }

    #[test]
    fn fetch_errors_keep_api_errors_typed() {
        let api = anyhow::Error::new(ApiError::synthetic(403, Some("forbidden"), "no access"));
        assert_eq!(
            FetchError::from_anyhow(api),
            FetchError::Api(ApiError::synthetic(403, Some("forbidden"), "no access"))
        );
        let network = FetchError::from_anyhow(anyhow::anyhow!("connection refused"));
        assert_eq!(network, FetchError::Network("connection refused".into()));
        assert_eq!(network.message(), "connection refused");
        assert_eq!(
            FetchError::Api(ApiError::synthetic(403, None, "no access")).message(),
            "no access"
        );
    }
}
