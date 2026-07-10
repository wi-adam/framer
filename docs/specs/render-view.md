# Render View Mode

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented; in-app render settings landed 2026-07-09 ·
> **Linked goal:** — · **Plan:** [2026-06-15-render-view-mode](../plans/2026-06-15-render-view-mode.md),
> [2026-07-08 view–workflow alignment](../plans/2026-07-08-view-workflow-alignment.md) ·
> **Last reviewed:** 2026-07-10

## Goal

Add a **Render** view mode to Framer that produces a gorgeous, physically-based
architectural rendering of the authored design — real path-traced lighting,
glass, soft shadows, and PBR materials. This is the first slice of a broader
"presentation" pillar (later: framing visualization, lighting fixtures,
finishes, electrical, flooring). The render must be implemented sustainably,
with a fully-tested, UI-agnostic core, and a headless render path that doubles
as both an export feature and the agent's screenshot/inspection tool.

## Decisions (from product owner)

- **Engine:** GPU compute path tracer for the live in-app render, backed by a
  CPU reference path tracer that is the tested source of truth.
- **Scope:** full vertical slice, fully tested.
- **Materials:** auto-derived from existing model data (no new material UI yet).
- **Tooling:** headless render CLI (`render <project.framer> <out.png>`).

## Architecture

```
framer-core    (existing) — BuildingModel, walls, openings, materials, load_project
framer-solver  (existing) — framing generation (not required for the first slice)
framer-render  (NEW, lib) — scene extraction + BVH + PBR materials + CPU path tracer
   └── bin: render (CLI, feature "cli", uses `image`) — headless PNG export
framer-app     (existing) — ViewportMode::Render + WGSL compute path tracer that
                            mirrors framer-render's math, fed by framer-render's Scene
```

### Single source of truth

`framer-render` owns the scene representation (`Scene`: triangles, BVH nodes,
materials, sun, sky, camera). The **CPU path tracer** renders a `Scene` to RGBA
bytes and is exhaustively unit + golden tested. The **app's WGSL compute path
tracer** consumes the *same* `Scene`, flattened into GPU storage buffers, and
implements the identical math (same formulas, same PCG RNG, same ACES). The GPU
output is validated against the CPU reference via headless captures. There is
exactly one definition of "how a pixel is computed," mirrored in two languages.

### Coordinate system & units

World space matches the existing app: X/Y are plan coordinates, **Z is up**,
units are **inches** (`framer_core::Length`, 16 ticks/inch, `.inches() as f32`).
Walls are placed via the existing `WallBasis` math (along = wall direction,
side = +90° perpendicular: `side = (-along.y, along.x)`).

## `framer-render` module layout

Pure-Rust, `#![forbid(unsafe_code)]`, **zero runtime deps in the library**
(`image`/`rayon` are optional, behind features). `f32` throughout to match WGSL.

| Module | Responsibility |
|---|---|
| `math` (`vec3`, `onb`) | Vec3 ops, reflect/refract, branchless Duff ONB |
| `rng` | PCG32 (XSH-RR 64/32), per-(x,y,sample,seed) SplitMix seeding |
| `ray`, `aabb` | Ray, slab test |
| `geom` | Triangle, Möller–Trumbore (no backface cull — glass from inside), Hit |
| `bvh` | Median-split build (correct, deterministic) + iterative stack traversal; flat node array portable to a GPU storage buffer |
| `material` | `Material` (diffuse / metal-GGX / dielectric-glass / emissive), Fresnel, sample + eval |
| `sampling` | cosine hemisphere, GGX VNDF (spherical caps), sun cone, sky |
| `camera` | eye/forward/up/fov from orbit (yaw/pitch/zoom) + jittered primary rays |
| `color` | Narkowicz ACES + linear→sRGB OETF + HDR→u8 |
| `integrator` | `path()`: NEE + MIS (power heuristic) + Russian roulette |
| `scene` | `Scene` container; `RenderScene::from_model(model, opts)` extraction |
| `lib` | `render(&Scene, w, h, spp, seed) -> Vec<u8>` (RGBA); GPU-buffer flattening |

