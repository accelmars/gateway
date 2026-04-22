use std::sync::Arc;

use clap::{Parser, Subcommand};

use accelmars_gateway::adapters::{
    new_deepseek_adapter, new_groq_adapter, new_openrouter_adapter, ClaudeAdapter, GeminiAdapter,
};
use accelmars_gateway::auth::AuthStore;
use accelmars_gateway::cli;
use accelmars_gateway::cli::complete::CompleteConstraints;
use accelmars_gateway::cli::status::PortSource;
use accelmars_gateway::concurrency::ConcurrencyLimiter;
use accelmars_gateway::config::GatewayConfig;
use accelmars_gateway::cost::CostTracker;
use accelmars_gateway::pid;
use accelmars_gateway::registry::AdapterRegistry;
use accelmars_gateway::router::Router;
use accelmars_gateway::server;
use accelmars_gateway_core::{MockAdapter, ModelTier};

#[derive(Parser)]
#[command(
    name = "gateway",
    version,
    about = "Universal AI gateway — multi-provider, OpenAI-compatible, Rust-native",
    long_about = None,
)]
struct Cli {
    /// Log level (error, warn, info, debug, trace)
    #[arg(long, global = true, default_value = "info", env = "GATEWAY_LOG_LEVEL")]
    log_level: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the gateway HTTP server (OpenAI-compatible API)
    Serve {
        /// Port to listen on (overrides config file and GATEWAY__PORT)
        #[arg(long, env = "GATEWAY_PORT")]
        port: Option<u16>,
        /// Path to config file (default: gateway.toml in CWD)
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Show gateway health and provider availability
    Status {
        /// Port the server is listening on (overrides PID file and config)
        #[arg(long)]
        port: Option<u16>,
        /// Path to config file (to read port default)
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Output JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Show cost summary and call statistics
    Stats {
        /// Filter to calls on or after this date (YYYY-MM-DD)
        #[arg(long)]
        since: Option<String>,
        /// Output JSON instead of human-readable text
        #[arg(long)]
        json: bool,
        /// Filter by provider name
        #[arg(long)]
        provider: Option<String>,
        /// Filter by tier (quick, standard, max, ultra)
        #[arg(long)]
        tier: Option<String>,
    },
    /// Execute a single completion (one-shot mode, no server needed)
    Complete {
        /// Prompt to complete
        prompt: String,
        /// Model tier to use (quick, standard, max, ultra)
        #[arg(short, long, default_value = "standard")]
        tier: String,
        /// Output JSON with full metadata
        #[arg(long)]
        json: bool,
        /// Path to config file (default: gateway.toml in CWD)
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Privacy constraint: open | sensitive | private
        #[arg(long)]
        privacy: Option<String>,
        /// Cost constraint: free | budget | default | unlimited
        #[arg(long)]
        cost: Option<String>,
        /// Latency constraint: normal | low
        #[arg(long)]
        latency: Option<String>,
        /// Explicit provider override (bypasses tier routing)
        #[arg(long)]
        provider: Option<String>,
    },
    /// Stop a running gateway server
    Stop {
        /// Port the server is listening on (default: read from PID file)
        #[arg(long)]
        port: Option<u16>,
    },
    /// Manage API keys
    Keys {
        #[command(subcommand)]
        action: KeysAction,
    },
}

#[derive(Subcommand)]
enum KeysAction {
    /// Create a new API key
    Create {
        /// Human-readable name for this key
        #[arg(long)]
        name: String,
    },
    /// List all API keys
    List {
        /// Output JSON instead of table
        #[arg(long)]
        json: bool,
    },
    /// Revoke an API key by prefix (e.g., gw_live_a8f2)
    Revoke {
        /// Key prefix shown in `gateway keys list`
        prefix: String,
    },
}

fn build_registry_from_config(config: &GatewayConfig) -> AdapterRegistry {
    use accelmars_gateway::config::GatewayMode;

    let mut registry = AdapterRegistry::new();

    // Mock adapter — always registered (used by mock mode + fallback)
    registry.register(Arc::new(MockAdapter::default()));

    if config.mode == GatewayMode::Mock {
        tracing::info!("GATEWAY_MODE=mock — mock adapter only");
        return registry;
    }

    for (name, provider_cfg) in &config.providers {
        let api_key = std::env::var(&provider_cfg.api_key_env).ok();
        let adapter: Arc<dyn accelmars_gateway_core::ProviderAdapter> = match name.as_str() {
            n if n.starts_with("gemini") => {
                Arc::new(GeminiAdapter::new(api_key, provider_cfg.model.clone()))
            }
            "deepseek" => Arc::new(new_deepseek_adapter(api_key)),
            n if n.starts_with("claude") => {
                Arc::new(ClaudeAdapter::new(api_key, provider_cfg.model.clone()))
            }
            n if n.starts_with("openrouter") => Arc::new(new_openrouter_adapter(api_key)),
            n if n.starts_with("groq") => Arc::new(new_groq_adapter(api_key)),
            _ => {
                tracing::warn!(provider = name, "unknown provider type — skipping");
                continue;
            }
        };
        registry.register(adapter);
    }

    let available = registry.available();
    let all = registry.all_providers();
    tracing::info!(
        available = ?available,
        registered = ?all,
        "adapter registry built"
    );

    registry
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize structured tracing
    let filter = tracing_subscriber::EnvFilter::try_new(&cli.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match cli.command {
        Some(Commands::Serve { port, config }) => {
            let config_path = config.as_deref();
            let mut gateway_config = match GatewayConfig::load(config_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("failed to load config: {e:#}");
                    std::process::exit(1);
                }
            };

            if let Some(p) = port {
                gateway_config.port = p;
            }

            match gateway_config.validate() {
                Ok(warnings) => {
                    for w in &warnings {
                        tracing::warn!("{w}");
                    }
                }
                Err(e) => {
                    tracing::error!("config validation failed: {e:#}");
                    std::process::exit(1);
                }
            }

            let serve_port = gateway_config.port;

            // Auth setup — read env var once at startup, store in AppState.
            let auth_disabled = std::env::var("GATEWAY_AUTH_DISABLED").is_ok();
            if auth_disabled {
                tracing::warn!(
                    "Authentication disabled (GATEWAY_AUTH_DISABLED is set). Do NOT use in production."
                );
            }

            let auth_db_path = AuthStore::default_path();
            let auth_store = match AuthStore::open(&auth_db_path) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    tracing::warn!("auth store unavailable (fail-open): {e:#}");
                    match AuthStore::in_memory() {
                        Ok(s) => Arc::new(s),
                        Err(e2) => {
                            tracing::error!("auth store fallback also failed: {e2:#}");
                            std::process::exit(1);
                        }
                    }
                }
            };

            // --- PID file: check for already-running instance ---
            if let Some(existing) = pid::read() {
                if pid::is_alive(existing.pid) {
                    eprintln!(
                        "error: Gateway already running on port {} (pid {}). \
                        Use `gateway status --port {}` to check.",
                        existing.port, existing.pid, existing.port
                    );
                    std::process::exit(1);
                } else {
                    tracing::warn!(
                        pid = existing.pid,
                        "stale PID file found (process not running) — overwriting"
                    );
                }
            }

            // --- Write PID file before starting server ---
            let pid_info = pid::PidInfo {
                pid: std::process::id(),
                port: serve_port,
                started: pid::iso_now(),
            };
            if let Err(e) = pid::write(&pid_info) {
                tracing::warn!("failed to write PID file (non-fatal): {e:#}");
            } else {
                tracing::info!(
                    pid = pid_info.pid,
                    port = serve_port,
                    path = %pid::default_path().display(),
                    "PID file written"
                );
            }

            let max_concurrent = gateway_config.concurrency.max as usize;
            let registry = build_registry_from_config(&gateway_config);
            let router = Arc::new(Router::new(gateway_config, registry));
            let limiter = Arc::new(ConcurrencyLimiter::new(max_concurrent));

            let db_path = CostTracker::default_path();
            let cost_tracker = match CostTracker::open(&db_path) {
                Ok(t) => Arc::new(t),
                Err(e) => {
                    tracing::warn!("cost tracker unavailable (fail-open): {e:#}");
                    match CostTracker::open(std::path::Path::new(":memory:")) {
                        Ok(t) => Arc::new(t),
                        Err(e2) => {
                            tracing::error!("cost tracker fallback also failed: {e2:#}");
                            std::process::exit(1);
                        }
                    }
                }
            };

            if let Err(e) = server::serve(
                serve_port,
                router,
                limiter,
                cost_tracker,
                auth_store,
                auth_disabled,
            )
            .await
            {
                tracing::error!("gateway server error: {e:#}");
                pid::cleanup();
                std::process::exit(1);
            }

            // Graceful shutdown: remove PID file
            pid::cleanup();
        }

        Some(Commands::Status { port, config, json }) => {
            // Port resolution order:
            // 1. --port flag (highest priority)
            // 2. PID file port (auto-discovery)
            // 3. Config file port
            // 4. 8080 default
            let (resolved_port, source) = if let Some(p) = port {
                (p, PortSource::Flag)
            } else if let Some(pid_info) = pid::read() {
                (pid_info.port, PortSource::PidFile)
            } else {
                let config_path = config.as_deref();
                let gateway_config = GatewayConfig::load(config_path).unwrap_or_default();
                if gateway_config.port != 8080 {
                    (gateway_config.port, PortSource::Config)
                } else {
                    (gateway_config.port, PortSource::Default)
                }
            };

            match cli::status::run(resolved_port, source, json).await {
                Ok(exit_code) => std::process::exit(exit_code),
                Err(e) => {
                    eprintln!("error: {e:#}");
                    std::process::exit(2);
                }
            }
        }

        Some(Commands::Stats {
            since,
            json,
            provider,
            tier,
        }) => {
            if let Err(e) =
                cli::stats::run(since.as_deref(), json, provider.as_deref(), tier.as_deref())
            {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        }

        Some(Commands::Complete {
            prompt,
            tier,
            json,
            config,
            privacy,
            cost,
            latency,
            provider,
        }) => {
            let config_path = config.as_deref();
            let gateway_config = match GatewayConfig::load(config_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error loading config: {e:#}");
                    std::process::exit(1);
                }
            };

            let model_tier = match tier.parse::<ModelTier>() {
                Ok(t) => t,
                Err(_) => {
                    eprintln!("unknown tier '{}' — use: quick, standard, max, ultra", tier);
                    std::process::exit(1);
                }
            };

            let constraints = CompleteConstraints {
                privacy,
                cost,
                latency,
                provider,
            };

            if let Err(e) =
                cli::complete::run(&gateway_config, model_tier, &prompt, json, &constraints).await
            {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        }

        Some(Commands::Stop { port }) => {
            let exit_code = cli::stop::run(port);
            std::process::exit(exit_code);
        }

        Some(Commands::Keys { action }) => {
            let auth_store = match AuthStore::open(&AuthStore::default_path()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: failed to open auth store: {e:#}");
                    std::process::exit(1);
                }
            };

            match action {
                KeysAction::Create { name } => {
                    cli::keys::create(&auth_store, &name);
                }
                KeysAction::List { json } => {
                    cli::keys::list(&auth_store, json);
                }
                KeysAction::Revoke { prefix } => {
                    let exit_code = cli::keys::revoke(&auth_store, &prefix);
                    std::process::exit(exit_code);
                }
            }
        }

        None => {
            eprintln!("AccelMars Gateway v{}", env!("CARGO_PKG_VERSION"));
            eprintln!("Run `gateway --help` for usage.");
        }
    }
}
