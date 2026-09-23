use anyhow::Result;
use clap::{Parser, Subcommand};
use whk::commands::{connections, destinations, events, sources};
use whk::{args, client, config, listen, tail};

#[derive(Parser)]
#[command(
    name = "whk",
    version,
    about = "Webhooker CLI: manage sources, destinations and events, and relay webhooks to localhost"
)]
struct Cli {
    /// Server base URL (defaults to the saved config, then https://app.webhooker.eu)
    #[arg(long, global = true, env = "WEBHOOKER_SERVER")]
    server: Option<String>,
    /// API key (whk_...); overrides the saved config
    #[arg(long, global = true, env = "WEBHOOKER_API_KEY", hide_env_values = true)]
    api_key: Option<String>,
    /// Print the API's JSON response instead of a human-readable rendering
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate and save an API key
    Login,
    /// Show the authenticated workspace
    Whoami,
    /// Stream event metadata for a source
    Tail {
        /// Source name, id or ingest token
        source: String,
    },
    /// Forward full webhooks for a source to a local URL
    Listen {
        /// Source name, id or ingest token
        source: String,
        /// Local URL to forward each webhook to
        #[arg(long)]
        forward: String,
        /// Also forward events whose signature verification failed
        #[arg(long)]
        skip_verify: bool,
        /// Extra header for the local request, "Name: Value" (repeatable)
        #[arg(long = "header")]
        headers: Vec<String>,
    },
    /// Manage webhook sources and their ingest URLs
    Sources {
        #[command(subcommand)]
        command: SourceCommand,
    },
    /// Manage delivery destinations
    #[command(alias = "dests")]
    Destinations {
        #[command(subcommand)]
        command: DestinationCommand,
    },
    /// Manage source-to-destination connections
    Connections {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    /// Connect a source to a destination (shorthand for `connections create`)
    Connect {
        /// Source name, id or ingest token
        source: String,
        /// Destination name or id
        destination: String,
        /// Filter rules as JSON, @file.json or - for stdin
        #[arg(long)]
        filter: Option<String>,
        /// Transformation as JSON, @file.json or - for stdin
        #[arg(long)]
        transform: Option<String>,
    },
    /// Inspect and replay received events
    Events {
        #[command(subcommand)]
        command: EventCommand,
    },
    /// Remove the saved credentials
    Logout,
}

#[derive(Subcommand)]
enum SourceCommand {
    /// List active sources with their ingest URLs
    Ls {
        /// Filter by name substring
        #[arg(short = 'q', long = "query")]
        search: Option<String>,
        #[arg(long)]
        page: Option<i64>,
        #[arg(long)]
        limit: Option<i64>,
    },
    /// Create a source and print its ingest URL
    Create {
        name: String,
        /// Hex color for the dashboard, e.g. #3b82f6
        #[arg(long)]
        color: Option<String>,
        /// Signature verification: a provider name (stripe, github, shopify) to be
        /// prompted for the secret, "none" to turn it off, or a full config as
        /// JSON, @file.json or -
        #[arg(long)]
        verify: Option<String>,
    },
    /// Show one source
    Get {
        /// Source name, id or ingest token
        source: String,
    },
    /// Change a source's settings
    Update {
        /// Source name, id or ingest token
        source: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        color: Option<String>,
        /// active or paused
        #[arg(long)]
        status: Option<String>,
        /// Signature verification, as in `sources create`
        #[arg(long)]
        verify: Option<String>,
    },
    /// Move a source to the trash
    Rm {
        /// Source name, id or ingest token
        source: String,
    },
    /// List trashed sources
    Trash {
        #[arg(short = 'q', long = "query")]
        search: Option<String>,
        #[arg(long)]
        page: Option<i64>,
        #[arg(long)]
        limit: Option<i64>,
    },
    /// Restore a trashed source by id (names resolve to live sources only)
    Restore { source_id: String },
    /// Issue a new ingest token; the old URL stops working within 30 seconds
    RotateToken {
        /// Source name, id or ingest token
        source: String,
    },
    /// Print only the ingest URL, ready to pipe into curl
    Url {
        /// Source name, id or ingest token
        source: String,
    },
}

#[derive(Subcommand)]
enum DestinationCommand {
    /// List destinations
    Ls,
    /// Create a destination
    Create {
        name: String,
        /// Absolute http(s) URL webhooks are delivered to
        #[arg(long)]
        url: String,
        /// Extra header sent with every delivery, "Name: Value" (repeatable)
        #[arg(long = "header")]
        headers: Vec<String>,
        /// Outbound auth: "hmac" to be prompted for the secret, "none", or a full
        /// config as JSON, @file.json or -
        #[arg(long)]
        auth: Option<String>,
        #[arg(long)]
        timeout_ms: Option<i64>,
        /// Retry policy as JSON, @file.json or -
        #[arg(long)]
        retry: Option<String>,
    },
    /// Show one destination
    Get { destination: String },
    /// Change a destination's settings
    Update {
        destination: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        url: Option<String>,
        /// Replaces the whole header set; passing none at all leaves it unchanged
        #[arg(long = "header")]
        headers: Vec<String>,
        /// Outbound auth, as in `destinations create`
        #[arg(long)]
        auth: Option<String>,
        #[arg(long)]
        timeout_ms: Option<i64>,
        /// Retry policy as JSON, @file.json, - or null to reset to the default
        #[arg(long)]
        retry: Option<String>,
        /// active or paused
        #[arg(long)]
        status: Option<String>,
    },
    /// Delete a destination
    Rm { destination: String },
}

#[derive(Subcommand)]
enum ConnectionCommand {
    /// List connections
    Ls {
        /// Limit to one source (name, id or ingest token)
        #[arg(long)]
        source: Option<String>,
    },
    /// Connect a source to a destination
    Create {
        #[arg(long)]
        source: String,
        #[arg(long = "dest")]
        destination: String,
        /// Filter rules as JSON, @file.json or -
        #[arg(long)]
        filter: Option<String>,
        /// Transformation as JSON, @file.json or -
        #[arg(long)]
        transform: Option<String>,
    },
    /// Change a connection
    Update {
        connection_id: String,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
        /// Filter rules as JSON, @file.json, - or null to clear
        #[arg(long)]
        filter: Option<String>,
        /// Transformation as JSON, @file.json, - or null to clear
        #[arg(long)]
        transform: Option<String>,
    },
    /// Delete a connection
    Rm { connection_id: String },
}

#[derive(Subcommand)]
enum EventCommand {
    /// List received events
    Ls {
        /// Limit to one source (name, id or ingest token)
        #[arg(long)]
        source: Option<String>,
        /// Verification status: verified, failed or skipped
        #[arg(long)]
        status: Option<String>,
        /// RFC 3339 timestamp, e.g. 2026-09-20T10:00:00Z
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        /// Filter by public id substring
        #[arg(short = 'q', long = "query")]
        search: Option<String>,
        #[arg(long)]
        page: Option<i64>,
        #[arg(long)]
        limit: Option<i64>,
    },
    /// Show one event with its headers, body and deliveries
    Get {
        /// Event id or public id
        event: String,
    },
    /// Re-queue one event's deliveries
    Replay {
        /// Event id or public id
        event: String,
        /// Connection to replay to (repeatable); defaults to every connection
        #[arg(long = "connection")]
        connections: Vec<String>,
    },
    /// Re-queue a range of deliveries for one connection
    ReplayBulk {
        #[arg(long = "connection")]
        connection_id: String,
        /// Delivery status to replay (repeatable); defaults to exhausted
        #[arg(long = "status")]
        statuses: Vec<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let Cli {
        server,
        api_key,
        json,
        command,
    } = Cli::parse();

    match command {
        Command::Login => login(server, api_key).await,
        Command::Logout => logout(),
        Command::Whoami => {
            let credentials = effective_config(&server, &api_key)?;
            let client = client::ApiClient::new(credentials.server.clone(), credentials.api_key)?;
            let me = client.me().await?;
            println!("Server:    {}", credentials.server);
            println!(
                "Workspace: {} ({}), {} plan",
                me.workspace.name, me.workspace.id, me.workspace.plan
            );
            println!("Key:       {}…", client.api_key_prefix());
            Ok(())
        }
        Command::Tail { source } => {
            let client = connect(&server, &api_key)?;
            tail::run(&client, &source, json).await
        }
        Command::Listen {
            source,
            forward,
            skip_verify,
            headers,
        } => {
            let client = connect(&server, &api_key)?;
            listen::run(&client, &source, &forward, skip_verify, &headers, json).await
        }
        Command::Sources { command } => {
            run_source_command(connect(&server, &api_key)?, command, json).await
        }
        Command::Destinations { command } => {
            run_destination_command(connect(&server, &api_key)?, command, json).await
        }
        Command::Connections { command } => {
            run_connection_command(connect(&server, &api_key)?, command, json).await
        }
        Command::Connect {
            source,
            destination,
            filter,
            transform,
        } => {
            let client = connect(&server, &api_key)?;
            connections::create(
                &client,
                &source,
                &destination,
                optional_json(filter.as_deref())?,
                optional_json(transform.as_deref())?,
                json,
            )
            .await
        }
        Command::Events { command } => {
            run_event_command(connect(&server, &api_key)?, command, json).await
        }
    }
}

async fn run_source_command(
    client: client::ApiClient,
    command: SourceCommand,
    json: bool,
) -> Result<()> {
    match command {
        SourceCommand::Ls {
            search,
            page,
            limit,
        } => sources::list(&client, search.as_deref(), page, limit, json).await,
        SourceCommand::Create {
            name,
            color,
            verify,
        } => {
            sources::create(
                &client,
                &name,
                color.as_deref(),
                optional_verification(verify.as_deref())?,
                json,
            )
            .await
        }
        SourceCommand::Get { source } => sources::get(&client, &source, json).await,
        SourceCommand::Update {
            source,
            name,
            description,
            color,
            status,
            verify,
        } => {
            let fields = sources::UpdateFields {
                name: name.as_deref(),
                description: description.as_deref(),
                color: color.as_deref(),
                status: status.as_deref(),
                verification: optional_verification(verify.as_deref())?,
            };
            sources::update(&client, &source, fields, json).await
        }
        SourceCommand::Rm { source } => sources::remove(&client, &source, json).await,
        SourceCommand::Trash {
            search,
            page,
            limit,
        } => sources::trash(&client, search.as_deref(), page, limit, json).await,
        SourceCommand::Restore { source_id } => sources::restore(&client, &source_id, json).await,
        SourceCommand::RotateToken { source } => {
            sources::rotate_token(&client, &source, json).await
        }
        SourceCommand::Url { source } => sources::url(&client, &source).await,
    }
}

async fn run_destination_command(
    client: client::ApiClient,
    command: DestinationCommand,
    json: bool,
) -> Result<()> {
    match command {
        DestinationCommand::Ls => destinations::list(&client, json).await,
        DestinationCommand::Create {
            name,
            url,
            headers,
            auth,
            timeout_ms,
            retry,
        } => {
            let fields = destinations::CreateFields {
                name: &name,
                url: &url,
                headers: &headers,
                auth: optional_auth(auth.as_deref())?,
                timeout_ms,
                retry_policy: optional_json(retry.as_deref())?,
            };
            destinations::create(&client, fields, json).await
        }
        DestinationCommand::Get { destination } => {
            destinations::get(&client, &destination, json).await
        }
        DestinationCommand::Update {
            destination,
            name,
            url,
            headers,
            auth,
            timeout_ms,
            retry,
            status,
        } => {
            let fields = destinations::UpdateFields {
                name: name.as_deref(),
                url: url.as_deref(),
                headers: &headers,
                auth: optional_auth(auth.as_deref())?,
                timeout_ms,
                retry_policy: optional_json(retry.as_deref())?,
                status: status.as_deref(),
            };
            destinations::update(&client, &destination, fields, json).await
        }
        DestinationCommand::Rm { destination } => {
            destinations::remove(&client, &destination, json).await
        }
    }
}

async fn run_connection_command(
    client: client::ApiClient,
    command: ConnectionCommand,
    json: bool,
) -> Result<()> {
    match command {
        ConnectionCommand::Ls { source } => {
            connections::list(&client, source.as_deref(), json).await
        }
        ConnectionCommand::Create {
            source,
            destination,
            filter,
            transform,
        } => {
            connections::create(
                &client,
                &source,
                &destination,
                optional_json(filter.as_deref())?,
                optional_json(transform.as_deref())?,
                json,
            )
            .await
        }
        ConnectionCommand::Update {
            connection_id,
            enable,
            disable,
            filter,
            transform,
        } => {
            let fields = connections::UpdateFields {
                enabled: match (enable, disable) {
                    (true, _) => Some(true),
                    (_, true) => Some(false),
                    _ => None,
                },
                filter_rules: optional_json(filter.as_deref())?,
                transformation: optional_json(transform.as_deref())?,
            };
            connections::update(&client, &connection_id, fields, json).await
        }
        ConnectionCommand::Rm { connection_id } => {
            connections::remove(&client, &connection_id, json).await
        }
    }
}

async fn run_event_command(
    client: client::ApiClient,
    command: EventCommand,
    json: bool,
) -> Result<()> {
    match command {
        EventCommand::Ls {
            source,
            status,
            since,
            until,
            search,
            page,
            limit,
        } => {
            let filters = events::ListFilters {
                source: source.as_deref(),
                verification_status: status.as_deref(),
                since: since.as_deref(),
                until: until.as_deref(),
                search: search.as_deref(),
                page,
                limit,
            };
            events::list(&client, filters, json).await
        }
        EventCommand::Get { event } => events::get(&client, &event, json).await,
        EventCommand::Replay { event, connections } => {
            events::replay(&client, &event, &connections, json).await
        }
        EventCommand::ReplayBulk {
            connection_id,
            statuses,
            since,
            until,
        } => {
            events::replay_bulk(
                &client,
                &connection_id,
                &statuses,
                since.as_deref(),
                until.as_deref(),
                json,
            )
            .await
        }
    }
}

async fn login(server: Option<String>, api_key: Option<String>) -> Result<()> {
    let path = config::default_path()?;
    // The key must always be supplied afresh, but the saved server is
    // reused so a self-hosted install is not silently repointed. An
    // unreadable config must not block re-login: it is about to be
    // overwritten anyway.
    let saved = config::load(&path).unwrap_or_default();
    let server = config::resolve_server(server, saved.as_ref());
    let key = match api_key {
        Some(key) => key,
        None => rpassword::prompt_password("API key (whk_...): ")?,
    };
    let client = client::ApiClient::new(server.clone(), key.clone())?;
    let me = client.me().await?; // validates the key
    let saved = config::update(&path, |config| {
        config.server = server.clone();
        config.api_key = key.clone();
    });
    if saved.is_err() {
        // An unreadable config is overwritten, as before: it holds nothing
        // this login could preserve.
        config::save(&path, &config::Config::new(server.clone(), key))?;
    }
    println!(
        "Logged in to {server} (workspace \"{}\", {} plan). Saved to {}",
        me.workspace.name,
        me.workspace.plan,
        path.display()
    );
    Ok(())
}

fn logout() -> Result<()> {
    let path = config::default_path()?;
    if config::delete(&path)? {
        println!("Removed {}", path.display());
    } else {
        println!("No saved credentials at {}", path.display());
    }
    Ok(())
}

fn connect(server: &Option<String>, api_key: &Option<String>) -> Result<client::ApiClient> {
    let credentials = effective_config(server, api_key)?;
    client::ApiClient::new(credentials.server, credentials.api_key)
}

fn effective_config(server: &Option<String>, api_key: &Option<String>) -> Result<config::Config> {
    let saved = config::load(&config::default_path()?)?;
    config::resolve(server.clone(), api_key.clone(), saved)
}

fn optional_json(raw: Option<&str>) -> Result<Option<serde_json::Value>> {
    raw.map(args::parse_json_arg).transpose()
}

fn optional_verification(raw: Option<&str>) -> Result<Option<serde_json::Value>> {
    raw.map(|raw| {
        args::parse_verification_arg(raw, || Ok(rpassword::prompt_password("Signing secret: ")?))
    })
    .transpose()
}

fn optional_auth(raw: Option<&str>) -> Result<Option<serde_json::Value>> {
    raw.map(|raw| {
        destinations::parse_auth_arg(raw, || {
            Ok(rpassword::prompt_password("Outbound signing secret: ")?)
        })
    })
    .transpose()
}
