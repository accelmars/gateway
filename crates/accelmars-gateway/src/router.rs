use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use accelmars_gateway_core::{ModelTier, ProviderAdapter, RoutingConstraints};

use crate::config::{GatewayConfig, GatewayMode};
use crate::registry::AdapterRegistry;

/// The result of routing a request — provider selected + adapter ready to use.
pub struct RouteDecision {
    /// Resolved provider name (for logging, cost tracking).
    pub provider_name: String,
    /// Actual model ID to send (from provider config).
    pub model_id: String,
    /// Adapter ready to call.
    pub adapter: Arc<dyn ProviderAdapter>,
}

/// Per-provider health state for failure tracking and circuit breaking.
#[derive(Debug, Clone)]
struct ProviderHealth {
    consecutive_failures: u32,
    last_failure: Option<Instant>,
    /// After this many consecutive failures, provider is marked unavailable.
    max_failures: u32,
    /// How long to wait before retrying after max_failures.
    cooldown: Duration,
}

impl ProviderHealth {
    fn new() -> Self {
        Self {
            consecutive_failures: 0,
            last_failure: None,
            max_failures: 3,
            cooldown: Duration::from_secs(30),
        }
    }

    /// Returns true if this provider should be skipped (too many recent failures).
    fn is_unavailable(&self) -> bool {
        if self.consecutive_failures < self.max_failures {
            return false;
        }
        // Check if cooldown has elapsed
        match self.last_failure {
            None => false,
            Some(t) => t.elapsed() < self.cooldown,
        }
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        self.last_failure = Some(Instant::now());
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.last_failure = None;
    }
}

/// Routes requests to providers based on tier, constraints, and health state.
///
/// Lives in the binary crate — not in core. Core defines types only.
pub struct Router {
    config: GatewayConfig,
    registry: AdapterRegistry,
    health: Mutex<HashMap<String, ProviderHealth>>,
}

