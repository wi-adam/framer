# Viewport Module Refactor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split the 6042-line `crates/framer-app/src/app/viewport.rs` into a `viewport/` directory of ~14 focused modules, behavior-preserving, with two targeted redesigns (a `PlanView<'_>` argument bundle and relocating misplaced shared items).

**Architecture:** The file is already decoupled — renderers are free functions taking explicit params and returning event values (`ViewClick`, `WallDragEvent`, `DesignElevationResponse`); only a thin 5-method `impl FramerApp` layer touches app state. We exploit that by extracting leaves-first into a layered DAG. Genuinely-shared pure helpers move into a `geom.rs` leaf and egui view-frame helpers into a `view_common.rs` leaf; pulling those two out is what turns the apparent cross-cluster cycles into a clean DAG.

**Tech Stack:** Rust 2024 edition, `eframe`/`egui`, `egui_wgpu`/`wgpu`, `framer-core`/`-render`/`-solver`. Regression guard: the existing test suite (44 `#[test]` in viewport.rs + `history_integration_tests` which imports `WallDragEvent`).

---

## Decisions (locked)

1. **Ambition:** split + light redesign. We do NOT introduce a `ViewportState` god-struct (cameras are already passed as `&mut View2dState`/`&mut View3dState`; wrapping all 10 fields only adds `&mut self` borrow friction in `workspace()`/`draw_project_render` for zero behavioral gain). We do NOT add a formal input/draw pass split (the return-events pattern already is that boundary).
2. **Elevation:** split 3 ways — `elevation_design.rs` (orchestrator) + `elevation_dimensions.rs` + `elevation_openings.rs`, plus `elevation_framing.rs` for the generated-framing view.
3. **PlanView bundle:** YES — collapse `draw_project_plan`'s 13-arg signature to 7 via a by-value `PlanView<'_>` bundle, matching the existing `AxonometricView`/`DesignElevationView` idiom. Move `WallDragEvent` (currently misplaced at line 3854 in the elevation byte-range) into `plan.rs`.
4. **Re-exports:** explicit named re-exports at the `viewport/mod.rs` boundary (only the parent-facing types); each submodule gets its own `use crate::app::{…}` block. No blanket `pub use *`.

## Critical correctness rules (from adversarial review — verdict: go-with-changes)

These are non-negotiable; ignoring them breaks the build:

- **R1 — Bodies are NOT byte-identical.** Inside `viewport/<sub>.rs`, `super::` resolves to `viewport`, not `app`. Every `super::render::…` / `super::render_job::…` path in `draw_project_render` (lines 865, 887, 888) must become `crate::app::render::…`. Inside `viewport/render.rs`, alias the path tracer to avoid shadowing: `use crate::app::render as path_render;`. The real invariant is **`cargo build -p framer-app` green after every step**, not textual equality.
- **R2 — Per-file imports.** ~278 bare parent references (`FramerApp`, `ViewClick`, `Selection`, `ViewportMode`, `WorkspaceMode`, `theme`, `design`, `resolve_snap`/`SnapContext`/`SnapResult`/`SnapKind`/`GuideAxis`, `kind_label`/`join_kind_label`, `OpeningDragState`/`OpeningEditHandle`/`WallEditHandle`) resolve today only via the top-of-file `use super::…`. Each submodule needs its own `use crate::app::{…}` (and `use crate::app::draw_wall::…` etc.) block, importing only what it uses. Distribute egui/epaint/wgpu imports per file (don't blanket-copy lines 9–13). `epaint::Vertex` belongs only in `view_cube.rs`.
- **R3 — Keep ONE test module.** The bottom `#[cfg(test)] mod tests` (lines 5099–6042) reaches into private fields across multiple future modules in single tests (e.g. `OrbitProjector.scale` + `Scene3d.points` + `View3dState` internals together). Keep it as one `#[cfg(test)] mod tests` in `viewport/mod.rs` with `use super::*` — `mod.rs` re-exports submodule items, so this needs **zero** field-visibility bumps and no helper duplication. (Exception: `view_2d_tests` at line 410 is cleanly separable and travels with `camera_2d.rs`.)
- **R4 — `pub(super)` every cross-boundary item.** Mark these `pub(super)` when moved: `color_to_rgba`, `brighten`, `ViewCubeAction`, `ViewCubeOrientation`, `GpuVertex`, `GpuUniforms`, `Framer3dCallback`, `Framer3dFrameKey`, `Framer3dFrameStore`, the predicates moving to `geom` (`distance_to_segment`, `point_hits_projected_quad`, `polygon_area`, `point_in_polygon`), and `ModelBounds`/`plan_point`/`plan_inverse_point`. (Parent enums `Selection`/`ViewClick`/`ViewportMode`/`WorkspaceMode` are private-to-`app` but visible to descendant modules — no bump needed.)
- **R5 — Preserve the exact re-export surface.** `viewport/mod.rs` must `pub(super) use` `View2dState`, `View3dState`, `WallDragEvent`, `DrawWallPlanInput` with exactly those names. `app/mod.rs:38` and `history_integration_tests.rs:12` import `WallDragEvent` from `viewport`; run `cargo test -p framer-app` (incl. integration tests) after the final step.

