use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::canary::CandidateConfig;

/// Top-level gateway configuration.
///
/// Loaded from (in order, later sources override earlier):
///   1. Compiled-in defaults
///   2. `gateway.toml` in CWD (or path from `--config`)
///   3. Environment variables: `GATEWAY__PORT=9090`, `GATEWAY__TIERS__STANDARD=claude`
///   4. `GATEWAY_MODE=mock` — special env var that forces mock mode
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GatewayConfig {
    pub port: u16,
    pub log_level: String,
    pub mode: GatewayMode,
    /// Path to a cassette file for fixture mode. Set via GATEWAY_FIXTURE_FILE or config.
    pub fixture_file: Option<String>,
    pub concurrency: ConcurrencyConfig,
    pub tiers: TierConfig,
    pub providers: HashMap<String, ProviderConfig>,
    pub constraints: ConstraintRules,
    /// Canary routing configuration — optional per-tier.
    /// Key: tier name ("quick", "standard", "max", "ultra").
    /// Absent key = no canary for that tier.
    #[serde(default)]
    pub canary: HashMap<String, CandidateConfig>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            log_level: "info".to_string(),
            mode: GatewayMode::Normal,
            fixture_file: None,
            concurrency: ConcurrencyConfig::default(),
            tiers: TierConfig::default(),
            providers: HashMap::new(),
            constraints: ConstraintRules::default(),
            canary: HashMap::new(),
        }
    }
}

impl GatewayConfig {
    /// Load configuration from file + environment variables.
    ///
    /// - `config_path`: optional explicit path; if None, looks for `gateway.toml` in CWD.
    /// - Env vars override file values: `GATEWAY__PORT=9090`
    /// - `GATEWAY_MODE=mock` env var forces mock mode (legacy compat)
    pub fn load(config_path: Option<&Path>) -> anyhow::Result<Self> {
        use config::{Config, Environment, File, FileFormat};

        let mut builder = Config::builder();

        // Layer 1: defaults
        builder = builder.set_default("port", 8080_i64)?;
        builder = builder.set_default("log_level", "info")?;
        builder = builder.set_default("mode", "normal")?;
        builder = builder.set_default("concurrency.max", 20_i64)?;
        builder = builder.set_default("tiers.quick", "gemini-flash-lite")?;
        builder = builder.set_default("tiers.standard", "deepseek")?;
        builder = builder.set_default("tiers.max", "claude")?;
        builder = builder.set_default("tiers.ultra", "claude-opus")?;

        // Layer 2: config file (optional — gateway starts without one)
        let file_path = config_path
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "gateway.toml".to_string());

        builder = builder.add_source(File::new(&file_path, FileFormat::Toml).required(false));

        // Layer 3: environment variables (GATEWAY__PORT, GATEWAY__TIERS__STANDARD, etc.)
        builder = builder.add_source(
            Environment::with_prefix("GATEWAY")
                .separator("__")
                .try_parsing(true),
        );

        let mut cfg: Self = builder.build()?.try_deserialize()?;

        // Layer 4: GATEWAY_MODE=mock legacy override
        if std::env::var("GATEWAY_MODE").as_deref() == Ok("mock") {
            cfg.mode = GatewayMode::Mock;
        }

        // Layer 4b: GATEWAY_MODE=fixture + GATEWAY_FIXTURE_FILE
        if std::env::var("GATEWAY_MODE").as_deref() == Ok("fixture") {
            cfg.mode = GatewayMode::Fixture;
        }
        if let Ok(path) = std::env::var("GATEWAY_FIXTURE_FILE") {
            cfg.fixture_file = Some(path);
        }

        Ok(cfg)
    }

    /// Load from a TOML string (for tests).
    pub fn from_toml_str(s: &str) -> anyhow::Result<Self> {
        use config::{Config, File, FileFormat};

        let cfg: Self = Config::builder()
            .add_source(File::from_str(s, FileFormat::Toml))
            .build()?
            .try_deserialize()?;
        Ok(cfg)
    }

    /// Validate that tier mappings and providers are consistent.
    ///
    /// Returns a list of warnings (providers missing API keys).
    /// Returns Err only if no providers are available at all.
    pub fn validate(&self) -> anyhow::Result<Vec<String>> {
        let mut warnings = Vec::new();

        // In mock mode: no provider validation needed
        if self.mode == GatewayMode::Mock {
            return Ok(warnings);
        }

        // In fixture mode: validate fixture_file is set and exists; skip provider validation
        if self.mode == GatewayMode::Fixture {
            match &self.fixture_file {
                None => {
                    anyhow::bail!("GATEWAY_MODE=fixture requires GATEWAY_FIXTURE_FILE to be set")
                }
                Some(path) => {
                    if !std::path::Path::new(path).exists() {
                        anyhow::bail!(
                            "fixture file not found: '{path}' — check GATEWAY_FIXTURE_FILE"
                        );
                    }
                }
            }
            return Ok(warnings);
        }

        // Check tier → provider mappings
        for (tier_name, provider_name) in [
            ("quick", &self.tiers.quick),
            ("standard", &self.tiers.standard),
            ("max", &self.tiers.max),
            ("ultra", &self.tiers.ultra),
        ] {
            if !self.providers.contains_key(provider_name.as_str()) {
                warnings.push(format!(
                    "tier '{tier_name}' maps to provider '{provider_name}' which is not configured"
                ));
            }
        }

        // think tier is optional; warn if configured but provider is absent
        if let Some(ref think_provider) = self.tiers.think {
            if !self.providers.contains_key(think_provider.as_str()) {
                warnings.push(format!(
                    "tier 'think' maps to provider '{think_provider}' which is not configured"
                ));
            }
        }

        // Check API key availability
        let mut available_count = 0;
        for (name, provider) in &self.providers {
            let key = std::env::var(&provider.api_key_env).unwrap_or_default();
            if key.is_empty() {
                warnings.push(format!(
                    "provider '{}': env var '{}' is not set — provider will be unavailable",
                    name, provider.api_key_env
                ));
            } else {
                available_count += 1;
            }
        }

        if available_count == 0 && !self.providers.is_empty() {
            anyhow::bail!("no providers have API keys configured — set at least one provider's API key or use GATEWAY_MODE=mock");
        }

        Ok(warnings)
    }
}

