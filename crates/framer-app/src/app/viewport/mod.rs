use std::sync::Arc;

use eframe::egui::{
    self, Align2, ComboBox, CursorIcon, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, Vec2, containers::menu::MenuButton,
};
use framer_core::{DimensionAxis, DimensionKind, Length, Point2, SystemKind};

use super::WorkspaceMode;
use super::actions::{self, ActionId};
use super::context_menu;
use super::draw_wall::SnapResult;
use super::labels::{dimension_axis_label, dimension_kind_label};
#[cfg(test)]
use super::model_edit::OpeningEditHandle;
use super::{FramerApp, Selection, ViewportMode, design, theme};

mod camera_2d;
#[cfg(test)]
pub(super) use camera_2d::View2dState;

mod camera_3d;
#[cfg(test)]
pub(super) use camera_3d::View3dState;
#[cfg(test)]
use camera_3d::{ViewCubeAction, ViewCubeOrientation};
// Referenced only from the `tests` module below (their non-test users moved into
// camera_3d), so gate the imports to keep non-test builds warning-clean.
#[cfg(test)]
use camera_3d::{DOLLY_MAX, DOLLY_MIN, PAN_MAX_RADII, ZOOM_MAX_3D};
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
#[cfg(test)]
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

mod render;

mod pane;
pub(super) use pane::ViewportPaneRuntime;

mod pane_view;
use pane_view::{
    OwnedPaneFrame, PANE_PRESENTATION_ACTIONS, PaneCanvasEvents, PaneCanvasOutput, PaneFrame,
    PaneGpuInput, PaneInteractionPolicy, PanePresentationAction, PaneToolInput, draw_pane_canvas,
};

mod deferred;
use deferred::DeferredPaneEvent;

mod layout;
#[cfg(test)]
pub(super) use layout::PaneIdGenerator;
pub(super) use layout::{BuiltInPreset, LayoutNode, PaneId, SplitAxis, SplitSide};

mod workspace_state;
pub(super) use workspace_state::{PaneRuntimeHandle, ViewportWorkspaceState};

mod plan;
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

mod elevation_design;

/// Plan-view input for the draw-wall tool: whether it is active, the in-progress
/// run's start point, the active grid snap increment, and the snap held from the
/// previous frame (for sticky hysteresis).
pub(super) struct DrawWallPlanInput {
    pub(super) active: bool,
    pub(super) start: Option<Point2>,
    pub(super) snap_step: Option<Length>,
    pub(super) previous_snap: Option<SnapResult>,
}

const PANE_HEADER_HEIGHT: f32 = 30.0;
const SPLITTER_HIT_WIDTH: f32 = 7.0;

#[derive(Clone)]
struct DockedPaneRect {
    id: PaneId,
    rect: Rect,
}

#[derive(Clone)]
struct DockedSplitter {
    path: Vec<SplitSide>,
    axis: SplitAxis,
    ratio: f32,
    bounds: Rect,
    rect: Rect,
}

enum PaneUiCommand {
    Activate(PaneId),
    SetMode(PaneId, ViewportMode),
    Split(PaneId, SplitAxis),
    Duplicate(PaneId, SplitAxis),
    PopOut(PaneId),
    Remove(PaneId),
    SetRatio(Vec<SplitSide>, f32),
}

struct DockedPaneOutput {
    id: PaneId,
    mode: ViewportMode,
    canvas: Rect,
    output: PaneCanvasOutput,
}

fn node_has_docked_pane(node: &LayoutNode) -> bool {
    match node {
        LayoutNode::Pane(pane) => !pane.config().is_popped_out(),
        LayoutNode::Split { first, second, .. } => {
            node_has_docked_pane(first) || node_has_docked_pane(second)
        }
    }
}

fn collect_docked_layout(
    node: &LayoutNode,
    bounds: Rect,
    path: &mut Vec<SplitSide>,
    panes: &mut Vec<DockedPaneRect>,
    splitters: &mut Vec<DockedSplitter>,
) {
    match node {
        LayoutNode::Pane(pane) => {
            if !pane.config().is_popped_out() {
                panes.push(DockedPaneRect {
                    id: pane.id(),
                    rect: bounds,
                });
            }
        }
        LayoutNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let first_visible = node_has_docked_pane(first);
            let second_visible = node_has_docked_pane(second);
            match (first_visible, second_visible) {
                (true, true) => {
                    let (first_bounds, second_bounds, splitter_rect) = match axis {
                        SplitAxis::Horizontal => {
                            let split_x = bounds.left() + bounds.width() * *ratio;
                            let half_gap = SPLITTER_HIT_WIDTH * 0.5;
                            (
                                Rect::from_min_max(
                                    bounds.min,
                                    Pos2::new(
                                        (split_x - half_gap).max(bounds.left()),
                                        bounds.bottom(),
                                    ),
                                ),
                                Rect::from_min_max(
                                    Pos2::new(
                                        (split_x + half_gap).min(bounds.right()),
                                        bounds.top(),
                                    ),
                                    bounds.max,
                                ),
                                Rect::from_min_max(
                                    Pos2::new(split_x - half_gap, bounds.top()),
                                    Pos2::new(split_x + half_gap, bounds.bottom()),
                                ),
                            )
                        }
                        SplitAxis::Vertical => {
                            let split_y = bounds.top() + bounds.height() * *ratio;
                            let half_gap = SPLITTER_HIT_WIDTH * 0.5;
                            (
                                Rect::from_min_max(
                                    bounds.min,
                                    Pos2::new(
                                        bounds.right(),
                                        (split_y - half_gap).max(bounds.top()),
                                    ),
                                ),
                                Rect::from_min_max(
                                    Pos2::new(
                                        bounds.left(),
                                        (split_y + half_gap).min(bounds.bottom()),
                                    ),
                                    bounds.max,
                                ),
                                Rect::from_min_max(
                                    Pos2::new(bounds.left(), split_y - half_gap),
                                    Pos2::new(bounds.right(), split_y + half_gap),
                                ),
                            )
                        }
                    };
                    splitters.push(DockedSplitter {
                        path: path.clone(),
                        axis: *axis,
                        ratio: *ratio,
                        bounds,
                        rect: splitter_rect,
                    });
                    path.push(SplitSide::First);
                    collect_docked_layout(first, first_bounds, path, panes, splitters);
                    path.pop();
                    path.push(SplitSide::Second);
                    collect_docked_layout(second, second_bounds, path, panes, splitters);
                    path.pop();
                }
                (true, false) => {
                    path.push(SplitSide::First);
                    collect_docked_layout(first, bounds, path, panes, splitters);
                    path.pop();
                }
                (false, true) => {
                    path.push(SplitSide::Second);
                    collect_docked_layout(second, bounds, path, panes, splitters);
                    path.pop();
                }
                (false, false) => {}
            }
        }
    }
}

