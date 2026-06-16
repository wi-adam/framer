//! Fixed synthetic scenes used by tests. [`reference_scene`] exercises every
//! material (diffuse, metal, glass, emissive) under a sun + procedural sky; it is
//! the single source of truth for both the CPU golden-image regression test and
//! the app's GPU↔CPU parity test, so the two validate the *same* scene.

use crate::camera::Camera;
use crate::geom::Triangle;
use crate::material::Material;
use crate::math::Vec3;
use crate::scene::{DirectionalSun, Scene, Sky};

/// Reference render dimensions and sampling (shared by the golden + parity tests).
pub const REFERENCE_WIDTH: u32 = 64;
pub const REFERENCE_HEIGHT: u32 = 48;
pub const REFERENCE_SPP: u32 = 12;
pub const REFERENCE_SEED: u64 = 7;

/// Pushes an axis-aligned cube centered at `c` with half-size `h`.
fn cube(tris: &mut Vec<Triangle>, c: Vec3, h: f32, mat: u32) {
    let corners = [
        Vec3::new(c.x - h, c.y - h, c.z - h),
        Vec3::new(c.x + h, c.y - h, c.z - h),
        Vec3::new(c.x + h, c.y + h, c.z - h),
        Vec3::new(c.x - h, c.y + h, c.z - h),
        Vec3::new(c.x - h, c.y - h, c.z + h),
        Vec3::new(c.x + h, c.y - h, c.z + h),
        Vec3::new(c.x + h, c.y + h, c.z + h),
        Vec3::new(c.x - h, c.y + h, c.z + h),
    ];
    const FACES: [[usize; 4]; 6] = [
        [0, 1, 2, 3],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ];
    for f in FACES {
        tris.push(Triangle::new(
            corners[f[0]],
            corners[f[1]],
            corners[f[2]],
            mat,
        ));
        tris.push(Triangle::new(
            corners[f[0]],
            corners[f[2]],
            corners[f[3]],
            mat,
        ));
    }
}

/// A fixed scene: a ground plane plus three cubes (diffuse red, metal, glass)
/// lit by a warm sun against a blue-grey gradient sky.
pub fn reference_scene() -> Scene {
    let mut tris = Vec::new();
    // Ground (material 0).
    let s = 20.0;
    tris.push(Triangle::new(
        Vec3::new(-s, -s, 0.0),
        Vec3::new(s, -s, 0.0),
        Vec3::new(s, s, 0.0),
        0,
    ));
    tris.push(Triangle::new(
        Vec3::new(-s, -s, 0.0),
        Vec3::new(s, s, 0.0),
        Vec3::new(-s, s, 0.0),
        0,
    ));
    // Three cubes: diffuse red (1), metal (2), glass (3).
    cube(&mut tris, Vec3::new(-2.2, 0.0, 1.0), 1.0, 1);
    cube(&mut tris, Vec3::new(0.0, 0.0, 1.0), 1.0, 2);
    cube(&mut tris, Vec3::new(2.2, 0.0, 1.0), 1.0, 3);

    let materials = vec![
        Material::Diffuse {
            albedo: Vec3::new(0.6, 0.6, 0.58),
        },
        Material::Diffuse {
            albedo: Vec3::new(0.75, 0.25, 0.2),
        },
        Material::Metal {
            albedo: Vec3::new(0.95, 0.9, 0.85),
            roughness: 0.15,
        },
        Material::Dielectric {
            ior: 1.5,
            tint: Vec3::new(0.9, 0.95, 0.93),
        },
    ];

    let camera = Camera::orbit(
        Vec3::new(0.0, 0.0, 1.0),
        4.5,
        -0.6,
        0.35,
        1.0,
        REFERENCE_WIDTH as f32 / REFERENCE_HEIGHT as f32,
        42.0,
    );
    let sun = DirectionalSun {
        dir: Vec3::new(0.4, -0.3, 0.85).normalize(),
        irradiance: Vec3::new(1.0, 0.95, 0.85) * 4.0,
        angular_radius: 0.03,
    };
    let sky = Sky {
        zenith: Vec3::new(0.16, 0.32, 0.75),
        horizon: Vec3::new(0.78, 0.83, 0.9),
        ground: Vec3::new(0.2, 0.18, 0.15),
    };
    Scene::new(tris, materials, sun, sky, camera, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_scene_is_non_trivial() {
        let scene = reference_scene();
        // Ground (2) + three cubes (12 each) = 38 triangles, four materials.
        assert_eq!(scene.triangles.len(), 38);
        assert_eq!(scene.materials.len(), 4);
        assert!(!scene.bvh.nodes.is_empty());
    }
}
