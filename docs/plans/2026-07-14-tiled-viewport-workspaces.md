# Tiled Viewport Workspaces — Implementation Plan (2026-07-14)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/viewport-layouts.md](../specs/viewport-layouts.md). This file is an archival record
> of how the work was sequenced; the spec is the durable source of truth.

## Goal

Deliver variable-N split viewport layouts, independently configured cameras/render runtimes,
deferred native pop-outs, built-in and persisted user presets, and the renderer/resource isolation
needed for repeated 3D and Render panes.

## Architecture / stack summary

The current `FramerApp` owns one global mode and one camera/render runtime, while
`viewport::workspace` draws one renderer per frame. The lower-level Plan, Elevation, and 3D
renderers already accept explicit read-only inputs plus mutable camera state; Render must be
extracted to the same shape. A new app-only split layout owns stable pane identities and per-pane
runtime. Deferred egui 0.35 viewports receive owned snapshots through `Arc<Mutex<_>>` and send
typed events to the root, which remains the sole model/history mutator.

## Risk ledger / coverage matrix

| Contract changed | Boundary | Required proof | Likely review failure if missed |
| --- | --- | --- | --- |
| Persisted custom preset decode/validation | eframe RON → app presentation | Valid round-trip; missing/malformed/unsupported version; blank/oversized/duplicate names and IDs; empty/duplicate/over-deep/over-count tree; bad active leaf; invalid ratio; non-finite/out-of-range pose; valid sibling survives invalid entry | Corrupt or future app storage panics, allocates unbounded panes, or silently erases usable presets |
| Split/close/duplicate/apply topology | layout → pane registry | Stable unique IDs, exact leaf coverage, ratio clamp, last-pane guard, deterministic active fallback, fresh render runtime on duplicate/apply | A close redirects another pane's GPU/window identity or produces a dangling leaf |
| Active-pane workflow/tool/view routing | command surfaces → viewport layout | Existing soft-default tests updated; actions affect active/source pane only; switching Render preserves other panes; diagnostic focus targets one pane | Global commands still replace every pane or act on a stale mode |
| Independent same-type cameras | layout → Plan/Elevation/3D/Render | Two Plan, Elevation, 3D, and Render pane state tests; mutate one and assert the other unchanged; 3D↔Render shares only within one pane | Repeated views mirror or reset one another despite distinct tiles |
| Exact tile sizing and pane-local egui IDs | split layout → renderers/accessibility | Narrow UI harness plus horizontal/vertical/four-up shots; unique accessible labels; tiny-rect guards | Historic minimums overflow cells or repeated Area IDs make controls act on the wrong pane |
| Deferred pop-out snapshot/event/close paths | child callback → root app | Deferred/embedded callback state test, stable `ViewportId`, close docks, child selection/action reaches root, missing snapshot fallback, root repaint request | Child mutates stale model/history, close deletes state, or pop-out only works with immediate callbacks |
| Deferred autosave workaround | eframe integration → app storage | App auto-save interval assertion and direct explicit preset-save storage test; shutdown path remains wired | Periodic child paint overwrites root window geometry or saved preset waits indefinitely |
| Interactive 3D callback resources | pane draw → shared egui-wgpu callback map | Distinct model/ViewCube keys per pane and close cleanup; two-pane UI/GPU shot | Multiple 3D panes both paint the last prepared mesh/camera |
| Path-trace callback resources | pane render state → shared egui-wgpu callback map | Resource-store identity unit test plus two-target off-screen/native smoke where available; GPU parity | Two Render panes resize/overwrite one accumulator, never converge, or blit the wrong view |
| CPU fallback lifecycle | pane runtime → worker thread/texture | Per-pane state isolation and drop/removal test | One pane cancels another's worker or closed panes leak rendering work |
| Product-visible behavior/docs | app UI → durable contract | Spec/status, command/design/camera/render docs, code map, markdown links, UI shots | Code ships with the old “one viewport per frame / Render exactly one view” contract |

## Slices / phases

### Slice 1 — Layout model and persistence

