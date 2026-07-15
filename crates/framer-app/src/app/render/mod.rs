//! In-app GPU path tracer for the Render view.
//!
//! A WGSL compute kernel (mirroring `framer-render`'s CPU math — see the headless
//! parity test in `tests/gpu_parity.rs`) accumulates path-traced radiance into a
//! storage buffer, and a fullscreen-triangle blit tone-maps it to the egui
//! surface. Integration follows the `egui_wgpu::CallbackTrait` pattern: the
//! compute dispatch is recorded into egui's encoder in `prepare`, and the blit is
//! drawn in `paint`. Progressive refinement accumulates one sample per egui frame
//! and resets when the camera or model changes.
//!
//! GPU resources are cached in `egui_wgpu::CallbackResources` and rebuilt only on
//! target-format, scene-geometry, backend, or resolution changes. The default
//! backend traverses Framer's own BVH in WGSL. When runtime configuration enables
//! ray-query rendering and the wgpu device exposes `EXPERIMENTAL_RAY_QUERY`, an
//! experimental backend uses hardware ray-query traversal over a BLAS/TLAS
//! instead. When the adapter lacks compute support the caller falls back to the
//! CPU renderer (`render_job`).

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu;
use framer_core::BuildingModel;
use framer_render::gpu::{GpuScene, GpuUniforms};
use framer_render::scene::Scene;
use framer_render::{MAX_BOUNCES, RenderOptions, SceneFraming, build_scene};

use super::render_job::model_signature;

/// Fixed seed keeps the GPU render reproducible across runs.
const SEED: u64 = 1;
/// Samples per pixel the progressive render converges to before idling.
const TARGET_SPP: u32 = 256;
/// Compute workgroup tile size (must match `@workgroup_size` in pathtrace.wgsl).
const WORKGROUP: u32 = 8;
/// Upper bound on samples × pixels traced in a single compute dispatch. Bursting
/// many samples per egui frame removes the frame-rate ceiling on convergence, but
/// one dispatch that runs too long can trip the GPU watchdog (TDR); this budget
/// keeps each dispatch bounded regardless of window size.
const SAMPLE_BUDGET_PER_DISPATCH: u64 = 8_000_000;
/// Hard cap on the per-dispatch sample burst (also bounds small-window dispatches).
const MAX_SPP_PER_DISPATCH: u32 = 32;
/// Samples-per-dispatch cap while the camera is moving. A motion frame is a
/// transient, denoised, reduced-resolution preview that is discarded next frame
/// (the accumulation key changes every motion frame, so `frame` resets to 0), so
/// a tiny burst trades preview grain for a large drop in dispatch time. The
/// budget formula otherwise pins motion frames near ~20–32 spp regardless of
/// resolution (`budget_cap = SAMPLE_BUDGET_PER_DISPATCH / pixels` simply lets spp
/// climb to the cap as the resolution drops); capping it to 2 cuts that ~10–16×
/// while still giving the denoiser enough signal to stay legible. Pure tuning
/// knob — no parity impact, since it is gated strictly on `moving` and the
/// still-image accumulation is left byte-for-byte unchanged.
const MOTION_SPP_CAP: u32 = 2;

const RNG_WGSL: &str = include_str!("rng.wgsl");
const PATHTRACE_WGSL: &str = include_str!("pathtrace.wgsl");
const BLIT_WGSL: &str = include_str!("blit.wgsl");
const DENOISE_WGSL: &str = include_str!("denoise.wgsl");

const SOFTWARE_INTERSECTION_BEGIN: &str = "// BEGIN SOFTWARE_BVH_INTERSECTION";
const SOFTWARE_INTERSECTION_END: &str = "// END SOFTWARE_BVH_INTERSECTION";

const RAY_QUERY_INTERSECTION_WGSL: &str = r#"
// ---- Geometry intersection (hardware ray-query traversal) -------------------

@group(0) @binding(6) var scene_tlas: acceleration_structure;

fn intersect_scene(ray: Ray) -> Hit {
    var hit: Hit;
    hit.t = ray.t_max;
    hit.valid = false;

    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, RayDesc(0u, 0xffu, ray.t_min, ray.t_max, ray.origin, ray.dir));
    while (rayQueryProceed(&rq)) {}

    let intersection = rayQueryGetCommittedIntersection(&rq);
    if (intersection.kind == RAY_QUERY_INTERSECTION_NONE) {
        return hit;
    }

    let ti = intersection.primitive_index;
    if (ti >= arrayLength(&triangles)) {
        return hit;
    }

    let tri = triangles[ti];
    hit.t = intersection.t;
    hit.point = ray.origin + ray.dir * intersection.t;
    hit.u = intersection.barycentrics.x;
    hit.v = intersection.barycentrics.y;
    let front = dot(ray.dir, tri.normal) < 0.0;
    hit.front_face = front;
    hit.normal = select(-tri.normal, tri.normal, front);
    hit.geom_normal = tri.normal;
    hit.material = tri.material;
    hit.valid = true;
    return hit;
}

fn occluded(ray: Ray) -> bool {
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, RayDesc(0x5u, 0xffu, ray.t_min, ray.t_max, ray.origin, ray.dir));
    while (rayQueryProceed(&rq)) {}
    let intersection = rayQueryGetCommittedIntersection(&rq);
    return intersection.kind != RAY_QUERY_INTERSECTION_NONE;
}
"#;

