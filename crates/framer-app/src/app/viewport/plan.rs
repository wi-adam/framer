//! The plan (top-down) renderer: the drafting grid/rulers, wall + opening
//! drawing, the draw-wall tool with snapping, wall-endpoint drag handles, and the
//! `WallDragEvent` the plan emits. `draw_project_plan` takes a `PlanView<'_>`
//! bundle to keep its call site legible (mirroring `AxonometricView` /
//! `DesignElevationView`).

use eframe::egui::{
    self, Align2, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use framer_core::{BuildingModel, Length, Point2};

use super::camera_2d::{View2dState, apply_view_2d_input, reset_view_on_empty_double_click};
use super::geom::{ModelBounds, distance_to_segment, plan_inverse_point, plan_point};
use super::view_common::{
    draw_dashed_line, draw_drafting_grid, draw_drafting_rulers, draw_plan_axis_indicator,
    draw_scale_bar, draw_view_background, draw_view_border, draw_view_empty, draw_view_title,
    viewport_drawing_rect, viewport_size,
};
use super::{DrawWallPlanInput, theme};
use crate::app::draw_wall::{GuideAxis, SnapContext, SnapKind, SnapResult, resolve_snap};
use crate::app::labels::join_kind_label;
use crate::app::model_edit::WallEditHandle;
use crate::app::{Selection, ViewClick};

/// Screen-pixel radius within which the draw tool *acquires* a snap. Converted to
/// model units per frame so the feel is constant across zoom levels.
const SNAP_ACQUIRE_PX: f64 = 12.0;
/// Screen-pixel radius a held snap must leave before it *releases* (hysteresis).
const SNAP_RELEASE_PX: f64 = 20.0;

/// A plan-view wall-endpoint drag, mirroring [`crate::app::model_edit::OpeningDragState`].
/// `Updated` carries the already-snapped model point for the dragged endpoint.
#[derive(Debug, Clone, Copy)]
pub(crate) enum WallDragEvent {
    Started {
        wall_index: usize,
        handle: WallEditHandle,
    },
    /// An endpoint handle moved to a snapped model point.
    Updated {
        point: Point2,
    },
    /// The body handle translated the whole wall by an incremental model delta.
    Translated {
        dx: Length,
        dy: Length,
    },
    Stopped,
}

/// Read-only plan-view inputs, grouped to match the `AxonometricView` /
/// `DesignElevationView` idiom. The `&mut camera` and `*_out` sinks stay separate
/// args (grouping mutable borrows would force a single combined borrow and lose
/// the disjoint-borrow ergonomics the call site relies on).
pub(super) struct PlanView<'a> {
    pub(super) model: &'a BuildingModel,
    pub(super) selected_wall: usize,
    pub(super) selection: &'a Selection,
    pub(super) show_grid: bool,
    pub(super) draw_tool: &'a DrawWallPlanInput,
    pub(super) room_tool_active: bool,
    pub(super) active_wall_drag: Option<(usize, WallEditHandle)>,
}

// === extracted plan block appended below; draw_project_plan signature reshaped ===

/// A draggable square handle; grows and thickens its outline when hovered.
fn draw_wall_handle(painter: &egui::Painter, point: Pos2, hovered: bool) {
    let size = if hovered { 11.0 } else { 8.0 };
    let handle = Rect::from_center_size(point, Vec2::splat(size));
    painter.rect_filled(handle, 1.5, theme::active_blue());
    painter.rect_stroke(
        handle,
        1.5,
        Stroke::new(if hovered { 2.0 } else { 1.0 }, theme::sheet()),
        StrokeKind::Outside,
    );
}

fn draw_selected_wall_handles(
    painter: &egui::Painter,
    start: Pos2,
    end: Pos2,
    hovered: Option<WallEditHandle>,
) {
    draw_wall_handle(painter, start, hovered == Some(WallEditHandle::Start));
    draw_wall_handle(painter, end, hovered == Some(WallEditHandle::End));
    // The midpoint handle grabs the whole wall (translate). It also lights up when
    // the body anywhere is hovered.
    draw_wall_handle(
        painter,
        Pos2::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0),
        hovered == Some(WallEditHandle::Body),
    );
}

