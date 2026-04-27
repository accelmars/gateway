# Changelog

All notable changes to this project will be documented in this file.

## [0.3.0] - 2026-04-27

### Features

- Cassette recording and replay — integration tests replay provider responses from fixture files without real API keys ([#21](https://github.com/accelmars/gateway/pull/21))
- Agent-consumable CLI — `gateway status` exit codes (0/1/2), `--json` mode, PID-file port discovery, per-request routing constraint flags on `complete`, and `gateway stop` ([#22](https://github.com/accelmars/gateway/pull/22))
- API key authentication — Bearer token validation, SHA-256 hashed SQLite key store, and `gateway keys create/list/revoke` CLI commands ([#24](https://github.com/accelmars/gateway/pull/24))
- Fly.io cloud deployment — persistent SQLite volume, auto-stop on idle, health checks on `/health`, and GitHub Actions auto-deploy on merge to main ([#25](https://github.com/accelmars/gateway/pull/25))
- Circuit breaker and fallback retry — CLOSED/OPEN/HALF-OPEN state machine with 3-failure threshold, 30-second recovery window, and server-side retry across the fallback chain; circuit state visible on `/status` ([#27](https://github.com/accelmars/gateway/pull/27))
- OpenTelemetry observability — GenAI semantic convention spans on every request, conditional OTLP export to Grafana Cloud, fail-open guarantee, and gateway overhead measurement ([#28](https://github.com/accelmars/gateway/pull/28))
- CLI intent-oriented UX — intent-aware error messages, fuzzy "Did you mean?" suggestions, `NO_COLOR` / `--no-color` support, and confirmation prompts across all 8 gateway command surfaces; 29 UX violations resolved ([#32](https://github.com/accelmars/gateway/pull/32))
- `GATEWAY_MODE=fixture` activation — load cassette files via `GATEWAY_FIXTURE_FILE` env var; engine developers replay recorded responses without modifying Rust source ([#33](https://github.com/accelmars/gateway/pull/33))
- Streaming SSE test coverage — fixture-backed server correctly serves SSE chunks and HTTP errors for `stream:true` requests ([#35](https://github.com/accelmars/gateway/pull/35))

### Bug Fixes

- OTLP traces not exported in deployed environments — explicitly reads `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_EXPORTER_OTLP_HEADERS`; sdk 0.31 does not auto-read these env vars ([#29](https://github.com/accelmars/gateway/pull/29))

## [0.2.0] - 2026-04-20

### Features

- OpenAI-compatible HTTP server — `gateway serve` exposes `POST /v1/chat/completions`; any OpenAI SDK client connects without modification ([#8](https://github.com/accelmars/gateway/pull/8))
- Five built-in provider adapters — Gemini, DeepSeek, Claude API, OpenRouter, and Groq; swap providers without changing application code ([#9](https://github.com/accelmars/gateway/pull/9))
- Config-driven routing — four quality tiers (quick/standard/max/ultra), per-provider constraints, health tracking, and automatic fallback chains ([#10](https://github.com/accelmars/gateway/pull/10))
- Concurrency control, cost tracking, and CLI observability — global request semaphore, per-request cost logged to SQLite, `gateway status/stats/complete` commands ([#11](https://github.com/accelmars/gateway/pull/11))

### Bug Fixes

- Correct cliff.toml link template for git-cliff 2.x ([#14](https://github.com/accelmars/gateway/pull/14))

## [0.1.0] - 2026-04-19

### Features

- Initial workspace scaffold with core types, CI, and release automation ([#1](https://github.com/accelmars/gateway/pull/1))
