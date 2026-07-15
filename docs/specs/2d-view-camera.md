# 2D View Camera (Pan / Zoom)

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** G-003 (Viewport Interaction) ·
> **Plan:** [2026-07-14 tiled viewport workspaces](../plans/2026-07-14-tiled-viewport-workspaces.md) ·
> **Last reviewed:** 2026-07-14

## Goal

Give the 2D views — the **Plan** ("shell") view and the **Elevation** ("wall")
views — pan and zoom. Before this feature, every 2D view was locked to a
fit-to-bounds framing computed fresh each frame, with no way to zoom into a
detail or scroll around a large drawing. Navigation should feel like a 2D design tool,
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
- **Reset / camera memory — per-pane memory.** Each pane remembers one Plan/Roof
  camera; each wall's elevation remembers its own camera inside that pane, keyed
  by wall id. Re-fit on demand via double-click on empty space or the `F` key.
  Different panes never share mutable 2D camera state.
- **One elevation camera per wall per pane, shared across both elevation variants.** Both
  the Design-workspace and Plan-workspace elevations fit the *same* wall
  (length × height), so a remembered zoom/pan is meaningful in either and the
  bookkeeping halves. Note the two variants fit into slightly different content
  rects — the Design elevation reserves asymmetric, dimension-count-dependent
  margins for its annotation gutters, the Plan elevation uses a uniform margin —
  so the *base fit* is not pixel-identical between them. Consequently a shared
  non-default camera may frame the wall slightly differently in each variant, and
  within the Design elevation the framing shifts when the dimension count changes
  the margins (this is the pre-existing re-fit behavior, now also visible when
  zoomed). The identity state (zoom 1, pan 0) is unaffected — `apply` reduces to
  the original fit for any rect — so the no-op guarantee holds; only non-default
  cameras drift. `F` / double-click re-fit at any time.

## Why fold into the existing transforms (not a new draw layer)

Grounded in the current codebase (`crates/framer-app/src/app/viewport/`):

- Every model→screen mapping in the 2D views funnels through just two places:
  - **Plan:** `plan_point` and its inverse `plan_inverse_point` in `geom.rs`.
  - **Elevation:** `WallElevationLayout { wall_rect, scale }`
    in `view_common.rs`, whose `new()` computes the fit. Both elevation renderers
    (`elevation_design.rs` and `elevation_framing.rs`) derive every line, rect,
    handle, and hit-test from those two fields.
- Therefore a camera folded into those two primitives propagates pan/zoom to all
  drawing **and** hit-testing for free — no call-site churn beyond threading a
  `&View2dState` parameter through.
- The base fit math is affine and stays byte-identical at the camera's identity
  state, so the change is provably a no-op when the camera is reset.

The camera is **pure presentation state**: it lives entirely in `framer-app`,
is never serialized to `.framer`, and is untouched by undo/redo (mirroring the
undo-redo spec's "camera is not restored" rule,
[`undo-redo.md`](undo-redo.md)).

## Architecture

```
crates/framer-core    (unchanged) — no camera concept here
crates/framer-app
  └── app/viewport/camera_2d.rs
        - View2dState: { zoom, pan } + pan_by / zoom_at / reset / transforms
        - apply_view_2d_input: shared input helper for all three drawings
  └── app/viewport/geom.rs + view_common.rs
        - plan_point / plan_inverse_point fold in &View2dState
        - WallElevationLayout::new bakes the camera into wall_rect + scale
  └── app/viewport/pane.rs
        - ViewportPaneRuntime owns plan_view: View2dState
        - and elevation_views: HashMap<String, View2dState>
  └── app/viewport/workspace_state.rs
        - load/new resets every live pane's 2D cameras
        - rebuild prunes deleted wall ids from every pane runtime
```

### Data structure (`viewport/camera_2d.rs`)

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

Each `ViewportPaneRuntime` owns two fields:

```rust
plan_view: View2dState,                         // the Plan/Roof camera
elevation_views: HashMap<String, View2dState>,  // keyed by wall id (ElementId.0)
```

The pane's active elevation camera is fetched with
`elevation_views.entry(wall.id.0.clone()).or_default()` before the elevation
draw call, so a wall's camera materializes on first view and persists while that
pane lives. Applying a layout preset creates fresh pane runtimes; named presets
deliberately omit project-dependent 2D pan/zoom.

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
/// Pan clamp, as a fraction of the viewport half-extent *per unit zoom*. The
/// bound scales with `zoom.max(1.0)` so that cursor-anchored zoom into a corner
/// (which legitimately needs pan ∝ zoom) is never clipped, while pan is still
/// bounded at fit (zoom 1) so the drawing can't be flung off-screen with no way
/// back. F / double-click always recenters regardless.
const PAN_LIMIT_FACTOR: f32 = 1.0;

fn pan_by(&mut self, delta: Vec2, drawing: Rect) {
    self.pan += delta;
    self.clamp_pan(drawing);
}

/// Per-axis clamp: |pan| ≤ half-extent · PAN_LIMIT_FACTOR · zoom.max(1.0).
fn clamp_pan(&mut self, drawing: Rect) {
    let z = self.zoom.max(1.0);
    let max_x = drawing.width() * 0.5 * PAN_LIMIT_FACTOR * z;
    let max_y = drawing.height() * 0.5 * PAN_LIMIT_FACTOR * z;
    self.pan.x = self.pan.x.clamp(-max_x, max_x);
    self.pan.y = self.pan.y.clamp(-max_y, max_y);
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

Rules, mirroring the Render view (`viewport/render.rs`):

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
- **Per-wall key hygiene:** every live pane's `elevation_views` is cleared on
  `new_project` / `reset_demo` / `reset_wall_demo` / `load_project_file` through
  `ViewportWorkspaceState::reset_2d_cameras`, and `rebuild()` prunes any entry
  whose wall id is no longer in the model from every runtime. The maps therefore
  stay in sync however a wall is removed. Double-click re-fit fires only over
  empty canvas (each view passes `over_element`), so it never fights selection.
- **Pan clamp:** `clamp_pan` bounds the offset to ±half-extent ·
  `PAN_LIMIT_FACTOR` · `zoom.max(1.0)` per axis. The zoom-scaling is what keeps
  the clamp from fighting a cursor-anchored zoom into a corner (whose required
  pan grows with zoom); at fit (zoom 1) it still bounds pan so the drawing stays
  reachable. `F`/double-click always recenters regardless.
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
