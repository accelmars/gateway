use crate::{GatewayRequest, ModelTier};

/// Snapshot of a provider's availability — passed to routing strategies.
///
/// `#[non_exhaustive]` allows additive fields in future versions (e.g., latency hint,
/// cost per token) without breaking strategy implementations.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub name: String,
    pub tier: ModelTier,
    pub is_available: bool,
}

impl ProviderInfo {
    pub fn new(name: String, tier: ModelTier, is_available: bool) -> Self {
        Self {
            name,
            tier,
            is_available,
        }
    }
}

/// Errors returned by a [`RoutingStrategy`].
#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    #[error("no provider available for tier {0}")]
    NoProviderAvailable(ModelTier),
    #[error("routing constraint unsatisfied: {0}")]
    ConstraintUnsatisfied(String),
}

/// Strategy for selecting a provider from a candidate list.
///
/// Implement this trait to add contract-aware, cost-aware, or other custom routing
/// logic. The open gateway ships `TierRouter` as the default implementation.
/// The closed platform layer will supply `ContractAwareRouter` without modifying
/// this crate.
///
/// # Send + Sync
/// Required because `Router` is shared across async tasks via `Arc`.
pub trait RoutingStrategy: Send + Sync {
    /// Select one provider from `available` for the given `request`.
    ///
    /// Returns the provider name to route to, or a `RoutingError` if no
    /// suitable provider can be found.
    fn select_provider(
        &self,
        request: &GatewayRequest,
        available: &[ProviderInfo],
    ) -> Result<String, RoutingError>;
}

/// Default routing strategy: selects the first available provider.
///
/// Replicates the inline tier-routing logic from `Router::resolve()` —
/// the candidate list passed by the router is already ordered with the
/// tier-default provider first, followed by constraint-filtered alternates.
#[derive(Default)]
pub struct TierRouter;

impl RoutingStrategy for TierRouter {
    fn select_provider(
        &self,
        request: &GatewayRequest,
        available: &[ProviderInfo],
    ) -> Result<String, RoutingError> {
        available
            .iter()
            .find(|p| p.is_available)
            .map(|p| p.name.clone())
            .ok_or(RoutingError::NoProviderAvailable(request.tier))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelTier, RoutingConstraints};
    use std::collections::HashMap;

    fn make_request(tier: ModelTier) -> GatewayRequest {
        GatewayRequest {
            tier,
            constraints: RoutingConstraints::default(),
            messages: vec![],
            max_tokens: None,
            stream: false,
            metadata: HashMap::new(),
        }
    }

    fn provider(name: &str, tier: ModelTier, available: bool) -> ProviderInfo {
        ProviderInfo::new(name.to_string(), tier, available)
    }

    #[test]
    fn quick_tier_selects_available_provider() {
        let router = TierRouter::default();
        let req = make_request(ModelTier::Quick);
        let infos = vec![provider("gemini", ModelTier::Quick, true)];
        assert_eq!(router.select_provider(&req, &infos).unwrap(), "gemini");
    }

    #[test]
    fn standard_tier_selects_available_provider() {
        let router = TierRouter::default();
        let req = make_request(ModelTier::Standard);
        let infos = vec![provider("deepseek", ModelTier::Standard, true)];
        assert_eq!(router.select_provider(&req, &infos).unwrap(), "deepseek");
    }

    #[test]
    fn max_tier_selects_available_provider() {
        let router = TierRouter::default();
        let req = make_request(ModelTier::Max);
        let infos = vec![provider("claude", ModelTier::Max, true)];
        assert_eq!(router.select_provider(&req, &infos).unwrap(), "claude");
    }

    #[test]
    fn unavailable_provider_skipped() {
        let router = TierRouter::default();
        let req = make_request(ModelTier::Quick);
        let infos = vec![
            provider("gemini", ModelTier::Quick, false),
            provider("deepseek", ModelTier::Quick, true),
        ];
        assert_eq!(router.select_provider(&req, &infos).unwrap(), "deepseek");
    }

    #[test]
    fn all_unavailable_returns_no_provider_available() {
        let router = TierRouter::default();
        let req = make_request(ModelTier::Quick);
        let infos = vec![provider("gemini", ModelTier::Quick, false)];
        let err = router.select_provider(&req, &infos).unwrap_err();
        assert!(
            matches!(err, RoutingError::NoProviderAvailable(ModelTier::Quick)),
            "expected NoProviderAvailable(Quick), got {err}"
        );
    }

    #[test]
    fn empty_list_returns_no_provider_available() {
        let router = TierRouter::default();
        let req = make_request(ModelTier::Standard);
        let err = router.select_provider(&req, &[]).unwrap_err();
        assert!(matches!(
            err,
            RoutingError::NoProviderAvailable(ModelTier::Standard)
        ));
    }
}
