//! `framer-render` — physically based rendering for Framer designs.
//!
//! This crate is the UI-agnostic, fully tested source of truth for Framer's
//! path-traced renderer. It extracts a [`scene::Scene`] from a building model,
//! builds a BVH, and renders it with a CPU path tracer (diffuse / metal /
//! dielectric-glass materials, a directional sun, a procedural sky, multiple
//! importance sampling, and ACES tone mapping). The app's WGSL compute path
//! tracer mirrors this exact math, fed by the same scene.
//!
//! The library's only runtime dependencies are `framer-core` and `bytemuck`
//! (the latter just for the `#[repr(C)]` GPU-buffer mirror structs in [`gpu`]);
//! `image` (PNG export) and `rayon` (parallel rendering) are optional and gated
//! behind the `cli` and `parallel` features respectively. All math is `f32` to
//! match WGSL's precision.
#![forbid(unsafe_code)]

pub mod aabb;
pub mod build;
pub mod bvh;
pub mod camera;
pub mod color;
pub mod geom;
pub mod gpu;
pub mod integrator;
pub mod material;
pub mod math;
pub mod ray;
pub mod rng;
pub mod sampling;
pub mod scene;
pub mod scenes;

pub use build::{RenderOptions, scene_from_model};

use math::Vec3;
use scene::Scene;

/// Maximum path-tracing bounce depth.
pub const MAX_BOUNCES: u32 = 8;

/// Adds `samples` more path-traced samples per pixel into `accum` (an HDR
/// linear-radiance sum, length `width * height`), using sample indices
/// `[first_sample, first_sample + samples)`. This is the progressive primitive:
/// the in-app renderer calls it repeatedly to refine an image, and [`render`]
/// calls it once.
///
/// Per-pixel seeding makes each pixel a pure function of `(x, y, sample, seed)`,
/// so accumulation is **independent of thread scheduling** — `parallel` output
/// is byte-identical to single-threaded.
pub fn accumulate(
    scene: &Scene,
    width: u32,
    height: u32,
    samples: u32,
    first_sample: u32,
    seed: u64,
    accum: &mut [Vec3],
) {
    assert_eq!(accum.len(), width as usize * height as usize);

    let sample_pixel = |x: u32, y: u32| -> Vec3 {
        let mut sum = Vec3::ZERO;
        for k in 0..samples {
            let sample = first_sample + k;
            // Stratified (low-discrepancy) sub-pixel jitter; the PCG stream is
            // reserved for the BSDF/light sampling inside `radiance`.
            let (jx, jy) = rng::stratified_jitter(x, y, sample, seed);
            let mut rng = rng::pixel_rng(x, y, sample, seed);
            let ray = scene
                .camera
                .ray(x as f32 + jx, y as f32 + jy, width, height);
            sum = sum + integrator::radiance(scene, ray, &mut rng, MAX_BOUNCES);
        }
        sum
    };

    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        accum.par_iter_mut().enumerate().for_each(|(i, pixel)| {
            let x = (i % width as usize) as u32;
            let y = (i / width as usize) as u32;
            *pixel = *pixel + sample_pixel(x, y);
        });
    }
    #[cfg(not(feature = "parallel"))]
    {
        for (i, pixel) in accum.iter_mut().enumerate() {
            let x = (i % width as usize) as u32;
            let y = (i / width as usize) as u32;
            *pixel = *pixel + sample_pixel(x, y);
        }
    }
}

/// Averages an HDR accumulator over `total_samples` and encodes it to a
/// tightly-packed RGBA8 buffer (exposure + ACES + sRGB).
pub fn tonemap_accum(accum: &[Vec3], total_samples: u32, exposure: f32) -> Vec<u8> {
    let inv = 1.0 / total_samples.max(1) as f32;
    let mut out = Vec::with_capacity(accum.len() * 4);
    for pixel in accum {
        let [r, g, b] = color::tonemap_to_u8(*pixel * inv, exposure);
        out.extend_from_slice(&[r, g, b, 255]);
    }
    out
}

/// Renders `scene` to a tightly-packed RGBA8 buffer (`width * height * 4` bytes,
/// row-major, top row first). Each pixel averages `spp` jittered samples.
///
/// The render is a pure function of `seed` (see [`accumulate`]).
pub fn render(scene: &Scene, width: u32, height: u32, spp: u32, seed: u64) -> Vec<u8> {
    let spp = spp.max(1);
    let mut accum = vec![Vec3::ZERO; width as usize * height as usize];
    accumulate(scene, width, height, spp, 0, seed, &mut accum);
    tonemap_accum(&accum, spp, scene.exposure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use crate::geom::Triangle;
    use crate::material::Material;
    use crate::scene::{DirectionalSun, Sky};

    fn tiny_scene() -> Scene {
        let tris = vec![Triangle::new(
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0,
        )];
        let materials = vec![Material::Diffuse {
            albedo: Vec3::new(0.7, 0.5, 0.3),
        }];
        let camera = Camera::orbit(Vec3::ZERO, 2.0, -0.6, 0.5, 1.0, 1.0, 40.0);
        let sun = DirectionalSun {
            dir: Vec3::new(0.3, 0.2, 1.0).normalize(),
            irradiance: Vec3::splat(4.0),
            angular_radius: 0.03,
        };
        let sky = Sky {
            zenith: Vec3::new(0.2, 0.4, 0.9),
            horizon: Vec3::new(0.7, 0.8, 0.95),
            ground: Vec3::new(0.2, 0.18, 0.15),
        };
        Scene::new(tris, materials, sun, sky, camera, 1.0)
    }

    #[test]
    fn render_produces_rgba_buffer_of_expected_size() {
        let scene = tiny_scene();
        let buf = render(&scene, 32, 24, 4, 1);
        assert_eq!(buf.len(), 32 * 24 * 4);
        // Alpha is fully opaque everywhere.
        assert!(buf.chunks_exact(4).all(|px| px[3] == 255));
    }

    #[test]
    fn render_is_deterministic() {
        let scene = tiny_scene();
        let a = render(&scene, 24, 18, 8, 42);
        let b = render(&scene, 24, 18, 8, 42);
        assert_eq!(a, b, "renders with the same seed must be identical");
    }

    #[test]
    fn render_is_not_blank() {
        // The sky background alone guarantees non-zero pixels.
        let scene = tiny_scene();
        let buf = render(&scene, 32, 24, 4, 7);
        assert!(buf.iter().any(|&b| b > 0));
    }

    #[test]
    fn progressive_accumulation_equals_single_render() {
        // Two accumulate passes (4 + 4 samples) must equal one render at 8 spp:
        // the in-app progressive renderer and the one-shot path agree exactly.
        let scene = tiny_scene();
        let (w, h, seed) = (20u32, 16u32, 5u64);
        let mut accum = vec![Vec3::ZERO; (w * h) as usize];
        accumulate(&scene, w, h, 4, 0, seed, &mut accum);
        accumulate(&scene, w, h, 4, 4, seed, &mut accum);
        let progressive = tonemap_accum(&accum, 8, scene.exposure);
        let one_shot = render(&scene, w, h, 8, seed);
        assert_eq!(progressive, one_shot);
    }
}
