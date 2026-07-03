use eframe::egui::{
    self, Align2, ComboBox, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    Ui, Vec2,
};
use framer_core::{DimensionAxis, DimensionKind, Length, Point2, SystemKind};

use super::actions::{self, ActionId};
use super::draw_wall::SnapResult;
use super::labels::{dimension_axis_label, dimension_kind_label};
#[cfg(test)]
use super::model_edit::OpeningEditHandle;
use super::{FramerApp, Selection, ViewClick, ViewportMode, WorkspaceMode, design, theme};

mod camera_2d;
pub(super) use camera_2d::View2dState;

mod camera_3d;
pub(super) use camera_3d::View3dState;
#[cfg(test)]
use camera_3d::{ViewCubeAction, ViewCubeOrientation};
// Referenced only from the `tests` module below (their non-test users moved into
// camera_3d), so gate the imports to keep non-test builds warning-clean.
#[cfg(test)]
use camera_3d::{DOLLY_MAX, DOLLY_MIN, PAN_MAX_RADII};
#[cfg(test)]
use framer_core::BuildingModel;
#[cfg(test)]
use framer_core::{DimensionAnchor, DimensionHorizontalReference, DimensionVerticalReference};
#[cfg(test)]
use framer_render::math::Vec3;
#[cfg(test)]
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

mod geom;
// All non-test geom consumers now live in their own modules; only the tests below
// still reach into geom (OrbitProjector, Point3, Scene3d math, …).
#[cfg(test)]
use geom::*;

mod view_common;
use view_common::*;

mod gpu;

mod scene_build;
// scene_build items (Scene3d + math) are consumed by axonometric and the tests below.
#[cfg(test)]
use scene_build::*;

mod view_cube;
// view_cube items are consumed by axonometric (its own module) and the tests below.
#[cfg(test)]
use view_cube::*;

mod axonometric;
use axonometric::*;

// Adds an `impl FramerApp { draw_project_render }` block; no items to import.
mod render;

mod plan;
use plan::{PlanView, draw_project_plan};
// Re-exported to the parent `app` module (consumed by handle_wall_drag_event and
// history_integration_tests) — preserves the existing `viewport::WallDragEvent` path.
pub(super) use plan::WallDragEvent;

mod elevation_dimensions;
// Consumed by elevation_design (its own module) and the tests below.
#[cfg(test)]
use elevation_dimensions::*;

mod elevation_openings;
use elevation_openings::*;

mod elevation_framing;
use elevation_framing::*;

mod elevation_design;
use elevation_design::*;

/// Plan-view input for the draw-wall tool: whether it is active, the in-progress
/// run's start point, the active grid snap increment, and the snap held from the
/// previous frame (for sticky hysteresis).
pub(super) struct DrawWallPlanInput {
    pub(super) active: bool,
    pub(super) start: Option<Point2>,
    pub(super) snap_step: Option<Length>,
    pub(super) previous_snap: Option<SnapResult>,
}

