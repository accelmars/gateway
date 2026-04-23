use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router as AxumRouter;
use futures_util::stream;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{error, info, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use accelmars_gateway_core::{
    AdapterError, Capability, CostPreference, GatewayRequest, Latency, Message, ModelTier, Privacy,
    RoutingConstraints,
};

use crate::auth::AuthStore;
use crate::concurrency::ConcurrencyLimiter;
use crate::cost::{CostTracker, RequestRecord};
use crate::openai::{
    ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, Choice,
    ErrorDetail, ErrorResponse, StreamChoice, StreamDelta, Usage,
};
use crate::router::{RouteDecision, Router};

/// Shared server state — cheaply cloneable (all fields are `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub router: Arc<Router>,
    pub limiter: Arc<ConcurrencyLimiter>,
    pub cost_tracker: Arc<CostTracker>,
    pub auth: Arc<AuthStore>,
    /// When true, all requests are allowed without an API key.
    /// Set from `GATEWAY_AUTH_DISABLED` env var at startup. Never check per-request env var.
    pub auth_disabled: bool,
    pub healthy: Arc<AtomicBool>,
    pub start_time: Instant,
    pub port: u16,
}

/// Start the gateway on the given port. Blocks until the server shuts down.
pub async fn serve(
    port: u16,
    router: Arc<Router>,
    limiter: Arc<ConcurrencyLimiter>,
    cost_tracker: Arc<CostTracker>,
    auth_store: Arc<AuthStore>,
    auth_disabled: bool,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    serve_with_listener(
        listener,
        router,
        limiter,
        cost_tracker,
        auth_store,
        auth_disabled,
        port,
    )
    .await
}

/// Start the gateway on an already-bound listener.
///
/// Exposed for integration tests: bind to port 0, get the assigned address, then call this.
pub async fn serve_with_listener(
    listener: TcpListener,
    router: Arc<Router>,
    limiter: Arc<ConcurrencyLimiter>,
    cost_tracker: Arc<CostTracker>,
    auth_store: Arc<AuthStore>,
    auth_disabled: bool,
    port: u16,
) -> anyhow::Result<()> {
    let healthy = Arc::new(AtomicBool::new(true));
    let state = AppState {
        router,
        limiter,
        cost_tracker,
        auth: auth_store,
        auth_disabled,
        healthy: healthy.clone(),
        start_time: Instant::now(),
        port,
    };

    let addr = listener.local_addr()?;
    info!("gateway listening on {addr}");

    let app = AxumRouter::new()
        .route("/v1/chat/completions", post(handle_completion))
        .route("/health", get(handle_health))
        .route("/status", get(handle_status))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(healthy))
        .await?;

    info!("gateway shut down cleanly");
    Ok(())
}

/// Waits for SIGTERM (Unix) or Ctrl-C, then marks the server unhealthy.
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
// Auth middleware
// ---------------------------------------------------------------------------

/// Validates `Authorization: Bearer <key>` on all routes except `/health` and `/status`.
///
/// On success, inserts the API key's record ID as a `String` extension so
/// `handle_completion` can attribute cost records to the specific key.
///
/// Fail-open: if the auth DB is unavailable, a warning is logged and the request
/// proceeds. Consistent with the cost tracker's fail-open philosophy.
async fn auth_middleware(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = request.uri().path();

    // Health and status endpoints are always exempt — load balancers and CLI use these.
    if path == "/health" || path == "/status" {
        return next.run(request).await;
    }

    // Auth globally disabled (local dev / mock mode) — skip validation.
    if state.auth_disabled {
        return next.run(request).await;
    }

    // Extract Bearer token from Authorization header.
    let key = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);

    match key {
        None => ErrorResponse::auth_error(
            "Missing API key. Include Authorization: Bearer <key> header.",
        )
        .into_response(),
        Some(key) => match state.auth.validate_key(&key) {
            Ok(Some(record)) => {
                // Attach key ID to request extensions for cost attribution.
                request.extensions_mut().insert(record.id.clone());
                next.run(request).await
            }
            Ok(None) => ErrorResponse::auth_error("Invalid or revoked API key.").into_response(),
            Err(_) => {
                // Auth DB error — fail-open so a broken auth DB doesn't take down production.
                tracing::warn!("auth store error — allowing request (fail-open)");
                next.run(request).await
            }
        },
    }
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

