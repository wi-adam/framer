//! The plan (top-down) renderer: the drafting grid/rulers, wall + opening
//! drawing, the draw-wall tool with snapping, wall-endpoint drag handles, and the
//! `WallDragEvent` the plan emits. `draw_project_plan` takes a `PlanView<'_>`
//! bundle to keep its call site legible (mirroring `AxonometricView` /
//! `DesignElevationView`).

use std::collections::BTreeMap;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use framer_core::{BuildingModel, ElementId, Length, Point2, QuarterTurn, Wall};

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
use crate::app::{Selection, ViewClick, ViewLayers, WallDisplay};

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
    pub(super) layers: ViewLayers,
    pub(super) draw_tool: &'a DrawWallPlanInput,
    pub(super) room_tool_active: bool,
    pub(super) ceiling_tool_active: bool,
    pub(super) floor_tool_active: bool,
    /// The top-down roof authoring view: overlay the authored roof-plane outlines
    /// and let clicks select them.
    pub(super) roof_plan_mode: bool,
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

/// The 2D part of `scene_build::WallBasis`: the wall's along/perpendicular unit
/// vectors in model space (inches), used to offset the centerline into thickness
/// bands. The side axis is `(-along_y, along_x)` — the same convention the 3D
/// renderer uses — so plan and 3D agree on which way "exterior" lies.
struct PlanWallBasis {
    side_x: f64,
    side_y: f64,
}

impl PlanWallBasis {
    fn new(wall: &Wall) -> Self {
        let dx = (wall.end.x - wall.start.x).inches();
        let dy = (wall.end.y - wall.start.y).inches();
        let length = (dx * dx + dy * dy).sqrt().max(1.0);
        let along_x = dx / length;
        let along_y = dy / length;
        Self {
            side_x: -along_y,
            side_y: along_x,
        }
    }

    /// Offset a centerline model point perpendicular to the wall by `side` inches.
    fn offset(&self, point: Point2, side: f64) -> Point2 {
        Point2::new(
            point.x + Length::from_inches(self.side_x * side),
            point.y + Length::from_inches(self.side_y * side),
        )
    }
}

/// A band quad spanning `start..end` along the wall and `side0..side1` across its
/// thickness, projected to screen so its fill scales with zoom. Corners are wound
/// interior(side0)-start → interior-end → exterior(side1)-end → exterior-start.
#[allow(clippy::too_many_arguments)]
fn band_quad(
    basis: &PlanWallBasis,
    start: Point2,
    end: Point2,
    side0: f64,
    side1: f64,
    bounds: ModelBounds,
    drawing: Rect,
    camera: &View2dState,
) -> [Pos2; 4] {
    [
        plan_point(basis.offset(start, side0), bounds, drawing, camera),
        plan_point(basis.offset(end, side0), bounds, drawing, camera),
        plan_point(basis.offset(end, side1), bounds, drawing, camera),
        plan_point(basis.offset(start, side1), bounds, drawing, camera),
    ]
}

/// Which way a wall's layer stack runs on the side axis (`(-along_y, along_x)`):
/// `+1` when the room interior is toward the plus-side, `-1` when toward the
/// minus-side. Walls absent from the topology map (ambiguous partitions / no
/// enclosing room) DEFAULT to `-1`, matching the 3D renderer so plan and 3D agree.
fn interior_sign(interior_sides: &BTreeMap<ElementId, bool>, wall_id: &ElementId) -> f64 {
    match interior_sides.get(wall_id) {
        Some(true) => 1.0,
        _ => -1.0,
    }
}

