# Framer Code Map

A navigation guide for developers and agents: **where things live, how the crates
fit together, the key types, the data-flow, and where to add common changes.**

This is the *concrete* companion to the conceptual [architecture.md](architecture.md).
Architecture explains the layering and product intent; this document points you at the
actual files, types, and functions. When the two disagree, the code wins — fix the doc.

## Workspace at a glance

Six crates in a strict dependency order (UI depends on logic, never the reverse):

```
framer-core   ─┬─→ framer-solver ─┬─→ framer-app
               │                  └─→ framer-standards
               ├─→ framer-render ─┤
               └─→ framer-library ─┘
```

| Crate | Responsibility | Depends on | UI? |
| --- | --- | --- | --- |
| [`framer-core`](../crates/framer-core) | Domain model: authored building intent, units, construction systems, materials, furnishings/MEP objects, standards packs, validation, room topology, `.framer` serialization. | — | No |
| [`framer-library`](../crates/framer-library) | Library resolution, exact content hashing, vendor-on-use import/remap, and update-lifecycle operations for `.framerlib` content. | `framer-core` | No |
| [`framer-solver`](../crates/framer-solver) | Deterministic framing generation + takeoffs (members, per-layer BOM, room schedule, diagnostics) and SVG/CSV exports. | `framer-core` | No |
| [`framer-standards`](../crates/framer-standards) | UI-free standards evaluator: compliance facts, Kleene logic, deterministic reports, CSV export, and diagnostics lowering. | core, solver | No |
| [`framer-render`](../crates/framer-render) | UI-agnostic CPU path tracer: extract a renderable scene from the model, build a BVH, path-trace it. Headless PNG CLI. | `framer-core` | No |
| [`framer-app`](../crates/framer-app) | Native desktop CAD shell (`eframe`/`egui` + `wgpu`): model tree, inspector, command surfaces, 2D/3D viewports, real-time GPU path-traced Render view. | core, library, solver, render | Yes |

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
| `src/model.rs` | All domain types: `BuildingModel`, construction systems, materials, furnishing/MEP families and instances, walls, openings, joins, rooms, dimensions, standards-stack references, and `ModelError` validation. |
| `src/project.rs` | `.framer` serialization envelope: `ProjectDocument`, `load_project`/`save_project`, schema versioning + canonicalization. |
| `src/library.rs` | `.framerlib` serialization envelope: `LibraryDocument`, `Library`, `load_library`/`save_library`, schema versioning + canonicalization; also loads the checked-in starter catalog. |
| `src/standards.rs` | Standards Engine v1 authored data types: `StandardsPack`, `SiteContext`, prescriptive tables, compliance predicates, pack validation, the IRC 2021 starter pack, and pure stack resolution. |
| `src/topology.rs` | Derives room boundaries/areas from the wall graph; level-scoped room-boundary helpers for stacked drafting; `wall_interior_sides`. |
| `src/units.rs` | `Length` (integer **ticks**, 16 = 1 inch) and `Point2`. The basis of determinism. |
| `src/constraints.rs` | Generic linear-constraint layer (`ConstraintSystem`) for driving dimensions / overconstraint checks. |

### Key types (file: `src/model.rs` unless noted)

- **`BuildingModel`** — root authored container:
  `site: SiteContext`, `standards: Vec<ElementId>`,
  `standards_packs: Vec<StandardsPack>`, `libraries: Vec<LibraryStamp>`,
  `materials: Vec<Material>`, `systems: Vec<ConstructionSystem>`,
  `furnishings: Vec<Furnishing>`,
  `mep_objects: Vec<MepObject>`, `levels`, `walls`, `wall_joins`, `rooms`,
  `furnishing_instances: Vec<FurnishingInstance>`, `mep_instances: Vec<MepInstance>`,
  `roof_planes: Vec<RoofPlane>`, `ceilings: Vec<Ceiling>`,
  `floor_decks: Vec<FloorDeck>`, `braced_wall_lines: Vec<BracedWallLine>`.
  This is the only thing persisted.
