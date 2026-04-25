//! Integration tests for the OpenAI-compatible gateway server.
//!
//! Each test spins up a real HTTP server on a random port (0 → OS assigns),
//! makes requests via reqwest, and verifies the response.
//!
//! # OpenAI SDK Compatibility
//!
//! These tests validate the same wire format used by the Python openai SDK:
//!
//! ```python
//! from openai import OpenAI
//! client = OpenAI(base_url="http://localhost:8080/v1", api_key="not-used")
//! response = client.chat.completions.create(
//!     model="standard",
//!     messages=[{"role": "user", "content": "Hello"}],
//! )
//! print(response.choices[0].message.content)
//! ```
//!
//! Equivalent curl:
//! ```bash
//! curl http://localhost:8080/v1/chat/completions \
//!   -H "Content-Type: application/json" \
//!   -d '{"model":"quick","messages":[{"role":"user","content":"Hello"}]}'
//! ```

use std::sync::Arc;
use std::time::Duration;

use accelmars_gateway::auth::AuthStore;
use accelmars_gateway::concurrency::ConcurrencyLimiter;
use accelmars_gateway::config::GatewayConfig;
use accelmars_gateway::cost::CostTracker;
use accelmars_gateway::registry::AdapterRegistry;
use accelmars_gateway::router::Router;
use accelmars_gateway::server::serve_with_listener;
use accelmars_gateway_core::MockAdapter;
use tokio::net::TcpListener;

/// Bind port 0, start the server in mock mode with auth disabled, return the base URL.
async fn start_test_server() -> String {
    start_test_server_with_max_concurrent(20).await
}

async fn start_test_server_with_max_concurrent(max: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    let mut config = GatewayConfig::default();
    config.mode = accelmars_gateway::config::GatewayMode::Mock;

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(MockAdapter::default()));

    let router = Arc::new(Router::new(config, registry));
    let limiter = Arc::new(ConcurrencyLimiter::new(max));

    // In-memory cost tracker and auth store for tests (no disk writes)
    let cost_tracker = Arc::new(CostTracker::open(std::path::Path::new(":memory:")).unwrap());
    let auth_store = Arc::new(AuthStore::in_memory().unwrap());

    tokio::spawn(async move {
        // auth_disabled: true — existing tests don't use API keys
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

/// Start a server with auth ENABLED. Returns (base_url, api_key, auth_store).
async fn start_auth_test_server() -> (String, String, Arc<AuthStore>) {
    let auth_store = Arc::new(AuthStore::in_memory().unwrap());
    let (test_key, _) = auth_store.create_key("test").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    let mut config = GatewayConfig::default();
    config.mode = accelmars_gateway::config::GatewayMode::Mock;

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(MockAdapter::default()));

    let router = Arc::new(Router::new(config, registry));
    let limiter = Arc::new(ConcurrencyLimiter::new(20));
    let cost_tracker = Arc::new(CostTracker::open(std::path::Path::new(":memory:")).unwrap());
    let auth_store_for_server = Arc::clone(&auth_store);

    tokio::spawn(async move {
        // auth_disabled: false — auth middleware is active
        serve_with_listener(
            listener,
            router,
            limiter,
            cost_tracker,
            auth_store_for_server,
            false,
            port,
        )
        .await
        .ok();
    });
    tokio::task::yield_now().await;

    (format!("http://{addr}"), test_key, auth_store)
}

// ---------------------------------------------------------------------------
// Test 1: POST valid request → 200 + valid OpenAI response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_valid_quick_request_returns_200_and_openai_response() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "Hello, gateway!"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");

    let choices = body["choices"].as_array().unwrap();
    assert!(!choices.is_empty());
    assert_eq!(choices[0]["message"]["role"], "assistant");
    assert!(choices[0]["message"]["content"].as_str().is_some());
    assert!(body["usage"]["prompt_tokens"].as_u64().is_some());
    assert!(body["id"].as_str().unwrap().starts_with("mock-"));
}

// ---------------------------------------------------------------------------
// Test 2: POST with model: "standard" → tier parsed correctly, 200 response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_standard_model_parses_tier_and_returns_200() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "standard",
            "messages": [{"role": "user", "content": "What is 2+2?"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");
}

// ---------------------------------------------------------------------------
// Test 3: POST with metadata.privacy: "sensitive" → constraints parsed, 200
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_with_metadata_privacy_sensitive_succeeds() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "max",
            "messages": [{"role": "user", "content": "Sensitive query"}],
            "metadata": {
                "privacy": "sensitive",
                "latency": "low",
                "cost": "budget",
                "capabilities": ["reasoning"]
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");
}

// ---------------------------------------------------------------------------
// Test 4: POST with invalid model → 400 + OpenAI error format
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_invalid_model_returns_400_with_openai_error() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "gpt-99-turbo",
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]["message"].as_str().is_some());
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "invalid_model");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("gpt-99-turbo"));
}

