use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::stream;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

use accelmars_gateway_core::{
    AdapterError, Capability, CostPreference, GatewayRequest, Latency, Message, ModelTier, Privacy,
    ProviderAdapter, RoutingConstraints,
};

use crate::openai::{
    ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, Choice,
    ErrorDetail, ErrorResponse, StreamChoice, StreamDelta, Usage,
};

/// Shared server state — cheaply cloneable (all fields are `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub adapter: Arc<dyn ProviderAdapter>,
    pub healthy: Arc<AtomicBool>,
}

/// Start the gateway on the given port. Blocks until the server shuts down.
pub async fn serve(port: u16, adapter: Arc<dyn ProviderAdapter>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    serve_with_listener(listener, adapter).await
}

/// Start the gateway on an already-bound listener.
///
/// Exposed for integration tests: bind to port 0, get the assigned address, then call this.
pub async fn serve_with_listener(
    listener: TcpListener,
    adapter: Arc<dyn ProviderAdapter>,
) -> anyhow::Result<()> {
    let healthy = Arc::new(AtomicBool::new(true));
    let state = AppState {
        adapter,
        healthy: healthy.clone(),
    };

    let addr = listener.local_addr()?;
    info!("gateway listening on {addr}");

    let app = Router::new()
        .route("/v1/chat/completions", post(handle_completion))
        .route("/health", get(handle_health))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(healthy))
        .await?;

    info!("gateway shut down cleanly");
    Ok(())
}

/// Waits for SIGTERM (Unix) or Ctrl-C, then marks the server unhealthy.
/// The health endpoint returns 503 while in-flight requests drain.
async fn shutdown_signal(healthy: Arc<AtomicBool>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let sigterm = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = sigterm => {},
    }

    healthy.store(false, Ordering::SeqCst);
    info!("shutdown signal received — draining in-flight requests");
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

async fn handle_health(State(state): State<AppState>) -> Response {
    if state.healthy.load(Ordering::SeqCst) {
        (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "shutting_down"})),
        )
            .into_response()
    }
}

