//! Opening editing in the design-elevation view: hit-testing the move/resize
//! targets and handles, drawing the edit handles + guide, and mapping drag motion
//! to wall-axis deltas. `OpeningDragEvent` is the event the design view emits.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2,
};
use framer_core::{Length, Wall};

use super::theme;
use super::view_common::opening_rect;
use crate::app::model_edit::OpeningEditHandle;

// === extracted opening items appended below; visibility adjusted in place ===

pub(super) enum OpeningDragEvent {
    Started {
        opening_id: String,
        handle: OpeningEditHandle,
    },
    Updated {
        delta_x: Length,
        delta_y: Length,
    },
    Stopped,
}

#[derive(Clone)]
pub(super) struct OpeningHandleHit {
    pub(super) opening_id: String,
    pub(super) handle: OpeningEditHandle,
}

pub(super) fn hit_opening_edit_target(
    drawing: Rect,
    scale: f32,
    wall: &Wall,
    position: Pos2,
) -> Option<OpeningHandleHit> {
    wall.openings.iter().rev().find_map(|opening| {
        let rect = opening_rect(drawing, scale, scale, opening);
        hit_opening_edit_handle(rect, position).map(|handle| OpeningHandleHit {
            opening_id: opening.id.0.clone(),
            handle,
        })
    })
}

pub(super) fn hit_opening_move_target(
    drawing: Rect,
    scale: f32,
    wall: &Wall,
    position: Pos2,
) -> Option<OpeningHandleHit> {
    wall.openings.iter().rev().find_map(|opening| {
        let rect = opening_rect(drawing, scale, scale, opening);
        hit_opening_move_handle(rect, position).then(|| OpeningHandleHit {
            opening_id: opening.id.0.clone(),
            handle: OpeningEditHandle::Move,
        })
    })
}

pub(super) fn hit_opening_move_handle(rect: Rect, position: Pos2) -> bool {
    rect.expand(11.0).contains(position)
}

pub(super) fn hit_opening_edit_handle(rect: Rect, position: Pos2) -> Option<OpeningEditHandle> {
    const HANDLE_HIT_RADIUS: f32 = 10.0;
    const EDGE_HIT_RADIUS: f32 = 7.0;

    let corner_hits = [
        (OpeningEditHandle::TopLeft, rect.left_top()),
        (OpeningEditHandle::TopRight, rect.right_top()),
        (OpeningEditHandle::BottomLeft, rect.left_bottom()),
        (OpeningEditHandle::BottomRight, rect.right_bottom()),
    ]
    .into_iter()
    .filter_map(|(handle, point)| {
        let distance = position.distance(point);
        (distance <= HANDLE_HIT_RADIUS).then_some((handle, distance))
    })
    .min_by(|(_, left), (_, right)| left.total_cmp(right))
    .map(|(handle, _)| handle);
    if corner_hits.is_some() {
        return corner_hits;
    }

    let within_y =
        position.y >= rect.top() - EDGE_HIT_RADIUS && position.y <= rect.bottom() + EDGE_HIT_RADIUS;
    let within_x =
        position.x >= rect.left() - EDGE_HIT_RADIUS && position.x <= rect.right() + EDGE_HIT_RADIUS;
    let edge_hits = [
        (
            OpeningEditHandle::Left,
            (position.x - rect.left()).abs(),
            within_y,
        ),
        (
            OpeningEditHandle::Right,
            (position.x - rect.right()).abs(),
            within_y,
        ),
        (
            OpeningEditHandle::Top,
            (position.y - rect.top()).abs(),
            within_x,
        ),
        (
            OpeningEditHandle::Bottom,
            (position.y - rect.bottom()).abs(),
            within_x,
        ),
    ]
    .into_iter()
    .filter(|(_, distance, in_range)| *in_range && *distance <= EDGE_HIT_RADIUS)
    .min_by(|(_, left, _), (_, right, _)| left.total_cmp(right))
    .map(|(handle, _, _)| handle);
    if edge_hits.is_some() {
        return edge_hits;
    }

    rect.contains(position).then_some(OpeningEditHandle::Move)
}

