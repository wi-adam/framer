use std::ops::Neg;
use std::{
    borrow::Cow,
    collections::HashMap,
    f32::consts::{FRAC_PI_2, FRAC_PI_4},
};

use bytemuck::{Pod, Zeroable};
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
use wgpu::util::DeviceExt as _;

use super::labels::{join_kind_label, kind_label};
use super::model_edit::{OpeningDragState, OpeningEditHandle};
use super::{FramerApp, Selection, ViewClick, ViewportMode, WorkspaceMode, design, theme};

#[derive(Debug, Clone, Copy)]
pub(super) struct View3dState {
    yaw: f32,
    pitch: f32,
    zoom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewCubeAction {
    Home,
    Snap(ViewCubeOrientation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ViewCubeOrientation {
    x: i8,
    y: i8,
    z: i8,
}

impl ViewCubeOrientation {
    const TOP: Self = Self { x: 0, y: 0, z: 1 };
    const BOTTOM: Self = Self { x: 0, y: 0, z: -1 };
    const FRONT: Self = Self { x: 0, y: 1, z: 0 };
    const BACK: Self = Self { x: 0, y: -1, z: 0 };
    const RIGHT: Self = Self { x: 1, y: 0, z: 0 };
    const LEFT: Self = Self { x: -1, y: 0, z: 0 };

    fn new(x: i8, y: i8, z: i8) -> Self {
        Self { x, y, z }
    }

    fn from_point(point: Point3) -> Self {
        Self::new(
            point.x.signum() as i8,
            point.y.signum() as i8,
            point.z.signum() as i8,
        )
    }

    fn from_points(start: Point3, end: Point3) -> Self {
        let component = |left: f32, right: f32| {
            if (left - right).abs() <= f32::EPSILON {
                left.signum() as i8
            } else {
                0
            }
        };
        Self::new(
            component(start.x, end.x),
            component(start.y, end.y),
            component(start.z, end.z),
        )
    }

    fn component_count(self) -> usize {
        [self.x, self.y, self.z]
            .into_iter()
            .filter(|component| *component != 0)
            .count()
    }

    fn includes_face(self, face: Self) -> bool {
        face.component_count() == 1
            && (face.x == 0 || self.x == face.x)
            && (face.y == 0 || self.y == face.y)
            && (face.z == 0 || self.z == face.z)
    }
}

impl ViewCubeAction {
    const TOP: Self = Self::Snap(ViewCubeOrientation::TOP);
    const BOTTOM: Self = Self::Snap(ViewCubeOrientation::BOTTOM);
    const FRONT: Self = Self::Snap(ViewCubeOrientation::FRONT);
    const BACK: Self = Self::Snap(ViewCubeOrientation::BACK);
    const RIGHT: Self = Self::Snap(ViewCubeOrientation::RIGHT);
    const LEFT: Self = Self::Snap(ViewCubeOrientation::LEFT);

    fn snap(orientation: ViewCubeOrientation) -> Self {
        Self::Snap(orientation)
    }

    fn orientation(self) -> Option<ViewCubeOrientation> {
        match self {
            Self::Home => None,
            Self::Snap(orientation) => Some(orientation),
        }
    }
}

impl Default for View3dState {
    fn default() -> Self {
        Self {
            yaw: -FRAC_PI_4,
            pitch: 0.55,
            zoom: 1.0,
        }
    }
}

impl View3dState {
    fn orbit(&mut self, delta: Vec2) {
        self.yaw += delta.x * 0.01;
        self.pitch = (self.pitch - delta.y * 0.01).clamp(-FRAC_PI_2 + 0.02, FRAC_PI_2 - 0.02);
    }

    fn zoom_by(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(0.35, 3.0);
        }
    }

    fn snap_to(&mut self, action: ViewCubeAction) {
        match action {
            ViewCubeAction::Home => *self = Self::default(),
            ViewCubeAction::Snap(orientation) => {
                let x = orientation.x as f32;
                let y = orientation.y as f32;
                let z = orientation.z as f32;
                let horizontal = (x * x + y * y).sqrt();
                self.pitch = z.atan2(horizontal);
                if horizontal > f32::EPSILON {
                    self.yaw = (-x / horizontal).atan2(y / horizontal);
                } else {
                    self.yaw = 0.0;
                }
            }
        }
    }
}

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
        let click = match self.viewport_mode {
            ViewportMode::Plan => draw_project_plan(
                ui,
                &self.model,
                self.selected_wall,
                &self.selected,
                self.grid,
                &mut self.cursor_model,
                &mut toolbar_anchor,
            ),
            ViewportMode::Elevation => {
                let Some(wall) = self.model.walls.get(self.selected_wall) else {
                    ui.label("No wall selected");
                    return;
                };
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
                    draw_wall_elevation(ui, wall, &wall_plan.members, selected_member, section_x)
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

        // Orbit + zoom, mirroring the 3D workspace controls.
        if response.dragged_by(egui::PointerButton::Primary) {
            self.view_3d.orbit(response.drag_delta());
        }
        if response.hovered() {
            let zoom_factor = ui.input(|input| {
                let wheel = (input.smooth_scroll_delta.y * 0.002).exp();
                wheel * input.zoom_delta()
            });
            if (zoom_factor - 1.0).abs() > f32::EPSILON {
                self.view_3d.zoom_by(zoom_factor);
            }
        }

        if self.model.walls.is_empty() {
            draw_view_empty(&painter, drawing, "No geometry to render");
            return;
        }

        // Internal render resolution: device pixels, capped, aspect-matched.
        let ppp = ui.ctx().pixels_per_point();
        let aspect = (drawing.width() / drawing.height()).max(0.1);
        let width = ((drawing.width() * ppp).round() as u32).clamp(64, 1000);
        let height = ((width as f32 / aspect).round() as u32).clamp(64, 1000);

        let opts = framer_render::RenderOptions {
            yaw: self.view_3d.yaw,
            pitch: self.view_3d.pitch,
            zoom: self.view_3d.zoom,
            aspect: width as f32 / height as f32,
            ..framer_render::RenderOptions::default()
        };

        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            super::render_job::model_signature(&self.model).hash(&mut hasher);
            ((self.view_3d.yaw * 2000.0) as i64).hash(&mut hasher);
            ((self.view_3d.pitch * 2000.0) as i64).hash(&mut hasher);
            ((self.view_3d.zoom * 1000.0) as i64).hash(&mut hasher);
            width.hash(&mut hasher);
            height.hash(&mut hasher);
            hasher.finish()
        };

        self.render_view
            .update(&ctx, &self.model, opts, width, height, key);

        if let Some(texture) = self.render_view.texture() {
            let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
            painter.image(texture.id(), drawing, uv, Color32::WHITE);
        } else {
            draw_view_empty(&painter, drawing, "Preparing render…");
        }

        // Progress / quality readout.
        let samples = self.render_view.samples();
        let target = self.render_view.target_spp();
        let label = if self.render_view.is_accumulating() {
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

        // Keep refining until converged (or while the user is interacting).
        if self.render_view.is_accumulating() || response.dragged() {
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

fn viewport_size(ui: &Ui) -> Vec2 {
    // Fill the central panel so the drawing surface reaches the panel edges; the
    // plan/3D projectors letterbox their content within this rect.
    let available = ui.available_size();
    Vec2::new(available.x.max(420.0), available.y.max(360.0))
}

fn viewport_drawing_rect(rect: Rect, margin: f32) -> Rect {
    Rect::from_min_max(
        rect.min + Vec2::splat(margin),
        rect.max - Vec2::new(margin, margin),
    )
}

fn draw_view_title(painter: &egui::Painter, drawing: Rect, title: impl Into<String>) {
    painter.text(
        drawing.left_top() + Vec2::new(0.0, -20.0),
        Align2::LEFT_CENTER,
        title.into(),
        FontId::proportional(13.0),
        theme::framing_line_dark(),
    );
}

fn draw_view_empty(painter: &egui::Painter, rect: Rect, label: &str) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(14.0),
        theme::text_muted(),
    );
}

fn draw_view_border(painter: &egui::Painter, drawing: Rect) {
    painter.rect_stroke(
        drawing,
        0.0,
        Stroke::new(1.0, theme::sheet_grid_major()),
        StrokeKind::Outside,
    );
}

fn draw_view_background(painter: &egui::Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect, 0.0, color);
}

fn draw_drafting_grid(painter: &egui::Painter, drawing: Rect) {
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

fn draw_drafting_rulers(painter: &egui::Painter, rect: Rect, drawing: Rect) {
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

fn draw_plan_axis_indicator(painter: &egui::Painter, rect: Rect) {
    let origin = rect.left_bottom() + Vec2::new(32.0, -30.0);
    let x_end = origin + Vec2::new(36.0, 0.0);
    let y_end = origin + Vec2::new(0.0, -36.0);
    painter.line_segment(
        [origin, x_end],
        Stroke::new(1.5, Color32::from_rgb(204, 60, 48)),
    );
    painter.line_segment(
        [origin, y_end],
        Stroke::new(1.5, Color32::from_rgb(52, 158, 83)),
    );
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

fn draw_selection_handle(painter: &egui::Painter, point: Pos2) {
    let handle = Rect::from_center_size(point, Vec2::splat(8.0));
    painter.rect_filled(handle, 1.5, theme::active_blue());
    painter.rect_stroke(
        handle,
        1.5,
        Stroke::new(1.0, theme::sheet()),
        StrokeKind::Outside,
    );
}

fn draw_selected_wall_handles(painter: &egui::Painter, start: Pos2, end: Pos2) {
    draw_selection_handle(painter, start);
    draw_selection_handle(painter, end);
    draw_selection_handle(
        painter,
        Pos2::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0),
    );
}

fn draw_project_plan(
    ui: &mut Ui,
    model: &BuildingModel,
    selected_wall: usize,
    selection: &Selection,
    show_grid: bool,
    cursor_out: &mut Option<Point2>,
    toolbar_out: &mut Option<Pos2>,
) -> Option<ViewClick> {
    let desired = viewport_size(ui);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let painter = ui.painter_at(rect);

    draw_view_background(&painter, rect, theme::sheet());
    let drawing = viewport_drawing_rect(rect, 58.0);
    draw_drafting_rulers(&painter, rect, drawing);
    if show_grid {
        draw_drafting_grid(&painter, drawing);
    }
    draw_view_border(&painter, drawing);

    let Some(bounds) = ModelBounds::from_model(model) else {
        draw_view_empty(&painter, rect, "No wall segments");
        return None;
    };

    if let Some(hover) = response.hover_pos() {
        if drawing.contains(hover) {
            *cursor_out = Some(plan_inverse_point(hover, bounds, drawing));
        }
    }

    let pointer = response.interact_pointer_pos();
    let mut clicked_wall = None;
    let mut clicked_opening = None;

    for join in &model.wall_joins {
        let point = plan_point(join.point, bounds, drawing);
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
        let start = plan_point(wall.start, bounds, drawing);
        let end = plan_point(wall.end, bounds, drawing);
        let hovered =
            pointer.is_some_and(|position| distance_to_segment(position, start, end) < 8.0);
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
            draw_selected_wall_handles(&painter, start, end);
        }
        if hovered && response.clicked() {
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
            let left = plan_point(wall.point_at_local_x(opening.left()), bounds, drawing);
            let right = plan_point(wall.point_at_local_x(opening.right()), bounds, drawing);
            let opening_hovered =
                pointer.is_some_and(|position| distance_to_segment(position, left, right) < 9.0);
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
            if opening_hovered && response.clicked() {
                clicked_opening = Some(ViewClick::Opening {
                    wall_index: index,
                    opening_id: opening.id.0.clone(),
                });
            }
        }
    }

    let scale = (drawing.width() / (bounds.max_x - bounds.min_x).max(1.0))
        .min(drawing.height() / (bounds.max_y - bounds.min_y).max(1.0));
    draw_scale_bar(&painter, drawing, scale);
    draw_view_title(&painter, drawing, "Whole-project plan");
    draw_plan_axis_indicator(&painter, drawing);

    clicked_opening.or(clicked_wall)
}

/// A drafting scale bar at the bottom-left of the plan, sized to a round number
/// of feet given the current pixels-per-inch `scale`.
fn draw_scale_bar(painter: &egui::Painter, drawing: Rect, scale: f32) {
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
    let dragging_primary = response.dragged_by(egui::PointerButton::Primary);
    let dragging_from_cube = dragging_primary && pointer_started_in_rect(press_origin, cube_rect);

    if dragging_primary {
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

const FRAMER_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

const FRAMER_3D_SHADER: &str = r#"
struct Uniforms {
    center: vec4<f32>,
    right: vec4<f32>,
    depth_axis: vec4<f32>,
    raw_center_scale: vec4<f32>,
    depth_center_scale: vec4<f32>,
};

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) normal: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let local = input.position - uniforms.center.xyz;
    let raw_x = dot(local.xy, uniforms.right.xy);
    let along_depth = dot(local.xy, uniforms.depth_axis.xy);
    let raw_y = along_depth * uniforms.depth_axis.z - local.z * uniforms.depth_axis.w;
    let depth = along_depth * uniforms.depth_axis.w + local.z * uniforms.depth_axis.z;

    var output: VertexOutput;
    output.position = vec4<f32>(
        (raw_x - uniforms.raw_center_scale.x) * uniforms.raw_center_scale.z,
        -(raw_y - uniforms.raw_center_scale.y) * uniforms.raw_center_scale.w,
        clamp(uniforms.depth_center_scale.z - (depth - uniforms.depth_center_scale.x) * uniforms.depth_center_scale.y, 0.0, 1.0),
        1.0,
    );
    output.color = input.color;
    output.normal = input.normal;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(-0.35, -0.45, 0.82));
    let shade = 0.50 + 0.50 * max(dot(normalize(input.normal), light), 0.0);
    return vec4<f32>(input.color.rgb * shade, input.color.a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 3],
    color: [f32; 4],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuUniforms {
    center: [f32; 4],
    right: [f32; 4],
    depth_axis: [f32; 4],
    raw_center_scale: [f32; 4],
    depth_center_scale: [f32; 4],
}

impl GpuUniforms {
    fn from_projector(projector: &OrbitProjector, drawing: Rect) -> Self {
        let scale_x = projector.scale / (drawing.width() / 2.0).max(1.0);
        let scale_y = projector.scale / (drawing.height() / 2.0).max(1.0);
        Self {
            center: [
                projector.center.x,
                projector.center.y,
                projector.center.z,
                0.0,
            ],
            right: [projector.right.x, projector.right.y, 0.0, 0.0],
            depth_axis: [
                projector.depth_axis.x,
                projector.depth_axis.y,
                projector.pitch.sin(),
                projector.pitch.cos(),
            ],
            raw_center_scale: [
                projector.raw_center.x,
                projector.raw_center.y,
                scale_x,
                scale_y,
            ],
            depth_center_scale: [projector.depth_center, projector.depth_scale, 0.5, 0.0],
        }
    }

    fn from_projector_with_depth_base(
        projector: &OrbitProjector,
        drawing: Rect,
        depth_base: f32,
    ) -> Self {
        let mut uniforms = Self::from_projector(projector, drawing);
        uniforms.depth_center_scale[2] = depth_base;
        uniforms
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Framer3dFrameKey(u64);

impl Framer3dFrameKey {
    const MODEL: Self = Self(1);
    const VIEW_CUBE: Self = Self(2);
}

struct Framer3dCallback {
    frame_key: Framer3dFrameKey,
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    opaque_index_count: u32,
    transparent_index_count: u32,
    uniforms: GpuUniforms,
    target_format: wgpu::TextureFormat,
}

struct Framer3dResources {
    target_format: wgpu::TextureFormat,
    bind_group_layout: wgpu::BindGroupLayout,
    opaque_pipeline: wgpu::RenderPipeline,
    transparent_pipeline: wgpu::RenderPipeline,
}

struct Framer3dFrame {
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    opaque_index_count: u32,
    transparent_index_count: u32,
}

#[derive(Default)]
struct Framer3dFrameStore {
    frames: HashMap<Framer3dFrameKey, Framer3dFrame>,
}

impl egui_wgpu::CallbackTrait for Framer3dCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let needs_resources = callback_resources
            .get::<Framer3dResources>()
            .is_none_or(|resources| resources.target_format != self.target_format);
        if needs_resources {
            callback_resources.insert(Framer3dResources::new(device, self.target_format));
        }
        let resources = callback_resources
            .get::<Framer3dResources>()
            .expect("3D render resources should exist");

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("framer_3d_vertices"),
            contents: bytemuck::cast_slice(&self.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("framer_3d_indices"),
            contents: bytemuck::cast_slice(&self.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("framer_3d_uniforms"),
            contents: bytemuck::bytes_of(&self.uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("framer_3d_bind_group"),
            layout: &resources.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        if callback_resources.get::<Framer3dFrameStore>().is_none() {
            callback_resources.insert(Framer3dFrameStore::default());
        }
        let frame_store = callback_resources
            .get_mut::<Framer3dFrameStore>()
            .expect("3D frame store should exist");
        frame_store.frames.insert(
            self.frame_key,
            Framer3dFrame {
                bind_group,
                vertex_buffer,
                index_buffer,
                opaque_index_count: self.opaque_index_count,
                transparent_index_count: self.transparent_index_count,
            },
        );

        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<Framer3dResources>() else {
            return;
        };
        let Some(frame) = callback_resources
            .get::<Framer3dFrameStore>()
            .and_then(|store| store.frames.get(&self.frame_key))
        else {
            return;
        };

        render_pass.set_bind_group(0, &frame.bind_group, &[]);
        render_pass.set_vertex_buffer(0, frame.vertex_buffer.slice(..));
        render_pass.set_index_buffer(frame.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        if frame.opaque_index_count > 0 {
            render_pass.set_pipeline(&resources.opaque_pipeline);
            render_pass.draw_indexed(0..frame.opaque_index_count, 0, 0..1);
        }

        if frame.transparent_index_count > 0 {
            let start = frame.opaque_index_count;
            let end = start + frame.transparent_index_count;
            render_pass.set_pipeline(&resources.transparent_pipeline);
            render_pass.draw_indexed(start..end, 0, 0..1);
        }
    }
}

impl Framer3dResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("framer_3d_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("framer_3d_shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(FRAMER_3D_SHADER)),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("framer_3d_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let opaque_pipeline =
            create_3d_pipeline(device, &pipeline_layout, &shader, target_format, None, true);
        let transparent_pipeline = create_3d_pipeline(
            device,
            &pipeline_layout,
            &shader,
            target_format,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
        );

        Self {
            target_format,
            bind_group_layout,
            opaque_pipeline,
            transparent_pipeline,
        }
    }
}

fn create_3d_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    depth_write_enabled: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("framer_3d_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GpuVertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x4,
                    2 => Float32x3
                ],
            }],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: FRAMER_DEPTH_FORMAT,
            depth_write_enabled: Some(depth_write_enabled),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
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

#[derive(Clone, Copy)]
struct ModelBounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl ModelBounds {
    fn from_model(model: &BuildingModel) -> Option<Self> {
        let mut bounds = None::<Self>;
        for point in model.walls.iter().flat_map(|wall| [wall.start, wall.end]) {
            let x = point.x.inches() as f32;
            let y = point.y.inches() as f32;
            bounds = Some(match bounds {
                Some(existing) => Self {
                    min_x: existing.min_x.min(x),
                    min_y: existing.min_y.min(y),
                    max_x: existing.max_x.max(x),
                    max_y: existing.max_y.max(y),
                },
                None => Self {
                    min_x: x,
                    min_y: y,
                    max_x: x,
                    max_y: y,
                },
            });
        }
        bounds
    }
}

fn plan_point(point: Point2, bounds: ModelBounds, drawing: Rect) -> Pos2 {
    let width = (bounds.max_x - bounds.min_x).max(1.0);
    let depth = (bounds.max_y - bounds.min_y).max(1.0);
    let scale = (drawing.width() / width).min(drawing.height() / depth);
    let used_width = width * scale;
    let used_height = depth * scale;
    Pos2::new(
        drawing.left()
            + (drawing.width() - used_width) / 2.0
            + (point.x.inches() as f32 - bounds.min_x) * scale,
        drawing.bottom()
            - (drawing.height() - used_height) / 2.0
            - (point.y.inches() as f32 - bounds.min_y) * scale,
    )
}

/// Inverse of [`plan_point`]: map a screen position back to model coordinates.
fn plan_inverse_point(screen: Pos2, bounds: ModelBounds, drawing: Rect) -> Point2 {
    let width = (bounds.max_x - bounds.min_x).max(1.0);
    let depth = (bounds.max_y - bounds.min_y).max(1.0);
    let scale = (drawing.width() / width).min(drawing.height() / depth);
    let used_width = width * scale;
    let used_height = depth * scale;
    let x =
        bounds.min_x + (screen.x - drawing.left() - (drawing.width() - used_width) / 2.0) / scale;
    let y = bounds.min_y
        + (drawing.bottom() - (drawing.height() - used_height) / 2.0 - screen.y) / scale;
    Point2::new(Length::from_inches(x as f64), Length::from_inches(y as f64))
}

#[derive(Clone, Copy)]
struct Point3 {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Clone, Copy)]
struct ProjectedPoint {
    pos: Pos2,
    depth: f32,
}

struct OrbitProjector {
    raw_center: Vec2,
    scale: f32,
    origin: Pos2,
    right: Vec2,
    depth_axis: Vec2,
    pitch: f32,
    center: Point3,
    depth_center: f32,
    depth_scale: f32,
}

impl OrbitProjector {
    #[cfg(test)]
    fn from_model(model: &BuildingModel, drawing: Rect, view: View3dState) -> Option<Self> {
        let points = model_3d_points(model)?;
        Self::from_points(&points, drawing, view)
    }

    fn from_points(points: &[Point3], drawing: Rect, view: View3dState) -> Option<Self> {
        if points.is_empty() {
            return None;
        }
        let yaw = view.yaw;
        let pitch = view.pitch.clamp(-FRAC_PI_2, FRAC_PI_2);
        let right = Vec2::angled(yaw);
        let depth_axis = Vec2::new(-right.y, right.x);
        let center = model_3d_center(points);
        let radius = model_3d_radius(points, center).max(1.0);
        let diameter = radius * 2.0;
        let scale = drawing.width().min(drawing.height()) / diameter * 0.92 * view.zoom;

        Some(Self {
            raw_center: Vec2::ZERO,
            scale,
            origin: drawing.center(),
            right,
            depth_axis,
            pitch,
            center,
            depth_center: 0.0,
            depth_scale: 0.45 / diameter,
        })
    }

    #[cfg(test)]
    fn project(&self, point: Point2, elevation: Length) -> ProjectedPoint {
        let point = Point3::new(point.x, point.y, elevation);
        self.project_point(point)
    }

    fn project_point(&self, point: Point3) -> ProjectedPoint {
        let (raw, depth) = raw_orbit(point, self.center, self.right, self.depth_axis, self.pitch);
        ProjectedPoint {
            pos: Pos2::new(
                self.origin.x + (raw.x - self.raw_center.x) * self.scale,
                self.origin.y + (raw.y - self.raw_center.y) * self.scale,
            ),
            depth,
        }
    }

    fn view_direction(&self) -> Point3 {
        Point3::vector(
            self.depth_axis.x * self.pitch.cos(),
            self.depth_axis.y * self.pitch.cos(),
            self.pitch.sin(),
        )
    }
}

impl Point3 {
    const X: Self = Self {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    const Y: Self = Self {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    const Z: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    #[cfg(test)]
    fn new(point_x: Length, point_y: Length, elevation: Length) -> Self {
        Self {
            x: point_x.inches() as f32,
            y: point_y.inches() as f32,
            z: elevation.inches() as f32,
        }
    }

    fn vector(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn distance_squared(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }

    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn offset(self, axis: Self, amount: f32) -> Self {
        Self {
            x: self.x + axis.x * amount,
            y: self.y + axis.y * amount,
            z: self.z + axis.z * amount,
        }
    }
}

impl Neg for Point3 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
}

#[cfg(test)]
fn model_3d_points(model: &BuildingModel) -> Option<Vec<Point3>> {
    let mut points = Vec::new();
    for wall in &model.walls {
        points.push(Point3::new(wall.start.x, wall.start.y, Length::ZERO));
        points.push(Point3::new(wall.end.x, wall.end.y, Length::ZERO));
        points.push(Point3::new(wall.start.x, wall.start.y, wall.height));
        points.push(Point3::new(wall.end.x, wall.end.y, wall.height));
    }

    (!points.is_empty()).then_some(points)
}

fn model_3d_center(points: &[Point3]) -> Point3 {
    let mut min = Point3 {
        x: f32::MAX,
        y: f32::MAX,
        z: f32::MAX,
    };
    let mut max = Point3 {
        x: f32::MIN,
        y: f32::MIN,
        z: f32::MIN,
    };

    for point in points {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }

    Point3 {
        x: (min.x + max.x) / 2.0,
        y: (min.y + max.y) / 2.0,
        z: (min.z + max.z) / 2.0,
    }
}

fn model_3d_radius(points: &[Point3], center: Point3) -> f32 {
    points
        .iter()
        .map(|point| point.distance_squared(center))
        .fold(0.0, f32::max)
        .sqrt()
}

fn raw_orbit(
    point: Point3,
    center: Point3,
    right: Vec2,
    depth_axis: Vec2,
    pitch: f32,
) -> (Vec2, f32) {
    let x = point.x - center.x;
    let y = point.y - center.y;
    let z = point.z - center.z;
    let along_depth = x * depth_axis.x + y * depth_axis.y;
    let raw = Vec2::new(
        x * right.x + y * right.y,
        along_depth * pitch.sin() - z * pitch.cos(),
    );
    let depth = along_depth * pitch.cos() + z * pitch.sin();

    (raw, depth)
}

fn point_hits_projected_quad(point: Pos2, points: &[Pos2; 4]) -> bool {
    if polygon_area(points) <= 8.0 {
        distance_to_segment(point, points[0], points[1]) < 8.0
    } else {
        point_in_polygon(point, points)
    }
}

fn polygon_area(points: &[Pos2]) -> f32 {
    if points.len() < 3 {
        return 0.0;
    }

    let mut area = 0.0;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        area += current.x * next.y - next.x * current.y;
    }
    area.abs() * 0.5
}

fn point_in_polygon(point: Pos2, points: &[Pos2]) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let a = points[current];
        let b = points[previous];
        if ((a.y > point.y) != (b.y > point.y))
            && (point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x)
        {
            inside = !inside;
        }
        previous = current;
    }

    inside
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

fn pointer_started_in_rect(press_origin: Option<Pos2>, rect: Rect) -> bool {
    press_origin.is_some_and(|origin| rect.contains(origin))
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

fn distance_to_segment(point: Pos2, start: Pos2, end: Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + segment * t)
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
    let layout = WallElevationLayout::new(drawing, wall);
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

    painter.rect_filled(
        wall_rect,
        0.0,
        Color32::from_rgba_unmultiplied(188, 179, 158, 34),
    );
    draw_view_border(&painter, wall_rect);
    for opening in &wall.openings {
        let opening_rect = opening_rect(wall_rect, scale, scale, opening);
        let hovered = pointer.is_some_and(|position| opening_rect.contains(position));
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
    } else if response.drag_started_by(egui::PointerButton::Primary) {
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
        let should_place_dimension =
            response.clicked() || response.drag_stopped_by(egui::PointerButton::Primary);
        if let Some(position) = pointer {
            if let Some(placement) = placement {
                if should_place_dimension {
                    output.click = Some(DesignElevationClick::DimensionPlacement {
                        axis: placement.axis,
                        line_offset: placement.line_offset,
                    });
                }
            } else if response.clicked()
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

    output
}

#[derive(Clone, Copy)]
struct WallElevationLayout {
    wall_rect: Rect,
    scale: f32,
}

impl WallElevationLayout {
    fn new(available: Rect, wall: &Wall) -> Self {
        let wall_width = wall.length.inches().max(1.0) as f32;
        let wall_height = wall.height.inches().max(1.0) as f32;
        let scale = (available.width() / wall_width)
            .min(available.height() / wall_height)
            .max(0.001);
        let wall_size = Vec2::new(wall_width * scale, wall_height * scale);
        Self {
            wall_rect: Rect::from_center_size(available.center(), wall_size),
            scale,
        }
    }
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
) -> Option<String> {
    let available = ui.available_size();
    let desired = Vec2::new(available.x.max(420.0), (available.y - 16.0).max(420.0));
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let painter = ui.painter_at(rect);

    let margin = 52.0;
    let drawing = Rect::from_min_max(
        rect.min + Vec2::splat(margin),
        rect.max - Vec2::new(margin, margin),
    );
    let layout = WallElevationLayout::new(drawing, wall);
    let wall_rect = layout.wall_rect;
    let scale = layout.scale;

    painter.rect_filled(rect, 0.0, theme::sheet());
    draw_drafting_rulers(&painter, rect, drawing);
    draw_drafting_grid(&painter, drawing);
    draw_view_border(&painter, drawing);
    let pointer = response.interact_pointer_pos();
    let mut clicked = None;

    draw_opening_guides(&painter, wall_rect, scale, scale, wall);

    for member in members {
        let member_rect = member_rect(wall_rect, scale, scale, member);
        let hovered = pointer.is_some_and(|position| member_rect.contains(position));
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
        Selection::Member { .. } | Selection::Join(_) | Selection::Level(_) | Selection::Wall => {
            Some(wall.length / 2)
        }
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
    fn wall_elevation_layout_preserves_wall_aspect_ratio() {
        let model = BuildingModel::demo_wall();
        let wall = &model.walls[0];
        let available = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 1000.0));
        let layout = WallElevationLayout::new(available, wall);

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
