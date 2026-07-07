//! 2D pan/zoom camera for the Plan and Elevation views.
//!
//! `View2dState` is pure presentation state layered on top of each view's
//! fit-to-bounds base transform: never serialized, untouched by undo/redo.

use eframe::egui::{self, CursorIcon, Pos2, Rect, Ui, Vec2};

/// Zoom bounds for the 2D Plan/Elevation camera (1.0 == fit-to-bounds).
const ZOOM_MIN_2D: f32 = 0.2;
const ZOOM_MAX_2D: f32 = 40.0;
/// Pan clamp, as a fraction of the viewport half-extent *per unit zoom*. Scaling
/// with `zoom.max(1.0)` keeps the clamp from fighting a cursor-anchored zoom into
/// a corner (whose required pan grows with zoom), while still bounding pan at fit
/// (zoom 1) so the drawing can't be flung off-screen with no way back.
/// `F` / double-click always recenter regardless.
const PAN_LIMIT_FACTOR_2D: f32 = 1.0;

/// Pan / zoom camera for the 2D Plan ("shell") and Elevation ("wall") views,
/// layered on top of a view's fit-to-bounds base transform. Pure presentation
/// state: never serialized, untouched by undo/redo. [`Default`] is the identity
/// transform — pixel-identical to the historical fit-to-bounds framing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct View2dState {
    /// Multiplicative scale on top of the fit-to-bounds base. 1.0 == fit.
    /// Read directly by plan/elevation drawing within the viewport module tree.
    pub(super) zoom: f32,
    /// Screen-space offset (egui points), anchored at the viewport center.
    pan: Vec2,
}

impl Default for View2dState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

impl View2dState {
    /// Display percentage for the live view scale. The camera's stored zoom is a
    /// multiplier on the fit-to-bounds transform, so 1.0 displays as 100%.
    pub(crate) fn zoom_percent(&self) -> u32 {
        (self.zoom * 100.0).round() as u32
    }

    /// Maps a base (fit-to-bounds) screen point to its final on-screen position:
    /// `c + (base − c)·zoom + pan`, anchored at the viewport center `c`.
    pub(super) fn apply(&self, base: Pos2, drawing: Rect) -> Pos2 {
        let c = drawing.center();
        c + (base - c) * self.zoom + self.pan
    }

    /// Inverse of [`apply`]: maps a final on-screen point back to its base
    /// (fit-to-bounds) coordinate, for hit-testing and cursor→model mapping.
    pub(super) fn unapply(&self, screen: Pos2, drawing: Rect) -> Pos2 {
        let c = drawing.center();
        c + (screen - c - self.pan) / self.zoom
    }

    /// Slides the view by a screen-space `delta` (egui points), then clamps.
    fn pan_by(&mut self, delta: Vec2, drawing: Rect) {
        self.pan += delta;
        self.clamp_pan(drawing);
    }

    /// Zooms by `factor`, keeping the model point under `cursor` fixed. Clamps
    /// zoom to `[ZOOM_MIN_2D, ZOOM_MAX_2D]` and re-derives the applied factor so
    /// the cursor stays pinned even at the limits. Non-finite/non-positive
    /// factors are ignored.
    fn zoom_at(&mut self, cursor: Pos2, drawing: Rect, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let new_zoom = (self.zoom * factor).clamp(ZOOM_MIN_2D, ZOOM_MAX_2D);
        // Re-derive the *applied* factor after clamping so the anchor formula
        // below stays exact even when the requested zoom is past a limit.
        let applied = new_zoom / self.zoom;
        // Keep the point under `cursor` fixed: pan' = (q−c)(1−f) + pan·f.
        let offset = cursor - drawing.center();
        self.pan = offset * (1.0 - applied) + self.pan * applied;
        self.zoom = new_zoom;
        self.clamp_pan(drawing);
    }

    /// Per-axis pan clamp: `|pan| ≤ half-extent · PAN_LIMIT_FACTOR_2D · zoom.max(1)`.
    fn clamp_pan(&mut self, drawing: Rect) {
        let scale = PAN_LIMIT_FACTOR_2D * self.zoom.max(1.0);
        let max_x = drawing.width() * 0.5 * scale;
        let max_y = drawing.height() * 0.5 * scale;
        self.pan.x = self.pan.x.clamp(-max_x, max_x);
        self.pan.y = self.pan.y.clamp(-max_y, max_y);
    }

