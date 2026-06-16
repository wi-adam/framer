//! Importance-sampling primitives and Fresnel terms for the BSDFs. All
//! directions here are in the local shading frame where +Z is the surface
//! normal. The WGSL renderer mirrors these routines.

use std::f32::consts::{PI, TAU};

use crate::math::Vec3;
use crate::rng::Pcg32;

/// Cosine-weighted hemisphere sample (local +Z up). The matching pdf is
/// `cos_theta / PI`.
#[inline]
pub fn cosine_sample_hemisphere(rng: &mut Pcg32) -> Vec3 {
    let r1 = rng.next_f32();
    let r2 = rng.next_f32();
    let phi = TAU * r1;
    let r = r2.sqrt();
    Vec3::new(r * phi.cos(), r * phi.sin(), (1.0 - r2).max(0.0).sqrt())
}

/// Schlick Fresnel for conductors (per-channel `f0` reflectance at normal
/// incidence).
#[inline]
pub fn fresnel_schlick(cos_theta: f32, f0: Vec3) -> Vec3 {
    let m = (1.0 - cos_theta).clamp(0.0, 1.0);
    let m5 = m * m * m * m * m;
    f0 + (Vec3::ONE - f0) * m5
}

/// Exact (un-approximated) dielectric Fresnel reflectance. `cos_i` is the cosine
/// of the angle between the incident ray and the surface normal (sign indicates
/// which side); `ior` is the glass index relative to air. Returns 1.0 on total
/// internal reflection.
#[inline]
pub fn fresnel_dielectric(cos_i: f32, ior: f32) -> f32 {
    let mut cos_i = cos_i.clamp(-1.0, 1.0);
    let (eta_i, eta_t) = if cos_i > 0.0 { (1.0, ior) } else { (ior, 1.0) };
    cos_i = cos_i.abs();
    let sin_t = eta_i / eta_t * (1.0 - cos_i * cos_i).max(0.0).sqrt();
    if sin_t >= 1.0 {
        return 1.0; // total internal reflection
    }
    let cos_t = (1.0 - sin_t * sin_t).max(0.0).sqrt();
    let r_parl = (eta_t * cos_i - eta_i * cos_t) / (eta_t * cos_i + eta_i * cos_t);
    let r_perp = (eta_i * cos_i - eta_t * cos_t) / (eta_i * cos_i + eta_t * cos_t);
    0.5 * (r_parl * r_parl + r_perp * r_perp)
}

/// GGX / Trowbridge–Reitz normal distribution. `noh` = dot(normal, half),
/// `alpha` = roughness².
#[inline]
pub fn ggx_d(noh: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = noh * noh * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d).max(1e-12)
}

#[inline]
fn smith_lambda(nox: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let c2 = (nox * nox).max(1e-7);
    let tan2 = (1.0 - c2) / c2;
    0.5 * (-1.0 + (1.0 + a2 * tan2).sqrt())
}

/// Smith masking term G1.
#[inline]
pub fn smith_g1(nov: f32, alpha: f32) -> f32 {
    1.0 / (1.0 + smith_lambda(nov, alpha))
}

/// Height-correlated Smith masking-shadowing term G2.
#[inline]
pub fn smith_g2(nov: f32, nol: f32, alpha: f32) -> f32 {
    1.0 / (1.0 + smith_lambda(nov, alpha) + smith_lambda(nol, alpha))
}

