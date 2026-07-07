//! View-frame utilities reused by every viewport renderer: pane sizing, the
//! drawing-rect inset, render resolution, background/border/title/empty states,
//! the drafting grid + rulers, the plan axis indicator, the scale bar, a
//! dashed-line helper, and the per-wall elevation layout frame.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2};
use framer_core::{Opening, Wall};

use super::camera_2d::View2dState;
use super::theme;

// === extracted ranges appended below; visibility adjusted in place ===

/// Smallest internal render dimension (device pixels) on either axis: floors the
/// resolution for very small panes so the accumulator is never degenerate.
pub(super) const MIN_RENDER_DIM: u32 = 64;
/// Largest internal render dimension (device pixels) on either axis. Caps GPU
/// cost and accumulator memory (four f32×4 buffers ⇒ ~270 MB at the limit).
/// Raised from the prior 1000 px so a settled frame on a hi-DPI display renders
/// at (or near) native resolution instead of being nearest-upscaled by the blit
/// — the upscale was the cause of jagged/soft edges on a stationary camera.
pub(super) const MAX_RENDER_DIM: u32 = 2048;

pub(super) fn viewport_size(ui: &Ui) -> Vec2 {
    // Fill the central panel so the drawing surface reaches the panel edges; the
    // plan/3D projectors letterbox their content within this rect.
    let available = ui.available_size();
    Vec2::new(available.x.max(420.0), available.y.max(360.0))
}

pub(super) fn viewport_drawing_rect(rect: Rect, margin: f32) -> Rect {
    Rect::from_min_max(
        rect.min + Vec2::splat(margin),
        rect.max - Vec2::new(margin, margin),
    )
}

/// Internal render resolution (device pixels) for a drawing rect of logical size
/// `rect_w` × `rect_h`, at `pixels_per_point` `ppp` and motion scale `res_scale`
/// (1.0 when the camera is still, `MOTION_RESOLUTION_SCALE` while orbiting).
///
/// Both axes are scaled by a single factor so the rect's aspect ratio is always
/// preserved — a tall pane stays tall — instead of clamping each axis on its own
/// (which squished portrait/landscape panes toward square). The factor floors the
/// short axis at `MIN_RENDER_DIM` and caps the long axis at `MAX_RENDER_DIM`; the
/// cap wins if the two bounds ever conflict on a degenerate sliver.
pub(super) fn render_resolution(rect_w: f32, rect_h: f32, ppp: f32, res_scale: f32) -> (u32, u32) {
    // Target device-pixel size of the drawing rect.
    let dw = (rect_w * ppp * res_scale).max(1.0);
    let dh = (rect_h * ppp * res_scale).max(1.0);
    let long = dw.max(dh);
    let short = dw.min(dh);
    // A single uniform factor keeps the aspect ratio intact: raise it to lift the
    // short axis to MIN_RENDER_DIM, then cap it so the long axis never exceeds
    // MAX_RENDER_DIM. `.min` is applied last, so the cost cap wins if the floor
    // and cap conflict (extreme aspect ratios).
    let scale = (MIN_RENDER_DIM as f32 / short)
        .max(1.0)
        .min(MAX_RENDER_DIM as f32 / long);
    let w = (dw * scale).round().max(1.0) as u32;
    let h = (dh * scale).round().max(1.0) as u32;
    (w, h)
}

pub(super) fn draw_view_title(painter: &egui::Painter, drawing: Rect, title: impl Into<String>) {
    painter.text(
        drawing.left_top() + Vec2::new(0.0, -20.0),
        Align2::LEFT_CENTER,
        title.into(),
        FontId::proportional(13.0),
        theme::framing_line_dark(),
    );
}

pub(super) fn draw_view_empty(painter: &egui::Painter, rect: Rect, label: &str) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(14.0),
        theme::text_muted(),
    );
}

pub(super) fn draw_view_border(painter: &egui::Painter, drawing: Rect) {
    painter.rect_stroke(
        drawing,
        0.0,
        Stroke::new(1.0, theme::sheet_grid_major()),
        StrokeKind::Outside,
    );
}