## Target module layout (14 files, layered DAG, no cycles)

```
viewport/
  mod.rs              ~430  workspace() dispatcher, canvas_view_controls, canvas_floating_toolbar,
                            draw_nav_cube, workspace_header, *_title, handle_opening_drag_event,
                            DrawWallPlanInput, the re-export surface, and the single `mod tests`
  camera_2d.rs        ~250  View2dState (+Default,+impl), ZOOM/PAN_*_2D consts, apply_view_2d_input,
                            reset_view_on_empty_double_click, view_2d_tests
  camera_3d.rs        ~250  View3dState (+Default,+impl), ViewCubeAction, ViewCubeOrientation,
                            PAN_RADII_PER_VIEWPORT/PAN_MAX_RADII/DOLLY_MIN/DOLLY_MAX
  geom.rs             ~360  Point3 (+impl,+Neg), ProjectedPoint, OrbitProjector (+impl),
                            model_3d_points/center/radius, raw_orbit, distance_to_segment,
                            point_hits_projected_quad, polygon_area, point_in_polygon,
                            ModelBounds (+impl), plan_point, plan_inverse_point
  view_common.rs      ~330  viewport_size, viewport_drawing_rect, render_resolution(+5 tests),
                            draw_view_title/empty/border/background, draw_drafting_grid/rulers,
                            draw_plan_axis_indicator, draw_scale_bar, draw_dashed_line,
                            pointer_started_in_rect(+test), WallElevationLayout (hoisted here)
  gpu.rs              ~350  FRAMER_DEPTH_FORMAT, FRAMER_3D_SHADER, GpuVertex, GpuUniforms(+impl),
                            Framer3dFrameKey/Callback/Resources/Frame/FrameStore, CallbackTrait impl,
                            create_3d_pipeline
  scene_build.rs      ~450  Scene3d(+impl), SceneBuilder(+impl), WallSegmentSpan, WallCuboid(+impl),
                            CuboidFace(+impl), PickSolid(+impl), WallBasis(+impl), CUBOID_FACE_INDICES,
                            color_to_rgba, brighten
  view_cube.rs        ~560  view_cube_rect/body_rect, ViewCubeGeometry/Face/Edge/CornerGeometry(+impl),
                            view_cube_projector/points, ViewCubeFaceSpec, view_cube_face_specs/edges/
                            face_has_edge/face_geometry, draw_view_cube, view_cube_mesh,
                            push_view_cube_face/quad, ViewCubeLabelSpec, view_cube_label_specs,
                            draw_view_cube_edges/labels/projected_label
  axonometric.rs      ~115  AxonometricView, draw_project_axonometric
  render.rs           ~175  impl FramerApp { draw_project_render }, MOTION_COOLDOWN_FRAMES,
                            MOTION_RESOLUTION_SCALE, MIN_RENDER_DIM, MAX_RENDER_DIM
  plan.rs             ~720  draw_project_plan (+PlanView<'_> bundle), draw_wall_handle,
                            draw_selected_wall_handles, hit_selected_wall_handle, snapped_wall_endpoint,
                            draw_wall_overlay, draw_snap_indicator, snap_kind_label,
                            point_in_screen_polygon, WallDragEvent (moved here)
  elevation_design.rs   ~320  DesignElevationClick/Response/View, draw_wall_design_elevation (orchestrator)
  elevation_dimensions.rs ~340  DimensionPlacement, draw_wall_dimension_annotations, dimension tick/
                            label/line/anchor helpers, PendingDimensionPreview, DimensionAnchorMarker/Kind,
                            *_anchor_markers
  elevation_openings.rs   ~300  OpeningDragEvent, OpeningHandleHit, hit_opening_edit/move_target,
                            hit_opening_move/edit_handle, draw_opening_edit_handles, opening_handle_position,
                            opening_drag_delta, cursor_for_opening_handle
  elevation_framing.rs    ~320  draw_wall_elevation, member_rect, draw_opening_guides/guide, opening_rect,
                            draw_member_rect, draw_section_line, member_color, section_position
```

