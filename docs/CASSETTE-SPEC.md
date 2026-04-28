# Cassette Specification

> Deterministic AI response replay for gateway. Record once, replay everywhere — no API keys required.

---

## Overview

A **cassette** is a JSON file containing pre-recorded gateway request/response pairs. The `FixtureAdapter` replays them in place of real AI providers. This means:

- **CI/CD pipelines** run with zero API keys and produce identical results on every run
- **Offline development** works without internet access or provider credentials
- **Deterministic testing** eliminates provider flakiness — the exact response is always returned
- **Full middleware coverage** — auth, routing, cost tracking, and telemetry all execute normally against fixture responses

Cassettes are authored once (manually or by recording from a live provider) and committed to source control as permanent test artifacts.

---

## Activation

Gateway reads two environment variables to enter fixture mode:

| Variable | Required | Description |
|----------|----------|-------------|
| `GATEWAY_MODE=fixture` | Yes | Activates fixture mode — all tier routing resolves to the `FixtureAdapter` |
| `GATEWAY_FIXTURE_FILE` | Yes | Absolute or relative path to the cassette JSON file |

```bash
GATEWAY_MODE=fixture \
GATEWAY_FIXTURE_FILE=tests/fixtures/my-cassette.json \
gateway serve
```

In fixture mode:
- `FixtureAdapter` is loaded from `GATEWAY_FIXTURE_FILE` and registered as the provider for all tiers
- Config validation skips provider API key checks — gateway starts cleanly with zero keys
- All other middleware (auth, cost, telemetry) runs normally

Gateway will fail at startup if `GATEWAY_MODE=fixture` is set but `GATEWAY_FIXTURE_FILE` is missing or the file does not exist.

---

## File Format

Cassettes are UTF-8 JSON files. One file = one cassette. There is no file-naming constraint enforced by the adapter, but the convention is:

```
tests/fixtures/{provider}-{scenario}.json
```

Examples: `tests/fixtures/deepseek-synthesis.json`, `tests/fixtures/openai-error-recovery.json`.

---

## Schema v1 (current)

### Top-level Cassette Object