/// Accumulated samples below which the display-only À-Trous denoiser runs (and
/// cross-fades out). A camera move resets the sample count to 0, so this also
/// governs how aggressively a freshly-moved (grainy) frame is denoised.
const DENOISE_SPP_LIMIT: u32 = 32;
/// À-Trous wavelet levels (tap strides 1, 2, 4, 8, 16).
const ATROUS_PASSES: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PathTraceBackend {
    ComputeBvh,
    RayQuery,
}

impl PathTraceBackend {
    pub(crate) fn from_config(ray_query_supported: bool, ray_query_enabled: bool) -> Self {
        if ray_query_supported && ray_query_enabled {
            Self::RayQuery
        } else {
            Self::ComputeBvh
        }
    }

    fn shader_source(self) -> String {
        match self {
            Self::ComputeBvh => format!("{RNG_WGSL}\n{PATHTRACE_WGSL}"),
            Self::RayQuery => format!("enable wgpu_ray_query;\n{RNG_WGSL}\n{}", ray_query_wgsl()),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ComputeBvh => "GPU BVH",
            Self::RayQuery => "GPU ray query",
        }
    }
}

fn ray_query_wgsl() -> String {
    let begin = PATHTRACE_WGSL
        .find(SOFTWARE_INTERSECTION_BEGIN)
        .expect("pathtrace.wgsl missing software intersection start marker");
    let end_marker = PATHTRACE_WGSL
        .find(SOFTWARE_INTERSECTION_END)
        .expect("pathtrace.wgsl missing software intersection end marker");
    let end = end_marker + SOFTWARE_INTERSECTION_END.len();
    format!(
        "{}{}{}",
        &PATHTRACE_WGSL[..begin],
        RAY_QUERY_INTERSECTION_WGSL,
        &PATHTRACE_WGSL[end..]
    )
}

/// App-side cache for the GPU render: the current scene, its flattened GPU
/// payload, and the progressive sample counter. Lives on `FramerApp`.
#[derive(Default)]
pub(crate) struct GpuRenderState {
    /// Key over (geometry, camera, lighting, exposure, size): a change restarts accumulation.
    accum_key: u64,
    scene: Option<Scene>,
    /// Orbit framing (pivot + radius) of the cached scene, used to re-aim the
    /// camera on a view change without rebuilding triangles + BVH.
    framing: Option<SceneFraming>,
    /// Geometry-only signature of the cached scene + uploaded GPU buffers.
    scene_key: u64,
    gpu_scene: Option<Arc<GpuScene>>,
    /// Next sample index to dispatch (also the accumulated sample count).
    frame: u32,
}

impl GpuRenderState {
    pub(crate) fn target_spp(&self) -> u32 {
        TARGET_SPP
    }

    pub(crate) fn samples(&self) -> u32 {
        self.frame
    }

    pub(crate) fn is_accumulating(&self) -> bool {
        self.frame < TARGET_SPP
    }

    /// Refreshes the cached scene for the current model + view, resetting the
    /// progressive counter when geometry, camera, lighting, exposure, or size changed.
    ///
    /// Triangles + BVH (and their GPU upload) are geometry-only, so they are
    /// rebuilt only when the geometry signature changes. A pure camera move
    /// (orbit/zoom/resize) re-aims the cached scene's camera from the cached
    /// framing instead — avoiding a full `scene_from_model` rebuild + `Bvh::build`
    /// every interactive frame.
    fn sync(&mut self, model: &BuildingModel, opts: &RenderOptions, width: u32, height: u32) {
        let geom_key = model_signature(model);
        let accum_key = accumulation_key(geom_key, opts, width, height);

        if self.scene_key != geom_key || self.scene.is_none() {
            // Geometry changed: rebuild triangles + BVH and re-upload to the GPU.
            let (scene, framing) = build_scene(model, opts);
            self.gpu_scene = Some(Arc::new(scene.to_gpu()));
            self.scene = Some(scene);
            self.framing = Some(framing);
            self.scene_key = geom_key;
        } else if self.accum_key != accum_key {
            // Same geometry, new view or render settings: update the cached
            // scene fields that feed uniforms without rebuilding triangles/BVH.
            if let (Some(framing), Some(scene)) = (self.framing, self.scene.as_mut()) {
                scene.camera = framing.camera(opts);
                scene.sun = opts.sun;
                scene.sky = opts.sky;
                scene.exposure = opts.exposure;
            }
        }

        if self.accum_key != accum_key {
            self.accum_key = accum_key;
            self.frame = 0;
        }
    }