impl FramerApp {
    pub(super) fn workspace(&mut self, ui: &mut Ui) {
        self.workspace_header(ui);
        if self.has_active_tool_options() {
            ui.add_space(4.0);
            self.tool_options_strip(ui);
        }
        ui.add_space(8.0);

        let canvas = Rect::from_min_size(ui.next_widget_position(), viewport_size(ui));
        self.cursor_model = None;
        let mut toolbar_anchor = None;
        // The draw tool's resolved snap for this frame, written back into tool
        // state so the next frame can apply sticky hysteresis.
        let mut snap_out: Option<SnapResult> = None;
        // The active wall-endpoint drag (state owned here) and the event the plan
        // emits for it this frame.
        let active_wall_drag = self.wall_drag.map(|drag| (drag.wall_index, drag.handle));
        let mut wall_drag_out: Option<WallDragEvent> = None;
        let click = match self.viewport_mode {
            ViewportMode::Plan | ViewportMode::RoofPlan => {
                let roof_plan_mode = self.viewport_mode == ViewportMode::RoofPlan;
                let draw_tool = DrawWallPlanInput {
                    active: self.draw_wall_tool.active,
                    start: self.draw_wall_tool.start,
                    snap_step: self.snap_step,
                    previous_snap: self.draw_wall_tool.previous_snap,
                };
                draw_project_plan(
                    ui,
                    PlanView {
                        model: &self.model,
                        selected_wall: self.selected_wall,
                        selection: &self.selected,
                        layers: self.layers,
                        draw_tool: &draw_tool,
                        room_tool_active: self.room_tool_active,
                        ceiling_tool_active: self.ceiling_tool_active,
                        vault_tool_active: self.vault_tool_active,
                        floor_tool_active: self.floor_tool_active,
                        roof_plan_mode,
                        active_wall_drag,
                    },
                    &mut self.plan_view,
                    &mut self.cursor_model,
                    &mut toolbar_anchor,
                    &mut snap_out,
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
                    let system = self.model.system_for(wall);
                    draw_wall_elevation(
                        ui,
                        wall,
                        &wall_plan.members,
                        selected_member,
                        section_x,
                        BuildUpContext {
                            system,
                            materials: &self.model.materials,
                        },
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
                        wall_display: self.layers.wall_display,
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

    fn workspace_header(&mut self, ui: &mut Ui) {
        let t = design::active();
        let code_name = self.model.code.display_name.clone();
        Frame::new()
            .fill(t.panel)
            .inner_margin(Margin::symmetric(6, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = design::space::LG;
                    self.workspace_switcher(ui);
                    self.viewport_tabs(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(code_name.as_str())
                                .size(design::text_size::LABEL)
                                .color(t.text_muted),
                        );
                    });
                });
            });
    }

    fn workspace_switcher(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = design::space::XS;
            if workspace_mode_tab(ui, WorkspaceMode::Design, self.workspace_mode).clicked() {
                self.set_workspace_mode(WorkspaceMode::Design);
            }
            if workspace_mode_tab(ui, WorkspaceMode::Plan, self.workspace_mode).clicked() {
                self.set_workspace_mode(WorkspaceMode::Plan);
            }
        });
    }

    fn viewport_tabs(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = design::space::XS;
            let design_mode = self.workspace_mode.allows_design_edits();
            let plan_label = if design_mode { "Shell" } else { "Plan" };
            let elevation_label = if design_mode { "Wall" } else { "Elevation" };
            if view_tab(
                ui,
                ActionId::ViewPlan,
                plan_label,
                self.viewport_mode == ViewportMode::Plan,
            )
            .clicked()
            {
                self.viewport_mode = ViewportMode::Plan;
            }
            if view_tab(
                ui,
                ActionId::ViewElevation,
                elevation_label,
                self.viewport_mode == ViewportMode::Elevation,
            )
            .clicked()
            {
                self.viewport_mode = ViewportMode::Elevation;
            }
            if view_tab(
                ui,
                ActionId::ViewRoof,
                "Roof",
                self.viewport_mode == ViewportMode::RoofPlan,
            )
            .clicked()
            {
                self.viewport_mode = ViewportMode::RoofPlan;
            }
            if view_tab(
                ui,
                ActionId::View3d,
                "3D",
                self.viewport_mode == ViewportMode::Axonometric,
            )
            .clicked()
            {
                self.viewport_mode = ViewportMode::Axonometric;
            }
            if view_tab(
                ui,
                ActionId::ViewRender,
                "Render",
                self.viewport_mode == ViewportMode::Render,
            )
            .clicked()
            {
                self.viewport_mode = ViewportMode::Render;
            }
        });
    }

    fn has_active_tool_options(&self) -> bool {
        self.draw_wall_tool.active
            || self.dimension_tool.active
            || self.room_tool_active
            || self.ceiling_tool_active
            || self.vault_tool_active
            || self.floor_tool_active
    }

    fn tool_options_strip(&mut self, ui: &mut Ui) {
        let t = design::active();
        Frame::new()
            .fill(t.toolbar)
            .stroke(t.soft_stroke())
            .corner_radius(design::radius::SM)
            .inner_margin(Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(design::space::SM, design::space::XS);
                    if self.draw_wall_tool.active {
                        self.wall_tool_options(ui);
                    } else if self.dimension_tool.active {
                        self.dimension_tool_options(ui);
                    } else if self.room_tool_active {
                        self.region_tool_options(ui, "Room options", "Room", "Unspecified");
                    } else if self.ceiling_tool_active {
                        self.surface_tool_options(ui, "Ceiling options", "Flat ceiling", "Flush");
                    } else if self.vault_tool_active {
                        self.surface_tool_options(ui, "Vault options", "Scissor vault", "4:12");
                    } else if self.floor_tool_active {
                        self.surface_tool_options(ui, "Floor options", "Floor deck", "Auto span");
                    }
                });
            });
    }

    fn wall_tool_options(&self, ui: &mut Ui) {
        let wall_system_name = self.default_wall_system_name();
        let wall_height = self.model.code.default_wall_height.to_string();
        let level_name = self.first_level_name();

        option_strip_title(ui, "Wall options");
        readonly_option(ui, "Type", wall_system_name.as_str());
        readonly_option(ui, "Baseline", "Centerline");
        readonly_option(ui, "Height", wall_height.as_str());
        readonly_option(ui, "Level", level_name.as_str());
        readonly_option(
            ui,
            "Placement",
            if self.draw_wall_tool.start.is_some() {
                "Next endpoint"
            } else {
                "First endpoint"
            },
        );
    }

    fn dimension_tool_options(&mut self, ui: &mut Ui) {
        option_strip_title(ui, "Dimension options");
        let kind_action = actions::metadata(ActionId::DimensionKind);
        labeled_option(ui, kind_action.label, |ui| {
            ComboBox::from_id_salt("context-dimension-tool-kind")
                .selected_text(dimension_kind_label(self.dimension_tool.kind))
                .width(96.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.dimension_tool.kind,
                        DimensionKind::Driving,
                        "Driving",
                    );
                    ui.selectable_value(
                        &mut self.dimension_tool.kind,
                        DimensionKind::Reference,
                        "Reference",
                    );
                })
                .response
                .on_hover_text(kind_action.tooltip);
        });

        let axis_action = actions::metadata(ActionId::DimensionAxis);
        labeled_option(ui, axis_action.label, |ui| {
            ComboBox::from_id_salt("context-dimension-tool-axis")
                .selected_text(dimension_axis_label(self.dimension_tool.axis))
                .width(104.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.dimension_tool.axis,
                        DimensionAxis::Horizontal,
                        "Horizontal",
                    );
                    ui.selectable_value(
                        &mut self.dimension_tool.axis,
                        DimensionAxis::Vertical,
                        "Vertical",
                    );
                })
                .response
                .on_hover_text(axis_action.tooltip);
        });
    }

    fn region_tool_options(&self, ui: &mut Ui, title: &str, kind: &str, usage: &str) {
        option_strip_title(ui, title);
        readonly_option(ui, "Type", kind);
        readonly_option(ui, "Usage", usage);
        readonly_option(ui, "Level", self.first_level_name().as_str());
        readonly_option(ui, "Placement", "Enclosed region");
    }

    fn surface_tool_options(&self, ui: &mut Ui, title: &str, kind: &str, setting: &str) {
        option_strip_title(ui, title);
        readonly_option(ui, "Type", kind);
        readonly_option(ui, "Setting", setting);
        readonly_option(ui, "Level", self.first_level_name().as_str());
        readonly_option(ui, "Placement", "Enclosed region");
    }

    fn default_wall_system_name(&self) -> String {
        self.model
            .systems
            .iter()
            .find(|system| {
                system.id.0 == "system-wall-exterior-1" && system.kind == SystemKind::Wall
            })
            .or_else(|| {
                self.model
                    .systems
                    .iter()
                    .find(|system| system.kind == SystemKind::Wall)
            })
            .map(|system| system.name.clone())
            .unwrap_or_else(|| "Wall system".to_owned())
    }

    fn first_level_name(&self) -> String {
        self.model
            .levels
            .first()
            .map(|level| level.name.clone())
            .unwrap_or_else(|| "Level 1".to_owned())
    }
}

