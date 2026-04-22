# Gateway Client Integration Guide

> Reference guide for engines integrating with the AccelMars Gateway.
> Version: v0.2.x — documents what is supported today.
>
> Coming in Phase 2: contract-aware routing via `task_type`/`quality_target`/`budget_usd`, fallback chains, semantic caching, OpenTelemetry traces.

---

## Quick Start (5 minutes)

Start the gateway, then send a completion request:

```bash
# 1. Start the gateway
cargo run --release -- serve

# 2. Send a request
curl -s http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "standard",
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "Hello"}
    ]
  }' | jq .
```

You will get back an OpenAI-compatible response:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "object": "chat.completion",
  "created": 1745000000,
  "model": "deepseek-chat",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Hello! How can I help you today?"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 18,
    "completion_tokens": 9,
    "total_tokens": 27
  }
}
```

---

## Configuration

### Environment Variable

```
ACCELMARS_GATEWAY_URL   Base URL of the gateway
                        Default: http://localhost:8080
```

**This is the ONE canonical env var name.** All engines use `ACCELMARS_GATEWAY_URL`. Do not use `GATEWAY_URL` or any other variant.

Example:
```bash
export ACCELMARS_GATEWAY_URL=https://gateway.accelmars.com
```

---

## Request Format

```
POST /v1/chat/completions
Content-Type: application/json
```

### Required Fields

| Field | Type | Values | Description |
|-------|------|--------|-------------|
| `model` | string | `quick` \| `standard` \| `max` \| `ultra` | Quality tier — NOT a model ID |
| `messages` | array | `[{role, content}]` | At least one message required |

**`model` takes a tier string, not a model ID.** The gateway maps tiers to actual providers via config.

### Optional Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_tokens` | integer | provider default | Maximum tokens to generate |
| `temperature` | float | provider default | Sampling temperature |
| `stream` | boolean | `false` | Stream response as SSE chunks |
| `metadata` | object | `{}` | AccelMars routing constraints (see below) |

### `metadata` — Routing Constraints

The `metadata` field is an AccelMars extension for expressing constraints orthogonal to quality tier. A standard OpenAI client omits it and the gateway applies defaults.

```json
{
  "metadata": {
    "privacy": "open",
    "latency": "normal",
    "cost": "default",
    "capabilities": [],
    "provider": null
  }
}
```

#### `privacy`

| Value | Meaning | Effect |
|-------|---------|--------|
| `open` | Any provider (default) | No filtering |
| `sensitive` | No data-residency concerns | Excludes DeepSeek |
| `private` | Self-hosted only | Restricts to `private_only` providers in config |

#### `latency`

| Value | Meaning | Effect |
|-------|---------|--------|
| `normal` | Optimize quality/cost (default) | No preference |
| `low` | Prefer fast inference (<1s) | Prefers Groq |

#### `cost`

| Value | Meaning | Effect |
|-------|---------|--------|
| `free` | Free-tier providers only | Gemini Flash-Lite, Groq |
| `budget` | Cheapest option for requested quality | Lowest cost available |
| `default` | Balanced quality/cost (default) | Per-tier default routing |
| `unlimited` | Best available, ignore cost | Highest quality available |

#### `capabilities`

Array of required capabilities. Providers that lack the capability are excluded.

| Value | Meaning |
|-------|---------|
| `reasoning` | Chain-of-thought / thinking models |
| `tool_use` | Function calling |
| `vision` | Image input |
| `code` | Code-specialized models |
| `long_context` | >100K context window |

Example: `"capabilities": ["reasoning"]` routes `max` tier to DeepSeek R1 or o3 rather than Claude.

#### `provider`

String. Explicitly names the provider to use, bypassing all tier and constraint routing.

```json
{"metadata": {"provider": "claude"}}
```

Use sparingly — prefer tiers + constraints for portability.

---

## Response Format

Standard OpenAI `chat.completion` format:

| Field | Type | Description |
|-------|------|-------------|
| `choices[0].message.content` | string | The response text |
| `choices[0].finish_reason` | string | Why the model stopped (`stop`, `length`, etc.) |
| `usage.prompt_tokens` | integer | Input tokens consumed |
| `usage.completion_tokens` | integer | Output tokens generated |
| `usage.total_tokens` | integer | Sum of input + output |
| `model` | string | Actual model ID used (not the tier) |
| `id` | string | Request UUID (used in cost ledger) |

---

## Streaming

Set `"stream": true` to receive SSE chunks:

```bash
curl -s http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "standard", "messages": [{"role": "user", "content": "Count to 3"}], "stream": true}'
```

