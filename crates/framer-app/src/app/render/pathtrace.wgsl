// WGSL compute path tracer. Concatenated after rng.wgsl, it mirrors the math of
// framer_render's CPU tracer exactly — same PCG draw order, same BVH traversal,
// same diffuse / metal-GGX / dielectric BSDFs, same NEE sun + procedural sky,
// Russian roulette and firefly clamp. The headless parity test validates that
// this kernel reproduces `framer_render::render` within a tight tolerance.
//
// One invocation = one pixel, one sample (sample index = u.frame). Results are
// accumulated as a running sum in `accum` (.xyz = radiance sum, .w = sample
// count); the blit pass averages and tone-maps. When u.frame == 0 the previous
// contents are discarded (camera/scene changed), otherwise they are added to.

struct Triangle {
    v0: vec3<f32>,
    material: u32,
    edge1: vec3<f32>,
    pad1: f32,
    edge2: vec3<f32>,
    pad2: f32,
    normal: vec3<f32>,
    pad3: f32,
};

struct BvhNode {
    aabb_min: vec3<f32>,
    left_first: u32,
    aabb_max: vec3<f32>,
    count: u32,
};

struct Material {
    color: vec3<f32>,
    kind: u32,
    tint: vec3<f32>,
    param: f32,
};

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
    pad0: u32,
    spp: u32,
};

@group(0) @binding(0) var<storage, read> triangles: array<Triangle>;
@group(0) @binding(1) var<storage, read> nodes: array<BvhNode>;
@group(0) @binding(2) var<storage, read> tri_indices: array<u32>;
@group(0) @binding(3) var<storage, read> materials: array<Material>;
@group(1) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(1) var<storage, read_write> accum: array<vec4<f32>>;

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;

const MAT_DIFFUSE: u32 = 0u;
const MAT_METAL: u32 = 1u;
const MAT_DIELECTRIC: u32 = 2u;
const MAT_EMISSIVE: u32 = 3u;

const PARALLEL_EPS: f32 = 1.0e-8;
const RAY_EPSILON: f32 = 1.0e-3;
const ORIGIN_OFFSET: f32 = 1.0e-2;
const MIN_BOUNCES: u32 = 3u;
const FIREFLY_CLAMP: f32 = 50.0;
const T_FAR: f32 = 1.0e30;
const STACK_SIZE: u32 = 64u;

fn max3(v: vec3<f32>) -> f32 {
    return max(max(v.x, v.y), v.z);
}

// ---- Rays & hits ------------------------------------------------------------

struct Ray {
    origin: vec3<f32>,
    dir: vec3<f32>,
    inv_dir: vec3<f32>,
    t_min: f32,
    t_max: f32,
};

fn make_ray(origin: vec3<f32>, dir: vec3<f32>, t_min: f32, t_max: f32) -> Ray {
    return Ray(origin, dir, vec3<f32>(1.0) / dir, t_min, t_max);
}

struct Hit {
    t: f32,
    point: vec3<f32>,
    normal: vec3<f32>,
    geom_normal: vec3<f32>,
    front_face: bool,
    material: u32,
    valid: bool,
};

// ---- Orthonormal basis (Duff, branchless) -----------------------------------

struct Onb {
    t: vec3<f32>,
    b: vec3<f32>,
    n: vec3<f32>,
};

fn onb_from_normal(n: vec3<f32>) -> Onb {
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    let t = vec3<f32>(1.0 + s * n.x * n.x * a, s * b, -s * n.x);
    let bt = vec3<f32>(b, s + n.y * n.y * a, -n.y);
    return Onb(t, bt, n);
}

fn onb_to_world(o: Onb, local: vec3<f32>) -> vec3<f32> {
    return o.t * local.x + o.b * local.y + o.n * local.z;
}

fn onb_to_local(o: Onb, w: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(w, o.t), dot(w, o.b), dot(w, o.n));
}

// ---- Geometry intersection (Möller–Trumbore, no backface cull) --------------

fn aabb_hit(lo: vec3<f32>, hi: vec3<f32>, ray: Ray, t_max: f32) -> bool {
    let t0 = (lo - ray.origin) * ray.inv_dir;
    let t1 = (hi - ray.origin) * ray.inv_dir;
    let tmin = min(t0, t1);
    let tmax = max(t0, t1);
    let enter = max(max(max(tmin.x, tmin.y), tmin.z), ray.t_min);
    let exit = min(min(min(tmax.x, tmax.y), tmax.z), t_max);
    return enter <= exit;
}

