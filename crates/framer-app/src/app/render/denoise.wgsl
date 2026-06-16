// Display-only edge-avoiding À-Trous wavelet denoiser (Dammertz et al. 2010).
//
// It never touches the radiance accumulator, so the converged still image and the
// GPU↔CPU parity contract are unaffected — the denoiser only feeds the blit while
// the image is still noisy (just after a camera move / low sample count), and the
// blit cross-fades its result back to the raw average as samples accumulate.
//
// Two entry points share this module:
//   * `resolve` averages the running-sum accumulator into a color buffer.
//   * `atrous`  runs one cross-bilateral wavelet pass, guided by the first-hit
//               world-normal + linear-depth gbuffer the path-trace kernel wrote,
//               ping-ponging between two color buffers with a growing tap stride.

struct DenoiseUniforms {
    width: u32,
    height: u32,
    // Tap stride for this À-Trous level (1, 2, 4, 8, 16). Unused by `resolve`.
    step: u32,
    pad: u32,
};

// Edge-stopping tuning. Normal: cosine raised to a high power (sharp). Depth:
// relative tolerance, widened with the tap stride (coarser levels span more).
// Luminance: tightened with the stride (Dammertz's per-level c_phi relaxation),
// so fine levels smooth freely and coarse levels respect lighting discontinuities.
const SIGMA_N: f32 = 64.0;
const SIGMA_Z: f32 = 0.2;
const SIGMA_L: f32 = 4.0;

// B3-spline kernel weights by |offset|: {0: 3/8, 1: 1/4, 2: 1/16}.
fn k_weight(d: i32) -> f32 {
    let a = abs(d);
    if (a == 0) { return 0.375; }
    if (a == 1) { return 0.25; }
    return 0.0625;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// ---- resolve: accumulator running-sum → averaged color ----------------------

@group(0) @binding(0) var<uniform> ru: DenoiseUniforms;
@group(0) @binding(1) var<storage, read> r_accum: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> r_out: array<vec4<f32>>;

@compute @workgroup_size(8, 8, 1)
fn resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= ru.width || gid.y >= ru.height) {
        return;
    }
    let idx = gid.y * ru.width + gid.x;
    let s = r_accum[idx];
    r_out[idx] = vec4<f32>(s.xyz / max(s.w, 1.0), 1.0);
}

// ---- atrous: one edge-avoiding wavelet pass ---------------------------------

@group(0) @binding(0) var<uniform> au: DenoiseUniforms;
@group(0) @binding(1) var<storage, read> a_in: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> a_gbuf: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> a_out: array<vec4<f32>>;

@compute @workgroup_size(8, 8, 1)
fn atrous(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= au.width || gid.y >= au.height) {
        return;
    }
    let w = i32(au.width);
    let h = i32(au.height);
    let step = i32(au.step);
    let cx = i32(gid.x);
    let cy = i32(gid.y);
    let idx = gid.y * au.width + gid.x;

    let c0 = a_in[idx].xyz;
    let g0 = a_gbuf[idx];
    let n0 = g0.xyz;
    let z0 = g0.w;
    let l0 = luma(c0);

    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var dy = -2; dy <= 2; dy = dy + 1) {
        for (var dx = -2; dx <= 2; dx = dx + 1) {
            let qx = cx + dx * step;
            let qy = cy + dy * step;
            if (qx < 0 || qy < 0 || qx >= w || qy >= h) {
                continue;
            }
            let qidx = u32(qy) * au.width + u32(qx);
            let cq = a_in[qidx].xyz;
            let gq = a_gbuf[qidx];

            let wn = pow(max(dot(n0, gq.xyz), 0.0), SIGMA_N);
            let wz = exp(-abs(z0 - gq.w) / (SIGMA_Z * max(abs(z0), 1.0) * f32(step) + 1.0e-4));
            let wl = exp(-abs(l0 - luma(cq)) * f32(step) / SIGMA_L);
            let weight = k_weight(dx) * k_weight(dy) * wn * wz * wl;

            sum = sum + cq * weight;
            wsum = wsum + weight;
        }
    }

    // wsum > 0 always for valid geometry (the center tap self-weights to 1);
    // background/sky pixels (zero normal) fall through to their raw value.
    let filtered = select(c0, sum / wsum, wsum > 0.0);
    a_out[idx] = vec4<f32>(filtered, 1.0);
}
