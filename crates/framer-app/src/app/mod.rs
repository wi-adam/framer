mod labels;
mod model_edit;
mod panels;
mod project_io;
mod viewport;

use std::fs;
use std::path::PathBuf;

use eframe::egui::{
    self, CentralPanel, Color32, CornerRadius, FontFamily, FontId, Panel, ScrollArea, Stroke,
    TextStyle, Vec2,
};
use framer_core::{
    BuildingModel, DimensionAnchor, DimensionAxis, DimensionConstraint, DimensionDirection,
    DimensionKind, Length, Opening, OpeningKind, Wall, load_project as load_project_document,
    save_project as save_project_document,
};
use framer_solver::{
    FrameMember, ProjectFramePlan, export_bom_csv, export_project_svg, generate_project_plan,
};

use model_edit::{
    OpeningDragConstraints, OpeningDragState, OpeningEditHandle, apply_opening_drag,
    next_dimension_id, next_opening_id,
};
use project_io::{DEFAULT_PROJECT_PATH, export_paths, write_text_file};
use viewport::View3dState;

pub(crate) struct FramerApp {
    model: BuildingModel,
    selected_wall: usize,
    selected: Selection,
    project_plan: Option<ProjectFramePlan>,
    error: Option<String>,
    project_path: String,
    file_status: Option<String>,
    artifact_status: Option<String>,
    dimension_status: Option<String>,
    workspace_mode: WorkspaceMode,
    viewport_mode: ViewportMode,
    view_3d: View3dState,
    dimension_tool: DimensionToolState,
    opening_drag: Option<OpeningDragState>,
    gpu_target_format: Option<eframe::wgpu::TextureFormat>,
    show_section: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Level(String),
    Wall,
    Opening(String),
    Dimension(String),
    Join(String),
    Member { wall_id: String, member_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceMode {
    Design,
    Plan,
}

impl WorkspaceMode {
    fn allows_design_edits(self) -> bool {
        matches!(self, Self::Design)
    }

    fn shows_generated_plan(self) -> bool {
        matches!(self, Self::Plan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewportMode {
    Plan,
    Elevation,
    Axonometric,
}

#[derive(Clone)]
enum ViewClick {
    Wall(usize),
    Opening {
        wall_index: usize,
        opening_id: String,
    },
    Dimension {
        wall_index: usize,
        dimension_id: String,
    },
    DimensionAnchor {
        wall_index: usize,
        anchor: DimensionAnchor,
    },
    DimensionPlacement {
        wall_index: usize,
        axis: DimensionAxis,
        line_offset: Length,
    },
    Member {
        wall_id: String,
        member_id: String,
    },
}

#[derive(Debug, Clone)]
struct DimensionToolState {
    active: bool,
    kind: DimensionKind,
    axis: DimensionAxis,
    first_anchor: Option<DimensionAnchorPick>,
    second_anchor: Option<DimensionAnchorPick>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DimensionAnchorPick {
    wall_index: usize,
    anchor: DimensionAnchor,
}

impl Default for DimensionToolState {
    fn default() -> Self {
        Self {
            active: false,
            kind: DimensionKind::Driving,
            axis: DimensionAxis::Horizontal,
            first_anchor: None,
            second_anchor: None,
        }
    }
}

impl DimensionToolState {
    fn clear_picks(&mut self) {
        self.first_anchor = None;
        self.second_anchor = None;
    }
}

fn dimension_kind_name(kind: DimensionKind) -> &'static str {
    match kind {
        DimensionKind::Driving => "driving",
        DimensionKind::Reference => "reference",
    }
}

fn dimension_axis_name(axis: DimensionAxis) -> &'static str {
    match axis {
        DimensionAxis::Horizontal => "horizontal",
        DimensionAxis::Vertical => "vertical",
    }
}

impl Default for FramerApp {
    fn default() -> Self {
        let mut app = Self {
            model: BuildingModel::demo_shell(),
            selected_wall: 0,
            selected: Selection::Wall,
            project_plan: None,
            error: None,
            project_path: DEFAULT_PROJECT_PATH.to_owned(),
            file_status: None,
            artifact_status: None,
            dimension_status: None,
            workspace_mode: WorkspaceMode::Design,
            viewport_mode: ViewportMode::Plan,
            view_3d: View3dState::default(),
            dimension_tool: DimensionToolState::default(),
            opening_drag: None,
            gpu_target_format: None,
            show_section: true,
        };
        app.rebuild();
        app
    }
}

impl FramerApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_app_style(&cc.egui_ctx);

        Self {
            gpu_target_format: cc
                .wgpu_render_state
                .as_ref()
                .map(|render_state| render_state.target_format),
            ..Self::default()
        }
    }

    fn rebuild(&mut self) {
        if self.selected_wall >= self.model.walls.len() {
            self.selected_wall = 0;
            self.selected = Selection::Wall;
        }

        self.model.apply_driving_dimensions();

        match generate_project_plan(&self.model) {
            Ok(plan) => {
                self.project_plan = Some(plan);
                self.error = None;
            }
            Err(error) => {
                self.project_plan = None;
                self.error = Some(error.to_string());
            }
        }
    }

    fn new_project(&mut self) {
        let code = framer_core::CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
        model.walls.push(Wall::new(
            "wall-1",
            "Untitled wall",
            Length::from_feet(12.0),
            &code,
        ));
        self.model = model;
        self.selected_wall = 0;
        self.selected = Selection::Wall;
        self.project_path = "untitled-alpha.framer".to_owned();
        self.file_status = Some("Created new project".to_owned());
        self.artifact_status = None;
        self.dimension_status = None;
        self.dimension_tool = DimensionToolState::default();
        self.opening_drag = None;
        self.workspace_mode = WorkspaceMode::Design;
        self.rebuild();
    }

    fn reset_demo(&mut self) {
        self.model = BuildingModel::demo_shell();
        self.selected_wall = 0;
        self.selected = Selection::Wall;
        self.project_path = DEFAULT_PROJECT_PATH.to_owned();
        self.file_status = Some("Reset to multi-wall demo shell".to_owned());
        self.artifact_status = None;
        self.dimension_status = None;
        self.dimension_tool = DimensionToolState::default();
        self.opening_drag = None;
        self.workspace_mode = WorkspaceMode::Design;
        self.rebuild();
    }

    fn reset_wall_demo(&mut self) {
        self.model = BuildingModel::demo_wall();
        self.selected_wall = 0;
        self.selected = Selection::Wall;
        self.project_path = "examples/projects/demo-wall.framer".to_owned();
        self.file_status = Some("Reset to Phase 1 demo wall".to_owned());
        self.artifact_status = None;
        self.dimension_status = None;
        self.dimension_tool = DimensionToolState::default();
        self.opening_drag = None;
        self.workspace_mode = WorkspaceMode::Design;
        self.rebuild();
    }

    fn save_project_file(&mut self) {
        let path = PathBuf::from(self.project_path.trim());
        if path.as_os_str().is_empty() {
            self.file_status = Some("Choose a project path before saving".to_owned());
            return;
        }

        let result = save_project_document(&self.model)
            .map_err(|error| error.to_string())
            .and_then(|document| write_text_file(&path, document));

        self.file_status = Some(match result {
            Ok(()) => format!("Saved {}", path.display()),
            Err(error) => format!("Save failed: {error}"),
        });
    }

    fn load_project_file(&mut self) {
        let path = PathBuf::from(self.project_path.trim());
        if path.as_os_str().is_empty() {
            self.file_status = Some("Choose a project path before opening".to_owned());
            return;
        }

        let result = fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|source| load_project_document(&source).map_err(|error| error.to_string()));

        match result {
            Ok(model) => {
                self.model = model;
                self.selected_wall = 0;
                self.selected = Selection::Wall;
                self.workspace_mode = WorkspaceMode::Design;
                self.rebuild();
                self.file_status = Some(format!("Opened {}", path.display()));
                self.artifact_status = None;
                self.dimension_status = None;
                self.dimension_tool = DimensionToolState::default();
                self.opening_drag = None;
            }
            Err(error) => {
                self.file_status = Some(format!("Open failed: {error}"));
            }
        }
    }

    fn export_current_artifacts(&mut self) {
        let Some(plan) = &self.project_plan else {
            self.artifact_status =
                Some("Export failed: regenerate a valid framing plan first".to_owned());
            return;
        };

        let (svg_path, csv_path) = export_paths(&self.project_path);
        let svg = export_project_svg(&self.model, plan);
        let csv = export_bom_csv(&plan.bom());

        let result = write_text_file(&svg_path, svg).and_then(|()| write_text_file(&csv_path, csv));
        self.artifact_status = Some(match result {
            Ok(()) => format!("Exported {} and {}", svg_path.display(), csv_path.display()),
            Err(error) => format!("Export failed: {error}"),
        });
    }

    fn set_workspace_mode(&mut self, mode: WorkspaceMode) {
        if self.workspace_mode == mode {
            return;
        }

        self.workspace_mode = mode;
        self.opening_drag = None;
        match mode {
            WorkspaceMode::Design => self.select_authored_for_design_mode(),
            WorkspaceMode::Plan => {
                self.dimension_tool.active = false;
                self.dimension_tool.clear_picks();
                self.rebuild();
            }
        }
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused() {
            return;
        }

        let (escape_pressed, dimension_pressed) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                input.consume_key(egui::Modifiers::NONE, egui::Key::D),
            )
        });

        if escape_pressed {
            self.exit_current_context();
        } else if dimension_pressed {
            self.activate_dimension_tool();
        }
    }

    fn activate_dimension_tool(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            self.set_workspace_mode(WorkspaceMode::Design);
        }

        self.dimension_tool.active = true;
        self.dimension_tool.clear_picks();
        self.opening_drag = None;
        self.dimension_status =
            Some("Pick two anchors, then move the pointer to place the dimension".to_owned());
        self.viewport_mode = ViewportMode::Elevation;
    }

    fn exit_current_context(&mut self) {
        if self.opening_drag.is_some() {
            self.opening_drag = None;
            return;
        }

        let dimension_tool_was_active = self.dimension_tool.active;
        let dimension_was_selected = matches!(self.selected, Selection::Dimension(_));
        self.dimension_tool.active = false;
        self.dimension_tool.clear_picks();
        if dimension_was_selected {
            self.dimension_status = None;
            self.selected = Selection::Wall;
            return;
        }
        if dimension_tool_was_active {
            self.dimension_status = None;
            return;
        }

        match &self.selected {
            Selection::Wall => {}
            Selection::Member { wall_id, .. } => {
                if let Some(index) = self
                    .model
                    .walls
                    .iter()
                    .position(|wall| wall.id.0 == *wall_id)
                {
                    self.selected_wall = index;
                }
                self.selected = Selection::Wall;
            }
            Selection::Level(_) | Selection::Opening(_) | Selection::Join(_) => {
                self.selected = Selection::Wall;
            }
            Selection::Dimension(_) => unreachable!("dimension selections exit above"),
        }
    }

    fn select_authored_for_design_mode(&mut self) {
        if let Selection::Member { wall_id, .. } = &self.selected {
            if let Some(index) = self
                .model
                .walls
                .iter()
                .position(|wall| wall.id.0 == *wall_id)
            {
                self.selected_wall = index;
            }
            self.selected = Selection::Wall;
        }
    }

    fn add_opening(&mut self, kind: OpeningKind) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }

        let Some(wall) = self.model.walls.get_mut(self.selected_wall) else {
            return;
        };

        let center = (wall.length / 2).max(Length::from_inches(24.0));
        let opening = match kind {
            OpeningKind::Door => {
                let (id, index) = next_opening_id(wall, "opening-door");
                Opening::door(
                    id,
                    format!("Door {index}"),
                    center,
                    Length::from_inches(36.0),
                    Length::from_inches(80.0),
                )
            }
            OpeningKind::GarageDoor => {
                let (id, index) = next_opening_id(wall, "opening-garage");
                Opening::door(
                    id,
                    format!("Garage Door {index}"),
                    center,
                    Length::from_feet(8.0),
                    Length::from_inches(84.0),
                )
                .with_kind(OpeningKind::GarageDoor)
            }
            OpeningKind::Window | OpeningKind::Skylight | OpeningKind::Stair => {
                let (id, index) = next_opening_id(wall, "opening-window");
                Opening::window(
                    id,
                    format!("Window {index}"),
                    center,
                    Length::from_inches(36.0),
                    Length::from_inches(42.0),
                    Length::from_inches(36.0),
                )
            }
        };

        self.selected = Selection::Opening(opening.id.0.clone());
        wall.openings.push(opening);
        self.rebuild();
    }

    fn begin_opening_drag(
        &mut self,
        wall_index: usize,
        opening_id: String,
        handle: OpeningEditHandle,
    ) {
        let Some(wall) = self.model.walls.get(wall_index) else {
            return;
        };
        let Some(opening) = wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == opening_id)
        else {
            return;
        };

        self.selected_wall = wall_index;
        self.selected = Selection::Opening(opening_id.clone());
        self.opening_drag = Some(OpeningDragState::new(
            wall_index, opening_id, handle, opening,
        ));
    }

    fn update_opening_drag(&mut self, delta_x: Length, delta_y: Length) {
        let Some(drag) = self.opening_drag.clone() else {
            return;
        };
        let constraints = OpeningDragConstraints::from_code(&self.model.code);
        let Some(wall) = self.model.walls.get_mut(drag.wall_index) else {
            self.opening_drag = None;
            return;
        };

        if apply_opening_drag(
            wall,
            &drag.opening_id,
            drag.handle,
            drag.start,
            delta_x,
            delta_y,
            constraints,
        ) {
            self.selected_wall = drag.wall_index;
            self.selected = Selection::Opening(drag.opening_id);
            self.rebuild();
        }
    }

    fn finish_opening_drag(&mut self) {
        self.opening_drag = None;
    }

    fn handle_dimension_anchor_click(&mut self, wall_index: usize, anchor: DimensionAnchor) {
        if !self.workspace_mode.allows_design_edits() || !self.dimension_tool.active {
            return;
        }

        let pick = DimensionAnchorPick { wall_index, anchor };
        let Some(first) = self.dimension_tool.first_anchor.clone() else {
            self.dimension_status = Some("Pick a second dimension anchor".to_owned());
            self.dimension_tool.first_anchor = Some(pick);
            self.dimension_tool.second_anchor = None;
            return;
        };

        if first.wall_index != pick.wall_index {
            self.dimension_status =
                Some("Dimension anchors must be on the same wall for now".to_owned());
            self.dimension_tool.first_anchor = Some(pick);
            self.dimension_tool.second_anchor = None;
            return;
        }

        if first.anchor == pick.anchor {
            self.dimension_status = Some("Pick a different anchor".to_owned());
            self.dimension_tool.second_anchor = None;
            return;
        }

        self.dimension_tool.second_anchor = Some(pick);
        self.dimension_status = Some(
            "Move the pointer to choose the dimension axis and line position, then click to place"
                .to_owned(),
        );
    }

    fn handle_dimension_placement_click(
        &mut self,
        wall_index: usize,
        axis: DimensionAxis,
        line_offset: Length,
    ) {
        if !self.workspace_mode.allows_design_edits() || !self.dimension_tool.active {
            return;
        }

        let Some(first) = self.dimension_tool.first_anchor.clone() else {
            return;
        };
        let Some(second) = self.dimension_tool.second_anchor.clone() else {
            return;
        };
        if first.wall_index != wall_index || second.wall_index != wall_index {
            self.dimension_status =
                Some("Dimension anchors must be on the same wall for now".to_owned());
            self.dimension_tool.clear_picks();
            return;
        }

        let Some(wall) = self.model.walls.get_mut(wall_index) else {
            return;
        };
        let Some(start_coordinate) = first.anchor.coordinate(wall, axis) else {
            self.dimension_status = Some("The first dimension anchor no longer exists".to_owned());
            return;
        };
        let Some(end_coordinate) = second.anchor.coordinate(wall, axis) else {
            self.dimension_status = Some("The second dimension anchor no longer exists".to_owned());
            return;
        };
        let measured = (end_coordinate - start_coordinate).abs();
        if measured <= Length::ZERO {
            self.dimension_status =
                Some("Move the pointer to place a non-zero dimension".to_owned());
            return;
        }

        let kind = self.dimension_tool.kind;
        let (id, index) = next_dimension_id(wall);
        let direction = if end_coordinate >= start_coordinate {
            DimensionDirection::Forward
        } else {
            DimensionDirection::Backward
        };
        let value = if kind == DimensionKind::Driving {
            Some(measured)
        } else {
            None
        };
        let dimension = DimensionConstraint::new(
            id.clone(),
            format!("Dimension {index}"),
            kind,
            first.anchor,
            second.anchor,
            direction,
            value,
        )
        .with_axis(axis)
        .with_line_offset(line_offset);
        if wall.would_overconstrain_driving_dimension(&dimension) {
            self.dimension_status =
                Some("Driving dimension would overconstrain this wall".to_owned());
            return;
        }
        wall.dimensions.push(dimension);
        self.dimension_tool.axis = axis;
        self.dimension_tool.clear_picks();

        self.selected_wall = wall_index;
        self.selected = Selection::Dimension(id);
        self.dimension_status = Some(format!(
            "Added {} {} dimension",
            dimension_axis_name(axis),
            dimension_kind_name(kind)
        ));
        self.rebuild();
    }

    fn selected_member(&self, wall_id: &str, member_id: &str) -> Option<&FrameMember> {
        self.project_plan
            .as_ref()?
            .wall_plans
            .iter()
            .find(|wall_plan| wall_plan.wall.0 == wall_id)?
            .members
            .iter()
            .find(|member| member.id == member_id)
    }

    fn handle_view_click(&mut self, click: ViewClick) {
        self.opening_drag = None;
        match click {
            ViewClick::Wall(index) => {
                self.selected_wall = index;
                self.selected = Selection::Wall;
                self.open_wall_view_from_design_shell();
            }
            ViewClick::Opening {
                wall_index,
                opening_id,
            } => {
                self.selected_wall = wall_index;
                self.selected = Selection::Opening(opening_id);
                self.open_wall_view_from_design_shell();
            }
            ViewClick::Dimension {
                wall_index,
                dimension_id,
            } => {
                self.selected_wall = wall_index;
                self.selected = Selection::Dimension(dimension_id);
                self.open_wall_view_from_design_shell();
            }
            ViewClick::DimensionAnchor { wall_index, anchor } => {
                self.handle_dimension_anchor_click(wall_index, anchor);
            }
            ViewClick::DimensionPlacement {
                wall_index,
                axis,
                line_offset,
            } => {
                self.handle_dimension_placement_click(wall_index, axis, line_offset);
            }
            ViewClick::Member { wall_id, member_id } => {
                if self.workspace_mode.shows_generated_plan() {
                    if let Some(index) = self
                        .model
                        .walls
                        .iter()
                        .position(|wall| wall.id.0 == wall_id)
                    {
                        self.selected_wall = index;
                    }
                    self.selected = Selection::Member { wall_id, member_id };
                }
            }
        }
    }

    fn open_wall_view_from_design_shell(&mut self) {
        if self.workspace_mode.allows_design_edits() && self.viewport_mode == ViewportMode::Plan {
            self.viewport_mode = ViewportMode::Elevation;
        }
    }
}