- **`Wall`** — `id, name, level, start, end, length, height, system: ElementId, openings,
  dimensions, bracing, tags`. A wall references a construction system **by id** (it does not
  embed its assembly). `Wall::new` defaults `system` to `"system-wall-exterior-1"`.
- **`ConstructionSystem`** — `id, name, kind: SystemKind {Wall,Floor,Roof,Ceiling},
  source: Option<Provenance>, layers: Vec<ConstructionLayer>`. Layers are ordered
  **interior → exterior** and are *never sorted* (order is semantic). Helpers:
  `framing_layer()`, `total_thickness()`, `exposure()`, `r_value_milli(materials)`. All four
  kinds (`Wall`/`Floor`/`Roof`/`Ceiling`) are wired through authoring, the solver, and render.
- **`ConstructionLayer`** — `function: LayerFunction, material: ElementId, thickness,
  framing: Option<FramingSpec>`. `framing` is present **iff** `function == Framing`.
- **`FramingSpec`** — `member: BoardProfile, spacing, pattern: FramingPattern,
  cavity_material: Option<ElementId>` (cavity insulation adds no through-wall depth).
- **`Material`** — `id, name, source: MaterialSource {Project|Library(Provenance)}, appearance:
  Appearance, tags, properties: BTreeMap<String, PropertyValue>`. Open/extensible: substance
  lives in `properties` (e.g. `"r_per_inch_milli": Int`), not in the enum. `PropertyValue`
  is float-free (`Int|Length|Text|Flag`) so the model stays `Eq` + deterministic.
- **`AssetRef`** / **`TextureRole`** — hash-only references for binary material assets.
  `Appearance::Textured` and `Appearance::DepthMapped` carry a fallback color, `AssetRef`, and
  positive `Length` scale; asset bytes stay outside `.framer`.
- **`ObjectSize`**, **`Furnishing`**, **`MepObject`** (`MepObjectKind`), and placed
  **`FurnishingInstance`** / **`MepInstance`** — simple library-backed object families and
  level-owned plan placements. Instance `rotation` is a `QuarterTurn`; family sizes and all
  positions are tick-based.
- **`LibraryStamp`** / **`Provenance`** — descriptive library metadata for vendored
  definitions. It is never used to resolve walls/materials/systems/families at load or solve
  time.
- **`Opening`** (`OpeningKind`: Door/Window/GarageDoor/Skylight/Stair…), **`WallJoin`**
  (`WallJoinKind`: Corner/EndToEnd/Tee/Cross), **`Room`** (`RoomUsage`), **`Level`**,
  **`DimensionConstraint`** (+ anchor/axis/kind enums) — the rest of the authored model.
- **`RoofPlane`** / **`Ceiling`** / **`FloorDeck`** — the level-owned surfaces. A roof plane and a
  **sloped** `Ceiling` (`slope: Option<CeilingSlope { pitch: Slope, low_edge }>`) share one affine
  lift — `surface_frame(outline, slope, low_edge, reference_elevation) -> RoofPlaneFrame`, reached via
  `RoofPlane::frame` / `Ceiling::frame` — so the solver and both meshers project identically. A sloped
  ceiling needs a `Polygon` region (validation enforces it); `slope == None` is flat. A region with no
  `Ceiling` is a *cathedral* (`BuildingModel::roof_cathedral_flags`). See
  [ceilings-and-roofs.md](specs/ceilings-and-roofs.md).
- **`StandardsPack`** / **`FramingDefaults`** (`src/standards.rs`) — project-embedded standards
  data, starter framing defaults, prescriptive tables, checks, overlays, and stack resolution.
  `StandardsPack::irc_2021_starter()` is the built-in seed. Not a complete code-compliance
  engine.