**Dependency tiers (leaves first):**
- Tier 0: `camera_2d`, `camera_3d`
- Tier 1: `geom` (→ camera_2d/3d for `View2dState`/`View3dState` in signatures)
- Tier 2: `view_common` (→ camera_3d), `gpu` (→ geom)
- Tier 3: `scene_build` (→ geom, gpu)
- Tier 4: `view_cube` (→ geom, camera_3d, gpu, scene_build, view_common)
- Tier 5: renderers `plan`, `elevation_design`/`_dimensions`/`_openings`, `axonometric`, `render`
- Tier 6: `elevation_framing`, `mod.rs`

`WallElevationLayout` is hoisted to `view_common` so `elevation_framing` and `elevation_design` are siblings with no cross-edge. Ambiguous shared helpers (e.g. `opening_rect`, `draw_opening_guides` — possibly used by both design and framing) are resolved per-step by the compiler: if an unresolved-import error shows cross-file use, move the item down to `view_common` or `geom` and `pub(super)` it.

## The PlanView bundle (Task 11)

```rust
/// Plan-view inputs, grouped to match the AxonometricView / DesignElevationView
/// idiom. `camera` and the `*_out` sinks stay separate &mut args.
struct PlanView<'a> {
    model: &'a BuildingModel,
    selected_wall: usize,
    selection: &'a Selection,
    show_grid: bool,
    draw_tool: &'a DrawWallPlanInput,
    room_tool_active: bool,
    active_wall_drag: Option<(usize, WallEditHandle)>,
}

fn draw_project_plan(
    ui: &mut Ui,
    plan: PlanView<'_>,
    camera: &mut View2dState,
    cursor_out: &mut Option<Point2>,
    toolbar_out: &mut Option<Pos2>,
    snap_out: &mut Option<SnapResult>,
    wall_drag_out: &mut Option<WallDragEvent>,
) -> Option<ViewClick> {
    let PlanView { model, selected_wall, selection, show_grid, draw_tool, room_tool_active, active_wall_drag } = plan;
    /* body unchanged */
}
```

Destructure at the top so the body is untouched. Update the single call site in `workspace()`.

---

## Execution protocol (every task)

Each task moves one cluster into its new file. The pattern is identical:

1. Create `viewport/<file>.rs`; move the listed items verbatim.
2. Add the file's own `use` block per **R2** (let the compiler tell you what's missing).
3. Rewrite any `super::` paths per **R1**.
4. Add `mod <file>;` to `viewport.rs` (still the monolith during migration) / `viewport/mod.rs` (after Task 0), and `pub(super) use <file>::<ParentFacingType>;` only for items used outside the new file.
5. `pub(super)` cross-boundary items per **R4**.
6. **Verify:** `cargo build -p framer-app` → green. Then `cargo test -p framer-app` → all pass.
7. **Commit** with `refactor(viewport): extract <file>`.

**Build invariant:** the crate compiles and all tests pass after *every* task. No task may leave the tree red.

### Task 0: Convert `viewport.rs` → `viewport/mod.rs`
- `git mv crates/framer-app/src/app/viewport.rs crates/framer-app/src/app/viewport/mod.rs`.
- Verify build + tests green (pure move, no code change). Commit: `refactor(viewport): make viewport a directory module`.