    /// Builds the callback for this frame and advances the sample counter while
    /// still converging. Returns `None` if there is nothing cached to render.
    #[allow(clippy::too_many_arguments)]
    fn callback(
        &mut self,
        target_id: u64,
        width: u32,
        height: u32,
        srgb_target: bool,
        moving: bool,
        target_format: wgpu::TextureFormat,
        backend: PathTraceBackend,
    ) -> Option<PathTraceCallback> {
        let scene = self.scene.as_ref()?;
        let gpu_scene = Arc::clone(self.gpu_scene.as_ref()?);
        let frame = self.frame;
        let dispatch = frame < TARGET_SPP;

        // Progressive burst: trace several samples per dispatch so convergence is
        // bounded by GPU throughput, not the egui frame cadence. Each dispatch
        // traces sample indices `[frame, frame + spp)`, landing exactly on
        // TARGET_SPP. The burst is capped while the camera moves so an interactive
        // frame costs a fraction of the still-image convergence burst.
        let pixels = (width as u64 * height as u64).max(1);
        let spp = dispatch_spp(frame, pixels, moving);

        let mut uniforms = GpuUniforms::new(scene, width, height, frame, SEED, MAX_BOUNCES);
        // The blit reads this spare lane to decide whether to apply the sRGB
        // transfer function (skipped when the surface format already encodes it).
        uniforms._pad0 = u32::from(srgb_target);
        uniforms.samples_per_dispatch = spp;

        // Display-only denoiser: full strength right after a reset (frame == 0,
        // i.e. every frame while orbiting), fading to the raw, unbiased result as
        // the still image converges. Disabled once enough samples have landed.
        let denoise = dispatch && frame < DENOISE_SPP_LIMIT;
        uniforms.denoise = u32::from(denoise);
        uniforms.denoise_strength = if denoise {
            1.0 - frame as f32 / DENOISE_SPP_LIMIT as f32
        } else {
            0.0
        };

        if dispatch {
            self.frame += spp;
        }

        Some(PathTraceCallback {
            target_id,
            scene_key: self.scene_key,
            gpu_scene,
            uniforms,
            width,
            height,
            dispatch,
            denoise,
            target_format,
            backend,
        })
    }
}

/// Samples to trace in a single dispatch, given the progressive `frame` counter,
/// the dispatch `pixels` count, and whether the camera is moving.
///
/// When still, this is the original budget-bounded burst: as many samples as fit
/// in `SAMPLE_BUDGET_PER_DISPATCH` (capped by `MAX_SPP_PER_DISPATCH`), never
/// overshooting `TARGET_SPP`. While moving, the result is further clamped to
/// `MOTION_SPP_CAP` so a transient preview frame stays cheap. `samples_per_dispatch`
/// only regroups the *same* per-sample math (each sample is seeded from its global
/// index `frame + s`), so capping it during motion does not change the converged
/// still image — see the `wgsl_burst_matches_single_sample` GPU parity test.
fn dispatch_spp(frame: u32, pixels: u64, moving: bool) -> u32 {
    let budget_cap =
        (SAMPLE_BUDGET_PER_DISPATCH / pixels.max(1)).clamp(1, MAX_SPP_PER_DISPATCH as u64) as u32;
    let spp = TARGET_SPP.saturating_sub(frame).min(budget_cap).max(1);
    if moving { spp.min(MOTION_SPP_CAP) } else { spp }
}

/// Hashes everything that, if changed, invalidates progressive accumulation:
/// geometry, the full camera (orbit + pan + dolly + zoom), lighting/exposure,
/// sky, and the render size.
/// Shared by the GPU path and the CPU fallback so they reset identically.
pub(crate) fn accumulation_key(
    geom_key: u64,
    opts: &RenderOptions,
    width: u32,
    height: u32,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    geom_key.hash(&mut hasher);
    ((opts.yaw * 2000.0) as i64).hash(&mut hasher);
    ((opts.pitch * 2000.0) as i64).hash(&mut hasher);
    ((opts.zoom * 1000.0) as i64).hash(&mut hasher);
    ((opts.pan.x * 2000.0) as i64).hash(&mut hasher);
    ((opts.pan.y * 2000.0) as i64).hash(&mut hasher);
    ((opts.pan.z * 2000.0) as i64).hash(&mut hasher);
    ((opts.dolly * 1000.0) as i64).hash(&mut hasher);
    hash_f32(opts.exposure, &mut hasher);
    hash_vec3(opts.sun.dir, &mut hasher);
    hash_vec3(opts.sun.irradiance, &mut hasher);
    hash_f32(opts.sun.angular_radius, &mut hasher);
    hash_vec3(opts.sky.zenith, &mut hasher);
    hash_vec3(opts.sky.horizon, &mut hasher);
    hash_vec3(opts.sky.ground, &mut hasher);
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    hasher.finish()
}

fn hash_f32(value: f32, state: &mut impl std::hash::Hasher) {
    std::hash::Hash::hash(&value.to_bits(), state);
}

fn hash_vec3(value: framer_render::math::Vec3, state: &mut impl std::hash::Hasher) {
    hash_f32(value.x, state);
    hash_f32(value.y, state);
    hash_f32(value.z, state);
}

/// Registers the GPU path-trace callback for `drawing`, syncing cached state and
/// returning whether the renderer is still accumulating (so the caller can keep
/// requesting repaints). Distinct target IDs always route to distinct mutable
/// callback resources, even when their scene, camera, and resolution are
/// otherwise identical. Returns `false` if the GPU scene could not be prepared.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_for_target(
    target_id: u64,
    state: &mut GpuRenderState,
    painter: &eframe::egui::Painter,
    drawing: eframe::egui::Rect,
    model: &BuildingModel,
    opts: &RenderOptions,
    width: u32,
    height: u32,
    moving: bool,
    target_format: wgpu::TextureFormat,
    backend: PathTraceBackend,
) -> bool {
    state.sync(model, opts, width, height);
    let Some(callback) = state.callback(
        target_id,
        width,
        height,
        target_format.is_srgb(),
        moving,
        target_format,
        backend,
    ) else {
        return false;
    };
    painter.add(egui_wgpu::Callback::new_paint_callback(drawing, callback));
    true
}

