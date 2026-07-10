# Ceilings & Roofs

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** v1 Implemented (core types, floor/ceiling joisting, roof rafters,
> render + 3-D viewport, authoring UX, roofed example) · **v2 Phase A Implemented** —
> cathedral underside finish + the ridge-board-vs-beam tie fork + cathedral/attic diagnostic
> (A1); the live sloped-ceiling model `CeilingSlope` + validation, schema **v12** (A2); sloped
> ceiling joists with true cut lengths + scissor diagnostics (A3); the sloped-ceiling render in
> both meshers (A4); authoring — the inspector per-ceiling slope editor + the one-click vault
> tool + the vaulted `demo-shell` example (A5); and **v2 Phase B Implemented** —
> rectangular hip roof auto-generation, hip/valley/jack member kinds, hip rafters,
> a shortened hip ridge, jack rafters dying into hips, and equal-pitch L-footprint
> valley rafters with jack rafters dying into the valley; unequal-pitch valleys
> are diagnosed as unsupported; and B4 render/example polish adds hip-roof
> render coverage plus a hip-roofed `demo-shell` example · **v3 Implemented** —
> eave/rake overhangs drive one shared derived roof outline in plan, 3-D, Render,
> picking, and takeoff; generated roof members carry explicit plan endpoints and
> appear in Plan-mode 3-D; and simple matched gables derive their wall infill,
> studs, and rake plates from the authored walls plus roof planes ·
> **Linked milestone:** M3 (Floors And Roofs) ·
> **Goal:** G-014 (Ceilings & Roofs) ·
> **Plans:** [2026-06-20 — v1](../plans/2026-06-20-ceilings-and-roofs.md) ·
> [2026-06-23 — v2](../plans/2026-06-23-ceilings-and-roofs-v2.md) ·
> [2026-07-09 — v3](../plans/2026-07-09-roof-overhangs-framing-gable-walls.md) ·
> **Last reviewed:** 2026-07-09

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
  plane. A **rectangular hip roof** is four stored planes (two trapezoids plus two triangles)
  meeting at a shortened central ridge and four hip lines. The auto roof tool authors all three
  forms as editable `RoofPlane`s.
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
  footprint plus a global pitch and a transient form choice, then **writes the resulting
  planes into the model** as editable objects (hybrid: generate, then store). It generates
  gable, shed, rectangular hip, and simple equal-pitch L-footprint valley planes; after
  generation, the user edits the stored planes directly.
- A user authors a flat ceiling with a **ceiling tool** that, like the room tool, requires a
  same-level enclosed region (reuses `topology::room_boundary`) and attaches the ceiling to that room.
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

### v2 — Sloped ceilings & the ridge structural fork (Implemented, Phase A)

Deepens the v1 surfaces so a roof and its ceiling describe the *space between them* honestly.
Sequenced and tracked in
[2026-06-23 — v2](../plans/2026-06-23-ceilings-and-roofs-v2.md).

- **Cathedral regions render their interior finish.** A roof region with **no** `Ceiling` is a
  cathedral: the room sees the roof assembly's *conditioned-side* (interior) finish on the
  underside, not the weather face. v1 renders both faces with the roofing material; v2 resolves
  the underside through the roof system's interior layer.
- **`Ceiling.slope` becomes live.** A ceiling may carry a `Some(CeilingSlope { pitch, low_edge })`
  — a `Slope` plus the polygon edge it springs from (Slice A2; a sloped ceiling requires a
  `Polygon` region); the solver frames its joists on that plane (true sloped cut length, plan length for
  spacing/area — the rafter math), and both meshers lift the surface via the shared
  `RoofPlaneFrame` projection rather than a constant elevation. `slope == None` stays flat and
  byte-identical. A **scissor/vault** ceiling is two opposing sloped ceilings, mirroring how a
  gable roof is two opposing planes.