- **Task 1.1** — Add stable pane IDs, split-tree operations, active-pane lifecycle, built-in
  presets, versioned custom-preset DTOs, and adversarial validation.
  - Files: `crates/framer-app/src/app/viewport/layout.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: focused layout/persistence unit tests
  - Commit: `feat(viewport): add tiled layout model and presets`
- **Task 1.2** — Load/save custom presets through eframe storage and wire explicit save behavior.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/main.rs`
  - Verify: memory-storage round-trip/fallback and app-save tests
  - Commit: `feat(app): persist viewport layout presets`

### Slice 2 — Independent pane runtime and docked tiling

- **Task 2.1** — Move cameras and CPU/GPU Render state into per-pane runtime; extract Render into
  an explicit input/state renderer and route app commands through the active pane.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/viewport/render.rs`
  - Verify: `cargo test -p framer-app` plus independent-state and workflow tests
  - Commit: `refactor(viewport): own camera and render state per pane`
- **Task 2.2** — Render the split tree with pane headers, split/duplicate/close/layout controls,
  responsive exact sizing, active focus, and pane-scoped IDs/events.
  - Files: `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/viewport/view_common.rs`, elevation renderers,
    `crates/framer-app/src/app/ui_harness_tests.rs`
  - Verify: focused UI harness tests at `1360 x 860` and `1040 x 680`
  - Commit: `feat(viewport): render resizable tiled panes`

### Slice 3 — Deferred native pop-outs

- **Task 3.1** — Add owned snapshot + typed event bridge, stable deferred viewport builders,
  embedded fallback, Dock/native-close lifecycle, and child shortcut/input routing.
  - Files: `crates/framer-app/src/app/mod.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/viewport/layout.rs`
  - Verify: deferred/embedded UI harness and channel/lifecycle unit tests; installed-app smoke
  - Commit: `feat(viewport): add deferred pop-out panes`

### Slice 4 — Renderer resource isolation

- **Task 4.1** — Qualify interactive 3D model/ViewCube callback resources by pane and clean up
  closed targets.
  - Files: `crates/framer-app/src/app/viewport/gpu.rs`,
    `crates/framer-app/src/app/viewport/axonometric.rs`,
    `crates/framer-app/src/app/viewport/view_cube.rs`
  - Verify: focused frame-key/store tests and two-3D-pane shot
  - Commit: `fix(viewport): isolate 3d callback resources per pane`
- **Task 4.2** — Key path-trace callback resources and app-side CPU/GPU accumulation by pane.
  - Files: `crates/framer-app/src/app/render/mod.rs`,
    `crates/framer-app/src/app/viewport/render.rs`, GPU integration tests
  - Verify: two-target regression, Render smoke, GPU parity
  - Commit: `fix(render): isolate viewport accumulation resources`

### Slice 5 — Product/UI closeout

- **Task 5.1** — Add screenshot-deck states for two-up, four-up, narrow, preset menu, and
  different-angle repeated 3D panes; inspect and tune layout.
  - Files: `crates/framer-app/src/app/ui_shots_tests.rs`, UI implementation as needed
  - Verify: `scripts/ui-shots.sh` plus direct PNG inspection
  - Commit: `test(ui): cover tiled viewport layouts`
- **Task 5.2** — Update durable docs and mark the spec implemented after all gates pass.
  - Files: `docs/specs/viewport-layouts.md`, `docs/specs/command-surfaces.md`,
    `docs/specs/design-system.md`, `docs/specs/2d-view-camera.md`,
    `docs/specs/render-view.md`, `docs/specs/app-configuration.md`,
    `docs/specs/view-layers.md`, `docs/architecture.md`, `docs/code-map.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: `docs(viewport): document tiled workspaces`

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
cargo test -p framer-app --test gpu_parity --locked -- --nocapture
scripts/ui-shots.sh
```

Also run the installed-app deferred-window smoke: pop out two panes, move them to separate monitors,
verify independent camera interaction/repaint, close one native window and observe it dock, save and
reapply a custom preset, quit/relaunch, and confirm the preset plus root-window geometry remain valid.
