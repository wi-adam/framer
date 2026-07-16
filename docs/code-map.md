# Framer Code Map

A navigation guide for developers and agents: **where things live, how the crates
fit together, the key types, the data-flow, and where to add common changes.**

This is the *concrete* companion to the conceptual [architecture.md](architecture.md).
Architecture explains the layering and product intent; this document points you at the
actual files, types, and functions. When the two disagree, the code wins — fix the doc.

## Workspace at a glance

Seven crates in a strict dependency order (UI depends on logic, never the reverse):

```
framer-core   ─┬─→ framer-solver ─┬─→ framer-geometry ─→ framer-app
               │                  └─→ framer-standards
               ├─→ framer-render ───────────────────────→┤
               └─→ framer-library ──────────────────────→┘
```

| Crate | Responsibility | Depends on | UI? |
| --- | --- | --- | --- |
| [`framer-core`](../crates/framer-core) | Domain model: authored building intent, units, construction systems, materials, furnishings/MEP objects, standards packs, validation, room topology, `.framer` serialization. | — | No |
| [`framer-library`](../crates/framer-library) | Library resolution, exact content hashing, vendor-on-use import/remap, and update-lifecycle operations for `.framerlib` content. | `framer-core` | No |
| [`framer-solver`](../crates/framer-solver) | Deterministic framing generation + takeoffs (members, per-layer BOM, room schedule, diagnostics) and SVG/CSV exports. | `framer-core` | No |
| [`framer-standards`](../crates/framer-standards) | UI-free standards evaluator: compliance facts, Kleene logic, deterministic reports, CSV export, and diagnostics lowering. | core, solver | No |
| [`framer-geometry`](../crates/framer-geometry) | UI-free physical scene: stable body identity, exact generated-member solids, finished assembly envelopes, and convex-piece lowering. | core, solver | No |
| [`framer-render`](../crates/framer-render) | UI-agnostic CPU path tracer: extract a renderable scene from the model, build a BVH, path-trace it. Headless PNG CLI. | `framer-core` | No |
| [`framer-app`](../crates/framer-app) | Native desktop CAD shell (`eframe`/`egui` + `wgpu`): model tree, inspector, command surfaces, 2D/3D viewports, real-time GPU path-traced Render view. | core, geometry, library, solver, render | Yes |