### Task 1: `camera_2d.rs` (leaf)
- Move `View2dState`+impls, `ZOOM_MIN_2D`/`ZOOM_MAX_2D`/`PAN_LIMIT_FACTOR_2D`, `apply_view_2d_input`, `reset_view_on_empty_double_click`, and the `view_2d_tests` module.
- `pub(super) use camera_2d::{View2dState, apply_view_2d_input, reset_view_on_empty_double_click};` (whatever the rest of mod uses).
- Risk: very low. Verify + commit.

### Task 2: `camera_3d.rs` (leaf)
- Move `View3dState`+impls, `ViewCubeAction`+impl, `ViewCubeOrientation`+impl, and the `PAN_RADII_PER_VIEWPORT`/`PAN_MAX_RADII`/`DOLLY_MIN`/`DOLLY_MAX` consts.
- `pub(super)` `View3dState`, `ViewCubeAction`, `ViewCubeOrientation` (used by view_cube, axonometric, render, tests).
- Risk: low. Verify + commit.

### Task 3: `geom.rs` (the load-bearing step)
- Move `Point3`(+impl,+Neg), `ProjectedPoint`, `OrbitProjector`(+impl), `model_3d_points/center/radius`, `raw_orbit`, **`distance_to_segment`** (from its accidental position ~3807), **`point_hits_projected_quad`/`polygon_area`/`point_in_polygon`** (from ~3204), `ModelBounds`(+impl), `plan_point`, `plan_inverse_point`.
- `pub(super)` all of the above (referenced by 3–4 later modules).
- Risk: low-medium (getting these moves right prevents later cycles). Verify + commit.

### Task 4: `view_common.rs`
- Move `viewport_size`, `viewport_drawing_rect`, `render_resolution`(+its 5 tests... but per **R3** the render_resolution tests live in the central `mod tests`; move only the fn), `draw_view_title/empty/border/background`, `draw_drafting_grid/rulers`, `draw_plan_axis_indicator`, `draw_scale_bar`, `draw_dashed_line`, `pointer_started_in_rect`, and **`WallElevationLayout`**(+impl, hoisted).
- `pub(super)` everything moved (used by 4–5 modules).
- Risk: low. Verify + commit.

### Task 5: `gpu.rs`
- Move `FRAMER_DEPTH_FORMAT`, `FRAMER_3D_SHADER`, `GpuVertex`, `GpuUniforms`(+impl), `Framer3dFrameKey/Callback/Resources/Frame/FrameStore`, the `CallbackTrait` impl, `create_3d_pipeline`.
- `pub(super)` `GpuVertex`, `GpuUniforms`, `Framer3dCallback`, `Framer3dFrameKey`, `Framer3dFrameStore` (used by scene_build, view_cube, axonometric).
- Distribute the `bytemuck`/`wgpu`/`egui_wgpu` imports here. Risk: low. Verify + commit.

### Task 6: `scene_build.rs`
- Move `Scene3d`+impl, `SceneBuilder`+impl, `WallSegmentSpan`, `WallCuboid`+impl, `CuboidFace`+impl, `PickSolid`+impl, `WallBasis`+impl, `CUBOID_FACE_INDICES`, `color_to_rgba`, `brighten`.
- `pub(super)` `color_to_rgba`, `brighten` (used by view_cube) + whatever axonometric needs (`Scene3d`).
- `PickSolid::hit_depth` now calls `geom::point_hits_projected_quad` — confirm import. Risk: medium. Verify + commit.

### Task 7: `view_cube.rs`
- Move the whole view-cube family (see layout). `epaint::Vertex` import lives here.
- Risk: medium (size + test fixtures, but no app state). Verify + commit.

### Task 8: `axonometric.rs`
- Move `AxonometricView`, `draw_project_axonometric`. Already param-bundled. Risk: low. Verify + commit.

### Task 9: `render.rs`
- Move `draw_project_render` (keep it as a second `impl FramerApp` block — legal across files), and `MOTION_COOLDOWN_FRAMES`/`MOTION_RESOLUTION_SCALE`/`MIN_RENDER_DIM`/`MAX_RENDER_DIM`.
- Apply **R1**: `super::render::…`→`crate::app::render::…`; alias `use crate::app::render as path_render;` to avoid shadowing `mod render`.
- Risk: medium (only multi-field app-state method). Verify + commit.

