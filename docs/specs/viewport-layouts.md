# Tiled Viewport Workspaces

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** G-003 (Viewport Interaction) / G-011
> (CAD Workspace UX) · **Plan:**
> [2026-07-14 tiled viewport workspaces](../plans/2026-07-14-tiled-viewport-workspaces.md) ·
> **Last reviewed:** 2026-07-14

## Intent / Purpose

Framer must let a user study one project through several simultaneous views instead of replacing
one global camera whenever their task changes. A workspace may combine view types — for example,
Plan, Elevation, 3D, and Render — or repeat a type with independent cameras, such as two 3D panes
at different angles. A pane can move into a deferred native window so multi-monitor users can
keep the model, drawing, and rendered output visible together.

Viewport layouts are disposable presentation over the same authored model and generated plan.
They never become authored construction intent, but users may explicitly save named app-local
layout presets for reuse across projects and launches.

## Requirements & behavior

- A workspace contains one to sixteen stable-identity viewport panes. Each pane independently
  selects `Plan`, `Roof`, `Elevation`, `3D`, or `Render` and owns its navigation and progressive
  render runtime. Two panes with the same view type never share mutable camera or accumulation
  state.
- Docked panes form a resizable split tree. Users can split the active pane horizontally or
  vertically, resize split dividers, duplicate a pane, and close any pane except the last. Closing
  deterministically activates the nearest surviving pane.
- One pane is active. It has a visible focus treatment; global view commands, workflow soft
  defaults, diagnostic focus, status zoom/cursor readouts, and tool auto-snap target that pane.
  Selection, component visibility/isolation, authored intent, generated plan, active drafting
  level, wall-display/layer settings, and render lighting settings remain shared project/workspace
  state.
- Within a pane, `Plan` and `Roof` share that pane's plan camera; Design/Plan variants of a wall
  elevation share that pane's per-wall camera; `3D` and `Render` share that pane's 3D vantage.
  Different panes never share those cameras. Elevation panes initially follow the globally
  selected wall; pinning different wall targets per pane is future work.
- Workflow tabs remain the global command context, but no workflow transition destroys the
  current layout. Selecting Render activates an existing Render pane or converts the active pane;
  returning to authoring activates/restores an authoring pane. A visible Render pane may continue
  refining while an authoring pane is active.
- Built-in presets are always available and are shipped as typed code, not copied into user
  storage:
  - **Focus** — one pane, preserving the active pane's current view configuration.
  - **Plan + 3D** — a balanced left/right split.
  - **Design Study** — Plan on the left, with Elevation above 3D on the right.
  - **Four Up** — Plan, Elevation, and two 3D panes with visibly different canonical angles.
  - **Design + Render** — 3D and Render side by side.
- Users can save the current topology, split ratios, active pane, pane view types, pop-out state,
  and sanitized 3D/Render camera poses as a named custom preset. Saving the same trimmed name
  replaces that user's prior preset; custom presets can be deleted. Project-dependent 2D pan/zoom,
  selected element IDs, tool state, layer state, and render accumulation are not saved.
- User presets persist through the existing `eframe::Storage` path under the dedicated, versioned
  `framer.viewport-layout-presets.v1` key. Missing, malformed, unsupported-version, duplicate,
  over-deep, over-count, or otherwise invalid data is non-fatal: built-ins and the default Focus
  layout remain available, and valid sibling presets survive an invalid entry. Raw payloads above
  512 KiB are rejected before catalog decoding, and each RON entry crosses the typed DTO boundary
  independently so one type-invalid entry cannot discard valid siblings.
- Camera pose input is finite and bounded by the authoritative 3D camera limits. Runtime camera
  state remains session-only; an explicit Save Preset action copies a sanitized pose into a
  versioned app preference. Neither form enters `.framer`.
- Popping out a pane uses `egui::Context::show_viewport_deferred` with a stable native
  `ViewportId` derived from the pane identity. Deferred children repaint independently and use an
  owned shared-state/message bridge; they never borrow or mutate the root `FramerApp` model/history
  directly. Native close requests dock the pane instead of deleting it. On backends without native
  multi-viewport support, egui's embedded-window fallback remains functional.
- Deferred panes support navigation, view switching, selection, and view-scoped presentation
  actions. Model/history mutations are always returned to and applied by the root app. Modal
  authoring gestures that require synchronous drag ownership remain docked-pane-only in this
  slice.
- Pane-local egui IDs and chrome accessibility labels, 3D frame keys, CPU render jobs, GPU
  accumulation state, and callback resources are qualified by stable pane identity. A 3D context
  menu records and activates its source pane before composition; it does not require one
  simultaneously stored menu object in every pane runtime. Closing or replacing a layout releases
  its per-pane CPU/GPU runtime instead of retaining orphaned workers or buffers.
- Split panes use their exact allocated rectangles. Small panes remain bounded and legible; a
  renderer must not force the historical single-canvas `420 x 360` minimum into a tile.

## Decisions (locked)

- **Split tree, not an automatic grid.** A recursive horizontal/vertical split model preserves
  deliberate topology and ratios in custom presets while supporting arbitrary N within a bounded
  safety limit.
