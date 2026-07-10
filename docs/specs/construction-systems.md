# Construction Systems & Material Library

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** G-008 (Code Profile Data) /
> G-001 (Project Files) · **Last reviewed:** 2026-07-10

> Authored retroactively from the shipped v7 implementation (PR #16) to capture the durable
> intent of an already-built feature. It is the worked example for the spec template.

## Intent / Purpose

A wall is not a single material — it is an assembly: interior finish, framing (with cavity
insulation), sheathing, weather barrier, continuous insulation, cladding. Framer models that
as reusable **construction systems** composed of **material layers**, applied to walls **by
reference**. This gives true layered geometry, a per-layer material takeoff, and a path toward
R-value and code reasoning — while keeping the authored model small and agent-editable.

This realizes the [vision](../vision.md) commitment to "model construction intent, not meshes":
the layer stack is authored intent; the framing members and BOM are derived from it.

## Requirements & behavior

- A **material library** and a **construction-system library** live on the model and are reused
  across walls. New models seed a starter library (`BuildingModel::starter_library()`).
- A **wall references a system by id** (`Wall.system`); it does not embed its assembly.
  Changing a wall's system re-sizes its framing and re-derives its takeoff.
- A **construction system** is an ordered stack of layers from **interior → exterior**. Layer
  order is **semantic and never sorted**.
- Each **layer** has a `function` (interior finish, framing, sheathing, weather barrier,
  continuous insulation, air gap, cladding, masonry, structure, roofing, underlayment, ceiling
  finish, other), a `material` (by id), a `thickness`, and — **iff** `function == Framing` — a
  `FramingSpec` (member profile, spacing, pattern, member family, optional between-studs cavity
  material).
- **Every system has exactly one framing layer** (wall, floor, roof, ceiling). Validation
  rejects zero or multiple.
- **Materials are open/extensible.** Substance lives in a typed property map
  (`r_per_inch_milli`, cost, …), not in the enum, so external/shared libraries plug in via the
  same reference shape without schema churn. Appearance can be `SolidColor`, `Textured`
  (fallback color + texture `AssetRef` + scale), or `DepthMapped` (fallback color + height-map
  `AssetRef` + scale).
- **Derived, not stored:** total through-wall thickness, exposure (exterior vs interior), and
  clear-wall R-value are computed from the layer stack; cavity insulation adds no extra depth.
- **Solver output:** the framing plan uses the wall system's framing layer to size studs,
  plates, and corner posts. At a `Corner`, studs/bottom/lower plates follow the framing band's
  primary through/butt span and the upper member of a double top plate counter-laps the seam;
  the physical end studs are reclassified as corner posts rather than duplicated. The solver
  also produces a **per-layer material takeoff** (area goods + volumetric goods) aggregated
  across walls. See [wall-corner-laps.md](wall-corner-laps.md) and
  [code-map.md](../code-map.md#framer-solver--deterministic-framing--takeoffs).
- **Validation** fails closed: a wall referencing an unknown system, a layer referencing an
  unknown material, a framing/`function` mismatch, non-positive thickness/spacing, or the wrong
  framing-layer count are all `ModelError`s caught before save.

## Decisions (locked)

- **Apply systems by reference, not by value.** One edit to a system updates every wall using
  it; keeps `.framer` small and diffs meaningful.
- **Closed enums for things the app reasons about** (`SystemKind`, `LayerFunction`,
  `FramingPattern`, `BoardProfile`, `Sheathing`), **open data for material substance**
  (`Material.properties: BTreeMap<String, PropertyValue>`). Rationale: rendering/BOM/code logic
  must enumerate roles, but material science shouldn't require schema bumps.
- **Float-free model.** `PropertyValue` is `Int | Length | Text | Flag` and lengths are integer
  ticks, so the model stays `Eq` and serialization stays deterministic.
- **Layer order is interior → exterior and is never sorted.** Order carries meaning
  (assembly build-up); only id-keyed collections are sorted for canonicalization.
- **Exposure is derived, not authored.** A system is `Exterior` iff it has an outboard envelope
  layer (weather barrier, cladding, masonry, or continuous insulation).
- **R-value is a clear-wall approximation.** Exact integer milli-R summed over layers; the
  parallel-path framing-factor derate is deferred (and labeled as such).

## Architecture (grounded in the codebase)

All in [`framer-core/src/model.rs`](../../crates/framer-core/src/model.rs) unless noted:

- `BuildingModel { materials: Vec<Material>, systems: Vec<ConstructionSystem>, … }`.
- `ConstructionSystem { id, name, kind: SystemKind, layers: Vec<ConstructionLayer> }` with
  `framing_layer()`, `total_thickness()`, `exposure()`, `r_value_milli(materials)`.
- `ConstructionLayer { function: LayerFunction, material: ElementId, thickness, framing:
  Option<FramingSpec> }`; `FramingSpec { member: BoardProfile, spacing, pattern, member_family:
  MemberFamily, cavity_material: Option<ElementId> }`.
- `Material { id, name, source: MaterialSource, appearance: Appearance, tags, properties }`;
  `PropertyValue`, `Appearance::{SolidColor, Textured, DepthMapped}`.
- `Wall.system: ElementId`; `BuildingModel::system_for(wall)`, `material(&id)`.
- Derived corner geometry: `BuildingModel::wall_envelope_span`, `wall_framing_span`, and
  `wall_counter_lap_framing_span` keep finished assemblies, primary structural framing, and
  upper-plate counter-laps separate without persisted join-detail state.
- Serialization: schema **v13** in [`project.rs`](../../crates/framer-core/src/project.rs)
  (`systems`/`materials` are top-level authored keys; v13-only — older files are rejected). The
  shape is documented in [project-files.md](../project-files.md).
- Takeoff: `layer_bom()` / `LayerBomItem` and the layered rendering in
  [`framer-solver`](../../crates/framer-solver/src/lib.rs),
  [`framer-render/src/build.rs`](../../crates/framer-render/src/build.rs), and
  `framer-app` `viewport/scene_build.rs` (layered plan walls + section swatch).

## Constraints & invariants

- `framer-core`/`framer-solver` stay UI-free; the model stays deterministic and `Eq`.
- Authored layer stack is the source of truth; thickness/exposure/R-value/members/BOM are all
  regenerated, never stored.
- `.framer` is v13-only and canonical (see [project-files.md](../project-files.md#determinism)).

## Out of scope (YAGNI)

- Sloped-ceiling and engineered-member assemblies. `SystemKind::Floor`/`Roof`/`Ceiling` are
  now fully wired (authoring, solver, render); cathedral/scissor ceilings and I-joist/LVL/truss
  members are later phases of [ceilings-and-roofs.md](ceilings-and-roofs.md).
- `Staggered`/`Double` framing-pattern *generation* (authored but not yet generated).
- Framing-factor (parallel-path) R-value derate; richer `Appearance` beyond texture/depth-map
  assets (lapped siding, masonry coursing) — the enum is the seam for these.
- External/shared material library *resolution* (`MaterialSource::External` is representable;
  the resolver widens later).
