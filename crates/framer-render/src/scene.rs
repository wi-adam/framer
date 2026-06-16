//! The renderable scene: geometry + materials + lighting + camera.

use crate::bvh::Bvh;
use crate::camera::Camera;
use crate::geom::{Hit, Triangle};
use crate::material::Material;
use crate::math::Vec3;
use crate::ray::Ray;

/// A distant directional light (the sun). `dir` points **toward** the sun.
/// `irradiance` is the linear power delivered to a surface facing the sun.
/// `angular_radius` (radians) softens shadow edges.
#[derive(Clone, Copy, Debug)]
pub struct DirectionalSun {
    pub dir: Vec3,
    pub irradiance: Vec3,
    pub angular_radius: f32,
}

impl DirectionalSun {
    /// A sun with no light (used to disable direct lighting, e.g. furnace tests).
    pub const DARK: Self = Self {
        dir: Vec3::new(0.0, 0.0, 1.0),
        irradiance: Vec3::ZERO,
        angular_radius: 0.02,
    };
}

/// A simple analytic sky: a horizon→zenith gradient above, fading to a muted
/// ground color below the horizon (only seen in reflections, since an explicit
/// ground plane usually intercepts downward rays first).
#[derive(Clone, Copy, Debug)]
pub struct Sky {
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub ground: Vec3,
}

impl Sky {
    /// Radiance arriving from direction `dir` (need not be normalized).
    #[inline]
    pub fn radiance(&self, dir: Vec3) -> Vec3 {
        let up = dir.normalize().z;
        if up >= 0.0 {
            self.horizon.lerp(self.zenith, up.powf(0.5))
        } else {
            self.horizon.lerp(self.ground, (-up).powf(0.5))
        }
    }

    /// A uniform sky of a single radiance (used by the furnace test).
    pub fn uniform(radiance: Vec3) -> Self {
        Self {
            zenith: radiance,
            horizon: radiance,
            ground: radiance,
        }
    }
}

/// A complete scene ready to render. `triangles[i]`'s material is
/// `materials[triangles[i].material]`.
#[derive(Clone, Debug)]
pub struct Scene {
    pub triangles: Vec<Triangle>,
    pub materials: Vec<Material>,
    pub bvh: Bvh,
    pub sun: DirectionalSun,
    pub sky: Sky,
    pub camera: Camera,
    /// Linear exposure multiplier applied before tone mapping.
    pub exposure: f32,
}

impl Scene {
    /// Builds the scene and its BVH from triangles, materials, lighting, and camera.
    pub fn new(
        triangles: Vec<Triangle>,
        materials: Vec<Material>,
        sun: DirectionalSun,
        sky: Sky,
        camera: Camera,
        exposure: f32,
    ) -> Self {
        let bvh = Bvh::build(&triangles);
        Self {
            triangles,
            materials,
            bvh,
            sun,
            sky,
            camera,
            exposure,
        }
    }

    /// Nearest intersection of `ray` with the scene.
    #[inline]
    pub fn intersect(&self, ray: &Ray) -> Option<Hit> {
        self.bvh.intersect(&self.triangles, ray)
    }

    /// Whether `ray` is blocked within its range (shadow query).
    #[inline]
    pub fn occluded(&self, ray: &Ray) -> bool {
        self.bvh.occluded(&self.triangles, ray)
    }

    /// The material struck by `hit`.
    #[inline]
    pub fn material(&self, hit: &Hit) -> Material {
        self.materials[hit.material as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_sky() -> Sky {
        Sky {
            zenith: Vec3::new(0.2, 0.4, 0.9),
            horizon: Vec3::new(0.8, 0.85, 0.95),
            ground: Vec3::new(0.2, 0.18, 0.15),
        }
    }

    #[test]
    fn sky_straight_up_is_zenith() {
        let sky = test_sky();
        let r = sky.radiance(Vec3::new(0.0, 0.0, 1.0));
        assert!((r - sky.zenith).length() < 1e-5);
    }

    #[test]
    fn sky_at_horizon_is_horizon_color() {
        let sky = test_sky();
        let r = sky.radiance(Vec3::new(1.0, 0.0, 0.0));
        assert!((r - sky.horizon).length() < 1e-5);
    }

    #[test]
    fn sky_downward_tends_to_ground() {
        let sky = test_sky();
        let r = sky.radiance(Vec3::new(0.0, 0.0, -1.0));
        assert!((r - sky.ground).length() < 1e-5);
    }

    #[test]
    fn uniform_sky_is_constant() {
        let sky = Sky::uniform(Vec3::splat(0.5));
        for d in [
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(1.0, 1.0, 0.2).normalize(),
        ] {
            assert!((sky.radiance(d) - Vec3::splat(0.5)).length() < 1e-5);
        }
    }

    #[test]
    fn scene_intersects_its_geometry() {
        let tris = vec![Triangle::new(
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0,
        )];
        let materials = vec![Material::Diffuse { albedo: Vec3::ONE }];
        let camera = Camera::orbit(Vec3::ZERO, 2.0, 0.0, 0.5, 1.0, 1.0, 40.0);
        let scene = Scene::new(tris, materials, DirectionalSun::DARK, test_sky(), camera, 1.0);
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0));
        let hit = scene.intersect(&ray).expect("should hit triangle");
        assert_eq!(scene.material(&hit), Material::Diffuse { albedo: Vec3::ONE });
    }
}
