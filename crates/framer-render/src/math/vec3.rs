//! A 3-component `f32` vector and the operations the renderer needs.

use std::ops::{Add, Mul, Neg, Sub};

/// A 3-component single-precision vector. Used for positions, directions, and
/// linear-space colors. `f32` throughout to match the WGSL renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self::splat(0.0);
    pub const ONE: Self = Self::splat(1.0);

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[inline]
    pub const fn splat(s: f32) -> Self {
        Self { x: s, y: s, z: s }
    }

    /// Component-wise (Hadamard) product — used for tinting colors by albedo.
    #[inline]
    pub fn mul(self, other: Self) -> Self {
        Self::new(self.x * other.x, self.y * other.y, self.z * other.z)
    }

    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    #[inline]
    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Returns the unit vector in the same direction. The zero vector maps to
    /// itself (avoids NaN); callers never normalize a zero direction in practice.
    #[inline]
    pub fn normalize(self) -> Self {
        let len = self.length();
        if len > 0.0 { self * (1.0 / len) } else { self }
    }

    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self::new(
            self.x.min(other.x),
            self.y.min(other.y),
            self.z.min(other.z),
        )
    }

    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self::new(
            self.x.max(other.x),
            self.y.max(other.y),
            self.z.max(other.z),
        )
    }

    #[inline]
    pub fn max_component(self) -> f32 {
        self.x.max(self.y).max(self.z)
    }

    #[inline]
    pub fn min_component(self) -> f32 {
        self.x.min(self.y).min(self.z)
    }

    #[inline]
    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs(), self.z.abs())
    }

    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }

    /// Reflects `self` (an incident direction) about the unit `normal`.
    /// `reflect(v, n) = v - 2 (v·n) n`.
    #[inline]
    pub fn reflect(self, normal: Self) -> Self {
        self - normal * (2.0 * self.dot(normal))
    }

    /// Refracts the unit incident direction `self` through a surface with
    /// outward unit `normal`, where `eta = n_from / n_into`. Returns `None` on
    /// total internal reflection. (Vector form from "Ray Tracing in One Weekend".)
    #[inline]
    pub fn refract(self, normal: Self, eta: f32) -> Option<Self> {
        let cos_theta = (-self).dot(normal).min(1.0);
        let r_out_perp = (self + normal * cos_theta) * eta;
        let perp_len_sq = r_out_perp.length_squared();
        if perp_len_sq > 1.0 {
            return None; // total internal reflection
        }
        let r_out_parallel = normal * -(1.0 - perp_len_sq).abs().sqrt();
        Some(r_out_perp + r_out_parallel)
    }
}

impl Add for Vec3 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < EPS
    }

    #[test]
    fn new_and_fields() {
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!((v.x, v.y, v.z), (1.0, 2.0, 3.0));
        assert_eq!(Vec3::splat(5.0), Vec3::new(5.0, 5.0, 5.0));
        assert_eq!(Vec3::ZERO, Vec3::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn arithmetic_operators() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, 5.0, 6.0);
        assert_eq!(a + b, Vec3::new(5.0, 7.0, 9.0));
        assert_eq!(b - a, Vec3::new(3.0, 3.0, 3.0));
        assert_eq!(a * 2.0, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(-a, Vec3::new(-1.0, -2.0, -3.0));
        assert_eq!(a.mul(b), Vec3::new(4.0, 10.0, 18.0));
    }

    #[test]
    fn dot_and_cross() {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        let z = Vec3::new(0.0, 0.0, 1.0);
        assert_eq!(x.dot(y), 0.0);
        assert_eq!(x.dot(x), 1.0);
        // Right-handed: x cross y = z.
        assert!(close(x.cross(y), z));
        assert!(close(y.cross(z), x));
        assert!(close(z.cross(x), y));
    }

    #[test]
    fn length_and_normalize() {
        let v = Vec3::new(3.0, 4.0, 0.0);
        assert!((v.length() - 5.0).abs() < EPS);
        assert!((v.length_squared() - 25.0).abs() < EPS);
        let n = v.normalize();
        assert!((n.length() - 1.0).abs() < EPS);
        assert!(close(n, Vec3::new(0.6, 0.8, 0.0)));
    }

    #[test]
    fn reflect_off_a_plane() {
        // A ray going down-right reflecting off the ground (normal +Z) flips Z.
        let incident = Vec3::new(1.0, 0.0, -1.0).normalize();
        let normal = Vec3::new(0.0, 0.0, 1.0);
        let r = incident.reflect(normal);
        assert!(close(r, Vec3::new(1.0, 0.0, 1.0).normalize()));
    }

    #[test]
    fn refract_obeys_snells_law() {
        // Air -> glass. `refract(uv, n, eta)` takes the unit incident direction,
        // the outward unit normal, and eta = n_from / n_into.
        let incident = Vec3::new(0.4, 0.0, -1.0).normalize();
        let n = Vec3::new(0.0, 0.0, 1.0);
        let into = incident.refract(n, 1.0 / 1.5).expect("should refract in");
        assert!((into.length() - 1.0).abs() < EPS, "refracted dir must be unit");
        // It must transmit (continue downward, past the surface).
        assert!(into.z < 0.0);
        // Snell: sin(theta_in) / sin(theta_out) = n_glass / n_air = 1.5.
        let minus_n = -n;
        let sin_in = incident.cross(minus_n).length(); // |sin| of angle to -n
        let sin_out = into.cross(minus_n).length();
        assert!((sin_in / sin_out - 1.5).abs() < 1e-3, "ratio={}", sin_in / sin_out);
    }

    #[test]
    fn refract_total_internal_reflection_returns_none() {
        // Steep ray inside glass leaving to air past the critical angle: TIR.
        let incident = Vec3::new(0.95, 0.0, -0.31).normalize();
        let n = Vec3::new(0.0, 0.0, 1.0);
        assert!(incident.refract(n, 1.5).is_none());
    }

    #[test]
    fn min_max_and_components() {
        let a = Vec3::new(1.0, 5.0, 3.0);
        let b = Vec3::new(4.0, 2.0, 6.0);
        assert_eq!(a.min(b), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(a.max(b), Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(a.max_component(), 5.0);
        assert_eq!(a.min_component(), 1.0);
    }

    #[test]
    fn lerp_interpolates() {
        let a = Vec3::ZERO;
        let b = Vec3::splat(10.0);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        assert_eq!(a.lerp(b, 0.5), Vec3::splat(5.0));
    }
}