/// Schedule release of callback-owned GPU resources for a closed logical pane.
pub(crate) fn release_target(painter: &eframe::egui::Painter, target_id: u64) {
    painter.add(egui_wgpu::Callback::new_paint_callback(
        painter.clip_rect(),
        PathTraceCleanupCallback { target_id },
    ));
}

struct PathTraceCleanupCallback {
    target_id: u64,
}

impl CallbackTrait for PathTraceCleanupCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(store) = resources.get_mut::<PtResourceStore<PtResources>>() {
            let _ = store.remove(self.target_id);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        _render_pass: &mut wgpu::RenderPass<'static>,
        _resources: &CallbackResources,
    ) {
    }
}

/// The per-frame paint callback: dispatches one accumulation sample and blits.
struct PathTraceCallback {
    target_id: u64,
    scene_key: u64,
    gpu_scene: Arc<GpuScene>,
    uniforms: GpuUniforms,
    width: u32,
    height: u32,
    /// Whether to run the compute dispatch (false once converged — just blit).
    dispatch: bool,
    /// Whether to run the display-only À-Trous denoise passes and blit the result.
    denoise: bool,
    target_format: wgpu::TextureFormat,
    backend: PathTraceBackend,
}

impl CallbackTrait for PathTraceCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<PtResourceStore<PtResources>>().is_none() {
            resources.insert(PtResourceStore::<PtResources>::default());
        }

        let store = resources
            .get_mut::<PtResourceStore<PtResources>>()
            .expect("pathtrace resource store exists");
        let needs_rebuild = store.get(self.target_id).is_none_or(|resource| {
            resource.target_format != self.target_format || resource.backend != self.backend
        });
        if needs_rebuild {
            let res = PtResources::new(
                device,
                self.target_format,
                &self.gpu_scene,
                self.scene_key,
                self.width,
                self.height,
                self.backend,
            );
            res.build_acceleration_structures(device, queue);
            store.insert(self.target_id, res);
        } else {
            let res = store
                .get_mut(self.target_id)
                .expect("target pathtrace resources exist");
            if res.scene_key != self.scene_key {
                res.upload_scene(device, &self.gpu_scene, self.scene_key);
                res.build_acceleration_structures(device, queue);
            }
            if res.width != self.width || res.height != self.height {
                res.resize(device, self.width, self.height);
            }
        }

        let res = store
            .get(self.target_id)
            .expect("target pathtrace resources exist");
        queue.write_buffer(&res.uniform_buf, 0, bytemuck::bytes_of(&self.uniforms));

        let groups_x = self.width.div_ceil(WORKGROUP);
        let groups_y = self.height.div_ceil(WORKGROUP);

        if self.dispatch {
            // Scope the compute pass so it drops before egui finishes the encoder.
            let mut cpass =
                egui_encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            cpass.set_pipeline(&res.compute_pipeline);
            cpass.set_bind_group(0, &res.scene_bg, &[]);
            cpass.set_bind_group(1, &res.frame_bg, &[]);
            cpass.dispatch_workgroups(groups_x, groups_y, 1);
        }

        // Display-only denoise: average into color_a, then À-Trous wavelet levels
        // ping-pong color_a↔color_b (final result lands in color_b). Each level is
        // a separate compute pass so wgpu inserts the read-after-write barrier.
        if self.denoise {
            {
                let mut pass =
                    egui_encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(&res.resolve_pipeline);
                pass.set_bind_group(0, &res.resolve_bg, &[]);
                pass.dispatch_workgroups(groups_x, groups_y, 1);
            }
            for bg in &res.atrous_bgs {
                let mut pass =
                    egui_encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(&res.atrous_pipeline);
                pass.set_bind_group(0, bg, &[]);
                pass.dispatch_workgroups(groups_x, groups_y, 1);
            }
        }

        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(res) = resources
            .get::<PtResourceStore<PtResources>>()
            .and_then(|store| store.get(self.target_id))
        else {
            return;
        };
        // Blit the denoised buffer (cross-faded toward raw by the uniform) while
        // denoising, otherwise the raw accumulator.
        let blit_bg = if self.denoise {
            &res.blit_bg_denoised
        } else {
            &res.blit_bg
        };
        render_pass.set_pipeline(&res.blit_pipeline);
        render_pass.set_bind_group(0, blit_bg, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// Target-qualified callback resources. The generic value keeps the routing
/// contract independently testable without constructing a GPU device.
struct PtResourceStore<T> {
    targets: HashMap<u64, T>,
}

impl<T> Default for PtResourceStore<T> {
    fn default() -> Self {
        Self {
            targets: HashMap::new(),
        }
    }
}

impl<T> PtResourceStore<T> {
    fn get(&self, target_id: u64) -> Option<&T> {
        self.targets.get(&target_id)
    }

    fn get_mut(&mut self, target_id: u64) -> Option<&mut T> {
        self.targets.get_mut(&target_id)
    }

    fn insert(&mut self, target_id: u64, resources: T) -> Option<T> {
        self.targets.insert(target_id, resources)
    }

    fn remove(&mut self, target_id: u64) -> Option<T> {
        self.targets.remove(&target_id)
    }
}

/// GPU pipelines and buffers cached across frames in `CallbackResources`.
struct PtResources {
    target_format: wgpu::TextureFormat,
    backend: PathTraceBackend,
    compute_pipeline: wgpu::ComputePipeline,
    blit_pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    scene_key: u64,
    scene_bg: wgpu::BindGroup,
    // Held so the bind group's buffer references stay alive.
    _scene_buffers: [wgpu::Buffer; 6],
    rt_scene: Option<RtSceneResources>,
    width: u32,
    height: u32,
    accum_buf: wgpu::Buffer,
    frame_bg: wgpu::BindGroup,
    blit_bg: wgpu::BindGroup,
    // Display-only denoiser resources.
    resolve_pipeline: wgpu::ComputePipeline,
    atrous_pipeline: wgpu::ComputePipeline,
    gbuffer: wgpu::Buffer,
    color_a: wgpu::Buffer,
    color_b: wgpu::Buffer,
    du_bufs: Vec<wgpu::Buffer>,
    resolve_bg: wgpu::BindGroup,
    atrous_bgs: Vec<wgpu::BindGroup>,
    blit_bg_denoised: wgpu::BindGroup,
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

impl PtResources {
    fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        gpu_scene: &GpuScene,
        scene_key: u64,
        width: u32,
        height: u32,
        backend: PathTraceBackend,
    ) -> Self {
        let compute_src = backend.shader_source();
        let compute_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(match backend {
                PathTraceBackend::ComputeBvh => "framer_pathtrace",
                PathTraceBackend::RayQuery => "framer_pathtrace_ray_query",
            }),
            source: wgpu::ShaderSource::Wgsl(compute_src.into()),
        });
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(match backend {
                PathTraceBackend::ComputeBvh => "framer_pathtrace_pipeline",
                PathTraceBackend::RayQuery => "framer_pathtrace_ray_query_pipeline",
            }),
            layout: None,
            module: &compute_module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("framer_blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("framer_blit_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &blit_module,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            // egui's render pass carries a depth attachment (eframe depth_buffer:
            // 24); the blit ignores depth but must declare a matching state.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("framer_pt_uniforms"),
            size: std::mem::size_of::<GpuUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Denoiser pipelines (resolve + À-Trous), sharing one module.
        let denoise_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("framer_denoise"),
            source: wgpu::ShaderSource::Wgsl(DENOISE_WGSL.into()),
        });
        let resolve_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("framer_denoise_resolve"),
            layout: None,
            module: &denoise_module,
            entry_point: Some("resolve"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let atrous_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("framer_denoise_atrous"),
            layout: None,
            module: &denoise_module,
            entry_point: Some("atrous"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let scene_buffers = make_scene_buffers(device, gpu_scene);
        let rt_scene = (backend == PathTraceBackend::RayQuery)
            .then(|| make_rt_scene_resources(device, gpu_scene));
        let scene_bg = make_scene_bind_group(
            device,
            &compute_pipeline,
            &scene_buffers,
            rt_scene.as_ref().map(|rt| &rt.tlas),
        );
        let accum_buf = make_accum_buffer(device, width, height);
        let gbuffer = make_pixel_buffer(device, "framer_pt_gbuffer", width, height);
        let color_a = make_pixel_buffer(device, "framer_pt_color_a", width, height);
        let color_b = make_pixel_buffer(device, "framer_pt_color_b", width, height);
        let du_bufs = make_denoise_uniforms(device, width, height);

        let frame_bg = make_frame_bind_group(
            device,
            &compute_pipeline,
            &uniform_buf,
            &accum_buf,
            &gbuffer,
        );
        let blit_bg =
            make_blit_bind_group(device, &blit_pipeline, &uniform_buf, &accum_buf, &accum_buf);
        let blit_bg_denoised =
            make_blit_bind_group(device, &blit_pipeline, &uniform_buf, &color_b, &accum_buf);
        let resolve_bg =
            make_resolve_bind_group(device, &resolve_pipeline, &du_bufs[0], &accum_buf, &color_a);
        let atrous_bgs = make_atrous_bind_groups(
            device,
            &atrous_pipeline,
            &du_bufs,
            &gbuffer,
            &color_a,
            &color_b,
        );

        Self {
            target_format,
            backend,
            compute_pipeline,
            blit_pipeline,
            uniform_buf,
            scene_key,
            scene_bg,
            _scene_buffers: scene_buffers,
            rt_scene,
            width,
            height,
            accum_buf,
            frame_bg,
            blit_bg,
            resolve_pipeline,
            atrous_pipeline,
            gbuffer,
            color_a,
            color_b,
            du_bufs,
            resolve_bg,
            atrous_bgs,
            blit_bg_denoised,
        }
    }

    fn upload_scene(&mut self, device: &wgpu::Device, gpu_scene: &GpuScene, scene_key: u64) {
        let scene_buffers = make_scene_buffers(device, gpu_scene);
        self.rt_scene = (self.backend == PathTraceBackend::RayQuery)
            .then(|| make_rt_scene_resources(device, gpu_scene));
        self.scene_bg = make_scene_bind_group(
            device,
            &self.compute_pipeline,
            &scene_buffers,
            self.rt_scene.as_ref().map(|rt| &rt.tlas),
        );
        self._scene_buffers = scene_buffers;
        self.scene_key = scene_key;
    }

    fn build_acceleration_structures(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(rt) = &self.rt_scene else {
            return;
        };

        let geometry = wgpu::BlasTriangleGeometry {
            size: &rt.geometry_size,
            vertex_buffer: &rt.vertex_buf,
            first_vertex: 0,
            vertex_stride: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
            index_buffer: None,
            first_index: None,
            transform_buffer: None,
            transform_buffer_offset: None,
        };
        let entry = wgpu::BlasBuildEntry {
            blas: &rt.blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![geometry]),
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("framer_rt_build_encoder"),
        });
        encoder.build_acceleration_structures(std::iter::once(&entry), std::iter::once(&rt.tlas));
        queue.submit(std::iter::once(encoder.finish()));
    }

    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.accum_buf = make_accum_buffer(device, width, height);
        self.gbuffer = make_pixel_buffer(device, "framer_pt_gbuffer", width, height);
        self.color_a = make_pixel_buffer(device, "framer_pt_color_a", width, height);
        self.color_b = make_pixel_buffer(device, "framer_pt_color_b", width, height);
        self.du_bufs = make_denoise_uniforms(device, width, height);
        self.frame_bg = make_frame_bind_group(
            device,
            &self.compute_pipeline,
            &self.uniform_buf,
            &self.accum_buf,
            &self.gbuffer,
        );
        self.blit_bg = make_blit_bind_group(
            device,
            &self.blit_pipeline,
            &self.uniform_buf,
            &self.accum_buf,
            &self.accum_buf,
        );
        self.blit_bg_denoised = make_blit_bind_group(
            device,
            &self.blit_pipeline,
            &self.uniform_buf,
            &self.color_b,
            &self.accum_buf,
        );
        self.resolve_bg = make_resolve_bind_group(
            device,
            &self.resolve_pipeline,
            &self.du_bufs[0],
            &self.accum_buf,
            &self.color_a,
        );
        self.atrous_bgs = make_atrous_bind_groups(
            device,
            &self.atrous_pipeline,
            &self.du_bufs,
            &self.gbuffer,
            &self.color_a,
            &self.color_b,
        );
        self.width = width;
        self.height = height;
    }
}