pub(super) fn draw_view_background(painter: &egui::Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect, 0.0, color);
}

pub(super) fn draw_drafting_grid(painter: &egui::Painter, drawing: Rect) {
    let minor = 24.0;
    let major_every = 5;
    let mut index = 0;
    let mut x = drawing.left();
    while x <= drawing.right() {
        let color = if index % major_every == 0 {
            theme::sheet_grid_major()
        } else {
            theme::sheet_grid()
        };
        painter.line_segment(
            [Pos2::new(x, drawing.top()), Pos2::new(x, drawing.bottom())],
            Stroke::new(0.6, color),
        );
        x += minor;
        index += 1;
    }

    index = 0;
    let mut y = drawing.top();
    while y <= drawing.bottom() {
        let color = if index % major_every == 0 {
            theme::sheet_grid_major()
        } else {
            theme::sheet_grid()
        };
        painter.line_segment(
            [Pos2::new(drawing.left(), y), Pos2::new(drawing.right(), y)],
            Stroke::new(0.6, color),
        );
        y += minor;
        index += 1;
    }
}

pub(super) fn draw_drafting_rulers(painter: &egui::Painter, rect: Rect, drawing: Rect) {
    let top_ruler = Rect::from_min_max(
        Pos2::new(drawing.left(), rect.top()),
        Pos2::new(drawing.right(), drawing.top()),
    );
    let left_ruler = Rect::from_min_max(
        Pos2::new(rect.left(), drawing.top()),
        Pos2::new(drawing.left(), drawing.bottom()),
    );
    painter.rect_filled(top_ruler, 0.0, theme::sheet_ruler());
    painter.rect_filled(left_ruler, 0.0, theme::sheet_ruler());

    let tick = theme::text_muted();
    let minor = 24.0;
    let mut index = 0;
    let mut x = drawing.left();
    while x <= drawing.right() {
        let major = index % 5 == 0;
        let y0 = if major {
            top_ruler.bottom() - 12.0
        } else {
            top_ruler.bottom() - 6.0
        };
        painter.line_segment(
            [Pos2::new(x, y0), Pos2::new(x, top_ruler.bottom())],
            Stroke::new(0.75, tick),
        );
        if major {
            painter.text(
                Pos2::new(x + 3.0, top_ruler.bottom() - 18.0),
                Align2::LEFT_CENTER,
                format!("{}'", index * 2),
                FontId::proportional(10.0),
                tick,
            );
        }
        x += minor;
        index += 1;
    }

    index = 0;
    let mut y = drawing.bottom();
    while y >= drawing.top() {
        let major = index % 5 == 0;
        let x0 = if major {
            left_ruler.right() - 12.0
        } else {
            left_ruler.right() - 6.0
        };
        painter.line_segment(
            [Pos2::new(x0, y), Pos2::new(left_ruler.right(), y)],
            Stroke::new(0.75, tick),
        );
        if major {
            painter.text(
                Pos2::new(left_ruler.right() - 16.0, y - 2.0),
                Align2::RIGHT_CENTER,
                format!("{}'", index * 2),
                FontId::proportional(10.0),
                tick,
            );
        }
        y -= minor;
        index += 1;
    }
}

pub(super) fn draw_plan_axis_indicator(painter: &egui::Painter, rect: Rect) {
    let origin = rect.left_bottom() + Vec2::new(36.0, -62.0);
    let x_end = origin + Vec2::new(30.0, 0.0);
    let y_end = origin + Vec2::new(0.0, -30.0);
    painter.line_segment([origin, x_end], Stroke::new(1.5, theme::danger()));
    painter.line_segment([origin, y_end], Stroke::new(1.5, theme::success()));
    painter.circle_filled(origin, 4.0, theme::active_blue());
    painter.text(
        x_end + Vec2::new(7.0, 0.0),
        Align2::LEFT_CENTER,
        "X",
        FontId::proportional(11.0),
        theme::framing_line_dark(),
    );
    painter.text(
        y_end + Vec2::new(0.0, -7.0),
        Align2::CENTER_BOTTOM,
        "Y",
        FontId::proportional(11.0),
        theme::framing_line_dark(),
    );
}