/// Gateway operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GatewayMode {
    /// Normal operation — use configured providers.
    #[default]
    Normal,
    /// Mock mode — all requests routed to MockAdapter. GATEWAY_MODE=mock.
    Mock,
    /// Fixture mode — route all requests through FixtureAdapter loaded from GATEWAY_FIXTURE_FILE.
    Fixture,
}

/// Concurrency limits (used by PF-005).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ConcurrencyConfig {
    /// Maximum concurrent requests across all providers.
    pub max: u32,
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self { max: 20 }
    }
}

/// Maps quality tiers to default provider names.
///
/// Values are provider names as keys in `GatewayConfig::providers`.
/// Not model IDs — actual model IDs live in `ProviderConfig::model`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TierConfig {
    pub quick: String,
    pub standard: String,
    pub max: String,
    pub ultra: String,
    /// Fifth tier: reasoning + context for Forge, Anchor-engine, Guild profiles.
    /// Schema-only — routing is not yet wired. Use `#[serde(default)]` for backward compat.
    #[serde(default)]
    pub think: Option<String>,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            quick: "gemini-flash-lite".to_string(),
            standard: "deepseek".to_string(),
            max: "claude".to_string(),
            ultra: "claude-opus".to_string(),
            think: None,
        }
    }
}

impl TierConfig {
    pub fn provider_for_tier(&self, tier: accelmars_gateway_core::ModelTier) -> &str {
        use accelmars_gateway_core::ModelTier;
        match tier {
            ModelTier::Quick => &self.quick,
            ModelTier::Standard => &self.standard,
            ModelTier::Max => &self.max,
            ModelTier::Ultra => &self.ultra,
        }
    }
}

/// Configuration for a single provider.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Name of the env var holding the API key, e.g. `"GEMINI_API_KEY"`.
    pub api_key_env: String,
    /// Actual model ID to send to this provider, e.g. `"gemini-2.5-flash-lite"`.
    /// This is the ONLY place model IDs appear — never in Rust logic.
    pub model: String,
    /// Maximum tokens for responses.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
    /// Tags for constraint filtering: e.g. `["free", "fast", "sensitive_ok"]`.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Fallback provider name when this provider fails.
    #[serde(default)]
    pub fallback: Option<String>,
    /// Cost per 1M input tokens in USD (for cost tracking in PF-005).
    #[serde(default)]
    pub cost_per_1m_input: f64,
    /// Cost per 1M output tokens in USD.
    #[serde(default)]
    pub cost_per_1m_output: f64,
}

fn default_timeout() -> u32 {
    120
}

/// Constraint-based provider filtering rules.
///
/// All rules are config-driven — the router reads tags, not hardcoded provider names.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ConstraintRules {
    /// Providers excluded when `privacy=sensitive`.
    pub sensitive_excluded: Vec<String>,
    /// Only these providers allowed when `privacy=private`.
    pub private_only: Vec<String>,
    /// Providers preferred when `latency=low`.
    pub low_latency_preferred: Vec<String>,
    /// Providers available when `cost=free` (supplement tag-based filtering).
    pub free_only: Vec<String>,
}

