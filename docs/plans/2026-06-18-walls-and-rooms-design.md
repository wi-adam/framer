# Walls & Rooms: Floor-Plan Authoring Design

Date: 2026-06-18
Status: Approved design, pre-implementation
Branch: `feat/walls-and-rooms`

## Summary

Framer can frame a connected multi-wall shell, but there is no way to *author*
walls (no draw/add/delete path — walls only exist at project init) and no concept
of a **room**. This feature adds a cohesive floor-plan authoring loop: draw
interior and exterior walls, and get named rooms from the loops those walls
enclose.

Walls are already a first-class data primitive (`Wall`, `WallJoin`, and
`WallExposure::{Exterior, Interior}`); the gap is authoring UX and solver
handling. Rooms do not exist anywhere — not in the code, and not yet in the
vision/scope (`docs/vision.md`), so this introduces a new authored primitive.

## Decisions (locked)

1. **Scope:** one cohesive feature — wall drawing *and* rooms together.
2. **Room model:** *seed-from-loop, persisted identity.* A room is an authored
   object that stores a stable id, name, usage, and a **seed point**. Its
   boundary, area, and perimeter are **derived** from the surrounding walls at
   rebuild and are **never serialized** — eliminating any second source of truth.
3. **Room shape:** boundary is a **general polygon** (ordered vertex loop), never
   assumed rectangular. Non-square rooms fall out for free once walls support
   angled/curved segments. Architecture stays shape-agnostic now; v1 traces
   rectilinear loops.
4. **Walls:** the draw tool is **ortho + smart snapping** (H/V lock, grid snap,
   snap to existing wall endpoints and along wall lines). Snapping
   **auto-creates the `WallJoin`** (`Corner` at an endpoint, `Tee` at mid-span).
   The axis-aligned invariant is kept for v1; free-angle walls are explicitly out
   of scope (but the data already stores arbitrary `Point2` endpoints).
5. **Interior walls / joins:** make **Tee *and* Cross** join framing real in this
   feature — not diagnosed-and-deferred.
6. **Room payload:** name + usage + derived area/perimeter. Height/volume deferred
   until Floors/Ceilings (M3) define a vertical extent.

## Out of scope for v1 (kept architecturally open)

- Angled / curved walls (room boundary type is kept general so this is a later
  unlock, not a rewrite).
- Room height / volume takeoff (waits for Floors/Ceilings, M3).

---

## Section 1 — Core data model & room derivation (`framer-core`)

We persist a room's *identity and seed point*, not its polygon.

```rust
// new, persisted (authored intent)
struct Room {
    id: ElementId,
    name: String,
    usage: RoomUsage,     // enum, default Unspecified
    level: ElementId,
    seed: Point2,         // a point inside the room; locates its bounding loop
    tags: Vec<String>,    // skip_serializing_if empty
}
// BuildingModel gains: rooms: Vec<Room>  (#[serde(default)], skip if empty)
```

Derived at rebuild (**never serialized**): boundary polygon, area, perimeter,
bounding-wall list.

- **Schema:** bump `PROJECT_SCHEMA_VERSION` 5 → 6 (`project.rs`). Migration is
  trivial (v5 → `rooms: []`). `#[serde(default)]` keeps old files loading; the
  version bump prevents older binaries from silently mis-reading new files.
- **IDs:** add `next_wall_id()` / `next_room_id()` with **global** scope. (The
  existing `next_opening_id()` is per-wall — the wrong scope here; `validate()`
  requires globally-unique ids.) Add `rooms` to `sort_deterministically()`.
- **Loop detection** lives in a new UI-agnostic `framer_core::topology` module
  used by *both* app and solver: build a planar arrangement from wall centerlines
  and join points, enumerate faces, and locate the face containing each room's
  `seed` via point-in-polygon. Written for general straight segments so angled
  walls later "just work."
- **Openness is a diagnostic, not a hard error.** A transiently-open loop while
  editing must not fail `validate()`; the room reports "boundary open" until it
  closes again, and its `seed` persists so it recovers.

---

## Section 2 — Solver & join semantics (`framer-solver` + a core correction)

**Core correction (join validation):** the current rule requires the join point
to be an endpoint of *both* walls (`model.rs` `validate()`), which is only true
for L-corners and end-to-end. A **Tee's** partition endpoint lands on the
through-wall's *mid-span*, so today's validation would *reject* a legal Tee.
Validation must branch by kind:

