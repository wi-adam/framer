//! Axis-aligned bounding boxes and the ray slab test used by the BVH.

use crate::math::Vec3;
use crate::ray::Ray;

/// An axis-aligned bounding box. An empty box has `min > max` on every axis so
/// that `grow`/`union` behave as identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// The empty box (absorbing element for `union`/`grow`).
    pub const EMPTY: Self = Self {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    #[inline]
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// Expands the box to include `point`.
    #[inline]
    pub fn grow(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    /// The smallest box containing both `self` and `other`.
    #[inline]
    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb::new(self.min.min(other.min), self.max.max(other.max))
    }

    #[inline]
    pub fn centroid(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    #[inline]
    pub fn extent(&self) -> Vec3 {
        self.max - self.min
    }

    /// Half the surface area is enough for SAH comparisons, but we return the
    /// full surface area for clarity. Empty boxes return 0.
    #[inline]
    pub fn surface_area(&self) -> f32 {
        let e = self.extent();
        if e.x < 0.0 || e.y < 0.0 || e.z < 0.0 {
            return 0.0;
        }
        2.0 * (e.x * e.y + e.y * e.z + e.z * e.x)
    }

    /// Slab test: does `ray` intersect the box within `[ray.t_min, t_max]`?
    #[inline]
    pub fn hit(&self, ray: &Ray, t_max: f32) -> bool {
        let t0 = (self.min - ray.origin).mul(ray.inv_dir);
        let t1 = (self.max - ray.origin).mul(ray.inv_dir);
        let tmin = t0.min(t1);
        let tmax = t0.max(t1);
        let enter = tmin.max_component().max(ray.t_min);
        let exit = tmax.min_component().min(t_max);
        enter <= exit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grow_and_union() {
        let mut b = Aabb::EMPTY;
        b.grow(Vec3::new(1.0, 2.0, 3.0));
        b.grow(Vec3::new(-1.0, 5.0, 0.0));
        assert_eq!(b.min, Vec3::new(-1.0, 2.0, 0.0));
        assert_eq!(b.max, Vec3::new(1.0, 5.0, 3.0));

        let other = Aabb::new(Vec3::new(0.0, 0.0, -2.0), Vec3::new(2.0, 2.0, 2.0));
        let u = b.union(&other);
        assert_eq!(u.min, Vec3::new(-1.0, 0.0, -2.0));
        assert_eq!(u.max, Vec3::new(2.0, 5.0, 3.0));
    }

    #[test]
    fn surface_area_of_unit_cube() {
        let b = Aabb::new(Vec3::ZERO, Vec3::ONE);
        assert_eq!(b.surface_area(), 6.0);
        assert_eq!(Aabb::EMPTY.surface_area(), 0.0);
    }

    #[test]
    fn ray_through_box_hits() {
        let b = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let ray = Ray::new(Vec3::new(-5.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        assert!(b.hit(&ray, f32::INFINITY));
    }

    #[test]
    fn ray_missing_box_does_not_hit() {
        let b = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let ray = Ray::new(Vec3::new(-5.0, 5.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        assert!(!b.hit(&ray, f32::INFINITY));
    }

    #[test]
    fn ray_pointing_away_does_not_hit() {
        let b = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let ray = Ray::new(Vec3::new(-5.0, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0));
        assert!(!b.hit(&ray, f32::INFINITY));
    }

    #[test]
    fn ray_from_inside_hits() {
        let b = Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        let ray = Ray::new(Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
        assert!(b.hit(&ray, f32::INFINITY));
    }

    #[test]
    fn box_beyond_t_max_does_not_hit() {
        let b = Aabb::new(Vec3::new(9.0, -1.0, -1.0), Vec3::new(11.0, 1.0, 1.0));
        let ray = Ray::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0));
        assert!(b.hit(&ray, 100.0));
        assert!(!b.hit(&ray, 5.0));
    }
}
