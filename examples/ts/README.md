# TypeScript Examples — AccelMars Gateway

Two examples covering the core developer journey, each runnable against a local gateway in mock mode (zero API keys required).

## Prerequisites

- Node.js 18+
- `npm install` (installs `openai` and `tsx`)
- Gateway running locally: `GATEWAY_MODE=mock gateway serve` *(or build from source: `GATEWAY_MODE=mock cargo run --release -- serve`)*

By default, examples connect to `http://localhost:4000`. Override with:

```bash
export ACCELMARS_GATEWAY_URL=http://localhost:8080
```

> **Node.js IPv6 note:** On systems where `localhost` resolves to `::1` (IPv6) and the gateway binds IPv4 only, use `http://127.0.0.1:4000` explicitly:
> ```bash
> export ACCELMARS_GATEWAY_URL=http://127.0.0.1:4000
> ```

---

## Examples

| File | What it demonstrates | Run |
|------|---------------------|-----|
| `basic.ts` | Minimal chat completion — one request, one response | `npx tsx basic.ts` |
| `streaming.ts` | Streaming response with per-chunk delta printing | `npx tsx streaming.ts` |

---

## Quick Start

```bash
# 1. Install dependencies
npm install

# 2. Start gateway in mock mode (no API keys needed)
GATEWAY_MODE=mock gateway serve

# 3. Run basic example
npx tsx basic.ts

# 4. Run streaming example
npx tsx streaming.ts
```

---

## Key Concepts

**Tier, not model ID.** The `model` field takes a tier name — `quick`, `standard`, `max`, `ultra` — not a provider model ID. The gateway resolves the tier to the configured provider.

**Zero keys locally.** `GATEWAY_MODE=mock` returns deterministic responses without calling any provider. No API keys needed for development or CI.

**Drop-in OpenAI replacement.** Set `baseURL` on the OpenAI client and point it at the gateway. No other code changes required.

**Routing constraints.** Pass `extra_body: { metadata: { ... } }` to express privacy, latency, or cost constraints. See `docs/CLIENT-INTEGRATION.md`.
