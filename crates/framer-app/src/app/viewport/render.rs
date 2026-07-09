//! The path-traced Render view: orbit/pan/dolly/zoom input, the GPU-compute path
//! tracer (with CPU-thread fallback), camera-motion resolution hysteresis, and the
//! progress readout. This is the one renderer that reads/writes `FramerApp` fields
//! directly, so it stays an `impl FramerApp` method.
//!
//! `crate::app::render` (the path tracer) is aliased `path_render` here so it does
//! not shadow this `viewport::render` module.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, StrokeKind, Ui, Vec2};

use super::theme;
use super::view_common::{
    draw_view_background, draw_view_border, draw_view_empty, render_resolution, viewport_size,
};
use crate::app::design::text_size;
use crate::app::render as path_render;
use crate::app::{FramerApp, render_job};

/// Frames to stay in reduced-resolution "moving" mode after the last camera
/// input, so a continuous orbit (which produces frequent tiny inputs) doesn't
/// flicker between resolution modes.
const MOTION_COOLDOWN_FRAMES: u32 = 6;
/// Internal-resolution scale for the Render view while the camera is moving
/// (0.5 ⇒ quarter the pixels, ~4× faster per frame).
const MOTION_RESOLUTION_SCALE: f32 = 0.5;

fn render_progress_chip_rect(drawing: Rect, label: &str) -> Rect {
    let width = (label.chars().count() as f32 * 6.6 + 18.0).clamp(118.0, 260.0);
    Rect::from_min_size(
        drawing.left_bottom() + Vec2::new(8.0, -30.0),
        Vec2::new(width, 22.0),
    )
}

fn draw_render_progress_chip(painter: &egui::Painter, drawing: Rect, label: &str) {
    let chip = render_progress_chip_rect(drawing, label);
    painter.rect_filled(chip, 4.0, theme::overlay());
    painter.rect_stroke(chip, 4.0, theme::soft_stroke(), StrokeKind::Outside);
    painter.text(
        chip.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(text_size::LABEL),
        theme::text_primary(),
    );
}

fn render_progress_label(renderer: &str, samples: u32, target: u32, accumulating: bool) -> String {
    if accumulating {
        format!("{renderer} — {samples}/{target} spp")
    } else {
        format!("{renderer} complete — {samples} spp")
    }
}

