use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Frame, Margin, Mesh, Pos2, Rect, RichText, Sense,
    Shape, Stroke, StrokeKind, Ui, Vec2, epaint::Vertex,
};
use eframe::{egui_wgpu, wgpu};
use framer_core::{
    BuildingModel, DimensionAnchor, DimensionAxis, DimensionHorizontalReference, DimensionKind,
    DimensionVerticalReference, Length, Opening, OpeningKind, Point2, Wall,
};
use framer_solver::{FrameMember, MemberKind, MemberOrientation, ProjectFramePlan};

use super::draw_wall::{GuideAxis, SnapContext, SnapKind, SnapResult, resolve_snap};
use super::labels::{join_kind_label, kind_label};
use super::model_edit::{OpeningDragState, OpeningEditHandle, WallEditHandle};
use super::{FramerApp, Selection, ViewClick, ViewportMode, WorkspaceMode, design, theme};

mod camera_2d;
pub(super) use camera_2d::View2dState;
use camera_2d::{apply_view_2d_input, reset_view_on_empty_double_click};

mod camera_3d;
pub(super) use camera_3d::View3dState;
use camera_3d::{ViewCubeAction, ViewCubeOrientation};
// Referenced only from the `tests` module below (their non-test users moved into
// camera_3d), so gate the imports to keep non-test builds warning-clean.
#[cfg(test)]
use camera_3d::{DOLLY_MAX, DOLLY_MIN, PAN_MAX_RADII};
#[cfg(test)]
use framer_render::math::Vec3;
#[cfg(test)]
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

mod geom;
// Glob during migration: the still-monolithic renderers below reference many geom
// items. Narrowed to explicit imports as each renderer extracts into its own file.
use geom::*;

mod view_common;
use view_common::*;

mod gpu;
use gpu::*;

/// Plan-view input for the draw-wall tool: whether it is active, the in-progress
/// run's start point, the active grid snap increment, and the snap held from the
/// previous frame (for sticky hysteresis).
pub(super) struct DrawWallPlanInput {
    pub(super) active: bool,
    pub(super) start: Option<Point2>,
    pub(super) snap_step: Option<Length>,
    pub(super) previous_snap: Option<SnapResult>,
}

/// Screen-pixel radius within which the draw tool *acquires* a snap. Converted to
/// model units per frame so the feel is constant across zoom levels.
const SNAP_ACQUIRE_PX: f64 = 12.0;
/// Screen-pixel radius a held snap must leave before it *releases* (hysteresis).
const SNAP_RELEASE_PX: f64 = 20.0;

/// Frames to stay in reduced-resolution "moving" mode after the last camera
/// input, so a continuous orbit (which produces frequent tiny inputs) doesn't
/// flicker between resolution modes.
const MOTION_COOLDOWN_FRAMES: u32 = 6;
/// Internal-resolution scale for the Render view while the camera is moving
/// (0.5 ⇒ quarter the pixels, ~4× faster per frame).
const MOTION_RESOLUTION_SCALE: f32 = 0.5;

impl FramerApp {
    pub(super) fn workspace(&mut self, ui: &mut Ui) {
        workspace_header(
            ui,
            self.workspace_mode,
            self.viewport_mode,
            self.model.code.display_name.as_str(),
        );
        ui.add_space(8.0);

        let canvas = Rect::from_min_size(ui.next_widget_position(), viewport_size(ui));
        self.cursor_model = None;
        let mut toolbar_anchor = None;
        // The draw tool's resolved snap for this frame, written back into tool
        // state so the next frame can apply sticky hysteresis.
        let mut snap_out: Option<SnapResult> = None;
        // The active wall-endpoint drag (state owned here) and the event the plan
        // emits for it this frame.
        let active_wall_drag = self
            .wall_drag
            .map(|drag| (drag.wall_index, drag.handle));
        let mut wall_drag_out: Option<WallDragEvent> = None;
        let click = match self.viewport_mode {
            ViewportMode::Plan => {
                let draw_tool = DrawWallPlanInput {
                    active: self.draw_wall_tool.active,
                    start: self.draw_wall_tool.start,
                    snap_step: self.snap_step,
                    previous_snap: self.draw_wall_tool.previous_snap,
                };
                draw_project_plan(
                    ui,
                    &self.model,
                    self.selected_wall,
                    &self.selected,
                    self.grid,
                    &mut self.cursor_model,
                    &mut toolbar_anchor,
                    &mut self.plan_view,
                    &draw_tool,
                    self.room_tool_active,
                    &mut snap_out,
                    active_wall_drag,
                    &mut wall_drag_out,
                )
            }
            ViewportMode::Elevation => {
                let Some(wall) = self.model.walls.get(self.selected_wall) else {
                    ui.label("No wall selected");
                    return;
                };
                // Per-wall camera, shared across both elevation variants and
                // remembered for the session (materializes on first view).
                let camera = self.elevation_views.entry(wall.id.0.clone()).or_default();
                if !self.workspace_mode.shows_generated_plan() {
                    let selected_opening = match &self.selected {
                        Selection::Opening(id) => Some(id.as_str()),
                        _ => None,
                    };
                    let selected_dimension = match &self.selected {
                        Selection::Dimension(id) => Some(id.as_str()),
                        _ => None,
                    };
                    let first_anchor = self
                        .dimension_tool
                        .first_anchor
                        .as_ref()
                        .filter(|pick| pick.wall_index == self.selected_wall)
                        .map(|pick| &pick.anchor);
                    let second_anchor = self
                        .dimension_tool
                        .second_anchor
                        .as_ref()
                        .filter(|pick| pick.wall_index == self.selected_wall)
                        .map(|pick| &pick.anchor);
                    let active_opening_drag = self
                        .opening_drag
                        .as_ref()
                        .filter(|drag| drag.wall_index == self.selected_wall);
                    let wall_index = self.selected_wall;
                    let elevation_response = draw_wall_design_elevation(
                        ui,
                        wall,
                        DesignElevationView {
                            selected_opening,
                            selected_dimension,
                            dimension_tool_active: self.dimension_tool.active,
                            dimension_tool_axis: self.dimension_tool.axis,
                            first_dimension_anchor: first_anchor,
                            second_dimension_anchor: second_anchor,
                            active_opening_drag,
                        },
                        camera,
                    );
                    if let Some(event) = elevation_response.opening_drag {
                        self.handle_opening_drag_event(wall_index, event);
                    }
                    elevation_response.click.map(|click| match click {
                        DesignElevationClick::Opening(opening_id) => ViewClick::Opening {
                            wall_index,
                            opening_id,
                        },
                        DesignElevationClick::Dimension(dimension_id) => ViewClick::Dimension {
                            wall_index,
                            dimension_id,
                        },
                        DesignElevationClick::DimensionAnchor(anchor) => {
                            ViewClick::DimensionAnchor { wall_index, anchor }
                        }
                        DesignElevationClick::DimensionPlacement { axis, line_offset } => {
                            ViewClick::DimensionPlacement {
                                wall_index,
                                axis,
                                line_offset,
                            }
                        }
                    })
                } else {
                    let Some(plan) = &self.project_plan else {
                        ui.label("No valid framing plan");
                        return;
                    };
                    let Some(wall_plan) = plan.wall_plan(&wall.id) else {
                        ui.label("No generated framing for selected wall");
                        return;
                    };
                    let selected_member = match &self.selected {
                        Selection::Member { wall_id, member_id } if wall_id == &wall.id.0 => {
                            Some(member_id.as_str())
                        }
                        _ => None,
                    };
                    let section_x = if self.show_section {
                        section_position(wall, &self.selected)
                    } else {
                        None
                    };
                    draw_wall_elevation(
                        ui,
                        wall,
                        &wall_plan.members,
                        selected_member,
                        section_x,
                        camera,
                    )
                    .map(|member_id| ViewClick::Member {
                        wall_id: wall.id.0.clone(),
                        member_id,
                    })
                }
            }
            ViewportMode::Axonometric => {
                let Some(plan) = &self.project_plan else {
                    ui.label("No valid framing plan");
                    return;
                };
                draw_project_axonometric(
                    ui,
                    AxonometricView {
                        model: &self.model,
                        plan,
                        selected_wall: self.selected_wall,
                        selection: &self.selected,
                        workspace_mode: self.workspace_mode,
                        gpu_target_format: self.gpu_target_format,
                    },
                    &mut self.view_3d,
                )
            }
            ViewportMode::Render => {
                self.draw_project_render(ui);
                None
            }
        };
        self.draw_wall_tool.previous_snap = snap_out;
        if let Some(event) = wall_drag_out {
            self.handle_wall_drag_event(event);
        }

        if let Some(click) = click {
            self.handle_view_click(click);
        }

        if !matches!(
            self.viewport_mode,
            ViewportMode::Axonometric | ViewportMode::Render
        ) {
            self.canvas_view_controls(ui, canvas);
        }
        if let Some(anchor) = toolbar_anchor {
            self.canvas_floating_toolbar(ui, anchor);
        }
    }

    fn canvas_view_controls(&mut self, ui: &mut Ui, canvas: Rect) {
        let t = design::active();

        egui::Area::new(egui::Id::new("canvas-nav-cube"))
            .fixed_pos(Pos2::new(canvas.right() - 64.0, canvas.bottom() - 118.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::click());
                draw_nav_cube(ui.painter(), rect, t);
                let response = response.on_hover_text("View from the top — click for 3D");
                if response.clicked() {
                    self.viewport_mode = ViewportMode::Axonometric;
                }
            });