// Updates `hit` with the nearest intersection in [ray.t_min, hit.t).
fn intersect_tri(ray: Ray, ti: u32, hit: ptr<function, Hit>) {
    let tri = triangles[ti];
    let h = cross(ray.dir, tri.edge2);
    let det = dot(tri.edge1, h);
    if (abs(det) < PARALLEL_EPS) {
        return;
    }
    let inv_det = 1.0 / det;
    let s = ray.origin - tri.v0;
    let uu = inv_det * dot(s, h);
    if (uu < 0.0 || uu > 1.0) {
        return;
    }
    let q = cross(s, tri.edge1);
    let vv = inv_det * dot(ray.dir, q);
    if (vv < 0.0 || uu + vv > 1.0) {
        return;
    }
    let t = inv_det * dot(tri.edge2, q);
    if (t < ray.t_min || t >= (*hit).t) {
        return;
    }
    (*hit).t = t;
    (*hit).point = ray.origin + ray.dir * t;
    let front = dot(ray.dir, tri.normal) < 0.0;
    (*hit).front_face = front;
    (*hit).normal = select(-tri.normal, tri.normal, front);
    (*hit).geom_normal = tri.normal;
    (*hit).material = tri.material;
    (*hit).valid = true;
}

// True if any triangle blocks the ray within [t_min, t_max].
fn intersect_tri_any(ray: Ray, ti: u32) -> bool {
    let tri = triangles[ti];
    let h = cross(ray.dir, tri.edge2);
    let det = dot(tri.edge1, h);
    if (abs(det) < PARALLEL_EPS) {
        return false;
    }
    let inv_det = 1.0 / det;
    let s = ray.origin - tri.v0;
    let uu = inv_det * dot(s, h);
    if (uu < 0.0 || uu > 1.0) {
        return false;
    }
    let q = cross(s, tri.edge1);
    let vv = inv_det * dot(ray.dir, q);
    if (vv < 0.0 || uu + vv > 1.0) {
        return false;
    }
    let t = inv_det * dot(tri.edge2, q);
    return t >= ray.t_min && t <= ray.t_max;
}

fn intersect_scene(ray: Ray) -> Hit {
    var hit: Hit;
    hit.t = ray.t_max;
    hit.valid = false;
    if (arrayLength(&nodes) == 0u) {
        return hit;
    }
    var stack: array<u32, STACK_SIZE>;
    var sp = 0u;
    stack[0] = 0u;
    sp = 1u;
    loop {
        if (sp == 0u) {
            break;
        }
        sp = sp - 1u;
        let node = nodes[stack[sp]];
        if (!aabb_hit(node.aabb_min, node.aabb_max, ray, hit.t)) {
            continue;
        }
        if (node.count > 0u) {
            for (var k = 0u; k < node.count; k = k + 1u) {
                intersect_tri(ray, tri_indices[node.left_first + k], &hit);
            }
        } else {
            stack[sp] = node.left_first;
            sp = sp + 1u;
            stack[sp] = node.left_first + 1u;
            sp = sp + 1u;
        }
    }
    return hit;
}

fn occluded(ray: Ray) -> bool {
    if (arrayLength(&nodes) == 0u) {
        return false;
    }
    var stack: array<u32, STACK_SIZE>;
    var sp = 0u;
    stack[0] = 0u;
    sp = 1u;
    loop {
        if (sp == 0u) {
            break;
        }
        sp = sp - 1u;
        let node = nodes[stack[sp]];
        if (!aabb_hit(node.aabb_min, node.aabb_max, ray, ray.t_max)) {
            continue;
        }
        if (node.count > 0u) {
            for (var k = 0u; k < node.count; k = k + 1u) {
                if (intersect_tri_any(ray, tri_indices[node.left_first + k])) {
                    return true;
                }
            }
        } else {
            stack[sp] = node.left_first;
            sp = sp + 1u;
            stack[sp] = node.left_first + 1u;
            sp = sp + 1u;
        }
    }
    return false;
}