async fn handle_status(State(state): State<AppState>) -> Response {
    let uptime_seconds = state.start_time.elapsed().as_secs();
    let mode = format!("{:?}", state.router.mode()).to_lowercase();
    let providers = state.router.provider_statuses();

    let payload = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "status": "running",
        "port": state.port,
        "mode": mode,
        "uptime_seconds": uptime_seconds,
        "concurrency": {
            "active": state.limiter.active(),
            "available": state.limiter.available(),
            "max": state.limiter.max()
        },
        "providers": providers
    });

    (StatusCode::OK, Json(payload)).into_response()
}

async fn handle_completion(
    State(state): State<AppState>,
    key_id: Option<axum::extract::Extension<String>>,
    Json(req): Json<ChatCompletionRequest>,
) -> Response {
    let span = tracing::info_span!("chat_completion");

    async {
        let key_id = key_id.map(|ext| ext.0);

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

        // Extract engine identifier from metadata (cortex, guild, etc.)
        let engine = req
            .metadata
            .as_ref()
            .and_then(|m| m.get("engine"))
            .and_then(|v| v.as_str())
            .map(String::from);

        // Set engine attribute on the span if provided
        if let Some(ref engine) = engine {
            tracing::Span::current()
                .set_attribute(crate::telemetry::GATEWAY_ENGINE, engine.clone());
        }

        // Set max_tokens on span if provided
        if let Some(max_tokens) = req.max_tokens {
            tracing::Span::current().set_attribute(
                crate::telemetry::GEN_AI_REQUEST_MAX_TOKENS,
                i64::from(max_tokens),
            );
        }

        // Acquire concurrency permit — queues until a slot opens or 30s timeout → 504
        let _permit = match state.limiter.acquire().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("concurrency queue timeout: {e}");
                return (
                    StatusCode::GATEWAY_TIMEOUT,
                    Json(ErrorResponse {
                        error: ErrorDetail {
                            message: e.to_string(),
                            error_type: "rate_limit_error".to_string(),
                            code: "concurrency_timeout".to_string(),
                        },
                    }),
                )
                    .into_response();
            }
        };

        // Measure gateway routing overhead (resolve + request assembly, excludes provider latency)
        let resolve_start = Instant::now();

        // Resolve provider via router (tier + constraints → RouteDecision)
        let decision = match state.router.resolve(tier, &constraints) {
            Ok(d) => d,
            Err(e) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(ErrorResponse::provider_error(e.to_string())),
                )
                    .into_response();
            }
        };

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

        let overhead_ms = resolve_start.elapsed().as_millis() as u64;

        if is_stream {
            complete_stream(state, decision, tier, gateway_req, key_id, overhead_ms).await
            // _permit dropped here — slot released after streaming completes
        } else {
            complete_json(state, decision, tier, gateway_req, key_id, overhead_ms).await
            // _permit dropped here — slot released after JSON response returned
        }
    }
    .instrument(span)
    .await
}

// ---------------------------------------------------------------------------
// Non-streaming path
// ---------------------------------------------------------------------------