- **`ElementId`** — stable semantic id (lowercase letters/digits/hyphens). **`ModelError`** —
  the validation error enum (dangling system refs, framing/layer mismatches, etc.).

### Entry points

- `BuildingModel::new()` / `demo_wall()` / `demo_shell()` / `demo_two_bedroom()` —
  construct models; `new`/demos seed the starter material + system library
  (`BuildingModel::starter_library()`), sourced from
  [`libraries/framer-starter.framerlib`](../libraries/framer-starter.framerlib), and seed the
  embedded IRC 2021 starter standards pack. Furnishing and MEP object families remain in the
  starter catalog until placed/imported.
- `BuildingModel::resolved_standards()` / `framing_defaults()` — resolve the ordered standards
  stack into the tables and defaults consumed by the solver and UI defaults.
- `BuildingModel::validate()` — full model validation (called before every save).
- `load_project(&str) -> BuildingModel` / `save_project(&BuildingModel) -> String` (`project.rs`).
- `load_library(&str) -> Library` / `save_library(&Library) -> String` (`library.rs`).
- `room_boundaries(model)` / `room_boundary(model, seed)` plus level-scoped
  `room_boundaries_on_level(model, level, seeds)` /
  `room_boundary_on_level(model, level, seed)` (`topology.rs`).
- Mutation helpers on `BuildingModel`: `move_wall_endpoint`, `translate_wall`, `remove_wall`,
  `reconcile_joins` (re-derive joins after geometry edits), `system_for(wall)`,
  `wall_envelope_span(wall)` (derived visual/render span that closes corner joins),
  `material(&ElementId)`, `sort_deterministically` / `into_deterministic`.

### `.framer` serialization (`src/project.rs`)

- Constants: `PROJECT_FORMAT = "framer.project"`, **`PROJECT_SCHEMA_VERSION = 13`**.
- The model is **v13-only**: `load_project` peeks a `SchemaHeader` first and returns
  `ProjectError::UnsupportedSchemaVersion` for any non-v13 file. `#[serde(deny_unknown_fields)]`
  rejects unknown keys.
- Canonical output: `to_canonical_json()` re-stamps the version, calls
  `sort_deterministically()` (sort by id; layer order preserved), pretty-prints, appends a
  trailing newline. Same model → byte-identical JSON regardless of in-memory order.
- Format + agent editing contract: [project-files.md](project-files.md).

### `.framerlib` serialization (`src/library.rs`)

- Constants: `LIBRARY_FORMAT = "framer.library"`, **`LIBRARY_SCHEMA_VERSION = 3`**.
- A library document is a headless, versioned catalog of typed definitions:
  `uid`, `version_id`, `version`, `coordinate`, `materials`, `systems`, `furnishings`, and
  `mep_objects`, plus `standards` packs.
- `load_library` peeks a header first, rejects non-library formats or unsupported
  schemas explicitly, then validates that every material/system/family/standards id is valid,
  every construction layer or cavity material reference resolves inside the library, and every
  standards pack validates. Furnishing and MEP family sizes must be positive.
- Canonical output mirrors project files: re-stamp the schema version, sort
  materials/systems/furnishings/MEP objects/standards packs by id, pretty-print, and append a
  trailing newline. Layer order remains semantic and is never sorted.
- The built-in starter catalog lives at
  [`libraries/framer-starter.framerlib`](../libraries/framer-starter.framerlib);
  `BuildingModel::new` and the demo constructors vendor those definitions into each
  self-contained project model.

---

## framer-library — reusable content resolution + vendoring

Headless, IO-capable support crate for the [Libraries spec](specs/libraries.md). It depends on
`framer-core` only among workspace crates; core stays IO-free.

| File | Contains |
| --- | --- |
| `src/lib.rs` | `Locator`, `LibraryResolver`, local/built-in/remote resolver, exact `blake3:<hex>` hashing, import functions, lifecycle diagnostics, re-sync/detach, content-addressed library and asset caches, and deterministic `.framerpkg` package IO. |

