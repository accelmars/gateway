# AccelMars Gateway

> Route all your AI calls through one service.

[![CI](https://github.com/accelmars/gateway/actions/workflows/ci.yml/badge.svg)](https://github.com/accelmars/gateway/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

A high-performance, OpenAI-compatible AI gateway written in Rust. One service, multi-provider, configurable quality tiers, no provider lock-in.

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

## Quick Start

```bash
# Install
brew install accelmars/tap/gateway   # macOS (Homebrew)
# or: curl -sSL https://github.com/accelmars/gateway/releases/latest/install.sh | sh

# Start the gateway
gateway serve

# Point your app at it
export ACCELMARS_GATEWAY_URL=http://localhost:8080
```

---

## Model Tiers

Express what quality level you need — the gateway picks the provider:

| Tier | Code | Default Provider | Cost |
|------|------|-----------------|------|
| **quick** | `ModelTier::Quick` | Gemini Flash-Lite | ~$0 (free) |
| **standard** | `ModelTier::Standard` | DeepSeek V3.2 | ~$0.28/M tokens |
| **max** | `ModelTier::Max` | Claude Sonnet | ~$3/M tokens |
| **ultra** | `ModelTier::Ultra` | Claude Opus | ~$5/M tokens |

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
| `latency` | `normal` / `low` | Prefer fast inference providers (Groq, Cerebras) |
| `cost` | `free` / `budget` / `default` / `unlimited` | Filter by cost tolerance |
| `capabilities` | `reasoning` / `tool_use` / `vision` / `code` / `long_context` | Route to capable providers |
| `provider` | provider name | Explicit override — bypass routing |

---

## Supported Providers

| Provider | Status | Notes |
|----------|--------|-------|
| **Gemini** | ✅ Built | Free tier available (Flash-Lite) |
| **DeepSeek** | ✅ Built | Very cost-effective (~$0.28/M) |
| **Claude (Anthropic)** | ✅ Built | Quality-critical work |
| **OpenRouter** | ✅ Built | Multi-provider proxy |
| **Groq** | ✅ Built | Low-latency inference |
| **Mock** | ✅ Built | Tests + CI, deterministic |

---

## CLI Commands

```bash
gateway serve                        # Start server (default: port 8080)
gateway serve --port 9000            # Custom port
gateway status                       # Health + provider availability
gateway stats                        # Cost summary (all time)
gateway stats --since 2026-04-01     # Filter by date
gateway stats --json                 # Machine-readable output
gateway complete "your prompt"       # One-shot completion (no server needed)
gateway complete "..." --tier max    # Override quality tier
```

---

## Mock Mode

```bash
GATEWAY_MODE=mock gateway serve
```

All clients get deterministic responses. No API keys. No cost. Perfect for CI.

---

## API

Standard OpenAI-compatible API. Any OpenAI SDK client works:

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:8080/v1",
    api_key="not-used",
)

response = client.chat.completions.create(
    model="standard",
    messages=[{"role": "user", "content": "Hello"}],
)
```

Or with curl:

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "quick", "messages": [{"role": "user", "content": "Hello"}]}'
```

---

## Development

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

---

_Built by [AccelMars](https://github.com/accelmars) — Apache 2.0_
_See also: [contract-spec](https://github.com/accelmars/contract-spec) · [pact](https://github.com/accelmars/pact)_
