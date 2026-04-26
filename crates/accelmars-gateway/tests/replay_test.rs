//! Replay-based integration tests for the gateway server.
//!
//! Each test loads a cassette fixture via [`FixtureAdapter`] and registers it as the
//! `"mock"` adapter so that `GatewayMode::Mock` routes all requests through the fixture.
//! No live API keys are required — all responses are replayed from disk.
//!
//! # Fixture location
//!
//! Fixtures live at `gateway/tests/fixtures/`. The [`fixture_path`] helper resolves paths
//! relative to `CARGO_MANIFEST_DIR` so tests work on any machine and in CI.

use std::path::PathBuf;
use std::sync::Arc;

use accelmars_gateway::adapters::fixture::FixtureAdapter;
use accelmars_gateway::auth::AuthStore;
use accelmars_gateway::concurrency::ConcurrencyLimiter;
use accelmars_gateway::config::{GatewayConfig, GatewayMode};
use accelmars_gateway::cost::CostTracker;
use accelmars_gateway::registry::AdapterRegistry;
use accelmars_gateway::router::Router;
use accelmars_gateway::server::serve_with_listener;
use tokio::net::TcpListener;

/// Resolve a fixture file path relative to the workspace root.
///
/// `CARGO_MANIFEST_DIR` points at `crates/accelmars-gateway/`; two levels up is the
/// workspace root where `tests/fixtures/` lives.
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Load a [`FixtureAdapter`] from a cassette file, register it as `"mock"`, start the
/// gateway in `GatewayMode::Mock`, and return the base URL (`http://127.0.0.1:{port}`).
async fn start_replay_server(fixture_name: &str) -> String {
    let path = fixture_path(fixture_name);
    let adapter =
        FixtureAdapter::from_file("mock", &path).expect("fixture file must exist and be valid");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    let mut config = GatewayConfig::default();
    config.mode = GatewayMode::Mock;

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(adapter));

    let router = Arc::new(Router::new(config, registry));
    let limiter = Arc::new(ConcurrencyLimiter::new(20));
    let cost_tracker = Arc::new(CostTracker::open(std::path::Path::new(":memory:")).unwrap());
    let auth_store = Arc::new(AuthStore::in_memory().unwrap());

    tokio::spawn(async move {
        serve_with_listener(
            listener,
            router,
            limiter,
            cost_tracker,
            auth_store,
            true,
            port,
        )
        .await
        .ok();
    });
    tokio::task::yield_now().await;

    format!("http://{addr}")
}

// ---------------------------------------------------------------------------
// Test 1: Successful quick request replayed from cassette
// ---------------------------------------------------------------------------

