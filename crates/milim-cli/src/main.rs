//! `milim` - the milim command-line interface.
//!
//! The serve command boots the HTTP server with a backend chosen from the
//! environment (OpenAI-compatible remote, or an explicit no-model fallback).

use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};

use milim_core::api::openai::ChatMessage;
use milim_core::config::ServerConfiguration;
use milim_core::paths::Paths;
use milim_inference::{remote::RemoteBackend, unavailable::UnavailableBackend, SharedService};
use milim_server::AppState;
use serde_json::{json, Value};

#[derive(Parser)]
#[command(
    name = "milim",
    version,
    about = "Own your AI - local inference server"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the inference server.
    Serve {
        /// Port to bind (overrides config; default 7377).
        #[arg(long, short)]
        port: Option<u16>,
        /// Bind on the LAN (0.0.0.0) instead of loopback only.
        #[arg(long)]
        expose: bool,
    },
    /// Check whether a server is running and print its health.
    Status {
        #[command(flatten)]
        client: ClientArgs,
        /// Print raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Chat with a model: one-shot if a prompt is given, else an interactive REPL.
    Run {
        #[command(flatten)]
        client: ClientArgs,
        /// System prompt to prepend.
        #[arg(long)]
        system: Option<String>,
        /// Sampling temperature.
        #[arg(long)]
        temperature: Option<f32>,
        /// Maximum output tokens.
        #[arg(long)]
        max_tokens: Option<u32>,
        /// Model id from `/v1/models` or a configured provider.
        model: String,
        /// Optional prompt; omit for an interactive session.
        prompt: Vec<String>,
    },
    /// List models from a running server.
    Models {
        #[command(flatten)]
        client: ClientArgs,
        /// Print raw JSON.
        #[arg(long)]
        json: bool,
    },
    /// Manage identity and msk-v1 access keys.
    Keys {
        #[command(subcommand)]
        action: KeysAction,
    },
    /// Re-send one recorded model step of a canonical run and compare responses.
    Replay {
        #[command(flatten)]
        client: ClientArgs,
        /// Model step to replay (defaults to the run's last model step).
        #[arg(long)]
        step: Option<usize>,
        /// Provider or local model to send to instead of the recorded one.
        #[arg(long)]
        model: Option<String>,
        /// Print the reconstructed request without calling a model.
        #[arg(long)]
        dry_run: bool,
        /// Print raw JSON.
        #[arg(long)]
        json: bool,
        /// Run id from `GET /control/v1/bootstrap` or the run details view.
        run_id: String,
    },
    /// Run a stdio MCP server (for Claude Desktop etc.) proxying to the local server.
    Mcp {
        #[command(flatten)]
        client: ClientArgs,
    },
    /// Print the version.
    Version,
}

