//! The renderable scene: geometry + materials + lighting + camera.

use crate::bvh::Bvh;
use crate::camera::Camera;
use crate::geom::{Hit, Triangle};
use crate::material::{Material, Texture};
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

    /// The sun's solid angle (steradians).
    #[inline]
    pub fn solid_angle(&self) -> f32 {
        std::f32::consts::TAU * (1.0 - self.angular_radius.cos())
    }

    /// Radiance seen when looking *directly* at the sun disk along `dir`, else
    /// zero. This is what **specular** bounces (and primary rays) pick up — the
    /// integrator excludes it for diffuse bounces, which sample the sun via
    /// next-event estimation instead (so there is no double counting). Radiance
    /// is `irradiance / solid_angle`, the inverse of how a diffuse surface turns
    /// the disk's radiance back into irradiance.
    #[inline]
    pub fn disk_radiance(&self, dir: Vec3) -> Vec3 {
        if self.irradiance.max_component() <= 0.0 {
            return Vec3::ZERO;
        }
        if dir.normalize().dot(self.dir) >= self.angular_radius.cos() {
            self.irradiance * (1.0 / self.solid_angle().max(1.0e-6))
        } else {
            Vec3::ZERO
        }
    }
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
    pub textures: Vec<Texture>,
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
        Self::with_textures(triangles, materials, Vec::new(), sun, sky, camera, exposure)
    }

    /// Builds the scene with decoded texture assets. Missing or invalid material
    /// texture indices still shade with their authored fallback colors.
    pub fn with_textures(
        triangles: Vec<Triangle>,
        materials: Vec<Material>,
        textures: Vec<Texture>,
        sun: DirectionalSun,
        sky: Sky,
        camera: Camera,
        exposure: f32,
    ) -> Self {
        let bvh = Bvh::build(&triangles);
        Self {
            triangles,
            materials,
            textures,
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
        let material = self.materials[hit.material as usize];
        match material {
            Material::TexturedDiffuse {
                fallback,
                texture,
                scale,
            } => {
                let albedo = self
                    .textures
                    .get(texture as usize)
                    .map(|texture| sample_texture_at_hit(texture, hit, scale))
                    .unwrap_or(fallback);
                Material::Diffuse { albedo }
            }
            Material::DepthMappedDiffuse {
                albedo,
                height,
                scale,
            } => {
                let albedo = self
                    .textures
                    .get(height as usize)
                    .map(|texture| {
                        let height = sample_texture_at_hit(texture, hit, scale);
                        let relief = height.luminance().clamp(0.0, 1.0);
                        albedo * (0.65 + 0.35 * relief)
                    })
                    .unwrap_or(albedo);
                Material::Diffuse { albedo }
            }
            other => other,
        }
    }
}

fn sample_texture_at_hit(texture: &Texture, hit: &Hit, scale: f32) -> Vec3 {
    let scale = scale.max(1.0e-3);
    // V1 uses a deterministic world-space projection. It avoids storing UVs in
    // authored geometry and keeps CPU/GPU parity straightforward.
    let u = (hit.point.x + hit.point.y) / scale;
    let v = hit.point.z / scale;
    texture.sample_repeat_nearest(u, v)
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
        let camera = Camera::orbit(Vec3::ZERO, 2.0, 0.0, 0.5, 1.0, 1.0, 40.0, 1.0);
        let scene = Scene::new(
            tris,
            materials,
            DirectionalSun::DARK,
            test_sky(),
            camera,
            1.0,
        );
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0));
        let hit = scene.intersect(&ray).expect("should hit triangle");
        assert_eq!(
            scene.material(&hit),
            Material::Diffuse { albedo: Vec3::ONE }
        );
    }

    #[test]
    fn textured_material_samples_resolved_texture() {
        let scene = Scene::with_textures(
            Vec::new(),
            vec![Material::TexturedDiffuse {
                fallback: Vec3::new(0.1, 0.2, 0.3),
                texture: 0,
                scale: 1.0,
            }],
            vec![Texture::new(
                2,
                2,
                vec![
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                    Vec3::new(0.0, 0.0, 1.0),
                    Vec3::new(1.0, 1.0, 0.0),
                ],
            )],
            DirectionalSun::DARK,
            test_sky(),
            Camera::orbit(Vec3::ZERO, 2.0, 0.0, 0.5, 1.0, 1.0, 40.0, 1.0),
            1.0,
        );
        let hit = Hit {
            t: 1.0,
            u: 0.0,
            v: 0.0,
            point: Vec3::new(0.75, 0.0, 0.75),
            normal: Vec3::new(0.0, 0.0, 1.0),
            geom_normal: Vec3::new(0.0, 0.0, 1.0),
            front_face: true,
            material: 0,
        };

        assert_eq!(
            scene.material(&hit),
            Material::Diffuse {
                albedo: Vec3::new(1.0, 1.0, 0.0)
            }
        );
    }

    #[test]
    fn depth_mapped_material_modulates_albedo_by_height_luminance() {
        let base = Vec3::new(0.8, 0.6, 0.4);
        let height = Vec3::new(0.25, 0.5, 1.0);
        let scene = Scene::with_textures(
            Vec::new(),
            vec![Material::DepthMappedDiffuse {
                albedo: base,
                height: 0,
                scale: 1.0,
            }],
            vec![Texture::new(1, 1, vec![height])],
            DirectionalSun::DARK,
            test_sky(),
            Camera::orbit(Vec3::ZERO, 2.0, 0.0, 0.5, 1.0, 1.0, 40.0, 1.0),
            1.0,
        );
        let hit = Hit {
            t: 1.0,
            u: 0.0,
            v: 0.0,
            point: Vec3::ZERO,
            normal: Vec3::new(0.0, 0.0, 1.0),
            geom_normal: Vec3::new(0.0, 0.0, 1.0),
            front_face: true,
            material: 0,
        };
        let expected = base * (0.65 + 0.35 * height.luminance());

        assert!(matches!(
            scene.material(&hit),
            Material::Diffuse { albedo } if (albedo - expected).length() < 1.0e-6
        ));
    }

    #[test]
    fn missing_texture_indices_use_material_fallbacks() {
        let textured_fallback = Vec3::new(0.1, 0.2, 0.3);
        let depth_fallback = Vec3::new(0.4, 0.5, 0.6);
        let scene = Scene::with_textures(
            Vec::new(),
            vec![
                Material::TexturedDiffuse {
                    fallback: textured_fallback,
                    texture: 99,
                    scale: 1.0,
                },
                Material::DepthMappedDiffuse {
                    albedo: depth_fallback,
                    height: 99,
                    scale: 1.0,
                },
            ],
            Vec::new(),
            DirectionalSun::DARK,
            test_sky(),
            Camera::orbit(Vec3::ZERO, 2.0, 0.0, 0.5, 1.0, 1.0, 40.0, 1.0),
            1.0,
        );
        let hit = |material| Hit {
            t: 1.0,
            u: 0.0,
            v: 0.0,
            point: Vec3::ZERO,
            normal: Vec3::new(0.0, 0.0, 1.0),
            geom_normal: Vec3::new(0.0, 0.0, 1.0),
            front_face: true,
            material,
        };

        assert_eq!(
            scene.material(&hit(0)),
            Material::Diffuse {
                albedo: textured_fallback
            }
        );
        assert_eq!(
            scene.material(&hit(1)),
            Material::Diffuse {
                albedo: depth_fallback
            }
        );
    }
}
