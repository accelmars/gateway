# Contributing to accelmars/gateway

Thank you for your interest in contributing. This document covers everything you need
to get from zero to a merged PR.

---

## 1. Prerequisites

- **Rust toolchain:** stable, latest (`rustup update stable`)
- **Required components:** `rustfmt`, `clippy`
  ```bash
  rustup component add rustfmt clippy
  ```

Minimum Rust version: **1.75**

---

## 2. Setup

```bash
git clone https://github.com/accelmars/gateway.git
cd gateway
cargo build
cargo test --workspace
```

Both commands should succeed on a clean clone before you make any changes. No API keys needed — the test suite uses the mock adapter.

---

## 3. Contribution Types

We welcome:

- **Bug fixes** — open an issue first if the fix is non-trivial.
- **New provider adapters** — implement `ProviderAdapter` from `accelmars-gateway-core`. See the existing adapters in `crates/accelmars-gateway/src/adapters/` for reference.
- **Routing improvements** — new constraint types, routing strategies, config options.
- **CLI improvements** — new subcommands, better output formatting.
- **Documentation** — README clarity, rustdoc, COMMANDS.md accuracy. Docs are first-class.

---

## 4. The Core Rule: No Provider SDKs in `accelmars-gateway-core`

`accelmars-gateway-core` defines traits and types only. **Zero provider SDK dependencies.**

```
accelmars-gateway-core  ←  traits, types, MockAdapter (no external deps)
accelmars-gateway       ←  all provider adapters, HTTP server, CLI
```

If you add a dependency to `accelmars-gateway-core/Cargo.toml`, CI will likely still pass, but reviewers will ask you to move it to the binary crate. This boundary is what makes the core embeddable and audit-friendly.

The second rule: **no model ID strings in Rust code**. Model IDs belong in `gateway.toml` under `[providers.*]`. Rust code uses `ModelTier` values only.

---

## 5. Commit Format

We use [Conventional Commits](https://www.conventionalcommits.org/). Enforced in CI.

| Type | When |
|------|------|
| `feat:` | New behavior |
| `fix:` | Bug corrected |
| `docs:` | Documentation only |
| `chore:` | Infrastructure, tooling |
| `refactor:` | Restructure without behavior change |
| `test:` | Tests only |
| `adapter:` | New or updated provider adapter — **specific to this repo** |

**`adapter:`** is for contributions that add or substantially change a provider adapter.

**AI-assisted commits:** include in commit message:
```
AI-assisted: Claude Code
```

---

## 6. Branching Strategy

All branches are created from `main` and merged back to `main`.

| Prefix | When |
|--------|------|
| `feat/` | New capability |
| `fix/` | Bug fix |
| `docs/` | Documentation only |
| `adapter/` | New or updated provider adapter |
| `chore/` | Infrastructure |
| `refactor/` | Restructure without behavior change |

Branch naming: lowercase, hyphen-separated. Example: `adapter/mistral`

---

## 7. Style Guide

- **Formatting:** `rustfmt` with default settings. Run `cargo fmt` before committing.
- **Lints:** `cargo clippy -- -D warnings`. All warnings are errors.
- **Error handling:** Use `thiserror` for library errors, `anyhow` in binary code.
- **Adapter errors:** Return typed `AdapterError` variants — don't panic, don't use `unwrap()` in production paths.

---

## 8. PR Process

Use the [PR template](.github/PULL_REQUEST_TEMPLATE.md). Fill every section.

When ready for review:
1. Ensure CI is green
2. Remove draft status
3. Request review from `@accelmars/gateway-maintainers`

Squash merge only. The PR title becomes the squash commit message.

---

## 9. Review SLA

**First response: 48 hours** after you remove draft status.

---

## 10. Adding a New Provider Adapter

1. Create `crates/accelmars-gateway/src/adapters/{name}.rs`
2. Implement `ProviderAdapter` — `name()`, `complete()`, `is_available()`
3. Register in `main.rs` → `build_registry_from_config()`
4. Add provider section to `gateway.example.toml`
5. Write unit tests with mocked HTTP responses
6. Update the Supported Providers table in `README.md`
7. Add a CHANGELOG entry under `[Unreleased]`

No changes to `accelmars-gateway-core` needed.

---

## 11. What Not to Commit

- **API keys or credentials** — even in tests. Use the mock adapter.
- **Provider SDK imports in `accelmars-gateway-core`** — core stays dependency-light.
- **Hardcoded model ID strings** — use `ModelTier`, map to model IDs in config only.
- **`gateway.toml`** — this file is gitignored. Use `gateway.example.toml` for config examples.
- **Telemetry or network calls** in tests without explicit mock setup.
