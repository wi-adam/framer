//! Physically based materials and their scattering behaviour.
//!
//! Each `scatter` returns the next ray direction and a throughput multiplier.
//! Diffuse surfaces are sampled cosine-weighted and also receive direct light
//! via next-event estimation in the integrator. Metal uses GGX VNDF sampling.
//! Dielectric (glass) is a smooth Fresnel reflect/refract. Emissive surfaces are
//! terminal light sources.

use std::f32::consts::PI;

use crate::geom::Hit;
use crate::math::{Onb, Vec3};
use crate::rng::Pcg32;
use crate::sampling::{
    cosine_sample_hemisphere, fresnel_dielectric, fresnel_schlick, sample_ggx_vndf, smith_g1,
    smith_g2,
};

/// A physically based material. Colors are linear reflectances / radiances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Material {
    /// Lambertian diffuse reflector.
    Diffuse { albedo: Vec3 },
    /// GGX microfacet conductor; `albedo` is the F0 reflectance, `roughness` in
    /// `[0, 1]` (perceptual; squared internally for the GGX alpha).
    Metal { albedo: Vec3, roughness: f32 },
    /// Smooth dielectric (glass). `ior` is the index of refraction; `tint` colors
    /// transmitted light.
    Dielectric { ior: f32, tint: Vec3 },
    /// Emitter — a light source. Terminal (does not scatter).
    Emissive { radiance: Vec3 },
}

/// The result of scattering a ray off a surface.
#[derive(Clone, Copy, Debug)]
pub struct Scatter {
    /// Next ray direction in world space (unit length).
    pub dir: Vec3,
    /// Multiply the path throughput by this.
    pub throughput: Vec3,
    /// When true the integrator skips next-event estimation for this bounce
    /// (the BSDF is a delta or near-delta, so direct light sampling is invalid).
    pub specular: bool,
    /// Solid-angle pdf of `dir`, for multiple importance sampling (0 if specular).
    pub pdf: f32,
}

impl Material {
    /// Emitted radiance (non-zero only for [`Material::Emissive`]).
    #[inline]
    pub fn emitted(&self) -> Vec3 {
        match self {
            Material::Emissive { radiance } => *radiance,
            _ => Vec3::ZERO,
        }
    }

    /// Whether direct-light sampling should be skipped for this material.
    #[inline]
    pub fn is_specular(&self) -> bool {
        matches!(self, Material::Dielectric { .. } | Material::Metal { .. })
    }

    /// The Lambertian albedo, used by the integrator for next-event estimation.
    #[inline]
    pub fn diffuse_albedo(&self) -> Option<Vec3> {
        match self {
            Material::Diffuse { albedo } => Some(*albedo),
            _ => None,
        }
    }