- **The ridge is framed from a real tie check, not an unconditional warning.** v1 always emits a
  `RidgeBoard` plus a fixed `roof.ridge.no-tie` warning. v2 detects whether a horizontal tie
  resists rafter thrust **at the bearing/plate line** (`level.elevation + level.height`): a
  **flat** ceiling enclosing the footprint at/near that elevation (later, explicit collar/rafter
  ties). A `FloorDeck` does **not** qualify by default — it resolves at `level.elevation` (the
  floor), not the plate — so it counts only if its elevation matches the bearing line; a dropped
  or sloped ceiling is not a full tie either. **Tied** ⇒ keep the ridge board, emit an `Info`
  note; **untied** (cathedral / scissor / no plate-line tie) ⇒ emit an `Unsupported` "structural
  ridge beam required" note.
  Geometry is unchanged (beam sizing is M4); the *judgment* becomes correct.
- **Validation extends to sloped ceilings** (fail closed): when `slope` is `Some`, `slope.run > 0`
  and the downslope reference is in range, mirroring `RoofPlane` checks. Flat ceilings keep
  today's rules.
- **A vault tool authors scissor/cathedral ceilings in one gesture.** Mirroring the room and roof
  tools, it is region-gated (an enclosed wall loop) and, given a ridge axis + pitch, generates the
  **two opposing sloped `Ceiling` planes** of a scissor/vault — written into the model as editable
  objects (hybrid generate-then-store, decision #4). The inspector also exposes per-ceiling slope
  (rise/run + downslope) for hand-editing a single plane. A cathedral is still authored by leaving
  the region ceiling-less.

### v2 — Hip & valley roofs (Implemented, Phase B)

The first non-opposing-plane roof geometry, built on Phase A. Tracked in the same v2 plan.

- **Hip roofs on a rectangular footprint.** The roof tool's transient form choice can emit four
  planes (two trapezoids + two triangles) meeting at a shortened central ridge with four hip
  lines, stored as editable `RoofPlane`s. No per-edge persisted roof-assembly state is added.
- **New member kinds:** `HipRafter`, `ValleyRafter`, `JackRafter`. The B1 multi-plane post-pass
  emits `HipRafter`s between adjacent planes with true sloped placement and shortens the ridge
  board to span hip-to-hip. B2 replaces overlong common rafters on hip-bounded planes with
  `JackRafter`s that shorten progressively and die into the hip line with true sloped cut
  lengths. B3 emits `ValleyRafter`s along equal-pitch L-footprint valley edges and uses the same
  clipped `JackRafter` path for rafters that die into the valley.
- **Valleys for equal-pitch L/T (multi-wing) footprints.** Where two right-angle wings of equal
  pitch meet, the valley bisects in plan; v2 frames the simple L-footprint case and diagnoses
  unequal-pitch valley edges as unsupported. Dormers and full straight-skeleton multi-wing
  auto-roofs are left to a later phase.
- **Multi-plane roofs render in both meshers.** Hip and valley planes are still ordinary stored
  `RoofPlane`s, so the path tracer and 3-D viewport lift each plane through the same
  `RoofPlaneFrame` path used by gables. The render suite carries a hip-roof golden scene, and
  `demo-shell.framer` is the checked-in hip-roof example.

### v3 — Overhangs, visible roof framing & gable walls (Implemented)

Closes the fidelity gaps exposed once finished roof assemblies became visible. Implemented by
[the 2026-07-09 plan](../plans/2026-07-09-roof-overhangs-framing-gable-walls.md) without changing
persisted intent or the `.framer` schema.

- **Overhangs are real derived geometry.** The stored `RoofPlane.outline` remains the bearing and
  topology footprint. A shared core derivation offsets the designated eave outward by
  `eave_overhang`, offsets only exposed rake edges by `rake_overhang`, and leaves shared
  ridge/hip/valley edges fixed. The roof-plan overlay, 3-D viewport, CPU/GPU Render extraction,
  scene bounds, picking, and roof layer takeoff all consume that one derived outline. Vertices
  outside the bearing footprint are still lifted through the original `RoofPlaneFrame`, so an
  eave tail drops by the authored pitch. Exact-edge-connected planes form one roof assembly:
  their eave/rake values must match (the inspector propagates edits across that component), so
  multi-plane seam endpoints stay coincident. Roof validation also rejects negative overhangs and
  redundant duplicate/collinear outline vertices rather than silently dropping an offset.
- **Derived roof framing is spatially complete and visible in Plan mode.** A sloped member carries
  exact integer-tick plan endpoints in addition to its endpoint elevations; common, jack, ridge,
  hip, valley, and blocking members therefore have one unambiguous 3-D placement. Plan-mode 3-D
  meshes every `RoofFramePlan` member and shows one translucent weather face per roof field for
  inspection, avoiding double-composited skin opacity. The
  Generated tree counts and selects wall and roof members. Design mode and the finished Render
  workspace continue to show authored assemblies rather than an exposed framing cutaway.
- **Simple gable walls are derived, never authored twice.** When exactly two same-level,
  non-eave roof edges cover an exterior wall from end to end, meet at one interior peak, agree on
  peak elevation, and spring from the authored wall top, core derives a triangular
  `GableWallProfile` from the unexpanded bearing outlines. Both meshers fill that profile with the
  authored wall system. The solver appends `GableStud` and `RakePlate` members plus the triangular
  layer takeoff to the existing `WallFramePlan`; studs stop beneath the rake-plate depth, and
  non-buildable end slivers/overlapping near-apex marks are omitted. Wall and project elevation
  SVGs include the full gable height and draw rake plates as slopes. Hip ends, sheds, interior
  walls, incomplete or mismatched roof edges, and overhang-only geometry do not synthesize a gable.

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
9. **(v2) A sloped ceiling is a single planar surface, like a roof plane.** It carries a `Slope`
   plus a downslope reference and reuses the `RoofPlaneFrame` lift; a scissor/vault is two
   opposing sloped ceilings, exactly as a gable is two opposing roof planes — generated as a pair
   by the vault tool (generate-then-store, decision #4) but stored as two independently editable
   `Ceiling`s. *Rejected: a ceiling with an internal ridge (a mini-roof object) — it forks the
   surface model and the joist generator for no expressive gain over two planes.*
10. **(v2) The ridge member follows a tie check; v1's blanket warning is replaced.** A flat
    ceiling or floor deck enclosing the footprint counts as a thrust tie (ridge board adequate);
    its absence (cathedral/scissor) means a ridge **beam** is required, surfaced as an
    `Unsupported` diagnostic. Geometry stays a ridge board in v2 (beam sizing is M4). *Rejected:
    auto-switching the framed member to a beam — sizing it needs the span tables that are M4.*
11. **(v2) Hips/valleys stay the hybrid "generate planes, store planes" model (decision 4).** The
    hip/jack/valley *members* are derived by a multi-plane post-pass over stored `RoofPlane`s;
    no new authored roof-assembly type. v2 limits the auto-generator to rectangular hips and
    equal-pitch right-angle valleys and **diagnoses** the rest, rather than shipping a
    general straight-skeleton solver that fights the integer-tick/canonical-JSON invariants.

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
    slope: Option<CeilingSlope> }` — `system.kind == Ceiling`; `slope` is `None` (flat) in v1.
    v2 makes `slope` live as `CeilingSlope { pitch: Slope, low_edge: u32 }` — the surface
    springs from the polygon's `low_edge` at `height` and rises at `pitch`, reusing the
    `RoofPlaneFrame` lift; a sloped ceiling requires a `Polygon` region (validation enforces this).
  - `FloorDeck { id, name, level, system, region: SurfaceRegion, span: SpanDirection }` —
    `system.kind == Floor`.
  - `Slope { rise: Length, run: Length }`; `SurfaceRegion = Room(ElementId) | Polygon(Vec<Point2>)`;
    `SpanDirection = Shorter | Along | Across | Explicit(..)`.
  - `RoofOpening { id, kind: OpeningKind, center: Point2, width, height }` — 2-D plane-local,
    nested in `RoofPlane.openings` (containment, no back-reference), distinct from the 1-D wall
    `Opening`. `OpeningKind::{Skylight, Stair}` already exist.
- **`FramingSpec`** (`model.rs`, `{member, spacing, pattern, cavity_material}`) gains a
  `member_family: MemberFamily` (`Stud | Rafter | CeilingJoist | FloorJoist | Truss`),
  `#[serde(default)]` → `Stud`, tagging the framing method a system produces. v1's solver
  selects the generator and the `MemberKind` from the framed object
  (`RoofPlane`/`Ceiling`/`FloorDeck`), not from `member_family`; the tag sets up family-based
  dispatch for the later truss/engineered-member work. **Span direction lives on the
  plane/deck element, not on `FramingSpec`** — bearing is instance data (same reason wall
  geometry lives on `Wall`, not its system), keeping the assembly generic.
- **`LayerFunction`** gains roof/floor roles: `Roofing` (the weather face), `Underlayment`,
  `CeilingFinish`. `exposure()` is wall-centric ("Exterior iff WeatherBarrier|Cladding|Masonry|
  ContinuousInsulation") and is re-scoped per `SystemKind` (a roof's weather face is `Roofing`).
  Roof/floor structural panels reuse the existing `Sheathing` function.
- **Validation** (`BuildingModel::validate`, `ConstructionSystem::validate`): new kind-matched
  system-reference checks; generalize the single-framing-layer rule (today gated `kind == Wall`)
  to `Roof`/`Floor`/`Ceiling`; range/geometry checks per the requirements.
- **Serialization** (`project.rs`): a schema bump (v1: **v10 → v11**; v2 Slice A2: **v11 → v12**
  for `CeilingSlope`; the loader is single-version,
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
- `MemberKind` gains `Rafter, CeilingJoist, FloorJoist, RidgeBoard, RimJoist, Blocking`,
  `HipRafter`, `ValleyRafter`, and `JackRafter`. B1 emits `HipRafter`; B2 emits hip-bounded
  `JackRafter`s; B3 emits `ValleyRafter`s for equal-pitch L-footprint shared valley edges, while
  `JackRafter` covers both hip-bounded and valley-bounded clipped rafters.
  The **exhaustive** matches
  (`MemberKind::label()`, `member_svg_color()`, and the app's `member_color()`) must be updated
  or the build breaks — the intended safety.
- **The sloped member.** `FrameMember` is 2-D-per-host (`x`, `elevation`, orientation
  `Horizontal|Vertical`). Its optional integer-tick **sloped placement** carries exact plan
  start/end points plus their building elevations, so a rafter or rake plate is "a member whose
  `z` varies linearly between two unambiguous world-plan points." Keep one `FrameMember` type
  (uniform BOM / provenance / diagnostics); do **not** fork a parallel `RoofMember`.
- **Bearing & span** reuse level-scoped `topology::room_boundaries` + `wall_interior_sides`
  to get the enclosed outline and bearing edges. Flat ceilings/floor decks are nearly fully automatic
  (default span = shorter direction; explicit override on the element).

### `framer-render` + app 3-D

- The path tracer's `Triangle` already computes `geom_normal = edge1.cross(edge2)` with no axis
  assumption and no backface cull, so **sloped surfaces need new triangle-emitting functions,
  not a new primitive**: a polygon emitter beside the wall-specific `push_box` in
  `build.rs::geometry_from_model` lifts each surface outline (roof planes via the shared
  `RoofPlane::frame()` — the one up-slope projection the solver also uses; ceilings/floors flat)
  and triangulates it with `framer_core::triangulate_simple_polygon`, an exact-integer ear-clip
  that is correct for the concave loops `room_boundary` produces (a naive fan is not). Route
  roof/ceiling layers through the existing `PaletteBuilder` so **no WGSL or GPU-parity change**
  is needed (opaque diffuse only in v1). New geometry must grow the same bounds `Aabb` feeding
  `SceneFraming`, and emit well-formed winding (degenerate tris are dropped at `PARALLEL_EPS`).
- The app's separate 3-D mesher (`viewport/scene_build.rs`, wall-vertical `WallCuboid`) gains a
  sloped roof solid + `PickSolid` + `member_color` entries so roofs/ceilings select like walls.
- Model-derived golden scenes lock the sloped path: a gable roof, a scissor vault, and a
  multi-plane hip roof in `golden.rs`; `gpu_parity.rs` already pins the shared extraction path
  via the roofed/scissor scenes.

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
- **`.framer` is single-version (currently v13; v2 Slice A2 introduced v12 and v1 used v11); no
  migration** — older files are rejected, not upgraded (current policy). New persisted structs use
  `#[serde(deny_unknown_fields)]` + serde defaults so empty projects/fixtures stay byte-stable
  (flat ceilings omit `slope`, so the v1 examples are byte-identical under v12).
- **CPU render is the reference; GPU mirrors it.** v1 adds only opaque-diffuse geometry through
  the shared `Triangle`/`Scene`/`to_gpu` path, preserving triangle/BVH order; `gpu_parity` stays
  green.

## Out of scope (YAGNI — architecturally open)

> Cathedral/scissor/sloped ceilings, rectangular hips, and equal-pitch L/T valleys move **into
> scope** with v2 (see the v2 requirements above). What remains out:

- **Unequal-pitch valleys, dormers, and full straight-skeleton multi-wing auto-roofs** — the
  geometry v2 deliberately diagnoses rather than frames (unequal-pitch valleys don't bisect at
  45°; a general multi-wing skeleton fights the integer-tick/canonical-JSON invariants). v2 ships
  rectangular hips and equal-pitch right-angle valleys; the rest is a later phase.
- **A structural ridge beam as a framed/sized member** — v2's tie check *diagnoses* when a
  beam is required but still frames a ridge board; sizing the beam needs M4 span tables.
- **Manufactured trusses** (profile + spacing + bearing, web design deferred to "the plant").
- **Engineered members** (I-joist / LVL / open-web): `BoardProfile` is capped at 2×12 with a
  hardcoded 1.5″ thickness and nominal depths — a richer `MemberProfile` comes with them.
- **Real IRC span/tie/connection lookups, header sizing, snow/wind tie forces** — M4 code
  profiles; v1 emits diagnostics only.
- **Varying plate heights / split levels under one roof; multi-level floor-of-N+1 = ceiling-of-N
  stacking** — v1 assumes one plate height per roof; the solver doesn't read `Level.elevation`
  for cross-level bearing yet.
- **Gable openings, lookout/outrigger design, dropped or unequal bearing, and trussed gable-end
  engineering.** v3 closes and stick-frames a simple matched gable, but does not size gable
  headers, design rake-overhang lookout load paths, or infer complex/mismatched end-wall profiles.
- **A roof framing-plan / building-section SVG export** — BOM/CSV fall out of the member list in
  v1; the projected drawing view is later.

## Open questions

- **`Sheathing` vs a new `Decking` function** for roof/floor structural panels — v1 reuses
  `Sheathing`; revisit only if render/BOM needs to distinguish a roof deck visually.
- **Span-direction default heuristic** (shorter clear span) vs. always-explicit authoring — v1
  defaults to shorter with an override on the element; confirm the heuristic on L/T regions.
- **Whether the derived `ProjectFramePlan` shape change** (new member kinds / sloped placement)
  warrants a versioned plan type, given the plan is round-tripped/compared in solver tests.
- **(v2) The downslope-reference field shape on `Ceiling`. — Resolved (Slice A2).** Adopted the
  `CeilingSlope { pitch: Slope, low_edge: u32 }` struct (`Ceiling.slope: Option<CeilingSlope>`),
  mirroring `RoofPlane`'s `(slope, eave_edge)`. The `Room`-region instability of a raw `low_edge`
  index is resolved by **requiring a `Polygon` region for any sloped ceiling** (validation enforces
  it) — which is what the vault tool emits anyway (two half-polygons, not a whole-room boundary).
  Drove the v11 → v12 schema bump.
- **(v2) Tie-detection containment test.** A1.1 must decide whether a ceiling/deck region
  "encloses" a roof plane's footprint to count as a thrust tie — exact polygon containment vs. a
  cheaper bbox/centroid test. Start with centroid-in-region (reuse `point_in_polygon`) and
  tighten if it misclassifies partial coverage.
