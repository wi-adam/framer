// Tone-map blit: a fullscreen triangle (no vertex buffer) that averages the
// running-sum accumulator and applies exposure + Narkowicz ACES + sRGB, mirroring
// framer_render::color. egui_wgpu sets the render-pass viewport to the callback's
// rect, so the interpolated UV spans 0..1 across the rect regardless of where it
// sits in the window; we flip uv.y because accumulator row 0 is the top row.
//
// `srgb_target` (packed into the uniforms' spare lane by the app) is 1 when the
// surface format is sRGB — then the surface encodes gamma on write and we output
// the linear ACES result; otherwise we apply the sRGB transfer function here.

struct Uniforms {
    cam_eye: vec3<f32>,
    half_w: f32,
    cam_forward: vec3<f32>,
    half_h: f32,
    cam_right: vec3<f32>,
    pad_r: f32,
    cam_up: vec3<f32>,
    pad_u: f32,
    sun_dir: vec3<f32>,
    sun_cos_angular: f32,
    sun_irradiance: vec3<f32>,
    sun_solid_angle: f32,
    sky_zenith: vec3<f32>,
    exposure: f32,
    sky_horizon: vec3<f32>,
    pad_h: f32,
    sky_ground: vec3<f32>,
    pad_g: f32,
    width: u32,
    height: u32,
    frame: u32,
    seed_lo: u32,
    seed_hi: u32,
    max_bounces: u32,
    srgb_target: u32,
    spp: u32,
    denoise: u32,
    denoise_strength: f32,
    pad_d0: u32,
    pad_d1: u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// Display buffer: either the À-Trous denoised color (stored averaged, .w = 1) or,
// when the denoiser is off, the raw accumulator (bound to both 1 and 2).
@group(0) @binding(1) var<storage, read> accum: array<vec4<f32>>;
// Raw running-sum accumulator, for the denoise→raw cross-fade.
@group(0) @binding(2) var<storage, read> raw_accum: array<vec4<f32>>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    out.uv = uv;
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp(
        (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}

fn linear_to_srgb_c(c: f32) -> f32 {
    if (c <= 0.0031308) {
        return 12.92 * c;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let px = min(u32(in.uv.x * f32(u.width)), u.width - 1u);
    // Flip Y: accumulator row 0 is the top of the image; viewport uv.y=1 is top.
    let py = min(u32((1.0 - in.uv.y) * f32(u.height)), u.height - 1u);
    let di = py * u.width + px;
    let disp = accum[di];
    let display_lin = disp.xyz / max(disp.w, 1.0);
    // Cross-fade the denoised display buffer toward the raw average; strength 0
    // shows the unbiased path-traced result (the two bindings alias when off).
    let raw = raw_accum[di];
    let raw_lin = raw.xyz / max(raw.w, 1.0);
    let lin = mix(raw_lin, display_lin, u.denoise_strength);
    let hdr = lin * u.exposure;
    let mapped = aces(hdr);
    if (u.srgb_target != 0u) {
        return vec4<f32>(mapped, 1.0);
    }
    return vec4<f32>(
        linear_to_srgb_c(mapped.x),
        linear_to_srgb_c(mapped.y),
        linear_to_srgb_c(mapped.z),
        1.0,
    );
}
