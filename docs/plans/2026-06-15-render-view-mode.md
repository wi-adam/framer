# Render View Mode Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans / TDD to implement this plan task-by-task.

**Goal:** Add a path-traced "Render" view mode to Framer with a tested CPU
reference renderer, a headless PNG CLI, and a live GPU (WGSL compute) path tracer.

**Architecture:** New `framer-render` crate owns scene extraction + BVH + PBR
materials + a CPU path tracer (the tested source of truth). A headless `render`
binary exports PNGs. `framer-app` gains `ViewportMode::Render`, whose WGSL
compute path tracer mirrors `framer-render`'s math, fed by the same `Scene`.

**Tech Stack:** Rust (edition 2024), wgpu 29 / egui 0.34 (`egui_wgpu` callback,
compute + blit), `image` 0.25 (PNG, CLI only), optional `rayon` (parallel CPU
render). Zero math deps — formulas owned to mirror WGSL.

See the [render-view spec](../specs/render-view.md) for the full design.

Conventions: tests inline `#[cfg(test)] mod tests`; `cargo test --workspace`;
`cargo fmt --all -- --check`; commit after every green task.

---

## Phase 0 — Crate scaffold

### Task 0.1: Create `framer-render` crate, wire into workspace
**Files:** Create `crates/framer-render/Cargo.toml`, `crates/framer-render/src/lib.rs`;
Modify `Cargo.toml` (workspace members + `framer-render` workspace dep).
- `lib.rs`: `#![forbid(unsafe_code)]` + module decls (stubs). Add a trivial
  `#[test] fn crate_links()`.
- Library zero-dep; `image` optional behind `cli`, `rayon` optional behind `parallel`.
- Verify: `cargo test -p framer-render` passes; `cargo build --workspace`.
- Commit: "Scaffold framer-render crate".

---

## Phase 1 — CPU path tracer core (the tested heart)

Each task: write tests first, run (fail), implement, run (pass), commit.

### Task 1.1: `math::vec3`
**Files:** Create `crates/framer-render/src/math/mod.rs`, `.../math/vec3.rs`.
- `Vec3 { x,y,z: f32 }` + `new/splat/add/sub/mul/scale/dot/cross/length/normalize/
  neg/index`, `reflect(v,n)`, `refract(uv,n,eta)->Option<Vec3>` (RTIOW vector form),
  `min/max/component`, `lerp`.
- Tests: dot/cross identities; `reflect` off a plane; `refract` Snell round-trip
  (into glass then out ≈ incoming); `normalize` unit length; refract TIR → `None`.

### Task 1.2: `math::onb` (Duff branchless ONB)
**Files:** Create `.../math/onb.rs`.
- `Onb::from_normal(n) -> (t, b)`; `to_world(local)`.
- Tests: orthonormal (pairwise dot ≈ 0, unit length), `cross(t,b)≈n`, **including
  `n=(0,0,-1)`** and `n=(0,0,1)`.

### Task 1.3: `rng::Pcg32` + per-pixel seeding
**Files:** Create `.../rng.rs`.
- Exact PCG32 XSH-RR 64/32 (`MUL=6364136223846793005`), `seed(state,seq)`,
  `next_u32`, `next_f32` (top 24 bits), `pixel_rng(x,y,sample,global_seed)`
  (SplitMix64 mix).
- Tests: `next_f32 ∈ [0,1)`; mean ≈ 0.5 over 1e6; **canary**: first 8 `next_u32`
  of `seed(42,54)` locked as literals; `pixel_rng` distinct streams for adjacent
  pixels; determinism (same inputs → same outputs).

### Task 1.4: `ray` + `aabb` slab test
**Files:** Create `.../ray.rs`, `.../aabb.rs`.
- `Ray { o,d, inv_d, tmin, tmax }`; `Aabb { min,max }` + `grow(point)`,
  `union`, `surface_area`, `hit(ray, t_max)->bool` (branch-min slab).
- Tests: ray through/around box; degenerate axis; behind-origin miss.

### Task 1.5: `geom::Triangle` + Möller–Trumbore + `Hit`
**Files:** Create `.../geom.rs`.
- `Triangle { v0, e1, e2, n, material }` (precomputed edges + geom normal);
  `Hit { t, u, v, normal, front_face, material }`; `intersect(ray) -> Option<Hit>`
  (no backface cull, `|det| < 1e-8` miss, `t∈[tmin,tmax]`); `centroid`, `aabb`.
- Tests: head-on hit `t`/barycentrics; parallel → `None`; just-outside-edge →
  `None`; **back-face still hits**; `front_face` sign correct both sides.

### Task 1.6: `bvh` (median split, iterative traversal)
**Files:** Create `.../bvh.rs`.
- Flat `BvhNode { aabb, left_first: u32, count: u32 }` (count>0 ⇒ leaf);
  `Bvh::build(&[Triangle])` median-split on longest centroid axis, leaf ≤4;
  `intersect(&tris, ray) -> Option<Hit>` iterative `[u32;64]` stack.
