//! Orthonormal basis around a surface normal, used to transform locally-sampled
//! directions (cosine / GGX hemisphere samples) into world space.

use crate::math::Vec3;

/// A right-handed orthonormal basis `{tangent, bitangent, normal}` where the
/// local +Z axis is the surface normal.
#[derive(Clone, Copy, Debug)]
pub struct Onb {
    pub tangent: Vec3,
    pub bitangent: Vec3,
    pub normal: Vec3,
}

impl Onb {
    /// Builds a stable basis from a unit `normal` using Duff et al. (2017),
    /// "Building an Orthonormal Basis, Revisited" — branchless and correct at
    /// the `n.z = -1` singularity that naive cross-product bases miss.
    #[inline]
    pub fn from_normal(normal: Vec3) -> Self {
        let sign = 1.0_f32.copysign(normal.z);
        let a = -1.0 / (sign + normal.z);
        let b = normal.x * normal.y * a;
        let tangent = Vec3::new(
            1.0 + sign * normal.x * normal.x * a,
            sign * b,
            -sign * normal.x,
        );
        let bitangent = Vec3::new(b, sign + normal.y * normal.y * a, -normal.y);
        Self {
            tangent,
            bitangent,
            normal,
        }
    }

    /// Transforms a direction expressed in the local frame (where +Z is the
    /// normal) into world space.
    #[inline]
    pub fn to_world(&self, local: Vec3) -> Vec3 {
        self.tangent * local.x + self.bitangent * local.y + self.normal * local.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;

    const EPS: f32 = 1e-5;

    fn assert_orthonormal(n: Vec3) {
        let onb = Onb::from_normal(n);
        // Unit length.
        assert!((onb.tangent.length() - 1.0).abs() < EPS);
        assert!((onb.bitangent.length() - 1.0).abs() < EPS);
        assert!((onb.normal.length() - 1.0).abs() < EPS);
        // Mutually orthogonal.
        assert!(onb.tangent.dot(onb.bitangent).abs() < EPS);
        assert!(onb.tangent.dot(onb.normal).abs() < EPS);
        assert!(onb.bitangent.dot(onb.normal).abs() < EPS);
        // Right-handed: t x b = n.
        let cross = onb.tangent.cross(onb.bitangent);
        assert!((cross - onb.normal).length() < EPS, "n={n:?} txb={cross:?}");
    }

    #[test]
    fn orthonormal_for_up() {
        assert_orthonormal(Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn orthonormal_for_down_singularity() {
        // The case a naive cross-product ONB fails on.
        assert_orthonormal(Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn orthonormal_for_arbitrary_normals() {
        for n in [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 2.0, 3.0).normalize(),
            Vec3::new(-0.3, 0.7, -0.64).normalize(),
            Vec3::new(0.001, 0.0, -0.9999995).normalize(),
        ] {
            assert_orthonormal(n);
        }
    }

    #[test]
    fn to_world_maps_z_axis_to_normal() {
        let n = Vec3::new(0.2, -0.5, 0.84).normalize();
        let onb = Onb::from_normal(n);
        let mapped = onb.to_world(Vec3::new(0.0, 0.0, 1.0));
        assert!((mapped - n).length() < EPS);
    }
}
