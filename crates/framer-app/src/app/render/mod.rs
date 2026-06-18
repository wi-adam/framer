//! In-app GPU compute path tracer for the Render view.
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
//! target-format, scene-geometry, or resolution changes. When the adapter lacks
//! compute support the caller falls back to the CPU renderer (`render_job`).

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

/// Accumulated samples below which the display-only À-Trous denoiser runs (and
/// cross-fades out). A camera move resets the sample count to 0, so this also
/// governs how aggressively a freshly-moved (grainy) frame is denoised.
const DENOISE_SPP_LIMIT: u32 = 32;
/// À-Trous wavelet levels (tap strides 1, 2, 4, 8, 16).
const ATROUS_PASSES: usize = 5;

/// App-side cache for the GPU render: the current scene, its flattened GPU
/// payload, and the progressive sample counter. Lives on `FramerApp`.
#[derive(Default)]
pub(crate) struct GpuRenderState {
    /// Key over (geometry, camera, size): a change restarts accumulation.
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
    /// progressive counter when geometry, camera, or size changed.
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
            // Same geometry, new view: re-aim the camera only.
            if let (Some(framing), Some(scene)) = (self.framing, self.scene.as_mut()) {
                scene.camera = framing.camera(opts);
            }
        }

        if self.accum_key != accum_key {
            self.accum_key = accum_key;
            self.frame = 0;
        }
    }

    /// Builds the callback for this frame and advances the sample counter while
    /// still converging. Returns `None` if there is nothing cached to render.
    fn callback(
        &mut self,
        width: u32,
        height: u32,
        srgb_target: bool,
        moving: bool,
        target_format: wgpu::TextureFormat,
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
            scene_key: self.scene_key,
            gpu_scene,
            uniforms,
            width,
            height,
            dispatch,
            denoise,
            target_format,
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
/// geometry, the full camera (orbit + pan + dolly + zoom), and the render size.
/// Shared by the GPU path and the CPU fallback so they reset identically.
pub(crate) fn accumulation_key(geom_key: u64, opts: &RenderOptions, width: u32, height: u32) -> u64 {
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
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    hasher.finish()
}

/// Registers the GPU path-trace callback for `drawing`, syncing cached state and
/// returning whether the renderer is still accumulating (so the caller can keep
/// requesting repaints). Returns `false` if the GPU scene could not be prepared.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    state: &mut GpuRenderState,
    painter: &eframe::egui::Painter,
    drawing: eframe::egui::Rect,
    model: &BuildingModel,
    opts: &RenderOptions,
    width: u32,
    height: u32,
    moving: bool,
    target_format: wgpu::TextureFormat,
) -> bool {
    state.sync(model, opts, width, height);
    let Some(callback) =
        state.callback(width, height, target_format.is_srgb(), moving, target_format)
    else {
        return false;
    };
    painter.add(egui_wgpu::Callback::new_paint_callback(drawing, callback));
    true
}

/// The per-frame paint callback: dispatches one accumulation sample and blits.
struct PathTraceCallback {
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
        let needs_rebuild = resources
            .get::<PtResources>()
            .is_none_or(|r| r.target_format != self.target_format);
        if needs_rebuild {
            let res = PtResources::new(
                device,
                self.target_format,
                &self.gpu_scene,
                self.scene_key,
                self.width,
                self.height,
            );
            resources.insert(res);
        } else {
            let res = resources
                .get_mut::<PtResources>()
                .expect("pathtrace resources exist");
            if res.scene_key != self.scene_key {
                res.upload_scene(device, &self.gpu_scene, self.scene_key);
            }
            if res.width != self.width || res.height != self.height {
                res.resize(device, self.width, self.height);
            }
        }

        let res = resources
            .get::<PtResources>()
            .expect("pathtrace resources exist");
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
        let Some(res) = resources.get::<PtResources>() else {
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

/// GPU pipelines and buffers cached across frames in `CallbackResources`.
struct PtResources {
    target_format: wgpu::TextureFormat,
    compute_pipeline: wgpu::ComputePipeline,
    blit_pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    scene_key: u64,
    scene_bg: wgpu::BindGroup,
    // Held so the bind group's buffer references stay alive.
    _scene_buffers: [wgpu::Buffer; 4],
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
    ) -> Self {
        let compute_src = format!("{RNG_WGSL}\n{PATHTRACE_WGSL}");
        let compute_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("framer_pathtrace"),
            source: wgpu::ShaderSource::Wgsl(compute_src.into()),
        });
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("framer_pathtrace_pipeline"),
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
        let scene_bg = make_scene_bind_group(device, &compute_pipeline, &scene_buffers);
        let accum_buf = make_accum_buffer(device, width, height);
        let gbuffer = make_pixel_buffer(device, "framer_pt_gbuffer", width, height);
        let color_a = make_pixel_buffer(device, "framer_pt_color_a", width, height);
        let color_b = make_pixel_buffer(device, "framer_pt_color_b", width, height);
        let du_bufs = make_denoise_uniforms(device, width, height);

        let frame_bg =
            make_frame_bind_group(device, &compute_pipeline, &uniform_buf, &accum_buf, &gbuffer);
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
            compute_pipeline,
            blit_pipeline,
            uniform_buf,
            scene_key,
            scene_bg,
            _scene_buffers: scene_buffers,
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
        self.scene_bg = make_scene_bind_group(device, &self.compute_pipeline, &scene_buffers);
        self._scene_buffers = scene_buffers;
        self.scene_key = scene_key;
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

fn storage_init(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    use wgpu::util::DeviceExt as _;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE,
    })
}

fn make_scene_buffers(device: &wgpu::Device, scene: &GpuScene) -> [wgpu::Buffer; 4] {
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
    ]
}

fn make_scene_bind_group(
    device: &wgpu::Device,
    compute_pipeline: &wgpu::ComputePipeline,
    buffers: &[wgpu::Buffer; 4],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("framer_pt_scene_bg"),
        layout: &compute_pipeline.get_bind_group_layout(0),
        entries: &[
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
        ],
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
    fn accumulation_key_reacts_to_pan_and_dolly() {
        use framer_render::math::Vec3;
        // A pan or dolly is a camera move and must restart progressive
        // accumulation, exactly like yaw/pitch/zoom already do.
        let base = RenderOptions::default();
        let key = |opts: &RenderOptions| accumulation_key(42, opts, 800, 600);
        let base_key = key(&base);

        let panned = RenderOptions {
            pan: Vec3::new(0.1, -0.2, 0.05),
            ..base
        };
        let dollied = RenderOptions {
            dolly: 0.5,
            ..base
        };

        assert_ne!(base_key, key(&panned), "pan must invalidate the accumulator");
        assert_ne!(base_key, key(&dollied), "dolly must invalidate the accumulator");
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
                assert!(spp >= 1, "spp must stay >= 1 (pixels={pixels} frame={frame})");
                assert!(
                    spp <= MOTION_SPP_CAP,
                    "moving spp must not exceed MOTION_SPP_CAP (pixels={pixels} frame={frame})"
                );
            }
        }
    }
}
