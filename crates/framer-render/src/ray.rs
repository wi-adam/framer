//! Rays. The reciprocal direction is precomputed for fast slab tests.

use crate::math::Vec3;

/// A half-line `origin + t * dir` valid for `t in [t_min, t_max]`.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
    pub inv_dir: Vec3,
    pub t_min: f32,
    pub t_max: f32,
}

/// Default ray epsilon — keeps secondary rays from re-hitting their origin surface.
pub const RAY_EPSILON: f32 = 1.0e-3;

impl Ray {
    /// Builds a ray with the standard epsilon `t_min` and an unbounded `t_max`.
    #[inline]
    pub fn new(origin: Vec3, dir: Vec3) -> Self {
        Self {
            origin,
            dir,
            inv_dir: Vec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z),
            t_min: RAY_EPSILON,
            t_max: f32::INFINITY,
        }
    }

    /// Builds a ray with an explicit `t_max` (used for shadow rays toward lights).
    #[inline]
    pub fn with_range(origin: Vec3, dir: Vec3, t_min: f32, t_max: f32) -> Self {
        let mut ray = Self::new(origin, dir);
        ray.t_min = t_min;
        ray.t_max = t_max;
        ray
    }

    #[inline]
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_evaluates_point() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(ray.at(3.0), Vec3::new(3.0, 0.0, 0.0));
        assert_eq!(ray.at(0.0), Vec3::ZERO);
    }

    #[test]
    fn precomputes_inverse_direction() {
        let ray = Ray::new(Vec3::ZERO, Vec3::new(2.0, 4.0, 0.5));
        assert_eq!(ray.inv_dir, Vec3::new(0.5, 0.25, 2.0));
    }

    #[test]
    fn with_range_sets_bounds() {
        let ray = Ray::with_range(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 0.5, 9.0);
        assert_eq!(ray.t_min, 0.5);
        assert_eq!(ray.t_max, 9.0);
    }
}