impl Router {
    pub fn new(config: GatewayConfig, registry: AdapterRegistry) -> Self {
        Self {
            config,
            registry,
            health: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve a provider for the given tier and constraints.
    ///
    /// Resolution order:
    /// 1. If GATEWAY_MODE=mock → MockAdapter
    /// 2. If constraints.provider is Some → direct lookup (bypass routing)
    /// 3. Get default provider for tier from config
    /// 4. Apply constraint filters (privacy, cost, latency)
    /// 5. If filtered set differs from default, select best available from filtered set
    /// 6. Try fallback if primary is unavailable
    pub fn resolve(
        &self,
        tier: ModelTier,
        constraints: &RoutingConstraints,
    ) -> Result<RouteDecision, RouterError> {
        // 1. Mock mode — always mock
        if self.config.mode == GatewayMode::Mock {
            return self.resolve_mock();
        }

        // 2. Explicit provider override
        if let Some(ref provider_name) = constraints.provider {
            return self.resolve_named(provider_name);
        }

        // 3. Get default provider for tier
        let default_provider = self.config.tiers.provider_for_tier(tier).to_string();

        // 4-5. Apply constraint filters to find candidate set
        let candidates = self.apply_constraints(tier, constraints);

        // Select from candidates, falling back to default if needed
        let selected = if candidates.is_empty() {
            // No filtered candidates — try the tier default directly
            default_provider.clone()
        } else if candidates.contains(&default_provider) {
            // Default is valid within constraints
            default_provider.clone()
        } else {
            // Default excluded by constraints — pick first healthy candidate
            candidates
                .into_iter()
                .find(|name| self.provider_is_healthy(name))
                .unwrap_or(default_provider.clone())
        };

        // 6. Try selected provider, then fallback
        self.resolve_with_fallback(&selected, 0)
    }

    /// Report a successful completion for a provider (resets failure count).
    pub fn on_success(&self, provider_name: &str) {
        let mut health = self.health.lock().expect("health lock poisoned");
        health
            .entry(provider_name.to_string())
            .or_insert_with(ProviderHealth::new)
            .record_success();
    }

    /// Report a failed completion for a provider (may trigger circuit break).
    pub fn on_failure(&self, provider_name: &str) {
        let mut health = self.health.lock().expect("health lock poisoned");
        health
            .entry(provider_name.to_string())
            .or_insert_with(ProviderHealth::new)
            .record_failure();
        let failures = health[provider_name].consecutive_failures;
        if failures >= health[provider_name].max_failures {
            tracing::warn!(
                provider = provider_name,
                consecutive_failures = failures,
                "provider marked unavailable — circuit open"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn resolve_mock(&self) -> Result<RouteDecision, RouterError> {
        let adapter = self.registry.get("mock").ok_or_else(|| {
            RouterError::NoProviderAvailable("mock adapter not registered".into())
        })?;
        Ok(RouteDecision {
            provider_name: "mock".to_string(),
            model_id: "mock".to_string(),
            adapter,
        })
    }

    fn resolve_named(&self, provider_name: &str) -> Result<RouteDecision, RouterError> {
        let adapter = self.registry.get(provider_name).ok_or_else(|| {
            RouterError::NoProviderAvailable(format!("provider '{provider_name}' not registered"))
        })?;
        if !adapter.is_available() {
            return Err(RouterError::NoProviderAvailable(format!(
                "provider '{provider_name}' is not available (missing API key)"
            )));
        }
        let model_id = self
            .config
            .providers
            .get(provider_name)
            .map(|p| p.model.clone())
            .unwrap_or_else(|| provider_name.to_string());
        Ok(RouteDecision {
            provider_name: provider_name.to_string(),
            model_id,
            adapter,
        })
    }

    fn resolve_with_fallback(
        &self,
        provider_name: &str,
        depth: u32,
    ) -> Result<RouteDecision, RouterError> {
        // Prevent infinite fallback loops
        if depth > 3 {
            return Err(RouterError::NoProviderAvailable(
                "fallback chain exhausted".into(),
            ));
        }

        let adapter = self.registry.get(provider_name);
        let health_ok = self.provider_is_healthy(provider_name);

        let adapter = match adapter {
            Some(a) if a.is_available() && health_ok => a,
            Some(_) | None => {
                // Provider unavailable or unhealthy — try fallback
                let fallback = self
                    .config
                    .providers
                    .get(provider_name)
                    .and_then(|p| p.fallback.as_deref())
                    .map(str::to_string);

                if let Some(fb) = fallback {
                    tracing::warn!(
                        primary = provider_name,
                        fallback = %fb,
                        "fallback triggered"
                    );
                    return self.resolve_with_fallback(&fb, depth + 1);
                }

                // No fallback configured — try mock if available
                if let Some(mock) = self.registry.get("mock") {
                    tracing::warn!(
                        provider = provider_name,
                        "no fallback configured — routing to mock"
                    );
                    return Ok(RouteDecision {
                        provider_name: "mock".to_string(),
                        model_id: "mock".to_string(),
                        adapter: mock,
                    });
                }

                return Err(RouterError::NoProviderAvailable(format!(
                    "provider '{provider_name}' unavailable and no fallback configured"
                )));
            }
        };

        let model_id = self
            .config
            .providers
            .get(provider_name)
            .map(|p| p.model.clone())
            .unwrap_or_else(|| provider_name.to_string());

        Ok(RouteDecision {
            provider_name: provider_name.to_string(),
            model_id,
            adapter,
        })
    }

    /// Returns provider names that satisfy the routing constraints.
    fn apply_constraints(&self, tier: ModelTier, constraints: &RoutingConstraints) -> Vec<String> {
        use accelmars_gateway_core::{CostPreference, Latency, Privacy};

        self.config
            .providers
            .keys()
            .filter(|name| {
                let Some(provider) = self.config.providers.get(*name) else {
                    return false;
                };

                // Privacy filter
                match constraints.privacy {
                    Privacy::Sensitive => {
                        if self.config.constraints.sensitive_excluded.contains(name) {
                            return false;
                        }
                    }
                    Privacy::Private => {
                        if !self.config.constraints.private_only.contains(name) {
                            return false;
                        }
                    }
                    Privacy::Open => {}
                }

                // Cost filter
                if constraints.cost == CostPreference::Free {
                    let is_free = provider.tags.contains(&"free".to_string())
                        || self.config.constraints.free_only.contains(name);
                    if !is_free {
                        return false;
                    }
                }

                // Latency filter — prefer fast providers (but don't exclude slow ones)
                if constraints.latency == Latency::Low {
                    // For low latency: only return preferred providers
                    if !self.config.constraints.low_latency_preferred.contains(name) {
                        return false;
                    }
                }

                // Default tier match — only include providers relevant to this tier
                // (providers in the tier mapping or with matching tags)
                let _ = tier; // tier used for ordering, not hard exclusion at this stage

                true
            })
            .cloned()
            .collect()
    }

    fn provider_is_healthy(&self, provider_name: &str) -> bool {
        let health = self.health.lock().expect("health lock poisoned");
        health
            .get(provider_name)
            .map(|h| !h.is_unavailable())
            .unwrap_or(true) // unknown providers are assumed healthy
    }
}

/// Errors from the router.
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("no provider available: {0}")]
    NoProviderAvailable(String),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use accelmars_gateway_core::{
        CostPreference, Latency, MockAdapter, Privacy, RoutingConstraints,
    };

    use super::*;
    use crate::config::{ConstraintRules, GatewayMode, ProviderConfig, TierConfig};

    fn make_config(mode: GatewayMode) -> GatewayConfig {
        let mut providers = HashMap::new();

        providers.insert(
            "gemini".to_string(),
            ProviderConfig {
                api_key_env: "GEMINI_API_KEY".to_string(),
                model: "gemini-2.5-flash-lite".to_string(),
                max_tokens: None,
                timeout_seconds: 60,
                tags: vec!["free".to_string()],
                fallback: Some("deepseek".to_string()),
                cost_per_1m_input: 0.0,
                cost_per_1m_output: 0.0,
            },
        );
        providers.insert(
            "deepseek".to_string(),
            ProviderConfig {
                api_key_env: "DEEPSEEK_API_KEY".to_string(),
                model: "deepseek-chat".to_string(),
                max_tokens: None,
                timeout_seconds: 120,
                tags: vec![],
                fallback: None,
                cost_per_1m_input: 0.28,
                cost_per_1m_output: 0.42,
            },
        );
        providers.insert(
            "claude".to_string(),
            ProviderConfig {
                api_key_env: "ANTHROPIC_API_KEY".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                max_tokens: None,
                timeout_seconds: 120,
                tags: vec!["sensitive_ok".to_string()],
                fallback: None,
                cost_per_1m_input: 3.0,
                cost_per_1m_output: 15.0,
            },
        );
        providers.insert(
            "groq".to_string(),
            ProviderConfig {
                api_key_env: "GROQ_API_KEY".to_string(),
                model: "llama-3.3-70b-versatile".to_string(),
                max_tokens: None,
                timeout_seconds: 30,
                tags: vec!["fast".to_string(), "free".to_string()],
                fallback: None,
                cost_per_1m_input: 0.0,
                cost_per_1m_output: 0.0,
            },
        );

        GatewayConfig {
            port: 8080,
            log_level: "info".to_string(),
            mode,
            concurrency: crate::config::ConcurrencyConfig { max: 20 },
            tiers: TierConfig {
                quick: "gemini".to_string(),
                standard: "deepseek".to_string(),
                max: "claude".to_string(),
                ultra: "claude".to_string(),
            },
            providers,
            constraints: ConstraintRules {
                sensitive_excluded: vec!["deepseek".to_string()],
                private_only: vec![],
                low_latency_preferred: vec!["groq".to_string()],
                free_only: vec!["gemini".to_string(), "groq".to_string()],
            },
        }
    }

    fn make_registry_with_available(names: &[&str]) -> AdapterRegistry {
        let mut registry = AdapterRegistry::new();
        // Always register mock
        registry.register(Arc::new(MockAdapter::default()));
        for &name in names {
            // Register a named mock adapter to simulate an available provider
            registry.register(Arc::new(MockAdapter::default().with_name(name)));
        }
        registry
    }

    #[test]
    fn mock_mode_always_routes_to_mock() {
        let config = make_config(GatewayMode::Mock);
        let registry = make_registry_with_available(&["deepseek", "claude"]);
        let router = Router::new(config, registry);

        let decision = router
            .resolve(ModelTier::Standard, &RoutingConstraints::default())
            .unwrap();
        assert_eq!(decision.provider_name, "mock");
    }

    #[test]
    fn provider_override_bypasses_tier_routing() {
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["gemini", "deepseek", "claude"]);
        let router = Router::new(config, registry);

        let mut constraints = RoutingConstraints::default();
        constraints.provider = Some("claude".to_string());

        let decision = router.resolve(ModelTier::Quick, &constraints).unwrap();
        assert_eq!(decision.provider_name, "claude");
        assert_eq!(decision.model_id, "claude-sonnet-4-6");
    }

    #[test]
    fn tier_maps_to_default_provider() {
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["gemini", "deepseek", "claude"]);
        let router = Router::new(config, registry);

        let decision = router
            .resolve(ModelTier::Standard, &RoutingConstraints::default())
            .unwrap();
        assert_eq!(decision.provider_name, "deepseek");
        assert_eq!(decision.model_id, "deepseek-chat");
    }

    #[test]
    fn privacy_sensitive_excludes_deepseek() {
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["gemini", "deepseek", "claude"]);
        let router = Router::new(config, registry);

        let constraints = RoutingConstraints {
            privacy: Privacy::Sensitive,
            ..Default::default()
        };

        // Standard tier maps to deepseek, but deepseek is excluded for sensitive
        // Should fall back or select from non-excluded candidates
        let decision = router.resolve(ModelTier::Standard, &constraints).unwrap();
        assert_ne!(decision.provider_name, "deepseek");
    }