/// Samples a visible GGX microfacet normal (Heitz 2018, isotropic). `ve` is the
/// view/outgoing direction in the local frame (+Z up); `alpha` = roughness².
/// Returns the sampled half-vector `wm` in the local frame.
#[inline]
pub fn sample_ggx_vndf(ve: Vec3, alpha: f32, rng: &mut Pcg32) -> Vec3 {
    // Stretch the view direction into the hemisphere configuration.
    let vh = Vec3::new(alpha * ve.x, alpha * ve.y, ve.z).normalize();
    // Orthonormal basis around vh.
    let lensq = vh.x * vh.x + vh.y * vh.y;
    let t1 = if lensq > 0.0 {
        Vec3::new(-vh.y, vh.x, 0.0) * (1.0 / lensq.sqrt())
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let t2 = vh.cross(t1);
    // Sample a point on the projected disk.
    let u1 = rng.next_f32();
    let u2 = rng.next_f32();
    let r = u1.sqrt();
    let phi = TAU * u2;
    let p_x = r * phi.cos();
    let mut p_y = r * phi.sin();
    let s = 0.5 * (1.0 + vh.z);
    p_y = (1.0 - s) * (1.0 - p_x * p_x).max(0.0).sqrt() + s * p_y;
    let p_z = (1.0 - p_x * p_x - p_y * p_y).max(0.0).sqrt();
    let nh = t1 * p_x + t2 * p_y + vh * p_z;
    // Unstretch to get the microfacet normal.
    Vec3::new(alpha * nh.x, alpha * nh.y, nh.z.max(0.0)).normalize()
}

/// Power heuristic (β = 2) weight for combining two sampling strategies.
#[inline]
pub fn power_heuristic(pdf_a: f32, pdf_b: f32) -> f32 {
    let a2 = pdf_a * pdf_a;
    let b2 = pdf_b * pdf_b;
    if a2 + b2 <= 0.0 { 0.0 } else { a2 / (a2 + b2) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_samples_stay_in_upper_hemisphere() {
        let mut rng = Pcg32::seed(1, 1);
        let mut mean_z = 0.0f64;
        let n = 200_000;
        for _ in 0..n {
            let d = cosine_sample_hemisphere(&mut rng);
            assert!(d.z >= 0.0, "below hemisphere: {d:?}");
            assert!((d.length() - 1.0).abs() < 1e-4);
            mean_z += d.z as f64;
        }
        // E[cos] for cosine-weighted sampling is 2/3.
        let mean = mean_z / n as f64;
        assert!((mean - 2.0 / 3.0).abs() < 5e-3, "mean z = {mean}");
    }

    #[test]
    fn dielectric_fresnel_normal_incidence() {
        // Air->glass at normal incidence: R0 = ((1.5-1)/(1.5+1))^2 = 0.04.
        let r = fresnel_dielectric(1.0, 1.5);
        assert!((r - 0.04).abs() < 1e-3, "R0 = {r}");
    }

    #[test]
    fn dielectric_fresnel_grazing_goes_to_one() {
        let r = fresnel_dielectric(0.001, 1.5);
        assert!(r > 0.95, "grazing reflectance too low: {r}");
    }

    #[test]
    fn dielectric_fresnel_total_internal_reflection() {
        // Inside glass (cos_i < 0) past the critical angle -> full reflection.
        let r = fresnel_dielectric(-0.31, 1.5);
        assert_eq!(r, 1.0);
    }

    #[test]
    fn schlick_fresnel_endpoints() {
        let f0 = Vec3::new(0.95, 0.64, 0.54); // copper-ish
        // Normal incidence returns f0.
        let at_normal = fresnel_schlick(1.0, f0);
        assert!((at_normal - f0).length() < 1e-5);
        // Grazing returns ~white.
        let at_grazing = fresnel_schlick(0.0, f0);
        assert!((at_grazing - Vec3::ONE).length() < 1e-4);
    }

    #[test]
    fn vndf_sample_is_a_valid_upper_hemisphere_normal() {
        let mut rng = Pcg32::seed(4, 9);
        let ve = Vec3::new(0.3, -0.2, 0.93).normalize();
        for _ in 0..10_000 {
            let wm = sample_ggx_vndf(ve, 0.3, &mut rng);
            assert!((wm.length() - 1.0).abs() < 1e-3);
            assert!(wm.z > 0.0, "microfacet normal below surface: {wm:?}");
        }
    }

    #[test]
    fn smith_terms_in_range() {
        for &nov in &[0.1, 0.5, 0.9, 1.0] {
            let g1 = smith_g1(nov, 0.4);
            assert!((0.0..=1.0).contains(&g1), "G1 out of range: {g1}");
            let g2 = smith_g2(nov, nov, 0.4);
            assert!((0.0..=1.0).contains(&g2));
            assert!(g2 <= g1 + 1e-4, "G2 should not exceed G1");
        }
    }

    #[test]
    fn power_heuristic_basic() {
        assert!((power_heuristic(1.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((power_heuristic(0.0, 1.0)).abs() < 1e-6);
        assert!((power_heuristic(1.0, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(power_heuristic(0.0, 0.0), 0.0);
    }
}
