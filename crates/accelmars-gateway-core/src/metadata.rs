use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::GatewayRequest;

/// Typed view over `GatewayRequest.metadata` — well-known AccelMars routing fields.
///
/// `#[non_exhaustive]` allows new fields in future versions without breaking callers.
/// `budget_usd` and `task_type` constraints are defined by the closed consumer (platform);
/// using `Option<String>` / `Option<f64>` keeps the struct open for v0.x.
#[non_exhaustive]
#[derive(Default, Clone, Debug, Deserialize, Serialize)]
pub struct RequestMetadata {
    /// Engine that originated this request (e.g., "cortex", "guild").
    pub engine: Option<String>,
    /// Contract ID being executed (e.g., "CI-003").
    pub contract_id: Option<String>,
    /// Task classification hint for contract-aware routing (e.g., "extraction").
    pub task_type: Option<String>,
    /// Budget cap in USD — interpreted by the closed routing consumer.
    pub budget_usd: Option<f64>,
    /// Any metadata keys not recognized as well-known fields.
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

/// Extract typed metadata fields from a request's freeform metadata map.
///
/// Reads well-known keys (`engine`, `contract_id`, `task_type`, `budget_usd`) out
/// of `request.metadata`. Unrecognized keys are collected into `extra`.
/// The original `request.metadata` HashMap is not modified.
pub fn from_request_metadata(request: &GatewayRequest) -> RequestMetadata {
    const KNOWN: &[&str] = &["engine", "contract_id", "task_type", "budget_usd"];
    let m = &request.metadata;
    RequestMetadata {
        engine: m.get("engine").and_then(|v| v.as_str()).map(String::from),
        contract_id: m
            .get("contract_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        task_type: m
            .get("task_type")
            .and_then(|v| v.as_str())
            .map(String::from),
        budget_usd: m.get("budget_usd").and_then(|v| v.as_f64()),
        extra: m
            .iter()
            .filter(|(k, _)| !KNOWN.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelTier, RoutingConstraints};

    fn request_with_metadata(meta: HashMap<String, Value>) -> GatewayRequest {
        GatewayRequest {
            tier: ModelTier::Quick,
            constraints: RoutingConstraints::default(),
            messages: vec![],
            max_tokens: None,
            stream: false,
            metadata: meta,
        }
    }

    #[test]
    fn from_request_metadata_round_trips_known_fields() {
        let mut meta = HashMap::new();
        meta.insert("engine".to_string(), Value::String("cortex".to_string()));
        meta.insert(
            "contract_id".to_string(),
            Value::String("CI-003".to_string()),
        );
        meta.insert(
            "task_type".to_string(),
            Value::String("extraction".to_string()),
        );
        meta.insert(
            "budget_usd".to_string(),
            Value::Number(serde_json::Number::from_f64(1.5).unwrap()),
        );

        let req = request_with_metadata(meta);
        let result = from_request_metadata(&req);

        assert_eq!(result.engine.as_deref(), Some("cortex"));
        assert_eq!(result.contract_id.as_deref(), Some("CI-003"));
        assert_eq!(result.task_type.as_deref(), Some("extraction"));
        assert_eq!(result.budget_usd, Some(1.5));
        assert!(result.extra.is_empty());
    }

    #[test]
    fn from_request_metadata_extra_field_passthrough() {
        let mut meta = HashMap::new();
        meta.insert("engine".to_string(), Value::String("cortex".to_string()));
        meta.insert(
            "custom_key".to_string(),
            Value::String("custom_val".to_string()),
        );

        let req = request_with_metadata(meta);
        let result = from_request_metadata(&req);

        assert_eq!(result.engine.as_deref(), Some("cortex"));
        assert_eq!(
            result.extra.get("custom_key").and_then(|v| v.as_str()),
            Some("custom_val")
        );
    }

    #[test]
    fn from_request_metadata_empty_yields_all_none() {
        let req = request_with_metadata(HashMap::new());
        let result = from_request_metadata(&req);

        assert!(result.engine.is_none());
        assert!(result.contract_id.is_none());
        assert!(result.task_type.is_none());
        assert!(result.budget_usd.is_none());
        assert!(result.extra.is_empty());
    }
}
