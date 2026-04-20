## Summary
<!-- One sentence: what does this PR do? -->

## Why
<!-- Motivation. What problem does this solve or what capability does it add? -->

## Scope
<!-- What is explicitly in scope for this PR? -->

## Out of scope
<!-- What is explicitly NOT in this PR? -->

## Tests
<!-- What tests were added or updated? How was correctness verified? -->

## CI status
- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --workspace` passes

## Manual verification
<!-- Steps you ran locally to verify this works beyond CI -->

## Breaking change
<!-- Yes/No. If yes: what breaks and what is the migration path? -->

## Checklist
- [ ] Commit messages follow conventional commit format
- [ ] CHANGELOG.md updated if this is a user-visible change
- [ ] No provider SDK imports added to `accelmars-gateway-core`
- [ ] No model ID strings in Rust code (tiers only — model IDs belong in config)
- [ ] AI-assisted note in commit if applicable (`AI-assisted: Claude Code`)