### Material derivation (auto)

From `BuildingModel` (no framing in the first slice — finished surfaces):

- **Exterior wall surface** (outside face): painted cladding (diffuse, light
  warm grey), governed by `WallAssembly.exposure == Exterior`.
- **Interior wall surface** (inside / interior walls): painted drywall (diffuse,
  near-white).
- **Window / Skylight opening:** glass (dielectric, IOR 1.5, slight tint, smooth).
- **Door opening:** solid wood (diffuse/low-gloss); **GarageDoor:** painted metal
  panel (GGX, low roughness); **Stair:** void (no fill).
- **Ground plane:** large matte plane at Z=0 (soft neutral).
- **Sky + sun:** procedural gradient sky (horizon→zenith) with a sun disk; a
  directional sun light with a small angular radius for soft shadows.

Wall solids are built as cuboids (reusing the `WallBasis` math), with opening
voids subtracted by emitting wall segments around each opening (same approach as
the app's `push_wall_envelope`), and a thin glass/door panel placed in each
opening. The local span of each wall cuboid comes from the same derived
`BuildingModel::wall_envelope_span` used by the app's 3D view: at a `Corner`
join, one visual wall body extends beyond the authored centerline endpoint to
the adjoining wall's outside face while the other retracts to the through wall's
inside face. The resulting butt/lap is closed and volume-disjoint; openings remain
cut from their authored local offsets. See
[wall-corner-laps.md](wall-corner-laps.md). Outward-facing normals drive
exterior-vs-interior material choice.

### Camera from orbit state

Given `View3dState { yaw, pitch, zoom }` and the scene bounding sphere
(`center`, `radius`):

```
forward = normalize(vec3(depth_axis.x*cos(pitch), depth_axis.y*cos(pitch), sin(pitch)))
          where depth_axis = (-sin(yaw)... ) consistent with OrbitProjector
eye     = center - forward * dist,  dist = radius / (sin(vfov/2)) / zoom  (frames sphere)
up      = +Z, re-orthogonalized;  right = normalize(cross(forward, up))
vfov    = fixed (~35°) for a natural architectural perspective
```

The CPU and GPU cameras share this derivation so both frame the model identically
to the existing 3D view's vantage.

## App integration (`framer-app`)

1. `ViewportMode::Render` variant (mod.rs).
2. Toolbar button in the VIEW group (panels.rs) — reuse `widgets::tool_button`.
3. `draw_project_render()` (viewport.rs): build `framer_render::Scene`, derive
   camera from `view_3d` + bounds, register a `PathTraceCallback`.
4. `PathTraceCallback : egui_wgpu::CallbackTrait`:
   - `prepare`: upload/refresh scene storage buffers (triangles, BVH, materials)
     + per-frame uniforms (camera, sun, sky, frame index, exposure); record one
     compute dispatch into egui's encoder (scoped so the `ComputePass` drops
     before `finish`). Accumulate a running sum in a `array<vec4<f32>>` storage
     buffer (`.xyz` = radiance sum, `.w` = sample count). No ping-pong needed.
   - `paint`: fullscreen-triangle blit pipeline (no vertex buffer) that averages
     the accumulator, applies exposure + Narkowicz ACES + sRGB, writes to egui's
     target.
   - Progressive accumulation: `request_repaint()` until `frame == max_spp`;
     reset `frame = 0` when the camera/scene changes (orbit, zoom, model edit).
5. Resources (compute pipeline, blit pipeline, scene buffers, accumulator,
   sample counter) cached in `egui_wgpu::CallbackResources`, keyed and
   invalidated on target-format change and scene-hash change.

GPU is an enhancement layer: if `gpu_target_format` is `None`, the Render tab
shows a clear "renderer unavailable" message (matching existing 3D behavior).

## Headless CLI (`framer-render` bin `render`, feature `cli`)

```
render <project.framer> <out.png> [--width W] [--height H] [--spp N]
       [--seed S] [--yaw deg] [--pitch deg] [--zoom Z]
```

