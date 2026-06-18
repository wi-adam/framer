# 2D View Camera (Pan / Zoom) — Design

Status: accepted (2026-06-18)
Branch: `feat/2d-view-camera`

## Goal

Give the 2D views — the **Plan** ("shell") view and the **Elevation** ("wall")
views — pan and zoom. Today every 2D view is locked to a fit-to-bounds framing
computed fresh each frame: there is no way to zoom into a detail or scroll
around a large drawing. We want navigation that feels like a 2D design tool,
mirrors the conventions the Render view's camera already established, and
layers on top of the existing fit-to-bounds math without rewriting any of the
drawing or hit-testing code.

## Decisions (from product owner)

- **Input scheme — design-tool style.** Two-finger trackpad scroll pans;
  pinch / Cmd+scroll zooms anchored at the cursor; middle-drag or Space+left-drag
  also pans. Plain left-drag/click stays entirely for selection and
  opening-handle dragging.
- **Scope — all three 2D drawings:** the Plan/shell view, the Design-workspace
  wall elevation, and the Plan-workspace wall elevation.
- **Reset / camera memory — per-view memory.** The Plan view remembers its own
  camera; each wall's elevation remembers its own, keyed by wall id. Re-fit on
  demand via double-click on empty space or the `F` key.
- **One elevation camera per wall, shared across both elevation variants.** Both
  the Design-workspace and Plan-workspace elevations fit the *same* wall
  (length × height) to the canvas, so the base transform is identical and a
  remembered zoom/pan is meaningful in either. Halves the bookkeeping.

## Why fold into the existing transforms (not a new draw layer)

Grounded in the current codebase (`crates/framer-app/src/app/viewport.rs`):

- Every model→screen mapping in the 2D views funnels through just two places:
  - **Plan:** `plan_point` (`viewport.rs:2083`) and its inverse
    `plan_inverse_point` (`viewport.rs:2100`).
  - **Elevation:** `WallElevationLayout { wall_rect, scale }`
    (`viewport.rs:3218`), whose `new()` computes the fit. *Both* elevation draw
    functions (`draw_wall_design_elevation` `viewport.rs:2999`,
    `draw_wall_elevation` `viewport.rs:3800`) derive every line, rect, handle,
    and hit-test from those two fields.
- Therefore a camera folded into those two primitives propagates pan/zoom to all
  drawing **and** hit-testing for free — no call-site churn beyond threading a
  `&View2dState` parameter through.
- The base fit math is affine and stays byte-identical at the camera's identity
  state, so the change is provably a no-op when the camera is reset.

The camera is **pure presentation state**: it lives entirely in `framer-app`,
is never serialized to `.framer`, and is untouched by undo/redo (mirroring the
undo-redo design's "camera is not restored" rule,
`docs/plans/2026-06-17-undo-redo-design.md`).

## Architecture

```
crates/framer-core    (unchanged) — no camera concept here
crates/framer-app
  └── app/viewport.rs
        - View2dState (NEW): { zoom, pan } + pan_by / zoom_at / reset / transforms
        - plan_point / plan_inverse_point: take &View2dState, fold camera in
        - WallElevationLayout::new: takes &View2dState, bakes camera into wall_rect+scale
        - apply_view_2d_input (NEW): shared input helper for all three views
        - draw_project_plan / draw_wall_design_elevation / draw_wall_elevation:
          take &mut View2dState; allocate Sense::click_and_drag()
  └── app/mod.rs
        - FramerApp gains: plan_view: View2dState,
                           elevation_views: HashMap<String, View2dState>
        - load/new clears elevation_views; wall delete prunes its entry
```

### Data structure (`viewport.rs`)

```rust
/// 2D pan/zoom camera, layered on top of a view's fit-to-bounds base transform.
/// Pure presentation state — never serialized, untouched by undo/redo.
#[derive(Debug, Clone, Copy)]
pub(super) struct View2dState {
    /// Multiplicative scale on top of the fit-to-bounds base. 1.0 == fit.
    zoom: f32,
    /// Screen-space offset (egui points), anchored at the viewport center.
    pan: Vec2,
}

impl Default for View2dState {
    fn default() -> Self { Self { zoom: 1.0, pan: Vec2::ZERO } }
}
```

`Default` is the identity transform — pixel-identical to today's fit-to-bounds.

`FramerApp` gains two fields:

```rust
plan_view: View2dState,                         // the shell/plan camera
elevation_views: HashMap<String, View2dState>,  // keyed by wall id (ElementId.0)
```

The active elevation camera is fetched with
`elevation_views.entry(wall.id.0.clone()).or_default()` before the elevation
draw call, so a wall's camera materializes on first view and persists after.

### The transform

The base fit-to-bounds map is unchanged. The camera layers an affine transform
on top, anchored at the viewport center `c` (`drawing.center()`):

```
screen_final = c + (screen_base − c) · zoom + pan
screen_base  = c + (screen_final − c − pan) / zoom     // inverse
```

- `View2dState::apply(base: Pos2, drawing: Rect) -> Pos2` — forward.
- `View2dState::unapply(final_: Pos2, drawing: Rect) -> Pos2` — inverse.

`plan_point` computes the base as today, then returns `apply(base, drawing)`.
`plan_inverse_point` runs `unapply` first, then the existing inverse.
`WallElevationLayout::new` bakes the camera into its two fields:
`scale *= zoom`, and `wall_rect` is recentered through `apply` — so the existing
field-based drawing/hit-testing inherits pan/zoom unchanged.

At `zoom = 1, pan = 0` every formula reduces to the identity → the no-op
guarantee.

### Camera operations

