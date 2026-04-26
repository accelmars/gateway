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

/// Circuit breaker state machine for a single provider.
///
/// ```text
/// CLOSED ──(max_failures consecutive failures)──► OPEN
/// OPEN   ──(current_cooldown elapsed)──────────► HALF_OPEN
/// HALF_OPEN ──(on_success)──────────────────────► CLOSED  (reset cooldown)
/// HALF_OPEN ──(on_failure)──────────────────────► OPEN    (cooldown *= 2)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitState {
    /// Normal operation — requests flow through.
    Closed,
    /// Provider failed too many times — all requests immediately fall back.
    Open,
    /// Cooldown elapsed — exactly one probe request allowed through.
    HalfOpen,
}

impl CircuitState {
    fn as_str(self) -> &'static str {
        match self {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half_open",
        }
    }
}

/// Per-provider health state with circuit breaker + exponential backoff.
#[derive(Debug, Clone)]
struct ProviderHealth {
    state: CircuitState,
    consecutive_failures: u32,
    last_failure: Option<Instant>,
    /// When the state last changed (used for OPEN → HALF-OPEN transition timing).
    last_state_change: Option<Instant>,
    /// Open circuit after this many consecutive failures. Default: 3.
    max_failures: u32,
    /// Initial cooldown before probe attempt. Default: 30s.
    base_cooldown: Duration,
    /// Current cooldown — starts at base, doubles on each HALF-OPEN probe failure.
    current_cooldown: Duration,
    /// Maximum cooldown cap. Default: 300s (5 min).
    max_cooldown: Duration,
}