- Deterministic tie-break (stable partition, `<` not `<=`).
- Tests: **traversal nearest-hit == brute-force** over a random-but-seeded set of
  triangles + rays; every triangle index reachable; empty/1-tri edge cases;
  node aabb encloses children.

### Task 1.7: `color` (ACES + sRGB + HDR→u8)
**Files:** Create `.../color.rs`.
- `aces_narkowicz(x: Vec3)`, `linear_to_srgb(c)`, `tonemap_to_u8(hdr, exposure)`.
- Tests: `aces(0)=0`, monotonic; `srgb(0)=0`, `srgb(1)=1`, join continuity at
  0.0031308; full pipeline of a known HDR value → expected bytes.

### Task 1.8: `material::Material` (Fresnel, sample, eval)
**Files:** Create `.../material.rs`, `.../sampling.rs`.
- `Material` enum/struct: `Diffuse{albedo}`, `Metal{albedo,roughness}`,
  `Dielectric{ior, tint, roughness}`, `Emissive{radiance}`; plus `kind` tag.
- `sampling.rs`: `cosine_sample_hemisphere`, `ggx_vndf_sample` (spherical caps),
  D/G/Smith, `fresnel_schlick`, `fresnel_dielectric` (+TIR), `power_heuristic`.
- `Material::scatter(wo, hit, rng) -> Option<Scatter{dir, throughput, specular}>`.
- Tests: Fresnel normal-incidence `R0≈0.04` (η=1.5), grazing→1; dielectric
  TIR triggers at correct angle; diffuse throughput == albedo (cos/pdf cancel);
  metal directional albedo ≤ 1 (MC integration); emissive returns radiance.

### Task 1.9: `camera` (orbit → rays)
**Files:** Create `.../camera.rs`.
- `Camera::orbit(center, radius, yaw, pitch, zoom, aspect, vfov)`; `ray(px,py,
  width,height, jitter) -> Ray`.
- Tests: center pixel ray points at scene center; ray dir normalized; aspect
  scaling; pitch/yaw produce expected eye octant.

### Task 1.10: `scene::Scene` + `integrator::path`
**Files:** Create `.../scene.rs`, `.../integrator.rs`; flesh out `lib.rs::render`.
- `Scene { bvh, triangles, materials, sun: DirectionalSun, sky: Sky, camera }`;
  `Sky::radiance(dir)` (horizon→zenith gradient + sun disk); `DirectionalSun
  { dir, angular_radius, radiance }` + cone sampling + pdf.
- `path(scene, ray, rng, max_bounce) -> Vec3`: NEE (sample sun, shadow ray) +
  MIS (power heuristic) with BSDF sampling + Russian roulette (after bounce 3);
  miss → sky radiance (weighted).
- `render(&Scene, w, h, spp, seed) -> Vec<u8>` (RGBA), per-pixel `pixel_rng`,
  rayon behind `parallel`, deterministic.
- Tests: **furnace** (albedo=1 enclosure → env radiance, MAE small); known-
  radiance head-on emissive → exact bytes; render returns `w*h*4` bytes;
  **parallel == single-thread byte-identical**.

### Task 1.11: Golden snapshot test
**Files:** Create `crates/framer-render/tests/golden.rs`,
`crates/framer-render/tests/golden/reference_64x48.rgba`.
- `render_reference_scene()` (fixed scene: diffuse+metal+glass+emitter+sun+sky),
  64×48, 16 spp, fixed seed; compare to committed RGBA: `MAE < 1.0`, max < 8;
  `UPDATE_GOLDEN=1` regenerates + writes viewable PNG (via CLI helper).
- Commit golden bytes + test.

---

## Phase 2 — Scene extraction from the model

### Task 2.1: `scene::from_model` — walls + openings + ground + sky/sun
**Files:** Create `.../scene_build.rs` (or extend `scene.rs`); add `RenderOptions`.
- `RenderScene::from_model(&BuildingModel, &RenderOptions) -> Scene`:
  - For each wall: emit wall-envelope cuboids around openings (mirror
    `push_wall_envelope` segmentation), outward normal → exterior cladding vs
    interior drywall material; thin glass panel for Window/Skylight, solid panel
    for Door, metal for GarageDoor, void for Stair.
  - Ground plane (2 triangles) at min-Z, large extent.
  - Bounding sphere (center, radius) for the camera; default orbit from
    `View3dState::default()`.
  - Sky + sun defaults (warm late-afternoon sun, blue-grey sky).
- Depends on `framer-core` (add workspace dep).
- Tests: single wall → expected triangle count + outward normals; wall with a
  window → a glass-material triangle exists; door → solid; ground present;
  bounds center/radius sane; `demo-shell.framer` (load via `framer_core::
  load_project`) extracts non-empty without panic.

