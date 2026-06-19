//! The generated-framing elevation renderer: lays out a wall's framing members as
//! rectangles, draws opening guides + the optional section line, and click-selects
//! members. Reuses the shared `WallElevationLayout` coordinate frame.

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use framer_core::{ConstructionSystem, LayerFunction, Length, Material, Wall};
use framer_solver::{FrameMember, MemberKind, MemberOrientation};

use super::camera_2d::{View2dState, apply_view_2d_input, reset_view_on_empty_double_click};
use super::elevation_openings::draw_opening_guide;
use super::scene_build::member_color;
use super::theme;
use super::view_common::{
    WallElevationLayout, draw_drafting_grid, draw_drafting_rulers, draw_view_border, opening_rect,
};
use crate::app::Selection;

// === extracted framing block appended below; visibility adjusted in place ===

pub(super) fn draw_wall_elevation(
    ui: &mut Ui,
    wall: &Wall,
    members: &[FrameMember],
    selected_member: Option<&str>,
    section_x: Option<Length>,
    system: Option<&ConstructionSystem>,
    materials: &[Material],
    camera: &mut View2dState,
) -> Option<String> {
    let available = ui.available_size();
    let desired = Vec2::new(available.x.max(420.0), (available.y - 16.0).max(420.0));
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    let margin = 52.0;
    let mut drawing = Rect::from_min_max(
        rect.min + Vec2::splat(margin),
        rect.max - Vec2::new(margin, margin),
    );
    // Reserve a band at the bottom for the build-up swatch (gap + bands + captions
    // + heading) so the wall plus swatch fit inside the original rect on square,
    // tall, or zoomed views instead of the swatch clipping past `rect.bottom()`.
    if system.is_some() {
        drawing.max.y -= SWATCH_RESERVE;
    }
    // Pan/zoom before laying out the wall. This view only click-selects members,
    // so Space+drag panning is always permitted.
    apply_view_2d_input(ui, &response, drawing, camera, true);
    let layout = WallElevationLayout::new(drawing, wall, camera);
    let wall_rect = layout.wall_rect;
    let scale = layout.scale;

    painter.rect_filled(rect, 0.0, theme::sheet());
    draw_drafting_rulers(&painter, rect, drawing);
    draw_drafting_grid(&painter, drawing);
    draw_view_border(&painter, drawing);
    let pointer = response.interact_pointer_pos();
    let mut clicked = None;
    let mut over_element = false;

    draw_opening_guides(&painter, wall_rect, scale, scale, wall);

    for member in members {
        let member_rect = member_rect(wall_rect, scale, scale, member);
        let hovered = pointer.is_some_and(|position| member_rect.contains(position));
        over_element |= hovered;
        let selected = selected_member == Some(member.id.as_str());
        draw_member_rect(&painter, member_rect, member.kind, selected, hovered);
        if hovered && response.clicked() {
            clicked = Some(member.id.clone());
        }
    }

    if let Some(section_x) = section_x {
        draw_section_line(&painter, wall_rect, scale, section_x);
    }

    draw_view_border(&painter, wall_rect);
    painter.text(
        Pos2::new(wall_rect.left(), wall_rect.bottom() + 20.0),
        Align2::LEFT_CENTER,
        format!("{} x {}", wall.length, wall.height),
        FontId::proportional(13.0),
        theme::framing_line_dark(),
    );

    // Build-up section swatch: a true-thickness layer strip below the dimension
    // text, anchored under the section line when a section cut is active.
    if let Some(system) = system {
        let swatch_left = section_x
            .map(|x| wall_rect.left() + x.inches() as f32 * scale)
            .unwrap_or(wall_rect.left());
        let anchor = Pos2::new(swatch_left, wall_rect.bottom() + 36.0);
        draw_section_swatch(&painter, anchor, system, materials, scale);
    }

    reset_view_on_empty_double_click(&response, camera, over_element);
    clicked
}

