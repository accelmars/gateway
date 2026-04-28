# Testing Your AI App with AccelMars Gateway

> No API keys required in development or CI. Record once, replay everywhere.

---

## Overview

Gateway ships three testing modes. Together they eliminate every reason you'd need a real AI provider until you're ready to ship.

| Mode | Env-var | Needs cassette? | Needs running gateway? | When to use |
|------|---------|-----------------|----------------------|-------------|
| **Mock** | `GATEWAY_MODE=mock` | No | Yes | Unit tests, smoke tests, local dev iteration |
| **Fixture** | `GATEWAY_MODE=fixture` | Yes | Yes | Integration tests, CI — realistic responses, no API keys |
| **Record** | _(feature flag)_ | No — creates one | Yes (real provider) | Capturing cassettes from live providers |

Start in mock mode. Move to fixture mode for integration tests. Record cassettes once, commit them, and replay forever.

---

## Mock Mode — Instant Deterministic Responses

Mock mode routes every request to `MockAdapter`, a built-in adapter that generates responses locally with zero network calls. The full gateway stack still runs — HTTP server, auth, routing, cost tracking — only the provider HTTP call is replaced.

### Activation

```bash
GATEWAY_MODE=mock gateway serve
```

Config file equivalent:
```toml
# gateway.toml
mode = "mock"
```

### What It Returns

| Field | Value |
|-------|-------|
| `content` | `"Mock response from AccelMars gateway."` (configurable) |
| `model` | `"mock"` |
| `tokens_in` | `total_input_chars / 4` — character heuristic |
| `tokens_out` | `response_chars / 4` — character heuristic |
| `finish_reason` | `"stop"` |

Every response is instantaneous. Cost tracking records `$0`. All tiers work. Auth middleware still runs — you still need a gateway API key (or disable auth with `GATEWAY_AUTH_DISABLED=1` in dev).

### Python Example

```python
import os
import pytest
from openai import OpenAI


@pytest.fixture
def gateway_client():
    # Start gateway with: GATEWAY_MODE=mock GATEWAY_AUTH_DISABLED=1 gateway serve
    return OpenAI(
        base_url=os.environ.get("GATEWAY_URL", "http://localhost:8080") + "/v1",
        api_key="not-needed",
    )


def test_classify_urgent(gateway_client):
    response = gateway_client.chat.completions.create(
        model="quick",
        messages=[{"role": "user", "content": "Classify this ticket as urgent or routine."}],
    )
    assert response.choices[0].finish_reason == "stop"
    # mock returns deterministic content — assert shape, not exact content
    assert isinstance(response.choices[0].message.content, str)
```

### When to Use

Use mock mode when you need to verify your app's request shape, auth wiring, or gateway config — not when you need to test how your app handles specific AI responses. For that, use fixture mode.

---

## Fixture Mode — Cassette Playback

Fixture mode replaces the AI provider with pre-recorded cassette files. The gateway plays back the cassette entry that matches each incoming request. Response content is realistic and provider-specific — exactly what the real provider returned when the cassette was recorded.

### Activation

```bash
GATEWAY_MODE=fixture \
GATEWAY_FIXTURE_FILE=tests/fixtures/my-cassette.json \
gateway serve
```

Both env-vars are required. Gateway fails at startup if `GATEWAY_FIXTURE_FILE` is missing or the file doesn't exist.

In fixture mode:
- `FixtureAdapter` is loaded from `GATEWAY_FIXTURE_FILE` and handles all tiers
- Provider API key validation is skipped — gateway starts with zero API keys
- Auth and all other middleware run normally

### What Cassettes Are

A cassette is a JSON file containing pre-recorded request/response pairs. See [CASSETTE-SPEC.md](CASSETTE-SPEC.md) for the full schema. A minimal cassette looks like this:

```json
{
  "schema_version": "1",
  "provider": "deepseek",
  "recorded_at": "2026-04-26T00:00:00Z",
  "entries": [
    {
      "request": {
        "tier": "standard",
        "messages": [{"role": "user", "content": "Summarize this report."}],
        "max_tokens": null,
        "stream": false,
        "constraints": {},
        "metadata": {}
      },
      "response": {
        "type": "success",
        "id": "fixture-1",
        "model": "deepseek-chat",
        "content": "Q1 2026: Revenue up 12%, margins stable at 18%.",
        "tokens_in": 8,
        "tokens_out": 14,
        "finish_reason": "stop"
      }
    }
  ]
}
```

By default, `FixtureAdapter` consumes entries in order (FIFO). For multi-step or non-deterministic call patterns, add `match_key` to entries to enable content-based matching:

```json
{
  "match_key": {
    "tier": "standard",
    "last_message_contains": "summarize"
  }
}
```