    /// Samples an outgoing direction. `wo` is the unit direction from the surface
    /// toward the viewer (i.e. `-ray.dir`). Returns `None` for terminal surfaces
    /// or when the sampled direction is invalid (below the surface).
    pub fn scatter(&self, wo: Vec3, hit: &Hit, rng: &mut Pcg32) -> Option<Scatter> {
        match self {
            Material::Emissive { .. } => None,

            Material::Diffuse { albedo } => {
                let onb = Onb::from_normal(hit.normal);
                let local = cosine_sample_hemisphere(rng);
                let dir = onb.to_world(local).normalize();
                Some(Scatter {
                    dir,
                    throughput: *albedo, // cosine and pdf cancel
                    specular: false,
                    pdf: local.z / PI,
                })
            }

            Material::Metal { albedo, roughness } => {
                let onb = Onb::from_normal(hit.normal);
                let wo_local = onb.to_local(wo);
                if wo_local.z <= 0.0 {
                    return None;
                }
                let alpha = (roughness * roughness).max(1.0e-4);
                let wm = sample_ggx_vndf(wo_local, alpha, rng);
                let wi_local = (-wo_local).reflect(wm);
                if wi_local.z <= 0.0 {
                    return None; // reflected below the surface
                }
                let nov = wo_local.z;
                let nol = wi_local.z;
                let cos_hm = wo_local.dot(wm).clamp(0.0, 1.0);
                let f = fresnel_schlick(cos_hm, *albedo);
                // VNDF sampling weight: F * G2 / G1 (D and pdf cancel).
                let weight = f * (smith_g2(nov, nol, alpha) / smith_g1(nov, alpha).max(1.0e-6));
                Some(Scatter {
                    dir: onb.to_world(wi_local).normalize(),
                    throughput: weight,
                    specular: true,
                    pdf: 0.0,
                })
            }

            Material::Dielectric { ior, tint } => {
                let incident = -wo; // direction of travel into the surface
                let cos_theta = wo.dot(hit.normal).clamp(0.0, 1.0);
                // Signed cosine w.r.t. the outward geometric normal for Fresnel.
                let signed_cos = if hit.front_face {
                    cos_theta
                } else {
                    -cos_theta
                };
                let reflectance = fresnel_dielectric(signed_cos, *ior);
                let ratio = if hit.front_face { 1.0 / *ior } else { *ior };

                let (dir, throughput) = if rng.next_f32() < reflectance {
                    (incident.reflect(hit.normal), Vec3::ONE)
                } else {
                    match incident.refract(hit.normal, ratio) {
                        Some(refracted) => (refracted, *tint),
                        None => (incident.reflect(hit.normal), Vec3::ONE),
                    }
                };
                Some(Scatter {
                    dir: dir.normalize(),
                    throughput,
                    specular: true,
                    pdf: 0.0,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Hit;
    use crate::math::Vec3;
    use crate::rng::Pcg32;

    fn hit_at(normal: Vec3, front_face: bool) -> Hit {
        Hit {
            t: 1.0,
            u: 0.0,
            v: 0.0,
            point: Vec3::ZERO,
            normal,
            geom_normal: if front_face { normal } else { -normal },
            front_face,
            material: 0,
        }
    }

    #[test]
    fn emitted_only_for_emissive() {
        let r = Vec3::new(2.0, 3.0, 4.0);
        assert_eq!(Material::Emissive { radiance: r }.emitted(), r);
        assert_eq!(
            Material::Diffuse { albedo: Vec3::ONE }.emitted(),
            Vec3::ZERO
        );
    }

    #[test]
    fn specular_classification() {
        assert!(!Material::Diffuse { albedo: Vec3::ONE }.is_specular());
        assert!(
            Material::Dielectric {
                ior: 1.5,
                tint: Vec3::ONE
            }
            .is_specular()
        );
        assert!(
            Material::Metal {
                albedo: Vec3::ONE,
                roughness: 0.2
            }
            .is_specular()
        );
        assert!(
            !Material::Emissive {
                radiance: Vec3::ONE
            }
            .is_specular()
        );
    }

    #[test]
    fn diffuse_scatters_into_upper_hemisphere_with_albedo_throughput() {
        let albedo = Vec3::new(0.8, 0.4, 0.2);
        let mat = Material::Diffuse { albedo };
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let hit = hit_at(normal, true);
        let wo = Vec3::new(0.0, 0.0, 1.0);
        let mut rng = Pcg32::seed(1, 2);
        for _ in 0..5000 {
            let s = mat
                .scatter(wo, &hit, &mut rng)
                .expect("diffuse always scatters");
            assert!(!s.specular);
            assert!(s.dir.dot(normal) > -1e-4, "below surface: {:?}", s.dir);
            assert!((s.dir.length() - 1.0).abs() < 1e-3);
            // Cosine sampling makes the throughput exactly the albedo.
            assert!((s.throughput - albedo).length() < 1e-5);
            assert!(s.pdf > 0.0);
        }
    }

    #[test]
    fn dielectric_is_specular_and_unit_directioned() {
        let mat = Material::Dielectric {
            ior: 1.5,
            tint: Vec3::ONE,
        };
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let hit = hit_at(normal, true);
        let wo = Vec3::new(0.0, 0.0, 1.0); // head-on
        let mut rng = Pcg32::seed(3, 4);
        let mut refracted = 0;
        for _ in 0..5000 {
            let s = mat.scatter(wo, &hit, &mut rng).expect("glass scatters");
            assert!(s.specular);
            assert!((s.dir.length() - 1.0).abs() < 1e-3);
            // Clear glass loses no energy.
            assert!((s.throughput - Vec3::ONE).length() < 1e-5);
            if s.dir.z < 0.0 {
                refracted += 1; // transmitted through to the far side
            }
        }
        // Near-normal incidence transmits the large majority (~96%).
        assert!(
            refracted > 4500,
            "expected mostly transmission, got {refracted}"
        );
    }

    #[test]
    fn dielectric_total_internal_reflection_from_inside() {
        // A ray inside glass hitting the boundary at a grazing angle reflects.
        let mat = Material::Dielectric {
            ior: 1.5,
            tint: Vec3::ONE,
        };
        // Back face: shading normal points "down" toward the ray inside the glass.
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let hit = hit_at(normal, false);
        // Shallow grazing view direction (mostly sideways).
        let wo = Vec3::new(0.95, 0.0, 0.31).normalize();
        let mut rng = Pcg32::seed(5, 6);
        let s = mat.scatter(wo, &hit, &mut rng).expect("scatters");
        // Must reflect back to the same side as wo (positive z), never transmit.
        assert!(s.dir.z > 0.0, "TIR must reflect, got {:?}", s.dir);
    }

    #[test]
    fn dielectric_back_face_refracts_outward() {
        // A ray exiting glass through a back face at shallow incidence must
        // refract *outward* (continue past the surface), not flip back inside.
        // `hit.normal` faces the ray (inward, -geom), which is exactly the normal
        // `refract` expects — this guards against a "fix" that would break it.
        let mat = Material::Dielectric {
            ior: 1.5,
            tint: Vec3::ONE,
        };
        // Real back-face exit through the +z face: geom normal = +z, so the
        // ray-facing shading normal is -z (what the intersector produces).
        let hit = hit_at(Vec3::new(0.0, 0.0, -1.0), false);
        // Viewer is inside the glass; the ray travels outward (+z), near normal.
        let wo = Vec3::new(0.2, 0.0, -0.98).normalize();
        let mut rng = Pcg32::seed(21, 4);
        let mut transmitted = 0;
        for _ in 0..5000 {
            let s = mat.scatter(wo, &hit, &mut rng).expect("scatters");
            assert!((s.dir.length() - 1.0).abs() < 1e-3);
            if s.dir.z > 0.0 {
                transmitted += 1; // exited outward through the +z face
            }
        }
        // Near-normal incidence transmits the large majority; none should be
        // trapped flipping back inward incorrectly.
        assert!(
            transmitted > 4000,
            "back-face refraction failed: {transmitted}/5000 exited"
        );
    }

    #[test]
    fn metal_conserves_energy() {
        let mat = Material::Metal {
            albedo: Vec3::new(0.95, 0.93, 0.88),
            roughness: 0.25,
        };
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let hit = hit_at(normal, true);
        let wo = Vec3::new(0.3, 0.0, 0.95).normalize();
        let mut rng = Pcg32::seed(7, 8);
        for _ in 0..10_000 {
            if let Some(s) = mat.scatter(wo, &hit, &mut rng) {
                assert!(s.specular);
                assert!((s.dir.length() - 1.0).abs() < 1e-3);
                // No single bounce may amplify energy.
                assert!(
                    s.throughput.max_component() <= 1.0 + 1e-4,
                    "energy gain: {:?}",
                    s.throughput
                );
            }
        }
    }
}