fn viewport_mode_label(mode: ViewportMode) -> &'static str {
    match mode {
        ViewportMode::Plan => "Plan",
        ViewportMode::RoofPlan => "Roof",
        ViewportMode::Elevation => "Elevation",
        ViewportMode::Axonometric => "3D",
        ViewportMode::Render => "Render",
    }
}

impl FramerApp {
    pub(super) fn workspace(&mut self, ui: &mut Ui) {
        self.drain_deferred_pane_events();
        // Keep legacy command/action surfaces wired to the active leaf while
        // the layout remains the authoritative per-pane mode store.
        if self.viewport_workspace.active_mode() != self.viewport_mode {
            self.viewport_workspace.set_active_mode(self.viewport_mode);
        }

        self.workspace_header(ui);
        if self.viewport_workspace.active_mode() != self.viewport_mode {
            self.viewport_workspace.set_active_mode(self.viewport_mode);
        }
        if self.has_active_tool_options() {
            ui.add_space(4.0);
            self.tool_options_strip(ui);
        }
        ui.add_space(6.0);

        let bounds = ui.available_rect_before_wrap();
        ui.allocate_rect(bounds, Sense::hover());
        let root = self.viewport_workspace.layout.root().clone();
        let mut panes = Vec::new();
        let mut splitters = Vec::new();
        collect_docked_layout(&root, bounds, &mut Vec::new(), &mut panes, &mut splitters);

        let t = design::active();
        let mut commands = Vec::new();
        for splitter in splitters {
            let id = ui.id().with(("viewport-splitter", &splitter.path));
            let response = ui.interact(splitter.rect, id, Sense::drag());
            let cursor = match splitter.axis {
                SplitAxis::Horizontal => CursorIcon::ResizeHorizontal,
                SplitAxis::Vertical => CursorIcon::ResizeVertical,
            };
            let response = response.on_hover_cursor(cursor);
            let divider_label = match splitter.axis {
                SplitAxis::Horizontal => "Horizontal viewport split divider",
                SplitAxis::Vertical => "Vertical viewport split divider",
            };
            response.widget_info(|| {
                egui::WidgetInfo::slider(true, f64::from(splitter.ratio), divider_label)
            });
            let center = splitter.rect.center();
            let line = match splitter.axis {
                SplitAxis::Horizontal => [
                    Pos2::new(center.x, splitter.bounds.top()),
                    Pos2::new(center.x, splitter.bounds.bottom()),
                ],
                SplitAxis::Vertical => [
                    Pos2::new(splitter.bounds.left(), center.y),
                    Pos2::new(splitter.bounds.right(), center.y),
                ],
            };
            ui.painter().line_segment(
                line,
                Stroke::new(
                    if response.hovered() || response.dragged() {
                        2.0
                    } else {
                        1.0
                    },
                    if response.hovered() || response.dragged() {
                        t.accent
                    } else {
                        t.divider
                    },
                ),
            );
            if response.dragged()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let ratio = match splitter.axis {
                    SplitAxis::Horizontal => {
                        (pointer.x - splitter.bounds.left()) / splitter.bounds.width().max(1.0)
                    }
                    SplitAxis::Vertical => {
                        (pointer.y - splitter.bounds.top()) / splitter.bounds.height().max(1.0)
                    }
                };
                commands.push(PaneUiCommand::SetRatio(splitter.path, ratio));
            } else if response.has_focus() {
                let keyboard_delta = ui.input(|input| match splitter.axis {
                    SplitAxis::Horizontal => {
                        if input.key_pressed(egui::Key::ArrowLeft) {
                            -0.05
                        } else if input.key_pressed(egui::Key::ArrowRight) {
                            0.05
                        } else {
                            0.0
                        }
                    }
                    SplitAxis::Vertical => {
                        if input.key_pressed(egui::Key::ArrowUp) {
                            -0.05
                        } else if input.key_pressed(egui::Key::ArrowDown) {
                            0.05
                        } else {
                            0.0
                        }
                    }
                });
                if keyboard_delta != 0.0 {
                    commands.push(PaneUiCommand::SetRatio(
                        splitter.path,
                        splitter.ratio + keyboard_delta,
                    ));
                }
            }
        }