Loads via `framer_core::load_project`, extracts the scene, runs the CPU path
tracer (rayon-parallel behind the `parallel` feature; deterministic regardless
of thread count via per-pixel seeding), writes a PNG via `image`. Non-zero exit
+ clear stderr on any error (bad path, parse error, bad dimensions). This is the
export feature *and* the agent's inspection tool (render → Read the PNG).

## Testing strategy

**Unit (analytic ground truth):** Vec3/reflect/refract round-trip; Duff ONB
(incl. `n=(0,0,-1)`); Möller–Trumbore (hit `t`/barycentrics, parallel miss,
backface still hits); AABB slab; Fresnel (`R0≈0.04` at η=1.5 normal incidence,
→1 at grazing); TIR threshold; BVH (every triangle reachable, traversal result
== brute-force nearest hit); camera ray direction; ACES/sRGB monotonic + join
continuity; PCG32 **canary** (first 8 outputs of `seed(42,54)` locked as
constants, cross-checked vs reference).

**Scene extraction:** wall → expected triangle count + outward normals; joined
corner walls form one closed, volume-disjoint butt/lap; window → glass
material; door → solid; ground plane present; bounds/center correct;
`demo-shell.framer` extracts without panic and is non-empty.

**Physical / convergence:** furnace test (albedo=1 enclosure → environment
radiance within MC noise); known-radiance head-on emissive triangle → exact post-
tonemap bytes; energy conservation (directional albedo ≤ 1).

**Golden snapshot:** fixed 64×48 scene (diffuse+metal+glass+emitter+sun+sky),
16 spp, fixed seed → committed raw RGBA (`tests/golden/*.rgba`); assert
`MAE < 1.0` and `max per-pixel < 8`; `UPDATE_GOLDEN=1` regenerates + writes a
viewable PNG. Determinism: parallel render == single-thread render, byte-identical.

**CLI integration:** render `demo-shell.framer` to a temp PNG; assert valid PNG
+ nonzero size + expected dimensions.

**App:** `viewport_mode` switching to/from `Render` (mirrors existing mod.rs
tests). WGSL parity: a debug GPU-readback render of the fixed scene matches the
CPU reference within a looser MAE (GPU `f32`/precision differences).

## Out of scope (first slice; tracked for later)

Material editor UI; framing/x-ray render overlay; textures/normal maps; HDRI
environment maps; denoiser; reflections of interior furniture; multi-level
stacking polish; roofs/floors as finished surfaces (walls + ground + sky first).

## What shipped (2026-06-15)

The first slice landed end-to-end:

- **`framer-render`** crate: full CPU path tracer (Vec3/ONB, PCG32 RNG, BVH,
  Möller–Trumbore, diffuse/metal/glass/emissive BSDFs, NEE sun + procedural sky,
  Russian roulette, firefly clamp, ACES) — ~90 unit tests plus physical tests
  (furnace/energy conservation), a golden-image regression test, and a CLI
  integration test.
- **Headless CLI** (`render`): path-traces a `.framer` file to PNG.
- **In-app Render view** (`ViewportMode::Render`): background-thread progressive
  path tracing displayed via an egui texture, with orbit/zoom and live
  refinement, reusing the exact `framer-render` core.

The renderer was tuned against real captures (warm sun, cool sky-lit shadows,
glazed windows with visible glass caustics) until it reads as a credible
architectural rendering.

## GPU compute path tracer — shipped (2026-06-16)

The long-term engine — a WGSL compute path tracer in the app for real-time
refinement — has landed, behind the same `Scene`/camera as the CPU tracer and
with a CPU fallback. The version-correct implementation reference (the
`egui_wgpu` integration pattern, storage-buffer layout, WGSL kernel snippets, and
the wgpu-29 breaking-change checklist) lives in
[2026-06-15-gpu-pathtracer-research.md](../plans/2026-06-15-gpu-pathtracer-research.md).
What shipped:

- **Scene flattening** (`framer-render/src/gpu.rs`): `#[repr(C)] bytemuck::Pod`
  mirror structs (`GpuTriangle`/`GpuBvhNode`/`GpuMaterial`/`GpuUniforms`, vec3
  padded to vec4) with `size_of` asserts pinning the std430/std140 layout, and
  `Scene::to_gpu()`. The flat BVH (consecutive children + iterative traversal)
  ports verbatim; `bvh.indices` is uploaded alongside.