struct RtSceneResources {
    vertex_buf: wgpu::Buffer,
    geometry_size: wgpu::BlasTriangleGeometrySizeDescriptor,
    blas: wgpu::Blas,
    tlas: wgpu::Tlas,
}

fn make_rt_scene_resources(device: &wgpu::Device, scene: &GpuScene) -> RtSceneResources {
    use wgpu::util::DeviceExt as _;

    let vertices = rt_vertices(scene);
    let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("framer_rt_vertices"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::BLAS_INPUT,
    });
    let geometry_size = wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: vertices.len() as u32,
        index_format: None,
        index_count: None,
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    };
    let blas = device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: Some("framer_rt_blas"),
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::Triangles {
            descriptors: vec![geometry_size.clone()],
        },
    );
    let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
        label: Some("framer_rt_tlas"),
        max_instances: 1,
        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
    });
    tlas[0] = Some(wgpu::TlasInstance::new(
        &blas,
        [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ],
        0,
        0xff,
    ));

    RtSceneResources {
        vertex_buf,
        geometry_size,
        blas,
        tlas,
    }
}

fn rt_vertices(scene: &GpuScene) -> Vec<[f32; 3]> {
    if scene.triangles.is_empty() {
        return vec![[1.0e20, 1.0e20, 1.0e20]; 3];
    }

    let mut vertices = Vec::with_capacity(scene.triangles.len() * 3);
    for tri in &scene.triangles {
        vertices.push(tri.v0);
        vertices.push(add3(tri.v0, tri.edge1));
        vertices.push(add3(tri.v0, tri.edge2));
    }
    vertices
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn storage_init(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    use wgpu::util::DeviceExt as _;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE,
    })
}