/// Draw a dashed line `a`→`b`. egui has no stable dashed primitive we rely on
/// here, so we emit short segments with gaps.
pub(super) fn draw_dashed_line(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    const DASH: f32 = 6.0;
    const GAP: f32 = 4.0;
    let span = b - a;
    let len = span.length();
    if len < 1.0 {
        return;
    }
    let dir = span / len;
    let mut t = 0.0;
    while t < len {
        let start = a + dir * t;
        let end = a + dir * (t + DASH).min(len);
        painter.line_segment([start, end], stroke);
        t += DASH + GAP;
    }
}

/// A drafting scale bar at the bottom-left of the plan, sized to a round number
/// of feet given the current pixels-per-inch `scale`.
pub(super) fn draw_scale_bar(painter: &egui::Painter, drawing: Rect, scale: f32) {
    let mut feet = 1.0_f32;
    for candidate in [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0] {
        if candidate * 12.0 * scale <= 96.0 {
            feet = candidate;
        }
    }
    let bar = feet * 12.0 * scale;
    let y = drawing.bottom() - 14.0;
    let x0 = drawing.left() + 86.0;
    let x1 = x0 + bar;
    let ink = theme::framing_line_dark();
    painter.line_segment([Pos2::new(x0, y), Pos2::new(x1, y)], Stroke::new(2.0, ink));
    for x in [x0, (x0 + x1) / 2.0, x1] {
        painter.line_segment(
            [Pos2::new(x, y - 4.0), Pos2::new(x, y + 4.0)],
            Stroke::new(1.5, ink),
        );
    }
    painter.text(
        Pos2::new(x0, y + 6.0),
        Align2::CENTER_TOP,
        "0",
        FontId::proportional(9.5),
        theme::text_muted(),
    );
    painter.text(
        Pos2::new(x1, y + 6.0),
        Align2::CENTER_TOP,
        format!("{feet:.0}'"),
        FontId::proportional(9.5),
        theme::text_muted(),
    );
}

pub(super) fn pointer_started_in_rect(press_origin: Option<Pos2>, rect: Rect) -> bool {
    press_origin.is_some_and(|origin| rect.contains(origin))
}

#[derive(Clone, Copy)]
pub(super) struct WallElevationLayout {
    pub(super) wall_rect: Rect,
    pub(super) scale: f32,
}

impl WallElevationLayout {
    pub(super) fn new(available: Rect, wall: &Wall, view: &View2dState) -> Self {
        let wall_width = wall.length.inches().max(1.0) as f32;
        let wall_height = wall.height.inches().max(1.0) as f32;
        let base_scale = (available.width() / wall_width)
            .min(available.height() / wall_height)
            .max(0.001);
        // Fold the camera into the two fields every elevation draw/hit-test
        // derives from: zoom scales the pixels-per-inch, and the wall rect's
        // center is panned/zoomed via the shared `apply` transform. This is
        // exactly equivalent to running `view.apply` on every drawn point.
        let scale = base_scale * view.zoom;
        let wall_size = Vec2::new(wall_width * scale, wall_height * scale);
        let center = view.apply(available.center(), available);
        Self {
            wall_rect: Rect::from_center_size(center, wall_size),
            scale,
        }
    }
}

/// An opening's screen rect in elevation space (pixels-per-inch `sx`/`sy`).
pub(super) fn opening_rect(drawing: Rect, sx: f32, sy: f32, opening: &Opening) -> Rect {
    let x = drawing.left() + opening.left().inches() as f32 * sx;
    let y = drawing.bottom() - opening.top().inches() as f32 * sy;
    let width = (opening.width.inches() as f32 * sx).max(4.0);
    let height = (opening.height.inches() as f32 * sy).max(4.0);
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, height))
}