Key entry points:

- `starter_library()` — load the checked-in starter `.framerlib` and compute its canonical
  library-version hash.
- `load_verified_library(&LibraryBytes)` — parse a `.framerlib`, compute the canonical
  `blake3` hash, and fail closed if an expected hash is present and does not match.
- `import_material` / `import_system` / `import_furnishing` / `import_mep_object` /
  `import_standards_pack` /
  `import_item` — stage an atomic vendor-on-use import into a `BuildingModel`, mint
  project-local ids, copy referenced material closure for systems, stamp `LibraryStamp` +
  per-item `Provenance`, validate the staged model, then commit it. Standards packs are
  single-item copies with only `pack.id` remapped.
- `library_lifecycle_issues` — detect locally modified, out-of-date, or missing-source
  vendored materials/systems/furnishings/MEP objects/standards packs by recomputing
  provenance-excluded hashes in source id space.
- `resync_material` / `resync_system` / `resync_furnishing` / `resync_mep_object` /
  `resync_standards_pack` /
  `resync_item` — replace a vendored definition from an available library while preserving
  project-local ids; system re-sync also refreshes the referenced material closure.
- `detach_material` / `detach_system` / `detach_furnishing` / `detach_mep_object` /
  `detach_standards_pack` /
  `detach_item` — clear provenance on selected vendored definitions and prune unused library
  stamps.
- `ContentAddressedAssetStore` / `asset_content_hash` / `referenced_asset_hashes` — store and
  discover binary assets by full `blake3:<hex>` hash.
- `save_project_package` / `load_project_package` — write/read deterministic `.framerpkg`
  archives with `project.framer`, `manifest.json`, and `assets/blake3-<hex>` blobs.
- `RemoteLibraryCache` — stores canonical `.framerlib` snapshots at `blake3-<hex>.framerlib`
  and verifies cached bytes before use.
- `RemoteLibraryProvider` / `HttpRemoteLibraryProvider` — injected remote byte providers. The
  default provider fetches URL libraries with `ureq`; future managed/RPC providers plug into the
  same request/response seam.
- `LocalSearchPathResolver` — resolves built-in, local path, installed-library, and hash-pinned
  remote locators. Remote resolution requires a configured cache root, validates the full
  `blake3:<hex>` pin, prefers verified cache bytes, and fails closed on invalid URLs,
  unsupported schemes, fetch errors, non-UTF-8 bodies, or content hash mismatches.

---

## framer-solver — deterministic framing + takeoffs

One file, `src/lib.rs` (~2.6k lines). Pure function of the model: same input → same plan.

### Key types

- **`ProjectFramePlan`** — `wall_plans`, `floor_plans`, `ceiling_plans`, `roof_plans`,
  `rooms: Vec<RoomSchedule>`, `diagnostics`. Methods `bom() -> Vec<BomItem>` (member takeoff
  grouped by cut length) and `layer_bom() -> Vec<LayerBomItem>` (per-layer material takeoff,
  aggregated across walls and surfaces).
- **`WallFramePlan`** — `wall: ElementId`, `members: Vec<FrameMember>`, diagnostics; its own
  `bom()` / `layer_bom()`.
- **`FrameMember`** — one generated piece: `kind: MemberKind` (BottomPlate, TopPlate,
  CommonStud, KingStud, JackStud, Header, RoughSill, CrippleStud, CornerPost, Rafter,
  RidgeBoard, HipRafter, JackRafter, …),
  `profile: BoardProfile`, `orientation`, position/length, and `provenance: RuleProvenance`
  (rule id + human-readable why).
- **`LayerBomItem`** — per-layer material takeoff row (area goods / volumetric goods by
  material + function + thickness). **`BomItem`** — member cut-list row.