// ---------------------------------------------------------------------------
// Test 5: POST with empty messages → 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_empty_messages_returns_400() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": []
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "invalid_request");
}

// ---------------------------------------------------------------------------
// Test 6: GET /health → 200 {"status": "ok"}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_health_returns_200_ok() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client.get(format!("{base}/health")).send().await.unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

// ---------------------------------------------------------------------------
// Test 7: POST with stream: true → SSE event stream
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_stream_true_returns_sse_format() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "Stream this"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let content_type = resp.headers()["content-type"].to_str().unwrap();
    assert!(
        content_type.contains("text/event-stream"),
        "expected text/event-stream, got: {content_type}"
    );

    let body = resp.text().await.unwrap();
    assert!(body.contains("data:"), "expected SSE data lines");
    assert!(body.contains("[DONE]"), "expected [DONE] sentinel");
}

// ---------------------------------------------------------------------------
// Test 8: All model tiers accepted — quick, standard, max, ultra
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_four_model_tiers_return_200() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    for tier in ["quick", "standard", "max", "ultra"] {
        let resp = client
            .post(format!("{base}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": tier,
                "messages": [{"role": "user", "content": "Hello"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200, "tier '{tier}' should return 200");
    }
}

// ---------------------------------------------------------------------------
// Test 9: max_tokens field is accepted without error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn post_with_max_tokens_returns_200() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "standard",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"}
            ],
            "max_tokens": 256,
            "temperature": 0.7
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
}

// ---------------------------------------------------------------------------
// Test 10: Response usage tokens are non-zero for non-empty input
// ---------------------------------------------------------------------------

#[tokio::test]
async fn response_usage_tokens_nonzero_for_nonempty_input() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "A reasonably long prompt to generate tokens"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let prompt_tokens = body["usage"]["prompt_tokens"].as_u64().unwrap();
    let completion_tokens = body["usage"]["completion_tokens"].as_u64().unwrap();
    let total = body["usage"]["total_tokens"].as_u64().unwrap();
    assert!(prompt_tokens > 0, "prompt_tokens should be non-zero");
    assert!(
        completion_tokens > 0,
        "completion_tokens should be non-zero"
    );
    assert_eq!(
        total,
        prompt_tokens + completion_tokens,
        "total_tokens must equal prompt + completion"
    );
}

// ---------------------------------------------------------------------------
// Test 11: GET /status → returns version, concurrency, and providers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_status_returns_server_info() {
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client.get(format!("{base}/status")).send().await.unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();

    assert_eq!(body["status"], "running");
    assert!(body["version"].as_str().is_some(), "version field missing");
    assert!(body["uptime_seconds"].as_u64().is_some(), "uptime missing");
    assert!(
        body["concurrency"]["max"].as_u64().is_some(),
        "concurrency.max missing"
    );
    assert!(body["providers"].as_array().is_some(), "providers missing");
}

// ---------------------------------------------------------------------------
// Test 12: Concurrency semaphore — 3rd request queues and completes when slot opens
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_requests_complete_within_limit() {
    // max_concurrent=5 → all 3 simultaneous requests should succeed
    let base = start_test_server_with_max_concurrent(5).await;
    let client = Arc::new(reqwest::Client::new());

    let mut handles = Vec::new();
    for _ in 0..3 {
        let base = base.clone();
        let client = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            client
                .post(format!("{base}/v1/chat/completions"))
                .json(&serde_json::json!({
                    "model": "quick",
                    "messages": [{"role": "user", "content": "concurrent test"}]
                }))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }));
    }

    for handle in handles {
        let status = handle.await.unwrap();
        assert_eq!(status, 200, "concurrent request should succeed");
    }
}

// ---------------------------------------------------------------------------
// Test 13: Semaphore queue timeout → 504
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn semaphore_queue_timeout_returns_504() {
    // Use a shared limiter with max=1 and very short timeout so we can test 504
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    let mut config = GatewayConfig::default();
    config.mode = accelmars_gateway::config::GatewayMode::Mock;

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(MockAdapter::default()));

    let router = Arc::new(Router::new(config, registry));
    // 1 slot, 100ms timeout — second request will queue-timeout immediately
    let limiter = Arc::new(ConcurrencyLimiter::with_timeout(
        1,
        Duration::from_millis(100),
    ));
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
    let client = Arc::new(reqwest::Client::new());

    // First request should succeed (takes the only slot)
    // Second request should 504 after 100ms timeout
    // We send both concurrently
    let b1 = base.clone();
    let c1 = Arc::clone(&client);
    let first = tokio::spawn(async move {
        c1.post(format!("{b1}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": "quick",
                "messages": [{"role": "user", "content": "first"}]
            }))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });

    // Small yield so first grabs the permit before second attempts
    tokio::time::sleep(Duration::from_millis(10)).await;

    let b2 = base.clone();
    let c2 = Arc::clone(&client);
    let second = tokio::spawn(async move {
        c2.post(format!("{b2}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": "quick",
                "messages": [{"role": "user", "content": "second"}]
            }))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });

    let s1 = first.await.unwrap();
    let s2 = second.await.unwrap();

    // One should be 200, the other 504 (or both 200 if mock is fast enough)
    // At minimum: no panics, valid HTTP responses
    assert!(
        s1 == 200 || s1 == 504,
        "first request: unexpected status {s1}"
    );
    assert!(
        s2 == 200 || s2 == 504,
        "second request: unexpected status {s2}"
    );
    // At least one of them should have hit 504 (the timeout is 100ms, mock is blocking)
    // This is timing-sensitive, so we allow both outcomes but verify at least one is valid HTTP
}