/// Hit-test the start/end handles of the selected wall (endpoints only), within a
/// generous pixel radius. Returns the wall index and which handle.
#[allow(clippy::too_many_arguments)]
fn hit_selected_wall_handle(
    position: Pos2,
    model: &BuildingModel,
    selected_wall: usize,
    selection: &Selection,
    bounds: ModelBounds,
    drawing: Rect,
    camera: &View2dState,
) -> Option<(usize, WallEditHandle)> {
    if !matches!(selection, Selection::Wall) {
        return None;
    }
    let wall = model.walls.get(selected_wall)?;
    const HIT_RADIUS: f32 = 11.0;
    const BODY_HIT_RADIUS: f32 = 8.0;
    let start = plan_point(wall.start, bounds, drawing, camera);
    let end = plan_point(wall.end, bounds, drawing, camera);
    let start_distance = position.distance(start);
    let end_distance = position.distance(end);
    // Endpoints win; otherwise grabbing the wall body translates it.
    if start_distance <= HIT_RADIUS && start_distance <= end_distance {
        Some((selected_wall, WallEditHandle::Start))
    } else if end_distance <= HIT_RADIUS {
        Some((selected_wall, WallEditHandle::End))
    } else if distance_to_segment(position, start, end) <= BODY_HIT_RADIUS {
        Some((selected_wall, WallEditHandle::Body))
    } else {
        None
    }
}

/// Resolve a wall-endpoint drag to a snapped model point: ortho-locked to the
/// wall's fixed far end, snapping to other walls' endpoints/midpoints/alignment
/// (the moving node and its coincident neighbours are excluded).
#[allow(clippy::too_many_arguments)]
fn snapped_wall_endpoint(
    model: &BuildingModel,
    wall_index: usize,
    handle: WallEditHandle,
    cursor: Pos2,
    bounds: ModelBounds,
    drawing: Rect,
    camera: &View2dState,
    scale: f32,
    grid_step: Option<Length>,
    suspend: bool,
) -> Point2 {
    let raw = plan_inverse_point(cursor, bounds, drawing, camera);
    let Some(wall) = model.walls.get(wall_index) else {
        return raw;
    };
    let (anchor, node) = match handle {
        WallEditHandle::Start => (wall.end, wall.start),
        WallEditHandle::End => (wall.start, wall.end),
        // The body handle translates via incremental deltas, not snapped points.
        WallEditHandle::Body => return raw,
    };
    // Exclude every wall touching the node — they all move together, so none is a
    // valid snap target (and a coincident endpoint would otherwise freeze the drag).
    let exclude: Vec<framer_core::ElementId> = model
        .walls
        .iter()
        .filter(|candidate| candidate.start == node || candidate.end == node)
        .map(|candidate| candidate.id.clone())
        .collect();
    let inv_scale = (1.0 / scale.max(0.0001)) as f64;
    resolve_snap(&SnapContext {
        model,
        raw,
        anchor: Some(anchor),
        exclude: &exclude,
        tolerance: Length::from_inches(SNAP_ACQUIRE_PX * inv_scale),
        release_tolerance: Length::from_inches(SNAP_RELEASE_PX * inv_scale),
        grid_step,
        suspend,
        previous: None,
    })
    .point
}