- **Deferred native viewports.** Pop-outs use `show_viewport_deferred`, not immediate viewports, so
  child repaint cost does not multiply root layout work. The required snapshot/event boundary is
  an intentional architecture seam, not a temporary raw-pointer shortcut.
- **Stable pane identity is separate from content.** Runtime IDs are monotonic session identities;
  two identical modes/cameras still require separate input, native-window, and GPU resources.
- **Root owns authored mutation and history.** Deferred callbacks operate on immutable owned
  snapshots and send typed actions to the root. This preserves deterministic edit/history paths
  and avoids locking the document behind a child window.
- **Shared document context, independent cameras.** Selection, visibility, workspace flavor,
  layers, and render lighting remain global; cameras, view mode, and render accumulation are per
  pane. Context-menu composition remains root-owned but is source-qualified: the originating pane
  is activated before the root builds or dispatches its menu.
- **Render is allowed in a mixed layout.** Render remains an output workflow command context, but
  it is no longer a singleton workspace surface. Multiple Render panes are supported and isolated
  by pane-keyed CPU/GPU state.
- **Presets are app preferences, not project data or startup configuration.** Built-ins are typed
  code; user presets use versioned eframe RON persistence. No `.framer` schema or `AppConfig`
  change is involved.
- **Explicit pose snapshot, not raw runtime serialization.** Named presets store a validated DTO
  for yaw, pitch, zoom, radius-relative pan, and dolly. Live `View3dState` remains an internal
  runtime type; 2D screen-point pan is deliberately omitted.
- **Bound untrusted persisted structure.** At most 32 user presets, 16 panes per preset, tree depth
  eight, and 64 Unicode scalar values per name keep malformed local storage from allocating an
  unbounded UI or GPU workload.
- **Close docks; remove is explicit.** A native window close preserves the pane and its place in
  the layout. The pane header's Close action is the only deletion path.

## Architecture (grounded in the codebase)

- `crates/framer-app/src/app/viewport/layout.rs` owns stable pane IDs, split topology, active-pane
  reduction, built-ins, versioned user-preset DTOs, validation, and eframe load/save helpers.
- `crates/framer-app/src/app/viewport/mod.rs` separates workspace chrome from recursive tile
  layout and pane dispatch. It draws all docked panes from one read-only frame input, collects
  pane-tagged events, then lets `FramerApp` apply those events after pane borrows end.
- `crates/framer-app/src/app/viewport/pane.rs` defines `ViewportPaneRuntime`: one
  `View2dState`, a per-wall elevation-camera map, `View3dState`, `RenderPaneState` (CPU fallback,
  `GpuRenderState`, and motion cooldown), plus cursor/snap caches. `workspace_state.rs` maps stable
  pane ids to separately locked runtime handles, reconciles deferred handles, and queues retired
  renderer targets for cleanup.
- `crates/framer-app/src/app/viewport/pane_view.rs` defines the shared read-only `PaneFrame`, its
  clone-owned deferred form, pane interaction policy, and target-tagged canvas events. The deferred
  handle wraps the pane runtime and latest owned document snapshot in `Arc<Mutex<_>>`; typed canvas,
  activation, dock, mode, and presentation-action events return through a channel drained by the
  root.
- `crates/framer-app/src/app/viewport/render.rs` is an explicit renderer over shared
  read-only inputs plus mutable per-pane runtime, matching the existing Plan/Elevation/3D seams.
- `crates/framer-app/src/app/viewport/gpu.rs` keys interactive model and ViewCube callbacks by
  `(pane id, role)`. `crates/framer-app/src/app/render/mod.rs` stores path-trace callback resources
  in a pane-keyed map so different resolutions/cameras own different accumulators.
- `FramerApp` owns workflow routing, root action application, and the shared document/presentation
  snapshot; `ViewportWorkspaceState` owns the layout/runtime registry and custom catalog.
  `eframe::App::save` writes theme plus valid user
  presets. Because pinned eframe 0.35 can periodically persist a deferred child's window geometry
  into the root key, Framer suppresses periodic autosave and explicitly saves named-preset edits;
  clean shutdown still persists against the root window.

## Constraints & invariants

- This is app-side presentation. `framer-core`, `framer-solver`, `framer-geometry`, and
  `framer-render` gain no UI dependency. The feature left then-current `.framer`
  schema v13 unchanged, and layout state remains absent from current schema v14.
- All panes in a frame consume one consistent authored/derived snapshot. Pane events apply only
  after drawing, so later panes cannot observe half-applied edits from an earlier pane.
- CPU render math and WGSL are unchanged. GPU changes concern resource identity/lifetime only and
  must keep CPU/GPU parity green.
- User-preset decode is an untrusted-input boundary: malformed or future data never panics, mutates
  the model, or erases still-valid siblings.

## Out of scope (YAGNI)

- Persisting layouts inside a project or syncing them through a library/cloud account.
- Saving/restoring monitor coordinates; Wayland and changing monitor topology make portable
  placement guarantees unreliable.
- Per-pane authored selection, visibility/layers, render lighting, standards context, or workspace
  flavor.
- Pinning separate walls/levels to different Elevation panes.
- Synchronized/coupled cameras, named camera bookmarks independent of layouts, or arbitrary tabbed
  panes inside one tile.
- Modal wall/opening/dimension drag authoring from deferred child windows in the first slice.