fn make_scene_buffers(device: &wgpu::Device, scene: &GpuScene) -> [wgpu::Buffer; 6] {
    [
        storage_init(
            device,
            "framer_pt_triangles",
            bytemuck::cast_slice(&scene.triangles),
        ),
        storage_init(
            device,
            "framer_pt_nodes",
            bytemuck::cast_slice(&scene.nodes),
        ),
        storage_init(
            device,
            "framer_pt_indices",
            bytemuck::cast_slice(&scene.indices),
        ),
        storage_init(
            device,
            "framer_pt_materials",
            bytemuck::cast_slice(&scene.materials),
        ),
        storage_init(
            device,
            "framer_pt_textures",
            bytemuck::cast_slice(&scene.textures),
        ),
        storage_init(
            device,
            "framer_pt_texels",
            bytemuck::cast_slice(&scene.texels),
        ),
    ]
}

fn make_scene_bind_group(
    device: &wgpu::Device,
    compute_pipeline: &wgpu::ComputePipeline,
    buffers: &[wgpu::Buffer; 6],
    tlas: Option<&wgpu::Tlas>,
) -> wgpu::BindGroup {
    let entries = if let Some(tlas) = tlas {
        vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers[0].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers[3].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers[4].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: buffers[5].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: tlas.as_binding(),
            },
        ]
    } else {
        vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers[0].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers[1].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers[2].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers[3].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffers[4].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: buffers[5].as_entire_binding(),
            },
        ]
    };

    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("framer_pt_scene_bg"),
        layout: &compute_pipeline.get_bind_group_layout(0),
        entries: &entries,
    })
}

fn make_accum_buffer(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("framer_pt_accum"),
        size: (width as u64) * (height as u64) * 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

fn make_frame_bind_group(
    device: &wgpu::Device,
    compute_pipeline: &wgpu::ComputePipeline,
    uniform_buf: &wgpu::Buffer,
    accum_buf: &wgpu::Buffer,
    gbuffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("framer_pt_frame_bg"),
        layout: &compute_pipeline.get_bind_group_layout(1),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: accum_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: gbuffer.as_entire_binding(),
            },
        ],
    })
}