    /// Resets to the fit-to-bounds default (zoom 1, no pan).
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Applies design-tool pan/zoom navigation to a 2D view's `camera`, returning
/// `true` when this frame is a pan gesture so the caller suppresses selection /
/// handle interaction. Bindings: two-finger scroll pans; pinch or Cmd+scroll
/// zooms anchored at the cursor; middle-drag or Space+left-drag pans;
/// double-click, or `F` while hovered, re-fits. `allow_primary_pan` is `false`
/// while the view owns an in-progress primary-button drag (e.g. an opening
/// drag), so Space+drag can't hijack it.
pub(super) fn apply_view_2d_input(
    ui: &Ui,
    response: &egui::Response,
    drawing: Rect,
    camera: &mut View2dState,
    allow_primary_pan: bool,
) -> bool {
    // Re-fit to bounds with `F` while hovered (and not typing into a field).
    // Double-click-to-refit is handled by each view via `reset_view_on_empty_double_click`,
    // which only fires over empty canvas so it doesn't fight element selection.
    if response.hovered()
        && !ui.ctx().text_edit_focused()
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F))
    {
        camera.reset();
    }

    // Pan: middle-drag always; Space + primary-drag only when the view isn't
    // already dragging something with the primary button.
    let space_down = ui.input(|input| input.key_down(egui::Key::Space));
    let middle_drag = response.dragged_by(egui::PointerButton::Middle);
    let primary_drag = response.dragged_by(egui::PointerButton::Primary);
    let panning = middle_drag || (allow_primary_pan && space_down && primary_drag);
    if panning {
        camera.pan_by(response.drag_delta(), drawing);
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if space_down && response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }

    // Wheel / trackpad zoom + pan. egui (zoom_modifier = COMMAND) already folds a
    // trackpad pinch *and* a Cmd+scroll into `zoom_delta()`, zeroing
    // `smooth_scroll_delta` for that input — so routing purely off those two
    // keeps a zoom's scroll out of the pan path entirely (no smoothed-tail
    // drift): a zoom factor != 1 zooms at the cursor, otherwise a non-zero scroll
    // pans. Hover-guarded so it never hijacks global scroll.
    if response.hovered() && !panning {
        let (scroll, zoom) = ui.input(|input| (input.smooth_scroll_delta, input.zoom_delta()));
        if (zoom - 1.0).abs() > f32::EPSILON {
            let cursor = response.hover_pos().unwrap_or_else(|| drawing.center());
            camera.zoom_at(cursor, drawing, zoom);
        } else if scroll != Vec2::ZERO {
            // Plain two-finger scroll pans; the drawing tracks the fingers.
            camera.pan_by(scroll, drawing);
        }
    }

    panning
}

/// Re-fits the view (resets the camera) on a double-click over empty canvas.
/// `over_element` is true when the pointer is over a selectable element, so a
/// double-click that selects something doesn't also snap the view to fit.
pub(super) fn reset_view_on_empty_double_click(
    response: &egui::Response,
    camera: &mut View2dState,
    over_element: bool,
) {
    if response.double_clicked() && !over_element {
        camera.reset();
    }
}

#[cfg(test)]
mod view_2d_tests {
    use super::{PAN_LIMIT_FACTOR_2D, Pos2, Rect, Vec2, View2dState, ZOOM_MAX_2D, ZOOM_MIN_2D};

    /// A non-origin viewport, so a hard-coded `center` assumption can't slip
    /// through. center = (420, 330); half-extents = (400, 300).
    fn viewport() -> Rect {
        Rect::from_min_size(Pos2::new(20.0, 30.0), Vec2::new(800.0, 600.0))
    }