- **`RoomSchedule`** — derived room takeoff (area, perimeter, enclosed?).
- **`PlanDiagnostic`** / `DiagnosticSeverity`, **`SolverError`**.

### Entry points

- **`generate_project_plan(&BuildingModel) -> Result<ProjectFramePlan, SolverError>`** — the
  one entry the app calls. Validates, then per wall calls `generate_wall_plan`, adds join
  members (corner posts / partition + backing studs), generates floor/ceiling/roof plans, adds
  hip or valley rafters from shared roof-plane edges, frames jack rafters where hip/valley-bounded
  planes shorten, and builds the room schedule, per-layer BOM, and fastening BOM from
  construction systems and resolved standards.
- `generate_wall_plan(wall, code, system, materials)` — single-wall framing.
- Exports: `export_bom_csv`, `export_layer_bom_csv`, `export_room_schedule_csv`,
  `export_wall_elevation_svg`, `export_project_svg`.

---

## framer-standards — compliance evaluation

UI-agnostic and I/O-free. It evaluates authored standards checks against a
`BuildingModel`, resolved standards tables, and the regenerated `ProjectFramePlan`.

| File | Contains |
| --- | --- |
| `src/lib.rs` | `Tri` Kleene logic, `FactValue`, entity scoping, `fact_value`, `evaluate`, `ComplianceReport::to_csv`, and `diagnostics` lowering into solver diagnostics. |

### Entry points

- `evaluate(model, resolved, plan) -> ComplianceReport` — evaluates active, non-waived checks
  from the resolved standards stack and returns sorted, deterministic entries.
- `fact_value(fact, entity, model, resolved, plan)` — frozen v1 fact table for wall, opening,
  room, and placeholder braced-wall-line facts.
- `diagnostics(&report)` — lowers only violation, advisory, and needs-review outcomes into
  `PlanDiagnostic`; pass, waived, and not-applicable entries stay report-only.

---

## framer-render — CPU path tracer

UI-agnostic, `#![forbid(unsafe_code)]`. Deterministic: output is a pure function of the seed
and pixel/sample index (parallel == serial, byte-for-byte). The app's GPU compute shader
mirrors this exact math.

| File | Contains |
| --- | --- |
| `src/lib.rs` | Public API: `accumulate`, `tonemap_accum`, `render`; re-exports `build::*`. |
| `src/build.rs` | **Scene extraction from the model**: `scene_from_model`, `build_scene`, `RenderOptions`, `SceneFraming` (auto-derives cladding/drywall/glass/door/ground materials + sky + sun; wall solids use `BuildingModel::wall_envelope_span` so corner-joined walls close visually). |
| `src/scenes.rs` | Shared render-test fixtures: the synthetic reference scene plus model-derived gable, scissor-vault, and hip-roof scenes used by golden and parity tests. |
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
| `mod.rs` | **`FramerApp`** struct + `impl eframe::App` + `ui_root` (panel layout) + project save/load/export + plan regeneration + selection/undo wiring + command-search execution dispatch. Active drafting state (`active_level`, `ortho`, `snap_step`, `cursor_model`, `layers`) is presentation-only and reset/clamped with the current document. Region-gated placement tools — room / ceiling / **vault** (`add_vault` + `scissor_halves`) / floor — are mutually exclusive (`deactivate_placement_tools`), route through `ViewClick::Place*`, and resolve enclosed loops through the active level's wall graph; the roof tool (`add_roof` + `footprint_roof_specs`) auto-generates gable, shed, rectangular hip planes, and simple L-footprint valley planes. Standards authoring edit ops (project-local pack creation, starter-pack import, stack add/reorder/remove, and waive overlays) also live here so every mutation goes through undoable `edit()`. The derived compliance report is regenerated with each plan, lowered into plan diagnostics, and exported as a CSV sidecar. |
| `actions.rs` | UI-only command metadata (`ActionId`, labels, icons, tooltips, command-surface homes, workflow-strip tab/panel/flyout placement) for the command-surface migration. It is metadata only; model mutations still live on `FramerApp`. |
| `panels.rs` | Model tree, inspector, app header quick-access/actions menus, command-search modal, tabbed workflow command strip with insertion flyouts, status bar — the egui panel bodies. The status Level control and model-browser level rows activate the drafting level used by new level-owned objects. Command placement rules live in [command-surfaces.md](specs/command-surfaces.md). The ceiling inspector edits per-ceiling slope (pitch + low edge), converting a room region to a polygon on enable. The document-level Site & standards inspector edits `SiteContext`, stack order/membership, starter-pack import, project-local pack creation, and per-rule waiver reasons; the Plan inspector groups compliance report entries by outcome and lets report rows focus their source element when the app has a selectable authored context. |
| `model_edit.rs` | Authored-model mutation primitives (wall/opening drag state, constrained edits, id generation, including `next_standards_pack_id`). |
| `draw_wall.rs` | Draw-wall tool: snapping engine (`resolve_snap`) + same-level auto-join derivation. |
| `history.rs` | `History<Snapshot>` undo/redo stack (+ `history_integration_tests.rs`). |
| `project_io.rs` | File-write + export-path helpers (orchestration lives in `mod.rs`). |
| `render_job.rs` | Background-thread **CPU** render job (progressive accumulation, fallback path). |
| `labels.rs`, `theme.rs` | Human-readable type labels; legacy theme shim. |
| `ui_harness_tests.rs` | Headless `egui_kittest` UI smoke tests. |