See [CASSETTE-SPEC.md §Request Key Derivation](CASSETTE-SPEC.md#request-key-derivation) for the full matching algorithm.

### CI Integration

The complete GitHub Actions workflow for fixture-mode CI testing:

```yaml
# .github/workflows/ai-tests.yml
name: AI integration tests

on: [push, pull_request]

jobs:
  ai-tests:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Build gateway
        run: cargo build --release --manifest-path gateway/Cargo.toml
        # Or: download a pre-built binary from GitHub Releases

      - name: Start gateway in fixture mode
        run: |
          GATEWAY_MODE=fixture \
          GATEWAY_FIXTURE_FILE=${{ github.workspace }}/tests/fixtures/deepseek-synthesis.json \
          GATEWAY_AUTH_DISABLED=1 \
          ./gateway/target/release/gateway serve &

          # Wait until gateway is ready
          for i in $(seq 1 10); do
            curl -sf http://localhost:8080/health && break
            sleep 1
          done

      - name: Run tests
        env:
          GATEWAY_URL: http://localhost:8080
        run: |
          # Python
          pip install -r requirements-test.txt
          pytest tests/integration/

          # Or Rust
          # cargo test --features integration

      - name: Stop gateway
        if: always()
        run: pkill -f "gateway serve" || true
```

No API key secrets needed. The cassette is committed to the repo — the same cassette plays on every CI run, producing identical results.

### Python pytest Example

```python
import os
import subprocess
import time
import pytest
import httpx


@pytest.fixture(scope="session")
def gateway(tmp_path_factory):
    """Start gateway in fixture mode for the test session."""
    cassette = "tests/fixtures/deepseek-synthesis.json"
    proc = subprocess.Popen(
        ["gateway", "serve"],
        env={
            **os.environ,
            "GATEWAY_MODE": "fixture",
            "GATEWAY_FIXTURE_FILE": cassette,
            "GATEWAY_AUTH_DISABLED": "1",
        },
    )

    # Wait for ready
    for _ in range(10):
        try:
            httpx.get("http://localhost:8080/health", timeout=1).raise_for_status()
            break
        except Exception:
            time.sleep(1)

    yield "http://localhost:8080"
    proc.terminate()


def test_synthesis_response(gateway):
    from openai import OpenAI

    client = OpenAI(base_url=f"{gateway}/v1", api_key="not-needed")
    response = client.chat.completions.create(
        model="standard",
        messages=[{"role": "user", "content": "Summarize this report."}],
    )
    # Fixture returns the recorded content exactly
    assert "Revenue" in response.choices[0].message.content
    assert response.choices[0].finish_reason == "stop"
```

### When to Use

Use fixture mode for any test that needs to assert on AI response content, multi-turn conversations, error handling, or cost tracking behavior with realistic provider responses. It is the primary tool for engine integration tests.

---

## Record Mode — Capturing Real Responses

Record mode captures real provider responses into cassette files. You record once (with a real API key), commit the cassette, and all future CI runs replay from the file.

Recording uses `RecordingAdapter` — a feature-gated Rust adapter that wraps a real provider, passes every request through, and captures the response.

### When to Record

Record a new cassette when:
- Starting a new feature that makes a new category of AI call
- A cassette goes stale after a provider updates its response format
- Adding test coverage for a new error path

Do NOT re-record when the existing cassette covers the scenario correctly.

### Recording Workflow

**Step 1 — Set your API key:**

```bash
export DEEPSEEK_API_KEY=sk-...
```

**Step 2 — Write a recording test** (in your Rust engine):

```rust
#[test]
#[ignore]  // run with: cargo test --features record-fixtures -- --ignored record_synthesis
fn record_synthesis() {
    let api_key = std::env::var("DEEPSEEK_API_KEY")
        .expect("DEEPSEEK_API_KEY required for recording");

    let real_adapter = new_deepseek_adapter(Some(api_key));
    let recording = RecordingAdapter::new(real_adapter);

    let request = GatewayRequest {
        tier: ModelTier::Standard,
        messages: vec![
            Message { role: "user".into(), content: "Summarize this report: ...".into() }
        ],
        max_tokens: Some(500),
        stream: false,
        constraints: RoutingConstraints::default(),
        metadata: HashMap::new(),
    };

    let _ = recording.complete(&request).expect("recording failed");

    recording
        .save_to_file(Path::new("tests/fixtures/deepseek-synthesis.json"))
        .expect("failed to save cassette");
}
```

**Step 3 — Run the recording test:**

```bash
cargo test --features record-fixtures -- --ignored record_synthesis
```

This creates `tests/fixtures/deepseek-synthesis.json`.

**Step 4 — Review the cassette:**

Open the JSON file and verify:
- Response content looks correct
- Token counts are populated
- `finish_reason` is `"stop"` (or as expected)
- No secrets, PII, or sensitive data in `request.messages`

**Step 5 — Commit the cassette:**

```bash
git add tests/fixtures/deepseek-synthesis.json
git commit -m "chore: add deepseek synthesis cassette"
```

The cassette is now a permanent artifact. CI replays it without any API keys.

**Step 6 — Switch from RecordingAdapter to FixtureAdapter:**

```rust
let adapter = FixtureAdapter::from_file(
    "deepseek",
    Path::new("tests/fixtures/deepseek-synthesis.json"),
)?;
// Same call pattern — replays from cassette now
let response = adapter.complete(&request)?;
```

Or use the external server approach for HTTP-level tests:

```bash
GATEWAY_MODE=fixture \
GATEWAY_FIXTURE_FILE=tests/fixtures/deepseek-synthesis.json \
gateway serve
```

### Where Files Land

By convention: `tests/fixtures/{provider}-{scenario}.json`

Examples:
- `tests/fixtures/deepseek-synthesis.json`
- `tests/fixtures/openai-error-recovery.json`
- `tests/fixtures/gemini-streaming.json`

---

## Cassette Authoring

### Manual Authoring

For simple test scenarios, write cassettes by hand. See [CASSETTE-SPEC.md §Authoring Cassettes](CASSETTE-SPEC.md#authoring-cassettes) for the minimum required fields.

Conventions:
- Use `"provider": "mock"` for hand-authored cassettes not recorded from a real provider
- Use `"id": "fixture-N"` (e.g., `"fixture-1"`, `"fixture-2"`) for hand-authored response IDs
- Use one cassette per scenario — do not combine unrelated call patterns in one file
- For streaming cassettes, use `schema_version: "2"` and `"type": "streaming_success"` entries

### Via RecordingAdapter

See [Record Mode](#record-mode--capturing-real-responses) above.

### Versioning

| Schema | What it covers |
|--------|---------------|
| `"1"` | `success` and `error` entries; sequential replay and keyed matching |
| `"2"` | All of v1 + `streaming_success` entries for SSE replay |

Use `schema_version: "2"` in any cassette that contains at least one `streaming_success` entry. See [CASSETTE-SPEC.md §Versioning](CASSETTE-SPEC.md#versioning) for the full policy.

---

## Test Isolation

In fixture mode, the cassette is loaded once and entries are consumed in order. If multiple tests share a gateway instance, they share the cassette — test ordering matters. The recommended pattern: one cassette per test, started fresh.

### Python: Per-Test Cassette with tmpdir

```python
import os
import shutil
import subprocess
import time
import pytest
import httpx


def _start_gateway(cassette_path: str) -> tuple[subprocess.Popen, str]:
    proc = subprocess.Popen(
        ["gateway", "serve"],
        env={
            **os.environ,
            "GATEWAY_MODE": "fixture",
            "GATEWAY_FIXTURE_FILE": cassette_path,
            "GATEWAY_AUTH_DISABLED": "1",
            # Use a random port per test to avoid collisions
            "GATEWAY_PORT": "0",  # or pick a fixed free port
        },
    )
    url = "http://localhost:8080"
    for _ in range(10):
        try:
            httpx.get(f"{url}/health", timeout=1).raise_for_status()
            return proc, url
        except Exception:
            time.sleep(1)
    proc.terminate()
    raise RuntimeError("Gateway failed to start")


@pytest.fixture
def synthesis_gateway(tmp_path):
    # Copy cassette to tmpdir so this test gets its own entry queue
    src = "tests/fixtures/deepseek-synthesis.json"
    dst = tmp_path / "cassette.json"
    shutil.copy(src, dst)

    proc, url = _start_gateway(str(dst))
    yield url
    proc.terminate()


def test_synthesis(synthesis_gateway):
    from openai import OpenAI

    client = OpenAI(base_url=f"{synthesis_gateway}/v1", api_key="not-needed")
    response = client.chat.completions.create(
        model="standard",
        messages=[{"role": "user", "content": "Summarize this report."}],
    )
    assert response.choices[0].finish_reason == "stop"
```

**Why copy to tmpdir?** The cassette file is consumed as entries are read. Copying it means the test starts with a fresh queue and the source fixture file stays unchanged for other tests.

For simpler setups where tests run serially, a session-scoped gateway with a single multi-entry cassette is fine — just ensure the cassette has enough entries for all tests in the session.

---

## Further Reading

| Topic | Document |
|-------|----------|
| Cassette JSON schema | [CASSETTE-SPEC.md](CASSETTE-SPEC.md) |
| Error simulation (rate limit, timeout, auth) | [CASSETTE-SPEC.md §CassetteResponse — error](CASSETTE-SPEC.md#cassette-response--error) |
| Streaming cassettes | [CASSETTE-SPEC.md §Schema v2](CASSETTE-SPEC.md#schema-v2-streaming) |
| Migrating from OpenAI SDK | [MIGRATING-FROM-OPENAI.md](MIGRATING-FROM-OPENAI.md) |

---

_AccelMars Co., Ltd. — gateway-public-dx_