pub(super) fn draw_opening_edit_handles(
    painter: &egui::Painter,
    rect: Rect,
    selected: bool,
    hovered_handle: Option<OpeningEditHandle>,
) {
    for handle in [
        OpeningEditHandle::TopLeft,
        OpeningEditHandle::Top,
        OpeningEditHandle::TopRight,
        OpeningEditHandle::Right,
        OpeningEditHandle::BottomRight,
        OpeningEditHandle::Bottom,
        OpeningEditHandle::BottomLeft,
        OpeningEditHandle::Left,
    ] {
        let center = opening_handle_position(rect, handle);
        let hovered = hovered_handle == Some(handle);
        let size = if hovered { 7.5 } else { 6.0 };
        let fill = if selected {
            theme::active_blue()
        } else {
            theme::sheet()
        };
        let stroke = if hovered {
            Stroke::new(2.0, theme::active_blue())
        } else {
            Stroke::new(1.25, theme::active_blue())
        };
        painter.rect_filled(Rect::from_center_size(center, Vec2::splat(size)), 1.0, fill);
        painter.rect_stroke(
            Rect::from_center_size(center, Vec2::splat(size)),
            1.0,
            stroke,
            StrokeKind::Outside,
        );
    }
}

pub(super) fn opening_handle_position(rect: Rect, handle: OpeningEditHandle) -> Pos2 {
    match handle {
        OpeningEditHandle::Move => rect.center(),
        OpeningEditHandle::Left => Pos2::new(rect.left(), rect.center().y),
        OpeningEditHandle::Right => Pos2::new(rect.right(), rect.center().y),
        OpeningEditHandle::Top => Pos2::new(rect.center().x, rect.top()),
        OpeningEditHandle::Bottom => Pos2::new(rect.center().x, rect.bottom()),
        OpeningEditHandle::TopLeft => rect.left_top(),
        OpeningEditHandle::TopRight => rect.right_top(),
        OpeningEditHandle::BottomLeft => rect.left_bottom(),
        OpeningEditHandle::BottomRight => rect.right_bottom(),
    }
}

pub(super) fn opening_drag_delta(delta: Vec2, scale: f32) -> (Length, Length) {
    let scale = scale.max(0.001) as f64;
    (
        Length::from_inches(delta.x as f64 / scale),
        Length::from_inches(-(delta.y as f64) / scale),
    )
}

pub(super) fn cursor_for_opening_handle(handle: OpeningEditHandle, active: bool) -> CursorIcon {
    match handle {
        OpeningEditHandle::Move => {
            if active {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            }
        }
        OpeningEditHandle::Left => CursorIcon::ResizeWest,
        OpeningEditHandle::Right => CursorIcon::ResizeEast,
        OpeningEditHandle::Top => CursorIcon::ResizeNorth,
        OpeningEditHandle::Bottom => CursorIcon::ResizeSouth,
        OpeningEditHandle::TopLeft => CursorIcon::ResizeNorthWest,
        OpeningEditHandle::TopRight => CursorIcon::ResizeNorthEast,
        OpeningEditHandle::BottomLeft => CursorIcon::ResizeSouthWest,
        OpeningEditHandle::BottomRight => CursorIcon::ResizeSouthEast,
    }
}

pub(super) fn draw_opening_guide(
    painter: &egui::Painter,
    rect: Rect,
    label: &str,
    selected: bool,
    hovered: bool,
) {
    let stroke = if selected {
        Stroke::new(2.0, theme::active_blue())
    } else if hovered {
        Stroke::new(1.5, theme::framing_line_dark())
    } else {
        Stroke::new(1.0, theme::framing_line())
    };
    painter.rect_filled(
        rect,
        0.0,
        Color32::from_rgba_unmultiplied(255, 255, 255, 76),
    );
    painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Outside);
    painter.text(
        rect.left_top() + Vec2::new(4.0, 12.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.0),
        Color32::from_rgb(99, 74, 39),
    );
}