### `src/app/design/` — design system

`mod.rs` (theme install + tokens), `tokens.rs`, `palette.rs`, `icons.rs` (Lucide icon font),
`widgets.rs` (semantic custom controls, including workflow tabs and command-strip panels). New UI
styling goes here, not inline; command routing policy belongs in
[command-surfaces.md](specs/command-surfaces.md).

### `src/app/viewport/` — the viewports (layered modules)

`workspace` (`viewport/mod.rs:91`) renders the workspace/view bar, contextual tool options
strip, selection context toolbar, and one viewport per frame based on `ViewportMode`. The
workspace/view bar owns Design/Plan switching and Shell/Plan, Wall/Elevation, Roof, 3D, and
Render view tabs; the tool options strip owns active Wall/Room/Ceiling/Vault/Floor/Dimension
placement context, including the active drafting level, and the selection context toolbar owns
selected-object lifecycle actions.

| File | Contains |
| --- | --- |
| `mod.rs` | `workspace` dispatcher + shared viewport input/header. |
| `plan.rs` | Top-down plan view: grid/rulers, walls, openings, placed furnishing/MEP footprints, selection context-toolbar anchors, draw-wall + room tools, endpoint drag, same-level room fills, and the wall display mode (outline/width/full) + layer-visibility guards. |
| `elevation_design.rs` | Single-wall elevation editor (openings + dimensions). |
| `elevation_framing.rs` | Plan-mode elevation overlay drawing generated members. |
| `elevation_openings.rs`, `elevation_dimensions.rs` | Opening edit handles; dimension drawing/anchors. |
| `scene_build.rs` | **`Scene3d::from_project`** — builds the 3D mesh + pick volumes from model + plan; wall envelopes use `BuildingModel::wall_envelope_span` so corner-joined walls close visually; `pick()` for selection. |
| `axonometric.rs`, `camera_2d.rs`, `camera_3d.rs`, `view_cube.rs`, `view_common.rs`, `geom.rs` | Ortho 3D view; 2D/3D cameras; view-cube widget; shared transforms/hit-tests. |
| `gpu.rs` | `wgpu` pipeline wrapper for the 3D scene. |
| `render.rs` | The path-traced **Render** view (orbit/dolly + progressive refinement). |

### `src/app/render/` — real-time GPU path tracer

