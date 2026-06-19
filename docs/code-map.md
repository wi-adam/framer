# Framer Code Map

A navigation guide for developers and agents: **where things live, how the crates
fit together, the key types, the data-flow, and where to add common changes.**

This is the *concrete* companion to the conceptual [architecture.md](architecture.md).
Architecture explains the layering and product intent; this document points you at the
actual files, types, and functions. When the two disagree, the code wins — fix the doc.

## Workspace at a glance

Four crates in a strict dependency order (UI depends on logic, never the reverse):

```
framer-core   ─┬─→ framer-solver ─┐
               ├─→ framer-render ─┼─→ framer-app
               └──────────────────┘
```

| Crate | Responsibility | Depends on | UI? |
| --- | --- | --- | --- |
| [`framer-core`](../crates/framer-core) | Domain model: authored building intent, units, construction systems, materials, code profiles, validation, room topology, `.framer` serialization. | — | No |
| [`framer-solver`](../crates/framer-solver) | Deterministic framing generation + takeoffs (members, per-layer BOM, room schedule, diagnostics) and SVG/CSV exports. | `framer-core` | No |
| [`framer-render`](../crates/framer-render) | UI-agnostic CPU path tracer: extract a renderable scene from the model, build a BVH, path-trace it. Headless PNG CLI. | `framer-core` | No |
| [`framer-app`](../crates/framer-app) | Native desktop CAD shell (`eframe`/`egui` + `wgpu`): model tree, inspector, 2D/3D viewports, real-time GPU path-traced Render view. | core, solver, render | Yes |