Chunks are `data: <json>\n\n` lines. The final line is `data: [DONE]\n\n`.

**Phase 1 note:** Streaming currently delivers a single content chunk followed by the DONE sentinel. Per-token streaming is a Phase 2 feature.

---

## Error Codes

| HTTP | `error.code` | Meaning | Client Action |
|------|-------------|---------|---------------|
| 400 | `invalid_model` | `model` field is not a valid tier string | Use `quick`, `standard`, `max`, or `ultra` |
| 400 | `invalid_request` | `messages` array is empty | Add at least one message |
| 401 | `invalid_api_key` | Provider rejected the API key | Check API key environment variable on the server |
| 429 | `rate_limit_exceeded` | Provider rate limit hit | Retry with exponential backoff |
| 500 | `parse_error` | Gateway could not parse provider response | Retry; file issue if persistent |
| 502 | `provider_error` | Provider returned an error | Retry; relax constraints if persistent |
| 503 | `provider_error` | No provider available for the given constraints | Relax privacy/cost/capability constraints |
| 504 | `concurrency_timeout` | Request queued too long (>30s) | Retry later; check gateway concurrency config |
| 504 | `gateway_timeout` | Provider did not respond in time | Retry; provider may be degraded |

Error response format:
```json
{
  "error": {
    "message": "unknown model tier: 'gpt-4' — expected quick, standard, max, or ultra",
    "type": "invalid_request_error",
    "code": "invalid_model"
  }
}
```

---

## Mock Mode

Set `GATEWAY_MODE=mock` on the server. Returns deterministic responses — no API keys needed, zero cost.

```bash
GATEWAY_MODE=mock cargo run --release -- serve
```

Use mock mode for:
- Unit and integration tests
- CI pipelines without API keys
- Local development without API costs

The mock adapter returns a fixed response regardless of input. Tier and constraints are still validated.

---

## Health & Status Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Returns `{"status": "ok"}` when healthy |
| `/status` | GET | Returns version, uptime, concurrency state, provider statuses |

```bash
curl http://localhost:8080/health
curl http://localhost:8080/status
```

---

## Reference Implementations

### cortex-engine (blocking, `reqwest::blocking`)

`crates/cortex-cli/src/backends/gateway.rs` in the `cortex-engine` repo.

Key patterns:
- Maps `ModelClass::Haiku/Sonnet/Opus` to gateway tiers `quick/standard/max`
- 120s timeout via `reqwest::blocking::Client`
- Reads URL from `ACCELMARS_GATEWAY_URL` via `GatewayBackend::from_env()`
- No streaming — blocking completion only

```rust
let backend = GatewayBackend::from_env();
let response = backend.complete(&ai_request)?;
```

### guild-engine (async, `reqwest`)

`crates/guild-core/src/gateway.rs` in the `guild-engine` repo.

Key patterns:
- `GatewayClient` trait + `HttpGatewayClient` impl (ENGINE-LESSONS Rule 1: trait at boundary)
- 120s timeout via `reqwest::Client`
- Hardcodes `standard` tier — no tier selection at the call site
- `MockGatewayClient` and `FailingGatewayClient` for test isolation

```rust
let client = HttpGatewayClient::new(&gateway_url)?;
let response = client.complete(prompt, system).await?;
```

---

## Common Mistakes

1. **Missing `model` field → 400** — Guild-engine's first integration attempt omitted the `model` field from the request body. The gateway returns HTTP 400 with `"code": "invalid_model"`. The `model` field is required even if you always use the same tier.

2. **Wrong env var name** — Use `ACCELMARS_GATEWAY_URL`, not `GATEWAY_URL`. Guild-engine's test operator used `GATEWAY_URL` and the integration worked only because the gateway happened to be running at the default URL (`http://localhost:8080`). In production, always set `ACCELMARS_GATEWAY_URL` explicitly.

3. **Sending a model ID instead of a tier string** — `"model": "claude-sonnet-4-6"` returns HTTP 400. The gateway accepts `quick | standard | max | ultra` only. Model IDs are an implementation detail of each provider adapter.

4. **No error handling for 503** — If the gateway has no provider available for the requested constraints, it returns 503. Engines should either relax constraints on retry or surface a clear error to the user.

5. **Ignoring `usage` in the response** — Cost tracking in Phase 2 will rely on clients passing token usage data. Parse and log `usage.prompt_tokens` / `usage.completion_tokens` from the start.

---

_AccelMars Co., Ltd. — gateway-engine · v0.2.x_
