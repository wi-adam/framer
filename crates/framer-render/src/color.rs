//! Tone mapping and color encoding: Narkowicz ACES filmic curve + the proper
//! linear→sRGB transfer function. The WGSL blit shader mirrors these exactly.

use crate::math::Vec3;

/// Narkowicz (2015) fitted ACES filmic tone-mapping curve, applied per channel.
/// Input and output are linear; gamma is applied separately by [`linear_to_srgb`].
#[inline]
pub fn aces_narkowicz(x: Vec3) -> Vec3 {
    const A: f32 = 2.51;
    const B: f32 = 0.03;
    const C: f32 = 2.43;
    const D: f32 = 0.59;
    const E: f32 = 0.14;
    let f = |x: f32| ((x * (A * x + B)) / (x * (C * x + D) + E)).clamp(0.0, 1.0);
    Vec3::new(f(x.x), f(x.y), f(x.z))
}

/// The sRGB opto-electronic transfer function (the proper piecewise curve, not a
/// bare `pow(1/2.2)`). Maps linear `[0,1]` to encoded `[0,1]`.
#[inline]
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Full display pipeline for one HDR pixel: exposure, ACES, sRGB, and 8-bit
/// quantization with round-to-nearest.
#[inline]
pub fn tonemap_to_u8(hdr: Vec3, exposure: f32) -> [u8; 3] {
    let mapped = aces_narkowicz(hdr * exposure);
    let encode = |c: f32| (linear_to_srgb(c) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    [encode(mapped.x), encode(mapped.y), encode(mapped.z)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;

    #[test]
    fn aces_maps_black_to_black() {
        assert_eq!(aces_narkowicz(Vec3::ZERO), Vec3::ZERO);
    }

    #[test]
    fn aces_is_monotonic_and_bounded() {
        let mut prev = -1.0;
        let mut x = 0.0;
        while x <= 20.0 {
            let y = aces_narkowicz(Vec3::splat(x)).x;
            assert!(y >= prev, "not monotonic at {x}");
            assert!((0.0..=1.0).contains(&y), "out of [0,1] at {x}: {y}");
            prev = y;
            x += 0.05;
        }
    }

    #[test]
    fn srgb_endpoints_and_continuity() {
        assert!((linear_to_srgb(0.0)).abs() < 1e-6);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        // The piecewise curve must join continuously at the threshold.
        let lo = 12.92 * 0.0031308;
        let hi = 1.055 * 0.0031308f32.powf(1.0 / 2.4) - 0.055;
        assert!((lo - hi).abs() < 1e-4, "discontinuity: {lo} vs {hi}");
    }

    #[test]
    fn tonemap_clamps_and_orders() {
        let black = tonemap_to_u8(Vec3::ZERO, 1.0);
        assert_eq!(black, [0, 0, 0]);
        // Very bright HDR saturates to white.
        let white = tonemap_to_u8(Vec3::splat(1000.0), 1.0);
        assert_eq!(white, [255, 255, 255]);
        // Mid grey lands in the expected bright-midtone band (curve shape check).
        let mid = tonemap_to_u8(Vec3::splat(0.5), 1.0)[0];
        assert!((200..=212).contains(&mid), "mid grey byte = {mid}");
    }

    #[test]
    fn exposure_brightens() {
        let dim = tonemap_to_u8(Vec3::splat(0.1), 1.0)[0];
        let bright = tonemap_to_u8(Vec3::splat(0.1), 4.0)[0];
        assert!(bright > dim);
    }
}
