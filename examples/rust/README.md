# Rust Examples — AccelMars Gateway

Two examples covering basic and streaming completions, runnable against a local gateway in mock mode (zero API keys required).

## Prerequisites

- Rust stable (`rustup default stable`)
- Gateway running locally: `GATEWAY_MODE=mock GATEWAY_AUTH_DISABLED=1 gateway serve` *(or build from source: `GATEWAY_MODE=mock GATEWAY_AUTH_DISABLED=1 cargo run --release -- serve`)*

By default, examples connect to `http://localhost:4000`. Override with:

```bash
export ACCELMARS_GATEWAY_URL=http://localhost:8080
```

---

## Examples

| Binary | What it demonstrates | Run |
|--------|---------------------|-----|
| `basic` | Minimal chat completion — one request, one response | `cargo run --bin basic` |
| `streaming` | Streaming response — SSE chunks printed as they arrive | `cargo run --bin streaming` |

---

## Running

```bash
# From this directory (examples/rust/)

# Basic completion
cargo run --bin basic

# Streaming completion
cargo run --bin streaming

# Override gateway URL
ACCELMARS_GATEWAY_URL=http://localhost:8080 cargo run --bin basic
```

---

## Cassettes

Pre-recorded responses in `../cassettes/` match each request. Use them with the gateway's fixture adapter for deterministic CI testing without running the gateway:

```bash
GATEWAY_MODE=fixture GATEWAY_FIXTURE_FILE=examples/cassettes/basic.json \
    GATEWAY_MODE=mock cargo run --release -- serve
```

---

## Key Concepts

**Tier, not model ID.** The `model` field takes `quick`, `standard`, `max`, or `ultra` — not a provider model ID. The gateway resolves the tier to the configured provider.

**Zero keys locally.** `GATEWAY_MODE=mock` returns deterministic responses without calling any provider. No API keys needed for development or CI.

**`ACCELMARS_GATEWAY_URL`.** The one canonical environment variable for all AccelMars engines. Never hardcode `localhost:4000`.