impl Default for ConstraintRules {
    fn default() -> Self {
        Self {
            // Chinese-hosted providers excluded by default for sensitive data routing.
            sensitive_excluded: vec![
                "qwen".to_string(),
                "stepfun".to_string(),
                "zhipu".to_string(),
                "minimax".to_string(),
                "moonshot".to_string(),
            ],
            private_only: Vec::new(),
            low_latency_preferred: Vec::new(),
            free_only: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_loads_from_toml_string() {
        let toml = r#"
port = 9090
log_level = "debug"

[tiers]
quick = "gemini"
standard = "deepseek"
max = "claude"
ultra = "claude-opus"

[providers.gemini]
api_key_env = "GEMINI_API_KEY"
model = "gemini-2.5-flash-lite"
timeout_seconds = 60
tags = ["free"]
cost_per_1m_input = 0.0
cost_per_1m_output = 0.0

[providers.deepseek]
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-chat"
timeout_seconds = 120
cost_per_1m_input = 0.28
cost_per_1m_output = 0.42
fallback = "gemini"

[constraints]
sensitive_excluded = ["deepseek"]
free_only = ["gemini"]
"#;
        let cfg = GatewayConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.port, 9090);
        assert_eq!(cfg.log_level, "debug");
        assert_eq!(cfg.tiers.standard, "deepseek");
        assert_eq!(cfg.providers["gemini"].model, "gemini-2.5-flash-lite");
        assert_eq!(
            cfg.providers["deepseek"].fallback.as_deref(),
            Some("gemini")
        );
        assert_eq!(cfg.constraints.sensitive_excluded, vec!["deepseek"]);
    }

    #[test]
    fn config_defaults_when_empty() {
        let cfg = GatewayConfig::from_toml_str("").unwrap();
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.mode, GatewayMode::Normal);
        assert_eq!(cfg.tiers.quick, "gemini-flash-lite");
        assert_eq!(cfg.concurrency.max, 20);
    }

    #[test]
    fn gateway_mode_mock_deserializes() {
        let toml = r#"mode = "mock""#;
        let cfg = GatewayConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.mode, GatewayMode::Mock);
    }

    #[test]
    fn gateway_mode_fixture_deserializes() {
        let toml = r#"mode = "fixture""#;
        let cfg = GatewayConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.mode, GatewayMode::Fixture);
    }

    #[test]
    fn validate_fixture_mode_requires_fixture_file() {
        let toml = r#"mode = "fixture""#;
        let cfg = GatewayConfig::from_toml_str(toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("GATEWAY_FIXTURE_FILE"),
            "error must mention GATEWAY_FIXTURE_FILE, got: {err}"
        );
    }

    #[test]
    fn validate_fixture_mode_rejects_missing_file() {
        let toml = "mode = \"fixture\"\nfixture_file = \"/nonexistent/path/cassette.json\"";
        let cfg = GatewayConfig::from_toml_str(toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("fixture file not found"),
            "error must mention 'fixture file not found', got: {err}"
        );
    }

    #[test]
    fn think_tier_is_none_by_default() {
        let cfg = GatewayConfig::from_toml_str("").unwrap();
        assert!(cfg.tiers.think.is_none());
    }

    #[test]
    fn think_tier_deserializes_from_toml() {
        let toml = "[tiers]\nthink = \"qwen\"";
        let cfg = GatewayConfig::from_toml_str(toml).unwrap();
        assert_eq!(cfg.tiers.think.as_deref(), Some("qwen"));
    }

    #[test]
    fn validate_passes_with_think_none() {
        let toml = "mode = \"mock\"";
        let cfg = GatewayConfig::from_toml_str(toml).unwrap();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_warns_when_think_provider_not_configured() {
        // Use PATH as api_key_env — always set and non-empty, so available_count > 0.
        // Think tier maps to "qwen" which is not in providers → must appear in warnings.
        let toml_full = r#"
[tiers]
quick = "gemini"
standard = "gemini"
max = "gemini"
ultra = "gemini"
think = "qwen"

[providers.gemini]
api_key_env = "PATH"
model = "gemini-flash"
"#;
        let cfg = GatewayConfig::from_toml_str(toml_full).unwrap();
        let warnings = cfg.validate().unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("think")),
            "expected a warning about 'think' tier, got: {warnings:?}"
        );
    }

    #[test]
    fn sensitive_excluded_defaults_include_new_chinese_providers() {
        let cfg = GatewayConfig::from_toml_str("").unwrap();
        let excluded = &cfg.constraints.sensitive_excluded;
        assert!(
            excluded.contains(&"qwen".to_string()),
            "qwen must be in sensitive_excluded default"
        );
        assert!(
            excluded.contains(&"stepfun".to_string()),
            "stepfun must be in sensitive_excluded default"
        );
        assert!(
            excluded.contains(&"zhipu".to_string()),
            "zhipu must be in sensitive_excluded default"
        );
        assert!(
            excluded.contains(&"minimax".to_string()),
            "minimax must be in sensitive_excluded default"
        );
        assert!(
            excluded.contains(&"moonshot".to_string()),
            "moonshot must be in sensitive_excluded default"
        );
    }

    #[test]
    fn tier_config_provider_for_tier() {
        use accelmars_gateway_core::ModelTier;
        let tc = TierConfig {
            quick: "gemini".to_string(),
            standard: "deepseek".to_string(),
            max: "claude".to_string(),
            ultra: "claude-opus".to_string(),
            think: None,
        };
        assert_eq!(tc.provider_for_tier(ModelTier::Quick), "gemini");
        assert_eq!(tc.provider_for_tier(ModelTier::Standard), "deepseek");
        assert_eq!(tc.provider_for_tier(ModelTier::Max), "claude");
        assert_eq!(tc.provider_for_tier(ModelTier::Ultra), "claude-opus");
    }
}