```json
{
  "schema_version": "1",
  "provider": "deepseek",
  "recorded_at": "2026-04-26T12:00:00Z",
  "entries": [ ... ]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `schema_version` | String | Yes | `"1"` for non-streaming cassettes. MUST be `"2"` if any entry uses `streaming_success`. |
| `provider` | String | Yes | Provider name. Informational — not enforced at replay. Use gateway provider key (`"deepseek"`, `"gemini"`, `"claude"`, `"openrouter"`, `"groq"`) or `"mock"`. |
| `recorded_at` | String | Yes | ISO 8601 UTC timestamp of recording session. Format: `"2026-04-26T12:00:00Z"`. |
| `entries` | Array | Yes | Ordered list of `CassetteEntry` objects. May be empty. |

**Validation rules enforced at load time:**

| Rule | Condition | Result |
|------|-----------|--------|
| V-70 | `schema_version: "1"` cassette contains a `streaming_success` entry | Load error — cassette rejected |
| V-72 | `schema_version` is not `"1"` or `"2"` | Load error — cassette rejected |

### CassetteEntry

```json
{
  "request": { ... },
  "response": { ... },
  "match_key": { ... }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `request` | GatewayRequest | Yes | The stored request. Used for documentation and optional keyed matching. In sequential mode, ignored at replay. |
| `response` | CassetteResponse | Yes | The response replayed when this entry is selected. |
| `match_key` | EntryMatcher | No | When present, enables content-based lookup. When absent, entry joins the sequential fallback pool. |

#### GatewayRequest Fields

```json
{
  "tier": "standard",
  "messages": [
    { "role": "user", "content": "What is the capital of France?" }
  ],
  "max_tokens": null,
  "stream": false,
  "constraints": {},
  "metadata": {}
}
```

| Field | Type | Required | Values |
|-------|------|----------|--------|
| `tier` | String | Yes | `"quick"` \| `"standard"` \| `"max"` \| `"ultra"` |
| `messages` | Array | Yes | `[{ "role": String, "content": String }]`. Roles: `"user"`, `"assistant"`, `"system"` |
| `max_tokens` | Integer or null | Yes | Token cap. `null` = provider default |
| `stream` | Boolean | Yes | Whether streaming was requested |
| `constraints` | Object | Yes | Routing constraints. May be `{}` |
| `metadata` | Object | Yes | Arbitrary key-value pairs. May be `{}` |

### CassetteResponse — `success`

```json
{
  "type": "success",
  "id": "chatcmpl-abc123",
  "model": "deepseek-chat",
  "content": "The capital of France is Paris.",
  "tokens_in": 10,
  "tokens_out": 8,
  "finish_reason": "stop"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | `"success"` | Yes | Discriminator |
| `id` | String | Yes | Response ID. Use provider's actual ID or `"fixture-N"` for hand-authored entries. |
| `model` | String | Yes | Model name (e.g., `"deepseek-chat"`, `"gemini-2.5-flash-lite"`) |
| `content` | String | Yes | Full response text |
| `tokens_in` | Integer (u32) | Yes | Input token count |
| `tokens_out` | Integer (u32) | Yes | Output token count |
| `finish_reason` | String | Yes | `"stop"` \| `"length"` \| provider-specific value |

All six non-type fields are REQUIRED. A `success` entry missing any field is invalid.

### CassetteResponse — `error`

```json
{
  "type": "error",
  "kind": "rate_limit",
  "message": "Rate limit exceeded. Retry after 60 seconds.",
  "retry_after_ms": 60000
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | `"error"` | Yes | Discriminator |
| `kind` | String | Yes | One of five error kinds — see table below |
| `message` | String | Yes | Human-readable description |
| `retry_after_ms` | Integer (u64) or null | Yes | Retry hint in milliseconds. `null` if not applicable. |

#### ErrorKind Values

| JSON value | HTTP status | Use for |
|-----------|-------------|---------|
| `"rate_limit"` | 429 | Fallback chain activation, retry logic |
| `"auth_error"` | 401 | Key revocation, invalid credentials |
| `"timeout"` | 504 | Circuit breaker tripping, slow provider |
| `"provider_error"` | 502 | Provider 500s, malformed upstream responses |
| `"parse_error"` | 500 | Unexpected response format from provider |

---

## Schema v2 (streaming)

Schema v2 adds one new response variant: `streaming_success`. All schema v1 entry types (`success`, `error`) remain valid in v2 cassettes. Use `schema_version: "2"` in any cassette that contains at least one `streaming_success` entry.

### CassetteResponse — `streaming_success`

```json
{
  "type": "streaming_success",
  "id": "chatcmpl-xyz789",
  "model": "deepseek-chat",
  "chunks": ["The capital", " of France", " is Paris."],
  "tokens_in": 10,
  "tokens_out": 8,
  "finish_reason": "stop"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | `"streaming_success"` | Yes | Discriminator |
| `id` | String | Yes | Response ID |
| `model` | String | Yes | Model name |
| `chunks` | Array of String | Yes | One string per SSE content delta. The server emits: role delta → N content deltas (one per element) → finish delta → `[DONE]` |
| `tokens_in` | Integer (u32) | Yes | Input token count |
| `tokens_out` | Integer (u32) | Yes | Output token count |
| `finish_reason` | String | Yes | `"stop"` \| `"length"` \| provider-specific value |

`FixtureAdapter::complete_chunks()` is overridden to return these chunks as-is in a `ChunkedResponse`. A `success` entry served through `complete_chunks()` is wrapped in a single-element `Vec` (one chunk = full content).

---

## Request Key Derivation

By default, `FixtureAdapter` operates in **sequential mode**: entries are consumed FIFO, and the incoming request is completely ignored. The adapter pops the next entry from the front of the queue regardless of what was sent.

**Keyed matching** is enabled by adding a `match_key` field to a `CassetteEntry`. When a `match_key` is present, that entry participates in content-based lookup instead of sequential position.

### EntryMatcher Fields

```json
{
  "match_key": {
    "tier": "standard",
    "last_message_contains": "capital of France",
    "message_count": 1
  }
}
```

| Field | Type | When absent |
|-------|------|-------------|
| `tier` | String | Matches any tier |
| `last_message_contains` | String | Matches any last message content |
| `message_count` | Integer | Matches any message count |

All fields are optional. Matching is case-insensitive for string fields. All specified fields must match (AND logic).

### What Is Included in Key Matching

| Dimension | Matched? | Notes |
|-----------|----------|-------|
| Request tier | Yes (if `tier` specified) | Case-insensitive |
| Last message content | Yes (if `last_message_contains` specified) | Substring match, case-insensitive |
| Message count | Yes (if `message_count` specified) | Exact count |
| Timestamps | No | Never included |
| Request IDs | No | Never included |
| `max_tokens` | No | Not a match dimension |
| `constraints` / `metadata` | No | Not match dimensions |

---

## Keyed Matching Rules

The matching algorithm runs on every `complete()` (or `complete_chunks()`) call:

```
1. KEYED LOOKUP
   Scan entries left-to-right for the first entry where:
     - match_key is present AND
     - all specified match_key fields satisfy the incoming request
   If found: remove that entry, return its response.

2. SEQUENTIAL FALLBACK
   Find the first entry where match_key is absent.
   If found: remove that entry, return its response.

3. EXHAUSTION
   Return error: "cassette exhausted — no matching entry"
```

Key properties:
- **Keyed entries take priority** over sequential entries regardless of position. A keyed entry at the end of the array is found before a sequential entry at the beginning.
- **First keyed match wins** — scan is left-to-right.
- **Entries are consumed** — each call removes the matched entry. It cannot be returned again.
- **Empty `match_key` (`{}`)** — an `EntryMatcher` with no fields specified matches any request. This is valid but unusual; use a sequential entry (no `match_key`) for generic fallback instead.

No-match behavior: when all keyed entries fail to match and no sequential fallback exists, gateway returns HTTP 502 with the exhaustion error.

---

## Streaming Cassettes

For streaming responses, use `streaming_success` entries with `schema_version: "2"`. The `chunks` array maps directly to SSE content deltas:

```
SSE wire format for a streaming_success entry:
  data: {"type":"content_block_delta","delta":{"type":"text_delta","text":""}}  ← role delta
  data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"The capital"}}
  data: {"type":"content_block_delta","delta":{"type":"text_delta","text":" of France"}}
  data: {"type":"content_block_delta","delta":{"type":"text_delta","text":" is Paris."}}
  data: {"type":"message_delta","delta":{"stop_reason":"stop"}}  ← finish delta
  data: [DONE]
