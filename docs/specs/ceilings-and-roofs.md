# Ceilings & Roofs

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Proposed · **Linked milestone:** M3 (Floors And Roofs) ·
> **Proposed goal:** G-014 (Ceilings & Roofs) ·
> **Plan:** [2026-06-20-ceilings-and-roofs.md](../plans/2026-06-20-ceilings-and-roofs.md) ·
> **Last reviewed:** 2026-06-22

## Intent / Purpose

Framer can author a connected multi-wall shell with rooms, but a building has no lid: there
is no way to cap a structure with a **roof** or a **ceiling**, and therefore no roof/ceiling
framing, no attic or vaulted volume, and no takeoff for rafters, ceiling joists, sheathing,
or roofing. This feature adds the third major framing system of a small wood structure (after
walls and — alongside this work — floor decks), realizing the
[vision](../vision.md#north-star) step "place openings in walls, floors, ceilings, and roofs"
and the bulk of milestone **M3 (Floors And Roofs)**.

The governing product idea: **a roof and a ceiling are two distinct authored surfaces over one
footprint, and the relationship between them is the primitive that matters.** A flat ceiling
with an attic above, a cathedral ceiling that *is* the roof underside, and a scissor-vaulted
ceiling at a shallower pitch look identical from the street but are three different framing
systems with different load paths. The model represents both surfaces explicitly and never
infers the ceiling from the roof; "vaulted/cathedral" is simply *the room has no `Ceiling`
element, so the roof system carries the finish*.

This continues the [construction-systems](construction-systems.md) commitment — "model
construction intent, not meshes": the user authors roof planes, ceilings, and floor decks plus
a referenced layered assembly; rafters, joists, the BOM, and the rendered geometry are all
derived.

## Requirements & behavior

The observable contract. Testable statements; edge cases are explicit.

### Authored objects

- A project may carry **roof planes**, **ceilings**, and **floor decks**, each a level-owned
  authored object with a stable `ElementId`, referencing a shared
  [`ConstructionSystem`](construction-systems.md) by id.
- A **roof plane** is a single planar (sloped or flat) structural face: a plan-projected
  polygon outline, a **pitch** (rise:run), a designated **eave (downslope) edge**, a
  **reference elevation** (the bearing/springing line), and eave/rake overhangs. A roof plane
  references a system of `kind == Roof`.
- A **gable roof** is two opposing roof planes sharing a ridge; a **shed/mono roof** is one
  plane. v1 authors these two forms.
- A **ceiling** is a per-region (per-room or explicit polygon) finished surface at an authored
  height below the level top, **flat in v1** (slope reserved for later vault/scissor work). It
  references a system of `kind == Ceiling`. A region with **no** ceiling object is a
  cathedral condition (the roof underside is the finished surface).
- A **floor deck** is the horizontal structural deck of a level (its region + a span
  direction), referencing a system of `kind == Floor`. A flat ceiling and a floor deck share
  the same joisting generator: a flat ceiling is structurally a floor viewed from below.
- **Skylights / roof openings** are authored as openings hosted *on* a roof plane (2-D
  plane-local position), reusing `OpeningKind::Skylight`. (Authoring UI may land after the
  structural slice; the model supports the nested opening from v1.)

### Derivation (framing plan)

- The solver generates, per roof plane: **common rafters** arrayed perpendicular to the eave
  edge at the system's on-center spacing, a **ridge board** along the ridge (gable), and
  rafter **blocking** at the plate. Rafter **cut length uses true (sloped) length**; spacing
  and plane area use **plan** length.
- The solver generates, per flat ceiling and per floor deck: **joists** arrayed across the
  region at on-center spacing in the chosen span direction, **rim/band** members at the
  bearing ends, and **blocking**. Openings (stairs, skylights) are headed off with
  trimmer/header members.
- Roof/ceiling/floor members feed the **same BOM and per-layer takeoff** as walls (grouped by
  profile + kind + cut length, and by material + function + thickness).
- **Structural judgment is surfaced as diagnostics, not enforced** (per the vision's "code
  compliance is explicit, never implied"): e.g. a ridge with no rafter tie / ceiling joist
  reports "ridge board used without a tie — a structural ridge beam may be required"; spans
  are emitted with a "span not checked against a code table" note; varying plate heights under
  one roof are flagged as unsupported. v1 performs **no** IRC span/tie/connection lookups
  (deferred to M4 code profiles).

### Authoring

- A user authors a roof with a **roof tool** that auto-generates planes from the building
  footprint plus a global pitch and a per-edge gable/hip flag, then **writes the resulting
  planes into the model** as editable objects (hybrid: generate, then store). v1 generates
  gable/shed planes for a rectangular footprint; other forms are hand-authored or come later.
- A user authors a flat ceiling with a **ceiling tool** that, like the room tool, requires an
  enclosed region (reuses `topology::room_boundary`) and attaches the ceiling to that room.
- Roof planes, ceilings, and floor decks appear in the **model tree** under their level
  (siblings of rooms), are **selectable** in 2-D and 3-D, and expose editable parameters in
  the **inspector** (pitch, overhangs, height, span, system).
- Every mutation flows through the app's `edit()` / gesture-coalesced undo path, so undo/redo
  works without special handling.

### Validation (fail closed)

- Each object's `system` reference must resolve **and** match the required `SystemKind`
  (`Roof` / `Ceiling` / `Floor`) — mirroring the existing `WallSystemWrongKind` rule.
- `level` must reference an existing level; ids are globally unique; a roof plane's eave-edge
  index is in range; its outline has ≥3 points and is non-self-intersecting; pitch `run > 0`.
- A roof/ceiling/floor system must have **exactly one framing layer** (the same rule walls
  follow), so the framing band is unambiguous.
- A transient open region (mid-edit) is a **diagnostic, not a hard error** (like open rooms):
  a ceiling/deck whose enclosing loop is open reports "boundary open" and recovers when the
  loop closes.

## Decisions (locked)

1. **Ceiling↔roof relationship is the primitive.** Model roof and ceiling as two independent
   authored surfaces; derive the relationship (attic / cathedral / scissor) from whether a
   ceiling exists for a region and its pitch. Never auto-couple a ceiling to a roof. *Rejected:
   a single "roof+ceiling" object — it cannot express attic vs. cathedral vs. scissor, which
   are different framing systems.*
2. **v1 scope: gable + shed roofs, flat ceilings, floor decks — rectangular, stick-framed.**
   The thinnest slice that still crosses every crate. Cathedral/scissor, hips/valleys,
   multi-wing footprints, dormers, trusses, and engineered members are later phases. *Rejected:
   a hips-and-valleys or truss-first v1 — the geometry/engineering risk dwarfs the value of
   first proving the end-to-end loop.*
3. **Ceilings are a first-class authored primitive with a new `SystemKind::Ceiling`.** Lets
   BOM/render name the underside finish distinctly, lets validation differ, and lets a ceiling
   carry its own height/region independent of the roof. *Rejected: auto-deriving a ceiling per
   room (fragile under edits, cannot express vault/decoupled) and modeling the ceiling as mere
   inner finish layers (cannot express a dropped/independent ceiling).*
4. **Hybrid roof authoring: generate planes, store planes.** The persisted model always holds
   explicit `RoofPlane` objects; the auto-from-footprint generator is an **app tool** that
   emits planes, not a model concept re-evaluated on load. *Rejected: pure parametric
   (footprint + pitch recomputed each load) — straight-skeleton degeneracies fight the
   integer-tick / canonical-JSON determinism invariants; and manual-only — a poor default UX.*
5. **Pitch is an integer rise:run ratio (`Slope`), float-free.** Round-trips deterministically,
   feeds rafter cut math (true length computed transiently in f64 only inside the solver/SVG
   boundary, never stored), and renders directly. Flat = `rise: 0`. *Rejected: float degrees or
   a single rise-per-12 scalar — the former breaks `Eq`/determinism, the latter loses the
   explicit run.*
6. **Structural correctness in v1 is diagnostics, not enforcement.** Generate geometry + BOM;
   surface ridge-board-vs-beam, missing ties, unchecked spans, and varying plate heights as
   explicit diagnostics. Real IRC span/tie/connection rules belong to M4 code profiles.
7. **Floor decks and flat ceilings share one joisting generator.** A flat ceiling is a floor
   deck viewed from below; modeling both now keeps the generator and the model symmetric and
   sets up multi-level stacking (floor-of-N+1 = ceiling-of-N) later.
8. **Reuse the layered `ConstructionSystem` wholesale.** A roof/floor/ceiling assembly is the
   same interior→exterior layer stack, reinterpreted as **conditioned-side → weather-side**;
   no parallel assembly model.

## Architecture (grounded in the codebase)

Where the requirements land in real types and files. Most seams already exist; the two
genuinely new capabilities are **the first sloped/3-D authored geometry** and **the first
non-axis-aligned framing member**.

### `framer-core` (authored model)

- **Vertical extent.** [`Level`](../../crates/framer-core/src/model.rs) is `{id, name,
  elevation}` today — no top datum. Add `height: Length` (`#[serde(default)]`) so a level's top
  plane (`elevation + height`) is the bearing/springing line for roofs and the hang reference
  for ceilings, without guessing from wall heights.
- **`SystemKind`** (`model.rs`, currently `{Wall, Floor, Roof}` with `ALL: [Self; 3]`) gains
  `Ceiling`; update `ALL` and `label()`. `Floor`/`Roof` already exist but are unwired.
- **New primitives** on `BuildingModel`, each a level-owned, id-bearing, integer-tick collection
  (`Vec`, `#[serde(default, skip_serializing_if = "Vec::is_empty")]`):
  - `RoofPlane { id, name, level, system, outline: Vec<Point2>, slope: Slope, eave_edge: u32,
    reference_elevation: Length, eave_overhang: Length, rake_overhang: Length,
    openings: Vec<RoofOpening> }` — `system.kind == Roof`.
  - `Ceiling { id, name, level, system, region: SurfaceRegion, height: Length,
    slope: Option<Slope> }` — `system.kind == Ceiling`; `slope` is `None` (flat) in v1.
  - `FloorDeck { id, name, level, system, region: SurfaceRegion, span: SpanDirection }` —
    `system.kind == Floor`.
  - `Slope { rise: Length, run: Length }`; `SurfaceRegion = Room(ElementId) | Polygon(Vec<Point2>)`;
    `SpanDirection = Shorter | Along | Across | Explicit(..)`.
  - `RoofOpening { id, kind: OpeningKind, center: Point2, width, height }` — 2-D plane-local,
    nested in `RoofPlane.openings` (containment, no back-reference), distinct from the 1-D wall
    `Opening`. `OpeningKind::{Skylight, Stair}` already exist.
- **`FramingSpec`** (`model.rs`, `{member, spacing, pattern, cavity_material}`) gains a
  `member_family: MemberFamily` (`Stud | Rafter | CeilingJoist | FloorJoist | Truss`),
  `#[serde(default)]` → `Stud`, so the solver dispatches member geometry by family. **Span
  direction lives on the plane/deck element, not on `FramingSpec`** — bearing is instance data
  (same reason wall geometry lives on `Wall`, not its system), keeping the assembly generic.
- **`LayerFunction`** gains roof/floor roles: `Roofing` (the weather face), `Underlayment`,
  `CeilingFinish`. `exposure()` is wall-centric ("Exterior iff WeatherBarrier|Cladding|Masonry|
  ContinuousInsulation") and is re-scoped per `SystemKind` (a roof's weather face is `Roofing`).
  Roof/floor structural panels reuse the existing `Sheathing` function.
- **Validation** (`BuildingModel::validate`, `ConstructionSystem::validate`): new kind-matched
  system-reference checks; generalize the single-framing-layer rule (today gated `kind == Wall`)
  to `Roof`/`Floor`/`Ceiling`; range/geometry checks per the requirements.
- **Serialization** (`project.rs`): a schema bump (**v10 → v11**; the loader is single-version,
  `MIN_SUPPORTED == PROJECT_SCHEMA_VERSION`). New collections join `sort_deterministically()`
  (id-sorted; nested `RoofOpening`s sorted by id) and the round-trip fixtures. See
  [project-files.md](../project-files.md).

### `framer-solver` (derived framing)

- Add `generate_roof_plan`, `generate_ceiling_plan`, `generate_floor_plan` as **siblings of
  `generate_wall_plan`**, called from `generate_project_plan` after the wall loop. The solver
  is free functions — no trait dispatch to satisfy.
- `ProjectFramePlan` gains `roof_plans`, `ceiling_plans`, `floor_plans` (separate `Vec`s, not a
  unified surface type — least churn to the existing `bom()` / `layer_bom()` flatteners, which
  just traverse the new lists).
- `MemberKind` gains `Rafter, CeilingJoist, FloorJoist, RidgeBoard, RimJoist, Blocking`
  (+ `HipRafter, ValleyRafter, JackRafter` reserved for the hips phase). The **exhaustive**
  matches (`MemberKind::label()`, `member_svg_color()`, and the app's `member_color()`) must be
  updated or the build breaks — the intended safety.
- **The sloped member.** `FrameMember` is 2-D-per-host (`x`, `elevation`, orientation
  `Horizontal|Vertical`). Extend it with an optional integer-tick **sloped placement** (a
  start/end elevation pair plus an in-plane axis in a roof-plane-local basis) so a rafter is "a
  member whose `z` varies linearly across the plane." Keep one `FrameMember` type (uniform BOM /
  provenance / diagnostics); do **not** fork a parallel `RoofMember`.
- **Bearing & span** reuse `topology::room_boundaries` + `wall_interior_sides` to get the
  enclosed outline and bearing edges. Flat ceilings/floor decks are nearly fully automatic
  (default span = shorter direction; explicit override on the element).

### `framer-render` + app 3-D

- The path tracer's `Triangle` already computes `geom_normal = edge1.cross(edge2)` with no axis
  assumption and no backface cull, so **sloped surfaces need new triangle-emitting functions,
  not a new primitive**: add a `push_quad` / fan-triangulator beside the wall-specific
  `push_box` in `build.rs::geometry_from_model`, with a roof-plane-local basis. Route
  roof/ceiling layers through the existing `PaletteBuilder` so **no WGSL or GPU-parity change**
  is needed (opaque diffuse only in v1). New geometry must grow the same bounds `Aabb` feeding
  `SceneFraming`, and emit well-formed winding (degenerate tris are dropped at `PARALLEL_EPS`).
- The app's separate 3-D mesher (`viewport/scene_build.rs`, wall-vertical `WallCuboid`) gains a
  sloped roof solid + `PickSolid` + `member_color` entries so roofs/ceilings select like walls.
- Add a model-derived roofed golden scene (demo-shell + a gable roof) to lock the sloped path
  in `golden.rs` / `gpu_parity.rs`.

### `framer-app` (authoring)

- New `ViewportMode::RoofPlan` (top-down, reuses the 2-D plan machinery; roofs are
  footprint-driven) for authoring plane outlines/pitch, with the 3-D view for verification.
- New tools following the established two-phase / `edit()`-wrapped / mutually-exclusive pattern:
  a **roof tool** (auto-from-footprint → emits planes) and a **flat-ceiling tool** (region-gated
  like the room tool). New `Selection` / `ViewClick` variants; model-tree and inspector arms.
- Wire `SystemKind::{Floor, Roof, Ceiling}` through system authoring/picker (un-hardcode the
  `kind == Wall` filter); un-fork `add_opening` so `Skylight` is not coerced to a window.

### `framer-library`

- `Library.systems` already holds any `SystemKind`; `import_system` / `vendor_system` deep-copy,
  remap, and stamp provenance regardless of kind, and `system_content_hash` covers new
  `FramingSpec`/`LayerFunction` fields automatically (whole-struct JSON hash). Ship seed
  Roof/Floor/Ceiling systems in `libraries/framer-starter.framerlib`. **Caveat:** if a later
  roof/floor system adds a *new cross-reference* beyond `layer.material` / `cavity_material`,
  the id-remap reversal in `vendored_system_content_hash` must be extended or drift detection
  false-positives. v1 introduces no such reference.

## Constraints & invariants

- **UI-free `framer-core` / `framer-solver` / `framer-render`.** All new authoring types,
  framing rules, and geometry stay out of the app crate.
- **Float-free, `Eq`, deterministic model.** Every geometric quantity is integer `Length`/
  `Point2` ticks (16 = 1 inch). Pitch is a `Slope` ratio of ticks; true sloped lengths are
  derived transiently, never stored. Same model + code profile → byte-identical `.framer`.
- **Three layers, one source of truth.** Roof/ceiling/floor *authored intent* is persisted;
  members, areas, R-value, BOM, drawings, and render geometry are regenerated, never stored.
- **Layer order is semantic (conditioned-side → weather-side) and never sorted;** only id-keyed
  collections are canonicalized.
- **Closed enums for things the app reasons about** (`SystemKind`, `LayerFunction`,
  `MemberKind`, `MemberFamily`, `OpeningKind`); open data only for material substance.
- **`.framer` is single-version (v11 after this change); no migration** — older files are
  rejected, not upgraded (current policy). New persisted structs use
  `#[serde(deny_unknown_fields)]` + serde defaults so empty projects/fixtures stay byte-stable.
- **CPU render is the reference; GPU mirrors it.** v1 adds only opaque-diffuse geometry through
  the shared `Triangle`/`Scene`/`to_gpu` path, preserving triangle/BVH order; `gpu_parity` stays
  green.

## Out of scope (YAGNI — architecturally open)

- **Cathedral / scissor / sloped ceilings** (just `Ceiling.slope = Some(..)` + the
  ridge-beam-vs-board structural fork) — next phase.
- **Hips, valleys, multi-wing footprints, dormers** — the hard geometry (straight-skeleton
  auto-roof, unequal-pitch valleys that don't bisect at 45°, a multi-plane member post-pass
  analogous to `add_join_members`, a non-convex integer-tick triangulator).
- **Manufactured trusses** (profile + spacing + bearing, web design deferred to "the plant").
- **Engineered members** (I-joist / LVL / open-web): `BoardProfile` is capped at 2×12 with a
  hardcoded 1.5″ thickness and nominal depths — a richer `MemberProfile` comes with them.
- **Real IRC span/tie/connection lookups, header sizing, snow/wind tie forces** — M4 code
  profiles; v1 emits diagnostics only.
- **Varying plate heights / split levels under one roof; multi-level floor-of-N+1 = ceiling-of-N
  stacking** — v1 assumes one plate height per roof; the solver doesn't read `Level.elevation`
  for cross-level bearing yet.
- **Gable-end stud triangulation and rake cuts on the wall top** — v1 leaves walls rectangular
  and lets the roof/overhang sit above (flagged as a fidelity gap).
- **A roof framing-plan / building-section SVG export** — BOM/CSV fall out of the member list in
  v1; the projected drawing view is later.

## Open questions

- **`Sheathing` vs a new `Decking` function** for roof/floor structural panels — v1 reuses
  `Sheathing`; revisit only if render/BOM needs to distinguish a roof deck visually.
- **Span-direction default heuristic** (shorter clear span) vs. always-explicit authoring — v1
  defaults to shorter with an override on the element; confirm the heuristic on L/T regions.
- **Whether the derived `ProjectFramePlan` shape change** (new member kinds / sloped placement)
  warrants a versioned plan type, given the plan is round-tripped/compared in solver tests.
- **Vision/backlog:** add **G-014 (Ceilings & Roofs)** to [vision.md](../vision.md#goal-backlog)
  and confirm M3's roof/ceiling bullets against this scope before implementation (per the
  vision's "update the vision before implementing conflicting behavior" rule).