impl FramerApp {
    // === method body appended below; super:: paths rewritten ===
    /// Draws the path-traced Render view. Geometry, materials, and lighting come
    /// from `framer-render`; the heavy work runs on a background thread
    /// ([`super::render_job`]) and refines progressively while the camera is still.
    pub(super) fn draw_project_render(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let desired = viewport_size(ui);
        let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        draw_view_background(&painter, rect, theme::sheet());
        let drawing = rect.shrink(1.0);
        draw_view_border(&painter, drawing);

        // Orbit / pan / dolly / telephoto zoom, mirroring the 3D workspace controls.
        // Left-drag orbits; middle-drag or Shift+left-drag pans; the wheel dollies
        // the eye in and out; Cmd+wheel (or a trackpad pinch) is telephoto zoom.
        let shift = ui.input(|input| input.modifiers.shift);
        let primary_drag = response.dragged_by(egui::PointerButton::Primary);
        let middle_drag = response.dragged_by(egui::PointerButton::Middle);
        let orbiting = primary_drag && !shift;
        let panning = middle_drag || (primary_drag && shift);
        if orbiting {
            self.view_3d.orbit(response.drag_delta());
        }
        if panning {
            self.view_3d.pan(response.drag_delta(), drawing.height());
        }
        let mut zooming = false;
        let mut dollying = false;
        if response.hovered() {
            let (scroll_y, pinch, cmd) = ui.input(|input| {
                (
                    input.smooth_scroll_delta.y,
                    input.zoom_delta(),
                    input.modifiers.command,
                )
            });
            // Plain wheel/two-finger scroll dollies the eye; a pinch gesture or
            // Cmd+wheel is telephoto (lens) zoom, kept off the plain wheel.
            let telephoto = (pinch - 1.0).abs() > f32::EPSILON || cmd;
            if telephoto {
                let zoom_factor = pinch * (scroll_y * 0.002).exp();
                if (zoom_factor - 1.0).abs() > f32::EPSILON {
                    self.view_3d.zoom_by(zoom_factor);
                    zooming = true;
                }
            } else if scroll_y.abs() > f32::EPSILON {
                // Scroll up (positive) moves the eye closer, so dolly < 1.
                self.view_3d.dolly_by((-scroll_y * 0.0015).exp());
                dollying = true;
            }
        }
        // Camera-motion hysteresis: while interacting (plus a short cooldown so a
        // continuous orbit doesn't flicker between modes) render at a lower
        // internal resolution to keep orbiting responsive; the denoiser keeps the
        // resulting low-sample preview clean, and the still frame returns to full
        // resolution and converges to the unbiased result.
        if orbiting || panning || zooming || dollying {
            self.render_motion_cooldown = MOTION_COOLDOWN_FRAMES;
        } else {
            self.render_motion_cooldown = self.render_motion_cooldown.saturating_sub(1);
        }
        let moving = self.render_motion_cooldown > 0;

        if self.model.walls.is_empty() {
            draw_view_empty(&painter, drawing, "No geometry to render");
            return;
        }

        // Internal render resolution: device pixels, aspect-preserving and bounded
        // (see `render_resolution`), scaled down while the camera moves so orbiting
        // stays responsive; a settled frame returns to native resolution and
        // converges crisp instead of being nearest-upscaled from a sub-native cap.
        let ppp = ui.ctx().pixels_per_point();
        let res_scale = if moving { MOTION_RESOLUTION_SCALE } else { 1.0 };
        let (width, height) = render_resolution(drawing.width(), drawing.height(), ppp, res_scale);

        let mut opts = framer_render::RenderOptions {
            yaw: self.view_3d.yaw,
            pitch: self.view_3d.pitch,
            zoom: self.view_3d.zoom,
            pan: self.view_3d.pan,
            dolly: self.view_3d.dolly,
            aspect: width as f32 / height as f32,
            ..framer_render::RenderOptions::default()
        };
        self.render_settings.apply_to_options(&mut opts);

        // Prefer the real-time GPU compute path tracer; fall back to the
        // background-thread CPU renderer when compute isn't available.
        let (samples, target, accumulating, renderer) =
            if let (true, Some(format)) = (self.gpu_compute_ok, self.gpu_target_format) {
                let backend = path_render::PathTraceBackend::from_config(
                    self.gpu_ray_query_ok,
                    self.config.render.ray_query,
                );
                let prepared = path_render::paint(
                    &mut self.render_gpu,
                    &painter,
                    drawing,
                    &self.model,
                    &opts,
                    width,
                    height,
                    moving,
                    format,
                    backend,
                );
                if !prepared {
                    draw_view_empty(&painter, drawing, "Preparing render…");
                }
                (
                    self.render_gpu.samples(),
                    self.render_gpu.target_spp(),
                    self.render_gpu.is_accumulating(),
                    backend.label(),
                )
            } else {
                // Reuse the GPU path's accumulation key so the CPU fallback resets
                // on exactly the same camera/geometry/size changes (incl. pan/dolly).
                let key = path_render::accumulation_key(
                    render_job::model_signature(&self.model),
                    &opts,
                    width,
                    height,
                );

                self.render_view
                    .update(&ctx, &self.model, opts, width, height, key);

                if let Some(texture) = self.render_view.texture() {
                    let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                    painter.image(texture.id(), drawing, uv, Color32::WHITE);
                } else {
                    draw_view_empty(&painter, drawing, "Preparing render…");
                }
                (
                    self.render_view.samples(),
                    self.render_view.target_spp(),
                    self.render_view.is_accumulating(),
                    "CPU",
                )
            };

        // Progress / quality readout.
        let label = render_progress_label(renderer, samples, target, accumulating);
        draw_render_progress_chip(&painter, drawing, &label);

        // Keep refining until converged, while interacting, or while the motion
        // cooldown is still ticking down (so it can settle back to full resolution).
        if accumulating || response.dragged() || moving {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_progress_chip_stays_inside_the_drawing_rect() {
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        let chip = render_progress_chip_rect(drawing, "GPU ray query — 128/256 spp");

        assert!(drawing.contains(chip.left_top()));
        assert!(drawing.contains(chip.right_bottom()));
    }

    #[test]
    fn render_progress_chip_width_is_bounded_for_long_labels() {
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        let chip = render_progress_chip_rect(
            drawing,
            "Render complete — 123456789012345678901234567890 spp",
        );

        assert!(chip.width() <= 260.0);
        assert!(drawing.contains(chip.right_bottom()));
    }

    #[test]
    fn render_progress_label_names_renderer_backend() {
        assert_eq!(
            render_progress_label("GPU ray query", 128, 256, true),
            "GPU ray query — 128/256 spp"
        );
        assert_eq!(
            render_progress_label("GPU BVH", 256, 256, false),
            "GPU BVH complete — 256 spp"
        );
        assert_eq!(
            render_progress_label("CPU", 12, 256, true),
            "CPU — 12/256 spp"
        );
    }
}
