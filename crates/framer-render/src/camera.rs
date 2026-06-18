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
    /// is up. `zoom > 1` is telephoto (narrows the FOV, magnifying uniformly).
    /// `dolly` scales the eye's distance from `center` along the view axis: `< 1`
    /// moves the eye toward (and eventually inside) the model, `> 1` pulls it back.
    /// `vfov_deg` is the vertical field of view.
    #[allow(clippy::too_many_arguments)]
    pub fn orbit(
        center: Vec3,
        radius: f32,
        yaw: f32,
        pitch: f32,
        zoom: f32,
        aspect: f32,
        vfov_deg: f32,
        dolly: f32,
    ) -> Self {
        // Reproduce the OrbitProjector's screen basis exactly so the path tracer
        // frames the model from the same vantage as the interactive 3D view (the
        // two share one orbit state). The projector defines, in world space:
        //   screen-right = (cos yaw, sin yaw, 0)
        //   screen-up    = (sin yaw sin pitch, -cos yaw sin pitch, cos pitch)
        //   eye offset   = (-sin yaw cos pitch, cos yaw cos pitch, sin pitch)  (toward the eye)
        // so `forward` (eye → scene) is the negation of that eye offset. These are
        // already orthonormal; `normalize` only guards against f32 drift.
        //
        // Deriving `right`/`up` from `forward × world_up` (the textbook approach)
        // does NOT reproduce this basis — it yields a vertically mirrored view —
        // because the projector's handedness is `forward = right × up`, not the
        // usual `up = right × forward`. Construct all three explicitly instead.
        let (sin_y, cos_y) = (yaw.sin(), yaw.cos());
        let (sin_p, cos_p) = (pitch.sin(), pitch.cos());
        // Positive pitch looks down from above (the natural architectural vantage):
        // forward tilts toward -Z, placing the eye above the center.
        let forward = Vec3::new(sin_y * cos_p, -cos_y * cos_p, -sin_p).normalize();
        let right = Vec3::new(cos_y, sin_y, 0.0).normalize();
        let up = Vec3::new(sin_y * sin_p, -cos_y * sin_p, cos_p).normalize();

        let vfov = vfov_deg.to_radians();
        // Zoom is telephoto, not dolly: narrow the field of view at a fixed framing
        // distance instead of moving the eye toward the model. The interactive 3D
        // view is an orthographic projection whose zoom magnifies the image
        // uniformly; a dolly would instead dive into the room and exaggerate
        // perspective as you zoom in, drifting out of sync with it. Narrowing the
        // FOV scales the whole image uniformly about the center (matching the ortho
        // zoom) and keeps the eye outside the model at any zoom.
        let half_h = (vfov * 0.5).tan() / zoom.max(1.0e-3);
        let half_w = half_h * aspect;

        // Distance that frames the bounding sphere at zoom 1, with a small margin.
        // Independent of zoom, so a telephoto zoom never enters the model. `dolly`
        // then scales this distance along the view axis: the one control that *does*
        // move the eye (toward the model at `dolly < 1`, away at `> 1`).
        let dist = radius / (vfov * 0.5).sin() * 1.05 * dolly;
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
            1.0,                        // dolly
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
    fn dolly_scales_eye_distance_without_moving_center_or_fov() {
        // Dolly moves the eye along the view axis toward/away from the pivot,
        // leaving the pivot and the field of view untouched. A 0.5× dolly halves
        // the eye→center distance; 2× doubles it. (Contrast telephoto zoom, which
        // moves the eye not at all — see `zoom_narrows_the_fov...`.)
        let base = Camera::orbit(Vec3::new(10.0, 20.0, 5.0), 8.0, -0.785, 0.55, 1.0, 1.6, 36.0, 1.0);
        let close = Camera::orbit(Vec3::new(10.0, 20.0, 5.0), 8.0, -0.785, 0.55, 1.0, 1.6, 36.0, 0.5);
        let far = Camera::orbit(Vec3::new(10.0, 20.0, 5.0), 8.0, -0.785, 0.55, 1.0, 1.6, 36.0, 2.0);

        // Pivot and FOV are invariant under dolly.
        assert_eq!(base.center, close.center);
        assert_eq!(base.center, far.center);
        assert!((base.half_h - close.half_h).abs() < 1e-6);
        assert!((base.half_w - close.half_w).abs() < 1e-6);

        let dist = |c: &Camera| (c.center - c.eye).length();
        let d_base = dist(&base);
        assert!(
            (dist(&close) - d_base * 0.5).abs() < 1e-3,
            "0.5× dolly should halve eye distance: base={d_base}, close={}",
            dist(&close)
        );
        assert!(
            (dist(&far) - d_base * 2.0).abs() < 1e-3,
            "2× dolly should double eye distance: base={d_base}, far={}",
            dist(&far)
        );

        // The eye still looks straight at the pivot at every dolly.
        let to_center = (close.center - close.eye).normalize();
        assert!((to_center - close.forward).length() < 1e-4);
    }

    #[test]
    fn zoom_narrows_the_fov_without_moving_the_camera() {
        // Telephoto zoom: the eye stays put (so the camera never dives into the
        // model) while the field of view narrows, magnifying the image uniformly
        // like the orthographic 3D view it must stay in sync with.
        let base = Camera::orbit(Vec3::ZERO, 5.0, 0.0, 0.4, 1.0, 1.0, 40.0, 1.0);
        let zoomed = Camera::orbit(Vec3::ZERO, 5.0, 0.0, 0.4, 2.0, 1.0, 40.0, 1.0);
        assert!(
            (zoomed.eye - base.eye).length() < 1e-5,
            "zoom must not move the eye: base={:?} zoomed={:?}",
            base.eye,
            zoomed.eye
        );
        // 2× zoom halves the image-plane half-extents, so objects appear 2× larger.
        assert!((base.half_h / zoomed.half_h - 2.0).abs() < 1e-4);
        assert!((base.half_w / zoomed.half_w - 2.0).abs() < 1e-4);
    }
}
