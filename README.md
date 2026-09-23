<h1 align="center">Webhooker CLI (whk)</h1>

<p align="center">
  Receive real webhooks on localhost without a tunnel, and manage your Webhooker sources,
  destinations and events from the terminal.
</p>

<p align="center">
  <a href="https://webhooker.eu/">webhooker.eu</a> ·
  <a href="#install">Install</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#commands">Commands</a> ·
  <a href="#faq">FAQ</a> ·
  <a href="https://docs.webhooker.eu/">Docs</a> ·
  <a href="https://github.com/webhooker-eu">More tools</a>
</p>

<p align="center">
  <a href="https://webhooker.eu/"><img src="https://img.shields.io/badge/made%20by-Webhooker-0f766e" alt="Made by Webhooker" /></a>
  <img src="https://img.shields.io/badge/written%20in-Rust-B7410E?logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows-555" alt="Linux, macOS, Windows" />
  <img src="https://img.shields.io/badge/license-MIT-green" alt="MIT" />
</p>

---

## What is whk?

`whk` is the official command-line client for [Webhooker](https://webhooker.eu/), an EU-hosted
inbound webhook gateway. It talks to your Webhooker workspace through the API, so you need a
Webhooker account and an API key to use it.

Most people start with it for local development. Stripe, GitHub or Shopify send webhooks to your
Webhooker ingest URL, and `whk listen` replays each one against `http://localhost:3000` (or any
local URL) with the original method, headers and body. The CLI connects out to Webhooker, so
nothing on your machine is exposed and you don't need ngrok or any other tunnel.

The rest of the commands cover what you would otherwise click through in the dashboard: sources and
their ingest URLs, destinations, connections with filters and transformations, the event log and
replays of failed deliveries. They work the same in a shell and in CI, and `--json` gives scripts
the raw API response.

`whk` is a single static binary. There is no runtime to install.

### Features

| Feature | Description |
|---|---|
| **Webhooks on localhost** | `whk listen <source> --forward http://localhost:3000/hooks` forwards every incoming webhook to your handler byte for byte, headers included. |
| **Live tail** | `whk tail <source>` prints one line per event as it arrives: method, size, content type and signature check result. |
| **Signature verification** | Sources verify Stripe, GitHub and Shopify signatures on arrival. By default, `listen` does not forward events that fail the check. |
| **Sources and ingest URLs** | Create, rename, pause, trash, restore and rotate tokens. `whk sources url` prints just the URL, ready for `curl`. |
| **Destinations and connections** | Deliver to your endpoints with custom headers, HMAC signing, timeouts and a retry policy. Filter and transform per connection. |
| **Event log and replay** | List and inspect stored events, then replay one event or a whole range of failed deliveries. |
| **Script friendly** | `--json` everywhere, JSON arguments from a file (`@file.json`) or stdin (`-`), credentials from environment variables. |
| **Safe credentials** | The API key is read without echo and saved to a file only your user can read (`0600` on Unix). |

## Install

### Download a binary

Grab the archive for your platform from the
[releases page](https://github.com/webhooker-eu/webhooker-cli/releases), unpack it and put `whk`
on your `PATH`:

```bash
tar -xzf whk-x86_64-unknown-linux-musl.tar.gz
sudo mv whk /usr/local/bin/
whk --version
```

| Platform | Archive |
|---|---|
| Linux x86_64 | `whk-x86_64-unknown-linux-musl` |
| Linux ARM64 | `whk-aarch64-unknown-linux-musl` |
| macOS Intel | `whk-x86_64-apple-darwin` |
| macOS Apple Silicon | `whk-aarch64-apple-darwin` |
| Windows x86_64 | `whk-x86_64-pc-windows-msvc` |

The Linux builds are statically linked with musl, so they run on any distribution, Alpine
included.

### Build with Cargo

With a recent stable [Rust toolchain](https://rustup.rs/):

```bash
cargo install --git https://github.com/webhooker-eu/webhooker-cli
```

## Quick start

**1. Get an API key.** Create one in the [Webhooker dashboard](https://app.webhooker.eu/). The
[API keys guide](https://docs.webhooker.eu/account/api-keys/) explains how keys work.

**2. Log in.**

```bash
whk login
```

```
API key (whk_...):
Logged in to https://app.webhooker.eu (workspace "Personal", free plan). Saved to ~/.config/webhooker/config.toml
```

**3. Create a source** and give its ingest URL to your provider. Pass `--verify stripe`,
`github` or `shopify` to check signatures on arrival. You will be asked for the signing
secret.

```bash
whk sources create stripe-test --verify stripe
whk sources url stripe-test          # https://app.webhooker.eu/in/<token>
```

**4. Forward webhooks to your local handler.**

```bash
whk listen stripe-test --forward http://localhost:3000/webhooks/stripe
```

```
  Forwarding "stripe-test" → http://localhost:3000/webhooks/stripe  (Ctrl-C to stop)
  2026-09-23T10:15:02Z POST ← evt_9rMz7kQpXa2FbtW (application/json, 2.4 KB)
           → 200 in 14ms
```

Trigger an event at the provider, or send one yourself:

```bash
curl -X POST "$(whk sources url stripe-test)" \
  -H 'Content-Type: application/json' -d '{"hello": "world"}'
```

The step-by-step version is the
[Test webhooks locally](https://docs.webhooker.eu/tutorials/local-development/) tutorial.

## Commands

```
whk login | logout | whoami
whk tail SOURCE
whk listen SOURCE --forward URL [--header 'Name: Value'] [--skip-verify]
whk sources       ls | create | get | update | rm | trash | restore | rotate-token | url
whk destinations  ls | create | get | update | rm          (alias: dests)
whk connections   ls | create | update | rm
whk connect SOURCE DESTINATION [--filter JSON] [--transform JSON]
whk events        ls | get | replay | replay-bulk
```

`whk <command> --help` lists every option. You can name a source by its name, id or ingest token.
You can name a destination by its name or id.

### Receive webhooks locally

```bash
# See what arrives, metadata only
whk tail github-prod

# Forward full requests to your app
whk listen github-prod --forward http://localhost:8000/github

# Add a header to the local request, and forward events that failed verification
whk listen github-prod --forward http://localhost:8000/github \
  --header 'X-Env: local' --skip-verify

# One JSON object per line, for jq
whk listen github-prod --forward http://localhost:8000/github --json | jq .
```

Each forwarded request carries an `X-Webhooker-Event-Id` header with the event's public id. You can
use it to find the event later in the log or in `whk events get`.

### Sources

```bash
whk sources ls -q stripe
whk sources create shopify-orders --verify shopify --color '#3b82f6'
whk sources update shopify-orders --status paused
whk sources rotate-token shopify-orders    # the old URL stops working within 30 seconds
whk sources rm shopify-orders              # moves it to the trash
whk sources trash
whk sources restore <source-id>
```

### Destinations and connections

A destination is an endpoint Webhooker delivers to, and a connection routes a source to it.

```bash
whk destinations create billing-api --url https://api.example.com/webhooks/stripe \
  --header 'Authorization: Bearer ...' --auth hmac --timeout-ms 10000

whk connect stripe-prod billing-api \
  --filter '{"operator": "and", "rules": [{"path": "body.type", "op": "eq", "value": "invoice.paid"}]}'

whk connections ls --source stripe-prod
whk connections update <connection-id> --disable
```

`--filter`, `--transform`, `--retry` and a custom `--verify` or `--auth` config take JSON inline,
from a file with `@rules.json`, or from stdin with `-`. See
[filters and transformations](https://docs.webhooker.eu/deliver/filters-and-transformations/) and
[retries and replay](https://docs.webhooker.eu/deliver/retries-and-replay/) for the formats.

### Events and replay

```bash
whk events ls --source stripe-prod --status failed --since 2026-09-20T00:00:00Z
whk events get evt_9rMz7kQpXa2FbtW                 # headers, body and every delivery attempt

whk events replay evt_9rMz7kQpXa2FbtW              # re-queue to every connection
whk events replay evt_9rMz7kQpXa2FbtW --connection <connection-id>

# After an outage: replay everything that ran out of retries
whk events replay-bulk --connection <connection-id> --since 2026-09-22T08:00:00Z
```

## Configuration

`whk login` saves the server and the API key to a TOML file in your user config directory:

| OS | Path |
|---|---|
| Linux | `~/.config/webhooker/config.toml` |
| macOS | `~/Library/Application Support/webhooker/config.toml` |
| Windows | `%APPDATA%\webhooker\config.toml` |

Flags and environment variables override the saved file:

| Flag | Variable | Description |
|---|---|---|
| `--api-key` | `WEBHOOKER_API_KEY` | API key, `whk_...`. |
| `--server` | `WEBHOOKER_SERVER` | API base URL. Defaults to `https://app.webhooker.eu`. |
| `--json` | | Print the API's JSON instead of formatted output. |

In CI, set `WEBHOOKER_API_KEY` from your secret store and skip `whk login`:

```bash
export WEBHOOKER_API_KEY="$WEBHOOKER_API_KEY_FROM_SECRETS"
whk events replay-bulk --connection "$CONNECTION_ID" --since "$OUTAGE_START"
```

## FAQ

**How do I receive Stripe, GitHub or Shopify webhooks on localhost without ngrok?**
Create a Webhooker source, set its ingest URL as the webhook endpoint at the provider, and run
`whk listen <source> --forward http://localhost:3000/your/path`. The provider talks to Webhooker's
public URL. `whk` opens an outbound connection from your machine and replays each request locally,
so no port is opened and no tunnel is needed. The blog post
[ngrok alternatives for webhooks](https://webhooker.eu/blog/ngrok-alternatives-for-webhooks)
compares the options.

**Will my handler's signature check pass on a forwarded webhook?**
Yes. The body is forwarded byte for byte, and the provider's signature headers
(`Stripe-Signature`, `X-Hub-Signature-256`, `X-Shopify-Hmac-Sha256`) are passed through unchanged.
Only transport headers such as `Host` and `Content-Length` are recomputed. Stripe signs a
timestamp, so a request replayed hours later is too old for Stripe's libraries. Forward live
events while you develop.

**What is the difference between `tail` and `listen`?**
`tail` prints metadata only: time, method, size, content type and verification result. It is a
cheap way to confirm that a provider really sends. `listen` receives the full request and forwards
it to your local URL.

**Why are some events not forwarded?**
If the source verifies signatures, events that fail the check are skipped, and `listen` prints
`signature invalid; use --skip-verify`. Usually the secret on the source is wrong. Fix it with
`whk sources update <source> --verify stripe`, or pass `--skip-verify` while you debug.

**Do I miss events while `whk listen` is not running?**
No. Webhooker stores every event whether or not you are listening. Find the one you need with
`whk events ls` and replay it to a destination with `whk events replay`, or resend it from the
[event log](https://docs.webhooker.eu/receive/events/).

**How do I send correctly signed test webhooks without the provider?**
Use [webhook-mock-sender](https://github.com/webhooker-eu/webhook-mock-sender). It signs Stripe,
GitHub and Shopify events with your secret and sends them to any URL, your Webhooker ingest URL
included.

## Local development

You need a recent stable [Rust toolchain](https://rustup.rs/).

```bash
cargo build                                   # debug build in target/debug/whk
cargo run -- --help
cargo test                                    # unit tests with a mock HTTP server, no network
cargo build --release                         # optimized binary in target/release/whk
```

To test against a local server, use `--server http://localhost:8080` or set `WEBHOOKER_SERVER`.

Releases are built by [GitHub Actions](.github/workflows/release.yml) when a `v*` tag is pushed:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

## Repository layout

```
.
├── src/
│   ├── main.rs            # Command-line interface (clap) and command dispatch
│   ├── client.rs          # HTTP client for the Webhooker REST API
│   ├── config.rs          # Saved credentials: location, loading, 0600 writes
│   ├── listen.rs          # `whk listen`: stream webhooks and forward them locally
│   ├── tail.rs            # `whk tail`: stream event metadata
│   ├── sse.rs             # Server-Sent Events reader with reconnects
│   ├── args.rs            # JSON, @file and stdin arguments; header flags
│   ├── output.rs          # Human-readable tables and JSON output
│   └── commands/          # sources, destinations, connections, events
├── .github/workflows/     # Cross-platform release binaries
├── Cargo.toml
└── Cargo.lock
```

## Security notes

- An API key gives full access to its workspace. Keep it in a secret store or in the saved config,
  not in scripts or shell history. `whk login` prompts for it without echo on purpose.
- The config file is created with mode `0600` on Unix. `whk logout` deletes it.
- `whk listen` only makes outbound connections: one to Webhooker and one to the URL you pass to
  `--forward`. It does not listen on any port.
- Revoke a key you no longer need in the dashboard. The
  [API keys guide](https://docs.webhooker.eu/account/api-keys/) shows each key's last use.

## About Webhooker

[Webhooker](https://webhooker.eu/) is an EU-hosted inbound webhook gateway: one ingest URL for any
provider, with signature verification, durable storage, retries and replay, so you never lose an
event. `whk` is its official command-line client. We also publish free, open-source utilities for
people who work with webhooks.

- Website: [webhooker.eu](https://webhooker.eu/)
- Documentation: [docs.webhooker.eu](https://docs.webhooker.eu/)
- Free webhook tools: [webhooker.eu/tools](https://webhooker.eu/tools)
- More open-source tools: [github.com/webhooker-eu](https://github.com/webhooker-eu)
- Need to see exactly what a provider sends? Try
  [webhook-tester](https://github.com/webhooker-eu/webhook-tester), a self-hosted request bin.
- Need signed test events for your endpoint? Try
  [webhook-mock-sender](https://github.com/webhooker-eu/webhook-mock-sender).
- Testing a Discord webhook? Try
  [discord-webhook-tester](https://github.com/webhooker-eu/discord-webhook-tester).

## License

[MIT](LICENSE) © [Webhooker](https://webhooker.eu/)

This project is not affiliated with Stripe, GitHub or Shopify. Their names are used only to
describe webhook formats.