// ---- Sampling ---------------------------------------------------------------

fn cosine_sample_hemisphere(rng: ptr<function, Rng>) -> vec3<f32> {
    let r1 = pcg_next_f32(rng);
    let r2 = pcg_next_f32(rng);
    let phi = TAU * r1;
    let r = sqrt(r2);
    return vec3<f32>(r * cos(phi), r * sin(phi), sqrt(max(1.0 - r2, 0.0)));
}

fn sample_cone(axis: vec3<f32>, cos_max: f32, rng: ptr<function, Rng>) -> vec3<f32> {
    let u1 = pcg_next_f32(rng);
    let u2 = pcg_next_f32(rng);
    let cos_t = 1.0 - u1 * (1.0 - cos_max);
    let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
    let phi = TAU * u2;
    let local = vec3<f32>(sin_t * cos(phi), sin_t * sin(phi), cos_t);
    return normalize(onb_to_world(onb_from_normal(axis), local));
}

fn sample_ggx_vndf(ve: vec3<f32>, alpha: f32, rng: ptr<function, Rng>) -> vec3<f32> {
    let vh = normalize(vec3<f32>(alpha * ve.x, alpha * ve.y, ve.z));
    let lensq = vh.x * vh.x + vh.y * vh.y;
    var t1: vec3<f32>;
    if (lensq > 0.0) {
        t1 = vec3<f32>(-vh.y, vh.x, 0.0) * (1.0 / sqrt(lensq));
    } else {
        t1 = vec3<f32>(1.0, 0.0, 0.0);
    }
    let t2 = cross(vh, t1);
    let u1 = pcg_next_f32(rng);
    let u2 = pcg_next_f32(rng);
    let r = sqrt(u1);
    let phi = TAU * u2;
    let p_x = r * cos(phi);
    var p_y = r * sin(phi);
    let s = 0.5 * (1.0 + vh.z);
    p_y = (1.0 - s) * sqrt(max(1.0 - p_x * p_x, 0.0)) + s * p_y;
    let p_z = sqrt(max(1.0 - p_x * p_x - p_y * p_y, 0.0));
    let nh = t1 * p_x + t2 * p_y + vh * p_z;
    return normalize(vec3<f32>(alpha * nh.x, alpha * nh.y, max(nh.z, 0.0)));
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    let m = clamp(1.0 - cos_theta, 0.0, 1.0);
    let m5 = m * m * m * m * m;
    return f0 + (vec3<f32>(1.0) - f0) * m5;
}

fn fresnel_dielectric(cos_i_in: f32, ior: f32) -> f32 {
    var cos_i = clamp(cos_i_in, -1.0, 1.0);
    var eta_i = 1.0;
    var eta_t = ior;
    if (cos_i <= 0.0) {
        eta_i = ior;
        eta_t = 1.0;
    }
    cos_i = abs(cos_i);
    let sin_t = eta_i / eta_t * sqrt(max(1.0 - cos_i * cos_i, 0.0));
    if (sin_t >= 1.0) {
        return 1.0;
    }
    let cos_t = sqrt(max(1.0 - sin_t * sin_t, 0.0));
    let r_parl = (eta_t * cos_i - eta_i * cos_t) / (eta_t * cos_i + eta_i * cos_t);
    let r_perp = (eta_i * cos_i - eta_t * cos_t) / (eta_i * cos_i + eta_t * cos_t);
    return 0.5 * (r_parl * r_parl + r_perp * r_perp);
}

fn smith_lambda(nox: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let c2 = max(nox * nox, 1.0e-7);
    let tan2 = (1.0 - c2) / c2;
    return 0.5 * (-1.0 + sqrt(1.0 + a2 * tan2));
}

fn smith_g1(nov: f32, alpha: f32) -> f32 {
    return 1.0 / (1.0 + smith_lambda(nov, alpha));
}

fn smith_g2(nov: f32, nol: f32, alpha: f32) -> f32 {
    return 1.0 / (1.0 + smith_lambda(nov, alpha) + smith_lambda(nol, alpha));
}

struct Refract {
    dir: vec3<f32>,
    ok: bool,
};

