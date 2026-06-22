# Ceilings & Roofs — Implementation Plan (2026-06-20)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/ceilings-and-roofs.md](../specs/ceilings-and-roofs.md). This file is an archival
> record of how the work was sequenced; the spec is the durable source of truth.

## Goal

Deliver **v1** of the spec: gable + shed roofs, flat ceilings, and floor decks — rectangular,
stick-framed, with structural judgment surfaced as diagnostics. The thinnest slice that crosses
every crate (core → solver → render → app), so the first roof/ceiling appears, frames, renders,
and shows up in the BOM. Cathedral/scissor, hips/valleys, trusses, and engineered members are
out of scope (later phases in the spec).

## Architecture / stack summary

Builds on existing seams (see the spec's Architecture section for the durable detail):

- `framer-core/src/model.rs` — `BuildingModel`, `Level` (`{id,name,elevation}`),
  `SystemKind {Wall,Floor,Roof}`, `ConstructionSystem`/`ConstructionLayer`/`FramingSpec`,
  `LayerFunction`, `OpeningKind` (`Skylight`/`Stair` already present), `validate()`,
  `sort_deterministically()`.
- `framer-core/src/project.rs` — `PROJECT_SCHEMA_VERSION = 10`, single-version loader.
- `framer-core/src/topology.rs` — `room_boundaries`, `wall_interior_sides` (bearing outline).
- `framer-solver/src/lib.rs` — `generate_project_plan` → `generate_wall_plan`;
  `ProjectFramePlan`, `FrameMember`, `MemberKind`, `MemberOrientation`; BOM/CSV/SVG exports.
- `framer-render/src/build.rs` (`geometry_from_model`, `push_box`, `PaletteBuilder`),
  `geom.rs` (`Triangle`, plane-agnostic normal); `scenes.rs`/`tests/golden.rs`,
  `framer-app/tests/gpu_parity.rs`.
- `framer-app/src/app/` — `mod.rs` (tools, `edit()`, `Selection`, `ViewClick`, `ViewportMode`),
  `panels.rs` (tree/inspector/toolbar), `viewport/plan.rs`, `viewport/scene_build.rs`.

## Slices / phases

Each slice leaves the workspace green and is independently reviewable.

### Slice 1 — Core types + schema v11 (no framing yet)

- **Task 1.1** — Add `Level.height: Length` (`#[serde(default)]`); add `SystemKind::Ceiling`
  (update `ALL` + `label()`).
  - Files: `crates/framer-core/src/model.rs`
  - Verify: `cargo test -p framer-core`
  - Commit: `feat(core): add Level.height and SystemKind::Ceiling`
- **Task 1.2** — Add `Slope`, `SurfaceRegion`, `SpanDirection`, `MemberFamily`; add `RoofPlane`,
  `Ceiling`, `FloorDeck`, `RoofOpening` types and the three `Vec`s on `BuildingModel`
  (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`). Re-export from `lib.rs`.
  - Files: `crates/framer-core/src/model.rs`, `crates/framer-core/src/lib.rs`
  - Verify: `cargo test -p framer-core`
  - Commit: `feat(core): add roof plane, ceiling, and floor deck primitives`
- **Task 1.3** — Extend `FramingSpec` with `member_family` (`#[serde(default)]` → `Stud`); add
  `LayerFunction::{Roofing, Underlayment, CeilingFinish}`; re-scope `exposure()` per
  `SystemKind`.
  - Files: `crates/framer-core/src/model.rs`
  - Verify: `cargo test -p framer-core`
  - Commit: `feat(core): roof/floor layer functions and framing member family`
- **Task 1.4** — Validation: kind-matched `system` checks for the three new objects; `level`
  membership; id uniqueness; eave-edge range; outline ≥3 pts / non-self-intersecting;
  `slope.run > 0`; generalize the single-framing-layer rule to `Roof`/`Floor`/`Ceiling`. Add to
  `sort_deterministically()` (incl. nested `RoofOpening`s). New `ModelError` variants.
  - Files: `crates/framer-core/src/model.rs`
  - Verify: `cargo test -p framer-core` (per-rule reject/accept unit tests)
  - Commit: `feat(core): validate roof/ceiling/floor objects, fail closed`
- **Task 1.5** — Schema **v10 → v11**: bump `PROJECT_SCHEMA_VERSION`; regenerate the three
  `examples/projects/*.framer`; update the version-pinned tests
  (`save_project_writes_schema_versioned_authored_model`, `load_project_rejects_old_schema`);
  update `docs/project-files.md`; update the schema-version invariant in `AGENTS.md` (v10→v11).
  - Files: `crates/framer-core/src/project.rs`, `examples/projects/*.framer`,
    `docs/project-files.md`, `AGENTS.md`
  - Verify: `cargo test -p framer-core` (byte-exact round-trip), `python3 scripts/check-markdown-links.py`
  - Commit: `feat(core): bump .framer schema to v11 for roofs/ceilings/floors`

### Slice 2 — Floor-deck & flat-ceiling joisting (the easy generator)

- **Task 2.1** — `generate_floor_plan` / `generate_ceiling_plan`: from a region's bearing
  outline (`topology`), array joists across the shorter span (or explicit) at o.c. from a layout
  origin; add rim/band members and end blocking. New `MemberKind::{FloorJoist, CeilingJoist,
  RimJoist, Blocking}` (+ `label()`, `member_svg_color()`). Wire into `generate_project_plan`;
  add `floor_plans`/`ceiling_plans` to `ProjectFramePlan` and the BOM/`layer_bom` flatteners.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver` (golden joist layout for a known rectangle; BOM rows)
  - Commit: `feat(solver): generate floor-deck and flat-ceiling joists`
- **Task 2.2** — Diagnostics: span-not-checked note; open-region warning carrying the element id.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver`
  - Commit: `feat(solver): joist span/open-region diagnostics`

### Slice 3 — Roof-plane rafter framing

- **Task 3.1** — Add the sloped placement to `FrameMember` (integer-tick start/end elevation +
  in-plane axis, serde-defaulted, `Eq`). `MemberKind::{Rafter, RidgeBoard}` (+ exhaustive
  matches). `generate_roof_plan`: common rafters perpendicular to the eave edge at o.c.
  (**true sloped cut length**; plan length for spacing/area), ridge board for gable, plate
  blocking. Add `roof_plans` to `ProjectFramePlan` + flatteners.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver` (rafter count + true-vs-plan length for a known pitch)
  - Commit: `feat(solver): generate roof-plane rafters and ridge board`
- **Task 3.2** — Structural diagnostics: ridge-board-without-tie note; varying-plate-height
  unsupported flag.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver`
  - Commit: `feat(solver): roof structural diagnostics (ridge/tie, plate height)`

### Slice 4 — Render the sloped + horizontal surfaces

- **Task 4.1** — `framer-render`: `push_quad`/fan-triangulator in `geometry_from_model` for
  roof/ceiling/floor faces in a plane-local basis; lower their system layers through
  `PaletteBuilder` (no new `MAT_*`); grow the bounds `Aabb` for `SceneFraming`. Add a
  model-derived roofed golden scene.
  - Files: `crates/framer-render/src/build.rs`, `crates/framer-render/src/scenes.rs`,
    `crates/framer-render/tests/golden.rs`
  - Verify: `cargo test -p framer-render` (`UPDATE_GOLDEN=1` to regen intentionally),
    then `cargo test -p framer-app --test gpu_parity` (must stay green; opaque-diffuse only)
  - Commit: `feat(render): emit roof/ceiling/floor geometry via shared Triangle path`
- **Task 4.2** — App 3-D mesher: sloped roof solid + horizontal slabs in `scene_build.rs`, with
  `PickSolid` + `member_color` entries so they select like walls/members.
  - Files: `crates/framer-app/src/app/viewport/scene_build.rs`
  - Verify: headless `cargo test -p framer-app`; manual 3-D check via `install-app` skill
  - Commit: `feat(app): show roof/ceiling/floor in the 3D viewport`

### Slice 5 — Authoring UX (tools, tree, inspector)

- **Task 5.1** — Wire `SystemKind::{Floor,Roof,Ceiling}` through system authoring + the
  inspector system picker (drop the `kind == Wall` filter); seed starter Roof/Floor/Ceiling
  systems in `libraries/framer-starter.framerlib`. Un-fork `add_opening` so `Skylight` isn't
  coerced to a window.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/panels.rs`,
    `libraries/framer-starter.framerlib`
  - Verify: `cargo test -p framer-app`; manual: assign a roof system to a plane
  - Commit: `feat(app): author roof/floor/ceiling construction systems`
- **Task 5.2** — Flat-ceiling tool (region-gated like the room tool) + `add_ceiling`/`add_floor`
  commits via `edit()`; `Selection`/`ViewClick` variants; model-tree nodes under each level;
  inspector arms (height, span, system).
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/plan.rs`
  - Verify: `cargo test -p framer-app` (undo = one step per add; round-trip); manual place
  - Commit: `feat(app): ceiling tool, tree nodes, and inspector`
- **Task 5.3** — `ViewportMode::RoofPlan` + roof tool: auto-generate gable/shed planes from a
  rectangular footprint (pitch + per-edge gable/hip flag) and write them into the model as
  editable `RoofPlane`s; inspector for pitch/overhangs/eave-edge.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/viewport/plan.rs`, `crates/framer-app/src/app/panels.rs`
  - Verify: `cargo test -p framer-app`; manual: one-click roof on demo-shell, edit pitch
  - Commit: `feat(app): roof tool with auto-from-footprint plane generation`

### Slice 6 — Example + docs

- **Task 6.1** — Add a roofed example (demo-shell + gable roof + flat ceilings) or extend an
  existing example; refresh BOM/SVG snapshots.
  - Files: `examples/projects/*.framer`, solver snapshot tests
  - Verify: `cargo test --workspace`
  - Commit: `feat(examples): roofed shell example project`
- **Task 6.2** — Flip the spec **Status** to *Partial*, set **Last reviewed**; add **G-014** to
  `docs/vision.md` backlog; update `docs/code-map.md` ("Where do I add X?" rows for roof/ceiling
  objects + members); update `docs/specs/construction-systems.md` so its `SystemKind`
  enumeration (and the "Floor and roof systems" out-of-scope line) reflect the now-wired
  `Roof`/`Floor`/`Ceiling` kinds.
  - Files: `docs/specs/ceilings-and-roofs.md`, `docs/vision.md`, `docs/code-map.md`,
    `docs/specs/construction-systems.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: `docs: record ceilings/roofs v1 in spec, vision, and code-map`

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Plus feature-specific checks: byte-exact `.framer` round-trip (the three examples), golden
render regen (`UPDATE_GOLDEN=1 cargo test -p framer-render --test golden`), GPU↔CPU parity
(`cargo test -p framer-app --test gpu_parity`), and a manual 3-D pass via the `install-app`
skill (place a roof, see rafters in Plan Mode, confirm BOM rows). When done, update the spec's
**Status**/**Last reviewed** and the affected docs.