async fn complete_json(
    state: AppState,
    decision: RouteDecision,
    tier: ModelTier,
    gateway_req: GatewayRequest,
    key_id: Option<String>,
    overhead_ms: u64,
) -> Response {
    // Retry loop: up to 3 attempts, re-resolving via the fallback chain on each adapter failure.
    // The concurrency permit (_permit in handle_completion) covers the entire loop — one slot
    // per request regardless of how many fallback attempts are made.
    const MAX_ATTEMPTS: u32 = 3;
    let mut current_decision = decision;
    let total_start = Instant::now();

    for attempt in 0..MAX_ATTEMPTS {
        let start = Instant::now();
        let provider_name = current_decision.provider_name.clone();
        // Arc clone is cheap — adapter is shared behind Arc<dyn ProviderAdapter>
        let adapter = current_decision.adapter.clone();
        let req_clone = gateway_req.clone();

        let result = tokio::task::spawn_blocking(move || adapter.complete(&req_clone)).await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Err(join_err) => {
                // Task panic — never retry (panic is not a transient error)
                error!("adapter task panicked: {join_err}");
                state.router.on_failure(&provider_name);
                state.cost_tracker.record(&RequestRecord {
                    id: new_request_id(),
                    timestamp: iso_now(),
                    tier: tier.to_string(),
                    provider: provider_name,
                    model: "unknown".to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost_usd: 0.0,
                    latency_ms,
                    status: "error".to_string(),
                    error_type: Some("internal_error".to_string()),
                    constraints: None,
                    key_id,
                });
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::provider_error("internal error")),
                )
                    .into_response();
            }
            Ok(Err(adapter_err)) => {
                let (status, body) = adapter_error_to_response(&adapter_err);
                error!(
                    error = %adapter_err,
                    tier = %tier,
                    provider = %provider_name,
                    latency_ms,
                    attempt,
                    "completion failed"
                );
                state.router.on_failure(&provider_name);
                state.cost_tracker.record(&RequestRecord {
                    id: new_request_id(),
                    timestamp: iso_now(),
                    tier: tier.to_string(),
                    provider: provider_name.clone(),
                    model: "unknown".to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost_usd: 0.0,
                    latency_ms,
                    status: "error".to_string(),
                    error_type: Some(adapter_error_type(&adapter_err).to_string()),
                    constraints: None,
                    key_id: key_id.clone(),
                });

                // Try to re-resolve through the fallback chain (circuit is now OPEN for
                // provider_name, so the router will skip it and try the next provider)
                if attempt + 1 < MAX_ATTEMPTS {
                    match state.router.resolve(tier, &gateway_req.constraints) {
                        Ok(next) if next.provider_name != provider_name => {
                            tracing::info!(
                                attempt = attempt + 1,
                                failed_provider = %provider_name,
                                next_provider = %next.provider_name,
                                "retrying with fallback provider"
                            );
                            current_decision = next;
                            continue;
                        }
                        _ => {
                            // No different provider available — return the error
                        }
                    }
                }

                return (status, Json(body)).into_response();
            }
            Ok(Ok(resp)) => {
                let total_latency_ms = total_start.elapsed().as_millis() as u64;
                info!(
                    tier = %tier,
                    provider = %provider_name,
                    model = %resp.model,
                    latency_ms,
                    tokens_in = resp.tokens_in,
                    tokens_out = resp.tokens_out,
                    "completion ok"
                );
                state.router.on_success(&provider_name);

                let (cost_in, cost_out) = state.router.provider_pricing(&provider_name);
                let cost_usd = CostTracker::calculate_cost(
                    resp.tokens_in as u64,
                    resp.tokens_out as u64,
                    cost_in,
                    cost_out,
                );

                // Set OTel span attributes — GenAI semantic conventions + gateway attributes.
                // When OTel is not active, these calls are no-ops.
                let span = tracing::Span::current();
                span.set_attribute(
                    crate::telemetry::GEN_AI_SYSTEM,
                    crate::telemetry::provider_to_system(&provider_name),
                );
                span.set_attribute(crate::telemetry::GEN_AI_REQUEST_MODEL, resp.model.clone());
                span.set_attribute(
                    crate::telemetry::GEN_AI_USAGE_INPUT_TOKENS,
                    resp.tokens_in as i64,
                );
                span.set_attribute(
                    crate::telemetry::GEN_AI_USAGE_OUTPUT_TOKENS,
                    resp.tokens_out as i64,
                );
                span.set_attribute(
                    crate::telemetry::GEN_AI_RESPONSE_FINISH_REASONS,
                    resp.finish_reason.clone(),
                );
                span.set_attribute(crate::telemetry::GATEWAY_TIER, tier.to_string());
                span.set_attribute(crate::telemetry::GATEWAY_PROVIDER, provider_name.clone());
                span.set_attribute(crate::telemetry::GATEWAY_COST_USD, cost_usd);
                span.set_attribute(
                    crate::telemetry::GATEWAY_LATENCY_MS,
                    total_latency_ms as i64,
                );
                span.set_attribute(crate::telemetry::GATEWAY_OVERHEAD_MS, overhead_ms as i64);
                span.set_attribute(crate::telemetry::GATEWAY_FALLBACK, attempt > 0);

                state.cost_tracker.record(&RequestRecord {
                    id: resp.id.clone(),
                    timestamp: iso_now(),
                    tier: tier.to_string(),
                    provider: provider_name,
                    model: resp.model.clone(),
                    tokens_in: resp.tokens_in as u64,
                    tokens_out: resp.tokens_out as u64,
                    cost_usd,
                    latency_ms,
                    status: "ok".to_string(),
                    error_type: None,
                    constraints: None,
                    key_id,
                });

                let now = unix_now();
                return (
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
                    .into_response();
            }
        }
    }

    // All attempts exhausted without success (shouldn't reach here due to early returns above,
    // but provides a safe fallback for the loop exit path)
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ErrorResponse::provider_error(
            "all providers exhausted — no fallback available",
        )),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Streaming path (SSE)