    #[test]
    fn cost_free_selects_free_tier_only() {
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["gemini", "deepseek", "claude", "groq"]);
        let router = Router::new(config, registry);

        let constraints = RoutingConstraints {
            cost: CostPreference::Free,
            ..Default::default()
        };

        let decision = router.resolve(ModelTier::Standard, &constraints).unwrap();
        // deepseek is the default for standard but is not free — should select gemini or groq
        assert!(
            decision.provider_name == "gemini" || decision.provider_name == "groq",
            "expected free provider, got {}",
            decision.provider_name
        );
    }

    #[test]
    fn health_tracking_marks_provider_unavailable_after_3_failures() {
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["gemini", "deepseek"]);
        let router = Router::new(config, registry);

        // Record 3 consecutive failures for deepseek
        router.on_failure("deepseek");
        router.on_failure("deepseek");
        router.on_failure("deepseek");

        assert!(!router.provider_is_healthy("deepseek"));
    }

    #[test]
    fn health_tracking_resets_on_success() {
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["gemini", "deepseek"]);
        let router = Router::new(config, registry);

        router.on_failure("deepseek");
        router.on_failure("deepseek");
        // Reset with a success
        router.on_success("deepseek");
        router.on_failure("deepseek"); // only 1 failure now — not at threshold

        assert!(router.provider_is_healthy("deepseek"));
    }

    #[test]
    fn fallback_triggered_when_primary_unavailable() {
        let config = make_config(GatewayMode::Normal);
        // Only register deepseek as fallback for gemini — gemini registered but not available
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(MockAdapter::default()));
        // gemini not registered (simulates unavailable)
        registry.register(Arc::new(MockAdapter::default().with_name("deepseek")));
        let router = Router::new(config, registry);

        // quick tier maps to gemini, which is unavailable — should fall back to deepseek
        let decision = router
            .resolve(ModelTier::Quick, &RoutingConstraints::default())
            .unwrap();
        assert_eq!(decision.provider_name, "deepseek");
    }

    #[test]
    fn latency_low_prefers_groq() {
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["gemini", "deepseek", "claude", "groq"]);
        let router = Router::new(config, registry);

        let constraints = RoutingConstraints {
            latency: Latency::Low,
            ..Default::default()
        };

        let decision = router.resolve(ModelTier::Standard, &constraints).unwrap();
        assert_eq!(decision.provider_name, "groq");
    }

    #[test]
    fn missing_api_key_provider_unavailable_others_still_work() {
        // This is tested through the registry: an adapter registered without a key
        // is is_available() = false.
        let config = make_config(GatewayMode::Normal);
        // Only register deepseek as actually available; gemini not registered
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(MockAdapter::default()));
        // gemini registered but missing API key — simulated by not registering it
        registry.register(Arc::new(MockAdapter::default().with_name("deepseek")));
        registry.register(Arc::new(MockAdapter::default().with_name("claude")));
        let router = Router::new(config, registry);

        // Quick tier (gemini) unavailable — falls back; standard and max still work
        let standard = router
            .resolve(ModelTier::Standard, &RoutingConstraints::default())
            .unwrap();
        assert_eq!(standard.provider_name, "deepseek");

        let max = router
            .resolve(ModelTier::Max, &RoutingConstraints::default())
            .unwrap();
        assert_eq!(max.provider_name, "claude");
    }
}