fn workspace_mode_tab(
    ui: &mut Ui,
    mode: WorkspaceMode,
    active_mode: WorkspaceMode,
) -> egui::Response {
    let action_id = match mode {
        WorkspaceMode::Design => ActionId::WorkspaceDesign,
        WorkspaceMode::Plan => ActionId::WorkspacePlan,
    };
    let label = match mode {
        WorkspaceMode::Design => "Design Workspace",
        WorkspaceMode::Plan => "Plan Workspace",
    };
    let action = actions::metadata(action_id);
    design::widgets::tab(ui, label, mode == active_mode).on_hover_text(action.tooltip)
}

fn view_tab(ui: &mut Ui, action_id: ActionId, label: &str, selected: bool) -> egui::Response {
    let action = actions::metadata(action_id);
    design::widgets::tab(ui, label, selected).on_hover_text(action.tooltip)
}

fn option_strip_title(ui: &mut Ui, label: &str) {
    let t = design::active();
    ui.label(
        RichText::new(label)
            .strong()
            .size(design::text_size::LABEL)
            .color(t.text),
    );
    option_divider(ui);
}

fn labeled_option(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let t = design::active();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = design::space::XS;
        ui.label(
            RichText::new(label)
                .size(design::text_size::LABEL)
                .color(t.text_muted),
        );
        add(ui);
    });
}