#[tokio::test]
async fn replay_gemini_quick_returns_200_with_correct_content() {
    let base = start_replay_server("gemini-quick-hello.json").await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    assert_eq!(
        content, "Hello! How can I help you today?",
        "response content must match the fixture"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Rate-limit error fixture → 429 TOO_MANY_REQUESTS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn replay_rate_limit_returns_429() {
    let base = start_replay_server("error-rate-limit.json").await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "standard",
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .send()
        .await
        .unwrap();

    // RateLimit maps to HTTP 429 in adapter_error_to_response
    assert_eq!(resp.status(), 429, "rate-limit cassette must produce 429");

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"].as_str().is_some(),
        "error response must include a message"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Multi-turn cassette replays entries in sequential order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn replay_multi_turn_returns_sequential_responses() {
    let base = start_replay_server("multi-turn.json").await;
    let client = reqwest::Client::new();

    let expected = [
        ("Hello", "Hello! How can I assist you today?"),
        ("What is 2+2?", "4"),
        ("Thanks", "You're welcome!"),
    ];

    for (i, (_prompt, expected_content)) in expected.iter().enumerate() {
        let resp = client
            .post(format!("{base}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": "quick",
                "messages": [{"role": "user", "content": "Hello"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200, "turn {i} should return 200");

        let body: serde_json::Value = resp.json().await.unwrap();
        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("");
        assert_eq!(
            content, *expected_content,
            "turn {i} response must match fixture entry"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: Exhausted cassette → 502 BAD_GATEWAY (ProviderError)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn replay_exhausted_cassette_returns_502() {
    // gemini-quick-hello.json has exactly 1 entry
    let base = start_replay_server("gemini-quick-hello.json").await;
    let client = reqwest::Client::new();

    let post = || {
        client
            .post(format!("{base}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": "quick",
                "messages": [{"role": "user", "content": "Hello"}]
            }))
    };

    // First request: cassette has one entry → 200
    let first = post().send().await.unwrap();
    assert_eq!(
        first.status(),
        200,
        "first request must consume the cassette entry"
    );

    // Second request: cassette exhausted → ProviderError → 502
    let second = post().send().await.unwrap();
    assert_eq!(
        second.status(),
        502,
        "exhausted cassette must return 502 BAD_GATEWAY"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Fixture mode activated via config (not env var)
// ---------------------------------------------------------------------------
//
// Proves that GatewayMode::Fixture routes through FixtureAdapter:
// 1. Config has mode="fixture" — router does NOT call resolve_mock() (mode != Mock)
// 2. No providers configured — resolve_with_fallback() exhausts the tier lookup
// 3. Router falls to its mock-fallback path: registry.get("mock") → FixtureAdapter
// 4. Cassette response is served correctly

#[tokio::test]
async fn fixture_mode_serves_cassette_responses_via_config() {
    let fixture_path_buf = fixture_path("gemini-quick-hello.json");
    let fixture_path_str = fixture_path_buf.to_string_lossy().to_string();

    // Build config with mode=fixture and fixture_file set
    let toml = format!("mode = \"fixture\"\nfixture_file = \"{fixture_path_str}\"");
    let config = GatewayConfig::from_toml_str(&toml).unwrap();
    assert_eq!(config.mode, GatewayMode::Fixture);

    let adapter = FixtureAdapter::from_file("mock", &fixture_path_buf)
        .expect("fixture file must exist and be valid");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(adapter));

    let router = Arc::new(Router::new(config, registry));
    let limiter = Arc::new(ConcurrencyLimiter::new(20));
    let cost_tracker = Arc::new(CostTracker::open(std::path::Path::new(":memory:")).unwrap());
    let auth_store = Arc::new(AuthStore::in_memory().unwrap());

    tokio::spawn(async move {
        serve_with_listener(
            listener,
            router,
            limiter,
            cost_tracker,
            auth_store,
            true,
            port,
        )
        .await
        .ok();
    });
    tokio::task::yield_now().await;

    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    assert_eq!(
        content, "Hello! How can I help you today?",
        "fixture mode must serve cassette responses via config"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Cassette round-trip serialization (no server required)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fixture_round_trip_serialization() {
    use accelmars_gateway::adapters::fixture::{
        Cassette, CassetteEntry, CassetteResponse, CASSETTE_SCHEMA_VERSION,
    };
    use accelmars_gateway_core::{
        GatewayRequest, GatewayResponse, Message, ModelTier, RoutingConstraints,
    };

    let cassette = Cassette {
        schema_version: CASSETTE_SCHEMA_VERSION.to_string(),
        provider: "test-provider".to_string(),
        recorded_at: "2026-04-22T00:00:00Z".to_string(),
        entries: vec![CassetteEntry {
            request: GatewayRequest {
                tier: ModelTier::Quick,
                constraints: RoutingConstraints::default(),
                messages: vec![Message {
                    role: "user".to_string(),
                    content: "round-trip test".to_string(),
                }],
                max_tokens: None,
                stream: false,
                metadata: Default::default(),
            },
            response: CassetteResponse::Success(GatewayResponse {
                id: "rt-1".to_string(),
                model: "test-model".to_string(),
                content: "round-trip response".to_string(),
                tokens_in: 10,
                tokens_out: 5,
                finish_reason: "stop".to_string(),
            }),
            match_key: None,
        }],
    };

    let tmpdir = std::env::temp_dir();
    let path = tmpdir.join("replay_test_round_trip.json");

    cassette.to_file(&path).unwrap();
    let loaded = Cassette::from_file(&path).unwrap();
    std::fs::remove_file(&path).ok();

    assert_eq!(loaded.schema_version, CASSETTE_SCHEMA_VERSION);
    assert_eq!(loaded.provider, "test-provider");
    assert_eq!(loaded.entries.len(), 1);

    let result = loaded.entries[0].response.clone().to_adapter_result();
    assert_eq!(result.unwrap().content, "round-trip response");
}