```

One `chunks` element = one SSE content delta. The role delta and finish delta are synthesized by the server.

---

## Annotated Example: Non-Streaming Cassette

```json
{
  "schema_version": "1",
  "provider": "deepseek",
  "recorded_at": "2026-04-26T12:00:00Z",
  "entries": [
    {
      "request": {
        "tier": "standard",
        "messages": [
          { "role": "user", "content": "Summarize the quarterly report" }
        ],
        "max_tokens": 500,
        "stream": false,
        "constraints": {},
        "metadata": {}
      },
      "response": {
        "type": "success",
        "id": "fixture-1",
        "model": "deepseek-chat",
        "content": "Q1 2026: Revenue up 12%, operating costs stable at $2.1M, net margin improved to 18%.",
        "tokens_in": 12,
        "tokens_out": 28,
        "finish_reason": "stop"
      },
      "match_key": {
        "last_message_contains": "quarterly report",
        "tier": "standard"
      }
    },
    {
      "request": {
        "tier": "quick",
        "messages": [
          { "role": "user", "content": "Classify this as urgent or routine" }
        ],
        "max_tokens": null,
        "stream": false,
        "constraints": {},
        "metadata": {}
      },
      "response": {
        "type": "error",
        "kind": "rate_limit",
        "message": "Too many requests. Retry after 30 seconds.",
        "retry_after_ms": 30000
      }
    }
  ]
}
```

This cassette has two entries:
1. A keyed entry that matches `standard` tier requests containing "quarterly report" in the last message.
2. A sequential fallback entry (no `match_key`) that returns a rate-limit error for any request that doesn't match entry 1.

---

## Annotated Example: Streaming Cassette

```json
{
  "schema_version": "2",
  "provider": "openai",
  "recorded_at": "2026-04-26T15:30:00Z",
  "entries": [
    {
      "request": {
        "tier": "max",
        "messages": [
          { "role": "system", "content": "You are a concise technical writer." },
          { "role": "user", "content": "Explain what a cassette is in one sentence." }
        ],
        "max_tokens": 100,
        "stream": true,
        "constraints": {},
        "metadata": {}
      },
      "response": {
        "type": "streaming_success",
        "id": "chatcmpl-stream-001",
        "model": "gpt-4o",
        "chunks": [
          "A cassette",
          " is a pre-recorded JSON file",
          " that gateway replays",
          " in place of real AI providers."
        ],
        "tokens_in": 25,
        "tokens_out": 17,
        "finish_reason": "stop"
      },
      "match_key": {
        "tier": "max",
        "message_count": 2
      }
    }
  ]
}
```

`schema_version: "2"` is required because the entry uses `streaming_success`. The `match_key` matches `max` tier requests with exactly 2 messages.

---

## Authoring Cassettes

### Manual Authoring

Minimum required fields for a valid cassette:

```json
{
  "schema_version": "1",
  "provider": "mock",
  "recorded_at": "2026-04-26T00:00:00Z",
  "entries": [
    {
      "request": {
        "tier": "standard",
        "messages": [{ "role": "user", "content": "hello" }],
        "max_tokens": null,
        "stream": false,
        "constraints": {},
        "metadata": {}
      },
      "response": {
        "type": "success",
        "id": "fixture-1",
        "model": "mock",
        "content": "Hello from fixture.",
        "tokens_in": 5,
        "tokens_out": 5,
        "finish_reason": "stop"
      }
    }
  ]
}
```

Conventions:
- Use `"provider": "mock"` for hand-authored cassettes not recorded from a real provider
- Use `"id": "fixture-N"` (e.g., `"fixture-1"`, `"fixture-2"`) for hand-authored response IDs
- Set `stream: true` in the request field when the cassette is intended for streaming replay

### Via RecordingAdapter

To capture real provider responses, use `RecordingAdapter` (requires `--features record-fixtures`):

```rust
#[test]
#[ignore]  // run with: cargo test --features record-fixtures -- --ignored record_deepseek
fn record_deepseek() {
    let api_key = std::env::var("DEEPSEEK_API_KEY").expect("required for recording");
    let real_adapter = new_deepseek_adapter(Some(api_key));
    let recording = RecordingAdapter::new(real_adapter);

    // Make the calls your engine will make in production
    let _ = recording.complete(&my_request).expect("recording failed");

    // Save the cassette
    recording.save_to_file(
        Path::new("tests/fixtures/deepseek-synthesis.json")
    ).expect("failed to save");
}
```

Run with:
```bash
DEEPSEEK_API_KEY=sk-... cargo test --features record-fixtures -- --ignored record_deepseek
```

Review the cassette before committing:
- Verify response content is correct
- Ensure no secrets, PII, or sensitive data appear in `request.messages`
- Check token counts and `finish_reason` are populated

Commit the cassette as source code. It is a permanent artifact — treat it like a test fixture in any other language.

---

## Versioning

### Schema Version Policy

| Change | Bumps `schema_version`? |
|--------|------------------------|
| New optional field with `#[serde(default)]` | No |
| New `CassetteResponse` variant | **Yes** — older parsers fail on unknown variants |
| Rename of existing field | **Yes** — breaking change |
| New required field | **Yes** — older cassettes missing the field would fail |
| New `ErrorKind` value | **Yes** — semantic change |

### Compatibility Guarantees

- Schema `"1"` cassettes are valid in any gateway version that supports schema `"1"` or `"2"`.
- Schema `"2"` cassettes require a gateway version that supports schema `"2"`.
- Parsers MUST reject cassettes with unknown `schema_version` values (rule V-72).
- Parsers MUST reject schema `"1"` cassettes containing `streaming_success` entries (rule V-70).
- Mixing `success`, `error`, and `streaming_success` entries in a single schema `"2"` cassette is allowed.

### Version History

| Schema | Introduced | Status | What it adds |
|--------|-----------|--------|-------------|
| `"1"` | GI-001 (2026-04-26) | Current | `success`, `error` variants; sequential replay; `EntryMatcher` keyed matching |
| `"2"` | GI-016b (2026-04-26) | Current | `streaming_success` variant; multi-chunk SSE replay |

---

_AccelMars Co., Ltd. — gateway-public-dx_
