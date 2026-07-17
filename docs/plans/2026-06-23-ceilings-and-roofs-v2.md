# Ceilings & Roofs v2 — Implementation Plan (2026-06-23)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/ceilings-and-roofs.md](../specs/ceilings-and-roofs.md). This file is an archival
> record of how the work was sequenced; the spec is the durable source of truth.

## Goal

Deepen the [Ceilings & Roofs](../specs/ceilings-and-roofs.md) feature (G-014) past the v1 slice
(gable/shed roofs, flat ceilings, floor decks) along the two axes the v1 spec left
"architecturally open":

- **Phase A — Sloped ceilings + the ridge structural fork.** Make a roof and its ceiling tell
  the truth about the *space between them*: render a cathedral (no ceiling) with the roof's
  **interior** finish, activate the dormant `Ceiling.slope` so scissor/vaulted ceilings frame and
  render, and turn the always-on `roof.ridge.no-tie` warning into a real **tie detection →
  ridge-board-vs-ridge-beam** judgment. Low geometry risk; reuses the existing
  `RoofPlaneFrame` lift. Crosses every crate thinly.
- **Phase B — Hip & valley roofs.** Add the first non-opposing-plane roof geometry: hip planes on
  a rectangular footprint, valleys where wings meet on an L/T footprint, the jack rafters that
  die into them, and a multi-plane member post-pass analogous to `add_join_members`. The hard
  geometry; several slices.

Phase A ships first and stands alone; Phase B builds on it. Trusses, engineered members, dormers,
and validated span/tie calculations remain out of scope (see the spec).

## Architecture / stack summary

