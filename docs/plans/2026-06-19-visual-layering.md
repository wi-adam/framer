<!--
  PLAN: how/when the view-layers feature was built. Point-in-time; spec is durable.
-->

# Visual Layering — Implementation Plan (2026-06-19)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/view-layers.md](../specs/view-layers.md). This file is an archival record of how
> the work was sequenced; the spec is the durable source of truth.

## Goal

Replace the always-on colored wall fill in the Plan and 3D views with a 3-state
**wall display mode** (Outline / Width / Full, default Outline) plus per-layer
visibility toggles (Grid, Rooms, Joins, Wall labels), surfaced through a **Layers**
popover. Session-only state; the Render view is untouched.

## Architecture / stack summary

Builds on existing presentation plumbing: view-toggle fields on `FramerApp` flow
into the view bundles (`PlanView`, `AxonometricView`) and are read by the renderers.
Reuses `PlanWallBasis`/`band_quad`/`draw_dashed_line`/`wall_plan_thickness` (2D),
and `WallCuboid`/`OrbitProjector::project_point` (3D). No core/solver changes.

## Slices / phases

### Slice 1 — Layer state + Layers popover

- **Task 1.1** — Define `WallDisplay` (enum, `#[default] Outline`) + `ViewLayers`
  (struct, all-visible default); replace `FramerApp.grid` with `layers: ViewLayers`.
  - Files: `framer-app/src/app/mod.rs`
  - Verify: `cargo check -p framer-app`
  - Commit: `feat(app): wall display mode + view-layers state`
- **Task 1.2** — Layers popover (`menu_button` with the wall-mode selector + four
  visibility toggles) in the status bar, replacing the standalone Grid toggle.
  - Files: `framer-app/src/app/panels.rs`
  - Commit: `feat(panels): Layers popover (wall mode + visibility)`

### Slice 2 — Plan-view rendering

- **Task 2.1** — Thread `layers` through `PlanView`; branch the wall body on the
  mode (`draw_wall_width` for Width, `draw_wall_layers` for Full); gate the opening
  white-gap to Full; gate rooms/joins/wall-labels by their flags.
  - Files: `framer-app/src/app/viewport/plan.rs`, `framer-app/src/app/viewport/mod.rs`
  - Verify: `cargo test -p framer-app` + visual check of each mode
  - Commit: `feat(viewport): per-mode plan wall rendering + layer toggles`

### Slice 3 — 3D/axonometric rendering

- **Task 3.1** — Add `wall_display` to `Scene3d::from_project`; branch
  `push_wall_envelope` (Full bands / Width monochrome band / Outline envelope
  edges); collect `outline_edges`; draw them as a painter overlay and guard the
  empty wgpu callback.
  - Files: `framer-app/src/app/viewport/scene_build.rs`,
    `framer-app/src/app/viewport/axonometric.rs`,
    `framer-app/src/app/viewport/mod.rs`
  - Verify: `cargo test -p framer-app` (the `scene_3d_*` mode tests)
  - Commit: `feat(viewport): wall display modes in the 3D view`

### Slice 4 — Tests + docs

- **Task 4.1** — Default-state + per-mode/layers-hidden no-panic tests
  (`ui_harness_tests.rs`); direct 3D-mode build assertions (`viewport/mod.rs` tests).
  - Commit: `test(app): cover wall display modes and layer toggles`
- **Task 4.2** — This plan + `docs/specs/view-layers.md`; specs README + code-map row.
  - Commit: `docs: spec + plan for visual layering`

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
```

Plus visual verification of the installed app: Plan view (Outline default → Width
dashed faces → Full colored bands; toggle each layer off), 3D view (Full bands →
Width monochrome → Outline edges), and that the Layers popover stays open across
toggle clicks. When done, the spec **Status** is set to Implemented.
