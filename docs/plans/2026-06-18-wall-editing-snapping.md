# Wall Editing & Snapping Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans / superpowers:test-driven-development to implement this plan task-by-task.

**Goal:** Make walls editable (drag corners/ends, move walls) and make snapping powerful and predictable (alignment guides, intersections, typed indicators, sticky hysteresis, suspend-snap, zoom-stable tolerance).

**Architecture:** Extract one pure snap engine (`framer-app/src/app/draw_wall.rs`) used by both the draw tool and a new node-based wall-drag editor (mirroring the opening-drag state machine). Join correctness is maintained by a geometry-derived `reconcile_joins` in `framer-core`. Rooms re-derive at rebuild.

**Tech Stack:** Rust, egui (immediate-mode UI), fixed-point `Length`/`Point2` (i64 ticks @ 1/128").

Design: `docs/plans/2026-06-18-wall-editing-snapping-design.md`.

---

## Slice 1 — Shared snap engine + snap-feel fixes (drawing only)

Refactor `resolve_draw_point` onto a richer `resolve_snap` returning a typed
`SnapResult`. Keep the old function as a thin wrapper so the existing test suite
stays green. Add `Midpoint` snapping, sticky hysteresis, suspend, and (in the
viewport) zoom-stable tolerance + typed indicators.

### Task 1.1: Add SnapKind / SnapResult / SnapContext types
- Modify: `crates/framer-app/src/app/draw_wall.rs`
- Add `SnapKind { Endpoint, Midpoint, Intersection, OnWall, Alignment, Grid, Free }`,
  `GuideAxis { Vertical, Horizontal }`, `Guide { axis, at: Length, source: Point2 }`,
  `SnapResult { point, kind, guides: Vec<Guide> }` with `forms_join()` helper,
  and `SnapContext<'a>` (model, raw, anchor, exclude, tolerance, release_tolerance,
  grid_step, suspend, previous).
- Commit: `feat(snap): typed snap result + context scaffolding`

### Task 1.2: resolve_snap covering existing behavior (endpoint → on-wall → ortho/grid)
- Test (in `draw_wall.rs` tests): `resolve_snap` returns `Endpoint` when near an
  endpoint, `OnWall` when projecting to interior, `Free` when nothing is near
  (ortho-locked + grid). Self-exclusion via `exclude`. `suspend` ⇒ `Free`.
- Implement `resolve_snap(ctx)`; reimplement `resolve_draw_point` as a wrapper that
  builds a context and maps `SnapResult` → `ResolvedPoint { point, on_existing =
  result.forms_join() }`. Keep `snap_to_endpoint`/`snap_to_wall_line` as helpers.
- Run: `cargo test -p framer-app draw_wall` → all green (old + new).
- Commit: `feat(snap): unified resolve_snap, resolve_draw_point delegates`

### Task 1.3: Midpoint snapping
- Test: cursor near a wall's midpoint returns `kind == Midpoint` at the midpoint;
  endpoints still win over midpoint when both are in range.
- Implement: add a midpoint candidate between endpoint and on-wall priority.
- Commit: `feat(snap): snap to wall midpoints`

### Task 1.4: Sticky hysteresis (acquire vs release radius)
- Test: given `previous` = a snap at point P, a raw point that is outside
  `tolerance` of P's source but inside `release_tolerance` keeps P; beyond
  `release_tolerance` it releases to the next-best candidate.
- Implement: at the top of `resolve_snap`, if `previous` is still valid within
  `release_tolerance`, re-emit it; else recompute with `tolerance`.
- Commit: `feat(snap): sticky snapping via release radius`

### Task 1.5: Viewport — zoom-stable tolerance, suspend key, typed indicators
- Modify: `crates/framer-app/src/app/viewport.rs` `draw_wall_overlay` (~1544) and
  `DrawWallPlanInput` (~29). Thread `previous: Option<SnapResult>` through tool state.
- Drop the `.min(24.0)` cap (line 1568); set `tolerance = 12px/scale`,
  `release_tolerance = 20px/scale`. Read `Alt` via `ui.input(|i| i.modifiers.alt)`
  → `suspend`. Replace the single marker+ring with a `match resolved.kind` drawing
  a distinct glyph + label per `SnapKind`.
- Store the last `SnapResult` in `DrawWallToolState` (mod.rs ~213) so hysteresis
  persists across frames; pass it in as `previous`.
- Verify: `cargo build -p framer-app`; GUI (install-app + computer-use): draw,
  observe stable snapping, glyphs, and Alt suspending snap.
- Commit: `feat(snap): zoom-stable tolerance, Alt-suspend, typed indicators`

---

## Slice 2 — Alignment guides + intersections

### Task 2.1: Alignment-source collection + guide candidates
- Test: with one wall, a raw point sharing the Y of an endpoint (but far in X)
  returns `kind == Alignment`, point snapped onto that Y, and one horizontal
  `Guide` whose `source` is the endpoint.
- Implement: collect alignment sources (endpoints + midpoints, minus `exclude`);
  add `Alignment` candidate (snap the off-axis coordinate to a source's X or Y)
  below `OnWall`, above `Grid`. Emit the `Guide`.
- Commit: `feat(snap): alignment extension guides`

### Task 2.2: Intersection snapping (guide×guide, guide×wall)
- Test: a raw point near where a vertical guide (off corner A's X) crosses a
  horizontal guide (off corner B's Y) returns `kind == Intersection` at that exact
  crossing, with both guides present. Guide×wall-line crossing likewise.
- Implement: when two guides (or a guide and a wall line) both fall within
  tolerance, compute their crossing and prefer it (priority just below Midpoint).
- Commit: `feat(snap): intersection snapping`

### Task 2.3: Render guides + cap at 2–3
- Modify: `draw_wall_overlay` — draw `resolved.guides` as thin dashed accent lines
  with the source ticked; cap simultaneous guides at 3 (nearest sources).
- Verify: GUI — drawing shows guides lighting up and intersection snaps.
- Commit: `feat(snap): render alignment guides`

---

## Slice 3 — Node-based wall editing (endpoint move)

### Task 3.1: reconcile_joins in framer-core (pure)
- Create: `crates/framer-core/src/model.rs` `reconcile_joins(model: &mut BuildingModel)`
  (or a new `joins.rs` re-exported from `lib.rs`).
- Test (core): coincident endpoints → `Corner`; endpoint-on-interior → `Tee`
  (first = through, second = partition); interior crossing → `Cross`; a join that
  no longer connects is dropped; an unchanged join keeps its id; result passes
  `validate()`. Match existing joins by (sorted wall-pair, kind) to preserve ids.
- Commit: `feat(core): geometry-derived reconcile_joins`

### Task 3.2: move_wall_endpoint helper in framer-core (pure)
- Test: moving an endpoint updates `start`/`end`, re-syncs `length` via
  `placement_length`, moves all coincident endpoints (node follow), keeps
  axis-aligned, `validate()` passes; calling `reconcile_joins` after a corner break
  removes the stale join.
- Implement `move_wall_endpoint(model, wall_id, which_end, new_point)` that updates
  the grabbed node and every wall endpoint equal to the old node point.
- Commit: `feat(core): move_wall_endpoint with node-follow`

### Task 3.3: WallDragState / event + begin/update/finish (app)
- Modify: `crates/framer-app/src/app/model_edit.rs` (add `WallDragState`,
  `WhichEnd`, `WallEditHandle`), `mod.rs` (`wall_drag: Option<WallDragState>`,
  `begin/update/finish_wall_drag`), `viewport.rs` (`WallDragEvent`,
  `handle_wall_drag_event`). Mirror the opening-drag trio exactly:
  `begin` → `history.begin(snapshot, "Move wall")`; `update` → `move_wall_endpoint`
  (snapped via `resolve_snap`) + `rebuild`; `finish` → `reconcile_joins` +
  `settle_history(false)`.
- Test (app, mirror `history_integration_tests.rs`): one undo step restores the
  pre-drag geometry and joins.
- Commit: `feat(edit): wall endpoint drag state machine + coalesced undo`

### Task 3.4: Plan-view handle hit-testing + cursors + drag emission
- Modify: `viewport.rs` — add `hit_wall_handle()` (endpoints ~10px priority, then
  body/midpoint), make `draw_selected_wall_handles` hover-aware, set
  `Resize*`/`Grab` cursors, gate selection on `clicked()` and drag on
  `drag_started_by(Primary)` over a handle, suppress primary-pan during a handle drag.
- Verify: GUI — select a wall, drag a corner, watch connected walls follow; drag a
  free end and drop onto another wall to form a Tee; undo in one step.
- Commit: `feat(edit): draggable wall handles in plan view`

---

## Slice 4 — Whole-wall move + polish

### Task 4.1: Body/midpoint drag → translate whole wall
- Both endpoints move as nodes; neighbours stretch. Ortho-constrained translation,
  snapped. Test (core): translate moves both ends + neighbours; `validate()` passes.
- Commit: `feat(edit): move whole wall (body/midpoint handle)`

### Task 4.2: Dimension reconciliation + clamp
- Verify driving-dimension interaction; clamp a move that would over-constrain a
  driving dimension; cursor cue. Test the clamp.
- Commit: `feat(edit): clamp endpoint moves against driving dimensions`

### Task 4.3: Run reconcile_joins inside add_wall/delete; refresh 2-bed example; docs
- Route draw + delete through `reconcile_joins` for one consistent join path.
- Update `docs/project-files.md` if needed; refresh `examples/projects/demo-two-bedroom.framer` if its joins change.
- Commit: `chore: unify join path, refresh example + docs`

---

## Done criteria
- `cargo test --workspace` green; `cargo clippy` clean.
- GUI verified: corner drag with node-follow, alignment guides + intersections,
  typed indicators, sticky feel, Alt-suspend, one-undo-per-drag.
- `requesting-code-review` before merge.
