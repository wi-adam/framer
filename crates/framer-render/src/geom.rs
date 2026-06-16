//! Triangles, ray–triangle intersection (Möller–Trumbore), and hit records.

use crate::aabb::Aabb;
use crate::math::Vec3;
use crate::ray::Ray;

/// Below this `|det|` the ray is treated as parallel to the triangle.
const PARALLEL_EPS: f32 = 1.0e-8;

/// A triangle with precomputed edges and geometric normal. `material` indexes
/// into the scene's material table.
#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub v0: Vec3,
    pub edge1: Vec3,
    pub edge2: Vec3,
    pub geom_normal: Vec3,
    pub material: u32,
}

/// A ray–triangle intersection record.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// Distance along the ray.
    pub t: f32,
    /// Barycentric coordinates within the triangle.
    pub u: f32,
    pub v: f32,
    /// World-space hit position.
    pub point: Vec3,
    /// Shading normal, always oriented to face the incoming ray.
    pub normal: Vec3,
    /// The triangle's geometric normal (winding-defined, not flipped).
    pub geom_normal: Vec3,
    /// True when the ray struck the front (winding) side.
    pub front_face: bool,
    pub material: u32,
}

impl Triangle {
    #[inline]
    pub fn new(v0: Vec3, v1: Vec3, v2: Vec3, material: u32) -> Self {
        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        Self {
            v0,
            edge1,
            edge2,
            geom_normal: edge1.cross(edge2).normalize(),
            material,
        }
    }

    #[inline]
    pub fn centroid(&self) -> Vec3 {
        self.v0 + (self.edge1 + self.edge2) * (1.0 / 3.0)
    }

    #[inline]
    pub fn aabb(&self) -> Aabb {
        let v1 = self.v0 + self.edge1;
        let v2 = self.v0 + self.edge2;
        let mut bb = Aabb::EMPTY;
        bb.grow(self.v0);
        bb.grow(v1);
        bb.grow(v2);
        bb
    }

    /// Möller–Trumbore intersection. Does **not** cull back faces — a ray must be
    /// able to hit glass from the inside. Returns the nearest hit within the
    /// ray's `[t_min, t_max]` range.
    #[inline]
    pub fn intersect(&self, ray: &Ray) -> Option<Hit> {
        let h = ray.dir.cross(self.edge2);
        let det = self.edge1.dot(h);
        if det.abs() < PARALLEL_EPS {
            return None;
        }
        let inv_det = 1.0 / det;
        let s = ray.origin - self.v0;
        let u = inv_det * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(self.edge1);
        let v = inv_det * ray.dir.dot(q);
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = inv_det * self.edge2.dot(q);
        if t < ray.t_min || t > ray.t_max {
            return None;
        }
        let front_face = ray.dir.dot(self.geom_normal) < 0.0;
        let normal = if front_face {
            self.geom_normal
        } else {
            -self.geom_normal
        };
        Some(Hit {
            t,
            u,
            v,
            point: ray.at(t),
            normal,
            geom_normal: self.geom_normal,
            front_face,
            material: self.material,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;
    use crate::ray::Ray;

    fn tri() -> Triangle {
        // A unit triangle in the z=0 plane, CCW when viewed from +z.
        Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            7,
        )
    }

    #[test]
    fn head_on_hit_reports_t_and_normal() {
        let t = tri();
        // Shoot from above the centroid straight down.
        let ray = Ray::new(Vec3::new(0.2, 0.2, 5.0), Vec3::new(0.0, 0.0, -1.0));
        let hit = t.intersect(&ray).expect("should hit");
        assert!((hit.t - 5.0).abs() < 1e-4, "t={}", hit.t);
        assert_eq!(hit.material, 7);
        // Front face (ray opposes geometric +z normal) -> shading normal faces +z.
        assert!(hit.front_face);
        assert!(hit.normal.dot(Vec3::new(0.0, 0.0, 1.0)) > 0.99);
    }

    #[test]
    fn ray_parallel_to_triangle_misses() {
        let t = tri();
        let ray = Ray::new(Vec3::new(0.2, 0.2, 1.0), Vec3::new(1.0, 0.0, 0.0));
        assert!(t.intersect(&ray).is_none());
    }

    #[test]
    fn ray_outside_the_triangle_misses() {
        let t = tri();
        // Above a point well outside the triangle (x+y > 1).
        let ray = Ray::new(Vec3::new(0.9, 0.9, 5.0), Vec3::new(0.0, 0.0, -1.0));
        assert!(t.intersect(&ray).is_none());
    }

    #[test]
    fn back_face_still_hits_with_flipped_normal() {
        let t = tri();
        // Shoot from below going up: hits the back face (needed for glass interiors).
        let ray = Ray::new(Vec3::new(0.2, 0.2, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let hit = t.intersect(&ray).expect("back face should hit");
        assert!(!hit.front_face);
        // Shading normal faces the ray (toward -z here).
        assert!(hit.normal.dot(Vec3::new(0.0, 0.0, -1.0)) > 0.99);
    }

    #[test]
    fn respects_t_range() {
        let t = tri();
        let ray = Ray::with_range(
            Vec3::new(0.2, 0.2, 5.0),
            Vec3::new(0.0, 0.0, -1.0),
            0.0,
            1.0, // surface is at t=5, beyond t_max
        );
        assert!(t.intersect(&ray).is_none());
    }

    #[test]
    fn bounds_and_centroid() {
        let t = tri();
        let bb = t.aabb();
        assert_eq!(bb.min, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(bb.max, Vec3::new(1.0, 1.0, 0.0));
        let c = t.centroid();
        assert!((c - Vec3::new(1.0 / 3.0, 1.0 / 3.0, 0.0)).length() < 1e-6);
    }
}
