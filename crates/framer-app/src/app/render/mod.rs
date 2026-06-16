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
use framer_render::{MAX_BOUNCES, RenderOptions, scene_from_model};

use super::render_job::model_signature;

/// Fixed seed keeps the GPU render reproducible across runs.
const SEED: u64 = 1;
/// Samples per pixel the progressive render converges to before idling.
const TARGET_SPP: u32 = 256;
/// Compute workgroup tile size (must match `@workgroup_size` in pathtrace.wgsl).
const WORKGROUP: u32 = 8;

const RNG_WGSL: &str = include_str!("rng.wgsl");
const PATHTRACE_WGSL: &str = include_str!("pathtrace.wgsl");
const BLIT_WGSL: &str = include_str!("blit.wgsl");

/// App-side cache for the GPU render: the current scene, its flattened GPU
/// payload, and the progressive sample counter. Lives on `FramerApp`.
#[derive(Default)]
pub(crate) struct GpuRenderState {
    /// Key over (geometry, camera, size): a change restarts accumulation.
    accum_key: u64,
    scene: Option<Scene>,
    /// Geometry-only signature of the uploaded GPU buffers.
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
    fn sync(&mut self, model: &BuildingModel, opts: &RenderOptions, width: u32, height: u32) {
        let geom_key = model_signature(model);
        let accum_key = accumulation_key(geom_key, opts, width, height);
        if self.accum_key != accum_key || self.scene.is_none() {
            self.scene = Some(scene_from_model(model, opts));
            self.accum_key = accum_key;
            self.frame = 0;
        }
        if self.scene_key != geom_key || self.gpu_scene.is_none() {
            let gpu_scene = self.scene.as_ref().expect("scene synced").to_gpu();
            self.gpu_scene = Some(Arc::new(gpu_scene));
            self.scene_key = geom_key;
        }
    }

    /// Builds the callback for this frame and advances the sample counter while
    /// still converging. Returns `None` if there is nothing cached to render.
    fn callback(
        &mut self,
        width: u32,
        height: u32,
        srgb_target: bool,
        target_format: wgpu::TextureFormat,
    ) -> Option<PathTraceCallback> {
        let scene = self.scene.as_ref()?;
        let gpu_scene = Arc::clone(self.gpu_scene.as_ref()?);
        let frame = self.frame;
        let dispatch = frame < TARGET_SPP;

        let mut uniforms = GpuUniforms::new(scene, width, height, frame, SEED, MAX_BOUNCES);
        // The blit reads this spare lane to decide whether to apply the sRGB
        // transfer function (skipped when the surface format already encodes it).
        uniforms._pad0 = u32::from(srgb_target);

        if dispatch {
            self.frame += 1;
        }

        Some(PathTraceCallback {
            scene_key: self.scene_key,
            gpu_scene,
            uniforms,
            width,
            height,
            dispatch,
            target_format,
        })
    }
}

fn accumulation_key(geom_key: u64, opts: &RenderOptions, width: u32, height: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    geom_key.hash(&mut hasher);
    ((opts.yaw * 2000.0) as i64).hash(&mut hasher);
    ((opts.pitch * 2000.0) as i64).hash(&mut hasher);
    ((opts.zoom * 1000.0) as i64).hash(&mut hasher);
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
    target_format: wgpu::TextureFormat,
) -> bool {
    state.sync(model, opts, width, height);
    let Some(callback) = state.callback(width, height, target_format.is_srgb(), target_format)
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

        if self.dispatch {
            // Scope the compute pass so it drops before egui finishes the encoder.
            let mut cpass =
                egui_encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            cpass.set_pipeline(&res.compute_pipeline);
            cpass.set_bind_group(0, &res.scene_bg, &[]);
            cpass.set_bind_group(1, &res.frame_bg, &[]);
            cpass.dispatch_workgroups(
                self.width.div_ceil(WORKGROUP),
                self.height.div_ceil(WORKGROUP),
                1,
            );
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
        render_pass.set_pipeline(&res.blit_pipeline);
        render_pass.set_bind_group(0, &res.blit_bg, &[]);
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

        let scene_buffers = make_scene_buffers(device, gpu_scene);
        let scene_bg = make_scene_bind_group(device, &compute_pipeline, &scene_buffers);
        let accum_buf = make_accum_buffer(device, width, height);
        let frame_bg = make_frame_bind_group(device, &compute_pipeline, &uniform_buf, &accum_buf);
        let blit_bg = make_blit_bind_group(device, &blit_pipeline, &uniform_buf, &accum_buf);

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
        self.frame_bg = make_frame_bind_group(
            device,
            &self.compute_pipeline,
            &self.uniform_buf,
            &self.accum_buf,
        );
        self.blit_bg = make_blit_bind_group(
            device,
            &self.blit_pipeline,
            &self.uniform_buf,
            &self.accum_buf,
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
        ],
    })
}

fn make_blit_bind_group(
    device: &wgpu::Device,
    blit_pipeline: &wgpu::RenderPipeline,
    uniform_buf: &wgpu::Buffer,
    accum_buf: &wgpu::Buffer,
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
                resource: accum_buf.as_entire_binding(),
            },
        ],
    })
}