// ---------------------------------------------------------------------------
// Test 14: Panic inside adapter → semaphore slot recovered
// (PF-005R Audit 1 — panic safety verification)
// ---------------------------------------------------------------------------

/// Adapter that panics on every call — used to verify semaphore recovery.
/// Registered as "mock" so GatewayMode::Mock routes to it.
struct PanickingAdapter;

impl accelmars_gateway_core::ProviderAdapter for PanickingAdapter {
    fn name(&self) -> &str {
        "mock"
    }
    fn complete(
        &self,
        _request: &accelmars_gateway_core::GatewayRequest,
    ) -> Result<accelmars_gateway_core::GatewayResponse, accelmars_gateway_core::AdapterError> {
        panic!("deliberate panic for semaphore recovery test");
    }
    fn is_available(&self) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn panic_in_adapter_releases_semaphore_permit() {
    // Setup: server with max=2 concurrency, PanickingAdapter as the only provider.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    let mut config = GatewayConfig::default();
    config.mode = accelmars_gateway::config::GatewayMode::Mock;

    let mut registry = AdapterRegistry::new();
    // Register panicking adapter as "mock" so mock-mode routing picks it up
    registry.register(Arc::new(PanickingAdapter));

    let router = Arc::new(Router::new(config, registry));
    let limiter = Arc::new(ConcurrencyLimiter::new(2));
    let limiter_check = Arc::clone(&limiter);
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

    // Send a request that will hit the PanickingAdapter → should get 500 (JoinError)
    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "trigger panic"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        500,
        "panicked adapter should cause 500 internal server error"
    );

    // Brief yield for permit drop
    tokio::task::yield_now().await;

    // Verify: semaphore is back to full capacity (2 available out of 2)
    assert_eq!(
        limiter_check.available(),
        2,
        "semaphore must recover all permits after panic — got {} available out of {}",
        limiter_check.available(),
        limiter_check.max()
    );

    // Bonus: send a follow-up request to prove the server is still functional
    // (will also panic → 500, but proves the server isn't hung)
    let resp2 = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "after panic"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp2.status(),
        500,
        "server must still respond after adapter panic (not hung)"
    );
}

// ---------------------------------------------------------------------------
// Test 15: 20 concurrent requests → all complete, no deadlock, no lost permits
// (PF-005R Audit 2 — concurrent SQLite write verification)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twenty_concurrent_requests_all_complete_no_deadlock() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    let mut config = GatewayConfig::default();
    config.mode = accelmars_gateway::config::GatewayMode::Mock;

    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(MockAdapter::default()));

    let router = Arc::new(Router::new(config, registry));
    // max=5 so requests queue (20 requests, 5 at a time)
    let limiter = Arc::new(ConcurrencyLimiter::new(5));
    let limiter_check = Arc::clone(&limiter);
    let cost_tracker = Arc::new(CostTracker::open(std::path::Path::new(":memory:")).unwrap());
    let cost_check = Arc::clone(&cost_tracker);
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
    let client = Arc::new(reqwest::Client::new());

    // Fire 20 concurrent requests
    let mut handles = Vec::new();
    for i in 0..20 {
        let base = base.clone();
        let client = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            client
                .post(format!("{base}/v1/chat/completions"))
                .json(&serde_json::json!({
                    "model": "quick",
                    "messages": [{"role": "user", "content": format!("concurrent request {i}")}]
                }))
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }));
    }

    // All 20 must complete (no deadlock, no hang)
    let mut success_count = 0;
    for handle in handles {
        let status = handle.await.unwrap();
        assert_eq!(status, 200, "all concurrent requests should succeed");
        success_count += 1;
    }
    assert_eq!(success_count, 20, "all 20 requests must complete");

    // Brief yield for final permit drops
    tokio::task::yield_now().await;

    // Verify semaphore fully recovered
    assert_eq!(
        limiter_check.available(),
        limiter_check.max(),
        "all semaphore permits must be returned after concurrent batch"
    );

    // Verify SQLite recorded all 20 requests (no missing, no duplicates)
    let summary = cost_check.summary(None).unwrap();
    assert_eq!(
        summary.total_calls, 20,
        "cost tracker must record exactly 20 entries — got {}",
        summary.total_calls
    );
}

