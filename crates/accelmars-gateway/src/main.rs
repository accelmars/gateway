use std::sync::Arc;

use clap::{Parser, Subcommand};

use accelmars_gateway::adapters::{
    new_deepseek_adapter, new_groq_adapter, new_openrouter_adapter, ClaudeAdapter, GeminiAdapter,
};
use accelmars_gateway::cli;
use accelmars_gateway::concurrency::ConcurrencyLimiter;
use accelmars_gateway::config::GatewayConfig;
use accelmars_gateway::cost::CostTracker;
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
        /// Port the server is listening on (default: read from config)
        #[arg(long)]
        port: Option<u16>,
        /// Path to config file (to read port default)
        #[arg(long)]
        config: Option<std::path::PathBuf>,
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
            let max_concurrent = gateway_config.concurrency.max as usize;
            let registry = build_registry_from_config(&gateway_config);
            let router = Arc::new(Router::new(gateway_config, registry));
            let limiter = Arc::new(ConcurrencyLimiter::new(max_concurrent));

            let db_path = CostTracker::default_path();
            let cost_tracker = match CostTracker::open(&db_path) {
                Ok(t) => Arc::new(t),
                Err(e) => {
                    tracing::warn!("cost tracker unavailable (fail-open): {e:#}");
                    // Create an in-memory fallback so the server still starts
                    match CostTracker::open(std::path::Path::new(":memory:")) {
                        Ok(t) => Arc::new(t),
                        Err(e2) => {
                            tracing::error!("cost tracker fallback also failed: {e2:#}");
                            std::process::exit(1);
                        }
                    }
                }
            };

            if let Err(e) = server::serve(serve_port, router, limiter, cost_tracker).await {
                tracing::error!("gateway server error: {e:#}");
                std::process::exit(1);
            }
        }

        Some(Commands::Status { port, config }) => {
            let config_path = config.as_deref();
            let gateway_config = GatewayConfig::load(config_path).unwrap_or_default();
            let resolved_port = port.unwrap_or(gateway_config.port);

            if let Err(e) = cli::status::run(resolved_port).await {
                eprintln!("error: {e:#}");
                std::process::exit(1);
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

            if let Err(e) = cli::complete::run(&gateway_config, model_tier, &prompt, json).await {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        }

        None => {
            eprintln!("AccelMars Gateway v{}", env!("CARGO_PKG_VERSION"));
            eprintln!("Run `gateway --help` for usage.");
        }
    }
}
