# Contributing to Framer

Framer is an open-source parametric CAD tool for wood-framed structures. This guide
covers setup, build/run, verification, and how we work. For the architecture
contract and the full doc index, read [AGENTS.md](AGENTS.md).

## Prerequisites

- **Rust 1.93.0**, pinned in [`rust-toolchain.toml`](rust-toolchain.toml) — `rustup`
  installs it automatically. CI uses the same version so formatting and lints match
  exactly. (Bump the toolchain and the CI `@1.93.0` refs in
  `.github/workflows/ci.yml` together.)
- **Linux only:** the desktop app needs eframe/winit/wgpu system libs:
  ```sh
  sudo apt-get install -y libxkbcommon-dev libwayland-dev \
    libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
  ```

## Build & run

```sh
cargo run -p framer-app          # opens examples/projects/demo-shell.framer
```

Headless render to a PNG (no app):

```sh
cargo run -p framer-render --features cli --release --bin render -- \
    examples/projects/demo-shell.framer out.png --width 1280 --height 720 --spp 256
```

To screenshot or drive the app with GUI tools, build + install the bundle first
(only installed bundles are visible to macOS screen capture) — see
`.claude/skills/install-app`.

## Verification (the gate)

Run from the workspace root before every commit. These mirror
[CI](.github/workflows/ci.yml):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

`cargo test --workspace --all-features` includes the headless `egui_kittest` UI
tests, the `framer-render` `cli`/golden tests, and the `.framer` round-trip +
schema-rejection tests.

Feature-specific checks:

| When you change… | Also run |
| --- | --- |
| GPU shader / renderer math | `cargo test -p framer-app --test gpu_parity -- --nocapture` (skips without a GPU adapter; CI runs it on macOS Metal + Linux lavapipe) |
| Renderer output intentionally | `UPDATE_GOLDEN=1 cargo test -p framer-render --test golden` (commits the new reference) |
| The `.framer` schema | bump `PROJECT_SCHEMA_VERSION`, update the three `examples/projects/*.framer`, add/adjust round-trip tests, update [docs/project-files.md](docs/project-files.md) |
| An example `.framer` file | `cargo test --workspace` (fixtures are byte-checked against canonical serialization) |

## How we work: spec-driven

Intent is written down and kept current; we separate **durable specs** from
**temporal plans**.

1. Tie the work to a goal in [docs/vision.md](docs/vision.md#goal-backlog) (or update
   the vision if it changes product intent).
2. Write/update the feature **spec** → [docs/specs/](docs/specs/) from
   [the template](docs/templates/spec-template.md).
3. Write the **plan** → [docs/plans/](docs/plans/) from
   [the template](docs/templates/plan-template.md).
4. Implement slice by slice; tests encode the spec's behavior.
5. Update the spec's Status and any affected docs.

Bug fixes and no-behavior refactors don't need a spec. Full detail:
[docs/spec-driven-development.md](docs/spec-driven-development.md).

## Commits

- `verb(scope): description` — e.g. `feat(core): construction systems`,
  `fix(viewport): clamp dolly`, `docs: refresh project-files to v7`.
- One focused change per commit; the workspace stays green after each.
- Don't commit regenerated artifacts (`*.svg`/`*.csv` exports, render output) into
  the repo or back into `.framer` files.

## Definition of Done

A change is done when the behavior is implemented in the right layer with focused
tests, output stays deterministic, persisted/`.framer` changes are review-friendly,
user-facing limitations are visible, docs are updated, and the full gate passes. The
authoritative list is in [docs/vision.md](docs/vision.md#definition-of-done).
