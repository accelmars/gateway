# Changelog

All notable changes to this project will be documented in this file.

## [0.4.0] - 2026-04-28

### Features
- RoutingStrategy trait and RequestMetadata struct — extension points for contract-aware routing; X-AccelMars-Context header carries engine, contract_id, task_type, and budget_usd per request (#50) ([#50](https://github.com/accelmars/gateway/pull/50))
- (**cli**) Add gateway demo command — zero-config mock mode showcase cycling Quick, Standard, and Max tiers (Wave A) (#48) ([#48](https://github.com/accelmars/gateway/pull/48))
- (**examples**) Add Rust example suite (Wave A) (#45) ([#45](https://github.com/accelmars/gateway/pull/45))
- (**examples**) Add Python example suite with cassettes (Wave A) (#44) ([#44](https://github.com/accelmars/gateway/pull/44))
- (**examples**) Add TypeScript example suite (Wave A) (#43) ([#43](https://github.com/accelmars/gateway/pull/43))
- Per-provider SSE streaming for DeepSeek and Claude — shared OAI-compat parse helper in openai_compat.rs and Anthropic native event-lifecycle implementation with cassette fixtures (#42) ([#42](https://github.com/accelmars/gateway/pull/42))
- Gemini native SSE streaming — `complete_chunks()` override with `streamGenerateContent?alt=sse`, per-chunk text collection, and token aggregation from final `usageMetadata` (#41) ([#41](https://github.com/accelmars/gateway/pull/41))
- Canary routing and shadow mode — weighted per-tier traffic splitting, rolling-window automatic rollback, and async shadow calls with independent health tracking (#40) ([#40](https://github.com/accelmars/gateway/pull/40))
- Seven new provider adapters and think tier — Qwen, Stepfun, NVIDIA NIM, OpenAI, Zhipu (via OpenRouter), MiniMax, and Moonshot; TierConfig.think field for config-driven think-tier routing (#39) ([#39](https://github.com/accelmars/gateway/pull/39))


### Documentation
- Testing-first README, OpenAI migration guide, and community provider guide with worked example (Wave A) (#49) ([#49](https://github.com/accelmars/gateway/pull/49))
- Add CASSETTE-SPEC.md — public cassette format specification (Wave A) (#47) ([#47](https://github.com/accelmars/gateway/pull/47))
- Add TESTING.md — mock, fixture, and cassette testing guide (Wave A) (#46) ([#46](https://github.com/accelmars/gateway/pull/46))

## [0.3.0] - 2026-04-27

### Features
- Streaming cassette format — StreamingSuccess variant enables multi-chunk SSE fixtures; schema v2 gate rejects invalid cassettes at load; FixtureAdapter.complete_chunks() returns pre-authored chunks without touching live providers (#37) ([#37](https://github.com/accelmars/gateway/pull/37))
- Complete_chunks() trait method and per-delta SSE emission — ProviderAdapter returns ChunkedResponse; server emits role, content, and finish deltas per OpenAI convention; Wave 3 adapter overrides unlock true token-level streaming (#36) ([#36](https://github.com/accelmars/gateway/pull/36))
- Streaming SSE test coverage — fixture-backed server correctly serves SSE chunks and HTTP errors for stream:true requests (#35) ([#35](https://github.com/accelmars/gateway/pull/35))
- Request-keyed cassette matching — EntryMatcher matches by tier or message content; keyed entries replay ahead of sequential fallback; backward compatible with all existing cassettes (#34) ([#34](https://github.com/accelmars/gateway/pull/34))
- GATEWAY_MODE=fixture activation — load cassette files via GATEWAY_FIXTURE_FILE env var; engine developers can replay recorded responses without modifying Rust source (#33) ([#33](https://github.com/accelmars/gateway/pull/33))
- CLI intent-oriented UX — intent-aware error messages, fuzzy "Did you mean?" suggestions, NO_COLOR support, and confirmation prompts across all 8 gateway command surfaces (#32) ([#32](https://github.com/accelmars/gateway/pull/32))
- OutputConfig — shared NO_COLOR / --no-color gate; colorize helper enables ANSI-free terminal output for all gateway commands (#31) ([#31](https://github.com/accelmars/gateway/pull/31))
- Suggest_similar utility — Levenshtein fuzzy matching for "Did you mean?" suggestions in CLI error paths (#30) ([#30](https://github.com/accelmars/gateway/pull/30))
- OpenTelemetry observability — GenAI semantic convention spans on every request, conditional OTLP export to Grafana Cloud, fail-open guarantee, and gateway overhead measurement (#28) ([#28](https://github.com/accelmars/gateway/pull/28))
- Circuit breaker + fallback retry — CLOSED/OPEN/HALF-OPEN state machine with exponential backoff, server-side retry across fallback chain on adapter failure, and circuit state visible on /status (#27) ([#27](https://github.com/accelmars/gateway/pull/27))
- Fly.io cloud deployment — persistent SQLite volume, auto-stop on idle, health checks on /health, and GitHub Actions auto-deploy on merge (#25) ([#25](https://github.com/accelmars/gateway/pull/25))
- API key authentication — Bearer token validation, SQLite key store, and gateway keys create/list/revoke commands (#24) ([#24](https://github.com/accelmars/gateway/pull/24))
- Agent-consumable gateway CLI — status exit codes (0/1/2), --json mode, PID file port discovery, routing constraint flags on complete, and gateway stop (#22) ([#22](https://github.com/accelmars/gateway/pull/22))
- Cassette recording/replay — integration tests replay provider responses from fixture files without API keys (#21) ([#21](https://github.com/accelmars/gateway/pull/21))


### Bug Fixes
- Remove secret-triggering phrase from v0.3.0 changelog entry
- OTLP traces not exported in deployed environments — explicitly read OTEL_EXPORTER_OTLP_ENDPOINT and OTEL_EXPORTER_OTLP_HEADERS in builder; sdk 0.31 does not auto-read these env vars (#29) ([#29](https://github.com/accelmars/gateway/pull/29))
- Disable release-plz GitHub Release creation — cargo-dist owns it (#17) ([#17](https://github.com/accelmars/gateway/pull/17))


### Documentation
- Client integration guide — quick start, tier strings, routing constraints, error codes, and reference implementations for cortex and guild (#23) ([#23](https://github.com/accelmars/gateway/pull/23))

## [0.2.0] - 2026-04-20

### Features
- Close PF-005 DONE A — concurrency semaphore, SQLite cost tracking, gateway status/stats/complete (#11) ([#11](https://github.com/accelmars/gateway/pull/11))
- Close PF-004 DONE A — config + routing (4 tiers, constraints, health tracking, fallback chains) (#10) ([#10](https://github.com/accelmars/gateway/pull/10))
- Close PF-003 DONE B — provider adapters (Gemini, DeepSeek, Claude, OpenRouter, Groq) + AdapterRegistry (#9) ([#9](https://github.com/accelmars/gateway/pull/9))
- OpenAI-compatible API server (gateway serve + README cleanup) (#8) ([#8](https://github.com/accelmars/gateway/pull/8))


### Bug Fixes
- Correct cliff.toml link template (url→href, index→text) (#14) ([#14](https://github.com/accelmars/gateway/pull/14))

## [0.1.0] - 2026-04-19

### Features
- Initial workspace scaffold with core types, CI, and release automation (#1) ([#1](https://github.com/accelmars/gateway/pull/1))


### Bug Fixes
- Disable crates.io publishing in release-plz until ready (#5) ([#5](https://github.com/accelmars/gateway/pull/5))
- Use RELEASE_PLZ_TOKEN PAT instead of GITHUB_TOKEN for release-plz (#3) ([#3](https://github.com/accelmars/gateway/pull/3))
- Add contents+pull-requests write permissions to release-plz workflow (#2) ([#2](https://github.com/accelmars/gateway/pull/2))


