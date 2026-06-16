//! The path-tracing integrator: estimates the radiance arriving along a camera
//! ray. Direct sunlight on diffuse surfaces is gathered with next-event
//! estimation (a shadow ray toward the sun); indirect light and sky illumination
//! come from BSDF-sampled bounces. The sun is not part of the sky environment,
//! so there is no double counting between the two. Paths are terminated with
//! Russian roulette after a few guaranteed bounces.

use std::f32::consts::PI;

use crate::geom::Hit;
use crate::math::Vec3;
use crate::ray::Ray;
use crate::rng::Pcg32;
use crate::sampling::sample_cone;
use crate::scene::Scene;

/// Surface offset (inches) for spawned rays, to avoid self-intersection.
const ORIGIN_OFFSET: f32 = 1.0e-2;
/// Bounces that always survive before Russian roulette begins.
const MIN_BOUNCES: u32 = 3;

/// Estimates the radiance arriving along `ray` using path tracing.
pub fn radiance(scene: &Scene, mut ray: Ray, rng: &mut Pcg32, max_bounce: u32) -> Vec3 {
    let mut throughput = Vec3::ONE;
    let mut accum = Vec3::ZERO;

    for bounce in 0..max_bounce {
        let Some(hit) = scene.intersect(&ray) else {
            // The ray escaped to the sky.
            accum = accum + throughput.mul(scene.sky.radiance(ray.dir));
            break;
        };
        let material = scene.material(&hit);

        // Emitted light from the surface itself.
        accum = accum + throughput.mul(material.emitted());

        let wo = -ray.dir;

        // Direct sunlight via next-event estimation (diffuse surfaces only; the
        // sun is excluded from the sky, so specular bounces pick it up indirectly).
        if let Some(albedo) = material.diffuse_albedo() {
            accum = accum + throughput.mul(direct_sun(scene, &hit, albedo, rng));
        }

        // Continue the path along a BSDF-sampled direction.
        let Some(scatter) = material.scatter(wo, &hit, rng) else {
            break;
        };
        throughput = throughput.mul(scatter.throughput);
        if throughput.max_component() <= 0.0 {
            break;
        }

        // Russian roulette.
        if bounce >= MIN_BOUNCES {
            let p = throughput.max_component().clamp(0.05, 1.0);
            if rng.next_f32() > p {
                break;
            }
            throughput = throughput * (1.0 / p);
        }

        let offset_normal = if scatter.dir.dot(hit.geom_normal) > 0.0 {
            hit.geom_normal
        } else {
            -hit.geom_normal
        };
        ray = Ray::new(hit.point + offset_normal * ORIGIN_OFFSET, scatter.dir);
    }

    accum
}

