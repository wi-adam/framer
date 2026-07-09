# App Configuration

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** — ·
> **Plan:** [2026-07-09-app-configuration](../plans/2026-07-09-app-configuration.md) ·
> **Last reviewed:** 2026-07-09

## Intent / Purpose

Framer needs a structured runtime configuration surface for app behavior that is
not authored project intent. Experimental render backends, smoke-test hooks, and
future local runtime preferences should be resolved once at startup instead of
being scattered as one-off environment-variable checks.

Runtime configuration is intentionally separate from `.framer` files: it affects
how this process starts or tests itself, not the serialized building model.

## Requirements & Behavior

- Configuration is app-only and currently owned by `framer-app`.
- Precedence is deterministic: built-in defaults, then an explicit TOML config
  file, then `FRAMER__...` environment variables, then CLI flags.
- A config file is loaded only when `--config <PATH>` is passed. The path is
  required when supplied, and the file format is TOML.
- Environment variables use `FRAMER` as the prefix and `__` as the nesting
  separator. For example, `FRAMER__RENDER__RAY_QUERY=true` maps to
  `render.ray_query`.
- CLI flags are the final override layer. Boolean settings must be overridable in
  both directions, such as `--render-ray-query` and `--no-render-ray-query`.
- Unknown config keys are rejected so typos do not silently change startup
  behavior.
- Defaults keep experimental behavior off. The hardware ray-query backend remains
  disabled unless configuration enables it and the adapter supports the required
  wgpu feature.

## Decisions (Locked)

- **Use the `config` crate for layered configuration.** It owns TOML/env merging,
  precedence, and Serde deserialization so Framer does not grow custom source
  merging logic.
- **Use `clap` for CLI parsing.** Startup flags need normal help text, validation,
  conflicts, and typed values.
- **Do not preserve unreleased environment names.** Until Framer ships a release,
  new runtime knobs may be renamed or reshaped within the same future version.
- **Keep runtime config out of project serialization.** `.framer` remains authored
  building intent only.

## Architecture (Grounded in the Codebase)

- `crates/framer-app/src/app_config.rs` defines `AppConfig`, typed nested
  settings, CLI parsing, and the config-loader pipeline.
- `crates/framer-app/src/main.rs` resolves `AppConfig` before constructing
  `eframe::NativeOptions`, so startup-sensitive settings such as wgpu feature
  requests are available before device creation.
- `crates/framer-app/src/app/mod.rs` stores the resolved config on `FramerApp` for
  later presentation/runtime decisions.
- `crates/framer-app/src/app/render/mod.rs` accepts resolved render settings
  instead of reading environment variables directly.

Example file:

```toml
[render]
ray_query = true
smoke_frames = 180
```

Equivalent environment overrides:

```sh
FRAMER__RENDER__RAY_QUERY=true
FRAMER__RENDER__SMOKE_FRAMES=180
```

Equivalent CLI overrides:

```sh
cargo run -p framer-app -- --render-ray-query --render-smoke-frames 180
```

## Constraints & Invariants

- `framer-core`, `framer-solver`, `framer-standards`, and `framer-render` remain
  unaware of app runtime configuration.
- Config must not affect deterministic model serialization or solver output.
- GPU render correctness still depends on the CPU reference and parity tests; a
  config setting may select an experimental backend but may not redefine render
  math by itself.

## Out of Scope (YAGNI)

- Live config reload.
- A preferences/settings UI.
- Persisting runtime settings through egui storage.
- Project-local config embedded in `.framer`.
- Default platform config directories. They can be added when Framer has an
  installer/user-settings story.