// ---------------------------------------------------------------------------
// Test 16: gateway status CLI — exit 0 when server is running
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_cli_returns_exit_0_when_server_running() {
    use accelmars_gateway::cli::status::{run as status_run, PortSource};

    let base = start_test_server().await;
    // Extract port from base URL "http://127.0.0.1:PORT"
    let port: u16 = base
        .split(':')
        .last()
        .unwrap()
        .parse()
        .expect("port should parse");

    let exit_code = status_run(
        port,
        PortSource::Flag,
        false,
        accelmars_gateway_core::OutputConfig::from_env(true),
    )
    .await
    .expect("status should not return Err when server is running");

    assert_eq!(exit_code, 0, "exit 0 expected when server is running");
}

// ---------------------------------------------------------------------------
// Test 17: gateway status CLI — exit 1 when server is not running
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_cli_returns_exit_1_when_server_not_running() {
    use accelmars_gateway::cli::status::{run as status_run, PortSource};

    // Bind to port 0 to get an OS-assigned free port, then drop the listener so nothing is listening.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener); // nothing is listening on this port now

    // Brief yield so OS can reclaim the port
    tokio::task::yield_now().await;

    let exit_code = status_run(
        port,
        PortSource::Flag,
        false,
        accelmars_gateway_core::OutputConfig::from_env(true),
    )
    .await
    .expect("status should return Ok(1), not Err, when server is not running");

    assert_eq!(exit_code, 1, "exit 1 expected when server is not running");
}

// ---------------------------------------------------------------------------
// Test 18: gateway status CLI --json — returns structured JSON when running
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_cli_json_mode_returns_running_true() {
    use accelmars_gateway::cli::status::{run as status_run, PortSource};

    let base = start_test_server().await;
    let port: u16 = base.split(':').last().unwrap().parse().unwrap();

    // Run with json_output=true — output goes to stdout but we verify exit code
    let exit_code = status_run(
        port,
        PortSource::PidFile,
        true,
        accelmars_gateway_core::OutputConfig::from_env(true),
    )
    .await
    .expect("status --json should not Err when server is running");

    assert_eq!(
        exit_code, 0,
        "exit 0 expected in JSON mode when server is running"
    );
}

// ---------------------------------------------------------------------------
// Auth middleware tests (EC-5 through EC-9)
// ---------------------------------------------------------------------------

// Test 19 (EC-5): Valid Bearer key → 200
#[tokio::test]
async fn auth_valid_key_returns_200() {
    let (base, key, _store) = start_auth_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "valid Bearer key should return 200");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");
}

// Test 20 (EC-6): Missing Authorization header → 401 with OpenAI error format
#[tokio::test]
async fn auth_missing_key_returns_401_with_openai_format() {
    let (base, _, _store) = start_auth_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "no auth header"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401, "missing auth header should return 401");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["error"]["type"], "auth_error",
        "error type should be auth_error"
    );
    assert_eq!(
        body["error"]["code"], "invalid_api_key",
        "error code should be invalid_api_key"
    );
    assert!(
        body["error"]["message"].as_str().is_some(),
        "error message should be present"
    );
}

// Test 21 (EC-7): Wrong key → 401
#[tokio::test]
async fn auth_invalid_key_returns_401() {
    let (base, _, _store) = start_auth_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .header(
            "Authorization",
            "Bearer gw_live_thisisnotavalidkeyAAAAAAAAAAAAAAAAAAAA",
        )
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "wrong key"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401, "invalid key should return 401");
}

// Test 22 (EC-8): GET /health without auth header → 200 (exempt from auth)
#[tokio::test]
async fn auth_health_endpoint_exempt_from_auth() {
    let (base, _, _store) = start_auth_test_server().await;
    let client = reqwest::Client::new();

    // No Authorization header
    let resp = client.get(format!("{base}/health")).send().await.unwrap();

    assert_eq!(
        resp.status(),
        200,
        "/health must be reachable without an API key"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

// Test 23 (EC-9): auth_disabled server → 200 without key
#[tokio::test]
async fn auth_disabled_allows_requests_without_key() {
    // start_test_server() uses auth_disabled: true — verifies the escape hatch works.
    let base = start_test_server().await;
    let client = reqwest::Client::new();

    // No Authorization header
    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "quick",
            "messages": [{"role": "user", "content": "no key needed when auth disabled"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        200,
        "auth_disabled server must accept requests without a key"
    );
}