// Phase 1: single content chunk + DONE sentinel.
// Phase 2+ (real providers): each token becomes a chunk.
// ---------------------------------------------------------------------------

async fn complete_stream(
    state: AppState,
    decision: RouteDecision,
    tier: ModelTier,
    gateway_req: GatewayRequest,
    key_id: Option<String>,
    overhead_ms: u64,
) -> Response {
    // Same retry-on-fallback pattern as complete_json. The concurrency permit
    // covers the entire retry loop — one slot per request.
    const MAX_ATTEMPTS: u32 = 3;
    let mut current_decision = decision;
    let total_start = Instant::now();

    for attempt in 0..MAX_ATTEMPTS {
        let start = Instant::now();
        let provider_name = current_decision.provider_name.clone();
        let adapter = current_decision.adapter.clone();
        let req_clone = gateway_req.clone();

        let result = tokio::task::spawn_blocking(move || adapter.complete(&req_clone)).await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Err(join_err) => {
                // Task panic — never retry
                error!("adapter task panicked: {join_err}");
                state.router.on_failure(&provider_name);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::provider_error("internal error")),
                )
                    .into_response();
            }
            Ok(Err(adapter_err)) => {
                error!(
                    error = %adapter_err,
                    tier = %tier,
                    provider = %provider_name,
                    latency_ms,
                    attempt,
                    stream = true,
                    "completion failed"
                );
                state.router.on_failure(&provider_name);

                // Try to re-resolve via fallback chain
                if attempt + 1 < MAX_ATTEMPTS {
                    match state.router.resolve(tier, &gateway_req.constraints) {
                        Ok(next) if next.provider_name != provider_name => {
                            tracing::info!(
                                attempt = attempt + 1,
                                failed_provider = %provider_name,
                                next_provider = %next.provider_name,
                                stream = true,
                                "retrying with fallback provider"
                            );
                            current_decision = next;
                            continue;
                        }
                        _ => {}
                    }
                }

                let (status, body) = adapter_error_to_response(&adapter_err);
                return (status, Json(body)).into_response();
            }
            Ok(Ok(resp)) => {
                let total_latency_ms = total_start.elapsed().as_millis() as u64;
                state.router.on_success(&provider_name);
                info!(
                    tier = %tier,
                    provider = %provider_name,
                    model = %resp.model,
                    latency_ms,
                    tokens_in = resp.tokens_in,
                    tokens_out = resp.tokens_out,
                    stream = true,
                    "completion ok (stream)"
                );

                let (cost_in, cost_out) = state.router.provider_pricing(&provider_name);
                let cost_usd = CostTracker::calculate_cost(
                    resp.tokens_in as u64,
                    resp.tokens_out as u64,
                    cost_in,
                    cost_out,
                );

                // Set OTel span attributes — GenAI semantic conventions + gateway attributes.
                let span = tracing::Span::current();
                span.set_attribute(
                    crate::telemetry::GEN_AI_SYSTEM,
                    crate::telemetry::provider_to_system(&provider_name),
                );
                span.set_attribute(crate::telemetry::GEN_AI_REQUEST_MODEL, resp.model.clone());
                span.set_attribute(
                    crate::telemetry::GEN_AI_USAGE_INPUT_TOKENS,
                    resp.tokens_in as i64,
                );
                span.set_attribute(
                    crate::telemetry::GEN_AI_USAGE_OUTPUT_TOKENS,
                    resp.tokens_out as i64,
                );
                span.set_attribute(
                    crate::telemetry::GEN_AI_RESPONSE_FINISH_REASONS,
                    resp.finish_reason.clone(),
                );
                span.set_attribute(crate::telemetry::GATEWAY_TIER, tier.to_string());
                span.set_attribute(crate::telemetry::GATEWAY_PROVIDER, provider_name.clone());
                span.set_attribute(crate::telemetry::GATEWAY_COST_USD, cost_usd);
                span.set_attribute(
                    crate::telemetry::GATEWAY_LATENCY_MS,
                    total_latency_ms as i64,
                );
                span.set_attribute(crate::telemetry::GATEWAY_OVERHEAD_MS, overhead_ms as i64);
                span.set_attribute(crate::telemetry::GATEWAY_FALLBACK, attempt > 0);

                state.cost_tracker.record(&RequestRecord {
                    id: resp.id.clone(),
                    timestamp: iso_now(),
                    tier: tier.to_string(),
                    provider: provider_name,
                    model: resp.model.clone(),
                    tokens_in: resp.tokens_in as u64,
                    tokens_out: resp.tokens_out as u64,
                    cost_usd,
                    latency_ms,
                    status: "ok".to_string(),
                    error_type: None,
                    constraints: None,
                    key_id,
                });

                let now = unix_now();

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
                    Ok(Event::default()
                        .data(serde_json::to_string(&finish_chunk).unwrap_or_default())),
                    Ok(Event::default().data("[DONE]")),
                ];

                return Sse::new(stream::iter(events)).into_response();
            }
        }
    }

    // All attempts exhausted
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ErrorResponse::provider_error(
            "all providers exhausted — no fallback available",
        )),
    )
        .into_response()
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

