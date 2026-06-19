//! Dimensioning for the design-elevation view: dimension-line annotations, the
//! pending-dimension placement preview, and the dimension-anchor markers + hit
//! testing. Driven by `elevation_design`'s orchestrator.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use framer_core::{
    DimensionAnchor, DimensionAxis, DimensionHorizontalReference, DimensionKind,
    DimensionVerticalReference, Length, Opening, Wall,
};

use super::geom::distance_to_segment;
use super::opening_rect;
use super::theme;

// === extracted dimension items appended below; visibility adjusted in place ===

#[derive(Clone, Copy)]
pub(super) struct DimensionPlacement {
    pub(super) axis: DimensionAxis,
    pub(super) line_offset: Length,
}

pub(super) fn draw_wall_dimension_annotations(
    painter: &egui::Painter,
    drawing: Rect,
    sx: f32,
    wall: &Wall,
    selected_dimension: Option<&str>,
    pointer: Option<Pos2>,
    click_enabled: bool,
) -> Option<String> {
    let mut clicked = None;
    let mut horizontal_index = 0usize;
    let mut vertical_index = 0usize;

    for dimension in &wall.dimensions {
        let Some(start_coordinate) = dimension.start.coordinate(wall, dimension.axis) else {
            continue;
        };
        let Some(end_coordinate) = dimension.end.coordinate(wall, dimension.axis) else {
            continue;
        };
        let placed = dimension.line_offset.is_some();
        let (line_start, line_end) = match dimension.axis {
            DimensionAxis::Horizontal => {
                let start = drawing.left() + start_coordinate.inches() as f32 * sx;
                let end = drawing.left() + end_coordinate.inches() as f32 * sx;
                let y = dimension.line_offset.map_or_else(
                    || {
                        let y = drawing.top() - 18.0 - horizontal_index.min(3) as f32 * 18.0;
                        horizontal_index += 1;
                        y
                    },
                    |line_offset| {
                        dimension_line_screen_position(
                            drawing,
                            sx,
                            DimensionAxis::Horizontal,
                            line_offset,
                        )
                    },
                );
                (Pos2::new(start, y), Pos2::new(end, y))
            }
            DimensionAxis::Vertical => {
                let start = drawing.bottom() - start_coordinate.inches() as f32 * sx;
                let end = drawing.bottom() - end_coordinate.inches() as f32 * sx;
                let x = dimension.line_offset.map_or_else(
                    || {
                        let x = drawing.right() + 50.0 + vertical_index.min(3) as f32 * 18.0;
                        vertical_index += 1;
                        x
                    },
                    |line_offset| {
                        dimension_line_screen_position(
                            drawing,
                            sx,
                            DimensionAxis::Vertical,
                            line_offset,
                        )
                    },
                );
                (Pos2::new(x, start), Pos2::new(x, end))
            }
        };
        let selected = selected_dimension == Some(dimension.id.0.as_str());
        let unsatisfied = dimension.kind == DimensionKind::Driving
            && !wall.is_driving_dimension_satisfied(dimension);
        let label = dimension_display_value(wall, dimension);
        let label_rect = dimension_label_rect(line_start, line_end, dimension.axis, &label);
        let hovered = pointer.is_some_and(|position| {
            distance_to_segment(position, line_start, line_end) < 7.0
                || label_rect.contains(position)
        });
        let color = if unsatisfied {
            theme::danger()
        } else if selected {
            theme::active_blue()
        } else if dimension.kind == DimensionKind::Reference {
            theme::text_muted()
        } else {
            theme::framing_line()
        };
        let stroke = Stroke::new(if selected || hovered { 2.0 } else { 1.25 }, color);

        draw_dimension_line_with_label_gap(
            painter,
            line_start,
            line_end,
            dimension.axis,
            label_rect,
            stroke,
        );
        if placed {
            if let Some(start_position) =
                dimension_anchor_position(drawing, sx, sx, wall, &dimension.start)
            {
                painter.line_segment([start_position, line_start], Stroke::new(0.75, color));
            }
            if let Some(end_position) =
                dimension_anchor_position(drawing, sx, sx, wall, &dimension.end)
            {
                painter.line_segment([end_position, line_end], Stroke::new(0.75, color));
            }
        } else {
            match dimension.axis {
                DimensionAxis::Horizontal => {
                    painter.line_segment(
                        [
                            Pos2::new(line_start.x, line_start.y),
                            Pos2::new(line_start.x, drawing.top() + 4.0),
                        ],
                        Stroke::new(0.75, color),
                    );
                    painter.line_segment(
                        [
                            Pos2::new(line_end.x, line_end.y),
                            Pos2::new(line_end.x, drawing.top() + 4.0),
                        ],
                        Stroke::new(0.75, color),
                    );
                }
                DimensionAxis::Vertical => {
                    painter.line_segment(
                        [
                            Pos2::new(line_start.x, line_start.y),
                            Pos2::new(drawing.right() - 4.0, line_start.y),
                        ],
                        Stroke::new(0.75, color),
                    );
                    painter.line_segment(
                        [
                            Pos2::new(line_end.x, line_end.y),
                            Pos2::new(drawing.right() - 4.0, line_end.y),
                        ],
                        Stroke::new(0.75, color),
                    );
                }
            }
        }
        draw_dimension_tick(painter, line_start, dimension.axis, color);
        draw_dimension_tick(painter, line_end, dimension.axis, color);

        let label_pos = dimension_label_position(line_start, line_end, dimension.axis);
        painter.text(
            label_pos,
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(11.0),
            color,
        );

        if hovered && click_enabled {
            clicked = Some(dimension.id.0.clone());
        }
    }

    clicked
}

