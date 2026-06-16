# GPU Compute Path Tracer — Implementation Reference

> Version-correct reference for building the WGSL compute path tracer that mirrors
> the tested CPU renderer in `framer-render`. Distilled from a research pass and
> validated against the existing wgpu-29 code already in
> `crates/framer-app/src/app/viewport.rs`. See the design + plan:
> [render-view-mode-design.md](2026-06-15-render-view-mode-design.md),
> [render-view-mode.md](2026-06-15-render-view-mode.md).

**Targets:** wgpu **29.0.3** / naga 29, eframe/egui **0.34.2**, embedded via
`egui_wgpu::CallbackTrait`. The CPU renderer is the source of truth; the WGSL
kernel must reproduce its math (same PCG32, same BSDFs, same ACES) so the
[headless parity test](#7-validation-headless-readback) holds.

---

## 0. wgpu-29 breaking-change checklist (read first)

Most older tutorials predate these and will not compile / will panic:

| Change | What you must do |
|---|---|
| `dispatch()` → **`dispatch_workgroups()`** | Use the new name (and `dispatch_workgroups_indirect`). |
| `PipelineLayoutDescriptor.bind_group_layouts: &[Option<&BindGroupLayout>]` | Wrap each in `Some(...)`. (The existing 3D pipeline already does this.) |
| `VertexState.buffers: &[Option<VertexBufferLayout>]` | Wrap in `Some(...)` — or avoid entirely with a vertex-pulling fullscreen triangle (§6). |
| naga 29: integer inter-stage I/O **must** be `@interpolate(flat)` | Any `u32`/`i32`/vec passed vertex→fragment needs the explicit attribute. Pure compute is unaffected; the blit pass is if it passes ints. |
| `LoadOp::DontCare` now requires `unsafe` | Use `LoadOp::Clear`/`Load`. |
| `Buffer::get_mapped_range` returns `Result` | Handle it (matters for the readback test, not the hot path). |
| **Drop the `ComputePass`/`RenderPass` before `encoder.finish()`** | Scope each pass in a `{ }` block, or `finish()` panics. This is the #1 wgpu-23+ gotcha. |

Sources: [wgpu CHANGELOG](https://github.com/gfx-rs/wgpu/blob/trunk/CHANGELOG.md),
[renderpass ownership PR #5884](https://github.com/gfx-rs/wgpu/pull/5884),
[ComputePass-before-finish #6145](https://github.com/gfx-rs/wgpu/issues/6145).

---

## 1. `egui_wgpu::CallbackTrait` (exact 0.34 surface)

```rust
pub trait CallbackTrait: Send + Sync {
    fn paint(&self, info: PaintCallbackInfo,
             render_pass: &mut RenderPass<'static>,        // note: 'static
             callback_resources: &CallbackResources);
    fn prepare(&self, _device: &Device, _queue: &Queue,
               _screen_descriptor: &ScreenDescriptor,
               _egui_encoder: &mut CommandEncoder,
               _callback_resources: &mut CallbackResources) -> Vec<CommandBuffer> { Vec::new() }
    fn finish_prepare(&self, ...) -> Vec<CommandBuffer> { Vec::new() }
}
```

Order: all `prepare` run **before** the egui render pass; `paint` runs **inside**
it. So: **run the compute dispatch in `prepare`**, **sample/blit the result in
`paint`**.

```rust
fn prepare(&self, device, queue, _sd, egui_encoder, res) -> Vec<CommandBuffer> {
    let r: &mut PtResources = res.get_mut().unwrap();
    queue.write_buffer(&r.uniform_buf, 0, bytemuck::bytes_of(&self.uniforms));
    {   // scope: ComputePass must drop before egui finishes the encoder
        let mut cpass = egui_encoder.begin_compute_pass(&Default::default());
        cpass.set_pipeline(&r.compute_pipeline);
        cpass.set_bind_group(0, &r.scene_bg, &[]);
        cpass.set_bind_group(1, &r.frame_bg, &[]);
        cpass.dispatch_workgroups(self.wg_x, self.wg_y, 1);   // NOT dispatch()
    }
    Vec::new()
}

fn paint(&self, _info, rp: &mut RenderPass<'static>, res: &CallbackResources) {
    let r: &PtResources = res.get().unwrap();
    rp.set_pipeline(&r.blit_pipeline);
    rp.set_bind_group(0, &r.blit_bg, &[]);
    rp.draw(0..3, 0..1);                  // fullscreen triangle, no vertex buffer
}
```

Everything bound in `paint` must outlive the pass → store it in
`CallbackResources` (a type-indexed map). `paint` **cannot** open a new render
pass; do all compute in `prepare`. Register the callback with
`ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, MyCallback { uniforms }))`.

Sources: [CallbackTrait docs 0.34.2](https://docs.rs/egui-wgpu/0.34.2/egui_wgpu/trait.CallbackTrait.html),
[custom3d_wgpu example](https://github.com/emilk/egui/blob/main/crates/egui_demo_app/src/apps/custom3d_wgpu.rs),
[egui discussion #4583 (compute in prepare)](https://github.com/emilk/egui/discussions/4583).

---

## 2. Buffer & resource layout

Use **storage buffers** for geometry/BVH/materials and a **storage-buffer
accumulator** (not a storage texture):

```
group(0) scene (read-only):  @0 triangles  @1 bvh_nodes  @2 materials
group(1) per-frame:          @0 Uniforms (camera/sun/sky/frame/exposure/dims)
                             @1 accum: array<vec4<f32>>  (read_write, width*height)
```

Why a storage buffer for accumulation: `rgba32float` storage textures are
**write-only** in core WebGPU (`read_write` only for `r32*`), which would force a
ping-pong pair. A `read_write` `array<vec4<f32>>` is portable and, since each
pixel is touched by exactly one invocation, needs **no atomics and no ping-pong**.

**vec3 alignment trap:** WGSL `vec3<f32>` is 16-byte-aligned but 12-byte-sized.
Pad every vec3 to vec4 in storage structs and mirror with `#[repr(C)]
#[derive(bytemuck::Pod, Zeroable)]`; assert `size_of` matches. Pack scalars
(material index, leaf flags) into the unused `.w` lanes:

```wgsl
struct Triangle { p0: vec3<f32>, _0: f32, p1: vec3<f32>, _1: f32,
                  p2: vec3<f32>, _2: f32, n: vec3<f32>, mat: u32 };
struct BvhNode  { aabb_min: vec3<f32>, left_first: i32,   // leaf: count>0
                  aabb_max: vec3<f32>, count: i32 };       // internal: count==0
```

> The CPU BVH (`bvh.rs`) is already a flat node array with **consecutive
> children** (right = left+1) and an **iterative** traversal — it ports directly.
> Match its leaf encoding: `count > 0` ⇒ leaf with `left_first` indexing into the
> reordered `indices` array; `count == 0` ⇒ internal node, children at
> `left_first` and `left_first + 1`. Upload `bvh.indices` too (or pre-reorder
> triangles on upload).

---

## 3. Compute kernel skeleton

```wgsl
@group(0) @binding(0) var<storage, read> triangles : array<Triangle>;
@group(0) @binding(1) var<storage, read> bvh_nodes : array<BvhNode>;
@group(0) @binding(2) var<storage, read> materials : array<Material>;
@group(1) @binding(0) var<uniform> u : Uniforms;
@group(1) @binding(1) var<storage, read_write> accum : array<vec4<f32>>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid : vec3<u32>) {
  if (gid.x >= u.width || gid.y >= u.height) { return; }   // guard non-multiples
  var rng = init_rng(gid.xy, vec2(u.width, u.height), u.frame);
  let jitter = vec2<f32>(rand(&rng), rand(&rng)) - 0.5;     // sub-pixel AA
  let ray = generate_camera_ray(vec2<f32>(gid.xy) + 0.5 + jitter);
  let radiance = path_trace(ray, &rng);                     // §4–5
  let idx = gid.y * u.width + gid.x;
  let prev = select(vec4(0.0), accum[idx], u.frame > 0u);   // reset when frame==0
  accum[idx] = prev + vec4(radiance, 1.0);                  // running SUM
}
```

`@workgroup_size(8, 8, 1)` (64 lanes) is the standard 2D sweet spot; dispatch
`ceil(width/8) × ceil(height/8)`. **RNG:** port `framer-render`'s PCG32
(`rng.rs`) — if 64-bit ops are costly on the target, use the 32-bit PCG variant
(`state*747796405u + 2891336453u; word = ((state>>((state>>28)+4))^state)*277803737u; (word>>22)^word`)
and add a 32-bit feature flag on the CPU side for exact bit-parity tests.

---

## 4. WGSL geometry (validate against `bvh.rs` / `geom.rs`)

Iterative BVH traversal with a fixed stack (no recursion on GPU):

```wgsl
const STACK_SIZE : u32 = 64u;
fn intersect_bvh(ray: Ray) -> Hit {
  var hit: Hit; hit.t = 1e30; hit.idx = -1;
  var stack: array<u32, STACK_SIZE>; var sp = 0u;
  stack[sp] = 0u; sp += 1u;
  let inv = 1.0 / ray.dir;
  while (sp > 0u) {
    sp -= 1u;
    let node = bvh_nodes[stack[sp]];
    if (!ray_aabb(ray, inv, node.aabb_min, node.aabb_max, hit.t)) { continue; }
    if (node.count > 0) {                              // LEAF
      let first = u32(node.left_first);
      for (var i = 0u; i < u32(node.count); i += 1u) { intersect_tri(ray, first + i, &hit); }
    } else {                                           // INTERNAL
      stack[sp] = u32(node.left_first);      sp += 1u;
      stack[sp] = u32(node.left_first) + 1u; sp += 1u;
    }
  }
  return hit;
}

fn ray_aabb(ray: Ray, inv: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>, t_max: f32) -> bool {
  let t0 = (lo - ray.origin) * inv; let t1 = (hi - ray.origin) * inv;
  let near = max(max(min(t0.x,t1.x), min(t0.y,t1.y)), min(t0.z,t1.z));
  let far  = min(min(max(t0.x,t1.x), max(t0.y,t1.y)), max(t0.z,t1.z));
  return near <= far && far > 0.0 && near < t_max;
}

fn intersect_tri(ray: Ray, ti: u32, hit: ptr<function, Hit>) {     // Möller–Trumbore
  let T = triangles[ti];
  let e1 = T.p1 - T.p0; let e2 = T.p2 - T.p0;
  let h = cross(ray.dir, e2); let det = dot(e1, h);
  if (abs(det) < 1e-8) { return; }                                 // no backface cull (glass)
  let f = 1.0 / det; let s = ray.origin - T.p0;
  let uu = f * dot(s, h); if (uu < 0.0 || uu > 1.0) { return; }
  let q = cross(s, e1); let vv = f * dot(ray.dir, q); if (vv < 0.0 || uu+vv > 1.0) { return; }
  let t = f * dot(e2, q);
  if (t > 1e-4 && t < (*hit).t) { (*hit).t = t; (*hit).idx = i32(ti); (*hit).uv = vec2(uu,vv); }
}
```

---

## 5. BSDFs & light transport (mirror `material.rs` / `sampling.rs` / `integrator.rs`)

- **Diffuse:** cosine-weighted hemisphere sample; throughput `*= albedo` (cos and
  pdf cancel). pdf = `cos/π`.
- **Metal (GGX):** VNDF sample (Heitz 2018 / Dupuy spherical-cap); throughput
  `*= F * G2(nov,nol) / G1(nov)` with `F = F0 + (1-F0)(1-dot(wi,wm))^5`, `F0 = albedo`.
- **Dielectric glass (exact Fresnel + Snell + TIR):**

```wgsl
fn fresnel_dielectric(cos_i: f32, ior: f32) -> f32 {
  var c = clamp(cos_i, -1.0, 1.0);
  var ei = 1.0; var et = ior;
  if (c < 0.0) { ei = ior; et = 1.0; c = -c; }
  let sin_t = ei/et * sqrt(max(0.0, 1.0 - c*c));
  if (sin_t >= 1.0) { return 1.0; }                       // TIR
  let cos_t = sqrt(max(0.0, 1.0 - sin_t*sin_t));
  let rp = (et*c - ei*cos_t) / (et*c + ei*cos_t);
  let rs = (ei*c - et*cos_t) / (ei*c + et*cos_t);
  return 0.5 * (rp*rp + rs*rs);
}
// choose reflect vs refract by comparing reflectance to rand(&rng); throughput=tint on refract.
```

- **Direct sun (NEE) on diffuse only**, exactly as the CPU integrator: cone-sample
  the sun, shadow-ray test, add `albedo/π * E * cos`. The sun is **excluded from
  the sky gradient**; **specular paths pick up the sun disk on escape** (track a
  `specular_path` flag and add `sun.irradiance / solid_angle` when the escaped
  direction is within the cone) — this avoids double counting. Mirror
  `integrator.rs::radiance`/`direct_sun` and `scene.rs::DirectionalSun::disk_radiance`.
- **Russian roulette** after ≥3 bounces: `p = clamp(max(throughput), 0.05, 1.0)`;
  break if `rand > p`, else `throughput /= p`.
- **Firefly clamp:** clamp the path's total radiance (CPU uses `50.0`).

---

## 6. Tonemap blit (mirror `color.rs`)

Fullscreen triangle with **no vertex buffer** (sidesteps the `Option`-wrapped
`VertexState.buffers` churn), fragment averages + exposure + Narkowicz ACES + sRGB:

```wgsl
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));  // (0,0)(2,0)(0,2)
  return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
fn aces(x: vec3<f32>) -> vec3<f32> {
  return clamp((x*(2.51*x+0.03)) / (x*(2.43*x+0.59)+0.14), vec3(0.0), vec3(1.0));
}
@fragment
fn fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
  let idx = u32(frag.y) * u.width + u32(frag.x);
  let hdr = accum[idx].rgb / max(accum[idx].w, 1.0) * u.exposure;
  let ldr = aces(hdr);
  // If the egui target format is *Srgb, return vec4(ldr,1) and skip manual sRGB
  // (the surface does it). Otherwise apply linear_to_srgb to avoid double-correction.
  return vec4(linear_to_srgb(ldr), 1.0);
}
```

⚠ Check `render_state.target_format`: applying manual sRGB **and** using an sRGB
target double-corrects. Mirror `color.rs::tonemap_to_u8` either way.

---

## 7. Progressive accumulation across egui frames

- Keep `frame: u32` in the uniforms. On camera/model change set `frame = 0` (the
  kernel's `select` then overwrites instead of accumulating). Increment each paint.
- In `App::update`, `ctx.request_repaint()` while `frame < max_spp` so egui keeps
  driving paints with no input; stop requesting once converged to idle the GPU.
- This running-sum scheme is exactly what `framer_render::accumulate` /
  `tonemap_accum` already do on the CPU — reuse the same per-pixel seeding so the
  parity test below holds.

---

## 8. Validation: headless readback

Add `crates/framer-app/tests/gpu_parity.rs`:

1. Request `Instance`/`Adapter`/`Device` headlessly; **skip (don't fail)** if no
   adapter (CI / no GPU).
2. Render the golden reference scene from `framer-render`'s `tests/golden.rs`
   (same `Scene`) on the GPU into the accumulator; copy to a mappable buffer;
   `device.poll(Wait)`; read back; tonemap to RGBA.
3. Compare to `framer_render::render(&scene, w, h, spp, seed)` with a **looser
   MAE** than the CPU golden (e.g. MAE < 6, max < 40) to absorb GPU `f32` /
   algorithm-order differences.

This validates the kernel without a visible window. The `egui_wgpu` in-app wiring
still needs a one-time visual check (or an eframe screenshot-to-PNG hook:
`egui::ViewportCommand::Screenshot` → read `egui::Event::Screenshot` → save PNG).
Keep the CPU path as a fallback when `gpu_target_format` is `None` or adapter
features are missing.

---

## Sources

- wgpu CHANGELOG / v29 breaks: https://github.com/gfx-rs/wgpu/blob/trunk/CHANGELOG.md
- egui_wgpu CallbackTrait 0.34.2: https://docs.rs/egui-wgpu/0.34.2/egui_wgpu/trait.CallbackTrait.html
- egui custom3d_wgpu: https://github.com/emilk/egui/blob/main/crates/egui_demo_app/src/apps/custom3d_wgpu.rs
- egui #4583 (compute in prepare): https://github.com/emilk/egui/discussions/4583
- WebGPU storage textures (access modes): https://webgpufundamentals.org/webgpu/lessons/webgpu-storage-textures.html
- nelari.us weekend raytracer (PCG, vec3 padding, accumulation): https://nelari.us/post/weekend_raytracing_with_wgpu_1/ , https://nelari.us/post/pathtracer_devlog/
- gnikoloff/webgpu-raytracer (BVH layout, WGSL traversal): https://github.com/gnikoloff/webgpu-raytracer
- Heitz 2018 VNDF (JCGT): https://jcgt.org/published/0007/04/01/paper.pdf ; Dupuy & Benyoub spherical caps: https://arxiv.org/pdf/2306.05044
- demofox Fresnel/refraction/TIR/Beer: https://blog.demofox.org/2017/01/09/raytracing-reflection-refraction-fresnel-total-internal-reflection-and-beers-law/
- Narkowicz ACES: https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/
- PBR Book §13.4 (MIS path tracer): https://pbr-book.org/4ed/Light_Transport_I_Surface_Reflection/A_Better_Path_Tracer