/// The side-axis span `[min, max]` (inches) of one layer band, laid out interior
/// -> exterior, mirroring `scene_build::layer_band_span` so plan and 3D place each
/// layer on the same physical side. With cumulative interior offset `off` and
/// thickness `t` the interior face is at `interior_sign * (total/2 - off)` and the
/// exterior face at `interior_sign * (total/2 - (off + t))`.
fn layer_band_span(interior_sign: f64, total: f64, off: f64, t: f64) -> (f64, f64) {
    let half = total / 2.0;
    let side_a = interior_sign * (half - off);
    let side_b = interior_sign * (half - (off + t));
    (side_a.min(side_b), side_a.max(side_b))
}

/// A material's plan-fill color, with a fallback neutral tone for a dangling id.
fn plan_layer_color(model: &BuildingModel, id: &framer_core::ElementId) -> Color32 {
    match model.material(id) {
        Some(material) => {
            let [r, g, b] = material.color();
            Color32::from_rgb(r, g, b)
        }
        None => Color32::from_rgb(188, 179, 158),
    }
}

/// Fill the wall as a true-thickness layered cross-section: one band quad per
/// construction layer, stacked interior -> exterior across `total_thickness`,
/// each filled with its material color. `interior_sign` (from topology) decides
/// which physical side faces the room, so reversing a wall no longer mirrors the
/// assembly; the stack still spans the full thickness across the centerline.
/// Openings widen to a full-thickness white gap drawn on top. This is FILL ONLY —
/// the centerline hit-test, drag handles, snapping, and selection/hover stroke
/// are unchanged and drawn over these bands.
fn draw_wall_layers(
    painter: &egui::Painter,
    model: &BuildingModel,
    wall: &Wall,
    interior_sign: f64,
    bounds: ModelBounds,
    drawing: Rect,
    camera: &View2dState,
) {
    let basis = PlanWallBasis::new(wall);
    match model.system_for(wall) {
        Some(system) => {
            let total = system.total_thickness().inches();
            let mut off = 0.0;
            for layer in &system.layers {
                let thickness = layer.thickness.inches();
                let (side0, side1) = layer_band_span(interior_sign, total, off, thickness);
                off += thickness;
                let color = plan_layer_color(model, &layer.material);
                let quad = band_quad(
                    &basis, wall.start, wall.end, side0, side1, bounds, drawing, camera,
                );
                painter.add(egui::Shape::convex_polygon(
                    quad.to_vec(),
                    color,
                    Stroke::NONE,
                ));
            }
        }
        // Degenerate model with no resolvable system: fall back to the code stud
        // depth so the wall still reads as a thick band.
        None => {
            let total = model.code.stud_profile.nominal_depth().inches();
            let (side0, side1) = layer_band_span(interior_sign, total, 0.0, total);
            let quad = band_quad(
                &basis, wall.start, wall.end, side0, side1, bounds, drawing, camera,
            );
            painter.add(egui::Shape::convex_polygon(
                quad.to_vec(),
                Color32::from_rgb(188, 179, 158),
                Stroke::NONE,
            ));
        }
    }
}

/// Draw the wall's full plan thickness as two parallel DASHED face lines (a
/// "width" outline) with no color fill — the [`WallDisplay::Width`] mode. The
/// faces are the wall's long edges at `±half` thickness on the side axis;
/// `band_quad` projects them so they scale with zoom. Uses `wall_plan_thickness`
/// so it works with or without a resolved construction system. The centerline,
/// handles, and hit-test are drawn over this by the caller, unchanged. No end
/// caps: walls butt at joins, so capping each independently would double-draw.
fn draw_wall_width(
    painter: &egui::Painter,
    model: &BuildingModel,
    wall: &Wall,
    bounds: ModelBounds,
    drawing: Rect,
    camera: &View2dState,
) {
    let half = wall_plan_thickness(model, wall) / 2.0;
    let basis = PlanWallBasis::new(wall);
    // band_quad winds [start@-half, end@-half, end@+half, start@+half], so the two
    // long faces are [0,1] (minus side) and [3,2] (plus side).
    let quad = band_quad(
        &basis, wall.start, wall.end, -half, half, bounds, drawing, camera,
    );
    let stroke = Stroke::new(1.0, theme::framing_line());
    draw_dashed_line(painter, quad[0], quad[1], stroke);
    draw_dashed_line(painter, quad[3], quad[2], stroke);
}

