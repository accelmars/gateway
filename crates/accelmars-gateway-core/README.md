# accelmars-gateway-core

Core traits and types for the AccelMars AI gateway.

Zero provider SDK dependencies — all external calls live in the binary crate or adapter crates.

## Custom Routing Strategies

Implement `RoutingStrategy` to replace the default tier-based provider selection:

```rust
use accelmars_gateway_core::routing::{ProviderInfo, RoutingError, RoutingStrategy};
use accelmars_gateway_core::GatewayRequest;

pub struct ConstantRouter {
    pub provider: String,
}

impl RoutingStrategy for ConstantRouter {
    fn select_provider(
        &self,
        _request: &GatewayRequest,
        available: &[ProviderInfo],
    ) -> Result<String, RoutingError> {
        available
            .iter()
            .find(|p| p.name == self.provider && p.is_available)
            .map(|p| p.name.clone())
            .ok_or_else(|| RoutingError::ConstraintUnsatisfied(
                format!("provider '{}' not available", self.provider),
            ))
    }
}
```

The open gateway ships `TierRouter` as the default. Contract-aware routing (`ContractAwareRouter`)
lives in `accelmars/platform` — the closed composition layer that extends this crate.

## Request Metadata

`RequestMetadata` provides a typed view over `GatewayRequest.metadata` for well-known fields:

```rust
use accelmars_gateway_core::metadata::from_request_metadata;

let meta = from_request_metadata(&request);
// meta.engine, meta.contract_id, meta.task_type, meta.budget_usd
```

Fields are populated from the request body (`metadata` JSON key) and/or the
`X-AccelMars-Context` HTTP header (merged by the server before routing).
