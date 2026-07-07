<!--
  SPEC: Visual layering for the Plan and 3D views. Durable intent + locked decisions.
  Temporal task breakdown: docs/plans/2026-06-19-visual-layering.md.
-->

# View Layers (Wall Display Modes & Visibility Toggles)

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** G-003 (Viewport Interaction) ·
> **Plan:** [2026-06-19-visual-layering.md](../plans/2026-06-19-visual-layering.md) ·
> **Last reviewed:** 2026-07-07

## Intent / Purpose

The Plan ("Shell") and 3D views render walls as true-thickness colored
construction-layer bands. That detail is valuable when studying an assembly but
noisy when reading the building's shape — in a full shell the colors crowd out the
outline. This feature lets the user choose **how much wall detail to draw** and
**which annotation layers are visible**, so the default reading is a clean line
drawing and the colored cross-section is one click away. It serves the viewport
interaction goal in [vision.md](../vision.md): the 2D/3D views should be legible and
controllable presentation surfaces over the authored model.

## Requirements & behavior

- A **wall display mode** with three mutually exclusive states, shared by the Plan
  and 3D views (one setting, not per-view):
  - **Outline** — each wall is a single centerline (2D) / envelope edge outline
    (3D); no thickness fill, no color. **The default.**
  - **Width** — wall thickness without color: 2D draws two parallel **dashed** face
    lines at the full thickness; 3D draws one **monochrome** full-thickness volume.
    Openings still cut the wall.
  - **Full** — the true-thickness colored construction-layer bands (2D opaque; 3D
    translucent so framing members show through). Openings cut the bands.
- **Per-layer visibility toggles** (independent on/off) for Plan-view annotations:
  **Grid**, **Rooms** (fills + labels), **Corners** (wall-join markers + labels),
  **Wall labels** (names). Grid, Rooms, and Wall labels default on; Corner labels
  default off so a fresh shell stays a clean line drawing.
- Corner labels are contextual even when the Corner layer is off: hovering a wall
  join or selecting a corner reveals that one quiet marker/label. Turning the
  Corner layer on reveals all corner markers/labels.
- Switching wall display mode never changes selection, hit-testing, snapping, or
  wall-endpoint editing — those stay on the wall centerline (2D) / pick envelope
  (3D) in every mode.
- Hiding a layer also removes its click targets (you cannot select what you cannot
  see); walls and openings remain clickable in all modes. Corners are the
  exception: their point target remains hoverable/clickable while the Corner layer
  is hidden so the hover/selected label lifecycle has a discoverable target without
  making all corner labels permanent.
- Controls live in a single **Layers** popover; it stays open while the user flips
  multiple toggles in one visit.
- The path-traced **Render** view is unaffected.

## Decisions (locked)

- **Outline is the default.** The cleanest reading is the one a freshly loaded shell
  should present; Full (the prior always-on behavior) is opt-in. Rationale: the
  colored cross-section is reference detail, not the primary read.
- **Corner labels default off.** Corner markers are useful diagnostics for
  junctions but read like selection badges when every label is always visible.
  Rationale: keep the default plan quiet, while hover, selection, and the Corner
  layer still make the junction metadata available.
- **Wall display is a 3-state mode, not three independent toggles.** The three looks
  are alternatives for the same wall body, so a mode prevents nonsensical
  combinations.
- **One shared mode for Plan + 3D**, not a per-view setting. Rationale: a single
  mental model; the user picks "how walls look" once.
- **Session-only, never persisted.** Layer state is presentation, not authored
  intent — it is re-initialized to defaults each launch, like `grid`/`ortho`. It is
  never written to `.framer`. (Alternative — persisting view prefs — rejected for
  now to keep the model the only source of truth.)
- **3D Outline is a CPU painter overlay, not a GPU wireframe.** The axonometric
  pipeline is triangle-only; wall envelope edges are projected with the same
  `OrbitProjector` that feeds the GPU uniforms and drawn over the scene. Rationale:
  no new GPU pipeline, and the overlay matches the GPU image exactly. (Alternative —
  a `LineList` wgpu pipeline — rejected as heavier with no visible benefit at 1px.)
- **3D Width keeps openings; 3D Outline does not (yet).** Width reuses the
  opening-cut band builder; Outline draws the envelope box edges only.

## Architecture (grounded in the codebase)

- **State:** `WallDisplay` (enum) and `ViewLayers` (struct) in
  `framer-app/src/app/mod.rs`; `FramerApp.layers: ViewLayers` replaces the former
  `grid: bool`. Threaded into the view bundles in
  `framer-app/src/app/viewport/mod.rs` (`PlanView.layers`,
  `AxonometricView.wall_display`).
- **UI:** the Layers popover is a `menu_button` in the status bar
  (`framer-app/src/app/panels.rs`), reusing `widgets::toggle_switch` and
  `ui.selectable_value`.
- **Plan rendering** (`framer-app/src/app/viewport/plan.rs`): the wall body is a
  `match layers.wall_display` — `Outline` draws nothing extra, `Width` calls
  `draw_wall_width` (two dashed faces via `band_quad` + `draw_dashed_line`), `Full`
  calls the existing `draw_wall_layers`. The centerline, handles, and hit-test stay
  unconditional. `draw_opening_gap` runs only in `Full`. Rooms and wall labels are
  gated by the matching `layers.*` flags. Wall joins are user-facing Corners:
  `layers.joins` reveals all labels, while hover/selection reveals one quiet corner
  marker and label even when the layer is off.
- **3D rendering** (`framer-app/src/app/viewport/scene_build.rs`): `from_project`
  takes a `WallDisplay`; `push_wall_envelope` branches — `Full` keeps the per-layer
  bands, `Width` pushes one neutral full-thickness band, `Outline` pushes the
  envelope's 12 edges into `Scene3d.outline_edges` (and feeds the corners into
  `points` so the projector stays framed). The pick envelope is pushed in every
  mode. `axonometric.rs` draws `outline_edges` as a painter overlay and skips the
  wgpu callback when there is no fill geometry.

## Constraints & invariants

- `framer-core`/`framer-solver`/`framer-render` are untouched — this is pure
  app-side presentation over the authored model and the solver plan.
- No change to `.framer`, the schema, determinism, or the GPU↔CPU path-tracer
  parity (`tests/gpu_parity.rs` covers only the Render view).
- Presentation never becomes a source of truth ([architecture.md](../architecture.md)).

## Out of scope (YAGNI)

- Persisting view preferences across launches.
- Per-view (independent Plan vs 3D) wall display modes.
- Breaking the 2D dashed faces / 3D outline edges at openings.
- Toggling members, openings, dimensions, or the ground as separate layers.
- A general user-defined layer system (named layers, per-element assignment).