/// Widen an opening's gap to the wall's full thickness: a white band quad over the
/// opening span, drawn on top of the layer bands so the cut reads at any zoom.
#[allow(clippy::too_many_arguments)]
fn draw_opening_gap(
    painter: &egui::Painter,
    model: &BuildingModel,
    wall: &Wall,
    left: Point2,
    right: Point2,
    bounds: ModelBounds,
    drawing: Rect,
    camera: &View2dState,
) {
    let total = wall_plan_thickness(model, wall);
    let basis = PlanWallBasis::new(wall);
    let half = total / 2.0;
    let quad = band_quad(&basis, left, right, -half, half, bounds, drawing, camera);
    painter.add(egui::Shape::convex_polygon(
        quad.to_vec(),
        theme::sheet(),
        Stroke::NONE,
    ));
}

/// The wall's plan-fill total thickness in inches: its system's `total_thickness`,
/// or the code stud depth when no system resolves.
fn wall_plan_thickness(model: &BuildingModel, wall: &Wall) -> f64 {
    model
        .system_for(wall)
        .map(|system| system.total_thickness().inches())
        .unwrap_or_else(|| model.code.stud_profile.nominal_depth().inches())
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
        layers,
        draw_tool,
        room_tool_active,
        ceiling_tool_active,
        floor_tool_active,
        roof_plan_mode,
        active_wall_drag,
    } = plan;
    // The room, ceiling, and floor tools are all region-gated placement tools:
    // while any is active, a click drops its object rather than selecting or
    // editing, so the wall-handle/selection interactions are all suppressed.
    let region_tool_active = room_tool_active || ceiling_tool_active || floor_tool_active;
    let desired = viewport_size(ui);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    draw_view_background(&painter, rect, theme::sheet());
    let drawing = viewport_drawing_rect(rect, 58.0);
    // Pan/zoom the view before mapping any model point. Space+primary-drag pans,
    // except while a wall handle is being dragged (so the two don't fight).
    apply_view_2d_input(ui, &response, drawing, camera, active_wall_drag.is_none());
    draw_drafting_rulers(&painter, rect, drawing);
    if layers.grid {
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
    let mut clicked_furnishing = None;
    let mut clicked_mep = None;
    let mut clicked_room = None;
    let mut clicked_roof = None;
    let mut over_element = false;

    // Which selected-wall handle the cursor is over (for hover emphasis + cursor),
    // only in selection mode.
    let hovered_wall_handle = (!draw_tool.active && !region_tool_active)
        .then(|| {
            response.hover_pos().and_then(|hover| {
                hit_selected_wall_handle(
                    hover,
                    model,
                    selected_wall,
                    selection,
                    bounds,
                    drawing,
                    camera,
                )
            })
        })
        .flatten()
        .map(|(_, handle)| handle);

    // Which side of each wall faces the room interior, derived from topology so the
    // layered fill lays out interior -> exterior independent of wall winding
    // (matching the 3D renderer). Only the `Full` wall mode reads this, and the
    // derivation is a non-trivial graph pass, so compute it once per frame only when
    // that mode is active (mirroring the room-boundary guard below).
    let interior_sides = matches!(layers.wall_display, WallDisplay::Full)
        .then(|| framer_core::wall_interior_sides(model));

    // Room fills + labels, drawn under the walls. Boundaries are derived from the
    // wall loop each frame (never stored); resolve them all in one graph pass.
    // When the Rooms layer is hidden, skip the graph pass entirely — the empty
    // boundary list makes the draw/pick loop below a no-op.
    let room_seeds: Vec<Point2> = model.rooms.iter().map(|room| room.seed).collect();
    let room_boundaries = if layers.rooms {
        framer_core::room_boundaries(model, &room_seeds)
    } else {
        Vec::new()
    };
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
            && !region_tool_active
            && response.clicked()
            && pointer.is_some_and(|position| point_in_screen_polygon(position, &screen))
        {
            clicked_room = Some(ViewClick::Room {
                room_id: room.id.0.clone(),
            });
        }
    }

    for instance in &model.furnishing_instances {
        let Some(family) = model
            .furnishings
            .iter()
            .find(|family| family.id == instance.family)
        else {
            continue;
        };
        let rect = object_footprint_rect(
            instance.position,
            family.size.width,
            family.size.depth,
            instance.rotation,
            bounds,
            drawing,
            camera,
        );
        let hovered = pointer.is_some_and(|position| rect.contains(position));
        over_element |= hovered;
        let selected =
            matches!(selection, Selection::FurnishingInstance(id) if id == &instance.id.0);
        draw_object_footprint(
            &painter,
            rect,
            &instance.name,
            selected,
            hovered,
            Color32::from_rgb(190, 172, 132),
        );
        if hovered && response.clicked() && !draw_tool.active && !region_tool_active {
            clicked_furnishing = Some(ViewClick::FurnishingInstance {
                instance_id: instance.id.0.clone(),
            });
        }
    }

    for instance in &model.mep_instances {
        let Some(family) = model
            .mep_objects
            .iter()
            .find(|family| family.id == instance.family)
        else {
            continue;
        };
        let rect = object_footprint_rect(
            instance.position,
            family.size.width,
            family.size.depth,
            instance.rotation,
            bounds,
            drawing,
            camera,
        );
        let hovered = pointer.is_some_and(|position| rect.contains(position));
        over_element |= hovered;
        let selected = matches!(selection, Selection::MepInstance(id) if id == &instance.id.0);
        draw_object_footprint(
            &painter,
            rect,
            &instance.name,
            selected,
            hovered,
            Color32::from_rgb(124, 162, 186),
        );
        if hovered && response.clicked() && !draw_tool.active && !region_tool_active {
            clicked_mep = Some(ViewClick::MepInstance {
                instance_id: instance.id.0.clone(),
            });
        }
    }

    if layers.joins {
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
    }

    for (index, wall) in model.walls.iter().enumerate() {
        let start = plan_point(wall.start, bounds, drawing, camera);
        let end = plan_point(wall.end, bounds, drawing, camera);
        let hovered =
            pointer.is_some_and(|position| distance_to_segment(position, start, end) < 8.0);
        over_element |= hovered;
        let selected = selected_wall == index && matches!(selection, Selection::Wall);
        // The wall body, per display mode. The centerline stroke is drawn on top
        // in every mode (hit-testing, handles, and snapping all stay on the
        // centerline below), so selection/hover read regardless of mode.
        //   Outline -> centerline only (no body)
        //   Width   -> two dashed face lines at the full thickness
        //   Full    -> true-thickness colored construction-layer bands
        match layers.wall_display {
            WallDisplay::Outline => {}
            WallDisplay::Width => draw_wall_width(&painter, model, wall, bounds, drawing, camera),
            WallDisplay::Full => {
                // `interior_sides` is `Some` in Full mode (guarded above).
                if let Some(interior_sides) = &interior_sides {
                    let sign = interior_sign(interior_sides, &wall.id);
                    draw_wall_layers(&painter, model, wall, sign, bounds, drawing, camera);
                }
            }
        }
        let stroke = if selected {
            Stroke::new(2.5, theme::active_blue())
        } else if hovered {
            Stroke::new(2.0, theme::framing_line_dark())
        } else {
            Stroke::new(1.0, theme::framing_line_dark())
        };
        painter.line_segment([start, end], stroke);
        if selected {
            draw_selected_wall_handles(&painter, start, end, hovered_wall_handle);
        }
        if hovered && response.clicked() && !draw_tool.active && !region_tool_active {
            clicked_wall = Some(ViewClick::Wall(index));
        }

        if layers.wall_labels {
            let midpoint = Pos2::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
            painter.text(
                midpoint + Vec2::new(5.0, -10.0),
                Align2::LEFT_CENTER,
                &wall.name,
                FontId::proportional(12.0),
                theme::framing_line_dark(),
            );
        }

        for opening in &wall.openings {
            let left_model = wall.point_at_local_x(opening.left());
            let right_model = wall.point_at_local_x(opening.right());
            let left = plan_point(left_model, bounds, drawing, camera);
            let right = plan_point(right_model, bounds, drawing, camera);
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
            // Widen the gap to the wall's full thickness: a white band quad cuts
            // through the layered fill, then a thin line marks the opening run.
            // Only `Full` has a fill to cut — in Outline/Width the white band
            // would just erase the grid, so skip it and let the marker line carry
            // the opening (drawn in every mode below).
            if layers.wall_display == WallDisplay::Full {
                draw_opening_gap(
                    &painter,
                    model,
                    wall,
                    left_model,
                    right_model,
                    bounds,
                    drawing,
                    camera,
                );
            }
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
            if opening_hovered && response.clicked() && !draw_tool.active && !region_tool_active {
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
    if !draw_tool.active && !region_tool_active {
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

    // Roof authoring overlay: draw each roof plane's plan outline on top of the
    // footprint and let a click select it. Only in the roof-plan view, so the
    // normal plan stays uncluttered (roofs always render in 3D regardless).
    if roof_plan_mode {
        for plane in &model.roof_planes {
            if plane.outline.len() < 3 {
                continue;
            }
            let screen: Vec<Pos2> = plane
                .outline
                .iter()
                .map(|vertex| plan_point(*vertex, bounds, drawing, camera))
                .collect();
            let selected = matches!(selection, Selection::RoofPlane(id) if id == &plane.id.0);
            let fill = if selected {
                theme::active_blue().gamma_multiply(0.20)
            } else {
                theme::framing_line().gamma_multiply(0.08)
            };
            painter.add(egui::Shape::convex_polygon(
                screen.clone(),
                fill,
                Stroke::new(
                    if selected { 2.0 } else { 1.0 },
                    if selected {
                        theme::active_blue()
                    } else {
                        theme::framing_line()
                    },
                ),
            ));
            // Mark the eave edge so the slope direction reads at a glance.
            let i = plane.eave_edge as usize % screen.len();
            painter.line_segment(
                [screen[i], screen[(i + 1) % screen.len()]],
                Stroke::new(2.5, theme::framing_line_dark()),
            );
            if let Some(centroid) = polygon_centroid(&screen) {
                painter.text(
                    centroid,
                    Align2::CENTER_CENTER,
                    &plane.name,
                    FontId::proportional(11.0),
                    theme::framing_line_dark(),
                );
            }
            if response.clicked()
                && pointer.is_some_and(|position| point_in_screen_polygon(position, &screen))
            {
                clicked_roof = Some(ViewClick::RoofPlane {
                    id: plane.id.0.clone(),
                });
            }
        }
    }

    draw_scale_bar(&painter, drawing, scale);
    draw_view_title(
        &painter,
        drawing,
        if roof_plan_mode {
            "Roof plan"
        } else {
            "Whole-project plan"
        },
    );
    draw_plan_axis_indicator(&painter, drawing);

    // Skip double-click-to-refit while a placement tool is active, so a quick
    // second click that places a point/room doesn't also reset the camera.
    if !draw_tool.active && !region_tool_active {
        reset_view_on_empty_double_click(&response, camera, over_element);
    }

    if draw_tool.active
        && let Some(click) = draw_wall_overlay(
            &painter, &response, model, bounds, drawing, camera, scale, draw_tool, snap_out,
        )
    {
        return Some(click);
    }

    if region_tool_active
        && response.clicked()
        && let Some(cursor) = response
            .interact_pointer_pos()
            .filter(|c| drawing.contains(*c))
    {
        let point = plan_inverse_point(cursor, bounds, drawing, camera);
        // Exactly one region tool is active at a time (activating one cancels the
        // others), so dispatch to whichever placed the click.
        let click = if ceiling_tool_active {
            ViewClick::PlaceCeiling { point }
        } else if floor_tool_active {
            ViewClick::PlaceFloor { point }
        } else {
            ViewClick::PlaceRoom { point }
        };
        return Some(click);
    }

    // In the roof-plan view a roof outline is the top-priority hit; elsewhere the
    // ordinary wall/opening/room priority applies (clicked_roof is always None).
    clicked_roof
        .or(clicked_opening)
        .or(clicked_furnishing)
        .or(clicked_mep)
        .or(clicked_wall)
        .or(clicked_room)
}

/// The average of `vertices` (a screen-space polygon's label anchor). `None` for
/// an empty polygon.
fn polygon_centroid(vertices: &[Pos2]) -> Option<Pos2> {
    if vertices.is_empty() {
        return None;
    }
    let sum = vertices
        .iter()
        .fold(Vec2::ZERO, |acc, vertex| acc + vertex.to_vec2());
    Some((sum / vertices.len() as f32).to_pos2())
}

#[allow(clippy::too_many_arguments)]
fn object_footprint_rect(
    position: Point2,
    width: Length,
    depth: Length,
    rotation: QuarterTurn,
    bounds: ModelBounds,
    drawing: Rect,
    camera: &View2dState,
) -> Rect {
    let rotated = matches!(rotation, QuarterTurn::Deg90 | QuarterTurn::Deg270);
    let footprint_width = if rotated { depth } else { width };
    let footprint_depth = if rotated { width } else { depth };
    let half_width = footprint_width / 2;
    let half_depth = footprint_depth / 2;
    Rect::from_two_pos(
        plan_point(
            Point2::new(position.x - half_width, position.y - half_depth),
            bounds,
            drawing,
            camera,
        ),
        plan_point(
            Point2::new(position.x + half_width, position.y + half_depth),
            bounds,
            drawing,
            camera,
        ),
    )
}

fn draw_object_footprint(
    painter: &egui::Painter,
    rect: Rect,
    name: &str,
    selected: bool,
    hovered: bool,
    color: Color32,
) {
    let fill = if selected {
        theme::active_blue().gamma_multiply(0.25)
    } else {
        color.gamma_multiply(0.35)
    };
    let stroke = if selected {
        Stroke::new(2.0, theme::active_blue())
    } else if hovered {
        Stroke::new(1.5, theme::framing_line_dark())
    } else {
        Stroke::new(1.0, color.gamma_multiply(0.9))
    };
    painter.rect_filled(rect, 2.0, fill);
    painter.rect_stroke(rect, 2.0, stroke, StrokeKind::Outside);
    let estimated_text_width = name.chars().count() as f32 * 5.8;
    if rect.width() >= estimated_text_width + 8.0 && rect.height() >= 22.0 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            name,
            FontId::proportional(10.0),
            theme::framing_line_dark(),
        );
    }
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
                let x = plan_point(
                    Point2::new(guide.at, guide.source.y),
                    bounds,
                    drawing,
                    camera,
                )
                .x;
                (Pos2::new(x, drawing.top()), Pos2::new(x, drawing.bottom()))
            }
            GuideAxis::Horizontal => {
                let y = plan_point(
                    Point2::new(guide.source.x, guide.at),
                    bounds,
                    drawing,
                    camera,
                )
                .y;
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
            painter.line_segment(
                [Pos2::new(at.x - r, at.y), Pos2::new(at.x + r, at.y)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(at.x, at.y - r), Pos2::new(at.x, at.y + r)],
                stroke,
            );
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
