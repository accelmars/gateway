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

use accelmars_gateway::registry::AdapterRegistry;
use accelmars_gateway::server::serve_with_listener;
use accelmars_gateway_core::MockAdapter;
use tokio::net::TcpListener;

/// Bind port 0, start the server in a background task, return the base URL.
async fn start_test_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(MockAdapter::default()));
    let registry = Arc::new(registry);
    tokio::spawn(async move {
        serve_with_listener(listener, registry).await.ok();
    });
    // Brief yield to let the server task start
    tokio::task::yield_now().await;
    format!("http://{addr}")
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
            "messages": [{"role": "system", "content": "You are helpful."}, {"role": "user", "content": "Hello"}],
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
