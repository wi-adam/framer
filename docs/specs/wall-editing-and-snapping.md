# Wall Editing & Snapping

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** G-003 (Viewport Interaction) · **Plan:** [2026-06-18-wall-editing-snapping](../plans/2026-06-18-wall-editing-snapping.md)

## Summary

The walls-and-rooms feature can *create* walls but not *edit* them, and its
snapping is limited and fiddly. Three user-reported problems:

1. **No way to move/extend a wall.** Walls commit click-by-click with no drag
   editing. A misplaced wall can only be deleted and redrawn.
2. **No alignment snapping at common intersections.** Snapping only targets
   existing endpoints and wall lines within tolerance; there is no indication or
   snap when the cursor lines up with another wall's edge (e.g. "same height as
   the existing wall").
3. **Intersection snapping is fiddly.** A single undifferentiated marker, a
   zoom-dependent tolerance capped at 24", no hysteresis (flicker between
   candidates), and a silent ortho filter make snapping unpredictable.

This overhaul (a) extracts one shared **snap engine** used by both drawing and a
new editing path, (b) adds **CAD-style alignment guides + intersection snapping**,
and (c) adds **node-based wall editing** (drag a corner → connected walls follow).

## Decisions (locked)

1. **Editing model: move the joint.** Dragging a shared corner moves *every* wall
   meeting there (the building stays connected). A free end moves alone. Dragging
   the wall body / midpoint handle translates the whole wall, stretching neighbors
   joined at either end. Endpoints re-snap and re-join on drop.
2. **Alignment: extension guides + intersections.** Dashed inference lines when the
   cursor aligns with an existing endpoint/midpoint's X or Y; snap to the guide and
   to where two guides (or a guide and a wall line) cross.