        let mut outputs = Vec::new();
        if panes.is_empty() {
            ui.painter().text(
                bounds.center(),
                Align2::CENTER_CENTER,
                "All viewports are popped out",
                FontId::proportional(design::text_size::HEADING),
                t.text_muted,
            );
        } else {
            for pane in panes {
                if let Some(output) = self.draw_docked_pane(ui, pane, &mut commands) {
                    outputs.push(output);
                }
            }
        }

        for output in outputs {
            self.apply_docked_pane_output(ui, output);
        }

        for command in commands {
            self.apply_pane_ui_command(command);
        }
        for id in self.viewport_workspace.take_retired_targets() {
            gpu::release_target(ui.painter(), id.get());
            crate::app::render::release_target(ui.painter(), id.get());
        }
        if self.viewport_workspace.has_deferred_panes() {
            let snapshots = self
                .viewport_workspace
                .deferred_pane_modes()
                .into_iter()
                .map(|(id, mode)| (id, self.deferred_pane_snapshot(mode)))
                .collect::<Vec<_>>();
            self.viewport_workspace.show_deferred(ui.ctx(), &snapshots);
        }
        self.viewport_preset_dialog(ui.ctx());
    }

    fn drain_deferred_pane_events(&mut self) {
        for event in self.viewport_workspace.drain_deferred_events() {
            match event {
                DeferredPaneEvent::Canvas(events) => {
                    let id = self
                        .viewport_workspace
                        .layout
                        .pane_ids()
                        .into_iter()
                        .find(|id| id.get() == events.target_id);
                    if let Some(id) = id {
                        self.apply_pane_canvas_events(id, *events);
                    }
                }
                DeferredPaneEvent::Activate(id) => {
                    self.apply_pane_ui_command(PaneUiCommand::Activate(id));
                }
                DeferredPaneEvent::Dock(id) => {
                    if let Err(error) = self.viewport_workspace.set_popped_out(id, false) {
                        self.file_status = Some(error.to_string());
                    }
                }
                DeferredPaneEvent::SetMode { pane_id, mode } => {
                    self.apply_pane_ui_command(PaneUiCommand::SetMode(pane_id, mode));
                }
                DeferredPaneEvent::Action { pane_id, action } => {
                    self.apply_pane_ui_command(PaneUiCommand::Activate(pane_id));
                    if self.action_enabled(action) {
                        self.execute_action(action);
                    } else if let Some(reason) = self.action_disabled_reason(action) {
                        self.file_status = Some(reason.to_owned());
                    }
                }
            }
        }
    }

    fn deferred_pane_snapshot(&self, viewport_mode: ViewportMode) -> Arc<OwnedPaneFrame> {
        let selected_components = self.selected_components();
        let frame = PaneFrame {
            model: &self.model,
            plan: self.project_plan.as_ref(),
            physical_scene: self.physical_scene.as_ref(),
            active_geometry_violation: self.active_geometry_violation.as_ref(),
            selected_wall: self.selected_wall,
            selection: &self.selected,
            selected_components: &selected_components,
            component_visibility: &self.component_visibility,
            workspace_mode: self.workspace_mode,
            layers: self.layers,
            show_section: self.show_section,
            render_settings: self.render_settings,
            tools: PaneToolInput::disabled(),
            gpu: PaneGpuInput {
                target_format: self.gpu_target_format,
                depth_format: self.gpu_depth_format,
                compute_ok: self.gpu_compute_ok,
                ray_query_ok: self.gpu_ray_query_ok,
                ray_query_enabled: self.config.render.ray_query,
            },
        };
        let snapshot = OwnedPaneFrame::from_frame(&frame);
        let actions = PANE_PRESENTATION_ACTIONS
            .into_iter()
            .map(|action| PanePresentationAction {
                action,
                enabled: self.action_enabled_for_viewport(action, viewport_mode),
                disabled_reason: self
                    .action_disabled_reason_for_viewport(action, viewport_mode)
                    .map(str::to_owned),
            })
            .collect();
        Arc::new(snapshot.with_presentation_actions(actions))
    }

    fn draw_docked_pane(
        &mut self,
        ui: &mut Ui,
        pane: DockedPaneRect,
        commands: &mut Vec<PaneUiCommand>,
    ) -> Option<DockedPaneOutput> {
        let id = pane.id;
        let active = self.viewport_workspace.active_id() == id;
        let mode = self.viewport_workspace.layout.pane(id)?.config().mode();
        let t = design::active();

        if ui.ctx().input(|input| {
            input
                .pointer
                .press_origin()
                .is_some_and(|position| pane.rect.contains(position))
        }) {
            commands.push(PaneUiCommand::Activate(id));
        }

        ui.painter().rect_filled(pane.rect, 2.0, t.canvas);
        ui.painter().rect_stroke(
            pane.rect.shrink(0.5),
            2.0,
            if active {
                Stroke::new(2.0, t.accent)
            } else {
                t.border_stroke()
            },
            StrokeKind::Inside,
        );

        let header = Rect::from_min_max(
            pane.rect.min,
            Pos2::new(
                pane.rect.right(),
                (pane.rect.top() + PANE_HEADER_HEIGHT).min(pane.rect.bottom()),
            ),
        );
        ui.painter().rect_filled(
            header,
            2.0,
            if active {
                t.accent_soft
            } else {
                t.panel_header
            },
        );
        ui.painter().line_segment(
            [header.left_bottom(), header.right_bottom()],
            t.soft_stroke(),
        );

        let mut header_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(("viewport-pane-header", id.get()))
                .max_rect(header.shrink2(Vec2::new(5.0, 3.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        header_ui.set_clip_rect(header);
        header_ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let show_identity = header.width() >= 170.0;
            if show_identity {
                ui.label(
                    RichText::new(format!("View {}", id.get()))
                        .strong()
                        .size(design::text_size::LABEL)
                        .color(if active { t.text } else { t.text_secondary }),
                );
            }
            let mut selected_mode = mode;
            let mode_width = if show_identity {
                72.0
            } else {
                (header.width() - 42.0).clamp(24.0, 72.0)
            };
            let mode_combo = ComboBox::from_id_salt(("viewport-pane-mode", id.get()))
                .selected_text(viewport_mode_label(mode))
                .width(mode_width)
                .show_ui(ui, |ui| {
                    for candidate in [
                        ViewportMode::Plan,
                        ViewportMode::RoofPlan,
                        ViewportMode::Elevation,
                        ViewportMode::Axonometric,
                        ViewportMode::Render,
                    ] {
                        ui.selectable_value(
                            &mut selected_mode,
                            candidate,
                            viewport_mode_label(candidate),
                        );
                    }
                });
            mode_combo.response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::ComboBox,
                    true,
                    format!("View {} mode", id.get()),
                )
            });
            if selected_mode != mode {
                commands.push(PaneUiCommand::SetMode(id, selected_mode));
            }
            let (actions, _) = MenuButton::new("•••").ui(ui, |ui| {
                if ui.button("Split left / right").clicked() {
                    commands.push(PaneUiCommand::Split(id, SplitAxis::Horizontal));
                    ui.close();
                }
                if ui.button("Split top / bottom").clicked() {
                    commands.push(PaneUiCommand::Split(id, SplitAxis::Vertical));
                    ui.close();
                }
                if ui.button("Duplicate viewport").clicked() {
                    commands.push(PaneUiCommand::Duplicate(id, SplitAxis::Horizontal));
                    ui.close();
                }
                if ui.button("Pop out viewport").clicked() {
                    commands.push(PaneUiCommand::PopOut(id));
                    ui.close();
                }
                ui.separator();
                if ui.button("Close viewport").clicked() {
                    commands.push(PaneUiCommand::Remove(id));
                    ui.close();
                }
            });
            actions.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("View {} viewport actions", id.get()),
                )
            });
            actions.on_hover_text("Split, duplicate, pop out, or close this viewport");
        });

        let canvas = Rect::from_min_max(
            Pos2::new(
                pane.rect.left() + 1.0,
                (header.bottom() + 1.0).min(pane.rect.bottom()),
            ),
            Pos2::new(pane.rect.right() - 1.0, pane.rect.bottom() - 1.0),
        );
        if canvas.width() < 2.0 || canvas.height() < 2.0 {
            return None;
        }

        let selected_components = self.selected_components();
        let first_dimension_anchor = self
            .dimension_tool
            .first_anchor
            .as_ref()
            .filter(|pick| pick.wall_index == self.selected_wall)
            .map(|pick| &pick.anchor);
        let second_dimension_anchor = self
            .dimension_tool
            .second_anchor
            .as_ref()
            .filter(|pick| pick.wall_index == self.selected_wall)
            .map(|pick| &pick.anchor);
        let active_wall_drag = self.wall_drag.map(|drag| (drag.wall_index, drag.handle));
        let frame = PaneFrame {
            model: &self.model,
            plan: self.project_plan.as_ref(),
            physical_scene: self.physical_scene.as_ref(),
            active_geometry_violation: self.active_geometry_violation.as_ref(),
            selected_wall: self.selected_wall,
            selection: &self.selected,
            selected_components: &selected_components,
            component_visibility: &self.component_visibility,
            workspace_mode: self.workspace_mode,
            layers: self.layers,
            show_section: self.show_section,
            render_settings: self.render_settings,
            tools: PaneToolInput {
                draw_wall_active: self.draw_wall_tool.active,
                draw_wall_start: self.draw_wall_tool.start,
                snap_step: self.snap_step,
                room_tool_active: self.room_tool_active,
                ceiling_tool_active: self.ceiling_tool_active,
                vault_tool_active: self.vault_tool_active,
                floor_tool_active: self.floor_tool_active,
                dimension_tool_active: self.dimension_tool.active,
                dimension_tool_axis: self.dimension_tool.axis,
                first_dimension_anchor,
                second_dimension_anchor,
                active_opening_drag: self.opening_drag.as_ref(),
                active_wall_drag,
            },
            gpu: PaneGpuInput {
                target_format: self.gpu_target_format,
                depth_format: self.gpu_depth_format,
                compute_ok: self.gpu_compute_ok,
                ray_query_ok: self.gpu_ray_query_ok,
                ray_query_enabled: self.config.render.ray_query,
            },
        };
        let runtime = self.viewport_workspace.runtime(id)?;
        let mut runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut canvas_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(("viewport-pane-canvas", id.get()))
                .max_rect(canvas)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        canvas_ui.set_clip_rect(canvas);
        let output = draw_pane_canvas(
            &mut canvas_ui,
            id.get(),
            mode,
            frame,
            if active {
                PaneInteractionPolicy::DOCKED
            } else {
                PaneInteractionPolicy::DEFERRED
            },
            &mut runtime,
        );
        drop(runtime);

        Some(DockedPaneOutput {
            id,
            mode,
            canvas,
            output,
        })
    }

    fn apply_docked_pane_output(&mut self, ui: &mut Ui, output: DockedPaneOutput) {
        let DockedPaneOutput {
            id,
            mode,
            canvas,
            output,
        } = output;
        let PaneCanvasOutput {
            events,
            axonometric_response,
        } = output;
        let toolbar_anchor = events.toolbar_anchor;
        let secondary_click = events.secondary_click.clone();
        self.apply_pane_canvas_events(id, events);

        if let Some(response) = axonometric_response {
            if response.secondary_clicked() {
                // Context composition reads the active view mode. Activate the
                // source pane now (before the queued generic pointer activation)
                // so an inactive 3D pane cannot inherit another pane's mode.
                self.apply_pane_ui_command(PaneUiCommand::Activate(id));
                self.prepare_viewport_context_menu(secondary_click);
            }
            let model = self
                .context_menu_context
                .as_ref()
                .map(context_menu::build_context_menu)
                .filter(|model| !model.is_empty());
            let mut chosen = None;
            response.context_menu(|ui| {
                let Some(model) = model.as_ref().filter(|_| !self.command_search.open) else {
                    ui.close();
                    return;
                };
                chosen = context_menu::render_context_menu(ui, model, |action| {
                    context_menu::ContextActionState {
                        enabled: self.action_enabled(action),
                        disabled_reason: self.action_disabled_reason(action),
                    }
                });
                if chosen.is_some() {
                    ui.close();
                }
            });
            if let Some(action) = chosen {
                self.execute_action(action);
                self.context_menu_context = None;
            } else if !response.context_menu_opened() {
                self.context_menu_context = None;
            }
        }

        if self.viewport_workspace.active_id() == id {
            if !matches!(mode, ViewportMode::Axonometric | ViewportMode::Render) {
                self.canvas_view_controls(ui, canvas, id);
            }
            if let Some(anchor) = toolbar_anchor
                && !self.command_search.open
            {
                self.canvas_context_toolbar(ui, anchor, canvas, id);
            }
            self.status_toast_overlay(ui, canvas.left_top() + Vec2::new(12.0, 12.0));
        }
    }

    fn apply_pane_canvas_events(&mut self, id: PaneId, events: PaneCanvasEvents) {
        debug_assert_eq!(events.target_id, id.get());
        if self.viewport_workspace.active_id() == id {
            self.cursor_model = events.cursor_model;
            self.draw_wall_tool.previous_snap = events.snap;
        }
        if let Some(opening_drag) = events.opening_drag {
            self.handle_opening_drag_event(opening_drag.wall_index, opening_drag.event);
        }
        if let Some(wall_drag) = events.wall_drag {
            self.handle_wall_drag_event(wall_drag);
        }
        if let Some(click) = events.primary_click {
            self.context_menu_context = None;
            self.handle_view_click_with_op(click, events.selection_op);
        }
    }

    fn apply_pane_ui_command(&mut self, command: PaneUiCommand) {
        let result = match command {
            PaneUiCommand::Activate(id) => {
                let result = self.viewport_workspace.set_active(id);
                if result.is_ok() {
                    self.viewport_mode = self.viewport_workspace.active_mode();
                    if self.viewport_mode != ViewportMode::Render {
                        self.last_authoring_viewport = self.viewport_mode;
                        self.last_authoring_pane = Some(id);
                    }
                }
                result
            }
            PaneUiCommand::SetMode(id, mode) => {
                let result = self.viewport_workspace.set_mode(id, mode);
                if result.is_ok() {
                    let _ = self.viewport_workspace.set_active(id);
                    self.viewport_mode = mode;
                    if mode != ViewportMode::Render {
                        self.last_authoring_viewport = mode;
                        self.last_authoring_pane = Some(id);
                    }
                }
                result
            }
            PaneUiCommand::Split(id, axis) => {
                self.viewport_workspace.split(id, axis).map(|new_id| {
                    self.viewport_mode = self
                        .viewport_workspace
                        .layout
                        .pane(new_id)
                        .expect("new pane exists")
                        .config()
                        .mode();
                })
            }
            PaneUiCommand::Duplicate(id, axis) => {
                self.viewport_workspace.duplicate(id, axis).map(|new_id| {
                    self.viewport_mode = self
                        .viewport_workspace
                        .layout
                        .pane(new_id)
                        .expect("duplicated pane exists")
                        .config()
                        .mode();
                })
            }
            PaneUiCommand::PopOut(id) => self.viewport_workspace.set_popped_out(id, true),
            PaneUiCommand::Remove(id) => {
                let result = self.viewport_workspace.remove(id);
                if result.is_ok() {
                    // Removing the active leaf may select a sibling with a
                    // different view type. Keep the legacy command-surface
                    // mirror in sync so the next frame does not overwrite the
                    // surviving pane's configuration.
                    self.viewport_mode = self.viewport_workspace.active_mode();
                    if self.viewport_mode != ViewportMode::Render {
                        self.last_authoring_viewport = self.viewport_mode;
                        self.last_authoring_pane = Some(self.viewport_workspace.active_id());
                    }
                }
                result
            }
            PaneUiCommand::SetRatio(path, ratio) => {
                self.viewport_workspace.layout.set_split_ratio(&path, ratio)
            }
        };
        if let Err(error) = result {
            self.file_status = Some(error.to_string());
        }
    }

    fn canvas_view_controls(&mut self, ui: &mut Ui, canvas: Rect, pane_id: PaneId) {
        let t = design::active();

        egui::Area::new(egui::Id::new(("canvas-nav-cube", pane_id.get())))
            .fixed_pos(Pos2::new(canvas.right() - 64.0, canvas.bottom() - 118.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::click());
                draw_nav_cube(ui.painter(), rect, t);
                let response = response.on_hover_text("View from the top — click for 3D");
                if response.clicked() {
                    self.apply_pane_ui_command(PaneUiCommand::SetMode(
                        pane_id,
                        ViewportMode::Axonometric,
                    ));
                }
            });
    }

    fn canvas_context_toolbar(&mut self, ui: &mut Ui, anchor: Pos2, canvas: Rect, pane_id: PaneId) {
        let duplicate_opening = self.can_duplicate_selected_opening();
        let delete_selection = self.action_enabled(ActionId::DeleteSelection);
        let presentation_actions = !self.renderable_selected_components().is_empty()
            || self.component_visibility.isolation_mode().is_some()
            || self.component_visibility.has_hidden();
        if !duplicate_opening && !delete_selection && !presentation_actions {
            return;
        }

        let t = design::active();
        let action_count = usize::from(duplicate_opening)
            + usize::from(delete_selection)
            + usize::from(presentation_actions);
        let spacing = 2.0;
        let width = action_count as f32 * design::control::ICON_BTN
            + action_count.saturating_sub(1) as f32 * spacing
            + 8.0;
        let height = design::control::ICON_BTN + 6.0;
        let inset = 8.0;
        let min_x = canvas.left() + inset;
        let max_x = canvas.right() - width - inset;
        let min_y = canvas.top() + inset;
        let max_y = canvas.bottom() - height - inset;
        let x = (anchor.x - width / 2.0).clamp(min_x, max_x.max(min_x));
        let above_y = anchor.y - height - 14.0;
        let preferred_y = if above_y >= min_y {
            above_y
        } else {
            anchor.y + 14.0
        };
        let y = preferred_y.clamp(min_y, max_y.max(min_y));
        egui::Area::new(egui::Id::new(("canvas-context-toolbar", pane_id.get())))
            .fixed_pos(Pos2::new(x, y))
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
                            if duplicate_opening {
                                let response = design::widgets::icon_button(
                                    ui,
                                    design::Icon::Duplicate,
                                    "Duplicate opening",
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        true,
                                        "Duplicate opening",
                                    )
                                });
                                if response.clicked() {
                                    self.duplicate_selected_opening();
                                }
                            }
                            if delete_selection {
                                let action = actions::metadata(ActionId::DeleteSelection);
                                let response =
                                    design::widgets::icon_button(ui, action.icon, action.tooltip);
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        true,
                                        action.label,
                                    )
                                });
                                if response.clicked() {
                                    self.execute_action(ActionId::DeleteSelection);
                                }
                            }
                            if presentation_actions {
                                let (response, _) = MenuButton::new(
                                    design::icon_text(design::Icon::Eye, 14.0)
                                        .color(t.text_secondary),
                                )
                                .ui(ui, |ui| {
                                    ui.set_min_width(184.0);
                                    for id in [
                                        ActionId::IsolateDim,
                                        ActionId::IsolateHide,
                                        ActionId::ExitIsolation,
                                    ] {
                                        let action = actions::metadata(id);
                                        let enabled = self.action_enabled(id);
                                        let button = ui
                                            .add_enabled(enabled, egui::Button::new(action.label));
                                        let button = if enabled {
                                            button.on_hover_text(action.tooltip)
                                        } else {
                                            button.on_disabled_hover_text(
                                                self.action_disabled_reason(id)
                                                    .unwrap_or(action.tooltip),
                                            )
                                        };
                                        if button.clicked() {
                                            self.execute_action(id);
                                            ui.close();
                                        }
                                    }
                                    ui.separator();
                                    for id in [ActionId::HideSelection, ActionId::ShowAllComponents]
                                    {
                                        let action = actions::metadata(id);
                                        let enabled = self.action_enabled(id);
                                        let button = ui
                                            .add_enabled(enabled, egui::Button::new(action.label));
                                        let button = if enabled {
                                            button.on_hover_text(action.tooltip)
                                        } else {
                                            button.on_disabled_hover_text(
                                                self.action_disabled_reason(id)
                                                    .unwrap_or(action.tooltip),
                                            )
                                        };
                                        if button.clicked() {
                                            self.execute_action(id);
                                            ui.close();
                                        }
                                    }
                                });
                                let enabled = response.enabled();
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        enabled,
                                        "Component visibility",
                                    )
                                });
                                response.on_hover_text("Component visibility and isolation");
                            }
                        });
                    });
            });
    }

    fn can_duplicate_selected_opening(&self) -> bool {
        self.workspace_mode.allows_design_edits()
            && self.selected_component_count() == 1
            && matches!(self.selected, Selection::Opening(_))
    }

    fn workspace_header(&mut self, ui: &mut Ui) {
        let t = design::active();
        let standards_name = self
            .model
            .base_standards_name()
            .unwrap_or("Standards starter pack")
            .to_owned();
        Frame::new()
            .fill(t.panel)
            .inner_margin(Margin::symmetric(6, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = design::space::SM;
                    self.viewport_tabs(ui);
                    self.viewport_layout_menu(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(standards_name.as_str())
                                .size(design::text_size::LABEL)
                                .color(t.text_muted),
                        );
                    });
                });
            });
    }

    fn viewport_tabs(&mut self, ui: &mut Ui) {
        if self.workspace_mode == WorkspaceMode::Render {
            return;
        }

        let design_mode = self.workspace_mode.allows_design_edits();
        let plan_label = if design_mode { "Shell" } else { "Plan" };
        let elevation_label = if design_mode { "Wall" } else { "Elevation" };
        let segments = [
            design::widgets::Segment {
                label: plan_label,
                selected: self.viewport_mode == ViewportMode::Plan,
                tooltip: actions::metadata(ActionId::ViewPlan).tooltip,
            },
            design::widgets::Segment {
                label: elevation_label,
                selected: self.viewport_mode == ViewportMode::Elevation,
                tooltip: actions::metadata(ActionId::ViewElevation).tooltip,
            },
            design::widgets::Segment {
                label: "Roof",
                selected: self.viewport_mode == ViewportMode::RoofPlan,
                tooltip: actions::metadata(ActionId::ViewRoof).tooltip,
            },
            design::widgets::Segment {
                label: "3D",
                selected: self.viewport_mode == ViewportMode::Axonometric,
                tooltip: actions::metadata(ActionId::View3d).tooltip,
            },
        ];
        if let Some(index) = design::widgets::segmented(ui, &segments) {
            let mode = match index {
                0 => ViewportMode::Plan,
                1 => ViewportMode::Elevation,
                2 => ViewportMode::RoofPlan,
                3 => ViewportMode::Axonometric,
                _ => unreachable!("view segment index is bounded by the segment array"),
            };
            self.set_authoring_viewport_mode(mode);
        }
    }

    fn viewport_layout_menu(&mut self, ui: &mut Ui) {
        let presets = self.viewport_workspace.presets.presets().to_vec();
        let mut built_in = None;
        let mut user_preset = None;
        let mut delete_preset = None;
        let mut save_current = false;

        let (response, _) = MenuButton::new("Layouts").ui(ui, |ui| {
            ui.set_min_width(210.0);
            ui.strong("Built-in layouts");
            for preset in BuiltInPreset::ALL {
                if ui.button(preset.name()).clicked() {
                    built_in = Some(preset);
                    ui.close();
                }
            }
            ui.separator();
            ui.strong("My layouts");
            if presets.is_empty() {
                ui.label(
                    RichText::new("No saved layouts")
                        .italics()
                        .color(design::active().text_muted),
                );
            } else {
                for preset in &presets {
                    ui.horizontal(|ui| {
                        if ui.button(preset.name()).clicked() {
                            user_preset = Some(preset.clone());
                            ui.close();
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text("Delete saved layout")
                            .clicked()
                        {
                            delete_preset = Some(preset.name().to_owned());
                            ui.close();
                        }
                    });
                }
            }
            ui.separator();
            if ui.button("Save current layout…").clicked() {
                save_current = true;
                ui.close();
            }
        });
        response.on_hover_text("Viewport tiling and saved layouts");

        let applied_built_in = built_in.is_some();
        let applied_user = user_preset.is_some();
        let result = if let Some(preset) = built_in {
            self.viewport_workspace.apply_builtin(preset)
        } else if let Some(preset) = user_preset {
            self.viewport_workspace.apply_user(&preset)
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.file_status = Some(error.to_string());
        } else if applied_built_in || applied_user {
            self.viewport_mode = self.viewport_workspace.active_mode();
        }
        if let Some(name) = delete_preset {
            self.viewport_workspace.delete_preset(&name);
        }
        if save_current {
            self.viewport_workspace.save_preset_open = true;
            if self.viewport_workspace.preset_name.is_empty() {
                self.viewport_workspace.preset_name = "My layout".to_owned();
            }
        }
    }

    fn viewport_preset_dialog(&mut self, ctx: &egui::Context) {
        if !self.viewport_workspace.save_preset_open {
            return;
        }
        let mut open = true;
        let mut name = self.viewport_workspace.preset_name.clone();
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Save viewport layout")
            .id(egui::Id::new("save-viewport-layout"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Name");
                let response = ui.text_edit_singleline(&mut name);
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add_enabled(!name.trim().is_empty(), egui::Button::new("Save"))
                        .clicked()
                    {
                        save = true;
                    }
                });
            });
        open &= !cancel;
        self.viewport_workspace.preset_name = name.clone();
        if save {
            match self.viewport_workspace.save_named_preset(&name) {
                Ok(()) => {
                    self.file_status = Some(format!("Saved viewport layout '{}'", name.trim()));
                    open = false;
                }
                Err(error) => self.file_status = Some(error.to_string()),
            }
        }
        self.viewport_workspace.save_preset_open = open;
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
        let wall_height = self
            .model
            .framing_defaults()
            .default_wall_height
            .to_string();
        let level_name = self.active_level_name();

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
        readonly_option(ui, "Level", self.active_level_name().as_str());
        readonly_option(ui, "Placement", "Enclosed region");
    }

    fn surface_tool_options(&self, ui: &mut Ui, title: &str, kind: &str, setting: &str) {
        option_strip_title(ui, title);
        readonly_option(ui, "Type", kind);
        readonly_option(ui, "Setting", setting);
        readonly_option(ui, "Level", self.active_level_name().as_str());
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
        FontId::proportional(design::text_size::MICRO),
        theme.text_secondary,
    );
    let compass = rect.shrink(5.0);
    for (label, align, pos) in [
        (
            "N",
            Align2::CENTER_TOP,
            compass.center_top() + Vec2::new(0.0, 1.0),
        ),
        (
            "S",
            Align2::CENTER_BOTTOM,
            compass.center_bottom() + Vec2::new(0.0, -1.0),
        ),
        (
            "W",
            Align2::LEFT_CENTER,
            compass.left_center() + Vec2::new(1.0, 0.0),
        ),
        (
            "E",
            Align2::RIGHT_CENTER,
            compass.right_center() + Vec2::new(-1.0, 0.0),
        ),
    ] {
        painter.text(
            pos,
            align,
            label,
            FontId::proportional(design::text_size::MICRO),
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
    fn four_up_layout_allocates_four_disjoint_exact_tile_rectangles() {
        let mut ids = PaneIdGenerator::default();
        let focus =
            layout::ViewportLayout::focus(&mut ids, layout::PaneConfig::new(ViewportMode::Plan))
                .unwrap();
        let layout = BuiltInPreset::FourUp.instantiate(&focus, &mut ids).unwrap();
        let bounds = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(800.0, 600.0));
        let mut panes = Vec::new();
        let mut splitters = Vec::new();

        collect_docked_layout(
            layout.root(),
            bounds,
            &mut Vec::new(),
            &mut panes,
            &mut splitters,
        );

        assert_eq!(panes.len(), 4);
        assert_eq!(splitters.len(), 3);
        for pane in &panes {
            assert!(bounds.contains(pane.rect.left_top()));
            assert!(bounds.contains(pane.rect.right_bottom()));
            assert!(pane.rect.width() > 1.0);
            assert!(pane.rect.height() > 1.0);
        }
        for (index, pane) in panes.iter().enumerate() {
            for other in panes.iter().skip(index + 1) {
                let overlap = pane.rect.intersect(other.rect);
                assert!(overlap.width() <= 0.0 || overlap.height() <= 0.0);
            }
        }
    }

    #[test]
    fn popped_out_leaf_is_removed_from_docked_projection_without_losing_topology() {
        let mut ids = PaneIdGenerator::default();
        let focus =
            layout::ViewportLayout::focus(&mut ids, layout::PaneConfig::new(ViewportMode::Plan))
                .unwrap();
        let mut layout = BuiltInPreset::PlanAnd3d
            .instantiate(&focus, &mut ids)
            .unwrap();
        let popped = layout.pane_ids()[0];
        layout
            .pane_mut(popped)
            .unwrap()
            .config_mut()
            .set_popped_out(true);
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(320.0, 180.0));
        let mut panes = Vec::new();
        let mut splitters = Vec::new();

        collect_docked_layout(
            layout.root(),
            bounds,
            &mut Vec::new(),
            &mut panes,
            &mut splitters,
        );

        assert_eq!(layout.pane_count(), 2);
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].rect, bounds);
        assert!(splitters.is_empty());
    }

    #[test]
    fn collapsed_popout_branch_preserves_nested_splitter_path() {
        let mut ids = PaneIdGenerator::default();
        let mut layout =
            layout::ViewportLayout::focus(&mut ids, layout::PaneConfig::new(ViewportMode::Plan))
                .unwrap();
        let popped = layout.active_id();
        let nested_first = layout
            .duplicate(popped, SplitAxis::Horizontal, 0.25, &mut ids)
            .unwrap();
        layout
            .duplicate(nested_first, SplitAxis::Vertical, 0.4, &mut ids)
            .unwrap();
        layout
            .pane_mut(popped)
            .unwrap()
            .config_mut()
            .set_popped_out(true);

        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(320.0, 180.0));
        let mut panes = Vec::new();
        let mut splitters = Vec::new();
        collect_docked_layout(
            layout.root(),
            bounds,
            &mut Vec::new(),
            &mut panes,
            &mut splitters,
        );

        assert_eq!(panes.len(), 2);
        assert_eq!(splitters.len(), 1);
        assert_eq!(splitters[0].path, vec![SplitSide::Second]);
        assert_eq!(splitters[0].axis, SplitAxis::Vertical);

        layout.set_split_ratio(&splitters[0].path, 0.75).unwrap();
        let LayoutNode::Split {
            ratio: root_ratio,
            second,
            ..
        } = layout.root()
        else {
            panic!("root should remain the collapsed horizontal split");
        };
        let LayoutNode::Split {
            ratio: nested_ratio,
            ..
        } = second.as_ref()
        else {
            panic!("second subtree should contain the visible vertical split");
        };
        assert_eq!(*root_ratio, 0.25);
        assert_eq!(*nested_ratio, 0.75);
    }

    #[test]
    fn view_3d_state_orbits_zooms_and_snaps() {
        let mut view = View3dState::default();
        let initial_yaw = view.yaw;
        let initial_pitch = view.pitch;

        view.orbit(Vec2::new(20.0, -10.0));
        assert!(view.yaw > initial_yaw);
        assert!(view.pitch > initial_pitch);

        view.zoom_by(10.0);
        assert_eq!(view.zoom, 10.0);

        view.zoom_by(10.0);
        assert_eq!(view.zoom, ZOOM_MAX_3D);

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
