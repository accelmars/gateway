# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - 2026-04-20

### Features

- Close PF-005 DONE A — concurrency semaphore, SQLite cost tracking, gateway status/stats/complete (#11) ([#11](https://github.com/accelmars/gateway/pull/11))

- Close PF-004 DONE A — config + routing (4 tiers, constraints, health tracking, fallback chains) (#10) ([#10](https://github.com/accelmars/gateway/pull/10))

- Close PF-003 DONE B — provider adapters (Gemini, DeepSeek, Claude, OpenRouter, Groq) + AdapterRegistry (#9) ([#9](https://github.com/accelmars/gateway/pull/9))

- OpenAI-compatible API server (gateway serve + README cleanup) (#8) ([#8](https://github.com/accelmars/gateway/pull/8))

### Bug Fixes

- Correct cliff.toml link template (url→href, index→text) (#14) ([#14](https://github.com/accelmars/gateway/pull/14))

### Testing

- Add PF-005R concurrency audit tests — panic safety + concurrent SQLite writes (#12) ([#12](https://github.com/accelmars/gateway/pull/12))

### Miscellaneous

- Oss-ready — community files, cliff.toml, CI Node.js 24, README Phase 1 state (#13) ([#13](https://github.com/accelmars/gateway/pull/13))

## [0.1.0] - 2026-04-19

### Features

- Initial workspace scaffold with core types, CI, and release automation ([#1](https://github.com/accelmars/gateway/pull/1))
