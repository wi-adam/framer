//! Headless GPU validation for the WGSL compute path tracer.
//!
//! These tests stand up a real `wgpu` device (no window/surface needed), run the
//! shaders that the in-app Render view uses, read the results back, and compare
//! them to `framer-render`'s CPU reference. They are the primary correctness
//! mechanism for the GPU kernel — the `egui_wgpu` wiring can only be eyeballed in
//! the running app, but the math is pinned here.
//!
//! If no GPU adapter is available (headless CI), the tests **skip** rather than
//! fail.

use eframe::wgpu;
use pollster::block_on;
use wgpu::util::DeviceExt as _;

const RNG_WGSL: &str = include_str!("../src/app/render/rng.wgsl");
const RNG_DEBUG_WGSL: &str = include_str!("../src/app/render/rng_debug.wgsl");
const PATHTRACE_WGSL: &str = include_str!("../src/app/render/pathtrace.wgsl");
const BLIT_WGSL: &str = include_str!("../src/app/render/blit.wgsl");

/// Requests a headless device + queue, or `None` if no adapter is available.
fn device_queue() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .ok()?;
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("framer-gpu-parity-test"),
        ..Default::default()
    }))
    .ok()?;
    Some((device, queue))
}

/// Maps `staging` (a `MAP_READ | COPY_DST` buffer) and returns its bytes.
fn read_back(device: &wgpu::Device, staging: &wgpu::Buffer) -> Vec<u8> {
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    rx.recv().expect("map channel").expect("map failed");
    let data = slice.get_mapped_range();
    let bytes = data.to_vec();
    drop(data);
    staging.unmap();
    bytes
}

#[test]
fn wgsl_rng_matches_cpu_pcg() {
    let Some((device, queue)) = device_queue() else {
        eprintln!("no GPU adapter available; skipping wgsl_rng_matches_cpu_pcg");
        return;
    };

    const N: usize = 16;
    let byte_size = (N * std::mem::size_of::<u32>()) as u64;

    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rng_out"),
        size: byte_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rng_staging"),
        size: byte_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let source = format!("{RNG_WGSL}\n{RNG_DEBUG_WGSL}");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("rng_debug"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("rng_debug_pipeline"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rng_bg"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: out.as_entire_binding(),
        }],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rng_encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&out, 0, &staging, 0, byte_size);
    queue.submit(Some(encoder.finish()));

    let bytes = read_back(&device, &staging);
    let gpu: &[u32] = bytemuck::cast_slice(&bytes);

    // Mirror the exact battery in rng_debug.wgsl on the CPU.
    use framer_render::rng::{Pcg32, pixel_rng};
    let mut expected = [0u32; N];
    let mut canary = Pcg32::seed(42, 54);
    for slot in expected.iter_mut().take(6) {
        *slot = canary.next_u32();
    }
    let mut a = pixel_rng(10, 20, 3, 0xDEAD_BEEF);
    for slot in expected.iter_mut().skip(6).take(4) {
        *slot = a.next_u32();
    }
    let mut b = pixel_rng(0, 0, 0, 1);
    for slot in expected.iter_mut().skip(10).take(4) {
        *slot = b.next_u32();
    }
    let mut c = pixel_rng(63, 47, 11, 0x1234_5678_9ABC_DEF0);
    expected[14] = c.next_u32();
    expected[15] = c.next_u32();

    // The first six are the canonical pcg_basic canary — anchors correctness.
    assert_eq!(expected[0], 0xa15c_02b7, "CPU canary drifted");
    assert_eq!(
        gpu, &expected,
        "GPU PCG output diverged from CPU\n gpu={gpu:08x?}\n cpu={expected:08x?}"
    );
}

// Higher than the golden's spp: the GPU and CPU are independently-rounded f32
// tracers, so a few pixels at the glass/sun-disk silhouette flip a branch on
// last-ULP differences. More samples average those rare divergences down, which
// lets us keep a tight max-error bound (a real bug shows up in the MAE instead).
const PARITY_SPP: u32 = 64;