pub(super) fn draw_dimension_tick(painter: &egui::Painter, point: Pos2, axis: DimensionAxis, color: Color32) {
    let tick = match axis {
        DimensionAxis::Horizontal => [point + Vec2::new(-4.0, 4.0), point + Vec2::new(4.0, -4.0)],
        DimensionAxis::Vertical => [point + Vec2::new(-4.0, -4.0), point + Vec2::new(4.0, 4.0)],
    };
    painter.line_segment(tick, Stroke::new(1.0, color));
}

pub(super) fn dimension_label_position(start: Pos2, end: Pos2, axis: DimensionAxis) -> Pos2 {
    match axis {
        DimensionAxis::Horizontal => Pos2::new((start.x + end.x) / 2.0, start.y - 2.0),
        DimensionAxis::Vertical => Pos2::new(start.x, (start.y + end.y) / 2.0),
    }
}

pub(super) fn dimension_label_rect(start: Pos2, end: Pos2, axis: DimensionAxis, label: &str) -> Rect {
    let center = dimension_label_position(start, end, axis);
    let width = (label.chars().count() as f32 * 6.5 + 12.0).clamp(34.0, 86.0);
    Rect::from_center_size(center, Vec2::new(width, 18.0))
}

pub(super) fn draw_dimension_line_with_label_gap(
    painter: &egui::Painter,
    start: Pos2,
    end: Pos2,
    axis: DimensionAxis,
    label_rect: Rect,
    stroke: Stroke,
) {
    match axis {
        DimensionAxis::Horizontal => {
            let y = start.y;
            let left = start.x.min(end.x);
            let right = start.x.max(end.x);
            let gap_left = label_rect.left().clamp(left, right);
            let gap_right = label_rect.right().clamp(left, right);
            if gap_left > left {
                painter.line_segment([Pos2::new(left, y), Pos2::new(gap_left, y)], stroke);
            }
            if gap_right < right {
                painter.line_segment([Pos2::new(gap_right, y), Pos2::new(right, y)], stroke);
            }
        }
        DimensionAxis::Vertical => {
            let x = start.x;
            let top = start.y.min(end.y);
            let bottom = start.y.max(end.y);
            let gap_top = label_rect.top().clamp(top, bottom);
            let gap_bottom = label_rect.bottom().clamp(top, bottom);
            if gap_top > top {
                painter.line_segment([Pos2::new(x, top), Pos2::new(x, gap_top)], stroke);
            }
            if gap_bottom < bottom {
                painter.line_segment([Pos2::new(x, gap_bottom), Pos2::new(x, bottom)], stroke);
            }
        }
    }
}