fn adapter_error_type(err: &AdapterError) -> &'static str {
    match err {
        AdapterError::RateLimit { .. } => "rate_limit",
        AdapterError::AuthError(_) => "auth_error",
        AdapterError::Timeout => "timeout",
        AdapterError::ProviderError(_) => "provider_error",
        AdapterError::ParseError(_) => "parse_error",
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn iso_now() -> String {
    // RFC 3339 / ISO 8601 UTC
    chrono_from_secs(unix_now())
}

fn chrono_from_secs(secs: u64) -> String {
    // Minimal ISO timestamp without pulling in chrono.
    // Format: "YYYY-MM-DDTHH:MM:SSZ"
    let s = secs;
    let (y, mo, d, h, mi, sec) = epoch_to_datetime(s);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{sec:02}Z")
}

/// Minimal epoch → (year, month, day, hour, min, sec) without external crates.
fn epoch_to_datetime(epoch: u64) -> (u32, u32, u32, u32, u32, u32) {
    let sec = (epoch % 60) as u32;
    let epoch = epoch / 60;
    let min = (epoch % 60) as u32;
    let epoch = epoch / 60;
    let hour = (epoch % 24) as u32;
    let mut days = epoch / 24;

    // Days since 1970-01-01
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days as u32 + 1, hour, min, sec)
}

fn is_leap(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn new_request_id() -> String {
    use uuid::Uuid;
    Uuid::new_v4().to_string()
}
