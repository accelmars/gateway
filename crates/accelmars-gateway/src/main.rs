use std::sync::Arc;

use clap::{Parser, Subcommand};

use accelmars_gateway::adapters::{
    new_deepseek_adapter, new_groq_adapter, new_openrouter_adapter, ClaudeAdapter, GeminiAdapter,
};
use accelmars_gateway::registry::AdapterRegistry;
use accelmars_gateway::server;
use accelmars_gateway_core::MockAdapter;

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
        /// Port to listen on
        #[arg(long, default_value = "8080", env = "GATEWAY_PORT")]
        port: u16,
    },
    /// Show gateway health and provider availability
    Status,
    /// Show cost summary and call statistics
    Stats,
    /// Execute a single completion (one-shot mode)
    Complete {
        /// Prompt to complete
        prompt: String,
    },
}

fn build_registry() -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();
    let mode = std::env::var("GATEWAY_MODE").unwrap_or_default();

    // Mock adapter — always registered
    registry.register(Arc::new(MockAdapter::default()));

    if mode == "mock" {
        tracing::info!("GATEWAY_MODE=mock — mock adapter only");
        return registry;
    }

    // Gemini (free tier) — GEMINI_API_KEY
    let gemini_key = std::env::var("GEMINI_API_KEY").ok();
    let gemini_model =
        std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-flash".to_string());
    registry.register(Arc::new(GeminiAdapter::new(gemini_key, gemini_model)));

    // DeepSeek — DEEPSEEK_API_KEY
    let deepseek_key = std::env::var("DEEPSEEK_API_KEY").ok();
    registry.register(Arc::new(new_deepseek_adapter(deepseek_key)));

    // Claude (Anthropic) — ANTHROPIC_API_KEY
    let claude_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let claude_model =
        std::env::var("CLAUDE_MODEL").unwrap_or_else(|_| "claude-sonnet-4-6".to_string());
    registry.register(Arc::new(ClaudeAdapter::new(claude_key, claude_model)));

    // OpenRouter — OPENROUTER_API_KEY
    let openrouter_key = std::env::var("OPENROUTER_API_KEY").ok();
    registry.register(Arc::new(new_openrouter_adapter(openrouter_key)));

    // Groq — GROQ_API_KEY
    let groq_key = std::env::var("GROQ_API_KEY").ok();
    registry.register(Arc::new(new_groq_adapter(groq_key)));

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
        Some(Commands::Serve { port }) => {
            let registry = Arc::new(build_registry());

            if let Err(e) = server::serve(port, registry).await {
                tracing::error!("gateway server error: {e:#}");
                std::process::exit(1);
            }
        }
        Some(Commands::Status) => {
            eprintln!("gateway status: not yet implemented — coming in PF-005");
        }
        Some(Commands::Stats) => {
            eprintln!("gateway stats: not yet implemented — coming in PF-005");
        }
        Some(Commands::Complete { prompt }) => {
            eprintln!("gateway complete '{prompt}': not yet implemented — coming in PF-005");
        }
        None => {
            eprintln!("AccelMars Gateway v{}", env!("CARGO_PKG_VERSION"));
            eprintln!("Run `gateway --help` for usage.");
        }
    }
}
