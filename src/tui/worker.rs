//! Runs `Effect`s on tokio tasks and reports back as `Action`s. Owns the API
//! client (replaced on login) and the request budget shared by every request,
//! including the streams Plans 3 and 4 open.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use tokio::sync::mpsc;

use crate::client::ApiClient;
use crate::config::{self, UiSection};
use crate::tui::action::{Action, Effect, FetchError, LoginSuccess, Mutation, Request};
use crate::tui::app::{DEFAULT_RETRY_AFTER, KEY_REJECTED_MESSAGE};
use crate::tui::budget::{Budget, Decision, Priority};
use crate::tui::model::Me;

struct Shared {
    client: RwLock<Option<Arc<ApiClient>>>,
    budget: Mutex<Budget>,
    config_path: PathBuf,
    actions: mpsc::UnboundedSender<Action>,
}

#[derive(Clone)]
pub struct Worker {
    shared: Arc<Shared>,
}

impl Worker {
    pub fn new(
        client: Option<ApiClient>,
        budget_limit: u32,
        config_path: PathBuf,
        actions: mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                client: RwLock::new(client.map(Arc::new)),
                budget: Mutex::new(Budget::new(budget_limit)),
                config_path,
                actions,
            }),
        }
    }

    pub fn client(&self) -> Option<Arc<ApiClient>> {
        self.shared.client.read().unwrap().clone()
    }

    pub fn actions(&self) -> mpsc::UnboundedSender<Action> {
        self.shared.actions.clone()
    }

    /// One request's worth of budget, for callers outside `run` (streams).
    pub fn acquire(&self, priority: Priority) -> Decision {
        self.shared
            .budget
            .lock()
            .unwrap()
            .acquire(priority, Instant::now())
    }

    pub fn run(&self, effect: Effect) {
        let shared = self.shared.clone();
        match effect {
            Effect::ConfigureBudget { limit } => shared.budget.lock().unwrap().set_limit(limit),
            Effect::Fetch {
                request,
                generation,
                priority,
            } => {
                tokio::spawn(fetch(shared, request, generation, priority));
            }
            Effect::Login { server, api_key } => {
                tokio::spawn(login(shared, server, api_key));
            }
            Effect::SaveSettings(section) => {
                tokio::spawn(save_settings(shared, *section));
            }
            Effect::Mutate { mutation } => {
                tokio::spawn(mutate(shared, mutation));
            }
        }
    }
}

async fn fetch(shared: Arc<Shared>, request: Request, generation: u64, priority: Priority) {
    if request.is_metered() {
        loop {
            let decision = shared
                .budget
                .lock()
                .unwrap()
                .acquire(priority, Instant::now());
            match decision {
                Decision::Go => break,
                Decision::Skip => {
                    let _ = shared.actions.send(Action::FetchSkipped {
                        request,
                        generation,
                    });
                    return;
                }
                Decision::Wait(delay) => tokio::time::sleep(delay).await,
            }
        }
    }
    let client = shared.client.read().unwrap().clone();
    let result = match client {
        Some(client) => client
            .get_json(&request.path())
            .await
            .map_err(FetchError::from_anyhow),
        None => Err(FetchError::Network("not logged in".to_string())),
    };
    if let Err(FetchError::Api(error)) = &result {
        if error.status == 429 {
            let until = Instant::now() + error.retry_after.unwrap_or(DEFAULT_RETRY_AFTER);
            shared.budget.lock().unwrap().pause_until(until);
        }
    }
    let _ = shared.actions.send(Action::Fetched {
        request,
        generation,
        result,
    });
}

async fn mutate(shared: Arc<Shared>, mutation: Mutation) {
    shared
        .budget
        .lock()
        .unwrap()
        .acquire(Priority::User, Instant::now());
    let client = shared.client.read().unwrap().clone();
    let result = match client {
        Some(client) => send_mutation(&client, &mutation)
            .await
            .map_err(FetchError::from_anyhow),
        None => Err(FetchError::Network("not logged in".to_string())),
    };
    if let Err(FetchError::Api(error)) = &result {
        if error.status == 429 {
            let until = Instant::now() + error.retry_after.unwrap_or(DEFAULT_RETRY_AFTER);
            shared.budget.lock().unwrap().pause_until(until);
        }
    }
    let _ = shared.actions.send(Action::Mutated { mutation, result });
}

async fn send_mutation(
    client: &ApiClient,
    mutation: &Mutation,
) -> anyhow::Result<serde_json::Value> {
    let method = mutation.method();
    let path = mutation.path();
    let body = mutation.body().unwrap_or_else(|| serde_json::json!({}));
    if method == reqwest::Method::POST {
        client.post_json(&path, body).await
    } else if method == reqwest::Method::PATCH {
        client.patch_json(&path, body).await
    } else if method == reqwest::Method::DELETE {
        client.delete(&path).await.map(|()| serde_json::Value::Null)
    } else {
        anyhow::bail!("unsupported method {method}")
    }
}