impl ProviderHealth {
    fn new() -> Self {
        let base = Duration::from_secs(30);
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            last_failure: None,
            last_state_change: None,
            max_failures: 3,
            base_cooldown: base,
            current_cooldown: base,
            max_cooldown: Duration::from_secs(300),
        }
    }

    /// Returns true if a request should be attempted on this provider.
    ///
    /// Side effect: transitions OPEN → HALF-OPEN when `current_cooldown` has elapsed.
    /// In HALF-OPEN state, claims the probe slot on first call (subsequent calls return false).
    fn should_attempt(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if cooldown has elapsed → transition to HALF-OPEN
                if let Some(changed) = self.last_state_change {
                    if changed.elapsed() >= self.current_cooldown {
                        self.state = CircuitState::HalfOpen;
                        self.last_state_change = Some(Instant::now());
                        true // allow probe
                    } else {
                        false // still cooling down
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => {
                // Probe slot already claimed — block all other requests
                false
            }
        }
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        self.last_failure = Some(Instant::now());

        match self.state {
            CircuitState::Closed => {
                if self.consecutive_failures >= self.max_failures {
                    self.state = CircuitState::Open;
                    self.last_state_change = Some(Instant::now());
                    // current_cooldown stays at base on first trip open
                }
            }
            CircuitState::HalfOpen => {
                // Probe failed — back to OPEN with doubled cooldown (exponential backoff)
                self.state = CircuitState::Open;
                self.last_state_change = Some(Instant::now());
                self.current_cooldown = (self.current_cooldown * 2).min(self.max_cooldown);
            }
            CircuitState::Open => {
                // Defensive: requests should be blocked in OPEN state
            }
        }
    }

    fn record_success(&mut self) {
        match self.state {
            CircuitState::HalfOpen => {
                // Probe succeeded — full reset
                self.state = CircuitState::Closed;
                self.consecutive_failures = 0;
                self.last_failure = None;
                self.last_state_change = Some(Instant::now());
                self.current_cooldown = self.base_cooldown; // reset backoff
            }
            CircuitState::Closed => {
                self.consecutive_failures = 0;
                self.last_failure = None;
            }
            CircuitState::Open => {
                // Defensive: treat as recovery
                self.state = CircuitState::Closed;
                self.consecutive_failures = 0;
                self.last_failure = None;
                self.last_state_change = Some(Instant::now());
                self.current_cooldown = self.base_cooldown;
            }
        }
    }

    /// Cooldown remaining in seconds (for /status). Returns None if not OPEN.
    fn cooldown_remaining_secs(&self) -> Option<u64> {
        if self.state != CircuitState::Open {
            return None;
        }
        self.last_state_change.map(|t| {
            let elapsed = t.elapsed();
            if elapsed >= self.current_cooldown {
                0
            } else {
                (self.current_cooldown - elapsed).as_secs()
            }
        })
    }

    /// Override `last_state_change` — test helper for time-sensitive transition tests.
    #[cfg(test)]
    fn set_last_state_change(&mut self, t: Instant) {
        self.last_state_change = Some(t);
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
    /// 6. Try fallback if primary is unavailable or circuit OPEN
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

        // 6. Try selected provider, then fallback chain
        self.resolve_with_fallback(&selected, 0, None)
    }

    /// Report a successful completion for a provider (resets failure count and circuit state).
    pub fn on_success(&self, provider_name: &str) {
        let mut health = self.health.lock().expect("health lock poisoned");
        health
            .entry(provider_name.to_string())
            .or_insert_with(ProviderHealth::new)
            .record_success();
    }

    /// Report a failed completion for a provider (may trigger circuit state transition).
    pub fn on_failure(&self, provider_name: &str) {
        let mut health = self.health.lock().expect("health lock poisoned");
        let h = health
            .entry(provider_name.to_string())
            .or_insert_with(ProviderHealth::new);
        let old_state = h.state;
        h.record_failure();
        let new_state = h.state;

        if old_state != new_state {
            tracing::warn!(
                provider = provider_name,
                old_state = old_state.as_str(),
                new_state = new_state.as_str(),
                consecutive_failures = h.consecutive_failures,
                cooldown_secs = h.current_cooldown.as_secs(),
                "circuit breaker state transition"
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

    /// Walk the fallback chain until a healthy, available provider is found.
    ///
    /// `original_provider`: the first provider in the chain (used for cost comparison).
    /// Lock discipline: `provider_is_healthy()` acquires the health mutex for the duration of
    /// the health check only — the lock is released before the recursive call.
    fn resolve_with_fallback(
        &self,
        provider_name: &str,
        depth: u32,
        original_provider: Option<&str>,
    ) -> Result<RouteDecision, RouterError> {
        // Prevent infinite fallback loops
        if depth > 3 {
            return Err(RouterError::NoProviderAvailable(
                "fallback chain exhausted".into(),
            ));
        }

        let adapter = self.registry.get(provider_name);
        // Lock acquired here, checked, released before recursive call below
        let health_ok = self.provider_is_healthy(provider_name);

        let adapter = match adapter {
            Some(a) if a.is_available() && health_ok => a,
            Some(_) | None => {
                // Provider unavailable or circuit OPEN — try fallback
                let fallback = self
                    .config
                    .providers
                    .get(provider_name)
                    .and_then(|p| p.fallback.as_deref())
                    .map(str::to_string);

                if let Some(fb) = fallback {
                    // original tracks the first provider for cost comparison across the chain
                    let original = original_provider.unwrap_or(provider_name);
                    let (orig_in, orig_out) = self.provider_pricing(original);
                    let (fb_in, fb_out) = self.provider_pricing(&fb);

                    if fb_in > orig_in || fb_out > orig_out {
                        tracing::warn!(
                            primary = original,
                            fallback = %fb,
                            primary_cost_in = orig_in,
                            fallback_cost_in = fb_in,
                            primary_cost_out = orig_out,
                            fallback_cost_out = fb_out,
                            "fallback cost premium — request will cost more than expected"
                        );
                    } else {
                        tracing::warn!(
                            primary = provider_name,
                            fallback = %fb,
                            "fallback triggered"
                        );
                    }
                    return self.resolve_with_fallback(&fb, depth + 1, Some(original));
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

    /// Returns true if the provider should accept a request right now.
    ///
    /// Side effect via `should_attempt()`: may transition OPEN → HALF-OPEN.
    /// Lock acquired for health check only — released before adapter call.
    fn provider_is_healthy(&self, provider_name: &str) -> bool {
        let mut health = self.health.lock().expect("health lock poisoned");
        health
            .entry(provider_name.to_string())
            .or_insert_with(ProviderHealth::new)
            .should_attempt()
    }
}

/// Provider availability info for the /status endpoint.
#[derive(Debug, serde::Serialize)]
pub struct ProviderStatusInfo {
    pub name: String,
    pub available: bool,
    pub tags: Vec<String>,
    /// Circuit breaker state: "closed" | "open" | "half_open"
    pub circuit_state: String,
    pub consecutive_failures: u32,
    /// Seconds until the circuit may probe again. None if not OPEN.
    pub cooldown_remaining_secs: Option<u64>,
}

impl Router {
    /// Gateway operating mode (for /status).
    pub fn mode(&self) -> crate::config::GatewayMode {
        self.config.mode
    }

    /// Returns (cost_per_1m_input, cost_per_1m_output) for a provider.
    /// Returns (0.0, 0.0) if provider not found.
    pub fn provider_pricing(&self, provider_name: &str) -> (f64, f64) {
        self.config
            .providers
            .get(provider_name)
            .map(|p| (p.cost_per_1m_input, p.cost_per_1m_output))
            .unwrap_or((0.0, 0.0))
    }

    /// List all registered providers with availability, tags, and circuit state (for /status).
    pub fn provider_statuses(&self) -> Vec<ProviderStatusInfo> {
        let health = self.health.lock().expect("health lock poisoned");
        let mut statuses: Vec<ProviderStatusInfo> = self
            .registry
            .all_providers()
            .into_iter()
            .map(|name| {
                let available = self
                    .registry
                    .get(name)
                    .map(|a| a.is_available())
                    .unwrap_or(false);
                let tags = self
                    .config
                    .providers
                    .get(name)
                    .map(|p| p.tags.clone())
                    .unwrap_or_default();
                let h = health.get(name);
                let circuit_state = h.map(|h| h.state.as_str()).unwrap_or("closed").to_string();
                let consecutive_failures = h.map(|h| h.consecutive_failures).unwrap_or(0);
                let cooldown_remaining_secs = h.and_then(|h| h.cooldown_remaining_secs());
                ProviderStatusInfo {
                    name: name.to_string(),
                    available,
                    tags,
                    circuit_state,
                    consecutive_failures,
                    cooldown_remaining_secs,
                }
            })
            .collect();
        statuses.sort_by(|a, b| a.name.cmp(&b.name));
        statuses
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
    use std::thread;
    use std::time::{Duration, Instant};

    use accelmars_gateway_core::{
        CostPreference, Latency, MockAdapter, ModelTier, Privacy, RoutingConstraints,
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
            fixture_file: None,
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

    // === Original routing tests ===

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

    // === Circuit breaker tests ===

    #[test]
    fn circuit_closed_allows_requests() {
        // Fresh provider is in CLOSED state — requests flow through.
        let mut h = ProviderHealth::new();
        assert_eq!(h.state, CircuitState::Closed);
        assert!(h.should_attempt());
        // Multiple calls in CLOSED all return true
        assert!(h.should_attempt());
    }

    #[test]
    fn circuit_opens_after_max_failures() {
        // 3 consecutive failures → OPEN, provider_is_healthy returns false.
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["deepseek"]);
        let router = Router::new(config, registry);

        router.on_failure("deepseek");
        router.on_failure("deepseek");
        router.on_failure("deepseek");

        assert!(!router.provider_is_healthy("deepseek"));

        let health = router.health.lock().unwrap();
        assert_eq!(health["deepseek"].state, CircuitState::Open);
    }

    #[test]
    fn circuit_half_open_after_cooldown() {
        // OPEN → HALF-OPEN when cooldown elapses. Probe request allowed.
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["deepseek"]);
        let router = Router::new(config, registry);

        // Open the circuit
        router.on_failure("deepseek");
        router.on_failure("deepseek");
        router.on_failure("deepseek");

        // Simulate cooldown expiry by setting last_state_change to the past
        {
            let mut health = router.health.lock().unwrap();
            let h = health.get_mut("deepseek").unwrap();
            h.set_last_state_change(Instant::now() - Duration::from_secs(31));
        }

        // First call: transitions OPEN → HALF-OPEN, returns true (probe allowed)
        assert!(router.provider_is_healthy("deepseek"));

        // Verify state is now HALF-OPEN
        {
            let health = router.health.lock().unwrap();
            assert_eq!(health["deepseek"].state, CircuitState::HalfOpen);
        }

        // Second call: probe slot claimed, returns false
        assert!(!router.provider_is_healthy("deepseek"));
    }

    #[test]
    fn half_open_success_closes_circuit() {
        // HALF-OPEN + on_success() → CLOSED with all counters reset and cooldown back to base.
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["deepseek"]);
        let router = Router::new(config, registry);

        // Drive to OPEN
        router.on_failure("deepseek");
        router.on_failure("deepseek");
        router.on_failure("deepseek");

        // Force to HALF-OPEN directly
        {
            let mut health = router.health.lock().unwrap();
            let h = health.get_mut("deepseek").unwrap();
            h.state = CircuitState::HalfOpen;
            h.last_state_change = Some(Instant::now());
        }

        // Probe succeeds → full reset
        router.on_success("deepseek");

        {
            let health = router.health.lock().unwrap();
            let h = &health["deepseek"];
            assert_eq!(h.state, CircuitState::Closed);
            assert_eq!(h.consecutive_failures, 0);
            assert_eq!(h.current_cooldown, h.base_cooldown);
        }

        // Provider is now healthy again
        assert!(router.provider_is_healthy("deepseek"));
    }

    #[test]
    fn half_open_failure_reopens_with_longer_cooldown() {
        // HALF-OPEN + on_failure() → OPEN with doubled cooldown (30 → 60s).
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["deepseek"]);
        let router = Router::new(config, registry);

        // Drive to OPEN
        router.on_failure("deepseek");
        router.on_failure("deepseek");
        router.on_failure("deepseek");

        // Force to HALF-OPEN
        {
            let mut health = router.health.lock().unwrap();
            let h = health.get_mut("deepseek").unwrap();
            h.state = CircuitState::HalfOpen;
            h.last_state_change = Some(Instant::now());
        }

        // Probe fails → back to OPEN with doubled cooldown
        router.on_failure("deepseek");

        {
            let health = router.health.lock().unwrap();
            let h = &health["deepseek"];
            assert_eq!(h.state, CircuitState::Open);
            assert_eq!(h.current_cooldown, Duration::from_secs(60));
        }
    }

    #[test]
    fn exponential_backoff_caps_at_max() {
        // Each HALF-OPEN probe failure doubles cooldown: 30 → 60 → 120 → 240 → 300 → 300 (cap).
        let mut h = ProviderHealth::new();
        assert_eq!(h.current_cooldown.as_secs(), 30); // initial

        let expected_after_each_probe_failure = [60u64, 120, 240, 300, 300];

        for &expected_secs in &expected_after_each_probe_failure {
            // Set to HALF-OPEN to trigger the probe-failure path in record_failure()
            h.state = CircuitState::HalfOpen;
            h.last_state_change = Some(Instant::now());
            h.record_failure();

            assert_eq!(
                h.current_cooldown.as_secs(),
                expected_secs,
                "expected cooldown {}s after probe failure, got {}s",
                expected_secs,
                h.current_cooldown.as_secs()
            );
        }
    }

    #[test]
    fn fallback_cost_premium_logged() {
        // When cheap provider (deepseek $0.28/M) falls back to expensive one (claude $3/M),
        // the router resolves to claude (behavioral verification).
        // Structured cost premium warning is emitted — verified by code review of
        // resolve_with_fallback(): "fallback cost premium" tracing::warn! call.
        let mut config = make_config(GatewayMode::Normal);
        // Set deepseek to fall back to claude (more expensive)
        config.providers.get_mut("deepseek").unwrap().fallback = Some("claude".to_string());

        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(MockAdapter::default()));
        // deepseek not registered (unavailable) → triggers fallback to claude
        registry.register(Arc::new(MockAdapter::default().with_name("claude")));

        let router = Router::new(config, registry);

        // Standard tier → deepseek (unavailable) → fallback to claude
        let decision = router
            .resolve(ModelTier::Standard, &RoutingConstraints::default())
            .unwrap();

        // Behavioral: fallback resolved to claude (the more expensive provider)
        assert_eq!(decision.provider_name, "claude");
        // Pricing confirms cost premium: deepseek ($0.28) → claude ($3.0)
        let (orig_in, _) = router.provider_pricing("deepseek");
        let (fb_in, _) = router.provider_pricing("claude");
        assert!(
            fb_in > orig_in,
            "claude should be more expensive than deepseek"
        );
    }

    #[test]
    fn fallback_skips_open_circuit_provider() {
        // When primary has OPEN circuit, router should resolve to fallback provider.
        let mut config = make_config(GatewayMode::Normal);
        config.providers.get_mut("deepseek").unwrap().fallback = Some("claude".to_string());

        let registry = make_registry_with_available(&["deepseek", "claude"]);
        let router = Router::new(config, registry);

        // Open deepseek's circuit (3 failures)
        router.on_failure("deepseek");
        router.on_failure("deepseek");
        router.on_failure("deepseek");
        assert!(!router.provider_is_healthy("deepseek"));

        // Standard tier → deepseek (OPEN) → falls back to claude
        let decision = router
            .resolve(ModelTier::Standard, &RoutingConstraints::default())
            .unwrap();
        assert_eq!(decision.provider_name, "claude");
    }

    #[test]
    fn full_circuit_lifecycle() {
        // Complete state machine: CLOSED → OPEN → HALF-OPEN → CLOSED.
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["deepseek"]);
        let router = Router::new(config, registry);

        // Phase 1: CLOSED — requests allowed
        assert!(router.provider_is_healthy("deepseek"));

        // Phase 2: 3 failures → OPEN
        router.on_failure("deepseek");
        router.on_failure("deepseek");
        router.on_failure("deepseek");
        assert!(!router.provider_is_healthy("deepseek"));
        {
            let health = router.health.lock().unwrap();
            assert_eq!(health["deepseek"].state, CircuitState::Open);
        }

        // Phase 3: Simulate cooldown expiry → HALF-OPEN on next check
        {
            let mut health = router.health.lock().unwrap();
            health
                .get_mut("deepseek")
                .unwrap()
                .set_last_state_change(Instant::now() - Duration::from_secs(31));
        }
        assert!(router.provider_is_healthy("deepseek")); // probe allowed
        {
            let health = router.health.lock().unwrap();
            assert_eq!(health["deepseek"].state, CircuitState::HalfOpen);
        }

        // Phase 4: Probe succeeds → CLOSED (full reset)
        router.on_success("deepseek");
        {
            let health = router.health.lock().unwrap();
            let h = &health["deepseek"];
            assert_eq!(h.state, CircuitState::Closed);
            assert_eq!(h.consecutive_failures, 0);
            assert_eq!(h.current_cooldown, h.base_cooldown);
        }

        // Back to healthy
        assert!(router.provider_is_healthy("deepseek"));
    }

    #[test]
    fn concurrent_health_checks_dont_deadlock() {
        // Concurrent on_failure() and provider_is_healthy() calls must not deadlock.
        // Guards against lock ordering bugs in the health Mutex.
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["deepseek", "claude"]);
        let router = Arc::new(Router::new(config, registry));

        let mut handles = Vec::new();

        for i in 0..10 {
            let r = router.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    if i % 2 == 0 {
                        r.on_failure("deepseek");
                        r.on_success("deepseek");
                    } else {
                        let _ = r.provider_is_healthy("deepseek");
                        let _ = r.provider_is_healthy("claude");
                    }
                }
            }));
        }

        for h in handles {
            h.join()
                .expect("thread panicked — possible deadlock or panic");
        }
        // Reaching here means no deadlock occurred within the thread lifetime
    }

    #[test]
    fn status_endpoint_shows_circuit_state() {
        // After failures, provider_statuses() reflects circuit state for observability.
        let config = make_config(GatewayMode::Normal);
        let registry = make_registry_with_available(&["deepseek", "gemini"]);
        let router = Router::new(config, registry);

        // Open deepseek's circuit
        router.on_failure("deepseek");
        router.on_failure("deepseek");
        router.on_failure("deepseek");

        let statuses = router.provider_statuses();

        let deepseek = statuses.iter().find(|s| s.name == "deepseek").unwrap();
        assert_eq!(deepseek.circuit_state, "open");
        assert_eq!(deepseek.consecutive_failures, 3);
        assert!(
            deepseek.cooldown_remaining_secs.is_some(),
            "OPEN provider should have a cooldown_remaining_secs"
        );

        let mock_status = statuses.iter().find(|s| s.name == "mock").unwrap();
        assert_eq!(mock_status.circuit_state, "closed");
        assert_eq!(mock_status.consecutive_failures, 0);
        assert!(mock_status.cooldown_remaining_secs.is_none());
    }
}