3. **Snap-feel fixes (all four):** typed snap indicators, sticky snapping
   (hysteresis), an `Alt`/`Option` suspend-snap modifier, and a zoom-stable
   tolerance (drop the 24" cap).
4. **Axis-aligned invariant kept.** All walls remain H/V for v1 (consistent with
   the walls-and-rooms design); free-angle walls stay out of scope.
5. **Join reconciliation is same-level geometry-derived, not patched.** A single
   `reconcile_joins(model)` run inside every structural edit derives the full join
   set from same-level wall geometry and matches against existing joins to preserve
   ids. Coincident or crossing walls on different levels stay independent.

## Out of scope for v1 (kept architecturally open)

- Free-angle / curved walls.
- Full dimension-vs-drag conflict resolution UX (v1 clamps drags that would
  over-constrain a driving dimension).
- Multi-select / box-select editing of several walls at once.
- Tear-off-corner-by-modifier (drag a shared corner but detach just one wall).

---

## Section 1 — The shared snap engine (`framer-app`, `draw_wall.rs` → snap module)

Today snapping lives in `resolve_draw_point` (`draw_wall.rs`), used only by the
draw-wall tool. Extract a single resolver used by **both** drawing and the new
endpoint-drag editing, so the feel is identical everywhere and every fix lands in
one place.

```rust
struct SnapContext<'a> {
    model: &'a BuildingModel,
    raw: Point2,
    anchor: Option<Point2>,      // ortho reference: segment start, or wall's fixed end
    exclude: &'a [ElementId],    // wall(s)/endpoint being edited cannot snap to self
    scale: f32,                  // screen px per inch, for tolerance + guide selection
    grid_step: Option<Length>,
    suspend: bool,               // Alt held -> Free only
    previous: Option<SnapResult>,// for hysteresis (sticky)
}

struct SnapResult { point: Point2, kind: SnapKind, guides: Vec<Guide> }

enum SnapKind { Endpoint, Midpoint, Intersection, OnWall, Alignment, Grid, Free }

struct Guide { axis: GuideAxis, through: Point2, source: Point2 } // dashed inference line
```

`resolve_snap(ctx) -> SnapResult` candidate priority (high → low), all inside one
zoom-stable radius:

1. **Endpoint** of an existing wall (forms `Corner`).
2. **Midpoint** of an existing wall.
3. **Intersection** — two wall lines, two alignment guides, or a guide × wall line.
4. **OnWall** — projection onto a wall interior (forms `Tee`).
5. **Alignment** — shares X or Y with an alignment source; snap onto that axis and
   emit the extension guide.
6. **Grid** intersection.
7. **Free** — ortho-locked to `anchor` (or raw when no anchor), then grid-snapped.

`resolve_draw_point` becomes a thin wrapper over `resolve_snap`. The `on_existing`
flag generalises to "is this a join-forming snap" = `matches!(kind, Endpoint |
OnWall | Intersection)` (Intersection only forms a join when it lands on a wall).

### Snap-feel fixes

- **Typed indicators** — the plan overlay matches on `SnapKind` and draws a
  distinct glyph + small label (□ endpoint, △ midpoint, ✕ intersection, ⊢ on-wall,
  ┊ alignment, · grid), replacing the single anonymous marker + ring.
- **Sticky snapping** — `previous` + two radii: a candidate must be within
  `ACQUIRE_PX` (~10px) to acquire a snap, but a held snap only releases past
  `RELEASE_PX` (~18px). Implemented purely in `resolve_snap`.
- **Suspend-snap** — `suspend` (Alt) short-circuits to `Free`, with a status hint.
- **Zoom-stable tolerance** — radius is a constant *screen-pixel* distance at all
  zooms (`px / scale`); **drop the 24" cap**.

### Coordinate / unit invariants

All candidates snap to whole ticks (`Length`/`Point2` are i64 ticks at 1/128").
Endpoint/interior equality tests are exact, so snapped points must land on ticks.

---

## Section 2 — Alignment guides & intersections (`framer-app`)

**Alignment sources** = every existing wall endpoint and midpoint, plus the drag's
own fixed `anchor`. When the live point's X is within tolerance of a source's X,
emit a **vertical** extension guide through that source and snap X onto it (mirror
for Y → horizontal guide).

**Intersections** are the top-value snap: when two guides are live (e.g. a
horizontal guide off corner A and a vertical guide off corner B), their crossing
is `Intersection`. Guide × wall-line crossings likewise.

**Anti-clutter:** cap simultaneous guides at ~2–3, choosing nearest sources;
render thin dashed lines in a distinct accent colour with the source ticked,
fading at the ends so they read as inference, not geometry.

This is shared by the draw tool *and* endpoint-drag editing (both call
`resolve_snap`).

---

## Section 3 — Node-based wall editing (`framer-app`)

**One rule: editing moves *nodes*; everything coincident with a node moves with
it.** Two gestures, both mirroring the existing opening-drag state machine
(`OpeningDragState` / `OpeningDragEvent` / `begin|update|finish_opening_drag`).

- **Drag an endpoint handle** — moves that wall's end. If the point is a *shared
  node* (other walls have an endpoint there), all coincident ends move together
  ("move the joint"); a free end moves alone. On drop, `resolve_snap` runs;
  landing on another wall's endpoint/mid-span forms `Corner`/`Tee`.
- **Drag the wall body / midpoint handle** — translates the whole wall; both
  endpoints are nodes, so neighbours joined at either end stretch to follow.

### Wiring

- The three handles `draw_selected_wall_handles` already draws become live. Add
  `hit_wall_handle()` (mirror `hit_opening_edit_target`): endpoints (~10px) take
  priority, then body/midpoint. Hover grows the handle; cursor is `ResizeHorizontal`
  / `ResizeVertical` for endpoints, `Grab`/`Grabbing` for the body.
- New `WallDragState { kind: WallDragKind, primary_wall, affected: Vec<(usize,
  WhichEnd)>, start: Vec<Point2>, ... }` and `WallDragEvent { Started, Updated,
  Stopped }`, plus `begin/update/finish_wall_drag` in `mod.rs`.
- `begin_wall_drag` opens one coalesced undo step (`history.begin`) and computes
  the affected endpoint set (all walls whose start/end == grabbed node for an
  endpoint drag; both nodes for a body drag), snapshotting their geometry.
- `update_wall_drag(delta)` moves every affected endpoint (snapped via
  `resolve_snap`, ortho relative to each wall's fixed end), re-syncs each wall's
  `length` via `with_placement`, then `rebuild()`.
- `finish_wall_drag` runs `reconcile_joins`, then `settle_history(false)`.

### Click vs drag

A drag on a handle must **not** trigger the click-to-open-elevation behaviour.
Selection stays gated on `response.clicked()`; the drag begins on
`drag_started_by(Primary)` over a handle (egui separates them). A handle press
also suppresses the camera's primary-pan (`allow_primary_pan = false`).

---

## Section 4 — Correctness ripple: joins, rooms, dimensions (`framer-core` + app)

- **`reconcile_joins(model)`** (new, `framer-core`) — pure pass deriving the full
  join set from same-level wall geometry (coincident endpoints → `Corner`; endpoint-on-
  interior → `Tee`; interior crossing → `Cross`), matched against existing joins
  by (wall-pair, kind, point) to **preserve ids/names**, dropping stale and adding
  new. Run inside every structural `edit()` (draw, delete, move). Generalises the
  same-level auto-join rule in `joins_for_new_wall`; correct regardless of how
  geometry changed.
- **Rooms** already re-derive from `topology::bounded_faces` at `rebuild()`; open
  loops are diagnostics, not errors. A transiently-torn room mid-drag recovers via
  its persistent `seed`. No new work beyond letting rebuild run.
- **Dimensions** — `length` is kept synced via `with_placement`, so
  `WallStart`/`WallEnd`/length anchors stay valid. A move that would over-constrain
  a *driving* dimension is **clamped** (cannot violate) with a cursor cue; full
  conflict UX deferred. (Exact driving-dimension interaction confirmed in Slice 3.)

**Invariants every drag must hold:** axis-aligned; positive length;
`placement_length == length`; integer-tick coordinates; `validate()` as a final
guard.

---

## Section 5 — Build order & testing

### Slices (each shippable)

1. **Snap engine extraction** — unified `resolve_snap`, zoom-stable tolerance,
   typed indicators, sticky hysteresis, `Alt`-suspend; refactor the draw tool onto
   it. *Immediate feel win, no model change.*
2. **Alignment guides + intersections** — guide candidates + dashed rendering
   (problem 2).
3. **Wall handle drag** — endpoint move with node-follow, `reconcile_joins`,
   coalesced undo (problem 1 core).
4. **Whole-wall move** (body/midpoint) + dimension reconciliation polish +
   refreshed 2-bed example + docs.

### Testing (extends existing suites)

- **Pure engine** (`draw_wall.rs`/snap module): candidate priority; acquire-vs-
  release hysteresis; `suspend` → `Free`; self-exclusion; zoom-stable tolerance;
  alignment + intersection geometry.
- **Reconciliation** (`framer-core`): corner break removes join; drop-on-wall
  creates `Tee`; id preservation when a join persists; `Cross` detection;
  `validate()` passes.
- **Endpoint move** (app): length sync; shared-node follow; room tear → open
  diagnostic → recover; **one undo step per drag** (mirror
  `history_integration_tests.rs`).
- **GUI** (install-app + computer-use): drag a corner and watch connected walls
  follow; draw against alignment guides; `Alt` suspends snap.

## Risks / notes

- **Join reconciliation churn:** id-matching must be stable so reconciliation
  doesn't rename joins on unrelated edits. Match on (sorted wall-pair, kind) first,
  then point.
- **Driving-dimension interaction** with endpoint moves needs verification; clamp
  is the conservative v1 stance.
- **Sticky + alignment together:** the held snap must include held *guides* so an
  alignment lock doesn't flicker as the cursor drifts along the guide.
