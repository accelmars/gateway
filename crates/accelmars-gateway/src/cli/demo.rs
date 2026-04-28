use std::io::Write;
use std::time::{Duration, Instant};

use accelmars_gateway_core::{
    GatewayRequest, Message, MockAdapter, ModelTier, ProviderAdapter, RoutingConstraints,
};

const DEMO_PROMPT: &str = "Hello from AccelMars Gateway! Tell me what you are.";

struct TierResult {
    tier: ModelTier,
    model: &'static str,
    tokens_out: u32,
    latency_ms: u128,
}

/// `gateway demo` — zero-config mock mode showcase.
///
/// Cycles Quick → Standard → Max, streams each response, prints a comparison table.
/// No API key. No config. No server process.
pub async fn run() -> anyhow::Result<()> {
    println!("AccelMars Gateway — mock mode demo");
    println!("No API key or configuration needed.\n");
    println!("Prompt: \"{DEMO_PROMPT}\"\n");

    let tiers: &[(ModelTier, Duration, &'static str)] = &[
        (ModelTier::Quick, Duration::from_millis(60), "quick-mock-v1"),
        (
            ModelTier::Standard,
            Duration::from_millis(180),
            "standard-mock-v1",
        ),
        (ModelTier::Max, Duration::from_millis(400), "max-mock-v1"),
    ];

    let mut results: Vec<TierResult> = Vec::new();

    for &(tier, latency, model) in tiers {
        let response_content = "I am AccelMars Gateway — a Rust-native, multi-provider AI router. \
             In production I route to the best available provider for your quality tier. \
             Zero-key local dev. Deterministic CI cassettes. 13 providers. Streaming native."
            .to_string();

        let adapter = MockAdapter::new(response_content)
            .with_latency(latency)
            .with_name(model);

        let request = GatewayRequest {
            tier,
            constraints: RoutingConstraints::default(),
            messages: vec![Message {
                role: "user".to_string(),
                content: DEMO_PROMPT.to_string(),
            }],
            max_tokens: None,
            stream: false,
            metadata: Default::default(),
        };

        print!("[{tier}] ");
        std::io::stdout().flush()?;

        let start = Instant::now();
        let response = tokio::task::spawn_blocking(move || adapter.complete(&request)).await??;
        let latency_ms = start.elapsed().as_millis();

        let content = response.content;
        let tokens_out = response.tokens_out;

        for word in content.split_whitespace() {
            print!("{word} ");
            std::io::stdout().flush()?;
            tokio::time::sleep(Duration::from_millis(12)).await;
        }
        println!();
        println!("   ↳ {latency_ms} ms | {tokens_out} tokens out\n");

        results.push(TierResult {
            tier,
            model,
            tokens_out,
            latency_ms,
        });
    }

    println!(
        "{:<12} {:<18} {:>8} {:>10}",
        "Tier", "Model selected", "Tokens", "Latency"
    );
    println!("{}", "-".repeat(52));
    for r in &results {
        println!(
            "{:<12} {:<18} {:>8} {:>7} ms",
            r.tier.to_string(),
            r.model,
            r.tokens_out,
            r.latency_ms,
        );
    }

    println!();
    println!(
        "Run `gateway init` to configure real providers, or `GATEWAY_MODE=fixture` for CI cassettes."
    );

    Ok(())
}