/// Runs the WGSL compute path tracer for `spp` samples of `scene` and returns
/// the running-sum accumulator buffer (`STORAGE | COPY_SRC`, `vec4<f32>` per
/// pixel). This is the exact kernel the in-app Render view dispatches.
fn accumulate_on_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &framer_render::scene::Scene,
    width: u32,
    height: u32,
    spp: u32,
    seed: u64,
) -> wgpu::Buffer {
    use framer_render::MAX_BOUNCES;
    use framer_render::gpu::GpuUniforms;

    let gpu = scene.to_gpu();
    let storage = |label, bytes: &[u8]| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        })
    };
    let tri_buf = storage("triangles", bytemuck::cast_slice(&gpu.triangles));
    let node_buf = storage("nodes", bytemuck::cast_slice(&gpu.nodes));
    let idx_buf = storage("indices", bytemuck::cast_slice(&gpu.indices));
    let mat_buf = storage("materials", bytemuck::cast_slice(&gpu.materials));

    let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("uniforms"),
        size: std::mem::size_of::<GpuUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let accum_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("accum"),
        size: (width as u64) * (height as u64) * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let source = format!("{RNG_WGSL}\n{PATHTRACE_WGSL}");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pathtrace"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("pathtrace_pipeline"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene_bg"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: tri_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: node_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: idx_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: mat_buf.as_entire_binding(),
            },
        ],
    });
    let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame_bg"),
        layout: &pipeline.get_bind_group_layout(1),
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
    });

    // Progressive accumulation: one dispatch per sample, frame index = sample.
    for frame in 0..spp {
        let uniforms = GpuUniforms::new(scene, width, height, frame, seed, MAX_BOUNCES);
        queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&uniforms));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pathtrace_encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &scene_bg, &[]);
            pass.set_bind_group(1, &frame_bg, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        queue.submit(Some(encoder.finish()));
    }

    accum_buf
}

#[test]
fn wgsl_pathtracer_matches_cpu_reference() {
    use framer_render::math::Vec3;
    use framer_render::scenes::{
        REFERENCE_HEIGHT as H, REFERENCE_SEED as SEED, REFERENCE_WIDTH as W, reference_scene,
    };
    use framer_render::{render, tonemap_accum};

    let Some((device, queue)) = device_queue() else {
        eprintln!("no GPU adapter available; skipping wgsl_pathtracer_matches_cpu_reference");
        return;
    };

    let scene = reference_scene();
    let accum_bytes = (W * H) as u64 * 16;
    let accum_buf = accumulate_on_gpu(&device, &queue, &scene, W, H, PARITY_SPP, SEED);

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("accum_staging"),
        size: accum_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("copy_encoder"),
    });
    encoder.copy_buffer_to_buffer(&accum_buf, 0, &staging, 0, accum_bytes);
    queue.submit(Some(encoder.finish()));
    let bytes = read_back(&device, &staging);
    let sums: &[f32] = bytemuck::cast_slice(&bytes);

    // Tone-map the GPU running-sum exactly as the CPU does, then compare bytes.
    let accum: Vec<Vec3> = (0..(W * H) as usize)
        .map(|i| Vec3::new(sums[i * 4], sums[i * 4 + 1], sums[i * 4 + 2]))
        .collect();
    // Every pixel must have accumulated exactly PARITY_SPP samples.
    for i in 0..(W * H) as usize {
        assert_eq!(
            sums[i * 4 + 3],
            PARITY_SPP as f32,
            "pixel {i} sample count wrong"
        );
    }
    let gpu_rgba = tonemap_accum(&accum, PARITY_SPP, scene.exposure);
    let cpu_rgba = render(&scene, W, H, PARITY_SPP, SEED);
    assert_eq!(gpu_rgba.len(), cpu_rgba.len());

    let (mae, max) = image_error(&gpu_rgba, &cpu_rgba);
    eprintln!("GPU↔CPU kernel parity: MAE={mae:.3}, max={max}");
    // MAE is the real correctness gate — a genuine math/traversal bug would push
    // it into the tens, but the bit-exact RNG + mirrored math keep it ~0.03. The
    // max bound guards against a single blown pixel (NaN, wrong branch) with
    // headroom for cross-vendor f32 rounding at the glass/sun-disk silhouette.
    assert!(
        mae < 2.0,
        "GPU↔CPU mean abs error {mae} too high (kernel bug?)"
    );
    assert!(max < 48, "GPU↔CPU max pixel error {max} too high");
}

