use std::sync::Arc;

use accelmars_gateway_core::{GatewayRequest, Message, ModelTier, RoutingConstraints};

use crate::config::GatewayConfig;
use crate::registry::AdapterRegistry;
use crate::router::Router;

/// `gateway complete` — one-shot completion without running the server.
pub async fn run(
    config: &GatewayConfig,
    tier: ModelTier,
    prompt: &str,
    json_output: bool,
) -> anyhow::Result<()> {
    let registry = build_registry(config);
    let router = Arc::new(Router::new(config.clone(), registry));

    let decision = router
        .resolve(tier, &RoutingConstraints::default())
        .map_err(|e| anyhow::anyhow!("routing failed: {e}"))?;

    let provider_name = decision.provider_name.clone();
    let model_id = decision.model_id.clone();
    let adapter = decision.adapter;

    let gateway_req = GatewayRequest {
        tier,
        constraints: RoutingConstraints::default(),
        messages: vec![Message {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
        max_tokens: None,
        stream: false,
        metadata: Default::default(),
    };

    let start = std::time::Instant::now();
    let result = tokio::task::spawn_blocking(move || adapter.complete(&gateway_req)).await??;
    let latency_ms = start.elapsed().as_millis();

    if json_output {
        let (cost_in, cost_out) = router.provider_pricing(&provider_name);
        let cost_usd = crate::cost::CostTracker::calculate_cost(
            result.tokens_in as u64,
            result.tokens_out as u64,
            cost_in,
            cost_out,
        );
        let obj = serde_json::json!({
            "id": result.id,
            "provider": provider_name,
            "model": model_id,
            "content": result.content,
            "tokens_in": result.tokens_in,
            "tokens_out": result.tokens_out,
            "cost_usd": cost_usd,
            "latency_ms": latency_ms,
            "finish_reason": result.finish_reason,
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else {
        print!("{}", result.content);
        if !result.content.ends_with('\n') {
            println!();
        }
    }

    Ok(())
}

fn build_registry(config: &GatewayConfig) -> AdapterRegistry {
    use crate::adapters::{
        new_deepseek_adapter, new_groq_adapter, new_openrouter_adapter, ClaudeAdapter,
        GeminiAdapter,
    };
    use crate::config::GatewayMode;
    use accelmars_gateway_core::MockAdapter;

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(MockAdapter::default()));

    if config.mode == GatewayMode::Mock {
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
            _ => continue,
        };
        registry.register(adapter);
    }

    registry
}
