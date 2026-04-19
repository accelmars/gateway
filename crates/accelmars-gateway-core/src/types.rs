use std::collections::HashMap;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Quality tier — WHAT level of intelligence you need.
/// Maps to actual model IDs at the gateway config layer. Never hardcode model IDs here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    Quick,
    Standard,
    Max,
    Ultra,
}

impl FromStr for ModelTier {
    type Err = ParseModelTierError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "quick" => Ok(Self::Quick),
            "standard" => Ok(Self::Standard),
            "max" => Ok(Self::Max),
            "ultra" => Ok(Self::Ultra),
            other => Err(ParseModelTierError(other.to_string())),
        }
    }
}

impl std::fmt::Display for ModelTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Quick => write!(f, "quick"),
            Self::Standard => write!(f, "standard"),
            Self::Max => write!(f, "max"),
            Self::Ultra => write!(f, "ultra"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown model tier: '{0}' — expected quick, standard, max, or ultra")]
pub struct ParseModelTierError(String);

/// Data residency / provider routing constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Privacy {
    /// Any provider (default).
    #[default]
    Open,
    /// Exclude providers with data residency concerns (e.g., DeepSeek).
    Sensitive,
    /// Self-hosted only — data never leaves your infrastructure.
    Private,
}

impl std::fmt::Display for Privacy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Sensitive => write!(f, "sensitive"),
            Self::Private => write!(f, "private"),
        }
    }
}

/// Inference speed constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Latency {
    /// Optimize for quality/cost (default).
    #[default]
    Normal,
    /// Prefer fast inference providers (<1s, e.g., Groq, Cerebras).
    Low,
}

impl std::fmt::Display for Latency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "normal"),
            Self::Low => write!(f, "low"),
        }
    }
}

/// Cost preference — orthogonal to quality tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CostPreference {
    /// Only free-tier providers.
    Free,
    /// Cheapest option at requested quality.
    Budget,
    /// Balanced quality/cost (default).
    #[default]
    Default,
    /// Best available, ignore cost.
    Unlimited,
}

impl std::fmt::Display for CostPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Free => write!(f, "free"),
            Self::Budget => write!(f, "budget"),
            Self::Default => write!(f, "default"),
            Self::Unlimited => write!(f, "unlimited"),
        }
    }
}

/// Model capability requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Reasoning,
    ToolUse,
    Vision,
    Code,
    LongContext,
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reasoning => write!(f, "reasoning"),
            Self::ToolUse => write!(f, "tool_use"),
            Self::Vision => write!(f, "vision"),
            Self::Code => write!(f, "code"),
            Self::LongContext => write!(f, "long_context"),
        }
    }
}

/// Routing constraints — orthogonal to quality tier.
/// Default: open, normal latency, default cost, no capabilities, no provider override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoutingConstraints {
    pub privacy: Privacy,
    pub latency: Latency,
    pub cost: CostPreference,
    pub capabilities: Vec<Capability>,
    /// Explicit provider override — bypasses all routing logic.
    pub provider: Option<String>,
}

/// A single chat message (OpenAI-compatible format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Outbound request to the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayRequest {
    pub tier: ModelTier,
    pub constraints: RoutingConstraints,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Normalized response from any provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayResponse {
    pub id: String,
    pub model: String,
    pub content: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub finish_reason: String,
}