/// End-to-end display-path test: accumulates on the GPU, then runs the *actual*
/// blit shader (fullscreen triangle + ACES + sRGB) into an offscreen target and
/// compares the rendered bytes to the CPU reference. This validates the parts the
/// kernel test skips — Y-orientation, tone-map, and gamma — which otherwise can
/// only be eyeballed in the running app (and macOS blocks screen capture).
#[test]
fn wgsl_blit_matches_cpu_reference() {
    use framer_render::gpu::GpuUniforms;
    use framer_render::scenes::{
        REFERENCE_HEIGHT as H, REFERENCE_SEED as SEED, REFERENCE_WIDTH as W, reference_scene,
    };
    use framer_render::{MAX_BOUNCES, render};

    let Some((device, queue)) = device_queue() else {
        eprintln!("no GPU adapter available; skipping wgsl_blit_matches_cpu_reference");
        return;
    };
    // W * 4 bytes/row must be 256-aligned for texture→buffer copy (64*4 == 256).
    assert_eq!((W * 4) % 256, 0, "test width must give 256-aligned rows");

    let scene = reference_scene();
    let accum_buf = accumulate_on_gpu(&device, &queue, &scene, W, H, PARITY_SPP, SEED);

    // Non-sRGB target + srgb_target=0 so the blit applies the sRGB OETF itself,
    // producing bytes directly comparable to the CPU render's encoded output.
    let mut uniforms = GpuUniforms::new(&scene, W, H, 0, SEED, MAX_BOUNCES);
    uniforms._pad0 = 0;
    let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("blit_uniforms"),
        contents: bytemuck::bytes_of(&uniforms),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit"),
        source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
    });
    let color_format = wgpu::TextureFormat::Rgba8Unorm;
    let depth_format = wgpu::TextureFormat::Depth24Plus;
    let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blit_pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &blit_module,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        // Mirror the in-app pipeline: egui's pass carries a depth attachment.
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
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
                format: color_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    });
    let blit_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("blit_bg"),
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
    });

    let extent = wgpu::Extent3d {
        width: W,
        height: H,
        depth_or_array_layers: 1,
    };
    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blit_color"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: color_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blit_depth"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: depth_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blit_staging"),
        size: (W * H * 4) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("blit_encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&blit_pipeline);
        pass.set_bind_group(0, &blit_bg, &[]);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: Some(H),
            },
        },
        extent,
    );
    queue.submit(Some(encoder.finish()));

    let gpu_rgba = read_back(&device, &staging);
    let cpu_rgba = render(&scene, W, H, PARITY_SPP, SEED);
    assert_eq!(gpu_rgba.len(), cpu_rgba.len());

    let (mae, max) = image_error(&gpu_rgba, &cpu_rgba);
    eprintln!("GPU↔CPU blit parity: MAE={mae:.3}, max={max}");
    assert!(
        mae < 2.0,
        "blit mean abs error {mae} too high (orientation/tonemap bug?)"
    );
    assert!(max < 48, "blit max pixel error {max} too high");
}

/// Mean-absolute and max per-byte error between two equal-length RGBA buffers.
fn image_error(a: &[u8], b: &[u8]) -> (f64, u32) {
    let mut total = 0u64;
    let mut max = 0u32;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = (*x as i32 - *y as i32).unsigned_abs();
        total += d as u64;
        max = max.max(d);
    }
    (total as f64 / a.len() as f64, max)
}