fn configure_app_style(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(16.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(12.5, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(10.0, FontFamily::Proportional),
    );
    style.spacing.item_spacing = Vec2::new(8.0, 4.0);
    style.spacing.button_padding = Vec2::new(7.0, 3.0);
    style.spacing.interact_size = Vec2::new(40.0, 22.0);
    style.spacing.window_margin = egui::Margin::symmetric(8, 6);
    style.visuals.panel_fill = Color32::from_rgb(24, 26, 27);
    style.visuals.window_fill = Color32::from_rgb(29, 31, 32);
    style.visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(61, 66, 68));
    style.visuals.extreme_bg_color = Color32::from_rgb(11, 12, 12);
    style.visuals.faint_bg_color = Color32::from_rgb(35, 38, 39);
    style.visuals.text_edit_bg_color = Some(Color32::from_rgb(13, 14, 14));
    style.visuals.selection.bg_fill = Color32::from_rgb(0, 114, 160);
    style.visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(239, 250, 253));
    style.visuals.hyperlink_color = Color32::from_rgb(82, 184, 222);
    style.visuals.warn_fg_color = Color32::from_rgb(229, 169, 77);
    style.visuals.error_fg_color = Color32::from_rgb(229, 96, 88);
    style.visuals.button_frame = true;
    style.visuals.collapsing_header_frame = false;
    style.visuals.indent_has_left_vline = true;

    let radius = CornerRadius::same(3);
    style.visuals.widgets.noninteractive.corner_radius = radius;
    style.visuals.widgets.inactive.corner_radius = radius;
    style.visuals.widgets.hovered.corner_radius = radius;
    style.visuals.widgets.active.corner_radius = radius;
    style.visuals.widgets.open.corner_radius = radius;
    style.visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(32, 35, 36);
    style.visuals.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(34, 37, 38);
    style.visuals.widgets.noninteractive.bg_stroke =
        Stroke::new(1.0, Color32::from_rgb(56, 61, 63));
    style.visuals.widgets.noninteractive.fg_stroke =
        Stroke::new(1.0, Color32::from_rgb(216, 221, 220));
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(42, 45, 47);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(45, 48, 50);
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(72, 78, 82));
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(218, 223, 222));
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(54, 59, 61);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(58, 64, 66);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(88, 98, 102));
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(244, 247, 247));
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(0, 91, 128);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(0, 114, 160);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(74, 190, 229));
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::from_rgb(250, 253, 253));
    style.visuals.widgets.open = style.visuals.widgets.hovered;

    ctx.set_global_style(style);
}

