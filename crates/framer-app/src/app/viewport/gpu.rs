//! GPU 3D pipeline for the Axonometric and view-cube renderers: the WGSL shader,
//! vertex/uniform layout, the `egui_wgpu` callback bridge, per-frame GPU resource
//! store, and pipeline creation. No `FramerApp` coupling.

use std::borrow::Cow;
use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use eframe::egui::{self, Rect};
use eframe::{egui_wgpu, wgpu};
use wgpu::util::DeviceExt as _;

use super::geom::OrbitProjector;

// === extracted block appended below; visibility adjusted in place ===

const FRAMER_3D_SHADER: &str = r#"
struct Uniforms {
    center: vec4<f32>,
    right: vec4<f32>,
    depth_axis: vec4<f32>,
    raw_center_scale: vec4<f32>,
    depth_center_scale: vec4<f32>,
};

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) normal: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let local = input.position - uniforms.center.xyz;
    let raw_x = dot(local.xy, uniforms.right.xy);
    let along_depth = dot(local.xy, uniforms.depth_axis.xy);
    let raw_y = along_depth * uniforms.depth_axis.z - local.z * uniforms.depth_axis.w;
    let depth = along_depth * uniforms.depth_axis.w + local.z * uniforms.depth_axis.z;

    var output: VertexOutput;
    output.position = vec4<f32>(
        (raw_x - uniforms.raw_center_scale.x) * uniforms.raw_center_scale.z,
        -(raw_y - uniforms.raw_center_scale.y) * uniforms.raw_center_scale.w,
        clamp(uniforms.depth_center_scale.z - (depth - uniforms.depth_center_scale.x) * uniforms.depth_center_scale.y, 0.0, 1.0),
        1.0,
    );
    output.color = input.color;
    output.normal = input.normal;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(-0.35, -0.45, 0.82));
    let shade = 0.50 + 0.50 * max(dot(normalize(input.normal), light), 0.0);
    return vec4<f32>(input.color.rgb * shade, input.color.a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct GpuVertex {
    pub(super) position: [f32; 3],
    pub(super) color: [f32; 4],
    pub(super) normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct GpuUniforms {
    center: [f32; 4],
    right: [f32; 4],
    depth_axis: [f32; 4],
    raw_center_scale: [f32; 4],
    depth_center_scale: [f32; 4],
}

impl GpuUniforms {
    pub(super) fn from_projector(projector: &OrbitProjector, drawing: Rect) -> Self {
        let scale_x = projector.scale / (drawing.width() / 2.0).max(1.0);
        let scale_y = projector.scale / (drawing.height() / 2.0).max(1.0);
        Self {
            center: [
                projector.center.x,
                projector.center.y,
                projector.center.z,
                0.0,
            ],
            right: [projector.right.x, projector.right.y, 0.0, 0.0],
            depth_axis: [
                projector.depth_axis.x,
                projector.depth_axis.y,
                projector.pitch.sin(),
                projector.pitch.cos(),
            ],
            raw_center_scale: [
                projector.raw_center.x,
                projector.raw_center.y,
                scale_x,
                scale_y,
            ],
            depth_center_scale: [projector.depth_center, projector.depth_scale, 0.5, 0.0],
        }
    }

    pub(super) fn from_projector_with_depth_base(
        projector: &OrbitProjector,
        drawing: Rect,
        depth_base: f32,
    ) -> Self {
        let mut uniforms = Self::from_projector(projector, drawing);
        uniforms.depth_center_scale[2] = depth_base;
        uniforms
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Framer3dFrameKey(u64);

impl Framer3dFrameKey {
    pub(super) const MODEL: Self = Self(1);
    pub(super) const VIEW_CUBE: Self = Self(2);
}

pub(super) struct Framer3dCallback {
    pub(super) frame_key: Framer3dFrameKey,
    pub(super) vertices: Vec<GpuVertex>,
    pub(super) indices: Vec<u32>,
    pub(super) opaque_index_count: u32,
    pub(super) transparent_index_count: u32,
    pub(super) uniforms: GpuUniforms,
    pub(super) target_format: wgpu::TextureFormat,
    pub(super) depth_format: Option<wgpu::TextureFormat>,
}

struct Framer3dResources {
    target_format: wgpu::TextureFormat,
    depth_format: Option<wgpu::TextureFormat>,
    bind_group_layout: wgpu::BindGroupLayout,
    opaque_pipeline: wgpu::RenderPipeline,
    transparent_pipeline: wgpu::RenderPipeline,
}

struct Framer3dFrame {
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    opaque_index_count: u32,
    transparent_index_count: u32,
}

#[derive(Default)]
struct Framer3dFrameStore {
    frames: HashMap<Framer3dFrameKey, Framer3dFrame>,
}

impl egui_wgpu::CallbackTrait for Framer3dCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let needs_resources =
            callback_resources
                .get::<Framer3dResources>()
                .is_none_or(|resources| {
                    resources.target_format != self.target_format
                        || resources.depth_format != self.depth_format
                });
        if needs_resources {
            callback_resources.insert(Framer3dResources::new(
                device,
                self.target_format,
                self.depth_format,
            ));
        }
        let resources = callback_resources
            .get::<Framer3dResources>()
            .expect("3D render resources should exist");

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("framer_3d_vertices"),
            contents: bytemuck::cast_slice(&self.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("framer_3d_indices"),
            contents: bytemuck::cast_slice(&self.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("framer_3d_uniforms"),
            contents: bytemuck::bytes_of(&self.uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("framer_3d_bind_group"),
            layout: &resources.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        if callback_resources.get::<Framer3dFrameStore>().is_none() {
            callback_resources.insert(Framer3dFrameStore::default());
        }
        let frame_store = callback_resources
            .get_mut::<Framer3dFrameStore>()
            .expect("3D frame store should exist");
        frame_store.frames.insert(
            self.frame_key,
            Framer3dFrame {
                bind_group,
                vertex_buffer,
                index_buffer,
                opaque_index_count: self.opaque_index_count,
                transparent_index_count: self.transparent_index_count,
            },
        );

        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<Framer3dResources>() else {
            return;
        };
        let Some(frame) = callback_resources
            .get::<Framer3dFrameStore>()
            .and_then(|store| store.frames.get(&self.frame_key))
        else {
            return;
        };

        render_pass.set_bind_group(0, &frame.bind_group, &[]);
        render_pass.set_vertex_buffer(0, frame.vertex_buffer.slice(..));
        render_pass.set_index_buffer(frame.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        if frame.opaque_index_count > 0 {
            render_pass.set_pipeline(&resources.opaque_pipeline);
            render_pass.draw_indexed(0..frame.opaque_index_count, 0, 0..1);
        }

        if frame.transparent_index_count > 0 {
            let start = frame.opaque_index_count;
            let end = start + frame.transparent_index_count;
            render_pass.set_pipeline(&resources.transparent_pipeline);
            render_pass.draw_indexed(start..end, 0, 0..1);
        }
    }
}

impl Framer3dResources {
    fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        depth_format: Option<wgpu::TextureFormat>,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("framer_3d_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("framer_3d_shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(FRAMER_3D_SHADER)),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("framer_3d_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let opaque_pipeline = create_3d_pipeline(
            device,
            &pipeline_layout,
            &shader,
            target_format,
            depth_format,
            None,
            true,
        );
        let transparent_pipeline = create_3d_pipeline(
            device,
            &pipeline_layout,
            &shader,
            target_format,
            depth_format,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
        );

        Self {
            target_format,
            depth_format,
            bind_group_layout,
            opaque_pipeline,
            transparent_pipeline,
        }
    }
}

fn create_3d_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    depth_format: Option<wgpu::TextureFormat>,
    blend: Option<wgpu::BlendState>,
    depth_write_enabled: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("framer_3d_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GpuVertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x4,
                    2 => Float32x3
                ],
            }],
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
        depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(depth_write_enabled),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}
