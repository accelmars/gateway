use std::sync::Arc;

use clap::{Parser, Subcommand};

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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize structured tracing
    let filter = tracing_subscriber::EnvFilter::try_new(&cli.log_level)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match cli.command {
        Some(Commands::Serve { port }) => {
            let mode = std::env::var("GATEWAY_MODE").unwrap_or_default();
            // Phase 1: MockAdapter is the only provider.
            // Real adapters (Gemini, DeepSeek, Claude) arrive in PF-003.
            let adapter: Arc<dyn accelmars_gateway_core::ProviderAdapter> = if mode == "mock" {
                tracing::info!("GATEWAY_MODE=mock — using deterministic mock adapter");
                Arc::new(MockAdapter::default())
            } else {
                tracing::info!("starting in phase-1 mode — mock adapter (real providers: PF-003)");
                Arc::new(MockAdapter::default())
            };

            if let Err(e) = server::serve(port, adapter).await {
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