pub(super) fn dimension_display_value(wall: &Wall, dimension: &framer_core::DimensionConstraint) -> String {
    let measured = wall.dimension_measurement(dimension);
    match dimension.kind {
        DimensionKind::Driving => dimension.value.or(measured).map_or_else(
            || "?".to_owned(),
            |value| {
                if wall.is_driving_dimension_satisfied(dimension) {
                    value.to_string()
                } else {
                    format!("! {value}")
                }
            },
        ),
        DimensionKind::Reference => measured
            .map(|value| format!("({value})"))
            .unwrap_or_else(|| "(?)".to_owned()),
    }
}

pub(super) struct PendingDimensionPreview<'a> {
    pub(super) first_anchor: &'a DimensionAnchor,
    pub(super) second_anchor: &'a DimensionAnchor,
    pub(super) pointer: Option<Pos2>,
    pub(super) fallback_axis: DimensionAxis,
}

pub(super) fn draw_pending_dimension_preview(
    painter: &egui::Painter,
    drawing: Rect,
    sx: f32,
    sy: f32,
    wall: &Wall,
    preview: PendingDimensionPreview<'_>,
) -> Option<DimensionPlacement> {
    let first_position = dimension_anchor_position(drawing, sx, sy, wall, preview.first_anchor)?;
    let second_position = dimension_anchor_position(drawing, sx, sy, wall, preview.second_anchor)?;
    let axis = dimension_axis_for_placement_position(
        first_position,
        second_position,
        preview.pointer,
        preview.fallback_axis,
    );
    let color = theme::active_blue();
    let stroke = Stroke::new(1.75, color);

    let line_offset = dimension_line_offset_for_position(
        drawing,
        sx,
        axis,
        preview
            .pointer
            .unwrap_or_else(|| pending_dimension_default_line_position(drawing, axis)),
    );
    let (line_start, line_end) = match axis {
        DimensionAxis::Horizontal => {
            let y = dimension_line_screen_position(drawing, sx, axis, line_offset);
            (
                Pos2::new(first_position.x, y),
                Pos2::new(second_position.x, y),
            )
        }
        DimensionAxis::Vertical => {
            let x = dimension_line_screen_position(drawing, sx, axis, line_offset);
            (
                Pos2::new(x, first_position.y),
                Pos2::new(x, second_position.y),
            )
        }
    };

    let label = preview
        .first_anchor
        .coordinate(wall, axis)
        .zip(preview.second_anchor.coordinate(wall, axis))
        .map(|(start, end)| (end - start).abs().to_string())
        .unwrap_or_else(|| "?".to_owned());
    let label_rect = dimension_label_rect(line_start, line_end, axis, &label);
    draw_dimension_line_with_label_gap(painter, line_start, line_end, axis, label_rect, stroke);
    match axis {
        DimensionAxis::Horizontal => {
            painter.line_segment([first_position, line_start], Stroke::new(0.75, color));
            painter.line_segment([second_position, line_end], Stroke::new(0.75, color));
        }
        DimensionAxis::Vertical => {
            painter.line_segment([first_position, line_start], Stroke::new(0.75, color));
            painter.line_segment([second_position, line_end], Stroke::new(0.75, color));
        }
    }
    draw_dimension_tick(painter, line_start, axis, color);
    draw_dimension_tick(painter, line_end, axis, color);

    painter.text(
        dimension_label_position(line_start, line_end, axis),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.0),
        color,
    );

    Some(DimensionPlacement { axis, line_offset })
}