#[derive(Args, Debug, Clone, Default)]
struct ClientArgs {
    /// Server base URL. Overrides --port and the configured port.
    #[arg(long)]
    url: Option<String>,
    /// Loopback port to use when --url is omitted.
    #[arg(long, short)]
    port: Option<u16>,
    /// Bearer token for authenticated servers.
    #[arg(long, env = "MILIM_API_TOKEN")]
    token: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RunOptions {
    system: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
}

#[derive(Subcommand)]
enum KeysAction {
    /// Print this machine's identity address.
    Identity,
    /// Mint an msk-v1 access key.
    Mint {
        /// Audience address (defaults to this machine's own address).
        #[arg(long)]
        audience: Option<String>,
        /// Optional human label.
        #[arg(long)]
        label: Option<String>,
        /// Expiry, in seconds from now (omit for no expiry).
        #[arg(long)]
        expires_secs: Option<i64>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Serve { port, expose } => serve(port, expose).await,
        Command::Status { client, json } => status(client, json).await,
        Command::Run {
            client,
            system,
            temperature,
            max_tokens,
            model,
            prompt,
        } => {
            run_chat(
                client,
                RunOptions {
                    system,
                    temperature,
                    max_tokens,
                },
                model,
                prompt,
            )
            .await
        }
        Command::Models { client, json } => models(client, json).await,
        Command::Replay {
            client,
            step,
            model,
            dry_run,
            json,
            run_id,
        } => replay(client, run_id, replay_body(step, model, dry_run), json).await,
        Command::Keys { action } => keys_cmd(action),
        Command::Mcp { client } => {
            let base = client_base_url(&client)?;
            milim_server::mcp_bridge::run_mcp_stdio(base, client.token)
                .await
                .context("mcp bridge")?;
            Ok(())
        }
        Command::Version => {
            println!("milim {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

async fn serve(port: Option<u16>, expose: bool) -> anyhow::Result<()> {
    let paths = Paths::resolve();
    paths.ensure().context("create milim directories")?;

    let mut config = ServerConfiguration::load_or_default(&paths.server_config_file());
    if let Some(p) = port {
        config.port = p;
    }
    if expose {
        config.expose_to_network = true;
        if let Some(token) =
            ensure_expose_auth(&mut config, paths.root(), &paths.server_config_file())?
        {
            println!("auth: enabled msk-v1 for --expose");
            println!("token: {token}");
            println!("PowerShell: $env:MILIM_API_TOKEN='{token}'");
        }
    }

    let backend = build_backend().await?;
    let addr: SocketAddr = format!("{}:{}", config.bind_host(), config.port)
        .parse()
        .context("invalid bind address")?;

    tracing::info!(backend = backend.name(), %addr, "starting milim server");
    println!("milim [{}] serving on http://{addr}", backend.name());
    if expose {
        println!("warning: exposed on the local network (0.0.0.0)");
    }

    // Built-in tools + filesystem tools sandboxed to a workspace directory.
    let workspace = paths.root().join("workspace");
    std::fs::create_dir_all(&workspace).context("create CLI workspace")?;
    let mut tools = milim_tools::ToolRegistry::with_builtins();
    tools.register_fs(&workspace);

    let auth_config = config.clone();
    let privacy = Arc::new(milim_server::privacy::PrivacyGate::from_env());
    let mut state = AppState::new(backend.clone(), config)
        .with_tools(tools)
        .with_privacy(privacy)
        .with_mobile_companion(Arc::new(
            milim_server::companion::MobileCompanionBridge::default(),
        ));
    match milim_server::media_library::MediaLibrary::open(paths.root().join("media")) {
        Ok(library) => state = state.with_media_library(library),
        Err(e) => tracing::warn!("media library disabled: {e}"),
    }
    state = configure_auth(state, paths.root(), &auth_config)?;

    // Enable memory/RAG if its database can be opened.
    let memory_db = paths.root().join("memory.db");
    match milim_storage::Database::open(&memory_db)
        .and_then(|db| milim_memory::MemoryStore::new(db, backend.clone()))
    {
        Ok(mem) => state = state.with_memory(mem),
        Err(e) => tracing::warn!("memory disabled: {e}"),
    }

    // Enable the named-agent store.
    let agents_db = paths.root().join("agents.db");
    match milim_storage::Database::open(&agents_db).and_then(milim_agents::AgentStore::new) {
        Ok(store) => state = state.with_agents(store),
        Err(e) => tracing::warn!("agents disabled: {e}"),
    }

    // Enable child-thread orchestration.
    let threads_db = paths.root().join("threads.db");
    match milim_storage::Database::open(&threads_db).and_then(milim_agents::ThreadStore::new) {
        Ok(store) => {
            state = state.with_threads(milim_server::threads::ThreadSupervisor::new(store))
        }
        Err(e) => tracing::warn!("child threads disabled: {e}"),
    }

    // Enable the cron schedule store.
    let schedules_db = paths.root().join("schedules.db");
    match milim_storage::Database::open(&schedules_db)
        .and_then(milim_automation::ScheduleStore::new)
    {
        Ok(store) => state = state.with_schedules(store),
        Err(e) => tracing::warn!("schedules disabled: {e}"),
    }

    // Enable the skills store.
    let skills_db = paths.root().join("skills.db");
    match milim_storage::Database::open(&skills_db).and_then(milim_skills::SkillStore::new) {
        Ok(store) => {
            if let Err(e) = store.import_global_skills() {
                tracing::warn!("global skills import failed: {e}");
            }
            state = state.with_skills(store)
        }
        Err(e) => tracing::warn!("skills disabled: {e}"),
    }

    // Start the background scheduler if schedules are enabled.
    if state.schedules.is_some() {
        milim_server::spawn_scheduler(state.clone());
    }

    milim_server::serve(state, addr)
        .await
        .context("server error")?;
    Ok(())
}

fn configure_auth(
    mut state: AppState,
    root: &Path,
    config: &ServerConfiguration,
) -> anyhow::Result<AppState> {
    let api_keys: Vec<String> = config
        .api_keys
        .iter()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let has_access_key_issuers = config
        .access_key_issuers
        .iter()
        .any(|issuer| !issuer.trim().is_empty());
    let access_keys_enabled = config.auth_required || has_access_key_issuers;
    let auth_enabled = access_keys_enabled || !api_keys.is_empty();

    if !api_keys.is_empty() {
        state = state.with_api_keys(api_keys);
    }
    if access_keys_enabled {
        let identity = milim_identity::LocalIdentity::load_or_create(&identity_key_path(root))
            .context("load local identity for msk-v1 auth")?;
        let address = identity.address()?;
        let mut validator = milim_identity::AccessKeyValidator::new(&address, &address);
        validator.allow_issuer(&address);
        for issuer in &config.access_key_issuers {
            let issuer = issuer.trim();
            if !issuer.is_empty() {
                validator.allow_issuer(issuer);
            }
        }
        state = state.with_access_validator(validator);
    }
    if auth_enabled {
        state = state.with_loopback_trust(false);
    }

    Ok(state)
}

fn ensure_expose_auth(
    config: &mut ServerConfiguration,
    root: &Path,
    config_file: &Path,
) -> anyhow::Result<Option<String>> {
    if auth_configured(config) {
        return Ok(None);
    }
    config.auth_required = true;
    let token = mint_local_msk_token(root, None, Some("serve --expose".to_string()), None)?;
    config
        .save(config_file)
        .with_context(|| format!("save {}", config_file.display()))?;
    Ok(Some(token))
}

fn auth_configured(config: &ServerConfiguration) -> bool {
    config.auth_required
        || config.api_keys.iter().any(|key| !key.trim().is_empty())
        || config
            .access_key_issuers
            .iter()
            .any(|issuer| !issuer.trim().is_empty())
}

fn identity_key_path(root: &Path) -> std::path::PathBuf {
    root.join("identity").join("master.key")
}

fn client_base_url(args: &ClientArgs) -> anyhow::Result<String> {
    let paths = Paths::resolve();
    let config = ServerConfiguration::load_or_default(&paths.server_config_file());
    client_base_url_from_config(args, &config)
}

fn client_base_url_from_config(
    args: &ClientArgs,
    config: &ServerConfiguration,
) -> anyhow::Result<String> {
    if args.url.is_some() && args.port.is_some() {
        anyhow::bail!("--url and --port cannot be used together");
    }
    if let Some(url) = &args.url {
        let trimmed = url.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            anyhow::bail!("--url cannot be empty");
        }
        return Ok(trimmed.to_string());
    }
    let port = args.port.unwrap_or(config.port);
    Ok(format!("http://127.0.0.1:{port}"))
}

async fn status(client_args: ClientArgs, raw_json: bool) -> anyhow::Result<()> {
    let base = client_base_url(&client_args)?;
    let client = reqwest::Client::new();
    let v = get_json_value(
        auth(
            client.get(format!("{base}/health")),
            client_args.token.as_deref(),
        ),
        &base,
    )
    .await?;
    if raw_json {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("running at {base} - {v}");
    }
    Ok(())
}

async fn models(client_args: ClientArgs, raw_json: bool) -> anyhow::Result<()> {
    let base = client_base_url(&client_args)?;
    let client = reqwest::Client::new();
    let v = get_json_value(
        auth(
            client.get(format!("{base}/v1/models")),
            client_args.token.as_deref(),
        ),
        &base,
    )
    .await?;
    if raw_json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let models = v["data"].as_array().cloned().unwrap_or_default();
    if models.is_empty() {
        println!("no models configured");
    } else {
        for model in models {
            if let Some(id) = model["id"].as_str() {
                println!("{id}");
            }
        }
    }
    Ok(())
}

async fn run_chat(
    client_args: ClientArgs,
    options: RunOptions,
    model: String,
    prompt: Vec<String>,
) -> anyhow::Result<()> {
    use std::io::Write;
    use tokio::io::AsyncBufReadExt;

    let base = client_base_url(&client_args)?;
    let client = reqwest::Client::new();
    let token = client_args.token.as_deref();
    let mut messages: Vec<ChatMessage> = Vec::new();
    if let Some(system) = options.system.as_deref().filter(|s| !s.trim().is_empty()) {
        messages.push(ChatMessage::text("system", system.trim()));
    }

    if prompt.is_empty() {
        // Interactive REPL.
        println!("milim chat [{base}] - type 'exit' or 'quit' to quit");
        let stdin = tokio::io::BufReader::new(tokio::io::stdin());
        let mut lines = stdin.lines();
        loop {
            print!("> ");
            std::io::stdout().flush().ok();
            let Some(line) = lines.next_line().await? else {
                break;
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == "exit" || line == "quit" {
                break;
            }
            messages.push(ChatMessage::text("user", line));
            let reply =
                post_chat_stream(&client, &base, token, &model, &messages, &options).await?;
            println!();
            messages.push(ChatMessage::text("assistant", reply));
        }
    } else {
        messages.push(ChatMessage::text("user", prompt.join(" ")));
        post_chat_stream(&client, &base, token, &model, &messages, &options).await?;
        println!();
    }
    Ok(())
}

async fn post_chat_stream(
    client: &reqwest::Client,
    base: &str,
    token: Option<&str>,
    model: &str,
    messages: &[ChatMessage],
    options: &RunOptions,
) -> anyhow::Result<String> {
    use futures::StreamExt;

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true
    });
    if let Some(temperature) = options.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(max_tokens) = options.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }

    let response = checked_response(
        auth(client.post(format!("{base}/v1/chat/completions")), token).json(&body),
        base,
    )
    .await?;
    let mut stream = response.bytes_stream();
    let mut pending = String::new();
    let mut full = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read chat stream")?;
        pending.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
        while let Some(idx) = pending.find("\n\n") {
            let raw = pending[..idx].to_string();
            pending.replace_range(..idx + 2, "");
            if handle_sse_event(&raw, &mut full)? {
                return Ok(full);
            }
        }
    }
    if !pending.trim().is_empty() {
        let _ = handle_sse_event(&pending, &mut full)?;
    }
    Ok(full)
}

fn handle_sse_event(raw: &str, full: &mut String) -> anyhow::Result<bool> {
    for line in raw.lines() {
        let Some(data) = line.trim_end_matches('\r').strip_prefix("data:") else {
            continue;
        };
        let data = data.trim_start();
        if data == "[DONE]" {
            return Ok(true);
        }
        if data.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(data).context("parse chat stream event")?;
        if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
            anyhow::bail!("{message}");
        }
        if let Some(content) = value["choices"][0]["delta"]["content"].as_str() {
            print!("{content}");
            std::io::stdout().flush().ok();
            full.push_str(content);
        }
    }
    Ok(false)
}