**The load-bearing invariant:** `framer-core`, `framer-solver`, `framer-geometry`, and `framer-render` carry
**no UI dependency**. They must stay testable, scriptable, and exportable without the app.
See [architecture.md](architecture.md#workspace) and
[vision.md](vision.md#product-principles).

---

## framer-geometry — shared physical solids

UI-free floating-point geometry derived from `BuildingModel` plus
`ProjectFramePlan`. It does not alter semantic solver endpoints or persist any
state.

| File | Contains |
| --- | --- |
| `src/solid.rs` | `PhysicalScene`, `PhysicalBody`, canonical `BodyRef`, collision domains/body kinds, indexed surface meshes, convex pieces, AABBs, and fail-closed build diagnostics. |
| `src/build/members.rs` | Wall-local cuboids, arbitrary spatial boards, rake plates, floor/ceiling members, and exact common-rafter profiles including birdsmouths and ridge-face setbacks. |
| `src/build/assemblies.rs` | Lapped/cavity-cut wall envelopes plus floor, ceiling, and overhung roof assembly bodies, including roof-opening cavities. |
| `src/build/mod.rs` | `build_physical_scene(model, plan)`, deterministic orchestration, shared math, and whole-scene inventory coverage. |
| `src/spatial.rs`, `src/query.rs`, `src/audit.rs`, `src/diagnostic.rs` | R-tree candidate enumeration, Parry convex contact queries, scale-aware penetration classification, canonical structured violations, and stable ordering. |
| `src/bin/geometry-audit.rs` | Headless `.framer` audit command with stable output and clean/violation/input-error exit codes. |

The app consumes each generated member body's indexed surface for both triangles
and picking. Assembly presentations continue using their existing core-derived
layer/material paths; parity tests keep those occupied boundaries aligned with
the geometry bodies.

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
  `Ceiling` is a *cathedral* (`BuildingModel::roof_cathedral_flags`).
  `BuildingModel::roof_surface_outline` derives the visible/takeoff polygon by offsetting the
  authored bearing outline's eave and exposed rakes while keeping shared roof seams fixed;
  `BuildingModel::roof_surface_triangulation` adds stable hole-aware triangles for modeled roof
  openings so the geometry audit, 3-D viewport/picking, and path tracer share the same cavities;
  `BuildingModel::connected_roof_plane_ids` defines the exact-edge component whose matching
  overhang pair keeps those seam endpoints watertight, while roof validation rejects negative
  overhangs and redundant duplicate/collinear boundary vertices;
  `BuildingModel::gable_wall_profiles` derives simple triangular end-wall infill from the original
  bearing edges. Neither helper adds persisted intent or changes the project schema. See
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
- `BuildingModel::extend_collinear_wall()` — conservatively absorbs a newly drawn
  straight continuation into one compatible authored wall, preserving world-space
  opening/bracing placement and refusing ambiguous or driving-dimension-breaking
  merges; the app follows it with `reconcile_joins()` before regeneration.
- `load_project(&str) -> BuildingModel` / `save_project(&BuildingModel) -> String` (`project.rs`).
- `load_library(&str) -> Library` / `save_library(&Library) -> String` (`library.rs`).
- `room_boundaries(model)` / `room_boundary(model, seed)` plus level-scoped
  `room_boundaries_on_level(model, level, seeds)` /
  `room_boundary_on_level(model, level, seed)` (`topology.rs`).
- Mutation helpers on `BuildingModel`: `move_wall_endpoint`, `translate_wall`, `remove_wall`,
  `reconcile_joins` (re-derive joins after geometry edits), `system_for(wall)`,
  `wall_envelope_span(wall)` (half-tick visual/render through-butt span),
  `wall_framing_span(wall)` / `wall_counter_lap_framing_span(wall)` (primary structural span
  plus the upper-top-plate counter-lap),
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
  CommonStud, GableStud, RakePlate, KingStud, JackStud, Header, RoughSill, CrippleStud,
  CornerPost, Rafter, RidgeBoard, HipRafter, JackRafter, …), `profile: BoardProfile`,
  `orientation`, position/length, and `provenance: RuleProvenance` (rule id + human-readable why).
  Sloped members carry exact plan `start`/`end` points and their building elevations in
  `SlopedPlacement`, so common/ridge/hip/valley/rake members have one renderer-independent 3-D
  placement.
- **`LayerBomItem`** — per-layer material takeoff row (area goods / volumetric goods by
  material + function + thickness). **`BomItem`** — member cut-list row.
- **`RoomSchedule`** — derived room takeoff (area, perimeter, enclosed?).
- **`PlanDiagnostic`** / `DiagnosticSeverity`, **`SolverError`**.

### Entry points

- **`generate_project_plan(&BuildingModel) -> Result<ProjectFramePlan, SolverError>`** — the
  one entry the app calls. Validates, then per wall calls `generate_wall_plan`, adds join
  members (physical lapped corner end studs are reclassified as corner posts; partition +
  backing studs are added), generates floor/ceiling/roof plans, adds
  hip or valley rafters from shared roof-plane edges, frames jack rafters where hip/valley-bounded
  planes shorten, appends buildable-clearance gable studs/rake plates to matched end-wall plans,
  and builds
  the room schedule, per-layer BOM, and fastening BOM from construction systems and resolved
  standards. Roof layer takeoff uses the same overhung outline both meshers draw.
- `generate_wall_plan(wall, code, system, materials)` — single-wall framing.
- Exports: `export_bom_csv`, `export_layer_bom_csv`, `export_room_schedule_csv`,
  `export_wall_elevation_svg`, `export_project_svg` (both wall-elevation paths include derived
  gable height and sloped rake-plate polygons).

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
| `src/build.rs` | **Scene extraction from the model**: `scene_from_model`, `build_scene`, `RenderOptions`, `SceneFraming` (auto-derives cladding/drywall/glass/door/ground materials + sky + sun; wall solids use `BuildingModel::wall_envelope_span` plus derived gable profiles, and roof solids use the shared overhang outline). |
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
| `src/app_config.rs` | App-only runtime configuration (`AppConfig`): TOML file loading via `--config`, `FRAMER__...` environment overrides, CLI flag overrides, and typed startup settings such as `render.ray_query` / `render.smoke_frames`. Durable policy lives in [app-configuration.md](specs/app-configuration.md). |
| `mod.rs` | **`FramerApp`** struct + `impl eframe::App` + `ui_root` (panel layout) + project save/load/export + plan regeneration + selection/undo wiring + command-search execution dispatch. It owns the ordered stable component selection and session-only component visibility/isolation state, centralizes replace/toggle/clear selection paths, routes explicit visibility actions through session undo/redo snapshots, and prunes presentation keys after regeneration. Active drafting state (`active_level`, `ortho`, `snap_step`, `cursor_model`, `layers`) is shared presentation state and reset/clamped with the current document; `RenderSettings` is shared session-only state for sun/exposure controls. `ViewportWorkspaceState` owns the split topology, per-pane runtimes, deferred bridges, and app-local preset catalog. `WorkflowTab`, `WorkspaceMode`, and the active `ViewportMode` mirror couple global command context to the active pane: Render activates an existing Render pane or converts the active leaf, authoring restores a surviving authoring pane, and soft defaults target only the active leaf. `eframe::App::save` persists theme and validated layout presets; periodic autosave is suppressed because deferred eframe 0.35 frames can otherwise overwrite root-window geometry. Region-gated placement tools — room / ceiling / **vault** (`add_vault` + `scissor_halves`) / floor — are mutually exclusive (`deactivate_placement_tools`), route through `ViewClick::Place*`, and resolve enclosed loops through the active level's wall graph; the roof tool (`add_roof` + `footprint_roof_specs`) auto-generates gable, shed, rectangular hip planes, and simple L-footprint valley planes. Standards authoring edit ops (project-local pack creation, starter-pack import, stack add/reorder/remove, and waive overlays) also live here so every mutation goes through undoable `edit()`. The derived compliance report is regenerated with each plan, lowered into plan diagnostics, and exported as a CSV sidecar. |
| `actions.rs` | UI-only command metadata (`ActionId`, `EnabledContext`, labels, icons, tooltips, command-surface homes, workflow-strip tab/panel/flyout placement) for the command-surface migration. It is metadata only; enabled/disabled state is evaluated by `FramerApp`, and model mutations still live on `FramerApp`. |
| `component_visibility.rs` | App-only stable `ComponentKey`, ordered `ComponentSelection`, per-component hidden overrides, frozen isolation targets/modes, and authored/generated/semantic-source appearance resolution. This is disposable presentation state; it never enters core, solver, or `.framer`. |
| `context_menu.rs` | App-only typed context-menu surface/target context, section/item model, explicit surface builders, and one egui renderer that reads existing `ActionId` state. Interactive 3-D owns the first builder; the future Model Browser menu remains independently composed, and a later contribution registry can replace builder internals without changing the model/renderer/dispatch contract. |
| `panels.rs` | Model tree, inspector, app header quick-access/actions menus, command-search modal, tabbed workflow command strip with insertion flyouts and Render settings panels, status bar — the egui panel bodies. Renderable authored/generated browser rows expose independent accessible visibility eyes and ordered multi-selection; the inspector renders a read-only summary for heterogeneous selections. The status Level control and model-browser level rows activate the drafting level used by new level-owned objects. Command placement rules live in [command-surfaces.md](specs/command-surfaces.md). The ceiling inspector edits per-ceiling slope (pitch + low edge), converting a room region to a polygon on enable. The document-level Site & standards inspector edits `SiteContext`, stack order/membership, starter-pack import, project-local pack creation, and per-rule waiver reasons; the Plan inspector groups compliance report entries by outcome and lets report rows focus their source element when the app has a selectable authored context. |
| `model_edit.rs` | Authored-model mutation primitives (wall/opening drag state, constrained edits, id generation, including `next_standards_pack_id`). |
| `draw_wall.rs` | Draw-wall tool: snapping engine (`resolve_snap`) + same-level auto-join derivation. |
| `history.rs` | Generic `History<Snapshot>` undo/redo stack; the app snapshot combines authored intent, complete component selection, and session-only component visibility/isolation (+ `history_integration_tests.rs`). |
| `project_io.rs` | File-write + export-path helpers (orchestration lives in `mod.rs`). |
| `render_job.rs` | Background-thread **CPU** render job (progressive accumulation, fallback path). |
| `labels.rs`, `theme.rs` | Human-readable type labels; legacy theme shim. |
| `ui_harness_tests.rs`, `ui_shots_tests.rs` | Headless `egui_kittest` behavior checks plus the off-screen visual-review deck, including component eyes, multi-selection, dim/hide isolation, and tiled/repeated viewport layouts. |

### `src/app/design/` — design system

`mod.rs` (theme install + tokens), `tokens.rs`, `palette.rs`, `icons.rs` (Lucide icon font),
`widgets.rs` (semantic custom controls, including workflow tabs, command-strip panels, and the
segmented authoring view control). New UI
styling goes here, not inline; command routing policy belongs in
[command-surfaces.md](specs/command-surfaces.md).

### `src/app/viewport/` — the viewports (layered modules)

`workspace` (`viewport/mod.rs`) renders the workspace/view bar, a recursive docked split tree,
per-pane headers/canvases, contextual tool options, the active pane's selection toolbar and
surface-scoped 3-D menu, and deferred native panes. Workflow tabs in `panels.rs` remain the global
Design/Render/Plan command context. The segmented authoring camera control and global view actions
target the active pane; each pane header can independently select Plan, Roof, Elevation, 3D, or
Render. The Layouts menu applies typed built-ins or validated app-local presets. Tool options and
status readouts follow the active pane, while document selection, visibility/isolation, drafting
level, layers, and render lighting are shared by every pane.

| File | Contains |
| --- | --- |
| `mod.rs` | Recursive split-tree measurement/rendering, pane headers and splitters, active-pane routing, Layouts/preset UI, pane-tagged event reduction, deferred-event draining, shared workspace chrome, and active-pane 3-D context-menu/toolbar invocation. |
| `layout.rs` | Monotonic session `PaneId`, bounded `LayoutNode` split tree, active-leaf lifecycle, typed built-in layouts, sanitized 3D pose snapshots, and versioned/adversarially validated user-preset RON DTOs under `framer.viewport-layout-presets.v1`. Runtime IDs are never restored from storage. |
| `workspace_state.rs` | `ViewportWorkspaceState`: cohesive layout/preset registry, `PaneId` → runtime/deferred-handle maps, split/duplicate/apply reconciliation, camera-pose capture, retired-target cleanup queue, and eframe-storage dirty state. |
| `pane.rs` | `ViewportPaneRuntime`: independent plan camera, per-wall elevation cameras, shared-within-pane 3D/Render camera, CPU/GPU Render state, cursor, and snap cache. |
| `pane_view.rs` | Explicit immutable `PaneFrame`/owned deferred snapshot input, `PaneInteractionPolicy`, target-tagged `PaneCanvasEvents`, and view-mode dispatch over one mutable pane runtime. Deferred snapshots omit modal authoring state. |
| `deferred.rs` | Stable `show_viewport_deferred` IDs, owned snapshot/runtime bridge, child header/mode/actions UI, native-close-to-dock handling, and typed events returned to root ownership. |
| `plan.rs` | Top-down plan view: grid/rulers, walls, openings, placed furnishing/MEP footprints, selection context-toolbar anchors, draw-wall + room tools, endpoint drag, same-level room fills, the overhang-aware roof authoring overlay, and the wall display mode (outline/width/full) + layer-visibility guards. |
| `elevation_design.rs` | Single-wall elevation editor (openings + dimensions). |
| `elevation_framing.rs` | Plan-mode elevation overlay drawing generated members. |
| `elevation_openings.rs`, `elevation_dimensions.rs` | Opening edit handles; dimension drawing/anchors. |
| `scene_build/mod.rs` | **`Scene3d::from_project_with_geometry`** runtime facade and `SceneBuilder` mesh sink. It consumes the app's cached physical scene plus stable selected-component and visibility state, owns the full emission recipe and alpha-classified opaque/transparent index partition, and delegates element-specific lowering to child modules. Hidden components omit geometry and picks; dimmed components retain picks in the transparent pass. The test-only `from_project` convenience builds its own scene. |
| `scene_build/walls.rs`, `scene_build/members.rs`, `scene_build/surfaces.rs` | Interactive 3-D lowering by reason to change: derived wall envelopes/layers/openings; geometry-owned generated-member surfaces; authored roof/ceiling/floor surfaces. Emitters apply host, exact-leaf, and semantic `FrameMember.source` visibility, including dimmed outline opacity. New element families belong in the matching emitter rather than the facade. |
| `scene_build/picking.rs`, `scene_build/style.rs`, `scene_build/tests.rs` | Pick shapes/depth; the emitters preserve wall/surface priority 1, opening 2, and member 3. Viewport color/material policy is shared with the view cube/elevation; focused scene fixtures and regressions live with the package. Every generated member uses the same `framer-geometry` indexed surface for rendering and picking. |
| `axonometric.rs`, `camera_2d.rs`, `camera_3d.rs`, `view_cube.rs`, `view_common.rs`, `geom.rs` | Ortho 3D view; overlap witness overlay and pair framing; 2D/3D cameras; pane-qualified ViewCube; exact-tile sizing; shared transforms/hit-tests. |
| `gpu.rs` | `wgpu` pipeline wrapper for interactive 3D. Callback frames are keyed by `(pane target, model/ViewCube role)` and retired pane targets schedule resource cleanup. |
| `render.rs` | Explicit-input path-traced **Render** pane (orbit/dolly + progressive refinement) with pane-owned CPU fallback, GPU state, and motion cooldown. |

### `src/app/render/` — real-time GPU path tracer

`mod.rs` holds **`GpuRenderState`** (one pane's WGSL compute path-tracer state), a
target-keyed callback-resource store, and the shaders: `pathtrace.wgsl`, `blit.wgsl`,
`denoise.wgsl`, `rng.wgsl`. **These mirror
`framer-render`'s CPU math exactly** — the CPU path is the reference; `tests/gpu_parity.rs`
validates equivalence. The default GPU backend traverses the uploaded flat BVH in WGSL; an
experimental native `wgpu` ray-query backend can be enabled with app runtime config
(`render.ray_query`, `FRAMER__RENDER__RAY_QUERY=true`, or `--render-ray-query`) when the device
exposes `EXPERIMENTAL_RAY_QUERY`, building a BLAS/TLAS from the same triangle stream. Its
accumulation key covers geometry, camera, render size, lighting, sky, and exposure so shared Render
settings restart each visible pane's progressive refinement without rebuilding unchanged geometry.
`paint_for_target` qualifies callback resources by stable pane identity, and a cleanup callback
releases a retired target without disturbing sibling Render panes. Edit render math in both CPU and
WGSL paths together.

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
   framer_geometry::build_physical_scene   framer_render::scene_from_model + accumulate
               │                           │
   build cache + audit_physical_scene      (path-traced Render view; GPU mirror in app/render)
               │                           │
   scene_build::from_project_with_geometry + diagnostics/focus
   (shared member mesh + pick triangles)   │
               │                           │
   ┌───────────▼───────────────────────────▼──────────────┐
   │ PRESENTATION  pane tree/runtimes, drawings, exports    │  ← disposable project artifacts
   └────────────────────────────────────────────────────────┘
```

- **Authoring** edits `FramerApp.model` only; geometry edits call `reconcile_joins`; edits are
  snapshotted into `History` on interaction end.
- **Solving** runs `generate_project_plan`, builds and audits its `PhysicalScene`, and caches all
  three disposable results on the app. Active geometry focus is retained only while its structured
  violation remains current.
- **Presentation** never becomes the source of truth. All panes consume one coherent root-owned
  document/plan snapshot; deferred child events return to `FramerApp` before any authored mutation.
  Named layout presets persist only a validated app-local subset, never project state. A change to
  derived/presented state must flow back into authored intent (or a future explicit override
  record), per the [Mode Contract](architecture.md#mode-contract).

---

## Where do I add X?

| Goal | Touch | Then |
| --- | --- | --- |
| **New opening kind** | `OpeningKind` in `framer-core/src/model.rs` | framing rules in `framer-solver` `generate_wall_plan`; render handling in `framer-render/src/build.rs`; interactive 3-D mesh/picking in `viewport/scene_build/walls.rs`; inspector in `framer-app` `panels.rs`. |
| **New roof plane / ceiling / floor deck object** | the element collection (`roof_planes` / `ceilings` / `floor_decks`) + its struct in `framer-core/src/model.rs`, and a kind-matched `SystemKind` system | framing in `framer-solver/src/lib.rs` (`generate_roof_plan` / shared joisting generator); surface meshing in `framer-render/src/build.rs` + `viewport/scene_build/surfaces.rs`; generated-member presentation in `viewport/scene_build/members.rs`; tools/tree/inspector in `framer-app` `panels.rs` + `viewport/plan.rs`; bump `PROJECT_SCHEMA_VERSION`; update [ceilings-and-roofs.md](specs/ceilings-and-roofs.md) + [project-files.md](project-files.md). |
| **New construction layer function / material** | `LayerFunction` / `Material` / `Appearance` in `model.rs`; seed it in `starter_library()` | per-layer BOM in `framer-solver` (`layer_bom`); appearance/material lowering in `framer-render/src/build.rs`; asset bytes via `framer-library` package/store helpers. |
| **New library item kind** | typed collection + validation in `framer-core/src/model.rs` / `library.rs` | add closure/remap support in `framer-library`; add browser/import/placement UI in `framer-app/src/app/panels.rs`; add drawing/picking in the relevant viewport; update [libraries.md](specs/libraries.md) and [project-files.md](project-files.md). |
| **New solver rule / member kind** | `MemberKind` + rule in `framer-solver/src/lib.rs`; the authoring-side family tag is `MemberFamily` on `FramingSpec` in `framer-core/src/model.rs` | the surface/wall generator passes the matching `MemberKind` explicitly (the solver does not dispatch on `member_family`); attach `RuleProvenance`; add a focused solver test; expect a diagnostic for unsupported cases. |
| **New viewport mode** | `ViewportMode`, its `pane_view.rs::draw_pane_canvas` dispatch arm, pane/deferred mode selectors, and preset DTO name mapping in `layout.rs` | add a `viewport/<mode>.rs` renderer over explicit `PaneFrame` input + mutable `ViewportPaneRuntime`; decide which camera/runtime it shares within one pane. |
| **New viewport layout operation or built-in** | topology/invariants in `viewport/layout.rs`, runtime reconciliation in `workspace_state.rs`, and pane/header or Layouts UI in `viewport/mod.rs` | add bounded positive/negative tests, keep runtime IDs fresh, and update [viewport-layouts.md](specs/viewport-layouts.md). |
| **New view layer / wall display mode** | `ViewLayers` / `WallDisplay` in `app/mod.rs`; toggle in the Layers popover (`panels.rs`) | gate the render in `viewport/plan.rs` (and `viewport/scene_build/walls.rs` for 3D); session-only, not persisted. See [view-layers.md](specs/view-layers.md). |
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
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Notable test suites: schema round-trip + rejection (`framer-core/src/project.rs`), solver
determinism + BOM (`framer-solver/src/lib.rs`), shared render fixtures
(`framer-render/src/scenes.rs`, including the hip-roof scene), golden render
(`framer-render/tests/golden.rs`, regen with `UPDATE_GOLDEN=1`), GPU↔CPU parity
(`framer-app/tests/gpu_parity.rs`), interactive 3-D scene lowering and picking
(`framer-app/src/app/viewport/scene_build/tests.rs`), and headless UI
(`framer-app/src/app/ui_harness_tests.rs`). Tiled-workspace tests additionally cover bounded
split/preset decoding, per-pane runtime identity, deferred bridge ownership, renderer target-key
isolation/cleanup, and the off-screen multi-pane screenshot states.