### Task 10: `plan.rs` + PlanView refactor (highest behavior surface)
- Move `draw_project_plan`, `draw_wall_handle`, `draw_selected_wall_handles`, `hit_selected_wall_handle`, `snapped_wall_endpoint`, `draw_wall_overlay`, `draw_snap_indicator`, `snap_kind_label`, `point_in_screen_polygon`, and **`WallDragEvent`** (moved here from 3854).
- Introduce `PlanView<'_>` (see above); destructure at top, body unchanged; update the one call site in `workspace()`.
- `pub(super) use plan::WallDragEvent;` from mod.rs so `app/mod.rs:38` + integration test still resolve (**R5**).
- Risk: medium-high. Verify + commit. GUI smoke check recommended (see Final verification).

### Task 11: `elevation_design.rs`
- Move `DesignElevationClick`, `DesignElevationResponse`, `DesignElevationView`, `draw_wall_design_elevation` (the orchestrator calling into dimensions + openings).
- Risk: low coupling. Verify + commit.

### Task 12: `elevation_dimensions.rs`
- Move `DimensionPlacement`, `draw_wall_dimension_annotations`, `draw_dimension_tick`, `dimension_label_position/rect`, `draw_dimension_line_with_label_gap`, `dimension_display_value`, `PendingDimensionPreview`, `draw_pending_dimension_preview`, `pending_dimension_default_line_position`, `dimension_line_offset_for_position`, `dimension_line_screen_position`, `dimension_anchor_position`, `dimension_axis_for_placement_position`, `DimensionAnchorSelection`, `draw_dimension_anchors`, `hit_dimension_anchor`, `DimensionAnchorMarker`, `DimensionAnchorKind`(+impl), `dimension_anchor_markers`, `push_wall/opening/point_anchor_markers`.
- `pub(super)` the items `draw_wall_design_elevation` calls. Risk: medium (size). Verify + commit.

### Task 13: `elevation_openings.rs`
- Move `OpeningDragEvent`, `OpeningHandleHit`, `hit_opening_edit_target`, `hit_opening_move_target`, `hit_opening_move_handle`, `hit_opening_edit_handle`, `draw_opening_edit_handles`, `opening_handle_position`, `opening_drag_delta`, `cursor_for_opening_handle`.
- `pub(super) use elevation_openings::OpeningDragEvent;` (consumed by `mod.rs`/`handle_opening_drag_event`). Risk: medium. Verify + commit.

### Task 14: `elevation_framing.rs`
- Move `draw_wall_elevation`, `member_rect`, `draw_opening_guides`, `opening_rect`, `draw_opening_guide`, `draw_member_rect`, `draw_section_line`, `member_color`, `section_position`.
- If `opening_rect`/`draw_opening_guides` turn out shared with the design side, hoist to `view_common` instead. Risk: low-medium. Verify + commit.

### Final verification
- `cargo fmt -p framer-app`, `cargo clippy -p framer-app` (no new warnings beyond the 2 pre-existing).
- `cargo test -p framer-app` — all green, including `history_integration_tests` (proves **R5**).
- Confirm `viewport/mod.rs` is down to ~430 LOC of dispatcher/chrome + the `mod tests`.
- GUI smoke check via the `verify` skill (Plan pan/zoom, wall drag, Axonometric orbit + view-cube, Render view, Elevation dimension/opening edit) to confirm no behavioral regression.

## Risks / notes
- The exact placement of interleaved helpers (`opening_rect`, `draw_opening_guides`, and any predicate the cluster maps double-listed) is resolved by the compiler at each step — an unresolved-import error across files means "move it down a tier and `pub(super)` it." Do not guess; let `cargo build` adjudicate.
- This is a strictly **sequential** refactor (each task depends on prior extractions compiling), so it is executed in-session step-by-step rather than via parallel subagents.
- Behavior preservation is enforced by the existing test suite + the per-task green-build invariant. No new tests are added; no function body logic changes except the mechanical `super::`→`crate::app::` rewrites (R1) and the `PlanView` destructure (Task 10), both of which are value/borrow-identical.
