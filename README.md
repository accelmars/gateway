# AccelMars Gateway

> Universal AI gateway — route any AI call through one service.

[![CI](https://github.com/accelmars/gateway/actions/workflows/ci.yml/badge.svg)](https://github.com/accelmars/gateway/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

A high-performance, OpenAI-compatible AI gateway written in Rust. Every AccelMars engine routes AI calls through here — one service, multi-provider, configurable quality tiers, no provider lock-in.

---

## Architecture

```
Engine (cortex, pact, canon, ...)
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

Engines have **zero AI SDK dependencies**. They POST standard HTTP to the gateway. The gateway decides which provider to use based on tier and routing constraints.

---

## Quick Start

```bash
# Install
brew install accelmars/tap/gateway   # macOS (Homebrew)
# or: curl -sSL https://github.com/accelmars/gateway/releases/latest/install.sh | sh

# Start the gateway
gateway serve

# All engines in your project now route through it
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

Tier-to-provider mapping is config-driven — swap providers without changing engine code.

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
| **Gemini** | Phase 1 (PF-003) | Free tier available |
| **DeepSeek** | Phase 1 (PF-003) | Very cost-effective |
| **Claude (Anthropic)** | Phase 1 (PF-003) | Quality-critical work |
| **OpenAI** | Phase 1 (PF-003) | Optional |
| **Mock** | Scaffold | Tests + CI, deterministic |

---

## Mock Mode

```bash
GATEWAY_MODE=mock gateway serve
```

All engines automatically use deterministic responses. No API keys. No cost. Perfect for CI.

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

---

## Development

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

---

## Related

- [accelmars/contract-spec](https://github.com/accelmars/contract-spec) — contract specification standard
- [accelmars/pact](https://github.com/accelmars/pact) — pact contract engine

---

_AccelMars Co., Ltd. — Apache 2.0_