/// Direct illumination from the sun on a Lambertian surface. Returns the
/// reflected radiance `albedo/PI * E * cos`, or zero if below the horizon or in
/// shadow.
fn direct_sun(scene: &Scene, hit: &Hit, albedo: Vec3, rng: &mut Pcg32) -> Vec3 {
    let sun = &scene.sun;
    if sun.irradiance.max_component() <= 0.0 {
        return Vec3::ZERO;
    }
    let light_dir = sample_cone(sun.dir, sun.angular_radius, rng);
    let cos = hit.normal.dot(light_dir);
    if cos <= 0.0 {
        return Vec3::ZERO;
    }
    let origin = hit.point + hit.normal * ORIGIN_OFFSET;
    let shadow = Ray::with_range(origin, light_dir, 1.0e-3, 1.0e9);
    if scene.occluded(&shadow) {
        return Vec3::ZERO;
    }
    albedo.mul(sun.irradiance) * (cos / PI)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use crate::geom::Triangle;
    use crate::material::Material;
    use crate::math::Vec3;
    use crate::rng::Pcg32;
    use crate::scene::{DirectionalSun, Scene, Sky};

    /// A camera high above the origin looking straight down at the z=0 plane.
    fn top_down_camera() -> Camera {
        let mut c = Camera::orbit(Vec3::ZERO, 2.0, 0.0, 0.0, 1.0, 1.0, 40.0);
        c.eye = Vec3::new(0.0, 0.0, 10.0);
        c.center = Vec3::ZERO;
        c.forward = Vec3::new(0.0, 0.0, -1.0);
        c.right = Vec3::new(1.0, 0.0, 0.0);
        c.up = Vec3::new(0.0, 1.0, 0.0);
        c
    }

    /// A large quad in the z=0 plane (normal +z) made of two triangles.
    fn floor(material: u32) -> Vec<Triangle> {
        let s = 50.0;
        vec![
            Triangle::new(
                Vec3::new(-s, -s, 0.0),
                Vec3::new(s, -s, 0.0),
                Vec3::new(s, s, 0.0),
                material,
            ),
            Triangle::new(
                Vec3::new(-s, -s, 0.0),
                Vec3::new(s, s, 0.0),
                Vec3::new(-s, s, 0.0),
                material,
            ),
        ]
    }

    fn average_radiance(scene: &Scene, samples: u32) -> Vec3 {
        let mut sum = Vec3::ZERO;
        for s in 0..samples {
            let mut rng = Pcg32::seed(s as u64 + 1, 0x51ED);
            // Straight-down ray onto the middle of the floor.
            let ray = scene.camera.ray(0.5, 0.5, 1, 1);
            sum = sum + radiance(scene, ray, &mut rng, 8);
        }
        sum * (1.0 / samples as f32)
    }

    #[test]
    fn furnace_white_surface_returns_environment() {
        // A perfectly white diffuse surface in a uniform environment must return
        // exactly the environment radiance (energy conservation).
        let l = Vec3::splat(0.5);
        let scene = Scene::new(
            floor(0),
            vec![Material::Diffuse { albedo: Vec3::ONE }],
            DirectionalSun::DARK,
            Sky::uniform(l),
            top_down_camera(),
            1.0,
        );
        let avg = average_radiance(&scene, 20_000);
        assert!((avg - l).length() < 0.01, "furnace failed: {avg:?} vs {l:?}");
    }

    #[test]
    fn half_albedo_reflects_half_the_environment() {
        let l = Vec3::splat(0.6);
        let scene = Scene::new(
            floor(0),
            vec![Material::Diffuse {
                albedo: Vec3::splat(0.5),
            }],
            DirectionalSun::DARK,
            Sky::uniform(l),
            top_down_camera(),
            1.0,
        );
        let avg = average_radiance(&scene, 20_000);
        // Lambertian under uniform L reflects albedo * L.
        assert!((avg - l * 0.5).length() < 0.01, "got {avg:?}");
    }

    #[test]
    fn emissive_surface_returns_its_radiance_exactly() {
        let r = Vec3::new(3.0, 2.0, 1.0);
        let scene = Scene::new(
            floor(0),
            vec![Material::Emissive { radiance: r }],
            DirectionalSun::DARK,
            Sky::uniform(Vec3::ZERO),
            top_down_camera(),
            1.0,
        );
        let mut rng = Pcg32::seed(1, 1);
        let ray = scene.camera.ray(0.5, 0.5, 1, 1);
        let got = radiance(&scene, ray, &mut rng, 8);
        assert!((got - r).length() < 1e-4, "got {got:?}");
    }

    #[test]
    fn sunlit_diffuse_is_brighter_than_shadowed() {
        // Floor lit by an overhead sun, with an occluder casting a shadow.
        let sun = DirectionalSun {
            dir: Vec3::new(0.0, 0.0, 1.0),
            irradiance: Vec3::splat(5.0),
            angular_radius: 0.02,
        };
        let mut tris = floor(0);
        // An occluder quad floating above the origin.
        let h = 5.0;
        let o = 3.0;
        tris.push(Triangle::new(
            Vec3::new(-o, -o, h),
            Vec3::new(o, -o, h),
            Vec3::new(o, o, h),
            0,
        ));
        tris.push(Triangle::new(
            Vec3::new(-o, -o, h),
            Vec3::new(o, o, h),
            Vec3::new(-o, o, h),
            0,
        ));
        let scene = Scene::new(
            tris,
            vec![Material::Diffuse {
                albedo: Vec3::splat(0.8),
            }],
            sun,
            Sky::uniform(Vec3::splat(0.05)),
            top_down_camera(),
            1.0,
        );

        let lit = sample_point(&scene, Vec3::new(30.0, 30.0, 0.0));
        let shadowed = sample_point(&scene, Vec3::new(0.0, 0.0, 0.0));
        assert!(
            lit.max_component() > shadowed.max_component() + 0.1,
            "lit={lit:?} shadowed={shadowed:?}"
        );
    }

    fn sample_point(scene: &Scene, target: Vec3) -> Vec3 {
        // View the floor point along a shallow ray from +Y that passes *under*
        // the overhead occluder (which sits at z=5), so we sample the floor, not
        // the occluder's top.
        let origin = target + Vec3::new(0.0, 8.0, 2.0);
        let dir = (target - origin).normalize();
        let mut sum = Vec3::ZERO;
        let n = 4000;
        for s in 0..n {
            let mut rng = Pcg32::seed(s as u64 + 1, 99);
            sum = sum + radiance(scene, crate::ray::Ray::new(origin, dir), &mut rng, 8);
        }
        sum * (1.0 / n as f32)
    }
}
