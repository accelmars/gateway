# Observability — Span Data Schema

> Gateway-engine OpenTelemetry span schema. Every `/v1/chat/completions` request
> produces a `chat_completion` span with the attributes below.
>
> **Phase 3 note:** This schema becomes the customer-facing observability product
> (Stream 3: $49-499/mo). Per-engine attribution (`gateway.engine`) becomes
> per-customer attribution. The schema must not change when customers arrive.

## Service Resource

| Key | Value |
|-----|-------|
| `service.name` | `accelmars-gateway` |
| `service.version` | Cargo package version (e.g., `0.2.0`) |

## GenAI Semantic Convention Attributes

Following [OpenTelemetry GenAI specification](https://opentelemetry.io/docs/specs/semconv/gen-ai/).

| Attribute | Type | Description | Example |
|-----------|------|-------------|---------|
| `gen_ai.system` | string | AI provider system identifier | `anthropic`, `google`, `deepseek`, `groq`, `openrouter`, `mock` |
| `gen_ai.request.model` | string | Resolved model ID sent to provider | `claude-sonnet-4-20250514` |
| `gen_ai.request.max_tokens` | i64 | Max tokens from request (if set) | `4096` |
| `gen_ai.usage.input_tokens` | i64 | Input tokens consumed | `150` |
| `gen_ai.usage.output_tokens` | i64 | Output tokens generated | `320` |
| `gen_ai.response.finish_reasons` | string | Why generation stopped | `stop`, `max_tokens` |

## Custom Gateway Attributes

Namespaced under `gateway.*` to avoid collision with standard conventions.

| Attribute | Type | Description | Example |
|-----------|------|-------------|---------|
| `gateway.tier` | string | Requested model tier | `quick`, `standard`, `max`, `ultra` |
| `gateway.provider` | string | Resolved provider name | `claude-sonnet`, `gemini-flash`, `deepseek` |
| `gateway.cost_usd` | f64 | Calculated cost for this request | `0.0042` |
| `gateway.latency_ms` | i64 | Total request latency (including retries) | `1250` |
| `gateway.overhead_ms` | i64 | Gateway routing overhead only (excludes provider latency) | `2` |
| `gateway.fallback` | bool | Whether a fallback provider was used | `true` |
| `gateway.engine` | string | Calling engine identifier (from request metadata) | `cortex`, `guild` |

### Notes

- **`gateway.overhead_ms`** measures only the time for `router.resolve()` + request
  assembly. It excludes provider call latency. Typical values < 5ms. If this exceeds
  10ms consistently, investigate the routing logic.
- **`gateway.fallback`** is `true` when the initial provider failed and a fallback
  was used via the circuit breaker chain.
- **`gateway.engine`** is set when the calling client includes
  `"metadata": {"engine": "cortex"}` in the request body. Foundation for per-customer
  cost dashboards in Phase 3.
- **`gateway.cost_usd`** uses the gateway's internal pricing model. May differ from
  provider invoices due to rounding and pricing lag.

## Activation

Controlled by standard OpenTelemetry environment variables:

| Env Var | Required | Description |
|---------|----------|-------------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Yes | OTLP collector endpoint (e.g., `https://otlp-gateway-prod-us-central-0.grafana.net/otlp`) |
| `OTEL_EXPORTER_OTLP_HEADERS` | For auth | Authentication header (e.g., `Authorization=Basic <base64>`) |

When `OTEL_EXPORTER_OTLP_ENDPOINT` is **not set**, the gateway runs in fmt-only
mode — console logging only, no OTel overhead, identical to Phase 1 behavior.

## Fail-Open Guarantee

- If OTel setup fails at startup: warning logged, fmt-only mode continues.
- If OTel export fails at runtime: spans are dropped silently by the batch processor.
  No AI request is ever blocked or delayed by observability failures.
- No OTel operation in the request path can return an error that aborts a request.

## Export Protocol

- **Transport:** HTTP/protobuf (OTLP)
- **Batch processing:** Background thread (not on tokio runtime)
- **Compatible with:** Grafana Cloud, Jaeger, any OTLP-compatible collector

## Grafana Cloud Setup

1. Create free-tier account at grafana.com
2. Generate OTLP endpoint and API token
3. Set environment variables:
   ```
   OTEL_EXPORTER_OTLP_ENDPOINT=https://<instance>.grafana.net/otlp
   OTEL_EXPORTER_OTLP_HEADERS=Authorization=Basic <base64-encoded-credentials>
   ```
4. Verify traces in Grafana Cloud Traces explorer

---

_AccelMars Co., Ltd. — gateway-engine observability spec_
