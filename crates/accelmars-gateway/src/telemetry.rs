//! OpenTelemetry tracing setup — GenAI semantic conventions + gateway attributes.
//!
//! Conditional on `OTEL_EXPORTER_OTLP_ENDPOINT`:
//! - **Set:** layered subscriber (fmt + OTel OTLP export via HTTP/protobuf)
//! - **Unset:** fmt-only subscriber (zero change from Phase 1 behavior)
//!
//! Fail-open: OTel setup errors are logged at `warn` and swallowed.
//! Observability never blocks AI requests.

use opentelemetry::trace::TracerProvider;
use opentelemetry::KeyValue;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// GenAI semantic convention attribute keys
// https://opentelemetry.io/docs/specs/semconv/gen-ai/
// ---------------------------------------------------------------------------

pub const GEN_AI_SYSTEM: &str = "gen_ai.system";
pub const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
pub const GEN_AI_REQUEST_MAX_TOKENS: &str = "gen_ai.request.max_tokens";
pub const GEN_AI_USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
pub const GEN_AI_USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
pub const GEN_AI_RESPONSE_FINISH_REASONS: &str = "gen_ai.response.finish_reasons";

// ---------------------------------------------------------------------------
// Custom gateway attributes (namespaced `gateway.*`)
// ---------------------------------------------------------------------------

pub const GATEWAY_TIER: &str = "gateway.tier";
pub const GATEWAY_PROVIDER: &str = "gateway.provider";
pub const GATEWAY_COST_USD: &str = "gateway.cost_usd";
pub const GATEWAY_LATENCY_MS: &str = "gateway.latency_ms";
pub const GATEWAY_OVERHEAD_MS: &str = "gateway.overhead_ms";
pub const GATEWAY_FALLBACK: &str = "gateway.fallback";
pub const GATEWAY_ENGINE: &str = "gateway.engine";

/// Initialize the tracing subscriber with optional OpenTelemetry export.
///
/// Returns the [`SdkTracerProvider`] handle when OTel is active (needed for
/// graceful shutdown to flush buffered spans). Returns `None` for fmt-only mode.
pub fn init_tracing(log_level: &str) -> Option<SdkTracerProvider> {
    let filter = EnvFilter::try_new(log_level).unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer();

    match std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        Ok(endpoint) if !endpoint.is_empty() => match build_tracer_provider() {
            Ok(provider) => {
                let tracer = provider.tracer("accelmars-gateway");
                let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

                tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt_layer)
                    .with(otel_layer)
                    .init();

                tracing::info!(
                    endpoint = %endpoint,
                    "OpenTelemetry tracing active — exporting spans via OTLP"
                );

                Some(provider)
            }
            Err(e) => {
                // Fail-open: continue with fmt-only
                tracing_subscriber::registry()
                    .with(filter)
                    .with(fmt_layer)
                    .init();

                tracing::warn!(
                    error = %e,
                    "OpenTelemetry setup failed — falling back to fmt-only logging"
                );

                None
            }
        },
        _ => {
            // No OTLP endpoint configured — fmt-only (local dev default)
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .init();

            None
        }
    }
}

/// Build the OTel tracer provider with OTLP HTTP exporter and service resource.
///
/// Reads `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_HEADERS` from
/// environment (standard OTel env vars — no hardcoded URLs).
fn build_tracer_provider() -> Result<SdkTracerProvider, Box<dyn std::error::Error>> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()?;

    let provider = SdkTracerProvider::builder()
        .with_resource(
            Resource::builder()
                .with_attributes([
                    KeyValue::new("service.name", "accelmars-gateway"),
                    KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                ])
                .build(),
        )
        .with_batch_exporter(exporter)
        .build();

    Ok(provider)
}

/// Map gateway provider name to GenAI semantic convention system identifier.
pub fn provider_to_system(provider: &str) -> &'static str {
    if provider.starts_with("claude") {
        "anthropic"
    } else if provider.starts_with("gemini") {
        "google"
    } else if provider == "deepseek" {
        "deepseek"
    } else if provider.starts_with("openrouter") {
        "openrouter"
    } else if provider.starts_with("groq") {
        "groq"
    } else if provider == "mock" {
        "mock"
    } else {
        "unknown"
    }
}

/// Flush buffered spans and shut down the OTel provider.
///
/// Call during graceful shutdown after the HTTP server stops but before process exit.
/// Safe to call with `None` (fmt-only mode — no-op).
pub fn shutdown_tracing(provider: Option<SdkTracerProvider>) {
    if let Some(provider) = provider {
        if let Err(e) = provider.shutdown() {
            tracing::warn!(error = %e, "OpenTelemetry provider shutdown error");
        }
    }
}
