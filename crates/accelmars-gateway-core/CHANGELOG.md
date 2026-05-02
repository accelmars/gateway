# Changelog

All notable changes to this project will be documented in this file.



## [0.3.0] - 2026-04-27

### Features

- `suggest_similar` utility — Levenshtein fuzzy matching for "Did you mean?" suggestions in CLI error paths ([#30](https://github.com/accelmars/gateway/pull/30))
- `OutputConfig` — shared `NO_COLOR` / `--no-color` gate; `colorize` helper enables ANSI-free terminal output for all gateway commands ([#31](https://github.com/accelmars/gateway/pull/31))
- Request-keyed cassette matching — `EntryMatcher` matches by tier or message content; keyed entries replay ahead of sequential fallback; backward-compatible with all v0.2.0 cassettes ([#34](https://github.com/accelmars/gateway/pull/34))
- `complete_chunks()` trait method — `ProviderAdapter` returns `ChunkedResponse`; default impl preserves Phase 1 behavior; per-provider streaming overrides unlock true token-level SSE emission ([#36](https://github.com/accelmars/gateway/pull/36))
- Streaming cassette schema v2 — `StreamingSuccess` variant with `chunks: Vec<String>`; `CASSETTE_SCHEMA_VERSION_STREAMING = "2"`; `FixtureAdapter::from_cassette()` validates schema version at load and rejects v1+StreamingSuccess and unknown versions ([#37](https://github.com/accelmars/gateway/pull/37))

## [0.2.0] - 2026-04-20

### Features

- Add `RoutingConstraints`, `ProviderConstraints`, and `RoutingConfig` types for config-driven provider selection across four quality tiers ([#10](https://github.com/accelmars/gateway/pull/10))

## [0.1.0] - 2026-04-19

### Features

- Initial workspace scaffold with core types, CI, and release automation ([#1](https://github.com/accelmars/gateway/pull/1))
