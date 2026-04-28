# AccelMars Gateway

> Test your AI app with zero API keys — then ship with 13 providers and canary routing.

[![CI](https://github.com/accelmars/gateway/actions/workflows/ci.yml/badge.svg)](https://github.com/accelmars/gateway/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

A high-performance, OpenAI-compatible AI gateway written in Rust. Zero API keys for local development and CI. One config change to swap providers in production.

---

## 30-Second Demo

```bash
brew install accelmars/tap/gateway   # macOS
# or: curl -sSL https://github.com/accelmars/gateway/releases/latest/install.sh | sh

gateway demo                         # No API key needed
```

Cycles through quality tiers, shows streaming, prints a cost comparison — all in mock mode.

---

## Why This vs LiteLLM / OpenRouter / Portkey

| Feature | AccelMars Gateway | LiteLLM | OpenRouter | Portkey |
|---------|:-----------------:|:-------:|:----------:|:-------:|
| Zero-key local dev (mock mode) | ✅ | ❌ | ❌ | ❌ |
| Deterministic CI cassettes (record & replay) | ✅ | ❌ | ❌ | ❌ |
| Canary routing + shadow mode | ✅ | ❌ | ❌ | Partial |
| Native streaming | ✅ | ✅ | ✅ | ✅ |
| Self-hosted (single binary, no cloud dep) | ✅ | ✅ | ❌ | Partial |
| Rust-native (~8MB image, no GC pauses) | ✅ | ❌ | ❌ | ❌ |
| Quality tier abstraction (not model names) | ✅ | ❌ | ❌ | ❌ |
| OpenAI-compatible API | ✅ | ✅ | ✅ | ✅ |

---

## Architecture

```
Your application
  │
  │  POST /v1/chat/completions  {"model": "quick", ...}
  ▼
accelmars/gateway
  │
  ├── tier: quick    → Gemini Flash-Lite (free)
  ├── tier: standard → DeepSeek V3.2 (~$0.28/M)
  ├── tier: max      → Claude Sonnet (~$3/M)
  ├── tier: ultra    → Claude Opus (~$5/M)
  └── GATEWAY_MODE=mock → deterministic responses ($0)
```

Clients have **zero AI SDK dependencies**. They POST standard HTTP to the gateway. The gateway decides which provider to use based on tier and routing constraints.

---

## Model Tiers

Express what quality level you need — the gateway picks the provider:

| Tier | Default Provider | Cost |
|------|-----------------|------|
| **quick** | Gemini Flash-Lite | ~$0 (free) |
| **standard** | DeepSeek V3.2 | ~$0.28/M tokens |
| **max** | Claude Sonnet | ~$3/M tokens |
| **ultra** | Claude Opus | ~$5/M tokens |

Tier-to-provider mapping is config-driven — swap providers without changing client code.

---

## Routing Constraints

Orthogonal to quality tier — express HOW to route:

```json
{
  "model": "standard",
  "metadata": {
    "privacy": "sensitive",
    "latency": "low",
    "cost": "budget",
    "capabilities": ["reasoning"]
  }
}
```

| Constraint | Values | Effect |
|-----------|--------|--------|
| `privacy` | `open` / `sensitive` / `private` | Exclude providers with data residency concerns |
| `latency` | `normal` / `low` | Prefer fast inference providers (Groq) |
| `cost` | `free` / `budget` / `default` / `unlimited` | Filter by cost tolerance |
| `capabilities` | `reasoning` / `tool_use` / `vision` / `code` / `long_context` | Route to capable providers |
| `provider` | provider name | Explicit override — bypass routing |

---

## Supported Providers (13)

| Provider | Notes |
|----------|-------|
| **Gemini** | Free tier available (Flash-Lite); Flash for standard |
| **DeepSeek** | Very cost-effective (~$0.28/M) |
| **Claude (Anthropic)** | Quality-critical work |
| **OpenAI** | GPT-4o and variants |
| **OpenRouter** | Multi-provider proxy with model marketplace |
| **Groq** | Low-latency inference |
| **MiniMax** | Chinese LLM provider |
| **Moonshot** | Chinese LLM provider (Kimi) |
| **NVIDIA** | NIM-hosted models |
| **Qwen** | Alibaba cloud models |
| **StepFun** | Step-1 series |
| **Zhipu** | GLM series |
| **Mock** | Tests + CI, deterministic, zero cost |

---

## Examples

| Language | Example | What it shows |
|----------|---------|--------------|
| Python | [`examples/python/basic.py`](examples/python/basic.py) | One-shot completion |
| Python | [`examples/python/streaming.py`](examples/python/streaming.py) | Streaming completions |
| Python | [`examples/python/openai_sdk.py`](examples/python/openai_sdk.py) | OpenAI SDK drop-in |
| Python | [`examples/python/multi_tier.py`](examples/python/multi_tier.py) | Comparing quality tiers |
| Python | [`examples/python/test_with_mock.py`](examples/python/test_with_mock.py) | Testing without API keys |
| Python | [`examples/python/constraint_routing.py`](examples/python/constraint_routing.py) | Routing constraints |
| Rust | [`examples/rust/`](examples/rust/) | Basic + streaming |
| TypeScript | [`examples/ts/`](examples/ts/) | Basic + streaming |

---

## Docs

| Guide | What it covers |
|-------|---------------|
| [`docs/TESTING.md`](docs/TESTING.md) | Test with zero API keys — mock, fixture, and cassette workflow |
| [`docs/CASSETTE-SPEC.md`](docs/CASSETTE-SPEC.md) | Cassette format reference — record once, replay in CI |
| [`docs/MIGRATING-FROM-OPENAI.md`](docs/MIGRATING-FROM-OPENAI.md) | Swap OpenAI SDK base URL in one line |

---

## CLI

```bash
gateway serve                        # Start server (default: port 8080)
gateway serve --port 9000            # Custom port
gateway demo                         # Zero-key demo: cycles tiers, shows streaming
gateway status                       # Health + provider availability
gateway stats                        # Cost summary (all time)
gateway stats --since 2026-04-01     # Filter by date
gateway stats --json                 # Machine-readable output
gateway complete "your prompt"       # One-shot completion (no server needed)
gateway complete "..." --tier max    # Override quality tier
```

---

## Zero-Key Development and CI

**Local development — no API key required:**

```bash
GATEWAY_MODE=mock gateway serve
```

All responses are deterministic. No API keys. No cost. Perfect for development.

**CI with cassettes — record once, replay forever:**

```bash
# Record from real providers (one-time, on your machine)
GATEWAY_MODE=record GATEWAY_FIXTURE_FILE=tests/fixtures/my.json gateway serve

# Replay in CI — no API key, zero cost, milliseconds
GATEWAY_MODE=fixture GATEWAY_FIXTURE_FILE=tests/fixtures/my.json gateway serve
```

See [`docs/TESTING.md`](docs/TESTING.md) for the full workflow.

---

## OpenAI-Compatible API

Any OpenAI SDK works — no new dependency needed:

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:8080/v1",
    api_key="not-used",
)

response = client.chat.completions.create(
    model="standard",    # quality tier, not a model name
    messages=[{"role": "user", "content": "Hello"}],
)
```

Or with curl:

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "quick", "messages": [{"role": "user", "content": "Hello"}]}'
```

See [`docs/MIGRATING-FROM-OPENAI.md`](docs/MIGRATING-FROM-OPENAI.md) for the one-step migration guide.

---

## Development

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

---

[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [Apache 2.0](LICENSE)

_Built by [AccelMars](https://github.com/accelmars) — See also: [contract-spec](https://github.com/accelmars/contract-spec) · [pact](https://github.com/accelmars/pact)_