        egui::Area::new(egui::Id::new("canvas-view-mode"))
            .fixed_pos(Pos2::new(canvas.right() - 78.0, canvas.bottom() - 46.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                Frame::new()
                    .fill(t.overlay)
                    .stroke(t.border_stroke())
                    .corner_radius(design::radius::MD)
                    .inner_margin(Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        let is_3d = self.viewport_mode == ViewportMode::Axonometric;
                        egui::ComboBox::from_id_salt("view-2d-3d")
                            .selected_text(if is_3d { "3D" } else { "2D" })
                            .width(44.0)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(!is_3d, "2D").clicked() {
                                    self.viewport_mode = ViewportMode::Plan;
                                }
                                if ui.selectable_label(is_3d, "3D").clicked() {
                                    self.viewport_mode = ViewportMode::Axonometric;
                                }
                            });
                    });
            });
    }

    /// Draws the path-traced Render view. Geometry, materials, and lighting come
    /// from `framer-render`; the heavy work runs on a background thread
    /// ([`super::render_job`]) and refines progressively while the camera is still.
    fn draw_project_render(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let desired = viewport_size(ui);
        let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
        let painter = ui.painter_at(rect);

        draw_view_background(&painter, rect, theme::sheet());
        let drawing = viewport_drawing_rect(rect, 42.0);
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

        let opts = framer_render::RenderOptions {
            yaw: self.view_3d.yaw,
            pitch: self.view_3d.pitch,
            zoom: self.view_3d.zoom,
            pan: self.view_3d.pan,
            dolly: self.view_3d.dolly,
            aspect: width as f32 / height as f32,
            ..framer_render::RenderOptions::default()
        };

        // Prefer the real-time GPU compute path tracer; fall back to the
        // background-thread CPU renderer when compute isn't available.
        let (samples, target, accumulating) =
            if let (true, Some(format)) = (self.gpu_compute_ok, self.gpu_target_format) {
                let prepared = super::render::paint(
                    &mut self.render_gpu,
                    &painter,
                    drawing,
                    &self.model,
                    &opts,
                    width,
                    height,
                    moving,
                    format,
                );
                if !prepared {
                    draw_view_empty(&painter, drawing, "Preparing render…");
                }
                (
                    self.render_gpu.samples(),
                    self.render_gpu.target_spp(),
                    self.render_gpu.is_accumulating(),
                )
            } else {
                // Reuse the GPU path's accumulation key so the CPU fallback resets
                // on exactly the same camera/geometry/size changes (incl. pan/dolly).
                let key = super::render::accumulation_key(
                    super::render_job::model_signature(&self.model),
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
                )
            };

        // Progress / quality readout.
        let label = if accumulating {
            format!("Rendering — {samples}/{target} spp")
        } else {
            format!("Render complete — {samples} spp")
        };
        painter.text(
            drawing.left_bottom() + Vec2::new(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            label,
            FontId::proportional(11.0),
            theme::text_muted(),
        );
        draw_view_title(&painter, drawing, "Render");

        // Keep refining until converged, while interacting, or while the motion
        // cooldown is still ticking down (so it can settle back to full resolution).
        if accumulating || response.dragged() || moving {
            ctx.request_repaint();
        }
    }

    fn canvas_floating_toolbar(&mut self, ui: &mut Ui, anchor: Pos2) {
        let t = design::active();
        egui::Area::new(egui::Id::new("canvas-floating-toolbar"))
            .fixed_pos(Pos2::new(anchor.x - 40.0, anchor.y - 44.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                Frame::new()
                    .fill(t.overlay)
                    .stroke(t.border_stroke())
                    .corner_radius(design::radius::MD)
                    .inner_margin(Margin::symmetric(4, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if design::widgets::icon_button(
                                ui,
                                design::Icon::Duplicate,
                                "Duplicate opening",
                            )
                            .clicked()
                            {
                                self.duplicate_selected_opening();
                            }
                            if design::widgets::icon_button(
                                ui,
                                design::Icon::Delete,
                                "Delete opening",
                            )
                            .clicked()
                            {
                                self.delete_selected_opening();
                            }
                        });
                    });
            });
    }
}

fn draw_nav_cube(painter: &egui::Painter, rect: Rect, theme: design::Theme) {
    painter.rect(
        rect,
        design::radius::MD,
        theme.overlay,
        theme.border_stroke(),
        StrokeKind::Inside,
    );
    let face = rect.shrink(11.0);
    painter.rect(
        face,
        2,
        theme.control,
        Stroke::new(1.0, theme.border),
        StrokeKind::Inside,
    );
    painter.text(
        face.center(),
        Align2::CENTER_CENTER,
        "TOP",
        FontId::proportional(9.0),
        theme.text_secondary,
    );
    for (label, align, pos) in [
        (
            "N",
            Align2::CENTER_TOP,
            rect.center_top() + Vec2::new(0.0, 1.0),
        ),
        (
            "S",
            Align2::CENTER_BOTTOM,
            rect.center_bottom() + Vec2::new(0.0, -1.0),
        ),
        (
            "W",
            Align2::LEFT_CENTER,
            rect.left_center() + Vec2::new(1.0, 0.0),
        ),
        (
            "E",
            Align2::RIGHT_CENTER,
            rect.right_center() + Vec2::new(-1.0, 0.0),
        ),
    ] {
        painter.text(
            pos,
            align,
            label,
            FontId::proportional(7.5),
            theme.text_muted,
        );
    }
}

fn workspace_header(
    ui: &mut Ui,
    workspace_mode: WorkspaceMode,
    viewport_mode: ViewportMode,
    code_name: &str,
) {
    let t = design::active();
    Frame::new()
        .fill(t.panel)
        .inner_margin(Margin::symmetric(6, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = design::space::LG;
                ui.label(
                    RichText::new(workspace_mode_title(workspace_mode))
                        .strong()
                        .size(design::text_size::HEADING)
                        .color(t.text),
                );
                design::widgets::tab(ui, viewport_mode_title(workspace_mode, viewport_mode), true);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(code_name)
                            .size(design::text_size::LABEL)
                            .color(t.text_muted),
                    );
                });
            });
        });
}

fn workspace_mode_title(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::Design => "Design Workspace",
        WorkspaceMode::Plan => "Plan Workspace",
    }
}

fn viewport_mode_title(workspace_mode: WorkspaceMode, viewport_mode: ViewportMode) -> &'static str {
    match (workspace_mode, viewport_mode) {
        (WorkspaceMode::Design, ViewportMode::Plan) => "Shell",
        (WorkspaceMode::Design, ViewportMode::Elevation) => "Wall",
        (_, ViewportMode::Plan) => "Plan",
        (_, ViewportMode::Elevation) => "Elevation",
        (_, ViewportMode::Axonometric) => "3D",
        (_, ViewportMode::Render) => "Render",
    }
}

impl FramerApp {
    fn handle_opening_drag_event(&mut self, wall_index: usize, event: OpeningDragEvent) {
        match event {
            OpeningDragEvent::Started { opening_id, handle } => {
                self.begin_opening_drag(wall_index, opening_id, handle);
            }
            OpeningDragEvent::Updated { delta_x, delta_y } => {
                self.update_opening_drag(delta_x, delta_y);
            }
            OpeningDragEvent::Stopped => {
                self.finish_opening_drag();
            }
        }
    }
}

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

#[allow(clippy::too_many_arguments)]
fn draw_project_plan(
    ui: &mut Ui,
    model: &BuildingModel,
    selected_wall: usize,
    selection: &Selection,
    show_grid: bool,
    cursor_out: &mut Option<Point2>,
    toolbar_out: &mut Option<Pos2>,
    camera: &mut View2dState,
    draw_tool: &DrawWallPlanInput,
    room_tool_active: bool,
    snap_out: &mut Option<SnapResult>,
    active_wall_drag: Option<(usize, WallEditHandle)>,
    wall_drag_out: &mut Option<WallDragEvent>,
) -> Option<ViewClick> {
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

struct AxonometricView<'a> {
    model: &'a BuildingModel,
    plan: &'a ProjectFramePlan,
    selected_wall: usize,
    selection: &'a Selection,
    workspace_mode: WorkspaceMode,
    gpu_target_format: Option<wgpu::TextureFormat>,
}

fn draw_project_axonometric(
    ui: &mut Ui,
    axonometric: AxonometricView<'_>,
    view: &mut View3dState,
) -> Option<ViewClick> {
    let AxonometricView {
        model,
        plan,
        selected_wall,
        selection,
        workspace_mode,
        gpu_target_format,
    } = axonometric;

    let desired = viewport_size(ui);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    draw_view_background(&painter, rect, theme::sheet());
    let drawing = viewport_drawing_rect(rect, 42.0);
    draw_view_border(&painter, drawing);
    let cube_rect = view_cube_rect(drawing);
    let pointer = response.interact_pointer_pos();
    let cube_hover_pointer = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|position| cube_rect.contains(*position));
    let press_origin = ui.input(|input| input.pointer.press_origin());
    let shift = ui.input(|input| input.modifiers.shift);
    let dragging_primary = response.dragged_by(egui::PointerButton::Primary);
    let dragging_middle = response.dragged_by(egui::PointerButton::Middle);
    // Middle-drag or Shift+left-drag pans; plain left-drag orbits. The shared pan
    // state keeps the Render view and this view on the same vantage.
    let panning = dragging_middle || (dragging_primary && shift);
    let orbiting = dragging_primary && !shift;
    let dragging_from_cube = orbiting && pointer_started_in_rect(press_origin, cube_rect);

    if panning {
        view.pan(response.drag_delta(), drawing.width().min(drawing.height()));
    } else if orbiting {
        view.orbit(response.drag_delta());
    }

    if response.hovered() {
        let zoom_factor = ui.input(|input| {
            let wheel_factor = (input.smooth_scroll_delta.y * 0.002).exp();
            wheel_factor * input.zoom_delta()
        });
        if (zoom_factor - 1.0).abs() > f32::EPSILON {
            view.zoom_by(zoom_factor);
        }
    }

    let Some(scene) = Scene3d::from_project(model, plan, selected_wall, selection, workspace_mode)
    else {
        draw_view_empty(&painter, rect, "No 3D geometry");
        return None;
    };
    let Some(projector) = OrbitProjector::from_points(&scene.points, drawing, *view) else {
        draw_view_empty(&painter, rect, "No wall segments");
        return None;
    };

    let clicked = pointer
        .filter(|position| !cube_rect.contains(*position))
        .and_then(|position| scene.pick(position, &projector))
        .filter(|_| response.clicked());

    if let Some(target_format) = gpu_target_format {
        let callback = egui_wgpu::Callback::new_paint_callback(
            drawing,
            Framer3dCallback {
                frame_key: Framer3dFrameKey::MODEL,
                vertices: scene.vertices,
                indices: scene.indices,
                opaque_index_count: scene.opaque_index_count,
                transparent_index_count: scene.transparent_index_count,
                uniforms: GpuUniforms::from_projector(&projector, drawing),
                target_format,
            },
        );
        painter.add(callback);
    } else {
        draw_view_empty(&painter, drawing, "WGPU renderer unavailable");
    }

    let cube_action = draw_view_cube(
        &painter,
        cube_rect,
        if dragging_from_cube {
            pointer.or(cube_hover_pointer)
        } else {
            cube_hover_pointer
        },
        response.clicked() && !dragging_from_cube,
        *view,
        gpu_target_format,
    );
    if let Some(action) = cube_action {
        view.snap_to(action);
        return None;
    }

    draw_view_title(&painter, drawing, "3D workspace");

    clicked
}

struct Scene3d {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    opaque_index_count: u32,
    transparent_index_count: u32,
    points: Vec<Point3>,
    picks: Vec<PickSolid>,
}

#[derive(Default)]
struct SceneBuilder {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    points: Vec<Point3>,
    picks: Vec<PickSolid>,
    opaque_index_count: u32,
}

struct WallSegmentSpan {
    x0: Length,
    x1: Length,
    z0: Length,
    z1: Length,
}

impl WallSegmentSpan {
    fn new(x0: Length, x1: Length, z0: Length, z1: Length) -> Self {
        Self { x0, x1, z0, z1 }
    }
}

impl Scene3d {
    fn from_project(
        model: &BuildingModel,
        plan: &ProjectFramePlan,
        selected_wall: usize,
        selection: &Selection,
        workspace_mode: WorkspaceMode,
    ) -> Option<Self> {
        if model.walls.is_empty() {
            return None;
        }

        let wall_depth = model.code.stud_profile.nominal_depth().inches() as f32;
        let mut builder = SceneBuilder::default();

        if workspace_mode.shows_generated_plan() {
            for (wall_index, wall) in model.walls.iter().enumerate() {
                if let Some(wall_plan) = plan.wall_plan(&wall.id) {
                    let wall_selected = selected_wall == wall_index;
                    for member in &wall_plan.members {
                        let member_selected = matches!(
                            selection,
                            Selection::Member { wall_id, member_id }
                                if wall_id == &wall.id.0 && member_id == &member.id
                        );
                        builder.push_member(
                            wall,
                            member,
                            wall_depth,
                            wall_selected,
                            member_selected,
                        );
                    }
                }
            }
        }

        builder.finish_opaque();

        for (wall_index, wall) in model.walls.iter().enumerate() {
            let wall_selected = selected_wall == wall_index && matches!(selection, Selection::Wall);
            builder.push_wall_envelope(wall, wall_index, wall_depth, wall_selected);
            for opening in &wall.openings {
                builder.push_opening_pick(wall, wall_index, opening.id.0.clone(), wall_depth);
            }
        }

        Some(builder.finish())
    }