/// Builds the blit bind group: `display_buf` (binding 1) is the buffer shown
/// (denoised color, or the raw accumulator when denoising is off); `raw_buf`
/// (binding 2) is always the raw accumulator for the denoise→raw cross-fade.
fn make_blit_bind_group(
    device: &wgpu::Device,
    blit_pipeline: &wgpu::RenderPipeline,
    uniform_buf: &wgpu::Buffer,
    display_buf: &wgpu::Buffer,
    raw_buf: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("framer_pt_blit_bg"),
        layout: &blit_pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: display_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: raw_buf.as_entire_binding(),
            },
        ],
    })
}

/// A `vec4<f32>`-per-pixel storage buffer (gbuffer or denoiser color ping-pong).
fn make_pixel_buffer(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (width as u64) * (height as u64) * 16,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

/// One `DenoiseUniforms` buffer per À-Trous level: `{width, height, step, pad}`
/// with `step = 1, 2, 4, 8, 16`. Rebuilt on resize (width/height change).
fn make_denoise_uniforms(device: &wgpu::Device, width: u32, height: u32) -> Vec<wgpu::Buffer> {
    use wgpu::util::DeviceExt as _;
    (0..ATROUS_PASSES)
        .map(|i| {
            let data: [u32; 4] = [width, height, 1u32 << i, 0];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("framer_denoise_uniforms"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        })
        .collect()
}

fn make_resolve_bind_group(
    device: &wgpu::Device,
    resolve_pipeline: &wgpu::ComputePipeline,
    du: &wgpu::Buffer,
    accum_buf: &wgpu::Buffer,
    color_a: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("framer_denoise_resolve_bg"),
        layout: &resolve_pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: du.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: accum_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: color_a.as_entire_binding(),
            },
        ],
    })
}