fn readonly_option(ui: &mut Ui, label: &str, value: &str) {
    labeled_option(ui, label, |ui| {
        let t = design::active();
        Frame::new()
            .fill(t.control)
            .stroke(t.soft_stroke())
            .corner_radius(design::radius::SM)
            .inner_margin(Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(value)
                        .size(design::text_size::LABEL)
                        .color(t.text),
                );
            });
    });
}

fn option_divider(ui: &mut Ui) {
    let t = design::active();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 20.0), Sense::hover());
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        Stroke::new(1.0, t.divider),
    );
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
        let scene = Scene3d::from_project(
            &model,
            &plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Plan,
            crate::app::WallDisplay::Full,
        )
        .unwrap();
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
        let scene = Scene3d::from_project(
            &model,
            &plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Plan,
            crate::app::WallDisplay::Full,
        )
        .unwrap();
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
        let scene = Scene3d::from_project(
            &model,
            &plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Plan,
            crate::app::WallDisplay::Full,
        )
        .unwrap();
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));

        let base =
            OrbitProjector::from_points(&scene.points, drawing, View3dState::default()).unwrap();
        let panned_view = View3dState {
            pan: Vec3::new(0.3, -0.15, 0.05),
            ..Default::default()
        };
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
        let mut tele = View3dState {
            zoom: 2.0,
            ..Default::default()
        };
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
        let mut v = View3dState {
            pan: Vec3::new(2.0, -1.0, 0.5),
            dolly: 0.4,
            ..Default::default()
        };
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
        let scene = Scene3d::from_project(
            &model,
            &plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Plan,
            crate::app::WallDisplay::Full,
        )
        .unwrap();
        // The wall cross-section spans its full system thickness, centered on the
        // centerline so it reaches +/- total/2 on the side axis (no longer the
        // bare stud depth) regardless of which side topology marks as interior.
        // Every demo wall shares one system, so any wall gives the section thickness.
        let total = model
            .system_for(&model.walls[0])
            .expect("wall resolves a system")
            .total_thickness()
            .inches() as f32;
        // The full system is thicker than the framing layer alone, so layering
        // genuinely deepens the wall in the side axis.
        let stud_depth = model.code.stud_profile.nominal_depth().inches() as f32;
        assert!(total > stud_depth);

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
            min_y <= -total / 2.0,
            "front wall should have full system thickness in plan depth"
        );
    }

    #[test]
    fn scene_3d_contains_pickable_members_openings_and_walls() {
        let model = BuildingModel::demo_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let plan_scene = Scene3d::from_project(
            &model,
            &plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Plan,
            crate::app::WallDisplay::Full,
        )
        .unwrap();

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

        let design_scene = Scene3d::from_project(
            &model,
            &plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Design,
            crate::app::WallDisplay::Full,
        )
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
    fn scene_3d_width_mode_collapses_layers_to_a_single_band() {
        let model = BuildingModel::demo_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let full = Scene3d::from_project(
            &model,
            &plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Plan,
            crate::app::WallDisplay::Full,
        )
        .unwrap();
        let width = Scene3d::from_project(
            &model,
            &plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Plan,
            crate::app::WallDisplay::Width,
        )
        .unwrap();

        // Width still fills the wall (one monochrome band per wall, opening-cut),
        // so it has transparent geometry — but strictly less than the multi-layer
        // Full cross-section. It draws no outline edges.
        assert!(width.transparent_index_count > 0);
        assert!(
            width.transparent_index_count < full.transparent_index_count,
            "one band per wall must be lighter than the full layered section"
        );
        assert!(width.outline_edges.is_empty());
    }

    #[test]
    fn scene_3d_outline_mode_emits_edges_not_wall_bands() {
        let model = BuildingModel::demo_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let outline = Scene3d::from_project(
            &model,
            &plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Plan,
            crate::app::WallDisplay::Outline,
        )
        .unwrap();

        // No wall fill bands (the transparent pass is empty); walls are carried by
        // the envelope edges instead — 12 per wall — and their corners still feed
        // the orbit projector so the view can frame an all-outline scene.
        assert_eq!(outline.transparent_index_count, 0);
        assert_eq!(outline.outline_edges.len(), model.walls.len() * 12);
        assert!(!outline.points.is_empty());
    }

    #[test]
    fn scene_3d_outline_highlights_only_the_selected_walls_edges() {
        let model = BuildingModel::demo_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        // Select wall index 1 (not the default 0) so the test can't pass by accident.
        let selected_wall = 1;
        let outline = Scene3d::from_project(
            &model,
            &plan,
            selected_wall,
            &Selection::Wall,
            WorkspaceMode::Design,
            crate::app::WallDisplay::Outline,
        )
        .unwrap();

        // Exactly the selected wall's 12 envelope edges carry `selected` (the overlay
        // paints those blue); every other wall's edges stay unselected.
        let selected_edges = outline.outline_edges.iter().filter(|e| e.selected).count();
        assert_eq!(selected_edges, 12);
        assert_eq!(outline.outline_edges.len(), model.walls.len() * 12);
    }

    #[test]
    fn scene_3d_width_band_is_cut_by_openings() {
        // Width draws one neutral band per wall, but it must still be cut by openings.
        // Build the demo (which has doors/windows) and the same model with openings
        // stripped, both in Design workspace (no members, so only wall fill counts).
        let with_openings = BuildingModel::demo_shell();
        let mut without_openings = with_openings.clone();
        for wall in &mut without_openings.walls {
            wall.openings.clear();
        }
        let plan = framer_solver::generate_project_plan(&with_openings).unwrap();
        let plan_plain = framer_solver::generate_project_plan(&without_openings).unwrap();

        let build = |model, plan| {
            Scene3d::from_project(
                model,
                plan,
                0,
                &Selection::Wall,
                WorkspaceMode::Design,
                crate::app::WallDisplay::Width,
            )
            .unwrap()
            .transparent_index_count
        };
        let cut = build(&with_openings, &plan);
        let uncut = build(&without_openings, &plan_plain);

        // Cutting the band around each opening splits it into more segments, so the
        // cut scene has strictly more triangles than the single uncut band per wall —
        // proving openings are not ignored in Width mode.
        assert!(uncut > 0);
        assert!(
            cut > uncut,
            "openings should split the Width band into more segments ({cut} vs {uncut})"
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