    fn pick(&self, pointer: Pos2, projector: &OrbitProjector) -> Option<ViewClick> {
        let mut best = None::<(u8, f32, ViewClick)>;
        for solid in &self.picks {
            let Some(depth) = solid.hit_depth(pointer, projector) else {
                continue;
            };
            let replace = best.as_ref().is_none_or(|(priority, best_depth, _)| {
                solid.priority > *priority || (solid.priority == *priority && depth > *best_depth)
            });
            if replace {
                best = Some((solid.priority, depth, solid.click.clone()));
            }
        }
        best.map(|(_, _, click)| click)
    }
}

impl SceneBuilder {
    fn push_member(
        &mut self,
        wall: &Wall,
        member: &FrameMember,
        wall_depth: f32,
        wall_selected: bool,
        selected: bool,
    ) {
        let half_member = member.cross_section_depth.inches() as f32 / 2.0;
        let (x0, x1, z0, z1) = match member.orientation {
            MemberOrientation::Horizontal => (
                member.x.inches() as f32,
                (member.x + member.cut_length).inches() as f32,
                member.elevation.inches() as f32,
                (member.elevation + member.cross_section_depth).inches() as f32,
            ),
            MemberOrientation::Vertical => (
                member.x.inches() as f32 - half_member,
                member.x.inches() as f32 + half_member,
                member.elevation.inches() as f32,
                (member.elevation + member.cut_length).inches() as f32,
            ),
        };
        let color = if selected {
            Color32::from_rgb(49, 116, 178)
        } else if wall_selected {
            brighten(member_color(member.kind), 20)
        } else {
            member_color(member.kind)
        };
        let solid = WallCuboid::new(wall, x0, x1, -wall_depth / 2.0, wall_depth / 2.0, z0, z1);
        self.push_cuboid(&solid, color_to_rgba(color));
        self.picks.push(PickSolid {
            click: ViewClick::Member {
                wall_id: wall.id.0.clone(),
                member_id: member.id.clone(),
            },
            priority: 3,
            corners: solid.corners,
        });
    }

    fn push_wall_envelope(
        &mut self,
        wall: &Wall,
        wall_index: usize,
        wall_depth: f32,
        selected: bool,
    ) {
        let color = if selected {
            Color32::from_rgba_unmultiplied(92, 145, 190, 82)
        } else {
            Color32::from_rgba_unmultiplied(188, 179, 158, 54)
        };
        let mut openings = wall.openings.iter().collect::<Vec<_>>();
        openings.sort_by_key(|opening| opening.left());
        let mut cursor = Length::ZERO;

        for opening in openings {
            self.push_wall_segment(
                wall,
                WallSegmentSpan::new(cursor, opening.left(), Length::ZERO, wall.height),
                wall_depth,
                color,
            );
            if opening.sill_height > Length::ZERO {
                self.push_wall_segment(
                    wall,
                    WallSegmentSpan::new(
                        opening.left(),
                        opening.right(),
                        Length::ZERO,
                        opening.sill_height,
                    ),
                    wall_depth,
                    color,
                );
            }
            if opening.top() < wall.height {
                self.push_wall_segment(
                    wall,
                    WallSegmentSpan::new(
                        opening.left(),
                        opening.right(),
                        opening.top(),
                        wall.height,
                    ),
                    wall_depth,
                    color,
                );
            }
            cursor = opening.right();
        }
        self.push_wall_segment(
            wall,
            WallSegmentSpan::new(cursor, wall.length, Length::ZERO, wall.height),
            wall_depth,
            color,
        );

        let solid = WallCuboid::new(
            wall,
            0.0,
            wall.length.inches() as f32,
            -wall_depth / 2.0,
            wall_depth / 2.0,
            0.0,
            wall.height.inches() as f32,
        );
        self.picks.push(PickSolid {
            click: ViewClick::Wall(wall_index),
            priority: 1,
            corners: solid.corners,
        });
    }

    fn push_wall_segment(
        &mut self,
        wall: &Wall,
        span: WallSegmentSpan,
        wall_depth: f32,
        color: Color32,
    ) {
        if span.x1 <= span.x0 || span.z1 <= span.z0 {
            return;
        }
        let solid = WallCuboid::new(
            wall,
            span.x0.inches() as f32,
            span.x1.inches() as f32,
            -wall_depth / 2.0,
            wall_depth / 2.0,
            span.z0.inches() as f32,
            span.z1.inches() as f32,
        );
        self.push_cuboid(&solid, color_to_rgba(color));
    }

    fn push_opening_pick(
        &mut self,
        wall: &Wall,
        wall_index: usize,
        opening_id: String,
        wall_depth: f32,
    ) {
        let Some(opening) = wall
            .openings
            .iter()
            .find(|candidate| candidate.id.0 == opening_id)
        else {
            return;
        };
        let solid = WallCuboid::new(
            wall,
            opening.left().inches() as f32,
            opening.right().inches() as f32,
            -wall_depth / 2.0,
            wall_depth / 2.0,
            opening.sill_height.inches() as f32,
            opening.top().inches() as f32,
        );
        self.picks.push(PickSolid {
            click: ViewClick::Opening {
                wall_index,
                opening_id,
            },
            priority: 2,
            corners: solid.corners,
        });
    }

    fn push_cuboid(&mut self, cuboid: &WallCuboid, color: [f32; 4]) {
        if cuboid.is_degenerate() {
            return;
        }

        self.points.extend(cuboid.corners);
        for face in cuboid.faces() {
            let base = self.vertices.len() as u32;
            for corner in face.corners {
                let point = cuboid.corners[corner];
                self.vertices.push(GpuVertex {
                    position: [point.x, point.y, point.z],
                    color,
                    normal: [face.normal.x, face.normal.y, face.normal.z],
                });
            }
            self.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    fn finish_opaque(&mut self) {
        self.opaque_index_count = self.indices.len() as u32;
    }

    fn finish(self) -> Scene3d {
        let total_index_count = self.indices.len() as u32;
        Scene3d {
            vertices: self.vertices,
            indices: self.indices,
            opaque_index_count: self.opaque_index_count,
            transparent_index_count: total_index_count - self.opaque_index_count,
            points: self.points,
            picks: self.picks,
        }
    }
}

#[derive(Clone, Copy)]
struct WallCuboid {
    corners: [Point3; 8],
    along: Point3,
    side: Point3,
}

impl WallCuboid {
    #[allow(clippy::too_many_arguments)]
    fn new(wall: &Wall, x0: f32, x1: f32, side0: f32, side1: f32, z0: f32, z1: f32) -> Self {
        let basis = WallBasis::new(wall);
        let corners = [
            basis.point(x0, side0, z0),
            basis.point(x1, side0, z0),
            basis.point(x1, side1, z0),
            basis.point(x0, side1, z0),
            basis.point(x0, side0, z1),
            basis.point(x1, side0, z1),
            basis.point(x1, side1, z1),
            basis.point(x0, side1, z1),
        ];
        Self {
            corners,
            along: Point3::vector(basis.along_x, basis.along_y, 0.0),
            side: Point3::vector(basis.side_x, basis.side_y, 0.0),
        }
    }

    fn is_degenerate(&self) -> bool {
        self.corners[0].distance_squared(self.corners[1]) < f32::EPSILON
            || self.corners[1].distance_squared(self.corners[2]) < f32::EPSILON
            || self.corners[0].distance_squared(self.corners[4]) < f32::EPSILON
    }

    fn faces(&self) -> [CuboidFace; 6] {
        [
            CuboidFace::new([0, 3, 2, 1], -Point3::Z),
            CuboidFace::new([4, 5, 6, 7], Point3::Z),
            CuboidFace::new([0, 1, 5, 4], -self.side),
            CuboidFace::new([1, 2, 6, 5], self.along),
            CuboidFace::new([2, 3, 7, 6], self.side),
            CuboidFace::new([3, 0, 4, 7], -self.along),
        ]
    }
}

#[derive(Clone, Copy)]
struct CuboidFace {
    corners: [usize; 4],
    normal: Point3,
}

impl CuboidFace {
    fn new(corners: [usize; 4], normal: Point3) -> Self {
        Self { corners, normal }
    }
}

struct PickSolid {
    click: ViewClick,
    priority: u8,
    corners: [Point3; 8],
}

impl PickSolid {
    fn hit_depth(&self, pointer: Pos2, projector: &OrbitProjector) -> Option<f32> {
        let mut best_depth = None::<f32>;
        for face in CUBOID_FACE_INDICES {
            let projected = face.map(|index| projector.project_point(self.corners[index]));
            let positions = projected.map(|point| point.pos);
            if point_hits_projected_quad(pointer, &positions) {
                let depth = projected.iter().map(|point| point.depth).sum::<f32>() / 4.0;
                best_depth = Some(best_depth.map_or(depth, |existing| existing.max(depth)));
            }
        }
        best_depth
    }
}

const CUBOID_FACE_INDICES: [[usize; 4]; 6] = [
    [0, 3, 2, 1],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [3, 0, 4, 7],
];

struct WallBasis {
    origin_x: f32,
    origin_y: f32,
    along_x: f32,
    along_y: f32,
    side_x: f32,
    side_y: f32,
}

impl WallBasis {
    fn new(wall: &Wall) -> Self {
        let dx = (wall.end.x - wall.start.x).inches() as f32;
        let dy = (wall.end.y - wall.start.y).inches() as f32;
        let length = (dx * dx + dy * dy).sqrt().max(1.0);
        let along_x = dx / length;
        let along_y = dy / length;
        Self {
            origin_x: wall.start.x.inches() as f32,
            origin_y: wall.start.y.inches() as f32,
            along_x,
            along_y,
            side_x: -along_y,
            side_y: along_x,
        }
    }

    fn point(&self, local_x: f32, side: f32, z: f32) -> Point3 {
        Point3 {
            x: self.origin_x + self.along_x * local_x + self.side_x * side,
            y: self.origin_y + self.along_y * local_x + self.side_y * side,
            z,
        }
    }
}

fn color_to_rgba(color: Color32) -> [f32; 4] {
    [
        color.r() as f32 / 255.0,
        color.g() as f32 / 255.0,
        color.b() as f32 / 255.0,
        color.a() as f32 / 255.0,
    ]
}

fn brighten(color: Color32, amount: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r().saturating_add(amount),
        color.g().saturating_add(amount),
        color.b().saturating_add(amount),
        color.a(),
    )
}

fn view_cube_rect(drawing: Rect) -> Rect {
    Rect::from_min_size(
        drawing.right_top() + Vec2::new(-118.0, 12.0),
        Vec2::splat(104.0),
    )
}

fn view_cube_body_rect(rect: Rect) -> Rect {
    Rect::from_min_max(
        rect.left_top() + Vec2::new(18.0, 8.0),
        rect.right_bottom() - Vec2::new(6.0, 6.0),
    )
}

struct ViewCubeGeometry {
    home_rect: Rect,
    faces: Vec<ViewCubeFaceGeometry>,
    edges: Vec<ViewCubeEdgeGeometry>,
    corners: Vec<ViewCubeCornerGeometry>,
}

#[derive(Clone, Copy)]
struct ViewCubeFaceGeometry {
    action: ViewCubeAction,
    points: [Pos2; 4],
}

#[derive(Clone, Copy)]
struct ViewCubeEdgeGeometry {
    action: ViewCubeAction,
    points: [Pos2; 2],
}

#[derive(Clone, Copy)]
struct ViewCubeCornerGeometry {
    action: ViewCubeAction,
    center: Pos2,
}

impl ViewCubeGeometry {
    fn from_rect(rect: Rect, view: View3dState) -> Self {
        let corners = view_cube_points();
        let body_rect = view_cube_body_rect(rect);
        let projector = view_cube_projector(body_rect, view);
        let camera_direction = projector.view_direction();
        let projected = corners.map(|point| projector.project_point(point).pos);
        let face_specs = view_cube_face_specs();
        let faces = face_specs
            .iter()
            .filter_map(|spec| {
                view_cube_face_geometry(&projector, &corners, *spec, camera_direction)
            })
            .collect::<Vec<_>>();
        let mut visible_corners = [false; 8];
        for face in &faces {
            if let Some(spec) = face_specs.iter().find(|spec| spec.action == face.action) {
                for corner in spec.face {
                    visible_corners[corner] = true;
                }
            }
        }

        Self {
            home_rect: Rect::from_min_size(
                rect.left_top() + Vec2::new(6.0, 6.0),
                Vec2::splat(22.0),
            ),
            edges: view_cube_edges()
                .into_iter()
                .filter(|[start, end]| {
                    visible_corners[*start]
                        && visible_corners[*end]
                        && faces.iter().any(|face| {
                            face_specs
                                .iter()
                                .find(|spec| spec.action == face.action)
                                .is_some_and(|spec| {
                                    view_cube_face_has_edge(spec.face, *start, *end)
                                })
                        })
                })
                .map(|[start, end]| ViewCubeEdgeGeometry {
                    action: ViewCubeAction::snap(ViewCubeOrientation::from_points(
                        corners[start],
                        corners[end],
                    )),
                    points: [projected[start], projected[end]],
                })
                .collect(),
            corners: visible_corners
                .iter()
                .enumerate()
                .filter(|(_, visible)| **visible)
                .map(|(index, _)| ViewCubeCornerGeometry {
                    action: ViewCubeAction::snap(ViewCubeOrientation::from_point(corners[index])),
                    center: projected[index],
                })
                .collect(),
            faces,
        }
    }

