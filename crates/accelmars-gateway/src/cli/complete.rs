use std::sync::Arc;

use accelmars_gateway_core::{
    CostPreference, GatewayRequest, Latency, Message, ModelTier, Privacy, RoutingConstraints,
};

use crate::config::GatewayConfig;
use crate::registry::AdapterRegistry;
use crate::router::Router;

/// Routing constraint flags accepted by `gateway complete`.
pub struct CompleteConstraints {
    /// Privacy constraint: "open" | "sensitive" | "private"
    pub privacy: Option<String>,
    /// Cost constraint: "free" | "budget" | "default" | "unlimited"
    pub cost: Option<String>,
    /// Latency constraint: "normal" | "low"
    pub latency: Option<String>,
    /// Explicit provider override (bypasses tier routing)
    pub provider: Option<String>,
}

/// `gateway complete` — one-shot completion without running the server.
pub async fn run(
    config: &GatewayConfig,
    tier: ModelTier,
    prompt: &str,
    json_output: bool,
    constraints_in: &CompleteConstraints,
) -> anyhow::Result<()> {
    let registry = build_registry(config);
    let router = Arc::new(Router::new(config.clone(), registry));

    let constraints = parse_constraints(constraints_in);

    let decision = router
        .resolve(tier, &constraints)
        .map_err(|e| anyhow::anyhow!("routing failed: {e}"))?;

    let provider_name = decision.provider_name.clone();
    let model_id = decision.model_id.clone();
    let adapter = decision.adapter;

    let gateway_req = GatewayRequest {
        tier,
        constraints,
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

/// Parse CLI constraint flags into a `RoutingConstraints` value.
///
/// Applies the same mapping as `server.rs::parse_constraints()` so the
/// CLI and HTTP API behave identically.
pub fn parse_constraints(flags: &CompleteConstraints) -> RoutingConstraints {
    let mut c = RoutingConstraints::default();

    if let Some(ref s) = flags.privacy {
        c.privacy = match s.as_str() {
            "sensitive" => Privacy::Sensitive,
            "private" => Privacy::Private,
            _ => Privacy::Open,
        };
    }
    if let Some(ref s) = flags.latency {
        if s == "low" {
            c.latency = Latency::Low;
        }
    }
    if let Some(ref s) = flags.cost {
        c.cost = match s.as_str() {
            "free" => CostPreference::Free,
            "budget" => CostPreference::Budget,
            "unlimited" => CostPreference::Unlimited,
            _ => CostPreference::Default,
        };
    }
    if let Some(ref s) = flags.provider {
        c.provider = Some(s.clone());
    }

    c
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

// ---------------------------------------------------------------------------
// Unit tests — constraint parsing
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::{CostPreference, Latency, Privacy};

    #[test]
    fn no_flags_produces_default_constraints() {
        let flags = CompleteConstraints {
            privacy: None,
            cost: None,
            latency: None,
            provider: None,
        };
        let c = parse_constraints(&flags);
        assert_eq!(c.privacy, Privacy::Open);
        assert_eq!(c.cost, CostPreference::Default);
        assert_eq!(c.latency, Latency::Normal);
        assert!(c.provider.is_none());
    }

    #[test]
    fn privacy_sensitive_flag_sets_constraint() {
        let flags = CompleteConstraints {
            privacy: Some("sensitive".to_string()),
            cost: None,
            latency: None,
            provider: None,
        };
        let c = parse_constraints(&flags);
        assert_eq!(c.privacy, Privacy::Sensitive);
    }

    #[test]
    fn cost_free_flag_sets_constraint() {
        let flags = CompleteConstraints {
            privacy: None,
            cost: Some("free".to_string()),
            latency: None,
            provider: None,
        };
        let c = parse_constraints(&flags);
        assert_eq!(c.cost, CostPreference::Free);
    }

    #[test]
    fn provider_override_flag_sets_provider() {
        let flags = CompleteConstraints {
            privacy: None,
            cost: None,
            latency: None,
            provider: Some("claude".to_string()),
        };
        let c = parse_constraints(&flags);
        assert_eq!(c.provider.as_deref(), Some("claude"));
    }

    #[test]
    fn latency_low_flag_sets_constraint() {
        let flags = CompleteConstraints {
            privacy: None,
            cost: None,
            latency: Some("low".to_string()),
            provider: None,
        };
        let c = parse_constraints(&flags);
        assert_eq!(c.latency, Latency::Low);
    }
}