pub(super) fn draw_project_plan(
    ui: &mut Ui,
    plan: PlanView<'_>,
    camera: &mut View2dState,
    cursor_out: &mut Option<Point2>,
    toolbar_out: &mut Option<Pos2>,
    snap_out: &mut Option<SnapResult>,
    wall_drag_out: &mut Option<WallDragEvent>,
) -> Option<ViewClick> {
    let PlanView {
        model,
        selected_wall,
        selection,
        show_grid,
        draw_tool,
        room_tool_active,
        active_wall_drag,
    } = plan;
    let desired = viewport_size(ui);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    draw_view_background(&painter, rect, theme::sheet());
    let drawing = viewport_drawing_rect(rect, 58.0);
    // Pan/zoom the view before mapping any model point. Space+primary-drag pans,
    // except while a wall handle is being dragged (so the two don't fight).
    apply_view_2d_input(ui, &response, drawing, camera, active_wall_drag.is_none());
    draw_drafting_rulers(&painter, rect, drawing);
    if show_grid {
        draw_drafting_grid(&painter, drawing);
    }
    draw_view_border(&painter, drawing);

    let bounds = match ModelBounds::from_model(model) {
        Some(bounds) => bounds,
        // An empty model has no bounds. When the draw-wall tool is active, fall
        // back to a default region around the origin so the user can still place
        // the first wall (which re-establishes real bounds next frame).
        None if draw_tool.active => ModelBounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 240.0,
            max_y: 240.0,
        },
        None => {
            draw_view_empty(&painter, rect, "No wall segments");
            return None;
        }
    };

    if let Some(hover) = response.hover_pos()
        && drawing.contains(hover)
    {
        *cursor_out = Some(plan_inverse_point(hover, bounds, drawing, camera));
    }

    let pointer = response.interact_pointer_pos();
    let mut clicked_wall = None;
    let mut clicked_opening = None;
    let mut clicked_room = None;
    let mut over_element = false;

    // Which selected-wall handle the cursor is over (for hover emphasis + cursor),
    // only in selection mode.
    let hovered_wall_handle = (!draw_tool.active && !room_tool_active)
        .then(|| {
            response.hover_pos().and_then(|hover| {
                hit_selected_wall_handle(hover, model, selected_wall, selection, bounds, drawing, camera)
            })
        })
        .flatten()
        .map(|(_, handle)| handle);

    // Room fills + labels, drawn under the walls. Boundaries are derived from the
    // wall loop each frame (never stored); resolve them all in one graph pass.
    let room_seeds: Vec<Point2> = model.rooms.iter().map(|room| room.seed).collect();
    let room_boundaries = framer_core::room_boundaries(model, &room_seeds);
    for (room, boundary) in model.rooms.iter().zip(&room_boundaries) {
        let Some(boundary) = boundary else {
            continue;
        };
        let screen: Vec<Pos2> = boundary
            .vertices
            .iter()
            .map(|vertex| plan_point(*vertex, bounds, drawing, camera))
            .collect();
        let selected = matches!(selection, Selection::Room(id) if id == &room.id.0);
        let fill = if selected {
            theme::active_blue().gamma_multiply(0.22)
        } else {
            theme::framing_line().gamma_multiply(0.10)
        };
        painter.add(egui::Shape::convex_polygon(
            screen.clone(),
            fill,
            Stroke::NONE,
        ));
        let label = plan_point(room.seed, bounds, drawing, camera);
        painter.text(
            label,
            Align2::CENTER_CENTER,
            format!("{}\n{:.0} sq ft", room.name, boundary.area_square_feet()),
            FontId::proportional(11.0),
            theme::framing_line_dark(),
        );
        // Selecting a room by click is the lowest-priority hit (walls/openings win),
        // and only when no tool is active.
        if !draw_tool.active
            && !room_tool_active
            && response.clicked()
            && pointer.is_some_and(|position| point_in_screen_polygon(position, &screen))
        {
            clicked_room = Some(ViewClick::Room {
                room_id: room.id.0.clone(),
            });
        }
    }

    for join in &model.wall_joins {
        let point = plan_point(join.point, bounds, drawing, camera);
        painter.circle_filled(point, 4.5, theme::active_blue());
        painter.text(
            point + Vec2::new(6.0, -7.0),
            Align2::LEFT_CENTER,
            join_kind_label(join.kind),
            FontId::proportional(10.0),
            theme::active_blue(),
        );
    }

    for (index, wall) in model.walls.iter().enumerate() {
        let start = plan_point(wall.start, bounds, drawing, camera);
        let end = plan_point(wall.end, bounds, drawing, camera);
        let hovered =
            pointer.is_some_and(|position| distance_to_segment(position, start, end) < 8.0);
        over_element |= hovered;
        let selected = selected_wall == index && matches!(selection, Selection::Wall);
        let stroke = if selected {
            Stroke::new(5.0, theme::active_blue())
        } else if hovered {
            Stroke::new(4.5, theme::framing_line_dark())
        } else {
            Stroke::new(3.5, theme::framing_line())
        };
        painter.line_segment([start, end], stroke);
        if selected {
            draw_selected_wall_handles(&painter, start, end, hovered_wall_handle);
        }
        if hovered && response.clicked() && !draw_tool.active && !room_tool_active {
            clicked_wall = Some(ViewClick::Wall(index));
        }

        let midpoint = Pos2::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
        painter.text(
            midpoint + Vec2::new(5.0, -10.0),
            Align2::LEFT_CENTER,
            &wall.name,
            FontId::proportional(12.0),
            theme::framing_line_dark(),
        );

        for opening in &wall.openings {
            let left = plan_point(
                wall.point_at_local_x(opening.left()),
                bounds,
                drawing,
                camera,
            );
            let right = plan_point(
                wall.point_at_local_x(opening.right()),
                bounds,
                drawing,
                camera,
            );
            let opening_hovered =
                pointer.is_some_and(|position| distance_to_segment(position, left, right) < 9.0);
            over_element |= opening_hovered;
            let opening_selected = matches!(selection, Selection::Opening(id) if id == &opening.id.0)
                && selected_wall == index;
            if opening_selected {
                *toolbar_out = Some(Pos2::new(
                    (left.x + right.x) / 2.0,
                    (left.y + right.y) / 2.0,
                ));
            }
            painter.line_segment([left, right], Stroke::new(7.0, theme::sheet()));
            painter.line_segment(
                [left, right],
                Stroke::new(
                    if opening_selected || opening_hovered {
                        3.0
                    } else {
                        2.0
                    },
                    if opening_selected {
                        theme::active_blue()
                    } else {
                        theme::framing_line()
                    },
                ),
            );
            if opening_hovered && response.clicked() && !draw_tool.active && !room_tool_active {
                clicked_opening = Some(ViewClick::Opening {
                    wall_index: index,
                    opening_id: opening.id.0.clone(),
                });
            }
        }
    }

    let scale = (drawing.width() / (bounds.max_x - bounds.min_x).max(1.0))
        .min(drawing.height() / (bounds.max_y - bounds.min_y).max(1.0))
        * camera.zoom;

    // Wall-endpoint editing (selection mode only): drag the selected wall's
    // start/end handles. The app owns the drag state and applies the events.
    if !draw_tool.active && !room_tool_active {
        if let Some((wall_index, handle)) = active_wall_drag {
            if response.drag_stopped() {
                *wall_drag_out = Some(WallDragEvent::Stopped);
            } else if response.dragged_by(egui::PointerButton::Primary) {
                if handle == WallEditHandle::Body {
                    // Whole-wall translate: total screen delta from drag start →
                    // model delta (y is flipped). The app accounts for what's been
                    // applied so the wall tracks the cursor absolutely.
                    if let Some(delta) = response.total_drag_delta() {
                        let inv_scale = (1.0 / scale.max(0.0001)) as f64;
                        let dx = Length::from_inches(delta.x as f64 * inv_scale);
                        let dy = Length::from_inches(-delta.y as f64 * inv_scale);
                        *wall_drag_out = Some(WallDragEvent::Translated { dx, dy });
                        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    }
                } else if let Some(cursor) = response.interact_pointer_pos() {
                    let suspend = ui.input(|input| input.modifiers.alt);
                    let point = snapped_wall_endpoint(
                        model,
                        wall_index,
                        handle,
                        cursor,
                        bounds,
                        drawing,
                        camera,
                        scale,
                        draw_tool.snap_step,
                        suspend,
                    );
                    *wall_drag_out = Some(WallDragEvent::Updated { point });
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
            }
        } else if response.drag_started_by(egui::PointerButton::Primary)
            && !ui.input(|input| input.key_down(egui::Key::Space))
            && let Some(hit) = ui
                .input(|input| input.pointer.press_origin())
                .and_then(|origin| {
                    hit_selected_wall_handle(
                        origin,
                        model,
                        selected_wall,
                        selection,
                        bounds,
                        drawing,
                        camera,
                    )
                })
        {
            *wall_drag_out = Some(WallDragEvent::Started {
                wall_index: hit.0,
                handle: hit.1,
            });
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if let Some(handle) = hovered_wall_handle {
            ui.ctx().set_cursor_icon(if handle == WallEditHandle::Body {
                CursorIcon::Move
            } else {
                CursorIcon::Grab
            });
        }
    }

    draw_scale_bar(&painter, drawing, scale);
    draw_view_title(&painter, drawing, "Whole-project plan");
    draw_plan_axis_indicator(&painter, drawing);

    // Skip double-click-to-refit while a placement tool is active, so a quick
    // second click that places a point/room doesn't also reset the camera.
    if !draw_tool.active && !room_tool_active {
        reset_view_on_empty_double_click(&response, camera, over_element);
    }

    if draw_tool.active
        && let Some(click) = draw_wall_overlay(
            &painter, &response, model, bounds, drawing, camera, scale, draw_tool, snap_out,
        )
    {
        return Some(click);
    }

    if room_tool_active && response.clicked() {
        if let Some(cursor) = response
            .interact_pointer_pos()
            .filter(|c| drawing.contains(*c))
        {
            return Some(ViewClick::PlaceRoom {
                point: plan_inverse_point(cursor, bounds, drawing, camera),
            });
        }
    }

    clicked_opening.or(clicked_wall).or(clicked_room)
}

/// Even-odd point-in-polygon test in screen space, for picking a room by click.
fn point_in_screen_polygon(point: Pos2, vertices: &[Pos2]) -> bool {
    if vertices.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = vertices.len() - 1;
    for i in 0..vertices.len() {
        let (xi, yi) = (vertices[i].x, vertices[i].y);
        let (xj, yj) = (vertices[j].x, vertices[j].y);
        if (yi > point.y) != (yj > point.y) && point.x < (xj - xi) * (point.y - yi) / (yj - yi) + xi
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Render the draw-wall tool's live preview (snap marker, rubber band, length
/// readout) and translate pointer input into draw clicks. Returns a draw-wall
/// `ViewClick` on a primary click (place a point) or secondary click (cancel
/// the run).
#[allow(clippy::too_many_arguments)]
fn draw_wall_overlay(
    painter: &egui::Painter,
    response: &egui::Response,
    model: &BuildingModel,
    bounds: ModelBounds,
    drawing: Rect,
    camera: &View2dState,
    scale: f32,
    draw_tool: &DrawWallPlanInput,
    snap_out: &mut Option<SnapResult>,
) -> Option<ViewClick> {
    if response.secondary_clicked() {
        return Some(ViewClick::DrawWallCancel);
    }

    let cursor = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos())?;
    if !drawing.contains(cursor) {
        return None;
    }

    let raw = plan_inverse_point(cursor, bounds, drawing, camera);
    // Tolerances are a constant screen-pixel distance converted to model units, so
    // the snap feels the same at every zoom. The release radius is larger than the
    // acquire radius so a held snap stays put instead of flickering (hysteresis).
    let inv_scale = (1.0 / scale.max(0.0001)) as f64;
    let tolerance = Length::from_inches(SNAP_ACQUIRE_PX * inv_scale);
    let release_tolerance = Length::from_inches(SNAP_RELEASE_PX * inv_scale);
    // Alt suspends snapping for precise free placement.
    let suspend = response.ctx.input(|input| input.modifiers.alt);

    let resolved = resolve_snap(&SnapContext {
        model,
        raw,
        anchor: draw_tool.start,
        exclude: &[],
        tolerance,
        release_tolerance,
        grid_step: draw_tool.snap_step,
        suspend,
        previous: draw_tool.previous_snap,
    });
    *snap_out = Some(resolved);
    let candidate = plan_point(resolved.point, bounds, drawing, camera);

    if let Some(start) = draw_tool.start {
        let start_screen = plan_point(start, bounds, drawing, camera);
        painter.line_segment(
            [start_screen, candidate],
            Stroke::new(2.5, theme::active_blue()),
        );
        painter.circle_filled(start_screen, 4.0, theme::active_blue());

        // Walls stay ortho, so exactly one axis differs; max gives the length.
        let length = (resolved.point.x - start.x)
            .abs()
            .max((resolved.point.y - start.y).abs());
        if length > Length::ZERO {
            let mid = Pos2::new(
                (start_screen.x + candidate.x) / 2.0,
                (start_screen.y + candidate.y) / 2.0,
            );
            painter.text(
                mid + Vec2::new(8.0, -8.0),
                Align2::LEFT_CENTER,
                length.to_string(),
                FontId::proportional(12.0),
                theme::active_blue(),
            );
        }
    }

    // Inference guides, drawn under the indicator so the marker stays legible.
    let guide_stroke = Stroke::new(1.0, theme::active_blue_soft());
    for guide in resolved.guides.iter().flatten() {
        let (a, b) = match guide.axis {
            GuideAxis::Vertical => {
                let x = plan_point(Point2::new(guide.at, guide.source.y), bounds, drawing, camera).x;
                (Pos2::new(x, drawing.top()), Pos2::new(x, drawing.bottom()))
            }
            GuideAxis::Horizontal => {
                let y = plan_point(Point2::new(guide.source.x, guide.at), bounds, drawing, camera).y;
                (Pos2::new(drawing.left(), y), Pos2::new(drawing.right(), y))
            }
        };
        draw_dashed_line(painter, a, b, guide_stroke);
        let source_screen = plan_point(guide.source, bounds, drawing, camera);
        painter.circle_stroke(source_screen, 3.0, guide_stroke);
    }

    draw_snap_indicator(painter, candidate, resolved.kind, suspend);

    response.clicked().then_some(ViewClick::DrawWallPoint {
        point: resolved.point,
    })
}

/// Draw a snap marker whose glyph identifies *what* the cursor snapped to, so the
/// user can tell an endpoint lock from a midpoint, mid-wall, or grid snap. When
/// snapping is suspended (Alt), a hint is shown instead of a geometry glyph.
fn draw_snap_indicator(painter: &egui::Painter, at: Pos2, kind: SnapKind, suspend: bool) {
    let color = theme::active_blue();
    let stroke = Stroke::new(2.0, color);

    if suspend {
        painter.circle_filled(at, 3.0, color);
        painter.text(
            at + Vec2::new(10.0, -10.0),
            Align2::LEFT_CENTER,
            "snap off",
            FontId::proportional(11.0),
            color,
        );
        return;
    }

    match kind {
        SnapKind::Endpoint => {
            // Filled square inside a ring.
            painter.rect_filled(Rect::from_center_size(at, Vec2::splat(7.0)), 1.0, color);
            painter.rect_stroke(
                Rect::from_center_size(at, Vec2::splat(14.0)),
                1.0,
                stroke,
                StrokeKind::Outside,
            );
        }
        SnapKind::Midpoint => {
            // Upward triangle outline.
            let r = 6.0;
            let top = Pos2::new(at.x, at.y - r);
            let left = Pos2::new(at.x - r, at.y + r * 0.7);
            let right = Pos2::new(at.x + r, at.y + r * 0.7);
            painter.line_segment([top, left], stroke);
            painter.line_segment([left, right], stroke);
            painter.line_segment([right, top], stroke);
        }
        SnapKind::OnWall => {
            // Diamond outline (lands on a wall's interior → Tee).
            let r = 6.0;
            let top = Pos2::new(at.x, at.y - r);
            let right = Pos2::new(at.x + r, at.y);
            let bottom = Pos2::new(at.x, at.y + r);
            let left = Pos2::new(at.x - r, at.y);
            painter.line_segment([top, right], stroke);
            painter.line_segment([right, bottom], stroke);
            painter.line_segment([bottom, left], stroke);
            painter.line_segment([left, top], stroke);
        }
        SnapKind::Intersection => {
            // Crossing of two guides — an X.
            let r = 6.0;
            painter.line_segment(
                [Pos2::new(at.x - r, at.y - r), Pos2::new(at.x + r, at.y + r)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(at.x - r, at.y + r), Pos2::new(at.x + r, at.y - r)],
                stroke,
            );
        }
        SnapKind::Alignment => {
            // Hollow circle; the dashed guide line conveys the alignment.
            painter.circle_stroke(at, 4.5, stroke);
        }
        SnapKind::Grid => {
            // Small plus.
            let r = 5.0;
            painter.line_segment([Pos2::new(at.x - r, at.y), Pos2::new(at.x + r, at.y)], stroke);
            painter.line_segment([Pos2::new(at.x, at.y - r), Pos2::new(at.x, at.y + r)], stroke);
        }
        SnapKind::Free => {
            painter.circle_filled(at, 3.5, color);
        }
    }

    if let Some(label) = snap_kind_label(kind) {
        painter.text(
            at + Vec2::new(10.0, -10.0),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(11.0),
            color,
        );
    }
}

/// Short label for a snap kind, or `None` for kinds that need no annotation.
fn snap_kind_label(kind: SnapKind) -> Option<&'static str> {
    match kind {
        SnapKind::Endpoint => Some("end"),
        SnapKind::Midpoint => Some("mid"),
        SnapKind::OnWall => Some("wall"),
        SnapKind::Alignment => Some("align"),
        SnapKind::Intersection | SnapKind::Grid | SnapKind::Free => None,
    }
}
