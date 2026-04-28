# Python Examples — AccelMars Gateway

Six examples covering the full developer journey, each runnable against a local gateway in mock mode (zero API keys required).

## Prerequisites

- Python 3.9+
- `pip install openai pytest`
- Gateway running locally: `GATEWAY_MODE=mock gateway serve` *(or build from source: `cargo run --release -- serve`)*

By default, examples connect to `http://localhost:4000`. Override with:

```bash
export ACCELMARS_GATEWAY_URL=http://localhost:8080
```

---

## Examples

| File | What it demonstrates | Run |
|------|---------------------|-----|
| `basic.py` | Minimal chat completion — one request, one response | `python examples/python/basic.py` |
| `streaming.py` | Streaming response with per-chunk delta printing | `python examples/python/streaming.py` |
| `openai_sdk.py` | Drop-in OpenAI SDK replacement (2-line diff) | `python examples/python/openai_sdk.py` |
| `multi_tier.py` | Same prompt across quick / standard / max tiers with latency | `python examples/python/multi_tier.py` |
| `test_with_mock.py` | pytest tests against the mock gateway (CI-ready) | `pytest examples/python/test_with_mock.py -v` |
| `constraint_routing.py` | Privacy constraint routing (`sensitive_excluded` policy) | `python examples/python/constraint_routing.py` |

---

## Cassettes

Pre-recorded responses in `examples/cassettes/` match each example. Use them with the gateway's fixture adapter for deterministic CI testing without running the gateway:

```bash
GATEWAY_MODE=fixture GATEWAY_FIXTURE_FILE=examples/cassettes/basic.json \
    gateway serve
```

Cassette schema: `1` (non-streaming), `2` (streaming — `streaming.json`).

---

## Key Concepts

**Tier, not model ID.** The `model` field takes a tier name — `quick`, `standard`, `max`, `ultra` — not a provider model ID. The gateway resolves the tier to the configured provider.

**Zero keys locally.** `GATEWAY_MODE=mock` returns deterministic responses without calling any provider. No API keys needed for development or CI.

**Routing constraints.** Pass `extra_body={"metadata": {...}}` to express privacy, latency, or cost constraints. See `constraint_routing.py` and `docs/CLIENT-INTEGRATION.md`.