pub(super) fn member_rect(drawing: Rect, sx: f32, sy: f32, member: &FrameMember) -> Rect {
    let start_x = drawing.left() + member.x.inches() as f32 * sx;
    let start_y = drawing.bottom() - member.elevation.inches() as f32 * sy;

    match member.orientation {
        MemberOrientation::Horizontal => {
            let width = (member.cut_length.inches() as f32 * sx).max(2.0);
            let height = (member.cross_section_depth.inches() as f32 * sy).max(3.0);
            Rect::from_min_size(
                Pos2::new(start_x, start_y - height),
                Vec2::new(width, height),
            )
        }
        MemberOrientation::Vertical => {
            let width = (member.cross_section_depth.inches() as f32 * sx).max(3.0);
            let height = (member.cut_length.inches() as f32 * sy).max(2.0);
            Rect::from_min_size(
                Pos2::new(start_x - width / 2.0, start_y - height),
                Vec2::new(width, height),
            )
        }
    }
}

pub(super) fn draw_opening_guides(
    painter: &egui::Painter,
    drawing: Rect,
    sx: f32,
    sy: f32,
    wall: &Wall,
) {
    for opening in &wall.openings {
        draw_opening_guide(
            painter,
            opening_rect(drawing, sx, sy, opening),
            opening.kind,
            false,
            false,
        );
    }
}

pub(super) fn draw_member_rect(
    painter: &egui::Painter,
    rect: Rect,
    kind: MemberKind,
    selected: bool,
    hovered: bool,
) {
    painter.rect_filled(rect, 1.0, member_color(kind));
    let stroke = if selected {
        Stroke::new(2.0, Color32::from_rgb(34, 95, 155))
    } else if hovered {
        Stroke::new(1.5, Color32::from_rgb(40, 40, 40))
    } else {
        Stroke::new(0.75, Color32::from_rgb(87, 70, 52))
    };
    painter.rect_stroke(rect, 1.0, stroke, StrokeKind::Outside);
}

pub(super) fn draw_section_line(painter: &egui::Painter, drawing: Rect, sx: f32, x: Length) {
    let px = drawing.left() + x.inches() as f32 * sx;
    painter.line_segment(
        [
            Pos2::new(px, drawing.top()),
            Pos2::new(px, drawing.bottom()),
        ],
        Stroke::new(1.5, Color32::from_rgb(45, 91, 138)),
    );
    painter.text(
        Pos2::new(px + 5.0, drawing.top() + 14.0),
        Align2::LEFT_CENTER,
        "A-A",
        FontId::proportional(12.0),
        Color32::from_rgb(45, 91, 138),
    );
}

/// Vertical extent (in pixels) of the build-up section swatch strip.
const SWATCH_HEIGHT: f32 = 48.0;
/// Smallest drawn band width so a hairline layer stays visible.
const SWATCH_MIN_BAND: f32 = 5.0;
/// Vertical space reserved below the wall for the build-up swatch: the ~36 gap to
/// the strip, the `SWATCH_HEIGHT` (48) bands, ~30 of stacked captions, and the
/// heading above the strip. Subtracted from `drawing.max.y` when a swatch draws.
const SWATCH_RESERVE: f32 = 120.0;

/// Draw a true-thickness build-up strip for `system`: each layer is a horizontal
/// band (interior on the left -> exterior on the right) whose pixel width is its
/// real thickness `* scale`, filled with its material color and labeled with the
/// material name + thickness (and R-value when known). The framing band is
/// diagonally hatched to distinguish the framed cavity from solid layers.
pub(super) fn draw_section_swatch(
    painter: &egui::Painter,
    anchor: Pos2,
    system: &ConstructionSystem,
    materials: &[Material],
    scale: f32,
) {
    // Heading keyed to the section tag so it reads as the cut shown above.
    painter.text(
        Pos2::new(anchor.x, anchor.y - 4.0),
        Align2::LEFT_BOTTOM,
        format!("A-A  {}", system.total_thickness()),
        FontId::proportional(11.0),
        theme::framing_line_dark(),
    );

    let top = anchor.y;
    let bottom = top + SWATCH_HEIGHT;
    let mut x = anchor.x;
    for layer in &system.layers {
        let material = materials.iter().find(|m| m.id == layer.material);
        let color = material
            .map(|m| {
                let [r, g, b] = m.color();
                Color32::from_rgb(r, g, b)
            })
            .unwrap_or(Color32::from_gray(150));
        let width = (layer.thickness.inches() as f32 * scale).max(SWATCH_MIN_BAND);
        let band = Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + width, bottom));

        painter.rect_filled(band, 0.0, color);
        if layer.function == LayerFunction::Framing {
            draw_band_hatch(painter, band, hatch_color(color));
        }
        painter.rect_stroke(
            band,
            0.0,
            Stroke::new(0.75, Color32::from_rgb(60, 48, 36)),
            StrokeKind::Inside,
        );

        // Stacked caption below the band: material name, thickness, optional R.
        // Layer R uses the model's exact per-tick math (no inch rounding) so it
        // matches `ConstructionSystem::r_value_milli`.
        let name = material.map(|m| m.name.as_str()).unwrap_or("(missing)");
        let mut caption = format!("{name}\n{}", layer.thickness);
        let r_milli = material.map(|m| m.r_value_milli(layer.thickness)).unwrap_or(0);
        if r_milli > 0 {
            caption.push_str(&format!("  R{:.1}", r_milli as f32 / 1000.0));
        }
        painter.text(
            Pos2::new(x + width / 2.0, bottom + 3.0),
            Align2::CENTER_TOP,
            caption,
            FontId::proportional(9.0),
            theme::text_muted(),
        );

        x += width;
    }

    // Frame the whole build-up so partial/min-clamped bands still read as one wall.
    let outline = Rect::from_min_max(Pos2::new(anchor.x, top), Pos2::new(x, bottom));
    painter.rect_stroke(
        outline,
        0.0,
        Stroke::new(1.25, theme::framing_line_dark()),
        StrokeKind::Outside,
    );
}