```rust
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 40.0;
/// Pan offset clamp, in multiples of the viewport half-extent, so the drawing
/// can't be flung off-screen with no way back. F / double-click always recenters.
const PAN_MAX_VIEWPORTS: f32 = 2.0;

fn pan_by(&mut self, delta: Vec2, drawing: Rect) {
    self.pan += delta;
    self.clamp_pan(drawing);     // per-axis clamp to ±PAN_MAX_VIEWPORTS · half-extent
}

/// Zoom by `factor`, keeping the model point under `cursor` fixed.
fn zoom_at(&mut self, cursor: Pos2, drawing: Rect, factor: f32) {
    if !factor.is_finite() || factor <= 0.0 { return; }
    let old = self.zoom;
    let new = (old * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    let f = new / old;                      // *actual* applied factor after clamp
    let c = drawing.center().to_vec2();
    let q = cursor.to_vec2();
    // pan_new = (q − c)(1 − f) + pan_old · f   — keeps q's model point pinned.
    self.pan = (q - c) * (1.0 - f) + self.pan * f;
    self.zoom = new;
    self.clamp_pan(drawing);
}

fn reset(&mut self) { *self = Self::default(); }
```

Recomputing the applied factor *after* clamping is what keeps the cursor pinned
even at the zoom limits (the root-cause fix, not a symptom patch).

### Input handling

A shared helper, called by each view immediately after it allocates its
response (changed from `Sense::click()` to `Sense::click_and_drag()`):

```rust
/// Applies design-tool pan/zoom to `cam`. Returns `true` if this frame is a
/// pan gesture, so the caller suppresses selection / handle interaction.
fn apply_view_2d_input(ui: &Ui, response: &Response, drawing: Rect,
                       cam: &mut View2dState) -> bool
```

| Gesture | Action |
|---|---|
| Two-finger trackpad scroll | Pan (`pan_by(smooth_scroll_delta)`) |
| Pinch, or Cmd+scroll | Zoom at cursor (`zoom_at`) |
| Middle-drag, or Space+left-drag | Pan (`pan_by(drag_delta)`) |
| Double-click empty space, or `F` (hovered) | `reset()` |
| Plain left-drag / click | Untouched → selection & opening-handle drag |

Rules, mirroring the Render view (`viewport.rs:450–485`):

- **Zoom/scroll fire only when `response.hovered()`** — never hijacks global
  scroll.
- **Disambiguation:** a pinch (`zoom_delta() ≠ 1`) or `Cmd` means zoom;
  otherwise scroll pans. (The Render view maps plain scroll to *dolly*; 2D has
  no dolly, so plain scroll naturally becomes pan.)
- **Pan precedence:** `panning = middle_drag || (primary_drag && space_down)`.
  When `panning`, the view skips its click/handle logic for the frame. Plain
  left-drag without Space is unchanged, so selection and opening-handle dragging
  behave exactly as today.
- **Cursor feedback:** grab/grabbing while a pan gesture is active.
- All deltas are egui *points* (DPI-independent) — no `pixels_per_point` math.

## Lifecycle & edge cases

- **Opening-handle drag vs pan (main integration point):** the `panning` flag
  gates off `draw_wall_design_elevation`'s opening-handle drag for the frame, so
  Space-pan and handle-drag never fire together. Plain left-drag still drags
  handles.
- **Per-wall key hygiene:** `elevation_views` is cleared on `new_project` /
  `load_project_file` (stale wall-id keys); deleting a wall prunes its entry.
- **Pan clamp:** `clamp_pan` bounds the offset to ±`PAN_MAX_VIEWPORTS` ·
  half-extent per axis, so the drawing stays reachable; `F`/double-click always
  recenters regardless.
- **Zoom-limit anchoring:** `zoom_at` recomputes the applied factor after
  clamping, so the cursor stays pinned at min/max zoom.
- **Degenerate viewport / empty model:** guard `drawing` extent > 0; the
  identity transform applies for a zero-sized canvas; existing empty-model paths
  unchanged.
- **Undo/redo / save:** camera is presentation state — not snapshotted, not
  serialized, not restored (consistent with the undo-redo design).

## Testing

`View2dState` is pure and deterministic, so unit tests carry the correctness
load (matching the repo's parity-test culture). In a `#[cfg(test)]` module
beside the struct (no egui event loop required — `Rect`/`Pos2`/`Vec2` are plain
values):

- **Identity:** `Default` ⇒ `apply(p) == p` and `unapply(p) == p` for any `p`.
- **Round-trip:** `unapply(apply(p)) ≈ p` within ε for non-trivial zoom + pan.
- **Cursor-anchored zoom:** after `zoom_at(cursor, …, f)`, the model point that
  was under `cursor` maps back to `cursor` (within ε) — and still does when `f`
  is large enough to hit `ZOOM_MIN` / `ZOOM_MAX`.
- **Pan:** `pan_by(d)` shifts `apply` output by exactly `d` (below the clamp);
  pan clamp caps the offset at the configured bound.
- **Reset:** `reset()` returns to `Default`.

Manual GUI verification via the `install-app` skill (build → install →
computer-use screenshots) to confirm feel across all three views: scroll-pan,
pinch/Cmd-zoom anchoring, Space/middle pan, double-click/`F` re-fit, and that
selection + opening-handle dragging still work.

## Out of scope (YAGNI)

- Zoom-to-selection / zoom-to-fit-selection (only fit-to-all on reset).
- A visible zoom percentage readout or on-screen zoom buttons (double-click / `F`
  cover re-fit; the Render view has no such control either).
- Camera in the Axonometric / Render views (already have their own 3D camera).
- Persisting 2D camera state to disk or across sessions.
- Inertial / animated pan-zoom easing.
