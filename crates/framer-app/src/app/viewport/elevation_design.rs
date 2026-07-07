//! The design-elevation renderer: orchestrates the wall drawing, dimensioning
//! (`elevation_dimensions`), and opening editing (`elevation_openings`) for a
//! single wall, returning a `DesignElevationResponse` of clicks + drag events.

use eframe::egui::{self, Color32, Rect, Sense, Ui, Vec2};
use framer_core::{DimensionAnchor, DimensionAxis, Length, Wall};

use super::camera_2d::{View2dState, apply_view_2d_input, reset_view_on_empty_double_click};
use super::elevation_dimensions::{
    DimensionAnchorSelection, PendingDimensionPreview, draw_dimension_anchors,
    draw_pending_dimension_preview, draw_wall_dimension_annotations, hit_dimension_anchor,
};
use super::elevation_openings::{
    OpeningDragEvent, cursor_for_opening_handle, draw_opening_edit_handles, draw_opening_guide,
    hit_opening_edit_target, hit_opening_move_target, opening_drag_delta,
};
use super::theme;
use super::view_common::{
    WallElevationLayout, draw_drafting_grid, draw_drafting_rulers, draw_view_border,
    draw_view_title, opening_rect,
};
use crate::app::model_edit::OpeningDragState;

// === extracted design block appended below; visibility adjusted in place ===

pub(super) enum DesignElevationClick {
    Opening(String),
    Dimension(String),
    DimensionAnchor(DimensionAnchor),
    DimensionPlacement {
        axis: DimensionAxis,
        line_offset: Length,
    },
}

#[derive(Default)]
pub(super) struct DesignElevationResponse {
    pub(super) click: Option<DesignElevationClick>,
    pub(super) opening_drag: Option<OpeningDragEvent>,
}

pub(super) struct DesignElevationView<'a> {
    pub(super) selected_opening: Option<&'a str>,
    pub(super) selected_dimension: Option<&'a str>,
    pub(super) dimension_tool_active: bool,
    pub(super) dimension_tool_axis: DimensionAxis,
    pub(super) first_dimension_anchor: Option<&'a DimensionAnchor>,
    pub(super) second_dimension_anchor: Option<&'a DimensionAnchor>,
    pub(super) active_opening_drag: Option<&'a OpeningDragState>,
}