The relevant existing seams (the spec's Architecture section holds the durable detail):

- `framer-core/src/model.rs` — `Ceiling { …, height, slope: Option<Slope> }` (slope present,
  validated nowhere, read nowhere), `RoofPlane`, `RoofPlaneFrame` (the shared
  `elevation_at` / `up_slope_distance` affine lift), `Slope`, `SurfaceRegion`, `SpanDirection`,
  `SystemKind`, `LayerFunction`, `MemberFamily`, `BuildingModel::validate`,
  `topology::{room_boundaries, triangulate_simple_polygon}`.
- `framer-core/src/project.rs` — `PROJECT_SCHEMA_VERSION = 11`, single-version loader,
  `sort_deterministically`.
- `framer-solver/src/lib.rs` — `generate_joist_plan` (lines ~816–940, the shared flat generator;
  emits no `SlopedPlacement`), `generate_ceiling_plan` (hardcodes `SpanDirection::Shorter`, drops
  `ceiling.slope`), `generate_roof_plan` / `generate_roof_plans` / `ridge_condition` /
  `varying_plate_height` (lines ~1093–1402; `carries_ridge` always emits `RidgeBoard` +
  `roof.ridge.no-tie`), `MemberKind` (no hip/valley/jack), `SlopedPlacement`, `RoofPlaneGeometry`,
  `add_join_members`.
- `framer-render/src/build.rs` — `push_roof_plane` (lifts via `plane.frame()`), `push_ceiling` /
  `push_floor_deck` / `push_horizontal_surface` (constant `z`, ignore slope), `surface_material`
  (`SurfaceFace::{Roof, Ceiling, Floor}`); `scenes.rs::roofed_scene`, `tests/golden.rs`.
- `framer-app/src/app/` — `mod.rs` (`RoofForm { Gable, Shed }`, `footprint_roof_specs`,
  `add_roof`, `Selection` / `ViewClick`, `ViewportMode::RoofPlan`), `panels.rs` (inspector arms;
  ceiling slope shown "flat" read-only), `viewport/scene_build.rs` (`roof_plane_outline_world`
  lift, flat `lift_outline` for ceilings/floors), `viewport/plan.rs`.
- `framer-app/tests/gpu_parity.rs` — `wgsl_pathtracer_matches_cpu_roofed`.

---

## Phase A — Sloped ceilings + ridge structural fork

### Slice A1 — Cathedral finish + the tie-detection ridge fork (no schema change)

The highest-value, lowest-risk slice: pure solver/render judgment over the existing v11 model.

- **Task A1.1** — Tie detection in the solver. Add a helper `roof_has_ceiling_tie(model, plane)`
  that reports whether a horizontal tie resists rafter thrust **at the bearing/plate line**
  (`level.elevation + level.height`): a **flat** `Ceiling` enclosing the plane's footprint at/near
  that elevation (ceilings hang from the level top, so a ceiling with a small `height` sits at the
  plate). A `FloorDeck` is **not** a tie by default — it resolves at `level.elevation`, the floor,
  not the plate — so it qualifies only when its elevation matches the bearing line; a sloped
  (scissor) ceiling is not a full tie. (Explicit collar/rafter ties are a later, M4-adjacent
  addition.) In `generate_roof_plan`, replace the unconditional
  `roof.ridge.no-tie` warning with the fork: **tied** → keep the `RidgeBoard`, emit an `Info`
  `roof.ridge.tied` ("a ridge board is adequate where a continuous tie resists rafter thrust");
  **untied** (cathedral / scissor / no deck) → emit `Unsupported` `roof.ridge.beam-required`
  ("no ceiling-joist or rafter tie resists thrust at the plate; a structural ridge beam is
  required — v1 frames a ridge board and does not size the beam"). Keep `RidgeBoard` geometry
  either way (sizing the beam is M4). Thread the model/region into `generate_roof_plan` (today it
  only gets the plane); follow the `generate_roof_plans` wrapper that already owns `model`.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver` (new unit tests: gable + flat ceiling → `tied`/Info;
    gable + no ceiling → `beam-required`/Unsupported; shed → neither, as today)
  - Commit: `feat(solver): ridge-board-vs-beam fork from ceiling-tie detection`
- **Task A1.2** — Cathedral underside finish — in the **shared core API and both consumers**.
  When a roof plane covers a region with **no** `Ceiling`, the room sees the roof's *interior*
  finish, not its weather face. The face lookup lives in `framer-core`
  (`ConstructionSystem::surface_finish_material`, which today always returns the roof **weather**
  face) and is called by both the path-tracer builder *and* the interactive viewport mesher — so a
  render-only change would leave the 3-D view still showing roofing underneath. Add an underside
  (conditioned-side / innermost) face to that core helper, then route the cathedral underside
  through it in **both** `framer-render` and the app's `SurfaceFace` / `surface_color` path. Keep
  the weather face as-is. No GPU/WGSL change (opaque diffuse only).
  - Files: `crates/framer-core/src/model.rs` (the `surface_finish_material` face API),
    `crates/framer-render/src/build.rs`, `crates/framer-app/src/app/viewport/scene_build.rs`
  - Verify: `cargo test -p framer-core` (underside face resolves the interior layer);
    `cargo test -p framer-render` (cathedral scene shows the interior finish underside);
    `cargo test -p framer-app` + `--test gpu_parity` stays green; manual 3-D: cathedral underside
    matches the render
  - Commit: `feat: cathedral underside shows the roof interior finish (core + render + 3-D)`
- **Task A1.3** — Surface the cathedral/attic relationship as a derived diagnostic so Plan Mode
  can explain it: per roof region, `roof.ceiling.cathedral` (Info) when no ceiling exists, vs the
  attic case. (Optional but cheap; it makes A1.1's fork legible in the diagnostics panel.)
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver`
  - Commit: `feat(solver): classify cathedral vs attic roof regions`

### Slice A2 — Sloped-ceiling model + validation (schema v11 → v12)

- **Task A2.1** — Give `Ceiling.slope` a direction. A bare `Slope` is ambiguous without a low
  edge; add the minimal descriptor the frame needs — reuse the roof-plane shape: a low/spring
  reference for the region (recommended: a `low_edge` selector mirroring `RoofPlane.eave_edge`, or
  a `SpanDirection`-style downslope vector) so a sloped ceiling reuses a `RoofPlaneFrame`-style
  lift. Add the field `#[serde(default, skip_serializing_if = …)]` so flat ceilings stay
  byte-stable. (Resolve the exact field shape here — see the spec's open question.)
  - Files: `crates/framer-core/src/model.rs`, `crates/framer-core/src/lib.rs`
  - Verify: `cargo test -p framer-core` (serde round-trip with slope present and absent)
  - Commit: `feat(core): give sloped ceilings a downslope reference`
- **Task A2.2** — Validation: when `slope` is `Some`, require `slope.run > 0`, the low-edge index
  in range, and (if a non-flat ceiling) reject a degenerate frame — mirroring `RoofPlane`
  validation. Flat ceilings (`slope == None`) keep today's checks. New `ModelError` variants.
  - Files: `crates/framer-core/src/model.rs`
  - Verify: `cargo test -p framer-core` (per-rule reject/accept)
  - Commit: `feat(core): validate sloped ceilings, fail closed`
- **Task A2.3** — Schema **v11 → v12**: bump `PROJECT_SCHEMA_VERSION`; regenerate the three
  `examples/projects/*.framer`; update the version-pinned tests
  (`save_project_writes_schema_versioned_authored_model`,
  `load_project_rejects_old_schema_with_unsupported_version_error`);
  update `docs/project-files.md` and the schema invariant in `AGENTS.md` (v11→v12).
  - Files: `crates/framer-core/src/project.rs`, `examples/projects/*.framer`,
    `docs/project-files.md`, `AGENTS.md`
  - Verify: `cargo test -p framer-core` (byte-exact round-trip),
    `python3 scripts/check-markdown-links.py`
  - Commit: `feat(core): bump .framer schema to v12 for sloped ceilings`

### Slice A3 — Sloped joist generation

- **Task A3.1** — Teach the shared joist generator to slope. Extend `generate_joist_plan` (or a
  sloped sibling) to accept an optional `Slope` + downslope reference; when present, compute each
  joist's `SlopedPlacement { low_elevation, high_elevation }` and scale its `cut_length` by
  `slope_factor` (reuse the rafter math: `slope_factor` / `slope_ratio` already exist), keeping
  plan length for spacing/area. Rim/band members close the sloped ends; blocking sits on the
  sloped surface. Pass `ceiling.slope` through `generate_ceiling_plan` (drop the hardcoded flat
  assumption); floor decks stay flat.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver` (golden sloped-ceiling joist layout for a known pitch:
    joist count on plan spacing, true-vs-plan cut length, low/high elevations)
  - Commit: `feat(solver): frame sloped ceiling joists with true cut lengths`
- **Task A3.2** — Scissor/vault structural diagnostics: a scissor ceiling (sloped, shallower than
  the roof) leaves a partial tie — emit `Info`/`Unsupported` notes consistent with A1.1's fork
  (no full tie ⇒ ridge beam). Confirm the A1 fork reads a sloped ceiling as *not* a flat tie.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver`
  - Commit: `feat(solver): scissor/vault ceiling structural diagnostics`

### Slice A4 — Render the sloped ceiling

- **Task A4.1** — Lift sloped ceilings in both meshers. In `framer-render::push_ceiling`, when
  `ceiling.slope` is `Some`, lift each outline vertex via a `RoofPlaneFrame`-style
  `elevation_at` instead of a constant `z` (reuse/parallel `push_roof_plane`); flat ceilings keep
  `push_horizontal_surface`. Mirror in the app mesher `viewport/scene_build.rs` (the
  `roof_plane_outline_world` lift already exists; route ceilings through it when sloped). Grow the
  bounds `Aabb`. Add a sloped-ceiling (scissor) golden scene; keep `gpu_parity` green.
  - Files: `crates/framer-render/src/build.rs`, `crates/framer-render/src/scenes.rs`,
    `crates/framer-render/tests/golden.rs`, `crates/framer-app/src/app/viewport/scene_build.rs`
  - Verify: `cargo test -p framer-render` (`UPDATE_GOLDEN=1` to regen intentionally),
    `cargo test -p framer-app --test gpu_parity`
  - Commit: `feat(render): emit sloped ceiling surfaces via the shared frame lift`

### Slice A5 — Authoring + example

- **Task A5.1** — Inspector slope editor. Replace the read-only "Slope: flat" ceiling field with a
  pitch (rise/run) + downslope editor, gated so flat stays the default; every edit flows through
  `edit()`. Update the model-tree label to distinguish flat / sloped / (no-ceiling = cathedral).
  - Files: `crates/framer-app/src/app/panels.rs`, `crates/framer-app/src/app/mod.rs`
  - Verify: `cargo test -p framer-app`; manual: slope a single ceiling, see it tilt in 3-D, undo = one step
  - Commit: `feat(app): edit per-ceiling slope in the inspector`
- **Task A5.2** — One-click vault tool. A region-gated tool (like the room/ceiling tool) that, given
  an enclosed wall loop plus a ridge axis + pitch, **generates the two opposing sloped `Ceiling`
  planes** of a scissor/vault and writes them into the model as two editable ceilings in a single
  `edit()` (one undo step). New `Selection` / `ViewClick` plumbing as needed; default ridge axis =
  the region's longer span, default pitch from the active ceiling system. The cathedral case stays
  "no ceiling," so no tool is needed for it.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/plan.rs`
  - Verify: `cargo test -p framer-app` (vault = two opposing ceilings; undo = one step; round-trip);
    manual: one-click vault on a `demo-shell` room, see the two slopes meet at a ridge in 3-D
  - Commit: `feat(app): one-click vault tool that authors a scissor ceiling`
- **Task A5.3** — Example + docs: extend an example with a cathedral room and a vaulted (scissor)
  room (e.g. give `demo-shell` one cathedral bay + one vaulted bay); refresh BOM/render snapshots.
  Flip the spec's Phase A status; update `docs/code-map.md`.
  - Files: `examples/projects/*.framer`, solver/render snapshots, `docs/specs/ceilings-and-roofs.md`,
    `docs/code-map.md`
  - Verify: `cargo test --workspace`, `python3 scripts/check-markdown-links.py`
  - Commit: `feat(examples): cathedral + vaulted ceiling example`

---

## Phase B — Hip & valley roofs

> Builds on Phase A. The genuinely new capability is the **first non-opposing-plane roof**: a hip
> meets two adjacent planes along a sloped line; a valley meets two wings; jack rafters die into
> both. Sequenced rectangular-hip → jacks → L/T valley so each slice ships standalone geometry.

### Slice B1 — Hip member kinds + rectangular hip roof

- **Task B1.1** — `MemberKind::{HipRafter, ValleyRafter, JackRafter}` with the exhaustive-match
  updates the compiler forces (`label`, `member_svg_color`, app `member_color`). No framing yet.
  - Files: `crates/framer-solver/src/lib.rs`, `crates/framer-app/src/app/viewport/scene_build.rs`
  - Verify: `cargo build --workspace`
  - Commit: `feat(solver): reserve hip/valley/jack member kinds`
- **Task B1.2** — Per-edge roof form. Extend the roof tool: `RoofForm` gains `Hip`; for a
  rectangular footprint, `footprint_roof_specs` emits **four** planes (two trapezoids + two
  triangles) meeting at a central ridge with four hip lines. Write them as editable `RoofPlane`s
  (the model already stores planes; no new model type). The **per-edge gable/hip choice is a
  transient input of the roof tool at generation time only** (it lives in the tool's gesture
  state, like pitch and overhang defaults) — it is **not** persisted and **not** a per-edge
  inspector field, since that would reintroduce the parametric roof-assembly the store-planes
  decision (#4 / #11) rejects. After generation the user edits the resulting `RoofPlane`s
  directly (per-plane pitch, move the ridge); re-running the tool replaces the planes.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/panels.rs`
  - Verify: `cargo test -p framer-app`; manual: one-click hip roof on `demo-shell`, then edit a
    resulting plane's pitch
  - Commit: `feat(app): hip roof auto-from-footprint (rectangular)`
- **Task B1.3** — Hip rafters in the solver: a multi-plane post-pass (sibling of
  `add_join_members`) that, given adjacent planes sharing a sloped edge, emits the `HipRafter`
  running corner→ridge with its true sloped placement. Ridge board spans hip-to-hip.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver` (hip rafter count = 4; true length for a known pitch;
    ridge length = footprint length − 2 × inset)
  - Commit: `feat(solver): frame hip rafters and the shortened hip ridge`

### Slice B2 — Jack rafters

- **Task B2.1** — Jack rafters die into each hip: along each hip-bounded plane, the common rafters
  shorten progressively to meet the hip line. Emit `JackRafter`s with descending cut lengths
  (true sloped), replacing the common rafters that would overrun the hip.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: `cargo test -p framer-solver` (jack lengths descend linearly to the hip; none overrun)
  - Commit: `feat(solver): frame jack rafters against hips`

### Slice B3 — Valleys (L/T multi-wing footprints)

- **Task B3.1** — Detect interior valley lines where two roof wings meet on an L/T footprint
  (equal-pitch ⇒ the valley bisects in plan; unequal-pitch is flagged Unsupported in v2). Emit
  `ValleyRafter` along the valley with jacks dying into it from both wings. This is the
  straight-skeleton-adjacent geometry; keep it to equal-pitch right-angle wings and diagnose the
  rest.
  - Files: `crates/framer-solver/src/lib.rs`, `crates/framer-app/src/app/mod.rs` (footprint
    decomposition for the auto-tool)
  - Verify: `cargo test -p framer-solver` (L-footprint: one valley, symmetric jacks; unequal
    pitch ⇒ `roof.valley.unequal-pitch` Unsupported)
  - Commit: `feat(solver): frame valleys for equal-pitch L/T roofs`

### Slice B4 — Render hips/valleys + example + docs

- **Task B4.1** — Render the multi-plane hip/valley solids in both meshers (each plane already
  lifts via its frame; verify hips/valleys read cleanly in 3-D and the path tracer). Add a
  hip-roof golden scene; keep `gpu_parity` green.
  - Files: `crates/framer-render/src/scenes.rs`, `crates/framer-render/tests/golden.rs`,
    `crates/framer-app/src/app/viewport/scene_build.rs`
  - Verify: `cargo test -p framer-render`, `cargo test -p framer-app --test gpu_parity`
  - Commit: `feat(render): hip/valley roof golden scene`
- **Task B4.2** — Example + docs: a hip-roofed example; flip the spec to fully Implemented; update
  `docs/code-map.md` and the spec's out-of-scope list (hips/valleys move out).
  - Files: `examples/projects/*.framer`, `docs/specs/ceilings-and-roofs.md`, `docs/code-map.md`
  - Verify: `cargo test --workspace`, `python3 scripts/check-markdown-links.py`
  - Commit: `docs: record ceilings/roofs v2 (sloped ceilings, hips/valleys)`

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Plus feature-specific checks: byte-exact `.framer` round-trip (the three examples), golden render
regen (`UPDATE_GOLDEN=1 cargo test -p framer-render --test golden`), GPU↔CPU parity
(`cargo test -p framer-app --test gpu_parity`), and a manual 3-D pass via the `install-app`
skill (slope a ceiling and see it tilt; place a hip roof and see jacks die into the hips; confirm
the ridge-beam diagnostic fires for a cathedral). When each phase lands, update the spec's
**Status** / **Last reviewed** and the affected docs.