impl eframe::App for FramerApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_keyboard_shortcuts(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        Panel::top("toolbar").show_inside(ui, |ui| self.toolbar(ui));
        Panel::left("model-tree")
            .resizable(true)
            .default_size(280.0)
            .size_range(240.0..=380.0)
            .show_inside(ui, |ui| self.model_tree(ui));
        Panel::right("inspector")
            .resizable(true)
            .default_size(360.0)
            .size_range(300.0..=520.0)
            .show_inside(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.inspector(ui));
            });
        CentralPanel::default().show_inside(ui, |ui| self.workspace(ui));
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, process};

    use framer_core::{DimensionHorizontalReference, DimensionVerticalReference};

    use super::*;

    fn place_pending_dimension(app: &mut FramerApp, axis: DimensionAxis) {
        app.handle_view_click(ViewClick::DimensionPlacement {
            wall_index: 0,
            axis,
            line_offset: dimension_line_offset(axis),
        });
    }

    fn dimension_line_offset(axis: DimensionAxis) -> Length {
        match axis {
            DimensionAxis::Horizontal => Length::from_inches(72.0),
            DimensionAxis::Vertical => Length::from_inches(96.0),
        }
    }

    #[test]
    fn app_saves_and_reopens_demo_project() {
        let path = std::env::temp_dir().join(format!("framer-demo-wall-{}.framer", process::id()));
        let mut app = FramerApp {
            project_path: path.display().to_string(),
            ..FramerApp::default()
        };

        app.save_project_file();
        assert!(matches!(app.file_status.as_deref(), Some(status) if status.starts_with("Saved ")));

        app.model.walls[0].name = "Changed wall".to_owned();
        app.load_project_file();

        assert!(
            matches!(app.file_status.as_deref(), Some(status) if status.starts_with("Opened "))
        );
        assert_eq!(app.model, BuildingModel::demo_shell());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn app_exports_svg_and_csv_artifacts() {
        let path =
            std::env::temp_dir().join(format!("framer-demo-export-{}.framer", process::id()));
        let svg_path = path.with_extension("svg");
        let csv_path = path.with_extension("csv");
        let mut app = FramerApp {
            project_path: path.display().to_string(),
            ..FramerApp::default()
        };

        app.export_current_artifacts();

        assert!(
            matches!(app.artifact_status.as_deref(), Some(status) if status.starts_with("Exported "))
        );
        let svg = fs::read_to_string(&svg_path).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("data-wall=\"wall-front\""));
        assert!(
            fs::read_to_string(&csv_path)
                .unwrap()
                .contains("quantity,profile,kind")
        );

        let _ = fs::remove_file(svg_path);
        let _ = fs::remove_file(csv_path);
    }

    #[test]
    fn new_project_creates_schema_backed_wall_intent() {
        let mut app = FramerApp::default();
        app.new_project();

        assert_eq!(app.model.walls.len(), 1);
        assert!(save_project_document(&app.model).is_ok());
    }

    #[test]
    fn design_mode_keeps_generated_members_out_of_the_editing_selection() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Plan);
        let wall_id = app.model.walls[0].id.0.clone();
        let member_id = app.project_plan.as_ref().unwrap().wall_plans[0].members[0]
            .id
            .clone();
        app.selected = Selection::Member { wall_id, member_id };

        app.set_workspace_mode(WorkspaceMode::Design);

        assert_eq!(app.workspace_mode, WorkspaceMode::Design);
        assert_eq!(app.selected, Selection::Wall);
    }

    #[test]
    fn plan_mode_does_not_mutate_authored_openings_from_catalog_actions() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Plan);
        let opening_count = app.model.walls[0].openings.len();

        app.add_opening(OpeningKind::Door);

        assert_eq!(app.model.walls[0].openings.len(), opening_count);
    }

    #[test]
    fn design_shell_wall_click_opens_wall_layout_view() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Design);
        app.viewport_mode = ViewportMode::Plan;

        app.handle_view_click(ViewClick::Wall(1));

        assert_eq!(app.selected_wall, 1);
        assert_eq!(app.selected, Selection::Wall);
        assert_eq!(app.viewport_mode, ViewportMode::Elevation);
    }

    #[test]
    fn plan_wall_click_keeps_plan_view() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Plan);
        app.viewport_mode = ViewportMode::Plan;

        app.handle_view_click(ViewClick::Wall(1));

        assert_eq!(app.selected_wall, 1);
        assert_eq!(app.selected, Selection::Wall);
        assert_eq!(app.viewport_mode, ViewportMode::Plan);
    }

    #[test]
    fn dimension_shortcut_enters_dimension_tool_in_wall_view() {
        let mut app = FramerApp {
            viewport_mode: ViewportMode::Plan,
            ..Default::default()
        };
        app.dimension_tool.first_anchor = Some(DimensionAnchorPick {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });

        app.activate_dimension_tool();

        assert!(app.dimension_tool.active);
        assert_eq!(app.dimension_tool.first_anchor, None);
        assert_eq!(app.dimension_tool.second_anchor, None);
        assert_eq!(app.viewport_mode, ViewportMode::Elevation);
        assert!(
            app.dimension_status
                .as_deref()
                .is_some_and(|status| status.contains("Pick two anchors"))
        );
    }

    #[test]
    fn escape_exits_selected_dimension() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.first_anchor = Some(DimensionAnchorPick {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.selected = Selection::Dimension("dimension-1".to_owned());
        app.dimension_status = Some("Pick a second dimension anchor".to_owned());

        app.exit_current_context();

        assert!(!app.dimension_tool.active);
        assert_eq!(app.dimension_tool.first_anchor, None);
        assert_eq!(app.dimension_tool.second_anchor, None);
        assert_eq!(app.selected, Selection::Wall);
        assert_eq!(app.dimension_status, None);
    }

    #[test]
    fn escape_cancels_dimension_tool_without_dropping_other_selection() {
        let mut app = FramerApp::default();
        let opening = app.model.walls[0].openings[0].id.0.clone();
        app.dimension_tool.active = true;
        app.selected = Selection::Opening(opening.clone());
        app.dimension_status = Some("Pick two anchors in the wall view".to_owned());

        app.exit_current_context();

        assert!(!app.dimension_tool.active);
        assert_eq!(app.selected, Selection::Opening(opening));
        assert_eq!(app.dimension_status, None);
    }

    #[test]
    fn dimension_tool_clicks_create_driving_dimension() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Driving;
        let opening = app.model.walls[0].openings[0].id.clone();
        let expected = app.model.walls[0].openings[0].center;

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningCenter { opening },
        });
        assert_eq!(app.model.walls[0].dimensions.len(), 0);
        assert!(app.dimension_tool.second_anchor.is_some());

        place_pending_dimension(&mut app, DimensionAxis::Horizontal);

        let dimension = &app.model.walls[0].dimensions[0];
        assert_eq!(dimension.axis, DimensionAxis::Horizontal);
        assert_eq!(dimension.kind, DimensionKind::Driving);
        assert_eq!(dimension.value, Some(expected));
        assert_eq!(
            dimension.line_offset,
            Some(dimension_line_offset(DimensionAxis::Horizontal))
        );
        assert_eq!(app.dimension_tool.first_anchor, None);
        assert_eq!(app.dimension_tool.second_anchor, None);
        assert_eq!(app.selected, Selection::Dimension(dimension.id.0.clone()));
    }

    #[test]
    fn reference_dimensions_store_no_driving_value() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Reference;
        let opening = app.model.walls[0].openings[0].id.clone();

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningLeft { opening },
        });
        place_pending_dimension(&mut app, DimensionAxis::Horizontal);

        let dimension = &app.model.walls[0].dimensions[0];
        assert_eq!(dimension.kind, DimensionKind::Reference);
        assert_eq!(dimension.value, None);
    }

    #[test]
    fn vertical_dimension_tool_clicks_create_height_dimension() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Driving;
        app.dimension_tool.axis = DimensionAxis::Vertical;
        let opening = app.model.walls[0].openings[0].id.clone();
        let expected = app.model.walls[0].openings[0].height;

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningPoint {
                opening: opening.clone(),
                horizontal: DimensionHorizontalReference::Center,
                vertical: DimensionVerticalReference::Bottom,
            },
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningPoint {
                opening,
                horizontal: DimensionHorizontalReference::Center,
                vertical: DimensionVerticalReference::Top,
            },
        });
        place_pending_dimension(&mut app, DimensionAxis::Vertical);

        let dimension = &app.model.walls[0].dimensions[0];
        assert_eq!(dimension.axis, DimensionAxis::Vertical);
        assert_eq!(dimension.kind, DimensionKind::Driving);
        assert_eq!(dimension.value, Some(expected));
        assert_eq!(
            dimension.line_offset,
            Some(dimension_line_offset(DimensionAxis::Vertical))
        );
        assert_eq!(app.selected, Selection::Dimension(dimension.id.0.clone()));
    }

    #[test]
    fn dimension_tool_uses_placement_axis_for_pending_dimension() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Driving;
        app.dimension_tool.axis = DimensionAxis::Horizontal;
        let opening = app.model.walls[0].openings[0].id.clone();
        let expected =
            (app.model.walls[0].height - app.model.walls[0].openings[0].sill_height).abs();

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallPoint {
                horizontal: DimensionHorizontalReference::Left,
                vertical: DimensionVerticalReference::Top,
            },
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningPoint {
                opening,
                horizontal: DimensionHorizontalReference::Center,
                vertical: DimensionVerticalReference::Bottom,
            },
        });
        assert_eq!(app.model.walls[0].dimensions.len(), 0);

        place_pending_dimension(&mut app, DimensionAxis::Vertical);

        let dimension = &app.model.walls[0].dimensions[0];
        assert_eq!(dimension.axis, DimensionAxis::Vertical);
        assert_eq!(dimension.value, Some(expected));
        assert_eq!(app.dimension_tool.axis, DimensionAxis::Vertical);
        assert!(
            app.dimension_status
                .as_deref()
                .is_some_and(|status| status.contains("vertical"))
        );
    }

    #[test]
    fn dimension_tool_rejects_overconstrained_driving_dimension_on_creation() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Driving;
        let opening = app.model.walls[0].openings[0].id.clone();

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningLeft {
                opening: opening.clone(),
            },
        });
        place_pending_dimension(&mut app, DimensionAxis::Horizontal);
        assert_eq!(app.model.walls[0].dimensions.len(), 1);

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningRight {
                opening: opening.clone(),
            },
        });
        place_pending_dimension(&mut app, DimensionAxis::Horizontal);
        assert_eq!(app.model.walls[0].dimensions.len(), 2);

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningLeft {
                opening: opening.clone(),
            },
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningRight { opening },
        });
        place_pending_dimension(&mut app, DimensionAxis::Horizontal);

        assert_eq!(app.model.walls[0].dimensions.len(), 2);
        assert!(
            app.dimension_status
                .as_deref()
                .is_some_and(|status| status.contains("overconstrain"))
        );
    }
}