    fn close(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn zoom_percent_reports_fit_relative_scale() {
        assert_eq!(View2dState::default().zoom_percent(), 100);
        assert_eq!(
            View2dState {
                zoom: 2.5,
                pan: Vec2::ZERO,
            }
            .zoom_percent(),
            250
        );
        assert_eq!(
            View2dState {
                zoom: 0.375,
                pan: Vec2::ZERO,
            }
            .zoom_percent(),
            38
        );
    }

    #[test]
    fn default_state_is_identity() {
        let cam = View2dState::default();
        let d = viewport();
        for p in [
            Pos2::new(20.0, 30.0),
            Pos2::new(820.0, 630.0),
            Pos2::new(420.0, 330.0),
            Pos2::new(500.0, 100.0),
        ] {
            assert!(close(cam.apply(p, d), p), "apply identity at {p:?}");
            assert!(close(cam.unapply(p, d), p), "unapply identity at {p:?}");
        }
    }

    #[test]
    fn forward_inverse_round_trips() {
        let cam = View2dState {
            zoom: 2.5,
            pan: Vec2::new(40.0, -25.0),
        };
        let d = viewport();
        for p in [
            Pos2::new(120.0, 90.0),
            Pos2::new(700.0, 540.0),
            Pos2::new(420.0, 330.0),
        ] {
            let r = cam.unapply(cam.apply(p, d), d);
            assert!(close(r, p), "round trip {p:?} -> {r:?}");
        }
    }

    #[test]
    fn pan_by_shifts_output_by_delta() {
        let d = viewport();
        let mut cam = View2dState::default();
        let p = Pos2::new(300.0, 400.0);
        let before = cam.apply(p, d);
        cam.pan_by(Vec2::new(50.0, 30.0), d); // well below the clamp
        let after = cam.apply(p, d);
        assert!(close(after, before + Vec2::new(50.0, 30.0)));
    }

    #[test]
    fn zoom_at_keeps_cursor_model_point_fixed() {
        let d = viewport();
        let cursor = Pos2::new(560.0, 240.0);
        let mut cam = View2dState::default();
        let base_under_cursor = cam.unapply(cursor, d);
        cam.zoom_at(cursor, d, 2.0);
        let now = cam.apply(base_under_cursor, d);
        assert!(close(now, cursor), "cursor drifted: {now:?} vs {cursor:?}");
    }

    #[test]
    fn zoom_at_pins_cursor_and_saturates_at_max() {
        let d = viewport();
        let cursor = Pos2::new(470.0, 360.0); // modest offset from center
        let mut cam = View2dState::default();
        let base_under_cursor = cam.unapply(cursor, d);
        cam.zoom_at(cursor, d, 1000.0); // requests far past the max
        assert!(
            (cam.zoom - ZOOM_MAX_2D).abs() < 1e-4,
            "zoom should saturate at max, got {}",
            cam.zoom
        );
        let now = cam.apply(base_under_cursor, d);
        assert!(
            close(now, cursor),
            "cursor drifted at clamp: {now:?} vs {cursor:?}"
        );
    }

    #[test]
    fn zoom_at_saturates_at_min() {
        let d = viewport();
        let mut cam = View2dState::default();
        cam.zoom_at(d.center(), d, 0.0001);
        assert!((cam.zoom - ZOOM_MIN_2D).abs() < 1e-4, "got {}", cam.zoom);
    }

    #[test]
    fn zoom_at_ignores_non_positive_or_nan_factor() {
        let d = viewport();
        let mut cam = View2dState::default();
        cam.zoom_at(Pos2::new(500.0, 300.0), d, 0.0);
        cam.zoom_at(Pos2::new(500.0, 300.0), d, f32::NAN);
        assert!((cam.zoom - 1.0).abs() < 1e-6);
        assert_eq!(cam.pan, Vec2::ZERO);
    }

    #[test]
    fn pan_by_clamps_offset() {
        let d = viewport(); // 800 × 600
        let mut cam = View2dState::default(); // zoom 1
        cam.pan_by(Vec2::new(100_000.0, -100_000.0), d);
        // max = half-extent · PAN_LIMIT_FACTOR_2D · zoom.max(1) = (400, 300)
        let max_x = 400.0 * PAN_LIMIT_FACTOR_2D;
        let max_y = 300.0 * PAN_LIMIT_FACTOR_2D;
        assert!((cam.pan.x - max_x).abs() < 1e-4, "pan.x = {}", cam.pan.x);
        assert!((cam.pan.y + max_y).abs() < 1e-4, "pan.y = {}", cam.pan.y);
    }

    #[test]
    fn reset_returns_to_default() {
        let d = viewport();
        let mut cam = View2dState::default();
        cam.zoom_at(Pos2::new(500.0, 300.0), d, 3.0);
        cam.pan_by(Vec2::new(20.0, 10.0), d);
        cam.reset();
        assert_eq!(cam.zoom, 1.0);
        assert_eq!(cam.pan, Vec2::ZERO);
    }
}