    fn hit(&self, position: Pos2) -> Option<ViewCubeAction> {
        if self.home_rect.contains(position) {
            Some(ViewCubeAction::Home)
        } else if let Some(corner) = self
            .corners
            .iter()
            .filter(|corner| corner.center.distance(position) <= 8.0)
            .min_by(|left, right| {
                left.center
                    .distance(position)
                    .total_cmp(&right.center.distance(position))
            })
        {
            Some(corner.action)
        } else if let Some(edge) = self
            .edges
            .iter()
            .filter(|edge| distance_to_segment(position, edge.points[0], edge.points[1]) <= 7.0)
            .min_by(|left, right| {
                distance_to_segment(position, left.points[0], left.points[1]).total_cmp(
                    &distance_to_segment(position, right.points[0], right.points[1]),
                )
            })
        {
            Some(edge.action)
        } else {
            self.faces
                .iter()
                .find(|face| point_in_polygon(position, &face.points))
                .map(|face| face.action)
        }
    }
}

fn view_cube_projector(rect: Rect, view: View3dState) -> OrbitProjector {
    let mut cube_view = view;
    cube_view.zoom = 1.0;
    OrbitProjector::from_points(&view_cube_points(), rect, cube_view)
        .expect("view cube has fixed geometry")
}

fn view_cube_points() -> [Point3; 8] {
    [
        Point3::vector(-1.0, -1.0, -1.0),
        Point3::vector(1.0, -1.0, -1.0),
        Point3::vector(1.0, 1.0, -1.0),
        Point3::vector(-1.0, 1.0, -1.0),
        Point3::vector(-1.0, -1.0, 1.0),
        Point3::vector(1.0, -1.0, 1.0),
        Point3::vector(1.0, 1.0, 1.0),
        Point3::vector(-1.0, 1.0, 1.0),
    ]
}

#[derive(Clone, Copy)]
struct ViewCubeFaceSpec {
    action: ViewCubeAction,
    face: [usize; 4],
    normal: Point3,
    color: Color32,
}

fn view_cube_face_specs() -> [ViewCubeFaceSpec; 6] {
    [
        ViewCubeFaceSpec {
            action: ViewCubeAction::BOTTOM,
            face: [0, 3, 2, 1],
            normal: -Point3::Z,
            color: Color32::from_rgb(192, 197, 193),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::TOP,
            face: [4, 5, 6, 7],
            normal: Point3::Z,
            color: Color32::from_rgb(228, 235, 232),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::BACK,
            face: [0, 1, 5, 4],
            normal: -Point3::Y,
            color: Color32::from_rgb(196, 201, 196),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::RIGHT,
            face: [1, 2, 6, 5],
            normal: Point3::X,
            color: Color32::from_rgb(230, 232, 229),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::FRONT,
            face: [2, 3, 7, 6],
            normal: Point3::Y,
            color: Color32::from_rgb(238, 238, 234),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::LEFT,
            face: [3, 0, 4, 7],
            normal: -Point3::X,
            color: Color32::from_rgb(186, 191, 188),
        },
    ]
}

fn view_cube_edges() -> [[usize; 2]; 12] {
    [
        [0, 1],
        [1, 2],
        [2, 3],
        [3, 0],
        [4, 5],
        [5, 6],
        [6, 7],
        [7, 4],
        [0, 4],
        [1, 5],
        [2, 6],
        [3, 7],
    ]
}

fn view_cube_face_has_edge(face: [usize; 4], start: usize, end: usize) -> bool {
    face.iter().enumerate().any(|(index, corner)| {
        let next = face[(index + 1) % face.len()];
        (*corner == start && next == end) || (*corner == end && next == start)
    })
}

fn view_cube_face_geometry(
    projector: &OrbitProjector,
    corners: &[Point3; 8],
    spec: ViewCubeFaceSpec,
    camera_direction: Point3,
) -> Option<ViewCubeFaceGeometry> {
    if spec.normal.dot(camera_direction) <= 0.0 {
        return None;
    }

    let points = spec
        .face
        .map(|index| projector.project_point(corners[index]).pos);
    Some(ViewCubeFaceGeometry {
        action: spec.action,
        points,
    })
}

fn draw_view_cube(
    painter: &egui::Painter,
    rect: Rect,
    pointer: Option<Pos2>,
    clicked: bool,
    view: View3dState,
    gpu_target_format: Option<wgpu::TextureFormat>,
) -> Option<ViewCubeAction> {
    let geometry = ViewCubeGeometry::from_rect(rect, view);
    let hovered_action = pointer.and_then(|position| geometry.hit(position));
    let hovered_home = hovered_action == Some(ViewCubeAction::Home);

    painter.rect_filled(
        rect,
        4.0,
        Color32::from_rgba_unmultiplied(250, 250, 248, 215),
    );
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, Color32::from_rgb(174, 176, 170)),
        StrokeKind::Outside,
    );
    painter.rect_filled(
        geometry.home_rect,
        3.0,
        if hovered_home {
            Color32::from_rgb(214, 225, 232)
        } else {
            Color32::from_rgb(232, 234, 229)
        },
    );
    painter.rect_stroke(
        geometry.home_rect,
        3.0,
        Stroke::new(1.0, Color32::from_rgb(129, 132, 127)),
        StrokeKind::Outside,
    );
    painter.text(
        geometry.home_rect.center(),
        Align2::CENTER_CENTER,
        "H",
        FontId::proportional(11.0),
        Color32::from_rgb(61, 67, 71),
    );

    let body_rect = view_cube_body_rect(rect);
    let projector = view_cube_projector(body_rect, view);
    if let Some(target_format) = gpu_target_format {
        let (vertices, indices) = view_cube_mesh(hovered_action);
        painter.add(egui_wgpu::Callback::new_paint_callback(
            body_rect,
            Framer3dCallback {
                frame_key: Framer3dFrameKey::VIEW_CUBE,
                opaque_index_count: indices.len() as u32,
                transparent_index_count: 0,
                uniforms: GpuUniforms::from_projector_with_depth_base(&projector, body_rect, 0.14),
                vertices,
                indices,
                target_format,
            },
        ));
        draw_view_cube_edges(painter, &geometry, hovered_action);
        draw_view_cube_labels(painter, &projector, &geometry);
    } else {
        draw_view_empty(painter, body_rect, "3D");
    }

    if clicked { hovered_action } else { None }
}

fn view_cube_mesh(hovered_action: Option<ViewCubeAction>) -> (Vec<GpuVertex>, Vec<u32>) {
    let corners = view_cube_points();
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    let hovered_orientation = hovered_action.and_then(ViewCubeAction::orientation);
    for spec in view_cube_face_specs() {
        let face_orientation = spec.action.orientation().expect("cube faces snap");
        let color = if hovered_orientation
            .is_some_and(|orientation| orientation.includes_face(face_orientation))
        {
            brighten(spec.color, 24)
        } else {
            spec.color
        };
        push_view_cube_face(
            &mut vertices,
            &mut indices,
            &corners,
            spec.face,
            spec.normal,
            color,
        );
    }

    (vertices, indices)
}

fn push_view_cube_face(
    vertices: &mut Vec<GpuVertex>,
    indices: &mut Vec<u32>,
    corners: &[Point3; 8],
    face: [usize; 4],
    normal: Point3,
    color: Color32,
) {
    push_view_cube_quad(
        vertices,
        indices,
        face.map(|index| corners[index]),
        normal,
        color_to_rgba(color),
    );
}