pub(super) fn pending_dimension_default_line_position(drawing: Rect, axis: DimensionAxis) -> Pos2 {
    match axis {
        DimensionAxis::Horizontal => Pos2::new(drawing.center().x, drawing.top() - 24.0),
        DimensionAxis::Vertical => Pos2::new(drawing.right() + 56.0, drawing.center().y),
    }
}

pub(super) fn dimension_line_offset_for_position(
    drawing: Rect,
    scale: f32,
    axis: DimensionAxis,
    position: Pos2,
) -> Length {
    let inches = match axis {
        DimensionAxis::Horizontal => (drawing.bottom() - position.y) / scale,
        DimensionAxis::Vertical => (position.x - drawing.left()) / scale,
    };
    Length::from_inches(inches as f64)
}

pub(super) fn dimension_line_screen_position(
    drawing: Rect,
    scale: f32,
    axis: DimensionAxis,
    line_offset: Length,
) -> f32 {
    match axis {
        DimensionAxis::Horizontal => drawing.bottom() - line_offset.inches() as f32 * scale,
        DimensionAxis::Vertical => drawing.left() + line_offset.inches() as f32 * scale,
    }
}

pub(super) fn dimension_anchor_position(
    drawing: Rect,
    sx: f32,
    sy: f32,
    wall: &Wall,
    anchor: &DimensionAnchor,
) -> Option<Pos2> {
    let (x, y) = anchor.point(wall)?;
    Some(Pos2::new(
        drawing.left() + x.inches() as f32 * sx,
        drawing.bottom() - y.inches() as f32 * sy,
    ))
}

pub(super) fn dimension_axis_for_placement_position(
    first_position: Pos2,
    second_position: Pos2,
    pointer: Option<Pos2>,
    fallback_axis: DimensionAxis,
) -> DimensionAxis {
    let Some(pointer) = pointer else {
        return fallback_axis;
    };
    let midpoint = first_position + (second_position - first_position) * 0.5;
    let offset = pointer - midpoint;
    if offset.x.abs() <= 4.0 && offset.y.abs() <= 4.0 {
        return fallback_axis;
    }
    if offset.x.abs() > offset.y.abs() {
        DimensionAxis::Vertical
    } else {
        DimensionAxis::Horizontal
    }
}

pub(super) struct DimensionAnchorSelection<'a> {
    pub(super) axis: DimensionAxis,
    pub(super) first_anchor: Option<&'a DimensionAnchor>,
    pub(super) second_anchor: Option<&'a DimensionAnchor>,
}

pub(super) fn draw_dimension_anchors(
    painter: &egui::Painter,
    drawing: Rect,
    sx: f32,
    sy: f32,
    wall: &Wall,
    selection: DimensionAnchorSelection<'_>,
) {
    for marker in dimension_anchor_markers(drawing, sx, sy, wall) {
        let selected = selection.first_anchor == Some(&marker.anchor)
            || selection.second_anchor == Some(&marker.anchor);
        let on_axis = marker.anchor.coordinate(wall, selection.axis).is_some();
        let radius = if selected { 5.5 } else { marker.kind.radius() };
        let fill = if selected {
            theme::active_blue()
        } else if marker.kind == DimensionAnchorKind::Center {
            Color32::from_rgb(224, 240, 225)
        } else if marker.kind == DimensionAnchorKind::Vertex {
            Color32::from_rgb(235, 241, 247)
        } else {
            Color32::from_rgb(247, 247, 242)
        };
        let stroke = if on_axis {
            theme::active_blue()
        } else {
            theme::text_muted()
        };
        painter.circle_filled(marker.position, radius, fill);
        painter.circle_stroke(
            marker.position,
            radius,
            Stroke::new(if selected { 2.0 } else { 1.25 }, stroke),
        );
    }
}