fn replay_body(step: Option<usize>, model: Option<String>, dry_run: bool) -> Value {
    let mut body = json!({ "dry_run": dry_run });
    if let Some(step) = step {
        body["step"] = json!(step);
    }
    if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
        body["model"] = json!(model.trim());
    }
    body
}

async fn replay(
    client_args: ClientArgs,
    run_id: String,
    body: Value,
    raw_json: bool,
) -> anyhow::Result<()> {
    let base = client_base_url(&client_args)?;
    let client = reqwest::Client::new();
    let url = format!(
        "{base}/control/v1/runs/{}/replay",
        encode_path_segment(run_id.trim())
    );
    let v = get_json_value(
        auth(client.post(url).json(&body), client_args.token.as_deref()),
        &base,
    )
    .await?;
    if raw_json || v["dry_run"] == true {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    println!(
        "run {} step {} (privacy {})",
        run_id.trim(),
        v["step"],
        v["privacy"].as_str().unwrap_or("off")
    );
    for (label, side) in [("original", &v["original"]), ("replay", &v["replay"])] {
        if side.is_null() {
            println!("\n[{label}] no recorded response");
            continue;
        }
        let tools = side["tool_calls"]
            .as_array()
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|call| call["function"]["name"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        println!(
            "\n[{label}] {} - finish {}, {} ms, tokens {}, tools [{tools}]",
            side["model"].as_str().unwrap_or("?"),
            side["finish_reason"].as_str().unwrap_or("?"),
            side["latency_ms"],
            side["usage"]["total_tokens"],
        );
        println!("{}", side["content"].as_str().unwrap_or_default());
    }
    let diff = &v["diff"];
    if !diff.is_null() {
        println!(
            "\ndiff: same tool names {}, same arguments {}, same finish {}, text similarity {:.2}",
            diff["same_tool_names"],
            diff["same_tool_arguments"],
            diff["same_finish_reason"],
            diff["text_similarity"].as_f64().unwrap_or(0.0),
        );
    }
    Ok(())
}

fn encode_path_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

async fn get_json_value(req: reqwest::RequestBuilder, base: &str) -> anyhow::Result<Value> {
    let response = checked_response(req, base).await?;
    response.json().await.context("read JSON response")
}

async fn checked_response(
    req: reqwest::RequestBuilder,
    base: &str,
) -> anyhow::Result<reqwest::Response> {
    let response = req
        .send()
        .await
        .with_context(|| format!("could not reach {base}; start `milim serve` or pass --url"))?;
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    anyhow::bail!("server at {base} returned {status}: {body}");
}

fn auth(req: reqwest::RequestBuilder, token: Option<&str>) -> reqwest::RequestBuilder {
    match token.filter(|t| !t.trim().is_empty()) {
        Some(token) => req.bearer_auth(token.trim()),
        None => req,
    }
}

fn keys_cmd(action: KeysAction) -> anyhow::Result<()> {
    let paths = Paths::resolve();
    paths.ensure().context("create milim directories")?;

    match action {
        KeysAction::Identity => {
            let key_path = identity_key_path(paths.root());
            let identity = milim_identity::LocalIdentity::load_or_create(&key_path)
                .context("load identity")?;
            println!("{}", identity.address()?);
        }
        KeysAction::Mint {
            audience,
            label,
            expires_secs,
        } => {
            if let Some(secs) = expires_secs {
                if secs <= 0 {
                    anyhow::bail!("--expires-secs must be positive");
                }
            }
            println!(
                "{}",
                mint_local_msk_token(paths.root(), audience, label, expires_secs)?
            );
        }
    }
    Ok(())
}

fn mint_local_msk_token(
    root: &Path,
    audience: Option<String>,
    label: Option<String>,
    expires_secs: Option<i64>,
) -> anyhow::Result<String> {
    let key_path = identity_key_path(root);
    let identity =
        milim_identity::LocalIdentity::load_or_create(&key_path).context("load identity")?;
    let address = identity.address()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs() as i64;
    let params = milim_identity::MintParams {
        audience: audience.unwrap_or(address),
        label,
        iat: now,
        exp: expires_secs.map(|s| now + s),
        cnt: now as u64,
        nonce: milim_identity::random_nonce(),
    };
    Ok(identity.mint_token(params)?)
}

/// Choose a backend from the environment, in priority order:
///   1. OpenAI-compatible remote when `MILIM_REMOTE_BASE_URL` is set.
///   2. Explicit unavailable backend (default; no synthetic model is advertised).
async fn build_backend() -> anyhow::Result<SharedService> {
    if let Some(base) = non_empty_env("MILIM_REMOTE_BASE_URL") {
        let key = non_empty_env("MILIM_REMOTE_API_KEY");
        return Ok(Arc::new(RemoteBackend::new("remote", base, key)));
    }

    Ok(Arc::new(UnavailableBackend::new()))
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("MILIM_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn client_base_url_uses_config_port_and_normalizes_url() {
        let config = ServerConfiguration {
            port: 9001,
            ..Default::default()
        };

        let args = ClientArgs::default();
        assert_eq!(
            client_base_url_from_config(&args, &config).unwrap(),
            "http://127.0.0.1:9001"
        );

        let args = ClientArgs {
            port: Some(9002),
            ..Default::default()
        };
        assert_eq!(
            client_base_url_from_config(&args, &config).unwrap(),
            "http://127.0.0.1:9002"
        );

        let args = ClientArgs {
            url: Some("http://127.0.0.1:7777/".to_string()),
            ..Default::default()
        };
        assert_eq!(
            client_base_url_from_config(&args, &config).unwrap(),
            "http://127.0.0.1:7777"
        );
    }

    #[test]
    fn client_base_url_rejects_url_and_port_together() {
        let args = ClientArgs {
            url: Some("http://127.0.0.1:7777".to_string()),
            port: Some(9002),
            token: None,
        };
        assert!(client_base_url_from_config(&args, &ServerConfiguration::default()).is_err());
    }

    #[test]
    fn clap_parses_status_client_flags() {
        let cli = Cli::try_parse_from([
            "milim",
            "status",
            "--url",
            "http://127.0.0.1:8888",
            "--token",
            "secret",
            "--json",
        ])
        .unwrap();
        match cli.command {
            Command::Status { client, json } => {
                assert_eq!(client.url.as_deref(), Some("http://127.0.0.1:8888"));
                assert_eq!(client.token.as_deref(), Some("secret"));
                assert!(json);
            }
            _ => panic!("expected status command"),
        }
    }

    #[test]
    fn clap_parses_replay_flags_into_the_request_body() {
        let cli = Cli::try_parse_from([
            "milim",
            "replay",
            "--step",
            "3",
            "--model",
            "gpt-x",
            "--dry-run",
            "run/1",
        ])
        .unwrap();
        match cli.command {
            Command::Replay {
                step,
                model,
                dry_run,
                run_id,
                ..
            } => {
                assert_eq!(
                    replay_body(step, model, dry_run),
                    json!({ "step": 3, "model": "gpt-x", "dry_run": true })
                );
                assert_eq!(encode_path_segment(&run_id), "run%2F1");
            }
            _ => panic!("expected replay command"),
        }
        assert_eq!(
            replay_body(None, Some(" ".into()), false),
            json!({ "dry_run": false })
        );
    }

    #[test]
    fn auth_required_accepts_local_msk_tokens() {
        let root = temp_root("milim-cli-auth");
        let config = ServerConfiguration {
            auth_required: true,
            ..Default::default()
        };

        let state = AppState::new(Arc::new(UnavailableBackend::new()), config.clone());
        let state = configure_auth(state, &root, &config).unwrap();
        assert!(!state.trust_loopback);

        let identity =
            milim_identity::LocalIdentity::load_or_create(&identity_key_path(&root)).unwrap();
        let address = identity.address().unwrap();
        let token = identity
            .mint_token(milim_identity::MintParams {
                audience: address,
                label: Some("test".to_string()),
                iat: 1_700_000_000,
                exp: None,
                cnt: 1,
                nonce: "nonce".to_string(),
            })
            .unwrap();
        assert!(state
            .access_validator
            .as_ref()
            .unwrap()
            .validate(&token)
            .is_valid());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn expose_auto_auth_enables_and_mints_local_token() {
        let root = temp_root("milim-cli-expose-auth");
        let config_file = root.join("config").join("server.json");
        let mut config = ServerConfiguration::default();

        let token = ensure_expose_auth(&mut config, &root, &config_file)
            .unwrap()
            .expect("token");

        assert!(config.auth_required);
        let saved = ServerConfiguration::load_or_default(&config_file);
        assert!(saved.auth_required);
        let state = AppState::new(Arc::new(UnavailableBackend::new()), config.clone());
        let state = configure_auth(state, &root, &config).unwrap();
        assert!(!state.trust_loopback);
        assert!(state
            .access_validator
            .as_ref()
            .unwrap()
            .validate(&token)
            .is_valid());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn expose_auto_auth_leaves_existing_auth_alone() {
        let root = temp_root("milim-cli-expose-existing-auth");
        let config_file = root.join("config").join("server.json");
        let mut config = ServerConfiguration {
            api_keys: vec!["static-secret".to_string()],
            ..Default::default()
        };

        assert!(ensure_expose_auth(&mut config, &root, &config_file)
            .unwrap()
            .is_none());
        assert!(!config.auth_required);
        assert!(!config_file.exists());

        let _ = std::fs::remove_dir_all(root);
    }
}