fn push_view_cube_quad(
    vertices: &mut Vec<GpuVertex>,
    indices: &mut Vec<u32>,
    points: [Point3; 4],
    normal: Point3,
    color: [f32; 4],
) {
    let base = vertices.len() as u32;
    for point in points {
        vertices.push(GpuVertex {
            position: [point.x, point.y, point.z],
            color,
            normal: [normal.x, normal.y, normal.z],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

#[derive(Clone, Copy)]
struct ViewCubeLabelSpec {
    action: ViewCubeAction,
    text: &'static str,
    center: Point3,
    u_axis: Point3,
    v_axis: Point3,
    width: f32,
}

fn view_cube_label_specs() -> [ViewCubeLabelSpec; 3] {
    [
        ViewCubeLabelSpec {
            action: ViewCubeAction::TOP,
            text: "TOP",
            center: Point3::vector(0.0, 0.0, 1.0),
            u_axis: Point3::Y,
            v_axis: Point3::X,
            width: 0.90,
        },
        ViewCubeLabelSpec {
            action: ViewCubeAction::RIGHT,
            text: "RIGHT",
            center: Point3::vector(1.0, 0.0, 0.0),
            u_axis: -Point3::Y,
            v_axis: Point3::Z,
            width: 1.28,
        },
        ViewCubeLabelSpec {
            action: ViewCubeAction::FRONT,
            text: "FRONT",
            center: Point3::vector(0.0, 1.0, 0.0),
            u_axis: Point3::X,
            v_axis: Point3::Z,
            width: 1.28,
        },
    ]
}

fn draw_view_cube_edges(
    painter: &egui::Painter,
    geometry: &ViewCubeGeometry,
    hovered_action: Option<ViewCubeAction>,
) {
    let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(82, 89, 88, 128));
    for face in &geometry.faces {
        for index in 0..face.points.len() {
            painter.line_segment(
                [
                    face.points[index],
                    face.points[(index + 1) % face.points.len()],
                ],
                stroke,
            );
        }
    }

    let Some(orientation) = hovered_action.and_then(ViewCubeAction::orientation) else {
        return;
    };

    let highlight = Stroke::new(2.25, Color32::from_rgb(42, 124, 186));
    match orientation.component_count() {
        1 => {
            if let Some(face) = geometry
                .faces
                .iter()
                .find(|face| face.action.orientation() == Some(orientation))
            {
                for index in 0..face.points.len() {
                    painter.line_segment(
                        [
                            face.points[index],
                            face.points[(index + 1) % face.points.len()],
                        ],
                        highlight,
                    );
                }
            }
        }
        2 => {
            if let Some(edge) = geometry
                .edges
                .iter()
                .find(|edge| edge.action.orientation() == Some(orientation))
            {
                painter.line_segment(edge.points, highlight);
            }
        }
        3 => {
            if let Some(corner) = geometry
                .corners
                .iter()
                .find(|corner| corner.action.orientation() == Some(orientation))
            {
                painter.circle_filled(
                    corner.center,
                    4.0,
                    Color32::from_rgba_unmultiplied(42, 124, 186, 130),
                );
                painter.circle_stroke(corner.center, 4.0, highlight);
            }
        }
        _ => {}
    }
}

fn draw_view_cube_labels(
    painter: &egui::Painter,
    projector: &OrbitProjector,
    geometry: &ViewCubeGeometry,
) {
    for spec in view_cube_label_specs() {
        if geometry.faces.iter().any(|face| face.action == spec.action) {
            draw_view_cube_projected_label(painter, projector, spec);
        }
    }
}

fn draw_view_cube_projected_label(
    painter: &egui::Painter,
    projector: &OrbitProjector,
    spec: ViewCubeLabelSpec,
) {
    let color = Color32::from_rgba_unmultiplied(53, 60, 62, 215);
    let galley = painter.layout_no_wrap(spec.text.to_owned(), FontId::proportional(12.0), color);
    let size = galley.rect.size();
    if size.x <= f32::EPSILON || size.y <= f32::EPSILON {
        return;
    }

    let center = projector.project_point(spec.center).pos;
    let u = projector
        .project_point(spec.center.offset(spec.u_axis, 1.0))
        .pos
        - center;
    let v = projector
        .project_point(spec.center.offset(spec.v_axis, 1.0))
        .pos
        - center;
    let point_scale = spec.width / size.x;
    let glyph_center = galley.rect.center();
    let font_image_size = painter.fonts_mut(|fonts| fonts.font_image_size());
    let uv_scale = Vec2::new(
        1.0 / font_image_size[0] as f32,
        1.0 / font_image_size[1] as f32,
    );
    let mut mesh = Mesh::default();

    for row in &galley.rows {
        if row.visuals.mesh.is_empty() {
            continue;
        }
        let index_offset = mesh.vertices.len() as u32;
        mesh.indices.extend(
            row.visuals
                .mesh
                .indices
                .iter()
                .map(|index| index + index_offset),
        );
        mesh.vertices
            .extend(row.visuals.mesh.vertices.iter().map(|vertex| {
                let local = row.pos + vertex.pos.to_vec2();
                let centered = local - glyph_center;
                let pos = center + u * (centered.x * point_scale) - v * (centered.y * point_scale);
                Vertex {
                    pos,
                    uv: (vertex.uv.to_vec2() * uv_scale).to_pos2(),
                    color,
                }
            }));
    }

    if !mesh.is_empty() {
        painter.add(Shape::mesh(mesh));
    }
}

enum DesignElevationClick {
    Opening(String),
    Dimension(String),
    DimensionAnchor(DimensionAnchor),
    DimensionPlacement {
        axis: DimensionAxis,
        line_offset: Length,
    },
}

#[derive(Clone, Copy)]
struct DimensionPlacement {
    axis: DimensionAxis,
    line_offset: Length,
}

#[derive(Default)]
struct DesignElevationResponse {
    click: Option<DesignElevationClick>,
    opening_drag: Option<OpeningDragEvent>,
}

enum OpeningDragEvent {
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

/// A plan-view wall-endpoint drag, mirroring [`OpeningDragEvent`]. `Updated`
/// carries the already-snapped model point for the dragged endpoint.
#[derive(Debug, Clone, Copy)]
pub(super) enum WallDragEvent {
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

#[derive(Clone)]
struct OpeningHandleHit {
    opening_id: String,
    handle: OpeningEditHandle,
}

struct DesignElevationView<'a> {
    selected_opening: Option<&'a str>,
    selected_dimension: Option<&'a str>,
    dimension_tool_active: bool,
    dimension_tool_axis: DimensionAxis,
    first_dimension_anchor: Option<&'a DimensionAnchor>,
    second_dimension_anchor: Option<&'a DimensionAnchor>,
    active_opening_drag: Option<&'a OpeningDragState>,
}

fn draw_wall_design_elevation(
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
            opening.kind,
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

    painter.text(
        Pos2::new(wall_rect.left(), wall_rect.bottom() + 20.0),
        Align2::LEFT_CENTER,
        format!("{} x {}", wall.length, wall.height),
        FontId::proportional(13.0),
        theme::framing_line_dark(),
    );

    reset_view_on_empty_double_click(&response, camera, over_element);
    output
}

fn draw_wall_dimension_annotations(
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

fn draw_dimension_tick(painter: &egui::Painter, point: Pos2, axis: DimensionAxis, color: Color32) {
    let tick = match axis {
        DimensionAxis::Horizontal => [point + Vec2::new(-4.0, 4.0), point + Vec2::new(4.0, -4.0)],
        DimensionAxis::Vertical => [point + Vec2::new(-4.0, -4.0), point + Vec2::new(4.0, 4.0)],
    };
    painter.line_segment(tick, Stroke::new(1.0, color));
}

fn dimension_label_position(start: Pos2, end: Pos2, axis: DimensionAxis) -> Pos2 {
    match axis {
        DimensionAxis::Horizontal => Pos2::new((start.x + end.x) / 2.0, start.y - 2.0),
        DimensionAxis::Vertical => Pos2::new(start.x, (start.y + end.y) / 2.0),
    }
}

fn dimension_label_rect(start: Pos2, end: Pos2, axis: DimensionAxis, label: &str) -> Rect {
    let center = dimension_label_position(start, end, axis);
    let width = (label.chars().count() as f32 * 6.5 + 12.0).clamp(34.0, 86.0);
    Rect::from_center_size(center, Vec2::new(width, 18.0))
}

fn draw_dimension_line_with_label_gap(
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

fn dimension_display_value(wall: &Wall, dimension: &framer_core::DimensionConstraint) -> String {
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

struct PendingDimensionPreview<'a> {
    first_anchor: &'a DimensionAnchor,
    second_anchor: &'a DimensionAnchor,
    pointer: Option<Pos2>,
    fallback_axis: DimensionAxis,
}

fn draw_pending_dimension_preview(
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

fn pending_dimension_default_line_position(drawing: Rect, axis: DimensionAxis) -> Pos2 {
    match axis {
        DimensionAxis::Horizontal => Pos2::new(drawing.center().x, drawing.top() - 24.0),
        DimensionAxis::Vertical => Pos2::new(drawing.right() + 56.0, drawing.center().y),
    }
}

fn dimension_line_offset_for_position(
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

fn dimension_line_screen_position(
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

fn dimension_anchor_position(
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

fn dimension_axis_for_placement_position(
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

struct DimensionAnchorSelection<'a> {
    axis: DimensionAxis,
    first_anchor: Option<&'a DimensionAnchor>,
    second_anchor: Option<&'a DimensionAnchor>,
}

fn draw_dimension_anchors(
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

fn hit_dimension_anchor(
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
struct DimensionAnchorMarker {
    anchor: DimensionAnchor,
    position: Pos2,
    kind: DimensionAnchorKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DimensionAnchorKind {
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

fn dimension_anchor_markers(
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

fn push_wall_anchor_markers(markers: &mut Vec<DimensionAnchorMarker>, rect: Rect) {
    push_point_anchor_markers(markers, rect, |horizontal, vertical| {
        DimensionAnchor::WallPoint {
            horizontal,
            vertical,
        }
    });
}

fn push_opening_anchor_markers(
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

fn push_point_anchor_markers(
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

fn draw_wall_elevation(
    ui: &mut Ui,
    wall: &Wall,
    members: &[FrameMember],
    selected_member: Option<&str>,
    section_x: Option<Length>,
    camera: &mut View2dState,
) -> Option<String> {
    let available = ui.available_size();
    let desired = Vec2::new(available.x.max(420.0), (available.y - 16.0).max(420.0));
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    let margin = 52.0;
    let drawing = Rect::from_min_max(
        rect.min + Vec2::splat(margin),
        rect.max - Vec2::new(margin, margin),
    );
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

    reset_view_on_empty_double_click(&response, camera, over_element);
    clicked
}

fn member_rect(drawing: Rect, sx: f32, sy: f32, member: &FrameMember) -> Rect {
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

fn draw_opening_guides(painter: &egui::Painter, drawing: Rect, sx: f32, sy: f32, wall: &Wall) {
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

fn opening_rect(drawing: Rect, sx: f32, sy: f32, opening: &Opening) -> Rect {
    let x = drawing.left() + opening.left().inches() as f32 * sx;
    let y = drawing.bottom() - opening.top().inches() as f32 * sy;
    let width = (opening.width.inches() as f32 * sx).max(4.0);
    let height = (opening.height.inches() as f32 * sy).max(4.0);
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, height))
}

fn hit_opening_edit_target(
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

fn hit_opening_move_target(
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

fn hit_opening_move_handle(rect: Rect, position: Pos2) -> bool {
    rect.expand(11.0).contains(position)
}

fn hit_opening_edit_handle(rect: Rect, position: Pos2) -> Option<OpeningEditHandle> {
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

fn draw_opening_edit_handles(
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

fn opening_handle_position(rect: Rect, handle: OpeningEditHandle) -> Pos2 {
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

fn opening_drag_delta(delta: Vec2, scale: f32) -> (Length, Length) {
    let scale = scale.max(0.001) as f64;
    (
        Length::from_inches(delta.x as f64 / scale),
        Length::from_inches(-(delta.y as f64) / scale),
    )
}

fn cursor_for_opening_handle(handle: OpeningEditHandle, active: bool) -> CursorIcon {
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

fn draw_opening_guide(
    painter: &egui::Painter,
    rect: Rect,
    kind: OpeningKind,
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
        kind_label(kind),
        FontId::proportional(11.0),
        Color32::from_rgb(99, 74, 39),
    );
}

fn draw_member_rect(
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

fn draw_section_line(painter: &egui::Painter, drawing: Rect, sx: f32, x: Length) {
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

fn member_color(kind: MemberKind) -> Color32 {
    match kind {
        MemberKind::BottomPlate | MemberKind::TopPlate => Color32::from_rgb(99, 85, 67),
        MemberKind::CornerPost => Color32::from_rgb(52, 95, 127),
        MemberKind::PartitionStud => Color32::from_rgb(79, 127, 95),
        MemberKind::BackingStud => Color32::from_rgb(127, 111, 79),
        MemberKind::CommonStud => Color32::from_rgb(186, 145, 94),
        MemberKind::KingStud => Color32::from_rgb(151, 100, 61),
        MemberKind::JackStud => Color32::from_rgb(211, 168, 95),
        MemberKind::Header => Color32::from_rgb(115, 130, 99),
        MemberKind::RoughSill => Color32::from_rgb(92, 121, 144),
        MemberKind::CrippleStud => Color32::from_rgb(218, 190, 139),
    }
}

fn section_position(wall: &Wall, selection: &Selection) -> Option<Length> {
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
        | Selection::Wall => Some(wall.length / 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_3d_state_orbits_zooms_and_snaps() {
        let mut view = View3dState::default();
        let initial_yaw = view.yaw;
        let initial_pitch = view.pitch;

        view.orbit(Vec2::new(20.0, -10.0));
        assert!(view.yaw > initial_yaw);
        assert!(view.pitch > initial_pitch);

        view.zoom_by(10.0);
        assert_eq!(view.zoom, 3.0);

        view.snap_to(ViewCubeAction::TOP);
        assert_close(view.yaw, 0.0);
        assert_close(view.pitch, FRAC_PI_2);

        view.snap_to(ViewCubeAction::RIGHT);
        assert_close(view.yaw, -FRAC_PI_2);
        assert_close(view.pitch, 0.0);

        view.snap_to(ViewCubeAction::snap(ViewCubeOrientation::new(0, 1, 1)));
        assert_close(view.yaw, 0.0);
        assert_close(view.pitch, FRAC_PI_4);

        view.snap_to(ViewCubeAction::snap(ViewCubeOrientation::new(1, 1, 1)));
        assert_close(view.yaw, -FRAC_PI_4);

        view.snap_to(ViewCubeAction::Home);
        assert_close(view.yaw, -FRAC_PI_4);
        assert_close(view.zoom, 1.0);
    }

    #[test]
    fn orbit_projector_changes_projection_when_view_rotates() {
        let model = BuildingModel::demo_shell();
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let front_end = model.walls[0].end;

        let home = OrbitProjector::from_model(&model, drawing, View3dState::default())
            .unwrap()
            .project(front_end, Length::ZERO)
            .pos;
        let mut right_view = View3dState::default();
        right_view.snap_to(ViewCubeAction::RIGHT);
        let right = OrbitProjector::from_model(&model, drawing, right_view)
            .unwrap()
            .project(front_end, Length::ZERO)
            .pos;

        assert!(home.distance(right) > 8.0);
    }

    #[test]
    fn orbit_projector_keeps_distance_stable_when_view_rotates() {
        let model = BuildingModel::demo_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let scene =
            Scene3d::from_project(&model, &plan, 0, &Selection::Wall, WorkspaceMode::Plan).unwrap();
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));

        let home =
            OrbitProjector::from_points(&scene.points, drawing, View3dState::default()).unwrap();
        let mut right_view = View3dState::default();
        right_view.snap_to(ViewCubeAction::RIGHT);
        let right = OrbitProjector::from_points(&scene.points, drawing, right_view).unwrap();

        assert_close(home.scale, right.scale);
    }

    #[test]
    fn orbit_projector_applies_explicit_zoom_without_auto_fit_drift() {
        let model = BuildingModel::demo_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let scene =
            Scene3d::from_project(&model, &plan, 0, &Selection::Wall, WorkspaceMode::Plan).unwrap();
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));

        let base =
            OrbitProjector::from_points(&scene.points, drawing, View3dState::default()).unwrap();
        let mut zoomed_view = View3dState::default();
        zoomed_view.zoom_by(1.25);
        let zoomed = OrbitProjector::from_points(&scene.points, drawing, zoomed_view).unwrap();

        assert_close(zoomed.scale / base.scale, 1.25);
    }

    #[test]
    fn orbit_projector_pans_rigidly_by_pan_offset() {
        let model = BuildingModel::demo_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let scene =
            Scene3d::from_project(&model, &plan, 0, &Selection::Wall, WorkspaceMode::Plan).unwrap();
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));

        let base =
            OrbitProjector::from_points(&scene.points, drawing, View3dState::default()).unwrap();
        let mut panned_view = View3dState::default();
        panned_view.pan = Vec3::new(0.3, -0.15, 0.05);
        let panned = OrbitProjector::from_points(&scene.points, drawing, panned_view).unwrap();

        // Pan is a uniform world translation of the pivot, so in the orthographic
        // view every point shifts on screen by the SAME vector (a rigid pan), by a
        // non-trivial amount.
        let pa = scene.points[0];
        let pb = scene.points[scene.points.len() / 2];
        let shift_a = panned.project_point(pa).pos - base.project_point(pa).pos;
        let shift_b = panned.project_point(pb).pos - base.project_point(pb).pos;
        assert!(
            shift_a.length() > 1.0,
            "pan must move the projection: {shift_a:?}"
        );
        assert!(
            (shift_a - shift_b).length() < 1e-2,
            "pan must be rigid across all points: {shift_a:?} vs {shift_b:?}"
        );
    }

    #[test]
    fn pan_drag_is_zero_for_zero_delta() {
        let mut v = View3dState::default();
        v.pan(Vec2::ZERO, 600.0);
        assert_eq!(v.pan, Vec3::ZERO);
    }

    #[test]
    fn horizontal_pan_moves_along_world_right_opposite_the_drag() {
        let (right, up) = View3dState::default().screen_basis();
        let mut v = View3dState::default();
        v.pan(Vec2::new(40.0, 0.0), 600.0);
        // Grab-the-scene: dragging right slides the pivot along −right (so the
        // content under the cursor tracks it), with no vertical component.
        assert!(
            v.pan.dot(up).abs() < 1e-6,
            "horizontal drag must not pan vertically: {:?}",
            v.pan
        );
        assert!(
            v.pan.dot(right) < 0.0,
            "drag right → pivot moves −right (grab scene): {:?}",
            v.pan
        );
    }

    #[test]
    fn vertical_pan_moves_along_world_up_with_the_drag() {
        let (right, up) = View3dState::default().screen_basis();
        let mut v = View3dState::default();
        v.pan(Vec2::new(0.0, 40.0), 600.0); // egui y grows downward
        assert!(
            v.pan.dot(right).abs() < 1e-6,
            "vertical drag must not pan horizontally: {:?}",
            v.pan
        );
        assert!(
            v.pan.dot(up) > 0.0,
            "drag down → pivot moves +up (grab scene): {:?}",
            v.pan
        );
    }

    #[test]
    fn telephoto_zoom_reduces_the_pan_rate() {
        let mut wide = View3dState::default();
        wide.pan(Vec2::new(0.0, 30.0), 600.0);
        let mut tele = View3dState::default();
        tele.zoom = 2.0;
        tele.pan(Vec2::new(0.0, 30.0), 600.0);
        assert!(wide.pan.length() > 0.0);
        assert!(
            (tele.pan.length() - wide.pan.length() * 0.5).abs() < 1e-4 * wide.pan.length(),
            "2× telephoto zoom should halve the pan rate: wide={}, tele={}",
            wide.pan.length(),
            tele.pan.length()
        );
    }

    #[test]
    fn pan_is_clamped_to_a_maximum_radius() {
        let mut v = View3dState::default();
        for _ in 0..2000 {
            v.pan(Vec2::new(0.0, 100.0), 600.0);
        }
        assert!(
            v.pan.length() <= PAN_MAX_RADII + 1e-3,
            "pan length must be bounded: {}",
            v.pan.length()
        );
    }

    #[test]
    fn dolly_by_multiplies_and_clamps() {
        let mut v = View3dState::default();
        v.dolly_by(0.5);
        assert!((v.dolly - 0.5).abs() < 1e-6, "dolly is multiplicative");

        let mut close = View3dState::default();
        close.dolly_by(0.0001);
        assert!(
            (close.dolly - DOLLY_MIN).abs() < 1e-6,
            "dolly clamps to DOLLY_MIN"
        );

        let mut far = View3dState::default();
        far.dolly_by(1000.0);
        assert!(
            (far.dolly - DOLLY_MAX).abs() < 1e-6,
            "dolly clamps to DOLLY_MAX"
        );

        let mut keep = View3dState::default();
        keep.dolly_by(-1.0);
        keep.dolly_by(f32::NAN);
        assert!(
            (keep.dolly - 1.0).abs() < 1e-6,
            "invalid factors are ignored"
        );
    }

    #[test]
    fn snapping_to_a_face_reframes_by_clearing_pan_and_dolly() {
        // Clicking a view-cube face re-frames the model, so any accumulated pan or
        // dolly is cleared — otherwise the snapped view could stay panned off the
        // model or dollied inside it.
        let mut v = View3dState::default();
        v.pan = Vec3::new(2.0, -1.0, 0.5);
        v.dolly = 0.4;
        v.snap_to(ViewCubeAction::FRONT);
        assert_eq!(v.pan, Vec3::ZERO, "face snap must recenter the pan");
        assert!(
            (v.dolly - 1.0).abs() < 1e-6,
            "face snap must reset the dolly"
        );
    }

    /// The Render view and the interactive 3D view share one `View3dState`, so a
    /// given (yaw, pitch, zoom) must frame the model from the *same* vantage in
    /// both. The path tracer's [`framer_render::camera::Camera`] is built to match
    /// the [`OrbitProjector`]; this pins that agreement so orbiting in Render and
    /// switching back to 3D can never flip or mirror the camera.
    #[test]
    fn render_camera_matches_orbit_projector_orientation() {
        // Project a world point through the path tracer's camera into normalized
        // device coordinates (origin centered, +x right, +y up), plus its
        // view-space depth so we can require the probe sits in front of the eye.
        fn render_ndc(camera: &framer_render::camera::Camera, point: Point3) -> (f32, f32, f32) {
            let to_point = Vec3::new(point.x, point.y, point.z) - camera.eye;
            let depth = to_point.dot(camera.forward);
            let ndc_x = to_point.dot(camera.right) / depth / camera.half_w;
            let ndc_y = to_point.dot(camera.up) / depth / camera.half_h;
            (ndc_x, ndc_y, depth)
        }

        let points = model_3d_points(&BuildingModel::demo_shell()).unwrap();
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let center = model_3d_center(&points);
        let radius = model_3d_radius(&points, center).max(1.0);
        let aspect = drawing.width() / drawing.height();

        // A representative spread of orbit states: the default vantage, an
        // orbit-dragged view, a snapped side, and an arbitrary positive-yaw angle.
        let mut dragged = View3dState::default();
        dragged.orbit(Vec2::new(60.0, -25.0));
        let mut side = View3dState::default();
        side.snap_to(ViewCubeAction::RIGHT);
        let views = [
            View3dState::default(),
            dragged,
            side,
            View3dState {
                yaw: 0.7,
                pitch: 0.3,
                zoom: 1.0,
                ..View3dState::default()
            },
        ];

        // Probe points offset from the model center along each world axis (and a
        // couple of diagonals). The offset is a fraction of the radius so every
        // probe stays comfortably inside the frustum, where perspective cannot
        // flip a sign relative to the orthographic OrbitProjector.
        let d = radius * 0.3;
        let offsets = [
            (d, 0.0, 0.0),
            (-d, 0.0, 0.0),
            (0.0, d, 0.0),
            (0.0, -d, 0.0),
            (0.0, 0.0, d),
            (0.0, 0.0, -d),
            (d, d, 0.0),
            (-d, d, d),
        ];

        for view in views {
            let projector = OrbitProjector::from_points(&points, drawing, view).unwrap();
            let camera = framer_render::camera::Camera::orbit(
                Vec3::new(center.x, center.y, center.z),
                radius,
                view.yaw,
                view.pitch,
                view.zoom,
                aspect,
                36.0,
                1.0,
            );
            for (ox, oy, oz) in offsets {
                let point = Point3::vector(center.x + ox, center.y + oy, center.z + oz);
                let screen = projector.project_point(point).pos;
                let (ndc_x, ndc_y, depth) = render_ndc(&camera, point);
                assert!(
                    depth > 0.0,
                    "probe must sit in front of the render camera (yaw={}, pitch={})",
                    view.yaw,
                    view.pitch
                );

                // egui screen-space is y-down; render NDC is y-up. A correct
                // camera never disagrees in sign on either axis. Compare via the
                // product so axes a probe lands exactly on (≈0 in both) are not
                // tripped by floating-point dust.
                let screen_dx = screen.x - projector.origin.x;
                let screen_dy = screen.y - projector.origin.y;
                assert!(
                    screen_dx * ndc_x >= -1.0e-3,
                    "horizontal mismatch: yaw={}, pitch={}, offset=({ox}, {oy}, {oz}): \
                     screen_dx={screen_dx}, ndc_x={ndc_x}",
                    view.yaw,
                    view.pitch,
                );
                assert!(
                    -screen_dy * ndc_y >= -1.0e-3,
                    "vertical mismatch: yaw={}, pitch={}, offset=({ox}, {oy}, {oz}): \
                     screen_dy={screen_dy}, ndc_y={ndc_y}",
                    view.yaw,
                    view.pitch,
                );
            }
        }
    }

    /// Zoom must magnify the Render view uniformly — exactly like the orthographic
    /// 3D view, where a zoom of `z` scales every on-screen offset by `z` about the
    /// center. The path tracer achieves this with a telephoto zoom (narrowing the
    /// field of view at a fixed distance); a dolly would instead magnify by a
    /// depth-dependent amount and drift out of sync. Probes span a range of depths
    /// so a dolly's perspective exaggeration would be caught, not just focal-plane
    /// scale.
    #[test]
    fn render_zoom_magnifies_uniformly_like_the_orbit_projector() {
        fn render_ndc(camera: &framer_render::camera::Camera, point: Point3) -> (f32, f32) {
            let to_point = Vec3::new(point.x, point.y, point.z) - camera.eye;
            let depth = to_point.dot(camera.forward);
            (
                to_point.dot(camera.right) / depth / camera.half_w,
                to_point.dot(camera.up) / depth / camera.half_h,
            )
        }

        // Relative closeness — robust at pixel scale, yet far tighter than a
        // dolly's double-digit-percent magnification error off the focal plane.
        fn close(actual: f32, expected: f32) -> bool {
            (actual - expected).abs() <= 1.0e-3 * expected.abs().max(1.0)
        }

        let points = model_3d_points(&BuildingModel::demo_shell()).unwrap();
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let center = model_3d_center(&points);
        let radius = model_3d_radius(&points, center).max(1.0);
        let aspect = drawing.width() / drawing.height();
        let make_camera = |zoom: f32| {
            framer_render::camera::Camera::orbit(
                Vec3::new(center.x, center.y, center.z),
                radius,
                -FRAC_PI_4,
                0.5,
                zoom,
                aspect,
                36.0,
                1.0,
            )
        };

        let base_view = View3dState {
            yaw: -FRAC_PI_4,
            pitch: 0.5,
            zoom: 1.0,
            ..View3dState::default()
        };
        let base_proj = OrbitProjector::from_points(&points, drawing, base_view).unwrap();
        let base_cam = make_camera(1.0);

        // Offsets toward and away from the eye, not just across the focal plane.
        let d = radius * 0.35;
        let offsets = [
            (d, 0.0, 0.0),
            (0.0, d, 0.0),
            (0.0, 0.0, d),
            (-d, -d, d),
            (d, -d, -d),
        ];

        for zoom in [0.5_f32, 1.5, 2.5] {
            let zoom_proj =
                OrbitProjector::from_points(&points, drawing, View3dState { zoom, ..base_view })
                    .unwrap();
            let zoom_cam = make_camera(zoom);
            for (ox, oy, oz) in offsets {
                let point = Point3::vector(center.x + ox, center.y + oy, center.z + oz);

                // Orthographic 3D view: the offset from center scales by exactly zoom.
                let base_screen = base_proj.project_point(point).pos - base_proj.origin;
                let zoom_screen = zoom_proj.project_point(point).pos - zoom_proj.origin;
                assert!(
                    close(zoom_screen.x, base_screen.x * zoom)
                        && close(zoom_screen.y, base_screen.y * zoom),
                    "orbit projector zoom not uniform at zoom={zoom}, offset=({ox}, {oy}, {oz})"
                );

                // Render view: NDC must scale by the same zoom factor, regardless of
                // the probe's depth (telephoto, not dolly).
                let (bx, by) = render_ndc(&base_cam, point);
                let (zx, zy) = render_ndc(&zoom_cam, point);
                assert!(
                    close(zx, bx * zoom) && close(zy, by * zoom),
                    "render zoom not uniform at zoom={zoom}, offset=({ox}, {oy}, {oz}): \
                     base=({bx}, {by}) zoomed=({zx}, {zy})"
                );
            }
        }
    }

    #[test]
    fn wall_elevation_layout_preserves_wall_aspect_ratio() {
        let model = BuildingModel::demo_wall();
        let wall = &model.walls[0];
        let available = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 1000.0));
        let layout = WallElevationLayout::new(available, wall, &View2dState::default());

        assert_close(
            layout.wall_rect.width() / wall.length.inches() as f32,
            layout.scale,
        );
        assert_close(
            layout.wall_rect.height() / wall.height.inches() as f32,
            layout.scale,
        );
        assert_close(
            layout.wall_rect.width() / layout.wall_rect.height(),
            wall.length.inches() as f32 / wall.height.inches() as f32,
        );
        assert_close(layout.wall_rect.center().x, available.center().x);
        assert_close(layout.wall_rect.center().y, available.center().y);
    }

    #[test]
    fn dimension_placement_pointer_chooses_closest_axis() {
        let first = Pos2::new(100.0, 180.0);
        let second = Pos2::new(240.0, 120.0);
        let midpoint = first + (second - first) * 0.5;

        assert_eq!(
            dimension_axis_for_placement_position(
                first,
                second,
                Some(midpoint + Vec2::new(160.0, 20.0)),
                DimensionAxis::Horizontal,
            ),
            DimensionAxis::Vertical
        );
        assert_eq!(
            dimension_axis_for_placement_position(
                first,
                second,
                Some(midpoint + Vec2::new(20.0, -160.0)),
                DimensionAxis::Vertical,
            ),
            DimensionAxis::Horizontal
        );
        assert_eq!(
            dimension_axis_for_placement_position(
                first,
                second,
                Some(midpoint),
                DimensionAxis::Vertical,
            ),
            DimensionAxis::Vertical
        );
    }

    #[test]
    fn opening_edit_hit_testing_prioritizes_resize_handles() {
        let rect = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::new(120.0, 72.0));

        assert_eq!(
            hit_opening_edit_handle(rect, rect.right_top()),
            Some(OpeningEditHandle::TopRight)
        );
        assert_eq!(
            hit_opening_edit_handle(rect, Pos2::new(rect.right(), rect.center().y)),
            Some(OpeningEditHandle::Right)
        );
        assert_eq!(
            hit_opening_edit_handle(rect, rect.center()),
            Some(OpeningEditHandle::Move)
        );
        assert_eq!(
            hit_opening_edit_handle(rect, rect.right_bottom() + Vec2::splat(16.0)),
            None
        );
    }

    #[test]
    fn opening_move_hit_testing_includes_dimension_anchor_rim() {
        let rect = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::new(120.0, 72.0));

        assert!(hit_opening_move_handle(
            rect,
            Pos2::new(rect.left() - 8.0, rect.center().y)
        ));
        assert_eq!(
            hit_opening_edit_handle(rect, Pos2::new(rect.left() - 8.0, rect.center().y)),
            None
        );
        assert_eq!(
            hit_opening_edit_handle(rect, Pos2::new(rect.left(), rect.center().y)),
            Some(OpeningEditHandle::Left)
        );
    }

    #[test]
    fn opening_drag_delta_maps_screen_motion_to_wall_axes() {
        let (delta_x, delta_y) = opening_drag_delta(Vec2::new(20.0, -12.0), 2.0);

        assert_eq!(delta_x, Length::from_inches(10.0));
        assert_eq!(delta_y, Length::from_inches(6.0));
    }

    #[test]
    fn dimension_anchor_markers_include_edges_vertices_and_centers() {
        let model = BuildingModel::demo_wall();
        let wall = &model.walls[0];
        let drawing = Rect::from_min_size(
            Pos2::new(100.0, 80.0),
            Vec2::new(wall.length.inches() as f32, wall.height.inches() as f32),
        );

        let markers = dimension_anchor_markers(drawing, 1.0, 1.0, wall);
        let opening = wall.openings[0].id.clone();

        assert!(markers.iter().any(|marker| {
            marker.anchor
                == DimensionAnchor::WallPoint {
                    horizontal: DimensionHorizontalReference::Left,
                    vertical: DimensionVerticalReference::Top,
                }
                && marker.kind == DimensionAnchorKind::Vertex
        }));
        assert!(markers.iter().any(|marker| {
            marker.anchor
                == DimensionAnchor::WallPoint {
                    horizontal: DimensionHorizontalReference::Center,
                    vertical: DimensionVerticalReference::Center,
                }
                && marker.kind == DimensionAnchorKind::Center
        }));
        assert!(markers.iter().any(|marker| {
            marker.anchor
                == DimensionAnchor::OpeningPoint {
                    opening: opening.clone(),
                    horizontal: DimensionHorizontalReference::Center,
                    vertical: DimensionVerticalReference::Top,
                }
                && marker.kind == DimensionAnchorKind::Edge
        }));
    }

    #[test]
    fn dimension_anchor_hit_testing_prioritizes_vertices() {
        let model = BuildingModel::demo_wall();
        let wall = &model.walls[0];
        let opening = &wall.openings[0];
        let drawing = Rect::from_min_size(
            Pos2::new(100.0, 80.0),
            Vec2::new(wall.length.inches() as f32, wall.height.inches() as f32),
        );
        let opening_rect = opening_rect(drawing, 1.0, 1.0, opening);

        assert_eq!(
            hit_dimension_anchor(opening_rect.left_top(), drawing, 1.0, 1.0, wall),
            Some(DimensionAnchor::OpeningPoint {
                opening: opening.id.clone(),
                horizontal: DimensionHorizontalReference::Left,
                vertical: DimensionVerticalReference::Top,
            })
        );
    }

    #[test]
    fn dimension_line_offsets_map_between_screen_and_wall_coordinates() {
        let drawing = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::new(240.0, 120.0));
        let scale = 2.0;

        let horizontal_position = Pos2::new(160.0, 140.0);
        let horizontal_offset = dimension_line_offset_for_position(
            drawing,
            scale,
            DimensionAxis::Horizontal,
            horizontal_position,
        );
        assert_eq!(horizontal_offset, Length::from_inches(30.0));
        assert_eq!(
            dimension_line_screen_position(
                drawing,
                scale,
                DimensionAxis::Horizontal,
                horizontal_offset
            ),
            horizontal_position.y
        );

        let vertical_position = Pos2::new(250.0, 120.0);
        let vertical_offset = dimension_line_offset_for_position(
            drawing,
            scale,
            DimensionAxis::Vertical,
            vertical_position,
        );
        assert_eq!(vertical_offset, Length::from_inches(75.0));
        assert_eq!(
            dimension_line_screen_position(
                drawing,
                scale,
                DimensionAxis::Vertical,
                vertical_offset
            ),
            vertical_position.x
        );
    }

    #[test]
    fn dimension_label_rect_sizes_to_text_instead_of_fixed_block() {
        let start = Pos2::new(100.0, 120.0);
        let end = Pos2::new(180.0, 120.0);

        let short_label = dimension_label_rect(start, end, DimensionAxis::Horizontal, "1' 6\"");
        let long_label =
            dimension_label_rect(start, end, DimensionAxis::Horizontal, "28' 0\" x 8' 0\"");

        assert!(short_label.width() < 50.0);
        assert!(long_label.width() > short_label.width());
        assert_eq!(
            short_label.center(),
            dimension_label_position(start, end, DimensionAxis::Horizontal)
        );
    }

    #[test]
    fn view_cube_geometry_hits_clickable_faces() {
        let rect = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::splat(104.0));
        let geometry = ViewCubeGeometry::from_rect(rect, View3dState::default());
        let top_face = geometry
            .faces
            .iter()
            .find(|face| face.action == ViewCubeAction::TOP)
            .expect("default view shows the top face");
        let right_face = geometry
            .faces
            .iter()
            .find(|face| face.action == ViewCubeAction::RIGHT)
            .expect("default view shows the right face");
        let front_face = geometry
            .faces
            .iter()
            .find(|face| face.action == ViewCubeAction::FRONT)
            .expect("default view shows the front face");

        assert_eq!(
            geometry.hit(geometry.home_rect.center()),
            Some(ViewCubeAction::Home)
        );
        assert_eq!(
            geometry.hit(view_cube_face_center(top_face)),
            Some(ViewCubeAction::TOP)
        );
        assert_eq!(
            geometry.hit(view_cube_face_center(right_face)),
            Some(ViewCubeAction::RIGHT)
        );
        assert_eq!(
            geometry.hit(view_cube_face_center(front_face)),
            Some(ViewCubeAction::FRONT)
        );
        assert_eq!(
            geometry.hit(rect.left_bottom() + Vec2::new(4.0, -4.0)),
            None
        );
    }

    #[test]
    fn view_cube_geometry_hits_unlabeled_faces_edges_and_corners() {
        let rect = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::splat(104.0));
        let mut left_view = View3dState::default();
        left_view.snap_to(ViewCubeAction::LEFT);
        let left_geometry = ViewCubeGeometry::from_rect(rect, left_view);
        let left_face = left_geometry
            .faces
            .iter()
            .find(|face| face.action == ViewCubeAction::LEFT)
            .expect("left face should be visible after left snap");
        assert_eq!(
            left_geometry.hit(view_cube_face_center(left_face)),
            Some(ViewCubeAction::LEFT)
        );

        let mut bottom_view = View3dState::default();
        bottom_view.snap_to(ViewCubeAction::BOTTOM);
        let bottom_geometry = ViewCubeGeometry::from_rect(rect, bottom_view);
        let bottom_face = bottom_geometry
            .faces
            .iter()
            .find(|face| face.action == ViewCubeAction::BOTTOM)
            .expect("bottom face should be visible after bottom snap");
        assert_eq!(
            bottom_geometry.hit(view_cube_face_center(bottom_face)),
            Some(ViewCubeAction::BOTTOM)
        );

        let geometry = ViewCubeGeometry::from_rect(rect, View3dState::default());
        let top_front = ViewCubeAction::snap(ViewCubeOrientation::new(0, 1, 1));
        let top_front_edge = geometry
            .edges
            .iter()
            .find(|edge| edge.action == top_front)
            .expect("default view shows the top/front edge");
        let edge_center = top_front_edge.points[0].lerp(top_front_edge.points[1], 0.5);
        assert_eq!(geometry.hit(edge_center), Some(top_front));

        let top_front_right = ViewCubeAction::snap(ViewCubeOrientation::new(1, 1, 1));
        let top_front_right_corner = geometry
            .corners
            .iter()
            .find(|corner| corner.action == top_front_right)
            .expect("default view shows the top/front/right corner");
        assert_eq!(
            geometry.hit(top_front_right_corner.center),
            Some(top_front_right)
        );
    }

    #[test]
    fn view_cube_drag_ownership_uses_press_origin() {
        let rect = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::splat(104.0));

        assert!(pointer_started_in_rect(Some(rect.center()), rect));
        assert!(!pointer_started_in_rect(
            Some(rect.right_bottom() + Vec2::splat(1.0)),
            rect
        ));
        assert!(!pointer_started_in_rect(None, rect));
    }

    #[test]
    fn view_cube_mesh_builds_solid_cube_faces() {
        let (vertices, indices) = view_cube_mesh(None);

        assert_eq!(vertices.len(), 24);
        assert_eq!(indices.len(), 36);
        assert!(
            vertices
                .iter()
                .any(|vertex| vertex.normal == [0.0, 0.0, 1.0])
        );
        assert!(
            vertices
                .iter()
                .any(|vertex| vertex.normal == [1.0, 0.0, 0.0])
        );
        assert!(
            vertices
                .iter()
                .any(|vertex| vertex.normal == [0.0, 1.0, 0.0])
        );
    }

    #[test]
    fn view_cube_label_specs_stay_on_visible_face_planes() {
        let [top, right, front] = view_cube_label_specs();

        assert_eq!(top.text, "TOP");
        assert_close(top.center.z, 1.0);
        assert_close(top.u_axis.y, 1.0);
        assert_eq!(right.text, "RIGHT");
        assert_close(right.center.x, 1.0);
        assert_eq!(front.text, "FRONT");
        assert_close(front.center.y, 1.0);
    }

    #[test]
    fn scene_3d_builds_depth_tested_wall_and_member_cuboids() {
        let model = BuildingModel::demo_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let scene =
            Scene3d::from_project(&model, &plan, 0, &Selection::Wall, WorkspaceMode::Plan).unwrap();
        let wall_depth = model.code.stud_profile.nominal_depth().inches() as f32;

        assert!(!scene.vertices.is_empty());
        assert!(scene.opaque_index_count > 0);
        assert!(scene.transparent_index_count > 0);
        assert_eq!(scene.opaque_index_count % 36, 0);
        assert_eq!(scene.transparent_index_count % 36, 0);

        let min_y = scene
            .points
            .iter()
            .map(|point| point.y)
            .fold(f32::MAX, f32::min);
        assert!(
            min_y <= -wall_depth / 2.0,
            "front wall should have real thickness in plan depth"
        );
    }

    #[test]
    fn scene_3d_contains_pickable_members_openings_and_walls() {
        let model = BuildingModel::demo_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let plan_scene =
            Scene3d::from_project(&model, &plan, 0, &Selection::Wall, WorkspaceMode::Plan).unwrap();

        assert!(
            plan_scene
                .picks
                .iter()
                .any(|pick| matches!(&pick.click, ViewClick::Wall(0)))
        );
        assert!(
            plan_scene
                .picks
                .iter()
                .any(|pick| matches!(&pick.click, ViewClick::Opening { .. }))
        );
        assert!(
            plan_scene
                .picks
                .iter()
                .any(|pick| matches!(&pick.click, ViewClick::Member { .. }))
        );

        let design_scene =
            Scene3d::from_project(&model, &plan, 0, &Selection::Wall, WorkspaceMode::Design)
                .unwrap();
        assert!(
            design_scene
                .picks
                .iter()
                .any(|pick| matches!(&pick.click, ViewClick::Wall(0)))
        );
        assert!(
            design_scene
                .picks
                .iter()
                .any(|pick| matches!(&pick.click, ViewClick::Opening { .. }))
        );
        assert!(
            design_scene
                .picks
                .iter()
                .all(|pick| !matches!(&pick.click, ViewClick::Member { .. }))
        );
    }

    #[test]
    fn render_resolution_uses_native_device_pixels_when_within_bounds() {
        // A settled frame (res_scale = 1.0) on a hi-DPI pane must render at full
        // device resolution. The old per-axis clamp capped width to 1000 px,
        // which is what made stationary frames look soft and jagged.
        let (w, h) = render_resolution(700.0, 500.0, 2.0, 1.0);
        assert_eq!((w, h), (1400, 1000));
    }

    #[test]
    fn render_resolution_preserves_aspect_on_tall_pane() {
        // Regression: width/height used to be clamped independently to 1000,
        // squishing a portrait pane toward square. Aspect must be preserved.
        let (w, h) = render_resolution(600.0, 900.0, 2.0, 1.0);
        assert!(h > w, "portrait pane must stay portrait, got {w}x{h}");
        let ratio = w as f32 / h as f32;
        assert!(
            (ratio - 600.0 / 900.0).abs() < 0.01,
            "aspect {ratio} should match 600/900"
        );
    }

    #[test]
    fn render_resolution_caps_long_axis_preserving_aspect() {
        // Oversized pane: the long axis is capped to MAX_RENDER_DIM and the short
        // axis scales by the same factor, rather than clamping each axis alone.
        let (w, h) = render_resolution(1500.0, 1000.0, 2.0, 1.0);
        assert_eq!(w.max(h), MAX_RENDER_DIM);
        let ratio = w as f32 / h as f32;
        assert!(
            (ratio - 1.5).abs() < 0.01,
            "aspect {ratio} should match 1.5"
        );
    }

    #[test]
    fn render_resolution_floors_tiny_pane_to_min() {
        let (w, h) = render_resolution(20.0, 15.0, 1.0, 1.0);
        assert_eq!(w.min(h), MIN_RENDER_DIM);
        let ratio = w as f32 / h as f32;
        assert!(
            (ratio - 20.0 / 15.0).abs() < 0.05,
            "aspect {ratio} should match 20/15"
        );
    }

    #[test]
    fn render_resolution_motion_scale_shrinks_uniformly() {
        let still = render_resolution(800.0, 600.0, 2.0, 1.0);
        let moving = render_resolution(800.0, 600.0, 2.0, 0.5);
        assert_eq!(still, (1600, 1200));
        assert_eq!(moving, (800, 600));
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {actual} to be close to {expected}"
        );
    }

    fn view_cube_face_center(face: &ViewCubeFaceGeometry) -> Pos2 {
        let center = face
            .points
            .iter()
            .fold(Vec2::ZERO, |sum, point| sum + point.to_vec2())
            / face.points.len() as f32;
        Pos2::new(center.x, center.y)
    }
}