// Vector-form refraction (RTIOW). eta = n_from / n_into. ok=false on TIR.
fn refract_dir(uv: vec3<f32>, n: vec3<f32>, eta: f32) -> Refract {
    let cos_theta = min(dot(-uv, n), 1.0);
    let r_out_perp = (uv + n * cos_theta) * eta;
    let perp_len_sq = dot(r_out_perp, r_out_perp);
    if (perp_len_sq > 1.0) {
        return Refract(vec3<f32>(0.0), false);
    }
    let r_out_parallel = n * -sqrt(abs(1.0 - perp_len_sq));
    return Refract(r_out_perp + r_out_parallel, true);
}

// ---- BSDF scatter -----------------------------------------------------------

struct Scatter {
    dir: vec3<f32>,
    throughput: vec3<f32>,
    specular: bool,
    ok: bool,
};

fn scatter(wo: vec3<f32>, hit: Hit, mat: Material, rng: ptr<function, Rng>) -> Scatter {
    if (mat.kind == MAT_DIFFUSE) {
        let onb = onb_from_normal(hit.normal);
        let local = cosine_sample_hemisphere(rng);
        let dir = normalize(onb_to_world(onb, local));
        return Scatter(dir, mat.color, false, true);
    }
    if (mat.kind == MAT_METAL) {
        let onb = onb_from_normal(hit.normal);
        let wo_local = onb_to_local(onb, wo);
        if (wo_local.z <= 0.0) {
            return Scatter(vec3<f32>(0.0), vec3<f32>(0.0), true, false);
        }
        let alpha = max(mat.param * mat.param, 1.0e-4);
        let wm = sample_ggx_vndf(wo_local, alpha, rng);
        let wi_local = reflect(-wo_local, wm);
        if (wi_local.z <= 0.0) {
            return Scatter(vec3<f32>(0.0), vec3<f32>(0.0), true, false);
        }
        let nov = wo_local.z;
        let nol = wi_local.z;
        let cos_hm = clamp(dot(wo_local, wm), 0.0, 1.0);
        let f = fresnel_schlick(cos_hm, mat.color);
        let weight = f * (smith_g2(nov, nol, alpha) / max(smith_g1(nov, alpha), 1.0e-6));
        return Scatter(normalize(onb_to_world(onb, wi_local)), weight, true, true);
    }
    if (mat.kind == MAT_DIELECTRIC) {
        let incident = -wo;
        let cos_theta = clamp(dot(wo, hit.normal), 0.0, 1.0);
        let signed_cos = select(-cos_theta, cos_theta, hit.front_face);
        let reflectance = fresnel_dielectric(signed_cos, mat.param);
        let ratio = select(mat.param, 1.0 / mat.param, hit.front_face);
        var dir: vec3<f32>;
        var throughput: vec3<f32>;
        if (pcg_next_f32(rng) < reflectance) {
            dir = reflect(incident, hit.normal);
            throughput = vec3<f32>(1.0);
        } else {
            let refr = refract_dir(incident, hit.normal, ratio);
            if (refr.ok) {
                dir = refr.dir;
                throughput = mat.tint;
            } else {
                dir = reflect(incident, hit.normal);
                throughput = vec3<f32>(1.0);
            }
        }
        return Scatter(normalize(dir), throughput, true, true);
    }
    // Emissive (or unknown): terminal.
    return Scatter(vec3<f32>(0.0), vec3<f32>(0.0), false, false);
}

// ---- Lighting ---------------------------------------------------------------

fn sky_radiance(dir: vec3<f32>) -> vec3<f32> {
    let up = normalize(dir).z;
    if (up >= 0.0) {
        return mix(u.sky_horizon, u.sky_zenith, pow(up, 0.5));
    }
    return mix(u.sky_horizon, u.sky_ground, pow(-up, 0.5));
}

fn sun_disk_radiance(dir: vec3<f32>) -> vec3<f32> {
    if (max3(u.sun_irradiance) <= 0.0) {
        return vec3<f32>(0.0);
    }
    if (dot(normalize(dir), u.sun_dir) >= u.sun_cos_angular) {
        return u.sun_irradiance * (1.0 / max(u.sun_solid_angle, 1.0e-6));
    }
    return vec3<f32>(0.0);
}

