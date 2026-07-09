# App Configuration — Implementation Plan (2026-07-09)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/app-configuration.md](../specs/app-configuration.md). This file is
> an archival record of how the work was sequenced; the spec is the durable source
> of truth.

## Goal

Replace one-off app environment-variable checks with a typed, layered runtime
configuration path for Framer's desktop shell. The first delivered settings cover
the experimental ray-query render backend and the render smoke-test hook.

## Architecture / Stack Summary

`framer-app` owns runtime configuration because these settings affect process
startup and presentation behavior, not authored project data. The implementation
uses `config` for TOML/env/source layering and `clap` for CLI parsing. `main.rs`
loads config before creating the wgpu device, then passes the resolved
`AppConfig` into `FramerApp`.

## Slices / Phases

### Slice 1 — Runtime Config Loader

- **Task 1.1** — Add typed app config, source layering, CLI flags, and tests.
  - Files: `crates/framer-app/src/app_config.rs`, `Cargo.toml`,
    `crates/framer-app/Cargo.toml`
  - Verify: `cargo test -p framer-app app_config --all-features --locked`
  - Commit: `feat(app): add layered runtime config`

### Slice 2 — Render Integration

- **Task 2.1** — Replace render env checks with resolved config.
  - Files: `crates/framer-app/src/main.rs`,
    `crates/framer-app/src/app/mod.rs`,
    `crates/framer-app/src/app/render/mod.rs`,
    `crates/framer-app/src/app/viewport/render.rs`
  - Verify: `cargo test -p framer-app app_config --all-features --locked`
  - Commit: `refactor(render): route experimental backend through config`

### Slice 3 — Docs

- **Task 3.1** — Document the config contract and the updated render knobs.
  - Files: `docs/specs/app-configuration.md`, `docs/specs/render-view.md`,
    `docs/specs/README.md`, `docs/code-map.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: `docs(app): document runtime configuration`

## Final Verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
```