pub(super) fn hit_dimension_anchor(
    position: Pos2,
    drawing: Rect,
    sx: f32,
    sy: f32,
    wall: &Wall,
) -> Option<DimensionAnchor> {
    dimension_anchor_markers(drawing, sx, sy, wall)
        .into_iter()
        .filter_map(|marker| {
            let distance = position.distance(marker.position);
            (distance <= marker.kind.hit_radius()).then_some((marker, distance))
        })
        .max_by(|(left, left_distance), (right, right_distance)| {
            left.kind
                .priority()
                .cmp(&right.kind.priority())
                .then_with(|| right_distance.total_cmp(left_distance))
        })
        .map(|(marker, _)| marker.anchor)
}

#[derive(Clone)]
pub(super) struct DimensionAnchorMarker {
    pub(super) anchor: DimensionAnchor,
    pub(super) position: Pos2,
    pub(super) kind: DimensionAnchorKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DimensionAnchorKind {
    Edge,
    Center,
    Vertex,
}

impl DimensionAnchorKind {
    fn priority(self) -> u8 {
        match self {
            Self::Vertex => 3,
            Self::Center => 2,
            Self::Edge => 1,
        }
    }

    fn radius(self) -> f32 {
        match self {
            Self::Vertex => 3.8,
            Self::Center => 4.5,
            Self::Edge => 3.5,
        }
    }

    fn hit_radius(self) -> f32 {
        match self {
            Self::Vertex => 11.0,
            Self::Center => 10.0,
            Self::Edge => 9.0,
        }
    }
}

pub(super) fn dimension_anchor_markers(
    drawing: Rect,
    sx: f32,
    sy: f32,
    wall: &Wall,
) -> Vec<DimensionAnchorMarker> {
    let mut anchors = Vec::new();
    push_wall_anchor_markers(&mut anchors, drawing);

    for opening in &wall.openings {
        let rect = opening_rect(drawing, sx, sy, opening);
        push_opening_anchor_markers(&mut anchors, rect, opening);
    }

    anchors
}

pub(super) fn push_wall_anchor_markers(markers: &mut Vec<DimensionAnchorMarker>, rect: Rect) {
    push_point_anchor_markers(markers, rect, |horizontal, vertical| {
        DimensionAnchor::WallPoint {
            horizontal,
            vertical,
        }
    });
}

pub(super) fn push_opening_anchor_markers(
    markers: &mut Vec<DimensionAnchorMarker>,
    rect: Rect,
    opening: &Opening,
) {
    push_point_anchor_markers(markers, rect, |horizontal, vertical| {
        DimensionAnchor::OpeningPoint {
            opening: opening.id.clone(),
            horizontal,
            vertical,
        }
    });
}

pub(super) fn push_point_anchor_markers(
    markers: &mut Vec<DimensionAnchorMarker>,
    rect: Rect,
    mut anchor: impl FnMut(DimensionHorizontalReference, DimensionVerticalReference) -> DimensionAnchor,
) {
    for (horizontal, x) in [
        (DimensionHorizontalReference::Left, rect.left()),
        (DimensionHorizontalReference::Center, rect.center().x),
        (DimensionHorizontalReference::Right, rect.right()),
    ] {
        for (vertical, y) in [
            (DimensionVerticalReference::Bottom, rect.bottom()),
            (DimensionVerticalReference::Center, rect.center().y),
            (DimensionVerticalReference::Top, rect.top()),
        ] {
            let kind = match (horizontal, vertical) {
                (DimensionHorizontalReference::Center, DimensionVerticalReference::Center) => {
                    DimensionAnchorKind::Center
                }
                (DimensionHorizontalReference::Center, _)
                | (_, DimensionVerticalReference::Center) => DimensionAnchorKind::Edge,
                _ => DimensionAnchorKind::Vertex,
            };
            markers.push(DimensionAnchorMarker {
                anchor: anchor(horizontal, vertical),
                position: Pos2::new(x, y),
                kind,
            });
        }
    }
}
