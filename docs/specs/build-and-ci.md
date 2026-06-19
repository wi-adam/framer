# Build & CI

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** — (engineering infrastructure; supports
> G-010 Packaging later) · **Last reviewed:** 2026-06-19

## Intent / Purpose

The build and CI exist to make Framer's core promise — **determinism and a UI-free,
testable core** — mechanically enforced rather than aspirational. Every push and PR must
prove the workspace formats, lints, builds, and tests identically across platforms, and that
the GPU path tracer still matches its CPU reference. The exact commands live in
[CONTRIBUTING.md](../../CONTRIBUTING.md#verification-the-gate) and
[AGENTS.md](../../AGENTS.md#verification-gates-must-pass-before-commit); this spec captures the
*why* behind the shape so it isn't re-derived from `.github/workflows/ci.yml` comments.

## Requirements & behavior

- **One gate, everywhere.** The same checks pass locally, in the `framer-commit` skill, and in
  CI: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features
  --locked -- -D warnings`; `cargo test --workspace --all-features --locked`.
- **Cross-platform.** Build + test run on Linux, macOS, and Windows (a `fail-fast: false`
  matrix) — the desktop app must build on all three.
- **GPU↔CPU parity is always exercised.** `framer-app/tests/gpu_parity.rs` runs against a real
  Metal adapter in the macOS test job and against Mesa **lavapipe** (software Vulkan) in a
  dedicated Linux `gpu` job, so the WGSL compute path tracer is validated even where no
  hardware GPU exists.
- **No silent skips.** The parity test *passes (skips)* when no adapter is found. The `gpu` job
  therefore hard-fails if `vulkaninfo` does not report a `lavapipe`/`llvmpipe` adapter — a
  broken Mesa install must turn the job red, not green-while-testing-nothing.
- **Reproducible.** `--locked` everywhere (CI must not silently update `Cargo.lock`).
  `--all-features` so `framer-render`'s `cli`/`parallel` paths are linted and tested;
  `--all-targets` so tests, benches, and examples are covered.
- **Fast feedback.** `lint` runs first; `test` and `gpu` depend on it. In-progress runs on a
  non-`main` ref are cancelled when superseded; `main` runs always finish so they populate the
  shared build cache (`Swatinem/rust-cache`, `cache-on-failure: true`).
- **Docs don't rot.** A `docs` job runs `scripts/check-markdown-links.py` and fails the build
  if any relative link in tracked markdown points at a missing file (local links only;
  external URLs are not fetched). This is the mechanized version of a real failure mode — a
  link broke during the docs reorg when a spec moved between `docs/specs/` and `docs/plans/`.

## Decisions (locked)

- **Pin the toolchain in [`rust-toolchain.toml`](../../rust-toolchain.toml) and match it in CI.**
  Currently `1.93.0`. Without a pin, a newer stable `rustfmt`/`clippy` silently reformats or
  re-lints the tree and `--check` starts failing. **Bump the `channel` and every
  `dtolnay/rust-toolchain@<channel>` ref in `ci.yml` together,** then run `cargo fmt --all`
  once to absorb new formatting.
- **MSRV is separate from the build toolchain.** The crates declare `rust-version = "1.92"` (the
  minimum-supported floor); the pinned `1.93.0` is what we *build and format with*. Don't
  conflate them.
- **Clippy is `-D warnings`.** Warnings are failures; fix or explicitly `#[allow(...)]` with a
  reason. `--all-features --all-targets` so nothing escapes the lint.
- **Lavapipe over skipping.** Validating the GPU kernel on a CPU Vulkan driver in CI is worth
  the dependency, because skipping would let the renderer and its WGSL mirror drift apart
  undetected.
- **Edition 2024, resolver 3, workspace-pinned dependencies** for reproducible builds across
  the four crates.

## Architecture (grounded in the codebase)

- CI: [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) — four jobs: `lint`
  (rustfmt + clippy, Linux), `test` (build + test matrix on Linux/macOS/Windows), `gpu`
  (lavapipe parity, Linux), and `docs` (markdown link check, no Rust toolchain). Triggers:
  push to `main`, pull requests, `workflow_dispatch`.
- Markdown link check: [`scripts/check-markdown-links.py`](../../scripts/check-markdown-links.py)
  (stdlib-only; runnable locally with the same command CI uses).
- Toolchain: [`rust-toolchain.toml`](../../rust-toolchain.toml).
- Workspace: [`Cargo.toml`](../../Cargo.toml) (members, edition 2024, resolver 3, MSRV 1.92,
  pinned `[workspace.dependencies]`).
- Linux system deps for `eframe`/`winit`/`wgpu` are installed in the jobs that build the app
  (`libxkbcommon`, `libwayland`, `libxcb-*`; plus `mesa-vulkan-drivers`/`vulkan-tools` for `gpu`).
- Local helpers: [`scripts/install-app.sh`](../../scripts/install-app.sh) (build + ad-hoc-sign +
  install the macOS bundle for GUI/visual verification); the `.codex/skills/framer-commit` skill
  runs the gate before committing.

## Constraints & invariants

- CI must mechanically enforce the [architecture invariants](../architecture.md): determinism
  (round-trip + golden + parity tests), UI-free core/solver/render (they build and test without
  the app), and the v7 `.framer` round-trip fixtures.
- The local gate, the commit skill, and CI must stay in lockstep — changing one means changing
  the others.

## Out of scope (YAGNI)

- Release packaging and signing of distributable artifacts (that's **G-010 Packaging**; this
  spec covers verification, not distribution).
- Coverage reporting, benchmark tracking/regression gates, and a spec-staleness lint (flagging
  specs whose **Last reviewed** is old) — noted as possible future work in
  [spec-driven-development.md](../spec-driven-development.md#future-work-not-built-now).
  Markdown link checking *is* enforced (see Requirements); external-URL liveness is
  deliberately not, to keep CI deterministic.
- A hardware-GPU CI runner (lavapipe + macOS Metal cover the parity need today).