fn direct_sun(hit: Hit, albedo: vec3<f32>, rng: ptr<function, Rng>) -> vec3<f32> {
    if (max3(u.sun_irradiance) <= 0.0) {
        return vec3<f32>(0.0);
    }
    let light_dir = sample_cone(u.sun_dir, u.sun_cos_angular, rng);
    let cosv = dot(hit.normal, light_dir);
    if (cosv <= 0.0) {
        return vec3<f32>(0.0);
    }
    let origin = hit.point + hit.normal * ORIGIN_OFFSET;
    let shadow = make_ray(origin, light_dir, 1.0e-3, 1.0e9);
    if (occluded(shadow)) {
        return vec3<f32>(0.0);
    }
    return albedo * u.sun_irradiance * (cosv / PI);
}

// ---- Integrator -------------------------------------------------------------

fn radiance(primary: Ray, rng: ptr<function, Rng>) -> vec3<f32> {
    var ray = primary;
    var throughput = vec3<f32>(1.0);
    var acc = vec3<f32>(0.0);
    var specular_path = true;
    // Mirrors integrator.rs: once a path has had a diffuse bounce it can only
    // reach the sun disk via an unsampleable caustic (the firefly speckles), so
    // suppress the disk pickup for it. Direct glints keep this false.
    var prior_nonspecular = false;

    for (var bounce = 0u; bounce < u.max_bounces; bounce = bounce + 1u) {
        let hit = intersect_scene(ray);
        if (!hit.valid) {
            var env = sky_radiance(ray.dir);
            if (specular_path && !prior_nonspecular) {
                env = env + sun_disk_radiance(ray.dir);
            }
            acc = acc + throughput * env;
            break;
        }
        let mat = materials[hit.material];
        if (mat.kind == MAT_EMISSIVE) {
            acc = acc + throughput * mat.color;
        }
        let wo = -ray.dir;
        if (mat.kind == MAT_DIFFUSE) {
            acc = acc + throughput * direct_sun(hit, mat.color, rng);
        }
        let sc = scatter(wo, hit, mat, rng);
        if (!sc.ok) {
            break;
        }
        specular_path = sc.specular;
        if (!sc.specular) {
            prior_nonspecular = true;
        }
        throughput = throughput * sc.throughput;
        if (max3(throughput) <= 0.0) {
            break;
        }
        if (bounce >= MIN_BOUNCES) {
            let p = clamp(max3(throughput), 0.05, 1.0);
            if (pcg_next_f32(rng) > p) {
                break;
            }
            throughput = throughput * (1.0 / p);
        }
        let offset_normal = select(-hit.geom_normal, hit.geom_normal, dot(sc.dir, hit.geom_normal) > 0.0);
        ray = make_ray(hit.point + offset_normal * ORIGIN_OFFSET, sc.dir, RAY_EPSILON, T_FAR);
    }

    return min(acc, vec3<f32>(FIREFLY_CLAMP));
}

fn camera_ray(sx: f32, sy: f32) -> Ray {
    let ndc_x = sx / f32(u.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - sy / f32(u.height) * 2.0;
    let dir = normalize(u.cam_forward + u.cam_right * (ndc_x * u.half_w) + u.cam_up * (ndc_y * u.half_h));
    return make_ray(u.cam_eye, dir, RAY_EPSILON, T_FAR);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.width || gid.y >= u.height) {
        return;
    }
    // Trace `spp` samples this dispatch (progressive burst). Each uses a distinct
    // global sample index `u.frame + s`, so the stream matches one-sample-per-
    // dispatch accumulation (frame index = sample) bit-for-bit — what the headless
    // parity test relies on (it drives spp = 1, frame = 0,1,2,...).
    let spp = max(u.spp, 1u);
    var rad = vec3<f32>(0.0);
    for (var s = 0u; s < spp; s = s + 1u) {
        var rng = pixel_rng(gid.x, gid.y, u.frame + s, vec2<u32>(u.seed_lo, u.seed_hi));
        let jx = pcg_next_f32(&rng);
        let jy = pcg_next_f32(&rng);
        let ray = camera_ray(f32(gid.x) + jx, f32(gid.y) + jy);
        rad = rad + radiance(ray, &rng);
    }

    let idx = gid.y * u.width + gid.x;
    let prev = select(vec4<f32>(0.0), accum[idx], u.frame > 0u);
    accum[idx] = prev + vec4<f32>(rad, f32(spp));
}
