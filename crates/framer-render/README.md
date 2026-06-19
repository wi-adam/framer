# framer-render

UI-agnostic, physically based CPU path tracer. `#![forbid(unsafe_code)]`. It extracts a
renderable scene from a `BuildingModel`, builds a BVH, and path-traces it with diffuse /
metal / dielectric-glass materials, soft sun shadows, and ACES tone mapping.

**Deterministic:** output is a pure function of the seed and per-pixel/sample index, so a
parallel render is byte-identical to a serial one. The app's real-time GPU compute shader
(`framer-app/src/app/render/*.wgsl`) mirrors this math exactly — **this crate is the
reference implementation**, validated by `framer-app/tests/gpu_parity.rs`.

Depends on: `framer-core`. Consumed by: `framer-app`.

## Modules (selected)

| File | Purpose |
| --- | --- |
| `src/lib.rs` | Public API: `accumulate`, `tonemap_accum`, `render`; re-exports `build::*`. |
| `src/build.rs` | Scene extraction from the model: `scene_from_model`, `build_scene`, `RenderOptions`, `SceneFraming` (auto-derives materials + sky + sun). |
| `src/integrator.rs` | Path-tracing integrator + BSDFs (the reference for the WGSL kernel). |
| `src/bvh.rs` / `src/aabb.rs` / `src/geom.rs` / `src/ray.rs` | BVH + geometry/ray primitives. |
| `src/rng.rs` | `Pcg32`, `pixel_rng` (independent per-pixel streams), stratified jitter. |
| `src/scene.rs`, `src/material.rs`, `src/camera.rs`, `src/color.rs`, `src/sampling.rs`, `src/math/` | Scene + lighting, materials, camera, ACES, sampling, `Vec3`/`Onb`. |
| `src/gpu.rs` | `bytemuck` GPU-mirror structs shared with the app's WGSL shaders. |
| `src/bin/` | Headless `render` CLI (feature `cli`). |

## Entry points

- `scene_from_model(model, &RenderOptions) -> Scene`
- `accumulate(...)` → HDR accumulator; `tonemap_accum(...)` / `render(scene, w, h, spp, seed)`.

## Features

- `cli` — builds the `render` binary (implies `parallel`).
- `parallel` — rayon-parallel rendering.

## Run & test

```sh
cargo test -p framer-render                          # unit + golden image tests
cargo test -p framer-render --features cli           # + CLI integration tests
UPDATE_GOLDEN=1 cargo test -p framer-render --test golden   # regen golden (intentional only)

cargo run -p framer-render --features cli --release --bin render -- \
    examples/projects/demo-shell.framer out.png --width 1280 --height 720 --spp 256
```

See [`docs/code-map.md`](../../docs/code-map.md#framer-render--cpu-path-tracer).