- **WGSL kernel** (`framer-app/src/app/render/`): `rng.wgsl` emulates the 64-bit
  PCG XSH-RR state with `vec2<u32>` (full 64-bit multiply/add/shift) so the GPU
  stream is **bit-identical** to the CPU's; `pathtrace.wgsl` mirrors camera ray
  gen, BVH + Möller–Trumbore, diffuse / metal-GGX-VNDF / dielectric BSDFs, NEE
  sun + procedural sky + specular-sees-sun-disk, Russian roulette, and the
  firefly clamp, in the exact same RNG draw order; `blit.wgsl` does the
  fullscreen-triangle exposure + Narkowicz ACES + sRGB tonemap. A running-sum
  `array<vec4<f32>>` accumulator needs no atomics or ping-pong.
- **Integration** (`PathTraceCallback : egui_wgpu::CallbackTrait`): compute
  dispatch recorded into egui's encoder in `prepare` (scoped so the pass drops
  before `finish`), fullscreen blit in `paint`. Pipelines/buffers cached in
  `CallbackResources`, rebuilt on target-format / scene-hash / resolution change;
  progressive accumulation via `request_repaint`, reset on camera/model change.
- **Experimental ray-query backend**: when
  [app runtime configuration](app-configuration.md) enables `render.ray_query`
  and the active native `wgpu` adapter exposes `EXPERIMENTAL_RAY_QUERY`, the app
  requests the feature and the Render view may swap the software WGSL BVH
  traversal for a BLAS/TLAS-backed `ray_query` shader. The default path remains
  the software BVH compute shader, and `framer-render`'s CPU tracer remains the
  correctness reference.
- **Validation** (`framer-app/tests/gpu_parity.rs`, headless `wgpu`, skips
  gracefully without an adapter): (1) the GPU PCG reproduces the CPU canary +
  `pixel_rng` bit-for-bit; (2) the compute kernel renders the golden reference
  scene and matches `framer_render::render` (MAE ≈ 0.03, max < 48 at 64 spp);
  (3) the actual blit shader renders to an offscreen target and matches the CPU
  reference (validating Y-orientation, ACES, and sRGB). The in-app
  `CallbackTrait` wiring is exercised by the `render.smoke_frames` config value
  or `--render-smoke-frames <frames>` startup flag, which drives the Render view
  on the real device and closes cleanly. macOS screen-capture is bypassed
  entirely — egui's framebuffer readback path is used, not OS screen capture.

## In-app render settings

Decided 2026-07-08 (routing lives in [command-surfaces.md](command-surfaces.md);
sequencing in the
[2026-07-08 view–workflow alignment plan](../plans/2026-07-08-view-workflow-alignment.md)):
the Render view is entered via the **Render output workflow tab**, whose command
strip surfaces the engine's existing `RenderOptions` fields (`exposure`,
`sun: DirectionalSun`, `sky: Sky` — `framer-render/src/build.rs`) that the app
today fills with `..RenderOptions::default()`:

- **Sun** — azimuth and elevation controls.
- **Environment** — exposure (sky parameters may join later).
- Settings are **session view state**: never written to `.framer`, no schema
  change. Defaults equal `RenderOptions::default()`, so default output —
  including golden images and the GPU↔CPU parity tests — is unchanged.
- The CPU fallback and the GPU compute path receive **identical options**, and a
  settings change joins the existing camera/scene-change accumulation reset so
  the progressive image restarts cleanly.

## Risks & mitigations

- **GPU integration is fiddly / unverifiable on screen** → CPU reference + the
  headless PNG inspection loop is the primary correctness mechanism; GPU is
  validated against it. macOS screen-capture is bypassed entirely.
- **Path-tracer subtle bugs** → physical (furnace/energy) tests catch the common
  bug classes; golden snapshot pins regressions; adversarial review pass.
- **Build determinism across arch** → MAE tolerance sized for `f32` micro-rounding.
