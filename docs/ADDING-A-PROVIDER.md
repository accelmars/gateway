# Adding a Community Provider

This guide shows how to implement the `ProviderAdapter` trait to add a new AI
provider to AccelMars Gateway. The trait is public — no gateway internals are
required, and no upstream changes are needed for a downstream fork.

## When to use this guide

**Downstream crate (fork or local build):** You want to add a provider for your
own use, your team, or your company. Fork the gateway repo, add your provider
to `crates/accelmars-gateway/src/adapters/`, register it in `main.rs`, and build
your own binary. Your provider never needs to ship in the upstream repo.

**Upstream PR:** You want your provider in the official release. Follow the same
implementation steps, then open a PR. The review process checks: trait compliance,
test coverage, and accurate `cost_per_1m_input` data. See [Contributing upstream](#contributing-upstream).

---

## Prerequisites

- Rust stable ≥ 1.75 (`rustup update stable`)
- The gateway repo cloned locally
- Your provider's API documentation

---

## Step 1: Implement `ProviderAdapter`

`ProviderAdapter` is the only boundary between the gateway core and any provider.
It lives in `crates/accelmars-gateway-core/src/adapter.rs`:

```rust
pub trait ProviderAdapter: Send + Sync {
    /// Provider identifier — e.g. "gemini", "deepseek", "my-provider".
    /// Must match the key in gateway.toml and the registry match arm.
    fn name(&self) -> &str;

    /// Execute a completion request, returning a normalized response.
    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError>;

    /// Execute a streaming completion (optional — default wraps complete()).
    /// Override to return token-level chunks via your provider's SSE stream.
    fn complete_chunks(&self, request: &GatewayRequest) -> Result<ChunkedResponse, AdapterError> {
        // default: wraps complete() as a single chunk
    }

    /// Whether this provider is configured and reachable.
    /// Called on startup to populate `gateway status`.
    fn is_available(&self) -> bool;
}
```

Create your adapter struct in a new file, e.g.
`crates/accelmars-gateway/src/adapters/my_provider.rs`:

```rust
use accelmars_gateway_core::{AdapterError, GatewayRequest, GatewayResponse, ProviderAdapter};

pub struct MyProvider {
    api_key: Option<String>,
    model: String,
}

impl MyProvider {
    pub fn new(api_key: Option<String>, model: String) -> Self {
        Self { api_key, model }
    }
}

impl ProviderAdapter for MyProvider {
    fn name(&self) -> &str {
        "my-provider"
    }

    fn is_available(&self) -> bool {
        self.api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false)
    }

    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        // Implement in Steps 2 and 3 below.
        todo!()
    }
}
```

---

## Step 2: Implement `complete()`

`complete()` maps a `GatewayRequest` to a `GatewayResponse`. The key types:

```rust
pub struct GatewayRequest {
    pub tier: ModelTier,           // quick | standard | max | ultra
    pub constraints: RoutingConstraints,
    pub messages: Vec<Message>,    // [{ role, content }, ...]
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub metadata: HashMap<String, serde_json::Value>,
}

pub struct GatewayResponse {
    pub id: String,           // unique request ID from your provider
    pub model: String,        // actual model name returned by provider
    pub content: String,      // completion text
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub finish_reason: String, // "stop", "length", etc.
}
```

**Model ID rule:** Never hardcode model IDs in Rust. Read the model from your
`ProviderConfig::model` field, which comes from `gateway.toml`. This lets
operators upgrade models without a code change.

**Error handling:** Map provider error types to `AdapterError` variants:

```rust
pub enum AdapterError {
    RateLimit { retry_after: Option<Duration> },
    AuthError(String),
    Timeout,
    ProviderError(String),
    ParseError(String),
}
```

**Streaming:** If your provider supports SSE, override `complete_chunks()` to
return one `String` per SSE `data:` event. The default implementation wraps
`complete()` into a single chunk — safe for Phase 1, but users won't see
progressive output.

---

## Step 3: Declare name and availability

```rust
fn name(&self) -> &str {
    "my-provider"   // must match gateway.toml key and registry match arm
}

fn is_available(&self) -> bool {
    // Check that the API key env var is set and non-empty.
    // The gateway calls this on startup and for `gateway status`.
    self.api_key.as_deref().map(|k| !k.is_empty()).unwrap_or(false)
}
```

Also declare which tiers your provider covers as documentation alongside your
struct (not part of the trait):

```rust
/// Tiers supported by MyProvider.
pub const SUPPORTED_TIERS: &[ModelTier] = &[ModelTier::Standard, ModelTier::Max];
```

---

## Step 4: Register your provider

**4a. Add to the adapter module** — in `crates/accelmars-gateway/src/adapters/mod.rs`:

```rust
pub mod my_provider;
pub use my_provider::MyProvider;
```

**4b. Add a match arm** — in `crates/accelmars-gateway/src/main.rs`,
inside `build_registry_from_config()`:

```rust
"my-provider" => Arc::new(MyProvider::new(api_key, provider_cfg.model.clone())),
```

**4c. Add to `gateway.toml`** — wire a tier to your provider:

```toml
[tiers]
standard = "my-provider"

[providers.my-provider]
api_key_env = "MY_PROVIDER_API_KEY"
model = "my-provider-model-v1"
cost_per_1m_input = 0.50
cost_per_1m_output = 1.50
```

The `tiers` section maps quality tiers to provider names. The `providers` section
holds per-provider config (API key env var, model ID, cost data).

---

## Step 5: Test it

**Start the gateway with your provider:**

```sh
MY_PROVIDER_API_KEY=your-key gateway start
gateway status   # should show my-provider as available
```

**Send a request targeting your provider's tier:**

```sh
gateway complete "What is 2 + 2?" --tier standard
# or via HTTP:
curl -s http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer local" \
  -H "Content-Type: application/json" \
  -d '{"model":"standard","messages":[{"role":"user","content":"What is 2+2?"}]}'
```

**Record a cassette of your provider's responses** for deterministic CI:

```sh
GATEWAY_MODE=record GATEWAY_FIXTURE_FILE=my-provider.json gateway start
# Run your test suite — responses are recorded.
# On CI: GATEWAY_MODE=fixture GATEWAY_FIXTURE_FILE=my-provider.json gateway start
```

See [`docs/TESTING.md`](TESTING.md) and [`docs/CASSETTE-SPEC.md`](CASSETTE-SPEC.md)
for the full cassette workflow.

---

## Worked example

`examples/community-provider/` is a complete minimal implementation — `StubProvider`
implements `ProviderAdapter` with no network calls:

```sh
cd examples/community-provider
cargo build
cargo test
```

Use it as a template: copy `src/lib.rs`, rename the struct, replace the echo
logic with real HTTP calls to your provider.

---

## Contributing upstream

To ship your provider in the official gateway release:

1. Follow all steps above in your fork.
2. Add unit tests (mock the HTTP client — see `crates/accelmars-gateway/src/adapters/`
   for examples using `mockall`).
3. Fill in accurate `cost_per_1m_input` and `cost_per_1m_output` values.
4. Open a PR. The review checks:
   - Trait compliance: all required methods implemented correctly
   - `is_available()` requires a non-empty API key (not hardcoded)
   - Model IDs come from `ProviderConfig::model`, not Rust literals
   - Test coverage: at least one unit test per method
   - Cost data: documented source for the cost figures

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the full contribution process.