`mod.rs` holds **`GpuRenderState`** (the WGSL compute path tracer driving the Render view) and
the shaders: `pathtrace.wgsl`, `blit.wgsl`, `denoise.wgsl`, `rng.wgsl`. **These mirror
`framer-render`'s CPU math exactly** — the CPU path is the reference; `tests/gpu_parity.rs`
validates equivalence. The default GPU backend traverses the uploaded flat BVH in WGSL; an
experimental native `wgpu` ray-query backend can be enabled with `FRAMER_RENDER_RAY_QUERY=1`
when the device exposes `EXPERIMENTAL_RAY_QUERY`, building a BLAS/TLAS from the same triangle
stream. Edit render math in both CPU and WGSL paths together.

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
| **New roof plane / ceiling / floor deck object** | the element collection (`roof_planes` / `ceilings` / `floor_decks`) + its struct in `framer-core/src/model.rs`, and a kind-matched `SystemKind` system | framing in `framer-solver/src/lib.rs` (`generate_roof_plan` / shared joisting generator); surface meshing in `framer-render/src/build.rs` + `viewport/scene_build.rs`; tools/tree/inspector in `framer-app` `panels.rs` + `viewport/plan.rs`; bump `PROJECT_SCHEMA_VERSION`; update [ceilings-and-roofs.md](specs/ceilings-and-roofs.md) + [project-files.md](project-files.md). |
| **New construction layer function / material** | `LayerFunction` / `Material` / `Appearance` in `model.rs`; seed it in `starter_library()` | per-layer BOM in `framer-solver` (`layer_bom`); appearance/material lowering in `framer-render/src/build.rs`; asset bytes via `framer-library` package/store helpers. |
| **New library item kind** | typed collection + validation in `framer-core/src/model.rs` / `library.rs` | add closure/remap support in `framer-library`; add browser/import/placement UI in `framer-app/src/app/panels.rs`; add drawing/picking in the relevant viewport; update [libraries.md](specs/libraries.md) and [project-files.md](project-files.md). |
| **New solver rule / member kind** | `MemberKind` + rule in `framer-solver/src/lib.rs`; the authoring-side family tag is `MemberFamily` on `FramingSpec` in `framer-core/src/model.rs` | the surface/wall generator passes the matching `MemberKind` explicitly (the solver does not dispatch on `member_family`); attach `RuleProvenance`; add a focused solver test; expect a diagnostic for unsupported cases. |
| **New viewport mode** | `ViewportMode` + a `match` arm in `viewport/mod.rs::workspace` | add a `viewport/<mode>.rs` returning a `ViewClick`. |
| **New view layer / wall display mode** | `ViewLayers` / `WallDisplay` in `app/mod.rs`; toggle in the Layers popover (`panels.rs`) | gate the render in `viewport/plan.rs` (and `scene_build.rs` for 3D); session-only, not persisted. See [view-layers.md](specs/view-layers.md). |
| **Schema change** | bump `PROJECT_SCHEMA_VERSION` in `project.rs`; add types in `model.rs` | update the three `examples/projects/*.framer` (round-trip tests are byte-exact); update [project-files.md](project-files.md); add a rejection/round-trip test. |
| **New UI control / styling** | `framer-app/src/app/design/` | use semantic tokens; don't hard-code colors inline. |
| **Render math change** | `framer-render` integrator/material/build scene (CPU = reference) | mirror it in `app/render/*.wgsl`; keep `tests/gpu_parity.rs` green. |

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
determinism + BOM (`framer-solver/src/lib.rs`), shared render fixtures
(`framer-render/src/scenes.rs`, including the hip-roof scene), golden render
(`framer-render/tests/golden.rs`, regen with `UPDATE_GOLDEN=1`), GPU↔CPU parity
(`framer-app/tests/gpu_parity.rs`), headless UI (`framer-app/src/app/ui_harness_tests.rs`).
