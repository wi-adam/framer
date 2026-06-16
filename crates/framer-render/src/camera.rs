//! A perspective pinhole camera derived from the app's orbit state, so the
//! render frames the model from the same vantage as the interactive 3D view.

use crate::math::Vec3;
use crate::ray::Ray;

/// A perspective pinhole camera. `forward`, `right`, and `up` form a right-handed
/// view basis; `half_w`/`half_h` are the image-plane half-extents at unit depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vec3,
    pub center: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub half_w: f32,
    pub half_h: f32,
}

impl Camera {
    /// Builds a camera orbiting `center` at `radius`, matching the app's orbit
    /// projector: `yaw` rotates in the XY plane, `pitch` tilts up, and world +Z
    /// is up. `zoom > 1` moves closer. `vfov_deg` is the vertical field of view.
    #[allow(clippy::too_many_arguments)]
    pub fn orbit(
        center: Vec3,
        radius: f32,
        yaw: f32,
        pitch: f32,
        zoom: f32,
        aspect: f32,
        vfov_deg: f32,
    ) -> Self {
        // Match OrbitProjector: right = (cos yaw, sin yaw), depth axis is its
        // +90° rotation, and the look direction tilts by pitch toward +Z.
        let cos_p = pitch.cos();
        let depth_axis = Vec3::new(-yaw.sin(), yaw.cos(), 0.0);
        // Positive pitch looks down from above (the natural architectural vantage),
        // so the look direction tilts toward -Z.
        let forward =
            Vec3::new(depth_axis.x * cos_p, depth_axis.y * cos_p, -pitch.sin()).normalize();

        let world_up = Vec3::new(0.0, 0.0, 1.0);
        let right = forward.cross(world_up).normalize();
        let up = right.cross(forward).normalize();

        let vfov = vfov_deg.to_radians();
        let half_h = (vfov * 0.5).tan();
        let half_w = half_h * aspect;

        // Distance that frames the bounding sphere, with a small margin.
        let dist = radius / (vfov * 0.5).sin() * 1.05 / zoom.max(1.0e-3);
        let eye = center - forward * dist;

        Self {
            eye,
            center,
            forward,
            right,
            up,
            half_w,
            half_h,
        }
    }

    /// A primary ray through image-plane sample `(sx, sy)` in pixel units (origin
    /// at the top-left; pass fractional coordinates that already include jitter).
    #[inline]
    pub fn ray(&self, sx: f32, sy: f32, width: u32, height: u32) -> Ray {
        let ndc_x = sx / width as f32 * 2.0 - 1.0;
        let ndc_y = 1.0 - sy / height as f32 * 2.0; // flip: row 0 is the top
        let dir =
            (self.forward + self.right * (ndc_x * self.half_w) + self.up * (ndc_y * self.half_h))
                .normalize();
        Ray::new(self.eye, dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;

    fn cam() -> Camera {
        Camera::orbit(
            Vec3::new(10.0, 20.0, 5.0), // center
            8.0,                        // radius
            -0.785,                     // yaw
            0.55,                       // pitch
            1.0,                        // zoom
            16.0 / 9.0,                 // aspect
            40.0,                       // vfov degrees
        )
    }

    #[test]
    fn eye_looks_toward_the_center() {
        let c = cam();
        let to_center = (c.center - c.eye).normalize();
        assert!(
            (to_center - c.forward).length() < 1e-4,
            "forward must point at center"
        );
    }

    #[test]
    fn eye_is_outside_the_bounding_sphere() {
        let c = cam();
        let dist = (c.center - c.eye).length();
        assert!(dist > 8.0, "camera must sit outside the model: dist={dist}");
    }

    #[test]
    fn pitch_raises_the_eye() {
        let c = cam();
        // Positive pitch looks down from above, so the eye is above the center.
        assert!(
            c.eye.z > c.center.z,
            "eye z={} center z={}",
            c.eye.z,
            c.center.z
        );
    }

    #[test]
    fn center_pixel_ray_points_along_forward() {
        let c = cam();
        let (w, h) = (200u32, 100u32);
        // Sampling the exact image center yields the forward direction.
        let ray = c.ray(w as f32 / 2.0, h as f32 / 2.0, w, h);
        assert!((ray.dir - c.forward).length() < 1e-3, "dir={:?}", ray.dir);
        assert!((ray.dir.length() - 1.0).abs() < 1e-4);
        assert_eq!(ray.origin, c.eye);
    }

    #[test]
    fn corner_rays_are_normalized_and_spread() {
        let c = cam();
        let (w, h) = (200u32, 100u32);
        let tl = c.ray(0.0, 0.0, w, h);
        let br = c.ray(w as f32, h as f32, w, h);
        assert!((tl.dir.length() - 1.0).abs() < 1e-4);
        assert!((br.dir.length() - 1.0).abs() < 1e-4);
        // Opposite corners diverge from the forward axis in opposite senses.
        assert!(tl.dir.dot(c.forward) < 1.0);
        assert!(br.dir.dot(c.forward) < 1.0);
        assert!((tl.dir - br.dir).length() > 0.1);
    }

    #[test]
    fn zoom_moves_camera_closer() {
        let near = Camera::orbit(Vec3::ZERO, 5.0, 0.0, 0.4, 2.0, 1.0, 40.0);
        let far = Camera::orbit(Vec3::ZERO, 5.0, 0.0, 0.4, 1.0, 1.0, 40.0);
        assert!(near.eye.length() < far.eye.length());
    }
}