**The load-bearing invariant:** `framer-core`, `framer-solver`, and `framer-render` carry
**no UI dependency**. They must stay testable, scriptable, and exportable without the app.
See [architecture.md](architecture.md#workspace) and
[vision.md](vision.md#product-principles).

---

## framer-core — the domain model

UI-agnostic source of truth. Everything else derives from a `BuildingModel`.

| File | Contains |
| --- | --- |
| `src/lib.rs` | Module wiring + the crate's public API (the `pub use` re-export list *is* the public surface). |
| `src/model.rs` | All domain types: `BuildingModel`, construction systems, materials, walls, openings, joins, rooms, dimensions, code profiles, and `ModelError` validation. (~3.8k lines.) |
| `src/project.rs` | `.framer` serialization envelope: `ProjectDocument`, `load_project`/`save_project`, schema versioning + canonicalization. |
| `src/topology.rs` | Derives room boundaries/areas from the wall graph; `wall_interior_sides`. |
| `src/units.rs` | `Length` (integer **ticks**, 16 = 1 inch) and `Point2`. The basis of determinism. |
| `src/constraints.rs` | Generic linear-constraint layer (`ConstraintSystem`) for driving dimensions / overconstraint checks. |

### Key types (file: `src/model.rs` unless noted)

- **`BuildingModel`** — root authored container:
  `code: CodeProfile`, `materials: Vec<Material>`, `systems: Vec<ConstructionSystem>`,
  `levels`, `walls`, `wall_joins`, `rooms`. This is the only thing persisted.
- **`Wall`** — `id, name, level, start, end, length, height, system: ElementId, openings,
  dimensions, tags`. A wall references a construction system **by id** (it does not embed its
  assembly). `Wall::new` defaults `system` to `"system-wall-exterior-1"`.
- **`ConstructionSystem`** — `id, name, kind: SystemKind {Wall,Floor,Roof}, layers:
  Vec<ConstructionLayer>`. Layers are ordered **interior → exterior** and are *never sorted*
  (order is semantic). Helpers: `framing_layer()`, `total_thickness()`, `exposure()`,
  `r_value_milli(materials)`. Only `Wall` systems are wired today.
- **`ConstructionLayer`** — `function: LayerFunction, material: ElementId, thickness,
  framing: Option<FramingSpec>`. `framing` is present **iff** `function == Framing`.
- **`FramingSpec`** — `member: BoardProfile, spacing, pattern: FramingPattern,
  cavity_material: Option<ElementId>` (cavity insulation adds no through-wall depth).
- **`Material`** — `id, name, source: MaterialSource {Project|External}, appearance:
  Appearance, tags, properties: BTreeMap<String, PropertyValue>`. Open/extensible: substance
  lives in `properties` (e.g. `"r_per_inch_milli": Int`), not in the enum. `PropertyValue`
  is float-free (`Int|Length|Text|Flag`) so the model stays `Eq` + deterministic.
- **`Opening`** (`OpeningKind`: Door/Window/GarageDoor/Skylight/Stair…), **`WallJoin`**
  (`WallJoinKind`: Corner/EndToEnd/Tee/Cross), **`Room`** (`RoomUsage`), **`Level`**,
  **`DimensionConstraint`** (+ anchor/axis/kind enums) — the rest of the authored model.
- **`CodeProfile`** / `PrescriptiveCode::Irc2021` — starter prescriptive defaults
  (`irc_2021_prescriptive()`). Not a complete code-compliance engine.
- **`ElementId`** — stable semantic id (lowercase letters/digits/hyphens). **`ModelError`** —
  the validation error enum (dangling system refs, framing/layer mismatches, etc.).

### Entry points

- `BuildingModel::new(code)` / `demo_wall()` / `demo_shell()` / `demo_two_bedroom()` —
  construct models; `new`/demos seed the starter material + system library
  (`BuildingModel::starter_library()`).
- `BuildingModel::validate()` — full model validation (called before every save).
- `load_project(&str) -> BuildingModel` / `save_project(&BuildingModel) -> String` (`project.rs`).
- `room_boundaries(model)` / `room_boundary(model, seed)` (`topology.rs`).
- Mutation helpers on `BuildingModel`: `move_wall_endpoint`, `translate_wall`, `remove_wall`,
  `reconcile_joins` (re-derive joins after geometry edits), `system_for(wall)`,
  `material(&ElementId)`, `sort_deterministically` / `into_deterministic`.

### `.framer` serialization (`src/project.rs`)

- Constants: `PROJECT_FORMAT = "framer.project"`, **`PROJECT_SCHEMA_VERSION = 7`**.
- The model is **v7-only**: `load_project` peeks a `SchemaHeader` first and returns
  `ProjectError::UnsupportedSchemaVersion` for any non-v7 file (no in-place migration of
  pre-v7 shapes). `#[serde(deny_unknown_fields)]` rejects unknown keys.
- Canonical output: `to_canonical_json()` re-stamps the version, calls
  `sort_deterministically()` (sort by id; layer order preserved), pretty-prints, appends a
  trailing newline. Same model → byte-identical JSON regardless of in-memory order.
- Format + agent editing contract: [project-files.md](project-files.md).

---

## framer-solver — deterministic framing + takeoffs

One file, `src/lib.rs` (~2.6k lines). Pure function of the model: same input → same plan.

### Key types

- **`ProjectFramePlan`** — `wall_plans: Vec<WallFramePlan>`, `rooms: Vec<RoomSchedule>`,
  `diagnostics`. Methods `bom() -> Vec<BomItem>` (member takeoff grouped by cut length) and
  `layer_bom() -> Vec<LayerBomItem>` (per-layer material takeoff, aggregated across walls).
- **`WallFramePlan`** — `wall: ElementId`, `members: Vec<FrameMember>`, diagnostics; its own
  `bom()` / `layer_bom()`.
- **`FrameMember`** — one generated piece: `kind: MemberKind` (BottomPlate, TopPlate,
  CommonStud, KingStud, JackStud, Header, RoughSill, CrippleStud, CornerPost, …),
  `profile: BoardProfile`, `orientation`, position/length, and `provenance: RuleProvenance`
  (rule id + human-readable why).
- **`LayerBomItem`** — per-layer material takeoff row (area goods / volumetric goods by
  material + function + thickness). **`BomItem`** — member cut-list row.
- **`RoomSchedule`** — derived room takeoff (area, perimeter, enclosed?).
- **`PlanDiagnostic`** / `DiagnosticSeverity`, **`SolverError`**.

### Entry points

- **`generate_project_plan(&BuildingModel) -> Result<ProjectFramePlan, SolverError>`** — the
  one entry the app calls. Validates, then per wall calls `generate_wall_plan`, adds join
  members (corner posts / partition + backing studs), and builds the room schedule + per-layer
  BOM from the wall's `ConstructionSystem`.
- `generate_wall_plan(wall, code, system, materials)` — single-wall framing.
- Exports: `export_bom_csv`, `export_layer_bom_csv`, `export_room_schedule_csv`,
  `export_wall_elevation_svg`, `export_project_svg`.

---

## framer-render — CPU path tracer

UI-agnostic, `#![forbid(unsafe_code)]`. Deterministic: output is a pure function of the seed
and pixel/sample index (parallel == serial, byte-for-byte). The app's GPU compute shader
mirrors this exact math.

| File | Contains |
| --- | --- |
| `src/lib.rs` | Public API: `accumulate`, `tonemap_accum`, `render`; re-exports `build::*`. |
| `src/build.rs` | **Scene extraction from the model**: `scene_from_model`, `build_scene`, `RenderOptions`, `SceneFraming` (auto-derives cladding/drywall/glass/door/ground materials + sky + sun). |
| `src/scene.rs` | `Scene`, lighting (`DirectionalSun`, `Sky`). |
| `src/bvh.rs`, `src/aabb.rs`, `src/geom.rs`, `src/ray.rs` | BVH acceleration + geometry/ray primitives. |
| `src/integrator.rs` | Path-tracing integrator + BSDF evaluation (the reference for the WGSL kernel). |
| `src/material.rs` | Render material enum (Diffuse / Metal / Dielectric glass). |
| `src/rng.rs` | `Pcg32` + `pixel_rng` (per-pixel independent streams) + stratified jitter. |
| `src/sampling.rs`, `src/color.rs`, `src/camera.rs`, `src/math/` | Sampling, ACES tone-map, camera, `Vec3`/`Onb`. |
| `src/gpu.rs` | `bytemuck` GPU-mirror structs shared with the app's WGSL shaders. |
| `src/bin/` | The headless `render` CLI (feature `cli`). |

### Entry points

- `scene_from_model(model, &RenderOptions) -> Scene` — build the renderable scene.
- `accumulate(...)` — add samples-per-pixel into an HDR accumulator (deterministic).
- `tonemap_accum(accum, total_samples, exposure) -> Vec<u8>` and `render(scene, w, h, spp,
  seed) -> Vec<u8>` (accumulate + tonemap convenience).
- Features: `cli` (enables the `render` bin + `parallel`), `parallel` (rayon).

---

## framer-app — desktop CAD shell

`eframe`/`egui` + `wgpu`. Holds the authored model, caches the solver plan, and renders the
2D/3D/Render viewports. Entry: `src/main.rs` → `FramerApp` (`src/app/mod.rs:42`).

### `src/app/` top-level modules

| File | Contains |
| --- | --- |
| `mod.rs` | **`FramerApp`** struct + `impl eframe::App` + `ui_root` (panel layout) + project save/load/export + plan regeneration + selection/undo wiring. |
| `panels.rs` | Model tree, inspector, toolbar, status bar — the egui panel bodies. |
| `model_edit.rs` | Authored-model mutation primitives (wall/opening drag state, constrained edits, id generation). |
| `draw_wall.rs` | Draw-wall tool: snapping engine (`resolve_snap`) + auto-join derivation. |
| `history.rs` | `History<Snapshot>` undo/redo stack (+ `history_integration_tests.rs`). |
| `project_io.rs` | File-write + export-path helpers (orchestration lives in `mod.rs`). |
| `render_job.rs` | Background-thread **CPU** render job (progressive accumulation, fallback path). |
| `labels.rs`, `theme.rs` | Human-readable type labels; legacy theme shim. |
| `ui_harness_tests.rs` | Headless `egui_kittest` UI smoke tests. |

### `src/app/design/` — design system

`mod.rs` (theme install + tokens), `tokens.rs`, `palette.rs`, `icons.rs` (Lucide icon font),
`widgets.rs` (semantic custom controls). New UI styling goes here, not inline.

### `src/app/viewport/` — the viewports (layered modules)

`workspace` (`viewport/mod.rs:91`) routes to one viewport per frame based on `ViewportMode`.

| File | Contains |
| --- | --- |
| `mod.rs` | `workspace` dispatcher + shared viewport input/header. |
| `plan.rs` | Top-down plan view: grid/rulers, walls, openings, draw-wall + room tools, endpoint drag, and the wall display mode (outline/width/full) + layer-visibility guards. |
| `elevation_design.rs` | Single-wall elevation editor (openings + dimensions). |
| `elevation_framing.rs` | Plan-mode elevation overlay drawing generated members. |
| `elevation_openings.rs`, `elevation_dimensions.rs` | Opening edit handles; dimension drawing/anchors. |
| `scene_build.rs` | **`Scene3d::from_project`** — builds the 3D mesh + pick volumes from model + plan; `pick()` for selection. |
| `axonometric.rs`, `camera_2d.rs`, `camera_3d.rs`, `view_cube.rs`, `view_common.rs`, `geom.rs` | Ortho 3D view; 2D/3D cameras; view-cube widget; shared transforms/hit-tests. |
| `gpu.rs` | `wgpu` pipeline wrapper for the 3D scene. |
| `render.rs` | The path-traced **Render** view (orbit/dolly + progressive refinement). |

### `src/app/render/` — real-time GPU path tracer

`mod.rs` holds **`GpuRenderState`** (the WGSL compute path tracer driving the Render view) and
the shaders: `pathtrace.wgsl`, `blit.wgsl`, `denoise.wgsl`, `rng.wgsl`. **These mirror
`framer-render`'s CPU math exactly** — the CPU path is the reference; `tests/gpu_parity.rs`
validates equivalence. Edit both together.

---

## Data flow: authored intent → derived → presentation

This is the three-layer model from [architecture.md](architecture.md#modeling-layers), with
the real symbols:

```
                  user edits (viewport tools / inspector)
                              │
   ┌──────────────────────────▼───────────────────────────┐
   │ INTENT  BuildingModel  (framer-core::model)           │  ← only thing saved (.framer)
   │   walls→system(ElementId)→ConstructionSystem→layers   │
   └──────────────────────────┬───────────────────────────┘
                              │  framer_solver::generate_project_plan(&model)
   ┌──────────────────────────▼───────────────────────────┐
   │ DERIVED  ProjectFramePlan  (framer-solver)            │  ← regenerated, never saved
   │   FrameMember[] · LayerBomItem[] · RoomSchedule[]     │
   └───────────┬───────────────────────────┬──────────────┘
               │                           │
   scene_build::Scene3d::from_project   framer_render::scene_from_model + accumulate
   (3D mesh + pick volumes)             (path-traced Render view; GPU mirror in app/render)
               │                           │
   ┌───────────▼───────────────────────────▼──────────────┐
   │ PRESENTATION  viewports, drawings, SVG/CSV exports     │  ← disposable artifacts
   └────────────────────────────────────────────────────────┘
```

- **Authoring** edits `FramerApp.model` only; geometry edits call `reconcile_joins`; edits are
  snapshotted into `History` on interaction end.
- **Solving** runs `generate_project_plan` and caches the `ProjectFramePlan` on the app.
- **Presentation** never becomes the source of truth. A change to derived/presented state must
  flow back into authored intent (or a future explicit override record), per the
  [Mode Contract](architecture.md#mode-contract).

---

## Where do I add X?

| Goal | Touch | Then |
| --- | --- | --- |
| **New opening kind** | `OpeningKind` in `framer-core/src/model.rs` | framing rules in `framer-solver` `generate_wall_plan`; render/3D handling in `framer-render/src/build.rs` + `viewport/scene_build.rs`; inspector in `framer-app` `panels.rs`. |
| **New construction layer function / material** | `LayerFunction` / `Material` in `model.rs`; seed it in `starter_library()` | per-layer BOM in `framer-solver` (`layer_bom`); appearance/material in `framer-render/src/build.rs`. |
| **New solver rule / member** | `MemberKind` + rule in `framer-solver/src/lib.rs` | attach `RuleProvenance`; add a focused solver test; expect a diagnostic for unsupported cases. |
| **New viewport mode** | `ViewportMode` + a `match` arm in `viewport/mod.rs::workspace` | add a `viewport/<mode>.rs` returning a `ViewClick`. |
| **New view layer / wall display mode** | `ViewLayers` / `WallDisplay` in `app/mod.rs`; toggle in the Layers popover (`panels.rs`) | gate the render in `viewport/plan.rs` (and `scene_build.rs` for 3D); session-only, not persisted. See [view-layers.md](specs/view-layers.md). |
| **Schema change** | bump `PROJECT_SCHEMA_VERSION` in `project.rs`; add types in `model.rs` | update the three `examples/projects/*.framer` (round-trip tests are byte-exact); update [project-files.md](project-files.md); add a rejection/round-trip test. |
| **New UI control / styling** | `framer-app/src/app/design/` | use semantic tokens; don't hard-code colors inline. |
| **Render math change** | `framer-render` integrator/material (CPU = reference) | mirror it in `app/render/*.wgsl`; keep `tests/gpu_parity.rs` green. |

---

## Tests & verification

See [project-files.md](project-files.md#determinism) for the file-format contract and the
repo [README](../README.md#test) / [CONTRIBUTING](../CONTRIBUTING.md) for the full gate. The
short version, run from the workspace root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Notable test suites: schema round-trip + rejection (`framer-core/src/project.rs`), solver
determinism + BOM (`framer-solver/src/lib.rs`), golden render (`framer-render/tests/golden.rs`,
regen with `UPDATE_GOLDEN=1`), GPU↔CPU parity (`framer-app/tests/gpu_parity.rs`), headless UI
(`framer-app/src/app/ui_harness_tests.rs`).
