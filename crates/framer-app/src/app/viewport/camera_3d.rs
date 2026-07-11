//! 3D orbit camera state for the Axonometric and Render views, plus the
//! view-cube orientation/snap model the camera snaps to.
//!
//! `View3dState` is pure presentation state (yaw/pitch/zoom/pan/dolly): never
//! serialized, untouched by undo/redo.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use eframe::egui::Vec2;
use framer_geometry::Aabb;
use framer_render::math::Vec3;

use super::geom::Point3;

/// World-height (in framing radii) visible across a full-viewport span at zoom 1.
/// Sets the pan rate so a full-viewport drag slides the view by ~one model height
/// and the point under the cursor tracks the cursor closely. Derived from the
/// render camera's framing (`2 · 1.05 / cos(vfov/2)` ≈ 2.2 at the default 36° FOV).
const PAN_RADII_PER_VIEWPORT: f32 = 2.1;
/// Clamp on the pan offset's length (radius units), so the pivot can't drift so
/// far the model is lost off-screen. The view-cube "Home" recenters.
pub(super) const PAN_MAX_RADII: f32 = 20.0;
/// Dolly bounds. The minimum lets the eye move well inside the building (the path
/// tracer renders fine from inside geometry); the maximum pulls comfortably back.
pub(super) const DOLLY_MIN: f32 = 0.05;
pub(super) const DOLLY_MAX: f32 = 6.0;