async fn login(shared: Arc<Shared>, server: String, api_key: String) {
    let result = validate_and_save(&shared, server, api_key).await;
    let _ = shared.actions.send(Action::LoginFinished(result));
}

async fn validate_and_save(
    shared: &Shared,
    server: String,
    api_key: String,
) -> Result<LoginSuccess, String> {
    let client =
        ApiClient::new(server.clone(), api_key.clone()).map_err(|error| error.to_string())?;
    let body =
        client.get_json("/api/v1/me").await.map_err(|error| {
            match FetchError::from_anyhow(error) {
                FetchError::Api(api) if api.status == 401 => KEY_REJECTED_MESSAGE.to_string(),
                other => other.message(),
            }
        })?;
    let me: Me = serde_json::from_value(body)
        .map_err(|error| format!("Unexpected response from the server: {error}"))?;
    config::update(&shared.config_path, |config| {
        config.server = server.clone();
        config.api_key = api_key.clone();
    })
    .map_err(|error| format!("Could not save the key: {error:#}"))?;
    *shared.client.write().unwrap() = Some(Arc::new(client));
    Ok(LoginSuccess { server, me })
}

async fn save_settings(shared: Arc<Shared>, section: UiSection) {
    let result = config::update(&shared.config_path, |config| {
        let state = std::mem::take(&mut config.ui.state);
        config.ui = UiSection { state, ..section };
    })
    .map(|_| ())
    .map_err(|error| format!("Could not save settings: {error:#}"));
    let _ = shared.actions.send(Action::SettingsSaved(result));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::UiState;
    use crate::tui::action::Mutation;
    use crate::tui::app::KEY_REJECTED_MESSAGE;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn worker_for(
        server: &MockServer,
        limit: u32,
        config_path: PathBuf,
    ) -> (Worker, mpsc::UnboundedReceiver<Action>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let client = ApiClient::new(server.uri(), "whk_old".to_string()).unwrap();
        (
            Worker::new(Some(client), limit, config_path, sender),
            receiver,
        )
    }

    async fn next(receiver: &mut mpsc::UnboundedReceiver<Action>) -> Action {
        tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("the worker must answer")
            .expect("the channel must stay open")
    }

    fn fetch(request: Request, priority: Priority) -> Effect {
        Effect::Fetch {
            request,
            generation: 7,
            priority,
        }
    }

    async fn mount_destinations(server: &MockServer, key: &str) {
        Mock::given(method("GET"))
            .and(path("/api/v1/destinations/"))
            .and(header("authorization", format!("Bearer {key}").as_str()))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn a_fetch_reports_the_parsed_body() {
        let server = MockServer::start().await;
        mount_destinations(&server, "whk_old").await;
        let directory = tempfile::tempdir().unwrap();
        let (worker, mut receiver) = worker_for(&server, 30, directory.path().join("c.toml"));
        worker.run(fetch(Request::Destinations, Priority::User));
        assert_eq!(
            next(&mut receiver).await,
            Action::Fetched {
                request: Request::Destinations,
                generation: 7,
                result: Ok(serde_json::json!({"items": []})),
            }
        );
    }

    #[tokio::test]
    async fn a_429_pauses_background_fetches() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/destinations/"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "30")
                    .set_body_json(serde_json::json!({
                        "error": {"code": "rate_limited", "message": "rate limit exceeded"}
                    })),
            )
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let (worker, mut receiver) = worker_for(&server, 30, directory.path().join("c.toml"));
        worker.run(fetch(Request::Destinations, Priority::User));
        let Action::Fetched {
            result: Err(FetchError::Api(error)),
            ..
        } = next(&mut receiver).await
        else {
            panic!("expected an API error");
        };
        assert_eq!(error.status, 429);
        worker.run(fetch(Request::Destinations, Priority::Background));
        assert_eq!(
            next(&mut receiver).await,
            Action::FetchSkipped {
                request: Request::Destinations,
                generation: 7
            }
        );
    }

    #[tokio::test]
    async fn background_fetches_stop_when_the_budget_is_spent() {
        let server = MockServer::start().await;
        mount_destinations(&server, "whk_old").await;
        let directory = tempfile::tempdir().unwrap();
        let (worker, mut receiver) = worker_for(&server, 1, directory.path().join("c.toml"));
        worker.run(fetch(Request::Destinations, Priority::Background));
        worker.run(fetch(Request::Destinations, Priority::Background));
        let first = next(&mut receiver).await;
        let second = next(&mut receiver).await;
        let skipped = [&first, &second]
            .iter()
            .filter(|action| matches!(action, Action::FetchSkipped { .. }))
            .count();
        assert_eq!(skipped, 1, "{first:?} / {second:?}");
    }

    #[tokio::test]
    async fn plans_never_spend_budget() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/plans"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let (worker, mut receiver) = worker_for(&server, 1, directory.path().join("c.toml"));
        worker.run(Effect::ConfigureBudget { limit: 1 });
        assert_eq!(worker.acquire(Priority::Background), Decision::Go);
        worker.run(fetch(Request::Plans, Priority::Background));
        assert!(matches!(
            next(&mut receiver).await,
            Action::Fetched { result: Ok(_), .. }
        ));
    }

    #[tokio::test]
    async fn login_validates_saves_and_switches_the_client() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .and(header("authorization", "Bearer whk_new"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "auth": "api_key",
                "workspace": {"id": "w", "name": "acme", "plan": "pro"}
            })))
            .mount(&server)
            .await;
        mount_destinations(&server, "whk_new").await;
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("webhooker").join("config.toml");
        let (worker, mut receiver) = worker_for(&server, 30, config_path.clone());

        worker.run(Effect::Login {
            server: server.uri(),
            api_key: "whk_new".into(),
        });
        let Action::LoginFinished(Ok(success)) = next(&mut receiver).await else {
            panic!("login must succeed");
        };
        assert_eq!(success.me.workspace.name, "acme");
        let saved = crate::config::load(&config_path).unwrap().unwrap();
        assert_eq!(saved.api_key, "whk_new");
        assert_eq!(saved.server, server.uri());

        worker.run(fetch(Request::Destinations, Priority::User));
        assert!(matches!(
            next(&mut receiver).await,
            Action::Fetched { result: Ok(_), .. }
        ));
    }

    #[tokio::test]
    async fn a_rejected_login_reports_inline_and_saves_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/me"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.toml");
        let (worker, mut receiver) = worker_for(&server, 30, config_path.clone());
        worker.run(Effect::Login {
            server: server.uri(),
            api_key: "whk_bad".into(),
        });
        assert_eq!(
            next(&mut receiver).await,
            Action::LoginFinished(Err(KEY_REJECTED_MESSAGE.to_string()))
        );
        assert!(!config_path.exists());
    }

    #[tokio::test]
    async fn saving_settings_keeps_the_key_and_the_remembered_state() {
        let server = MockServer::start().await;
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.toml");
        let mut existing = crate::config::Config::new("https://hooks.internal", "whk_saved");
        existing.ui.state = UiState {
            last_relay_url: Some("http://localhost:4000".into()),
            last_screen: None,
        };
        crate::config::save(&config_path, &existing).unwrap();
        let (worker, mut receiver) = worker_for(&server, 30, config_path.clone());

        let section = crate::config::UiSection {
            theme: Some("light".into()),
            ..crate::config::UiSection::default()
        };
        worker.run(Effect::SaveSettings(Box::new(section)));
        assert_eq!(next(&mut receiver).await, Action::SettingsSaved(Ok(())));
        let saved = crate::config::load(&config_path).unwrap().unwrap();
        assert_eq!(saved.api_key, "whk_saved");
        assert_eq!(saved.ui.theme.as_deref(), Some("light"));
        assert_eq!(
            saved.ui.state.last_relay_url.as_deref(),
            Some("http://localhost:4000")
        );
    }

    #[tokio::test]
    async fn a_mutation_posts_its_body_and_reports_the_answer() {
        use wiremock::matchers::body_json;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/events/e1/resend"))
            .and(body_json(serde_json::json!({"connection_ids": ["c1"]})))
            .respond_with(
                ResponseTemplate::new(202).set_body_json(serde_json::json!({"created": 1})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let (worker, mut receiver) = worker_for(&server, 1, directory.path().join("c.toml"));
        let mutation = Mutation::ReplayEvent {
            event_id: "e1".into(),
            connection_ids: vec!["c1".into()],
        };
        worker.run(Effect::Mutate {
            mutation: mutation.clone(),
        });
        assert_eq!(
            next(&mut receiver).await,
            Action::Mutated {
                mutation,
                result: Ok(serde_json::json!({"created": 1})),
            }
        );
    }

    #[tokio::test]
    async fn a_failed_mutation_is_reported_and_never_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/deliveries/resend-bulk"))
            .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
                "error": {"code": "validation_error", "message": "unknown connection"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let (worker, mut receiver) = worker_for(&server, 30, directory.path().join("c.toml"));
        worker.run(Effect::Mutate {
            mutation: Mutation::ResendBulk {
                connection_id: "c9".into(),
                statuses: vec![],
                since: None,
                until: None,
            },
        });
        let Action::Mutated {
            result: Err(error), ..
        } = next(&mut receiver).await
        else {
            panic!("expected a failure");
        };
        assert_eq!(error.message(), "unknown connection");
    }
}