async fn handle_completion(
    State(state): State<AppState>,
    Json(req): Json<ChatCompletionRequest>,
) -> Response {
    // Validate messages present
    if req.messages.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::invalid_request("messages cannot be empty")),
        )
            .into_response();
    }

    // Parse tier from model field (quick / standard / max / ultra)
    let tier = match req.model.parse::<ModelTier>() {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::invalid_model(&req.model)),
            )
                .into_response();
        }
    };

    let constraints = parse_constraints(&req);
    let is_stream = req.stream.unwrap_or(false);

    let gateway_req = GatewayRequest {
        tier,
        constraints,
        messages: req
            .messages
            .iter()
            .map(|m| Message {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect(),
        max_tokens: req.max_tokens,
        stream: is_stream,
        metadata: Default::default(),
    };

    if is_stream {
        complete_stream(state, tier, gateway_req).await
    } else {
        complete_json(state, tier, gateway_req).await
    }
}

// ---------------------------------------------------------------------------
// Non-streaming path
// ---------------------------------------------------------------------------

async fn complete_json(state: AppState, tier: ModelTier, gateway_req: GatewayRequest) -> Response {
    let start = Instant::now();
    let adapter = state.adapter.clone();

    // ProviderAdapter::complete is sync — run on blocking thread pool (ENGINE-LESSONS Rule 2 equivalent)
    let result = tokio::task::spawn_blocking(move || adapter.complete(&gateway_req)).await;
    let latency_ms = start.elapsed().as_millis();

    match result {
        Err(join_err) => {
            error!("adapter task panicked: {join_err}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::provider_error("internal error")),
            )
                .into_response()
        }
        Ok(Err(adapter_err)) => {
            let (status, body) = adapter_error_to_response(&adapter_err);
            error!(
                error = %adapter_err,
                tier = %tier,
                latency_ms,
                "completion failed"
            );
            (status, Json(body)).into_response()
        }
        Ok(Ok(resp)) => {
            info!(
                tier = %tier,
                provider = %resp.model,
                latency_ms,
                tokens_in = resp.tokens_in,
                tokens_out = resp.tokens_out,
                "completion ok"
            );
            let now = unix_now();
            (
                StatusCode::OK,
                Json(ChatCompletionResponse {
                    id: resp.id,
                    object: "chat.completion".to_string(),
                    created: now,
                    model: resp.model,
                    choices: vec![Choice {
                        index: 0,
                        message: ChatMessage {
                            role: "assistant".to_string(),
                            content: resp.content,
                        },
                        finish_reason: resp.finish_reason,
                    }],
                    usage: Usage {
                        prompt_tokens: resp.tokens_in,
                        completion_tokens: resp.tokens_out,
                        total_tokens: resp.tokens_in + resp.tokens_out,
                    },
                }),
            )
                .into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Streaming path (SSE)
// Phase 1: MockAdapter only — full response as a single content chunk + DONE sentinel.
// Phase 2+ (real providers): each token becomes a chunk.
// ---------------------------------------------------------------------------

async fn complete_stream(
    state: AppState,
    tier: ModelTier,
    gateway_req: GatewayRequest,
) -> Response {
    let start = Instant::now();
    let adapter = state.adapter.clone();

    let result = tokio::task::spawn_blocking(move || adapter.complete(&gateway_req)).await;
    let latency_ms = start.elapsed().as_millis();

    match result {
        Err(join_err) => {
            error!("adapter task panicked: {join_err}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::provider_error("internal error")),
            )
                .into_response()
        }
        Ok(Err(adapter_err)) => {
            let (status, body) = adapter_error_to_response(&adapter_err);
            (status, Json(body)).into_response()
        }
        Ok(Ok(resp)) => {
            info!(
                tier = %tier,
                provider = %resp.model,
                latency_ms,
                tokens_in = resp.tokens_in,
                tokens_out = resp.tokens_out,
                stream = true,
                "completion ok (stream)"
            );
            let now = unix_now();

            // Content chunk
            let content_chunk = ChatCompletionChunk {
                id: resp.id.clone(),
                object: "chat.completion.chunk".to_string(),
                created: now,
                model: resp.model.clone(),
                choices: vec![StreamChoice {
                    index: 0,
                    delta: StreamDelta {
                        role: Some("assistant".to_string()),
                        content: Some(resp.content),
                    },
                    finish_reason: None,
                }],
            };

            // Finish chunk
            let finish_chunk = ChatCompletionChunk {
                id: resp.id,
                object: "chat.completion.chunk".to_string(),
                created: now,
                model: resp.model,
                choices: vec![StreamChoice {
                    index: 0,
                    delta: StreamDelta {
                        role: None,
                        content: None,
                    },
                    finish_reason: Some(resp.finish_reason),
                }],
            };

            let events: Vec<Result<Event, Infallible>> = vec![
                Ok(Event::default()
                    .data(serde_json::to_string(&content_chunk).unwrap_or_default())),
                Ok(Event::default().data(serde_json::to_string(&finish_chunk).unwrap_or_default())),
                Ok(Event::default().data("[DONE]")),
            ];

            Sse::new(stream::iter(events)).into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_constraints(req: &ChatCompletionRequest) -> RoutingConstraints {
    let mut c = RoutingConstraints::default();
    let Some(ref meta) = req.metadata else {
        return c;
    };

    if let Some(s) = meta.get("privacy").and_then(|v| v.as_str()) {
        c.privacy = match s {
            "sensitive" => Privacy::Sensitive,
            "private" => Privacy::Private,
            _ => Privacy::Open,
        };
    }
    if let Some(s) = meta.get("latency").and_then(|v| v.as_str()) {
        if s == "low" {
            c.latency = Latency::Low;
        }
    }
    if let Some(s) = meta.get("cost").and_then(|v| v.as_str()) {
        c.cost = match s {
            "free" => CostPreference::Free,
            "budget" => CostPreference::Budget,
            "unlimited" => CostPreference::Unlimited,
            _ => CostPreference::Default,
        };
    }
    if let Some(arr) = meta.get("capabilities").and_then(|v| v.as_array()) {
        for item in arr {
            let cap = match item.as_str() {
                Some("reasoning") => Some(Capability::Reasoning),
                Some("tool_use") => Some(Capability::ToolUse),
                Some("vision") => Some(Capability::Vision),
                Some("code") => Some(Capability::Code),
                Some("long_context") => Some(Capability::LongContext),
                _ => None,
            };
            if let Some(cap) = cap {
                c.capabilities.push(cap);
            }
        }
    }
    if let Some(s) = meta.get("provider").and_then(|v| v.as_str()) {
        c.provider = Some(s.to_string());
    }
    c
}

fn adapter_error_to_response(err: &AdapterError) -> (StatusCode, ErrorResponse) {
    match err {
        AdapterError::RateLimit { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            ErrorResponse {
                error: ErrorDetail {
                    message: err.to_string(),
                    error_type: "rate_limit_error".to_string(),
                    code: "rate_limit_exceeded".to_string(),
                },
            },
        ),
        AdapterError::AuthError(_) => (
            StatusCode::UNAUTHORIZED,
            ErrorResponse {
                error: ErrorDetail {
                    message: err.to_string(),
                    error_type: "authentication_error".to_string(),
                    code: "invalid_api_key".to_string(),
                },
            },
        ),
        AdapterError::Timeout => (
            StatusCode::GATEWAY_TIMEOUT,
            ErrorResponse {
                error: ErrorDetail {
                    message: err.to_string(),
                    error_type: "timeout_error".to_string(),
                    code: "gateway_timeout".to_string(),
                },
            },
        ),
        AdapterError::ProviderError(_) => (
            StatusCode::BAD_GATEWAY,
            ErrorResponse {
                error: ErrorDetail {
                    message: err.to_string(),
                    error_type: "provider_error".to_string(),
                    code: "provider_error".to_string(),
                },
            },
        ),
        AdapterError::ParseError(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorResponse {
                error: ErrorDetail {
                    message: err.to_string(),
                    error_type: "api_error".to_string(),
                    code: "parse_error".to_string(),
                },
            },
        ),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
