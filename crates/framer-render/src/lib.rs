//! `framer-render` — physically based rendering for Framer designs.
//!
//! This crate is the UI-agnostic, fully tested source of truth for Framer's
//! path-traced renderer. It extracts a [`scene::Scene`] from a building model,
//! builds a BVH, and renders it with a CPU path tracer (diffuse / metal /
//! dielectric-glass materials, a directional sun, a procedural sky, multiple
//! importance sampling, and ACES tone mapping). The app's WGSL compute path
//! tracer mirrors this exact math, fed by the same scene.
//!
//! The library has **zero runtime dependencies** beyond `framer-core`; `image`
//! (PNG export) and `rayon` (parallel rendering) are optional and gated behind
//! the `cli` and `parallel` features respectively. All math is `f32` to match
//! WGSL's precision.
#![forbid(unsafe_code)]

pub mod aabb;
pub mod build;
pub mod bvh;
pub mod camera;
pub mod color;
pub mod geom;
pub mod integrator;
pub mod material;
pub mod math;
pub mod ray;
pub mod rng;
pub mod sampling;
pub mod scene;

pub use build::{RenderOptions, scene_from_model};

use math::Vec3;
use scene::Scene;

/// Maximum path-tracing bounce depth.
pub const MAX_BOUNCES: u32 = 8;

/// Renders `scene` to a tightly-packed RGBA8 buffer (`width * height * 4` bytes,
/// row-major, top row first). Each pixel averages `spp` jittered samples.
///
/// The render is a pure function of `seed`: per-pixel seeding makes it
/// **independent of thread scheduling**, so the `parallel` feature produces
/// byte-identical output to the single-threaded path.
pub fn render(scene: &Scene, width: u32, height: u32, spp: u32, seed: u64) -> Vec<u8> {
    let count = width as usize * height as usize;
    let spp = spp.max(1);

    let render_pixel = |i: usize| -> [u8; 4] {
        let x = (i % width as usize) as u32;
        let y = (i / width as usize) as u32;
        let mut sum = Vec3::ZERO;
        for sample in 0..spp {
            let mut rng = rng::pixel_rng(x, y, sample, seed);
            let jx = rng.next_f32();
            let jy = rng.next_f32();
            let ray = scene.camera.ray(x as f32 + jx, y as f32 + jy, width, height);
            sum = sum + integrator::radiance(scene, ray, &mut rng, MAX_BOUNCES);
        }
        let avg = sum * (1.0 / spp as f32);
        let [r, g, b] = color::tonemap_to_u8(avg, scene.exposure);
        [r, g, b, 255]
    };

    let pixels: Vec<[u8; 4]> = {
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            (0..count).into_par_iter().map(render_pixel).collect()
        }
        #[cfg(not(feature = "parallel"))]
        {
            (0..count).map(render_pixel).collect()
        }
    };
    pixels.into_iter().flatten().collect()
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
}

