# framer-app

The native desktop CAD shell (`eframe`/`egui` + `wgpu`). It holds the authored
`BuildingModel`, caches the solver's `ProjectFramePlan`, and renders the 2D/3D viewports
plus a real-time GPU path-traced **Render** view. This is the only crate with a UI
dependency.

Depends on: `framer-core`, `framer-solver`, `framer-render`.

Entry: `src/main.rs` → `FramerApp` (`src/app/mod.rs`), whose `ui_root` lays out the
app header, workflow command strip, model tree, inspector, viewport, and status bar.

## Module groups

| Path | Purpose |
| --- | --- |
| `src/app/mod.rs` | `FramerApp` state + `impl eframe::App` + `ui_root`; project save/load/export; plan regeneration; selection/undo wiring. |
| `src/app/panels.rs` | Model tree, inspector, app header, workflow command strip, status bar bodies. |
| `src/app/model_edit.rs`, `draw_wall.rs`, `history.rs` | Authored-model mutation primitives; draw-wall snapping + auto-joins; undo/redo. |
| `src/app/render_job.rs`, `project_io.rs`, `labels.rs`, `theme.rs` | Background CPU render job; file/export helpers; labels; theme shim. |
| `src/app/design/` | Design system: theme install, tokens, palette, Lucide icons, semantic widgets. |
| `src/app/viewport/` | The viewports (layered modules): `mod.rs` dispatcher, `plan.rs`, `elevation_*`, `axonometric.rs`, `camera_2d/3d.rs`, `scene_build.rs` (`Scene3d::from_project`), `gpu.rs`, `render.rs`, `view_cube.rs`. |
| `src/app/render/` | Real-time **GPU** compute path tracer: `GpuRenderState` + WGSL shaders (`pathtrace.wgsl`, `blit.wgsl`, `denoise.wgsl`, `rng.wgsl`) that mirror `framer-render`. |

## Run

```sh
cargo run -p framer-app          # opens examples/projects/demo-shell.framer
```

To screenshot or drive the app with GUI tools, build + install it first (only installed
bundles are visible to macOS screen capture) — see `.claude/skills/install-app`.

## Test

```sh
cargo test -p framer-app                                          # incl. headless egui UI tests
cargo test -p framer-app --test gpu_parity -- --nocapture        # GPU↔CPU path-tracer parity
```

Headless UI tests use `egui_kittest` (`src/app/ui_harness_tests.rs`) — note the font warm-up /
`with_max_steps` gotchas documented there. The GPU parity test skips when no adapter is
available (CI runs it on macOS Metal and on Linux lavapipe).

See [`docs/code-map.md`](../../docs/code-map.md#framer-app--desktop-cad-shell).
