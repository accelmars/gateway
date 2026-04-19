use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "gateway",
    version,
    about = "Universal AI gateway — multi-provider, OpenAI-compatible, Rust-native",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the gateway server (OpenAI-compatible API on localhost)
    Serve,
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

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Serve) => {
            eprintln!("gateway serve: not yet implemented — coming in PF-002");
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
