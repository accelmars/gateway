# community-provider — AccelMars Gateway Provider Example

A minimal working implementation of `ProviderAdapter` for AccelMars Gateway.
`StubProvider` returns a deterministic echo response with no API key, no network
calls, and no external dependencies. Use it as the starting point for your own provider.

## What this demonstrates

- Implementing the `ProviderAdapter` trait from `accelmars-gateway-core`
- Returning a typed `GatewayResponse` from `complete()`
- Signaling availability via `is_available()`
- Declaring which tiers a provider covers (inherent method, not trait)
- Writing unit tests against `ProviderAdapter` without a running gateway

## Build

```sh
# From this directory:
cargo build

# Run the tests:
cargo test
```

Rust stable (≥ 1.75) is required. No API keys or environment variables needed.

## Wiring into a local gateway build

See [`docs/ADDING-A-PROVIDER.md`](../../docs/ADDING-A-PROVIDER.md) in the gateway
repo for the complete step-by-step guide — including how to register the provider
in `main.rs` and target it via `gateway.toml`.

## Path dependency note

`Cargo.toml` uses a path dependency to `accelmars-gateway-core`:

```toml
accelmars-gateway-core = { path = "../../crates/accelmars-gateway-core" }
```

Once `accelmars-gateway-core` is published to crates.io, you can replace this with
a version dependency:

```toml
accelmars-gateway-core = "0.3"
```