pub(super) fn draw_wall_design_elevation(
    ui: &mut Ui,
    wall: &Wall,
    view: DesignElevationView<'_>,
    camera: &mut View2dState,
) -> DesignElevationResponse {
    let DesignElevationView {
        selected_opening,
        selected_dimension,
        dimension_tool_active,
        dimension_tool_axis,
        first_dimension_anchor,
        second_dimension_anchor,
        active_opening_drag,
    } = view;
    let available = ui.available_size();
    let desired = Vec2::new(available.x.max(420.0), (available.y - 16.0).max(420.0));
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    let side_margin = 52.0;
    let horizontal_dimension_count = wall
        .dimensions
        .iter()
        .filter(|dimension| dimension.axis == DimensionAxis::Horizontal)
        .count();
    let vertical_dimension_count = wall
        .dimensions
        .iter()
        .filter(|dimension| dimension.axis == DimensionAxis::Vertical)
        .count();
    let top_margin = (64.0 + horizontal_dimension_count.min(4) as f32 * 18.0).min(136.0);
    let right_margin = (96.0 + vertical_dimension_count.min(4) as f32 * 18.0).min(168.0);
    let drawing = Rect::from_min_max(
        rect.min + Vec2::new(side_margin, top_margin),
        rect.max - Vec2::new(right_margin, side_margin),
    );
    // Pan/zoom before laying out the wall. While an opening drag is active,
    // Space+drag must not be claimed for panning, so it can't hijack the drag.
    let panning = apply_view_2d_input(
        ui,
        &response,
        drawing,
        camera,
        active_opening_drag.is_none(),
    );
    let layout = WallElevationLayout::new(drawing, wall, camera);
    let wall_rect = layout.wall_rect;
    let scale = layout.scale;

    painter.rect_filled(rect, 0.0, theme::sheet());
    draw_drafting_rulers(&painter, rect, drawing);
    draw_drafting_grid(&painter, drawing);
    draw_view_border(&painter, drawing);
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let press_origin = ui.input(|input| input.pointer.press_origin());
    let mut output = DesignElevationResponse::default();
    let pending_dimension_active = dimension_tool_active
        && first_dimension_anchor.is_some()
        && second_dimension_anchor.is_some();
    let mut hovered_handle = None;
    let mut hovered_dimension_move = None;
    if let Some(position) = pointer {
        if !dimension_tool_active {
            hovered_handle = hit_opening_edit_target(wall_rect, scale, wall, position);
        } else if !pending_dimension_active {
            hovered_dimension_move = hit_opening_move_target(wall_rect, scale, wall, position);
        }
    }
    let mut over_element = hovered_handle.is_some() || hovered_dimension_move.is_some();

    painter.rect_filled(
        wall_rect,
        0.0,
        Color32::from_rgba_unmultiplied(188, 179, 158, 34),
    );
    draw_view_border(&painter, wall_rect);
    for opening in &wall.openings {
        let opening_rect = opening_rect(wall_rect, scale, scale, opening);
        let hovered = pointer.is_some_and(|position| opening_rect.contains(position));
        over_element |= hovered;
        let handle_hovered = hovered_handle
            .as_ref()
            .is_some_and(|hit| hit.opening_id == opening.id.0)
            || hovered_dimension_move
                .as_ref()
                .is_some_and(|hit| hit.opening_id == opening.id.0);
        let active = active_opening_drag
            .as_ref()
            .is_some_and(|drag| drag.opening_id == opening.id.0);
        let selected = selected_opening == Some(opening.id.0.as_str());
        draw_opening_guide(
            &painter,
            opening_rect,
            &opening.name,
            selected || active,
            hovered || handle_hovered,
        );
        if !dimension_tool_active && (selected || hovered || active || handle_hovered) {
            draw_opening_edit_handles(
                &painter,
                opening_rect,
                selected || active,
                hovered_handle
                    .as_ref()
                    .filter(|hit| hit.opening_id == opening.id.0)
                    .map(|hit| hit.handle),
            );
        }
        if hovered && response.clicked() && !dimension_tool_active {
            output.click = Some(DesignElevationClick::Opening(opening.id.0.clone()));
        }
    }

    if let Some(active) = active_opening_drag {
        if response.drag_stopped() {
            output.opening_drag = Some(OpeningDragEvent::Stopped);
        } else if response.dragged_by(egui::PointerButton::Primary)
            && let Some(delta) = response.total_drag_delta()
        {
            let (delta_x, delta_y) = opening_drag_delta(delta, scale);
            output.opening_drag = Some(OpeningDragEvent::Updated { delta_x, delta_y });
            ui.ctx()
                .set_cursor_icon(cursor_for_opening_handle(active.handle, true));
        }
    } else if !panning && response.drag_started_by(egui::PointerButton::Primary) {
        let hit = press_origin.and_then(|position| {
            if pending_dimension_active {
                None
            } else if dimension_tool_active {
                hit_opening_move_target(wall_rect, scale, wall, position)
            } else {
                hit_opening_edit_target(wall_rect, scale, wall, position)
            }
        });
        if let Some(hit) = hit {
            output.click = None;
            output.opening_drag = Some(OpeningDragEvent::Started {
                opening_id: hit.opening_id,
                handle: hit.handle,
            });
            ui.ctx()
                .set_cursor_icon(cursor_for_opening_handle(hit.handle, true));
        }
    } else if let Some(hit) = hovered_handle {
        ui.ctx()
            .set_cursor_icon(cursor_for_opening_handle(hit.handle, false));
    } else if let Some(hit) = hovered_dimension_move {
        ui.ctx()
            .set_cursor_icon(cursor_for_opening_handle(hit.handle, false));
    }

    let dimension_click = draw_wall_dimension_annotations(
        &painter,
        wall_rect,
        scale,
        wall,
        selected_dimension,
        pointer,
        response.clicked() && !dimension_tool_active,
    );
    if let Some(dimension_id) = dimension_click {
        output.click = Some(DesignElevationClick::Dimension(dimension_id));
    }

    if dimension_tool_active {
        let placement = if let (Some(first_anchor), Some(second_anchor)) =
            (first_dimension_anchor, second_dimension_anchor)
        {
            draw_pending_dimension_preview(
                &painter,
                wall_rect,
                scale,
                scale,
                wall,
                PendingDimensionPreview {
                    first_anchor,
                    second_anchor,
                    pointer,
                    fallback_axis: dimension_tool_axis,
                },
            )
        } else {
            None
        };
        draw_dimension_anchors(
            &painter,
            wall_rect,
            scale,
            scale,
            wall,
            DimensionAnchorSelection {
                axis: placement
                    .map(|placement| placement.axis)
                    .unwrap_or(dimension_tool_axis),
                first_anchor: first_dimension_anchor,
                second_anchor: second_dimension_anchor,
            },
        );
        // A Space+primary-drag is a pan, and its release frame still fires
        // `drag_stopped_by(Primary)` / `clicked()` (the frame-local `panning`
        // flag is already false by then, so it can't gate this). While Space is
        // held, primary input is reserved for panning — never dimension
        // placement — so gate on the Space modifier directly.
        let space_pan = ui.input(|input| input.key_down(egui::Key::Space));
        let should_place_dimension = !space_pan
            && (response.clicked() || response.drag_stopped_by(egui::PointerButton::Primary));
        if let Some(position) = pointer {
            if let Some(placement) = placement {
                if should_place_dimension {
                    output.click = Some(DesignElevationClick::DimensionPlacement {
                        axis: placement.axis,
                        line_offset: placement.line_offset,
                    });
                }
            } else if !space_pan
                && response.clicked()
                && let Some(anchor) = hit_dimension_anchor(position, wall_rect, scale, scale, wall)
            {
                output.click = Some(DesignElevationClick::DimensionAnchor(anchor));
            }
        }
    }

    draw_view_title(
        &painter,
        drawing,
        format!(
            "{} elevation - {} x {}",
            wall.name, wall.length, wall.height
        ),
    );

    reset_view_on_empty_double_click(&response, camera, over_element);
    output
}
