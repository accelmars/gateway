# Changelog

All notable changes to this project will be documented in this file.

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