/// Diagonal 45-degree hatch lines clipped to `band`, used to mark the framing
/// (cavity) layer apart from solid material bands.
fn draw_band_hatch(painter: &egui::Painter, band: Rect, color: Color32) {
    let stroke = Stroke::new(0.75, color);
    let step = 6.0;
    let height = band.height();
    // Sweep diagonals across the band; each line is sheared by the band height so
    // it lands at a consistent 45 degrees regardless of band width.
    let mut offset = -height;
    while offset < band.width() {
        let start = Pos2::new(band.left() + offset, band.bottom());
        let end = Pos2::new(band.left() + offset + height, band.top());
        if let Some(segment) = clip_segment_to_rect(start, end, band) {
            painter.line_segment([segment.0, segment.1], stroke);
        }
        offset += step;
    }
}

/// A medium-contrast hatch color derived from the band fill: darkened for light
/// fills, lightened for dark fills, so the stripes always read.
fn hatch_color(fill: Color32) -> Color32 {
    let luma = 0.299 * fill.r() as f32 + 0.587 * fill.g() as f32 + 0.114 * fill.b() as f32;
    if luma > 128.0 {
        Color32::from_rgba_unmultiplied(20, 16, 12, 150)
    } else {
        Color32::from_rgba_unmultiplied(235, 230, 220, 150)
    }
}

/// Liang-Barsky clip of segment `a`-`b` to `rect`, returning the visible portion.
fn clip_segment_to_rect(a: Pos2, b: Pos2, rect: Rect) -> Option<(Pos2, Pos2)> {
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let edges = [
        (-dx, a.x - rect.left()),
        (dx, rect.right() - a.x),
        (-dy, a.y - rect.top()),
        (dy, rect.bottom() - a.y),
    ];
    for (p, q) in edges {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t1 {
                    return None;
                }
                if t > t0 {
                    t0 = t;
                }
            } else {
                if t < t0 {
                    return None;
                }
                if t < t1 {
                    t1 = t;
                }
            }
        }
    }
    Some((
        Pos2::new(a.x + t0 * dx, a.y + t0 * dy),
        Pos2::new(a.x + t1 * dx, a.y + t1 * dy),
    ))
}

pub(super) fn section_position(wall: &Wall, selection: &Selection) -> Option<Length> {
    match selection {
        Selection::Opening(id) => wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == *id)
            .map(|opening| opening.center),
        Selection::Dimension(id) => wall
            .dimensions
            .iter()
            .find(|dimension| dimension.id.0 == *id)
            .and_then(|dimension| {
                let start = dimension.start.local_x(wall)?;
                let end = dimension.end.local_x(wall)?;
                Some((start + end) / 2)
            }),
        Selection::Member { .. }
        | Selection::Join(_)
        | Selection::Level(_)
        | Selection::Room(_)
        | Selection::System(_)
        | Selection::Material(_)
        | Selection::Wall => Some(wall.length / 2),
    }
}