#[derive(Debug, Clone, Copy)]
pub(crate) struct View3dState {
    pub(super) yaw: f32,
    pub(super) pitch: f32,
    pub(super) zoom: f32,
    /// Orbit-pivot offset in radius-relative world units (see [`RenderOptions::pan`]).
    pub(super) pan: Vec3,
    /// Eye-distance multiplier for the perspective Render view (the orthographic
    /// 3D workspace ignores it — it has no eye distance to change).
    pub(super) dolly: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewCubeAction {
    Home,
    Snap(ViewCubeOrientation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ViewCubeOrientation {
    x: i8,
    y: i8,
    z: i8,
}

impl ViewCubeOrientation {
    pub(super) const TOP: Self = Self { x: 0, y: 0, z: 1 };
    pub(super) const BOTTOM: Self = Self { x: 0, y: 0, z: -1 };
    pub(super) const FRONT: Self = Self { x: 0, y: 1, z: 0 };
    pub(super) const BACK: Self = Self { x: 0, y: -1, z: 0 };
    pub(super) const RIGHT: Self = Self { x: 1, y: 0, z: 0 };
    pub(super) const LEFT: Self = Self { x: -1, y: 0, z: 0 };

    pub(super) fn new(x: i8, y: i8, z: i8) -> Self {
        Self { x, y, z }
    }

    pub(super) fn from_point(point: Point3) -> Self {
        Self::new(
            point.x.signum() as i8,
            point.y.signum() as i8,
            point.z.signum() as i8,
        )
    }

    pub(super) fn from_points(start: Point3, end: Point3) -> Self {
        let component = |left: f32, right: f32| {
            if (left - right).abs() <= f32::EPSILON {
                left.signum() as i8
            } else {
                0
            }
        };
        Self::new(
            component(start.x, end.x),
            component(start.y, end.y),
            component(start.z, end.z),
        )
    }

    pub(super) fn component_count(self) -> usize {
        [self.x, self.y, self.z]
            .into_iter()
            .filter(|component| *component != 0)
            .count()
    }

    pub(super) fn includes_face(self, face: Self) -> bool {
        face.component_count() == 1
            && (face.x == 0 || self.x == face.x)
            && (face.y == 0 || self.y == face.y)
            && (face.z == 0 || self.z == face.z)
    }
}

impl ViewCubeAction {
    pub(super) const TOP: Self = Self::Snap(ViewCubeOrientation::TOP);
    pub(super) const BOTTOM: Self = Self::Snap(ViewCubeOrientation::BOTTOM);
    pub(super) const FRONT: Self = Self::Snap(ViewCubeOrientation::FRONT);
    pub(super) const BACK: Self = Self::Snap(ViewCubeOrientation::BACK);
    pub(super) const RIGHT: Self = Self::Snap(ViewCubeOrientation::RIGHT);
    pub(super) const LEFT: Self = Self::Snap(ViewCubeOrientation::LEFT);

    pub(super) fn snap(orientation: ViewCubeOrientation) -> Self {
        Self::Snap(orientation)
    }

    pub(super) fn orientation(self) -> Option<ViewCubeOrientation> {
        match self {
            Self::Home => None,
            Self::Snap(orientation) => Some(orientation),
        }
    }
}

impl Default for View3dState {
    fn default() -> Self {
        Self {
            yaw: -FRAC_PI_4,
            pitch: 0.55,
            zoom: 1.0,
            pan: Vec3::ZERO,
            dolly: 1.0,
        }
    }
}

impl View3dState {
    #[cfg(test)]
    pub(crate) fn roof_framing_detail_shot() -> Self {
        Self {
            zoom: 3.0,
            pan: Vec3::new(0.0, 0.0, 0.28),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn roof_framing_eave_detail_shot() -> Self {
        Self {
            yaw: -FRAC_PI_2,
            pitch: 0.05,
            zoom: 3.0,
            pan: Vec3::new(0.0, -0.55, 0.16),
            ..Self::default()
        }
    }

    pub(super) fn orbit(&mut self, delta: Vec2) {
        self.yaw += delta.x * 0.01;
        self.pitch = (self.pitch - delta.y * 0.01).clamp(-FRAC_PI_2 + 0.02, FRAC_PI_2 - 0.02);
    }

    pub(super) fn zoom_by(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(0.35, 3.0);
        }
    }

    pub(in crate::app) fn frame_bounds(&mut self, scene: Aabb, focus: Aabb) {
        let center = |bounds: Aabb| {
            Vec3::new(
                ((bounds.min.x + bounds.max.x) * 0.5) as f32,
                ((bounds.min.y + bounds.max.y) * 0.5) as f32,
                ((bounds.min.z + bounds.max.z) * 0.5) as f32,
            )
        };
        let radius = |bounds: Aabb| {
            let dx = (bounds.max.x - bounds.min.x) as f32;
            let dy = (bounds.max.y - bounds.min.y) as f32;
            let dz = (bounds.max.z - bounds.min.z) as f32;
            (dx * dx + dy * dy + dz * dz).sqrt() * 0.5
        };
        let scene_radius = radius(scene).max(1.0);
        let focus_radius = radius(focus).max(1.0);
        let scene_center = center(scene);
        let focus_center = center(focus);
        self.pan = (focus_center - scene_center) * (1.0 / scene_radius);
        self.zoom = (scene_radius / focus_radius * 0.72).clamp(0.35, 3.0);
        self.dolly = 1.0;
    }

    /// The camera's world-space screen basis (`right`, `up`) for the current yaw
    /// and pitch — the exact vectors `framer_render::camera::Camera::orbit` uses,
    /// so panning slides the pivot along the same axes the Render view shows.
    pub(super) fn screen_basis(&self) -> (Vec3, Vec3) {
        let (sin_y, cos_y) = (self.yaw.sin(), self.yaw.cos());
        let (sin_p, cos_p) = (self.pitch.sin(), self.pitch.cos());
        let right = Vec3::new(cos_y, sin_y, 0.0);
        let up = Vec3::new(sin_y * sin_p, -cos_y * sin_p, cos_p);
        (right, up)
    }

    /// Slides the orbit pivot in the camera's screen plane by a pointer `delta`
    /// (egui points; y grows downward), over a viewport of `viewport_span` points.
    /// "Grab-the-scene": the world point under the cursor tracks the cursor, so the
    /// pivot moves opposite the drag horizontally and with it vertically. The rate
    /// is radius-relative and scales inversely with telephoto zoom, so a drag pans
    /// the same fraction of the framed model at any model scale or zoom.
    pub(super) fn pan(&mut self, delta: Vec2, viewport_span: f32) {
        if viewport_span <= f32::EPSILON || (delta.x == 0.0 && delta.y == 0.0) {
            return;
        }
        let (right, up) = self.screen_basis();
        let per_px = PAN_RADII_PER_VIEWPORT / self.zoom.max(1.0e-3) / viewport_span;
        self.pan = self.pan - right * (delta.x * per_px) + up * (delta.y * per_px);
        let len = self.pan.length();
        if len > PAN_MAX_RADII {
            self.pan = self.pan * (PAN_MAX_RADII / len);
        }
    }

    /// Dollies the eye toward (`factor < 1`) or away from (`> 1`) the pivot,
    /// multiplicatively, clamped to `[DOLLY_MIN, DOLLY_MAX]`. Non-finite or
    /// non-positive factors are ignored.
    pub(super) fn dolly_by(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.dolly = (self.dolly * factor).clamp(DOLLY_MIN, DOLLY_MAX);
        }
    }

    pub(super) fn snap_to(&mut self, action: ViewCubeAction) {
        match action {
            ViewCubeAction::Home => *self = Self::default(),
            ViewCubeAction::Snap(orientation) => {
                let x = orientation.x as f32;
                let y = orientation.y as f32;
                let z = orientation.z as f32;
                let horizontal = (x * x + y * y).sqrt();
                self.pitch = z.atan2(horizontal);
                if horizontal > f32::EPSILON {
                    self.yaw = (-x / horizontal).atan2(y / horizontal);
                } else {
                    self.yaw = 0.0;
                }
                // Re-frame the model: a face snap clears any accumulated pan/dolly
                // so the snapped view is centered and at the framing distance.
                self.pan = Vec3::ZERO;
                self.dolly = 1.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use framer_geometry::Point3 as PhysicalPoint3;

    use super::*;

    #[test]
    fn frame_bounds_centers_and_enlarges_the_focused_pair() {
        let scene = Aabb {
            min: PhysicalPoint3::new(0.0, 0.0, 0.0),
            max: PhysicalPoint3::new(200.0, 100.0, 100.0),
        };
        let focus = Aabb {
            min: PhysicalPoint3::new(140.0, 40.0, 40.0),
            max: PhysicalPoint3::new(160.0, 60.0, 60.0),
        };
        let mut view = View3dState::default();

        view.frame_bounds(scene, focus);

        assert!(view.pan.x > 0.3, "focus center should pan toward +X");
        assert!(view.pan.y.abs() < 1.0e-6);
        assert!(view.pan.z.abs() < 1.0e-6);
        assert!(view.zoom > 1.0);
        assert_eq!(view.dolly, 1.0);
    }
}