---

## Phase 3 — Headless render CLI

### Task 3.1: `bin/render.rs` (feature `cli`)
**Files:** Create `crates/framer-render/src/bin/render.rs`; Modify `Cargo.toml`
(`[[bin]] required-features=["cli"]`, `image` optional).
- Parse args (`<in.framer> <out.png>` + `--width/--height/--spp/--seed/--yaw/
  --pitch/--zoom`); load project; extract scene; render; write PNG (`RgbaImage::
  from_raw` → `save`). Clear stderr + non-zero exit on errors.
- Verify by running:
  `cargo run -p framer-render --features cli --bin render -- examples/projects/demo-shell.framer /tmp/demo.png --spp 64`
  then **Read /tmp/demo.png** to visually confirm it's gorgeous.

### Task 3.2: CLI integration test
**Files:** Create `crates/framer-render/tests/cli.rs`.
- Render `demo-shell.framer` to a temp file at low spp; assert PNG magic bytes,
  nonzero size, correct dimensions (decode header via `image`).
- Commit.

**Checkpoint:** adversarial review of the CPU tracer + scene extraction (Workflow).

---

## Phase 4 — App: Render viewport mode (GPU compute path tracer)

### Task 4.1: `ViewportMode::Render` + toolbar + dispatch (no GPU yet)
**Files:** Modify `crates/framer-app/src/app/mod.rs` (enum + Cargo dep on
`framer-render`), `panels.rs` (VIEW group button), `viewport.rs` (match arm +
`draw_project_render` stub that shows a placeholder + builds the scene/camera).
- Tests (mod.rs style): switching `viewport_mode` to `Render` and back works;
  Render mode is reachable from the toolbar state.
- Verify: `cargo build -p framer-app`, `cargo test -p framer-app`.
- Commit.

### Task 4.2: GPU buffer flattening in `framer-render`
**Files:** Create `crates/framer-render/src/gpu.rs` (feature-gated `gpu` or always
compiled, `#[repr(C)] Pod` mirror structs).
- `GpuTriangle`, `GpuBvhNode`, `GpuMaterial`, `GpuUniforms` (vec3→vec4 padded);
  `Scene::to_gpu() -> GpuSceneData { triangles, nodes, materials }`; size asserts.
- Tests: counts match scene; `size_of` mirrors WGSL std430 expectations (16-byte
  alignment); a known triangle round-trips fields.

### Task 4.3: WGSL compute path tracer + blit, `PathTraceCallback`
**Files:** Create `crates/framer-app/src/app/render/mod.rs`,
`.../render/pathtrace.wgsl`, `.../render/blit.wgsl`; wire from `viewport.rs`.
- WGSL mirrors CPU math (PCG, camera, BVH traversal, Möller–Trumbore, BSDFs,
  MIS, RR, sky/sun). Accumulator = `array<vec4<f32>>` storage buffer (running
  sum). Compute recorded into egui encoder in `prepare` (scoped). `paint` =
  fullscreen-triangle blit (ACES + sRGB-aware of target format).
- Progressive: `request_repaint` until `max_spp`; reset on camera/scene change
  (hash scene + camera into a key; frame=0 on change).
- Resources cached in `CallbackResources`; rebuild on target-format / scene-hash
  change.
- Verify (build + run): `cargo run -p framer-app`; orbit in Render mode.

### Task 4.4: GPU↔CPU parity readback test (debug)
**Files:** Create `crates/framer-app/tests/gpu_parity.rs` (headless wgpu,
`Instance`/`Adapter`/`Device` request; skip gracefully if no adapter).
- Render the fixed reference scene on the GPU to a readback buffer; compare to
  the CPU reference with a looser `MAE` (GPU precision); skip (not fail) if no
  GPU available in CI.

---

## Phase 5 — Polish & finish

### Task 5.1: Exposure/quality controls + "Rendering… N spp" overlay
- Small overlay showing accumulation progress; a "Render quality" affordance
  (target spp). Keep minimal.

### Task 5.2: `fmt`, full workspace test, README + docs update
- `cargo fmt --all`; `cargo test --workspace`; update README (Render mode +
  headless render command); note new crate in architecture docs.

### Task 5.3: Final adversarial code review (Workflow) + address findings.

### Task 5.4: Finish branch (superpowers:finishing-a-development-branch) —
merge to `main` once green and renders verified gorgeous.

---

## Verification loop (used throughout)

1. `cargo test -p framer-render` after every Phase-1/2 task.
2. After Phase 3: `render` the demo to PNG and **Read the image** — iterate on
   materials/lighting until it genuinely looks gorgeous.
3. `cargo test --workspace` + `cargo fmt --all -- --check` before each commit
   that touches multiple crates.
4. Adversarial review workflows at the Phase-3 and Phase-5 checkpoints.