- `Corner` / `EndToEnd`: point is an endpoint of both (today's rule).
- `Tee`: point is an endpoint of exactly one (the **partition**) and lies *on*
  the other (the **through** wall). Needs a new axis-aligned
  `Wall::point_on_segment()` helper.
- `Cross`: point lies in the interior of both.

This also gives the solver a deterministic partition-vs-through rule: the through
wall owns the join mid-span; the partition owns it as an endpoint. For `Cross`,
pick the through wall by a stable rule (longer, then lowest id).

**Framing generation** — replace the `Corner | EndToEnd` gate (`framer-solver`
`lib.rs`) with a per-kind strategy:

- **Tee:** through-wall plates run continuous; partition plates stop at the
  through-wall *face* (reuse existing corner-post face-offset math). Generate a
  partition **end stud** plus an **intersection backing channel** in the through
  wall (studs for nailing + drywall backing). **No corner post.**
- **Cross:** one through wall continuous; crossing stubs butt from both sides into
  shared backing; de-clip the overlap so studs/plates are not double-counted.

**Room takeoff:** the solver pulls each room's derived boundary from `topology`,
computes **area + perimeter**, and emits a **room schedule** (name, usage, area,
perimeter) as a new derived output beside the BOM — rendered as plan labels in
the SVG and a CSV room-schedule export. Open-loop rooms surface a `Warning`
diagnostic carrying the room's `ElementId`. Rooms generate no members
themselves; they are space takeoffs that Floors/Ceilings (M3) will consume.

---

## Section 3 — Authoring UX (`framer-app`)

Tools mirror the proven **Dimension-tool** pattern (two-phase, live preview,
gesture-coalesced undo). Add `DrawWallTool` and `RoomTool` state, mutually
exclusive with the dimension tool; new `ViewClick::DrawWallPoint` /
`ViewClick::PlaceRoom` dispatched in `handle_view_click` (`mod.rs`). Both live in
the **plan view** (`draw_project_plan`, `viewport.rs`). Screen → world via the
existing `plan_inverse_point()` / camera `unapply()`.

- **Draw-wall:** click sets start; rubber-band preview (low-opacity, like the
  dimension preview) with a **live length readout**, ortho-lock to H/V, grid snap
  (reuse `OpeningDragConstraints.snap_step`), and snap to existing wall endpoints
  / along wall lines (snap target highlighted). Click commits the segment via
  `add_wall()` inside `edit("Draw wall", …)`; the endpoint chains into the next
  start (polyline mode) until Esc/right-click. On a snap, auto-create the right
  `WallJoin` (`Corner` at an endpoint, `Tee` at mid-span). **One undo step per
  committed segment.**
- **Room tool:** click inside an area → `topology` locates the enclosing face →
  `add_room()` with `seed` = click point, auto-name ("Room N"), default usage.
  Click outside any closed loop → inline "no enclosed area here." Rooms render as
  translucent fills with name + area labels.
- **Wall delete + cascade** (now mandatory once walls can be added): removes the
  wall's nested openings + dimensions, drops `WallJoin`s referencing it, and
  re-derives rooms — a room whose loop breaks goes **open (diagnostic), not
  deleted**; its `seed` persists so it recovers when reconnected. Del key +
  model-tree affordance, wrapped in `edit("Delete wall", …)`.
- **Tree & inspector:** Room nodes under each Level (sibling to walls); Room
  inspector = editable name/usage + read-only area/perimeter/bounding-walls.
  Toolbar BUILD group gains `+Wall` and Room-tool buttons. All mutations flow
  through `edit()`, so undo "just works."

---

## Section 4 — Testing & implementation slices

### Testing (deterministic, focused, per the DoD)

- **core:** Room round-trip + **v5 → v6 migration** (old file loads with
  `rooms: []`); join validation per kind (Tee/Cross accepted, malformed
  rejected); `point_on_segment` units; **topology** — closed rectangle → 1 face,
  two rooms sharing a wall → 2 faces, open chain → no face, seed-in-face
  correctness, sorted/deterministic output; global id uniqueness.
- **solver:** golden tests for a **Tee** (partition end stud + backing channel,
  no corner post, plate clipping) and a **Cross** (shared backing, overlap
  de-clip, no double-count); room area/perimeter for known shapes; open-room
  diagnostic; demo-shell BOM regression unchanged.
- **app:** undo = one step per drawn wall and per cascade delete, round-tripped
  (mirror `history_integration_tests.rs`); snap/auto-join geometry units
  (mid-span → `Tee`, endpoint → `Corner`); canonical-JSON-stable-after-rebuild
  guard.
- **GUI:** via the `install-app` skill + computer-use — draw walls, close a loop,
  place a room, see area, delete a wall and watch the room go *open*.

### Build order (thin vertical slices, each shippable)

1. **Wall authoring + delete** — ids, `add_wall` / cascade delete, draw-wall tool
   with ortho + grid + endpoint snap & auto-`Corner` join, toolbar, undo.
   *Corner joins only.* Immediately lets you hand-author shells.
2. **Schema v6 + Rooms + topology** — `Room`, migration, loop detection, room
   tool, room schedule (area/perimeter) + SVG labels / CSV, inspector & tree
   nodes. Rooms work for corner-formed loops.
3. **Tee + Cross framing** — per-kind join validation, mid-span snap → `Tee`,
   remove the `Unsupported` gate, Tee/Cross framing strategies + backing /
   clipping. **Interior walls now produce correct framing and carve rooms.**
4. **Polish** — wall-line snap + highlights, length readout, polyline chaining,
   room-schedule table, a 2-bedroom example project, docs (add rooms to vision
   scope; `project-files.md` v6).

## Risks / notes

- **Cross framing** is the trickiest member-generation work (intersection backing,
  member priority where two walls cross, overlap de-clipping).
- **Loop detection** must handle walls shared between adjacent rooms and walls
  that touch without a `WallJoin` record (snapping should make the latter rare).
- **Vision update required:** rooms are not currently in `docs/vision.md` scope;
  Slice 4 adds them intentionally (per the vision's "update the vision before
  implementing conflicting behavior" rule).
