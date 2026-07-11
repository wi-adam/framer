# AGENTS.md — Framer contributor & agent contract

Read this first. It is the single entry point for any developer or coding agent
(Claude, Codex, …) working in this repo. It is navigational: it points at the
authoritative docs rather than duplicating them.

Framer is an open-source parametric CAD tool for **wood-framed structures**: model
construction intent → generate framing plans, a bill of materials, and a
path-traced rendering. Product source of truth: [docs/vision.md](docs/vision.md).

## Repo map

Rust workspace, seven crates (strict dependency order — UI depends on logic, never
the reverse):

| Crate | Responsibility |
| --- | --- |
| [`crates/framer-core`](crates/framer-core) | Domain model: authored building intent, units, construction systems, materials, standards packs, room topology, validation, `.framer` serialization. **No UI.** |
| [`crates/framer-library`](crates/framer-library) | Library resolution, exact content hashing, and vendor-on-use import/remap for reusable `.framerlib` content. **No UI.** |
| [`crates/framer-solver`](crates/framer-solver) | Deterministic framing generation + per-layer BOM + room schedule + diagnostics; SVG/CSV exports. **No UI.** |
| [`crates/framer-standards`](crates/framer-standards) | UI-free compliance facts, evaluator, report CSV, and diagnostics lowering over resolved standards + solver output. **No UI.** |
| [`crates/framer-geometry`](crates/framer-geometry) | UI-free physical solids for authored assemblies and generated members; stable body identity and convex-piece lowering. **No UI.** |
| [`crates/framer-render`](crates/framer-render) | UI-agnostic CPU path tracer (reference math for the app's GPU shader). **No UI.** |
| [`crates/framer-app`](crates/framer-app) | Native desktop CAD shell (`eframe`/`egui` + `wgpu`). |

Docs index:

- [docs/vision.md](docs/vision.md) — product source of truth, principles, milestones, goal backlog (G-001…G-012).
- [docs/architecture.md](docs/architecture.md) — conceptual system shape & layering.
- [docs/code-map.md](docs/code-map.md) — **concrete** navigation: modules, key types, data-flow, "where to add X".
- [docs/project-files.md](docs/project-files.md) — the `.framer` format + agent editing contract.
- [docs/spec-driven-development.md](docs/spec-driven-development.md) — how we work (specs vs. plans).
- [docs/specs/](docs/specs/) — durable feature specs. [docs/plans/](docs/plans/) — dated implementation plans.
- [CONTRIBUTING.md](CONTRIBUTING.md) — setup, build/run, verification, Definition of Done.

## Architecture invariants (do not break)

1. **`framer-core`, `framer-solver`, `framer-geometry`, and `framer-render` carry no UI dependency.**
   They stay testable, scriptable, and exportable without the app.
2. **Three layers, one source of truth:** authored *intent* (`BuildingModel`) →
   derived *framing* (`ProjectFramePlan`, regenerated) → *presentation* (viewports,
   drawings, exports, disposable). Only authored intent is editable and persisted.
   See [architecture.md](docs/architecture.md#modeling-layers).
3. **Determinism.** Same model + standards stack → byte-identical `.framer` and
   identical framing/render. Lengths are integer **ticks** (16 = 1 inch), no floats
   in the model; `.framer` is ID-sorted + canonical; the renderer is seeded (PCG).
4. **`.framer` is schema v13 and v13-only.** Bumping the schema means updating
   `PROJECT_SCHEMA_VERSION`, the three `examples/projects/*.framer`, the round-trip
   tests, [project-files.md](docs/project-files.md), and the version references in
   [crates/framer-core/README.md](crates/framer-core/README.md),
   [docs/architecture.md](docs/architecture.md), and
   [docs/code-map.md](docs/code-map.md).
5. **CPU render is the reference;** the app's WGSL compute shader mirrors it. Change
   both together and keep `tests/gpu_parity.rs` green.
6. **Code compliance is explicit, never implied.** The IRC 2021 standards pack is a
   *starter* shape; label unsupported conditions with diagnostics.

## How we work: spec-driven

Intent is written down and kept current. The key rule: **separate durable specs
from temporal plans.**

- A **spec** (durable, feature-named, no date) → [docs/specs/](docs/specs/), from
  [the template](docs/templates/spec-template.md).
- A **plan** (point-in-time, dated, archival) → [docs/plans/](docs/plans/), from
  [the template](docs/templates/plan-template.md).

New feature or product-visible / schema change → write or update the spec, then a
plan, then implement. Bug fixes and no-behavior refactors don't need a spec. Full
loop: [spec-driven-development.md](docs/spec-driven-development.md).

## Verification gates (must pass before commit)

Run from the workspace root. These match CI (`.github/workflows/ci.yml`) and the
toolchain pinned in `rust-toolchain.toml` (1.93.0):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Feature-specific checks when relevant:

- GPU↔CPU path-tracer parity: `cargo test -p framer-app --test gpu_parity -- --nocapture`
  (skips without a GPU adapter; CI runs it on macOS Metal + Linux lavapipe).
- UI screenshots for visual review: `scripts/ui-shots.sh` renders the full app UI
  off-screen to `target/ui-shots/*.png` (~15 s, no install/window needed).
- Golden render regen (intentional only): `UPDATE_GOLDEN=1 cargo test -p framer-render --test golden`.
- Editing example `.framer` files still requires `cargo test --workspace` (round-trip fixtures).
- Editing docs/markdown: `python3 scripts/check-markdown-links.py` (relative-link check; CI's
  `docs` job runs it on every PR).

Definition of Done: [docs/vision.md](docs/vision.md#definition-of-done).

## Commit & change conventions

- Commit messages: `verb(scope): description` (e.g. `feat(solver): per-layer takeoff`,
  `fix(viewport): clamp dolly`). One focused change per commit; leave the workspace
  green after each.
- Editing `.framer`: edit `authored` intent only; preserve IDs and `schema_version`;
  apply construction by reference (no dangling `system`/`material` ids); keep layer
  order interior → exterior. See the
  [agent editing contract](docs/project-files.md#agent-editing-contract).
- Update docs when the product surface or architecture changes — including the
  affected spec's **Status** and `code-map.md`/`project-files.md` where relevant.

## Tool notes

- **Codex** reads this `AGENTS.md` natively; use `.codex/skills/framer-development`
  for substantial feature work / PR readiness and `.codex/skills/framer-commit`
  for scoped validation and commits.
- **Claude Code** loads [CLAUDE.md](CLAUDE.md) (a thin pointer here) and has an app
  build/install skill at `.claude/skills/install-app` for GUI/visual verification.