/// Per-level À-Trous bind groups, ping-ponging color_a↔color_b. Even levels read
/// color_a and write color_b; odd levels vice-versa. With `ATROUS_PASSES = 5`
/// (odd) the final filtered image lands in color_b.
fn make_atrous_bind_groups(
    device: &wgpu::Device,
    atrous_pipeline: &wgpu::ComputePipeline,
    du_bufs: &[wgpu::Buffer],
    gbuffer: &wgpu::Buffer,
    color_a: &wgpu::Buffer,
    color_b: &wgpu::Buffer,
) -> Vec<wgpu::BindGroup> {
    let layout = atrous_pipeline.get_bind_group_layout(0);
    du_bufs
        .iter()
        .enumerate()
        .map(|(i, du)| {
            let (input, output) = if i % 2 == 0 {
                (color_a, color_b)
            } else {
                (color_b, color_a)
            };
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("framer_denoise_atrous_bg"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: du.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: gbuffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: output.as_entire_binding(),
                    },
                ],
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pathtrace_resource_store_routes_identical_values_by_target() {
        let mut store = PtResourceStore::default();
        store.insert(17, vec![1_u8]);
        store.insert(23, vec![1_u8]);

        store.get_mut(17).expect("target 17 exists").push(2);

        assert_eq!(store.targets.len(), 2);
        assert_eq!(store.get(17).map(Vec::as_slice), Some([1, 2].as_slice()));
        assert_eq!(store.get(23).map(Vec::as_slice), Some([1].as_slice()));
        assert!(store.get(0).is_none());

        assert_eq!(store.remove(17), Some(vec![1, 2]));
        assert!(store.get(17).is_none());
        assert!(store.get(23).is_some());
    }

    #[test]
    fn gpu_render_callback_carries_the_logical_target_id() {
        let model = BuildingModel::demo_wall();
        let opts = RenderOptions::default();
        let mut state = GpuRenderState::default();
        state.sync(&model, &opts, 64, 48);

        let callback = state
            .callback(
                91,
                64,
                48,
                true,
                false,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                PathTraceBackend::ComputeBvh,
            )
            .expect("synced state produces a callback");

        assert_eq!(callback.target_id, 91);
    }

    #[test]
    fn ray_query_shader_replaces_only_intersection_layer() {
        let source = PathTraceBackend::RayQuery.shader_source();
        assert!(source.starts_with("enable wgpu_ray_query;"));
        assert!(source.contains("@group(0) @binding(6) var scene_tlas"));
        assert!(!source.contains(SOFTWARE_INTERSECTION_BEGIN));
        assert!(!source.contains(SOFTWARE_INTERSECTION_END));
        assert_eq!(source.matches("fn intersect_scene").count(), 1);
        assert_eq!(source.matches("fn occluded").count(), 1);

        let compute_source = PathTraceBackend::ComputeBvh.shader_source();
        assert!(compute_source.contains(SOFTWARE_INTERSECTION_BEGIN));
        assert!(compute_source.contains("var<storage, read> nodes"));
    }

    #[test]
    fn rt_vertices_preserve_triangle_primitive_order() {
        let gpu = framer_render::scenes::reference_scene().to_gpu();
        let vertices = rt_vertices(&gpu);
        assert_eq!(vertices.len(), gpu.triangles.len() * 3);

        for (tri, chunk) in gpu.triangles.iter().zip(vertices.chunks_exact(3)) {
            assert_eq!(chunk[0], tri.v0);
            assert_eq!(chunk[1], add3(tri.v0, tri.edge1));
            assert_eq!(chunk[2], add3(tri.v0, tri.edge2));
        }
    }

    #[test]
    fn accumulation_key_reacts_to_view_and_render_settings() {
        use framer_render::math::Vec3;
        // View movement and render settings must restart progressive
        // accumulation without needing a geometry rebuild.
        let base = RenderOptions::default();
        let key = |opts: &RenderOptions| accumulation_key(42, opts, 800, 600);
        let base_key = key(&base);

        let panned = RenderOptions {
            pan: Vec3::new(0.1, -0.2, 0.05),
            ..base
        };
        let dollied = RenderOptions { dolly: 0.5, ..base };
        let exposed = RenderOptions {
            exposure: base.exposure * 1.25,
            ..base
        };
        let sun_shifted = RenderOptions {
            sun: framer_render::scene::DirectionalSun {
                dir: Vec3::new(-0.25, 0.45, 0.86).normalize(),
                ..base.sun
            },
            ..base
        };
        let sky_shifted = RenderOptions {
            sky: framer_render::scene::Sky {
                horizon: Vec3::new(0.65, 0.72, 0.84),
                ..base.sky
            },
            ..base
        };

        assert_ne!(
            base_key,
            key(&panned),
            "pan must invalidate the accumulator"
        );
        assert_ne!(
            base_key,
            key(&dollied),
            "dolly must invalidate the accumulator"
        );
        assert_ne!(
            base_key,
            key(&exposed),
            "exposure must invalidate the accumulator"
        );
        assert_ne!(
            base_key,
            key(&sun_shifted),
            "sun direction must invalidate the accumulator"
        );
        assert_ne!(
            base_key,
            key(&sky_shifted),
            "sky colors must invalidate the accumulator"
        );
    }

    #[test]
    fn gpu_render_sync_updates_cached_scene_render_settings() {
        use framer_render::math::Vec3;

        let model = BuildingModel::demo_shell();
        let mut state = GpuRenderState::default();
        let base = RenderOptions::default();
        state.sync(&model, &base, 320, 180);
        state.frame = 12;
        let scene_key = state.scene_key;

        let changed = RenderOptions {
            exposure: 1.75,
            sun: framer_render::scene::DirectionalSun {
                dir: Vec3::new(0.0, 1.0, 0.0),
                ..base.sun
            },
            ..base
        };

        state.sync(&model, &changed, 320, 180);

        let scene = state.scene.as_ref().expect("scene remains cached");
        assert_eq!(state.scene_key, scene_key, "geometry should stay cached");
        assert_eq!(state.frame, 0, "settings changes reset accumulation");
        assert_eq!(scene.exposure.to_bits(), 1.75_f32.to_bits());
        assert!((scene.sun.dir - Vec3::new(0.0, 1.0, 0.0)).length() < 1.0e-6);
    }

    /// The original budget-bounded burst, kept here as the reference the still
    /// path must continue to match exactly.
    fn still_spp(frame: u32, pixels: u64) -> u32 {
        let budget_cap = (SAMPLE_BUDGET_PER_DISPATCH / pixels.max(1))
            .clamp(1, MAX_SPP_PER_DISPATCH as u64) as u32;
        TARGET_SPP.saturating_sub(frame).min(budget_cap).max(1)
    }

    #[test]
    fn moving_caps_spp_to_motion_budget() {
        // A ~816×480 reduced-resolution motion frame: the still formula would
        // burst ~20 spp (8M / 391_680 ≈ 20), pinning per-frame work near the 8M
        // budget. A moving frame is clamped to the cheap motion cap instead.
        let pixels = 816 * 480;
        assert_eq!(still_spp(0, pixels), 20);
        assert_eq!(dispatch_spp(0, pixels, false), 20);
        assert_eq!(dispatch_spp(0, pixels, true), MOTION_SPP_CAP);

        // Small windows pin the still burst at the 32-spp hard cap; motion still
        // clamps to MOTION_SPP_CAP.
        let small = 320 * 240;
        assert_eq!(still_spp(0, small), MAX_SPP_PER_DISPATCH);
        assert_eq!(dispatch_spp(0, small, true), MOTION_SPP_CAP);
    }

    #[test]
    fn still_path_is_unchanged_by_motion_cap() {
        // Across a sweep of sizes and progress, the non-moving spp is byte-for-byte
        // the original budget-bounded formula — the converged still image is
        // therefore untouched by the motion change.
        for &pixels in &[64u64 * 64, 500 * 281, 816 * 480, 1000 * 1000, 2000 * 2000] {
            for frame in [0u32, 1, 31, 100, 255, 256, 1000] {
                assert_eq!(
                    dispatch_spp(frame, pixels, false),
                    still_spp(frame, pixels),
                    "still spp diverged at pixels={pixels} frame={frame}"
                );
            }
        }
    }

    #[test]
    fn moving_spp_is_bounded_and_nonzero() {
        for &pixels in &[64u64 * 64, 250_000, 391_680, 1_000_000, 16_000_000] {
            for frame in [0u32, 1, 31, 255, 256, 1000] {
                let spp = dispatch_spp(frame, pixels, true);
                assert!(
                    spp >= 1,
                    "spp must stay >= 1 (pixels={pixels} frame={frame})"
                );
                assert!(
                    spp <= MOTION_SPP_CAP,
                    "moving spp must not exceed MOTION_SPP_CAP (pixels={pixels} frame={frame})"
                );
            }
        }
    }
}
