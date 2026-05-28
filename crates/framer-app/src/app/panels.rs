use std::collections::BTreeSet;

use eframe::egui::{
    self, Align, Color32, ComboBox, Frame, Layout, Margin, Response, RichText, ScrollArea, Stroke,
    Ui, Vec2,
};
use framer_core::{
    DimensionAnchor, DimensionConstraint, DimensionKind, ElementId, Length, Level, Opening,
    OpeningKind, Wall, WallJoin, WallJoinKind,
};
use framer_solver::{DiagnosticSeverity, FrameMember, PlanDiagnostic, ProjectFramePlan};

use super::labels::{diagnostic_code_prefix, dimension_kind_label, join_kind_label, kind_label};
use super::model_edit::{
    opening_max_bottom, opening_top_clearance, set_wall_length_keep_direction,
};
use super::{FramerApp, Selection, ViewportMode, WorkspaceMode};

impl FramerApp {
    pub(super) fn toolbar(&mut self, ui: &mut Ui) {
        command_bar_frame().show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 5.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Framer")
                            .strong()
                            .size(18.0)
                            .color(Color32::from_rgb(238, 241, 240)),
                    );
                    toolbar_divider(ui);
                    ui.label(RichText::new("Project").small().color(toolbar_muted_text()));
                    let path_width = (ui.available_width() * 0.48).clamp(300.0, 620.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.project_path)
                            .desired_width(path_width)
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        status_chip(
                            ui,
                            self.model.code.display_name.as_str(),
                            StatusTone::Neutral,
                        );
                    });
                });

                ui.horizontal_wrapped(|ui| {
                    toolbar_group(ui, "PROJECT", |ui| {
                        if command_button(ui, "New", 48.0)
                            .on_hover_text("Start an empty wall project")
                            .clicked()
                        {
                            self.new_project();
                        }
                        if command_button(ui, "Open", 50.0)
                            .on_hover_text("Open the project path")
                            .clicked()
                        {
                            self.load_project_file();
                        }
                        if command_button(ui, "Save", 50.0)
                            .on_hover_text("Save the current model")
                            .clicked()
                        {
                            self.save_project_file();
                        }
                        if enabled_command_button(
                            ui,
                            self.workspace_mode.shows_generated_plan(),
                            "Export",
                            58.0,
                        )
                        .on_hover_text("Export plan artifacts from Plan workspace")
                        .clicked()
                        {
                            self.export_current_artifacts();
                        }
                    });
                    toolbar_divider(ui);

                    toolbar_group(ui, "SAMPLES", |ui| {
                        if command_button(ui, "Shell", 54.0)
                            .on_hover_text("Load the multi-wall shell demo")
                            .clicked()
                        {
                            self.reset_demo();
                        }
                        if command_button(ui, "Wall", 52.0)
                            .on_hover_text("Load the single-wall demo")
                            .clicked()
                        {
                            self.reset_wall_demo();
                        }
                    });
                    toolbar_divider(ui);

                    toolbar_group(ui, "WORKSPACE", |ui| {
                        let mut next_mode = self.workspace_mode;
                        workspace_segment(ui, &mut next_mode, WorkspaceMode::Design, "Design");
                        workspace_segment(ui, &mut next_mode, WorkspaceMode::Plan, "Plan");
                        if next_mode != self.workspace_mode {
                            self.set_workspace_mode(next_mode);
                        }
                    });
                    toolbar_divider(ui);

                    toolbar_group(ui, "VIEW", |ui| {
                        let shell_label = if self.workspace_mode.allows_design_edits() {
                            "Shell"
                        } else {
                            "Plan"
                        };
                        let wall_label = if self.workspace_mode.allows_design_edits() {
                            "Wall"
                        } else {
                            "Elevation"
                        };
                        view_segment(ui, &mut self.viewport_mode, ViewportMode::Plan, shell_label);
                        view_segment(
                            ui,
                            &mut self.viewport_mode,
                            ViewportMode::Elevation,
                            wall_label,
                        );
                        view_segment(ui, &mut self.viewport_mode, ViewportMode::Axonometric, "3D");
                        if self.workspace_mode.shows_generated_plan() {
                            ui.checkbox(&mut self.show_section, "Section");
                        }
                    });

                    if self.workspace_mode.allows_design_edits() {
                        toolbar_divider(ui);
                        toolbar_group(ui, "TOOLS", |ui| {
                            if segment_button(ui, self.dimension_tool.active, "Dimension", 84.0)
                                .on_hover_text("Place a wall dimension")
                                .clicked()
                            {
                                self.dimension_tool.active = !self.dimension_tool.active;
                                self.dimension_tool.first_anchor = None;
                                self.opening_drag = None;
                                self.dimension_status = if self.dimension_tool.active {
                                    Some("Pick two anchors in the wall view".to_owned())
                                } else {
                                    None
                                };
                                if self.dimension_tool.active {
                                    self.viewport_mode = ViewportMode::Elevation;
                                }
                            }
                            if self.dimension_tool.active {
                                ComboBox::from_id_salt("dimension-tool-kind")
                                    .selected_text(dimension_kind_label(self.dimension_tool.kind))
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
                                    });
                            }
                        });
                    }
                });
            });
        });

        if self.file_status.is_some()
            || self.artifact_status.is_some()
            || self.dimension_status.is_some()
        {
            Frame::new()
                .fill(Color32::from_rgb(29, 32, 33))
                .inner_margin(Margin::symmetric(10, 4))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(6.0, 3.0);
                        if let Some(status) = &self.file_status {
                            status_chip(ui, status, StatusTone::Info);
                        }
                        if let Some(status) = &self.artifact_status {
                            status_chip(ui, status, StatusTone::Success);
                        }
                        if let Some(status) = &self.dimension_status {
                            status_chip(ui, status, StatusTone::Warning);
                        }
                    });
                });
        }
    }

    pub(super) fn model_tree(&mut self, ui: &mut Ui) {
        panel_header(ui, "Model Tree", self.workspace_badge());

        ScrollArea::vertical().show(ui, |ui| {
            egui::CollapsingHeader::new("Authored")
                .default_open(true)
                .show(ui, |ui| {
                    let levels: Vec<_> = self
                        .model
                        .levels
                        .iter()
                        .map(|level| (level.id.0.clone(), level.name.clone()))
                        .collect();
                    let walls: Vec<_> = self
                        .model
                        .walls
                        .iter()
                        .enumerate()
                        .map(|(index, wall)| {
                            (
                                index,
                                wall.id.0.clone(),
                                wall.name.clone(),
                                wall.level.0.clone(),
                                wall.openings
                                    .iter()
                                    .map(|opening| {
                                        (opening.id.0.clone(), opening.kind, opening.name.clone())
                                    })
                                    .collect::<Vec<_>>(),
                                wall.dimensions
                                    .iter()
                                    .map(|dimension| {
                                        (
                                            dimension.id.0.clone(),
                                            dimension.kind,
                                            dimension.name.clone(),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect();
                    let joins: Vec<_> = self
                        .model
                        .wall_joins
                        .iter()
                        .map(|join| (join.id.0.clone(), join.name.clone(), join.kind))
                        .collect();

                    for (level_id, level_name) in levels {
                        let level_selected =
                            matches!(&self.selected, Selection::Level(id) if id == &level_id);
                        if ui
                            .selectable_label(level_selected, format!("Level: {level_name}"))
                            .clicked()
                        {
                            self.selected = Selection::Level(level_id.clone());
                        }

                        ui.indent(format!("level-{level_id}"), |ui| {
                            for (index, wall_id, wall_name, wall_level, openings, dimensions) in
                                &walls
                            {
                                if wall_level != &level_id {
                                    continue;
                                }

                                let wall_selected = self.selected_wall == *index
                                    && matches!(self.selected, Selection::Wall);
                                if ui
                                    .selectable_label(
                                        wall_selected,
                                        format!("Wall segment: {wall_name}"),
                                    )
                                    .clicked()
                                {
                                    self.selected_wall = *index;
                                    self.selected = Selection::Wall;
                                    self.rebuild();
                                }

                                ui.indent(format!("wall-{wall_id}"), |ui| {
                                    for (opening_id, opening_kind, opening_name) in openings {
                                        let selected = matches!(
                                            &self.selected,
                                            Selection::Opening(id) if id == opening_id
                                        );
                                        if ui
                                            .selectable_label(
                                                selected,
                                                format!(
                                                    "{}: {}",
                                                    kind_label(*opening_kind),
                                                    opening_name
                                                ),
                                            )
                                            .clicked()
                                        {
                                            self.selected_wall = *index;
                                            self.selected = Selection::Opening(opening_id.clone());
                                            self.rebuild();
                                        }
                                    }
                                    for (dimension_id, dimension_kind, dimension_name) in dimensions
                                    {
                                        let selected = matches!(
                                            &self.selected,
                                            Selection::Dimension(id) if id == dimension_id
                                        );
                                        if ui
                                            .selectable_label(
                                                selected,
                                                format!(
                                                    "{} dimension: {}",
                                                    dimension_kind_label(*dimension_kind),
                                                    dimension_name
                                                ),
                                            )
                                            .clicked()
                                        {
                                            self.selected_wall = *index;
                                            self.selected =
                                                Selection::Dimension(dimension_id.clone());
                                            self.rebuild();
                                        }
                                    }
                                });
                            }
                        });
                    }

                    if !joins.is_empty() {
                        ui.separator();
                        ui.strong("Wall joins");
                        for (join_id, join_name, join_kind) in joins {
                            let selected =
                                matches!(&self.selected, Selection::Join(id) if id == &join_id);
                            if ui
                                .selectable_label(
                                    selected,
                                    format!("{}: {}", join_kind_label(join_kind), join_name),
                                )
                                .clicked()
                            {
                                self.selected = Selection::Join(join_id);
                            }
                        }
                    }
                });

            if self.workspace_mode.shows_generated_plan() {
                let generated_count = self
                    .project_plan
                    .as_ref()
                    .map(|plan| {
                        plan.wall_plans
                            .iter()
                            .map(|wall_plan| wall_plan.members.len())
                            .sum::<usize>()
                    })
                    .unwrap_or_default();
                egui::CollapsingHeader::new(format!("Generated ({generated_count} members)"))
                    .default_open(true)
                    .show(ui, |ui| {
                        if let Some(plan) = &self.project_plan {
                            for wall_plan in &plan.wall_plans {
                                let wall_name = self
                                    .model
                                    .walls
                                    .iter()
                                    .find(|wall| wall.id == wall_plan.wall)
                                    .map(|wall| wall.name.as_str())
                                    .unwrap_or(wall_plan.wall.0.as_str());
                                let wall_selected = self
                                    .model
                                    .walls
                                    .get(self.selected_wall)
                                    .is_some_and(|wall| wall.id == wall_plan.wall);
                                egui::CollapsingHeader::new(format!(
                                    "Framing: {wall_name} ({} members)",
                                    wall_plan.members.len()
                                ))
                                .default_open(wall_selected)
                                .show(ui, |ui| {
                                    for member in &wall_plan.members {
                                        let selected = matches!(
                                            &self.selected,
                                            Selection::Member { wall_id, member_id }
                                                if wall_id == &wall_plan.wall.0
                                                    && member_id == &member.id
                                        );
                                        if ui
                                            .selectable_label(
                                                selected,
                                                format!("{}: {}", member.kind.label(), member.id),
                                            )
                                            .clicked()
                                        {
                                            if let Some(index) = self
                                                .model
                                                .walls
                                                .iter()
                                                .position(|wall| wall.id == wall_plan.wall)
                                            {
                                                self.selected_wall = index;
                                            }
                                            self.selected = Selection::Member {
                                                wall_id: wall_plan.wall.0.clone(),
                                                member_id: member.id.clone(),
                                            };
                                        }
                                    }
                                });
                            }
                        } else {
                            ui.label("No generated framing");
                        }
                    });
            } else {
                ui.separator();
                panel_subheader(ui, "Catalog");
                if ui.button("+ Door").clicked() {
                    self.add_opening(OpeningKind::Door);
                }
                if ui.button("+ Window").clicked() {
                    self.add_opening(OpeningKind::Window);
                }
                if ui.button("+ Garage Door").clicked() {
                    self.add_opening(OpeningKind::GarageDoor);
                }
            }
        });
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui) {
        let mut changed = false;
        let selection = self.selected.clone();
        let can_edit = self.workspace_mode.allows_design_edits();
        let level_options = self
            .model
            .levels
            .iter()
            .map(|level| (level.id.0.clone(), level.name.clone()))
            .collect::<Vec<_>>();
        let wall_options = self
            .model
            .walls
            .iter()
            .map(|wall| (wall.id.0.clone(), wall.name.clone()))
            .collect::<Vec<_>>();

        panel_header(ui, "Inspector", selection_badge(&selection));

        match selection {
            Selection::Level(id) => {
                if let Some(level) = self.model.levels.iter_mut().find(|level| level.id.0 == id) {
                    if can_edit {
                        ui.label(&level.id.0);
                        changed |= text_edit(ui, "Name", &mut level.name);
                        changed |= coordinate_drag(ui, "Elevation", &mut level.elevation);
                    } else {
                        level_summary(ui, level);
                    }
                } else {
                    ui.label("Level no longer exists");
                }
            }
            Selection::Wall => {
                let wall_length_driver =
                    self.model.walls.get(self.selected_wall).and_then(|wall| {
                        driven_length_field(wall, DimensionVariableKey::WallLength)
                    });
                let mut select_dimension = None;
                if let Some(wall) = self.model.walls.get_mut(self.selected_wall) {
                    if can_edit {
                        ui.label(&wall.id.0);
                        changed |= text_edit(ui, "Name", &mut wall.name);

                        let mut level_id = wall.level.0.clone();
                        ComboBox::from_label("Level")
                            .selected_text(level_display_name(&level_options, &level_id))
                            .show_ui(ui, |ui| {
                                for (id, name) in &level_options {
                                    ui.selectable_value(&mut level_id, id.clone(), name);
                                }
                            });
                        if level_id != wall.level.0 {
                            wall.level = ElementId::new(level_id);
                            changed = true;
                        }

                        let mut wall_length = wall.length;
                        if driven_length_drag(
                            ui,
                            "Length",
                            &mut wall_length,
                            length_drag_spec(24.0, 480.0, "ft"),
                            wall_length_driver.as_ref(),
                            &mut select_dimension,
                        ) {
                            set_wall_length_keep_direction(wall, wall_length);
                            changed = true;
                        }
                        changed |= driven_length_drag(
                            ui,
                            "Height",
                            &mut wall.height,
                            length_drag_spec(48.0, 168.0, "ft"),
                            None,
                            &mut select_dimension,
                        );
                        changed |= driven_length_drag(
                            ui,
                            "Stud spacing",
                            &mut wall.stud_spacing,
                            length_drag_spec(8.0, 32.0, "in"),
                            None,
                            &mut select_dimension,
                        );
                        ui.separator();
                        ui.strong("Placement");
                        let placement_changed = coordinate_drag(ui, "Start X", &mut wall.start.x)
                            | coordinate_drag(ui, "Start Y", &mut wall.start.y)
                            | coordinate_drag(ui, "End X", &mut wall.end.x)
                            | coordinate_drag(ui, "End Y", &mut wall.end.y);
                        if placement_changed {
                            if let Some(length) = wall.placement_length() {
                                wall.length = length;
                            }
                            changed = true;
                        }
                    } else {
                        wall_summary(ui, wall, &level_options);
                    }
                }

                if let Some(dimension_id) = select_dimension {
                    self.selected = Selection::Dimension(dimension_id);
                }
            }
            Selection::Opening(id) => {
                let top_clearance = opening_top_clearance(&self.model.code);
                let driven_fields = self
                    .model
                    .walls
                    .get(self.selected_wall)
                    .map(|wall| opening_driven_fields(wall, &id))
                    .unwrap_or_default();
                let mut select_dimension = None;
                if let Some(wall) = self.model.walls.get_mut(self.selected_wall) {
                    let mut remove = false;
                    let wall_height = wall.height;
                    if let Some(opening) =
                        wall.openings.iter_mut().find(|opening| opening.id.0 == id)
                    {
                        if can_edit {
                            ui.label(&opening.id.0);
                            changed |= text_edit(ui, "Name", &mut opening.name);
                            ComboBox::from_label("Kind")
                                .selected_text(kind_label(opening.kind))
                                .show_ui(ui, |ui| {
                                    changed |= ui
                                        .selectable_value(
                                            &mut opening.kind,
                                            OpeningKind::Door,
                                            "Door",
                                        )
                                        .changed();
                                    changed |= ui
                                        .selectable_value(
                                            &mut opening.kind,
                                            OpeningKind::Window,
                                            "Window",
                                        )
                                        .changed();
                                    changed |= ui
                                        .selectable_value(
                                            &mut opening.kind,
                                            OpeningKind::GarageDoor,
                                            "Garage door",
                                        )
                                        .changed();
                                });
                            changed |= driven_length_drag(
                                ui,
                                "Center",
                                &mut opening.center,
                                length_drag_spec(0.0, 480.0, "ft"),
                                driven_fields.center.as_ref(),
                                &mut select_dimension,
                            );
                            changed |= driven_length_drag(
                                ui,
                                "Width",
                                &mut opening.width,
                                length_drag_spec(12.0, 240.0, "in"),
                                driven_fields.width.as_ref(),
                                &mut select_dimension,
                            );
                            changed |= driven_length_drag(
                                ui,
                                "Height",
                                &mut opening.height,
                                length_drag_spec(12.0, 120.0, "in"),
                                None,
                                &mut select_dimension,
                            );
                            changed |= driven_length_drag(
                                ui,
                                "Bottom",
                                &mut opening.sill_height,
                                length_drag_spec(
                                    0.0,
                                    opening_max_bottom(wall_height, opening.height, top_clearance)
                                        .inches(),
                                    "in",
                                ),
                                None,
                                &mut select_dimension,
                            );

                            ui.separator();
                            if ui.button("Remove Opening").clicked() {
                                remove = true;
                            }
                        } else {
                            opening_summary(ui, opening);
                        }
                    } else {
                        ui.label("Opening no longer exists");
                    }

                    if remove {
                        wall.openings.retain(|opening| opening.id.0 != id);
                        self.selected = Selection::Wall;
                        changed = true;
                    }
                }

                if let Some(dimension_id) = select_dimension {
                    self.selected = Selection::Dimension(dimension_id);
                }
            }
            Selection::Dimension(id) => {
                if let Some(wall) = self.model.walls.get_mut(self.selected_wall) {
                    let mut remove = false;
                    if let Some(dimension_index) = wall
                        .dimensions
                        .iter()
                        .position(|dimension| dimension.id.0 == id)
                    {
                        if can_edit {
                            changed |= dimension_inspector(ui, wall, dimension_index, &mut remove);
                        } else {
                            dimension_summary(ui, wall, &wall.dimensions[dimension_index]);
                        }
                    } else {
                        ui.label("Dimension no longer exists");
                    }

                    if remove {
                        wall.dimensions.retain(|dimension| dimension.id.0 != id);
                        self.selected = Selection::Wall;
                        changed = true;
                    }
                }
            }
            Selection::Join(id) => {
                if let Some(join) = self
                    .model
                    .wall_joins
                    .iter_mut()
                    .find(|join| join.id.0 == id)
                {
                    if can_edit {
                        ui.label(&join.id.0);
                        changed |= text_edit(ui, "Name", &mut join.name);
                        ComboBox::from_label("Kind")
                            .selected_text(join_kind_label(join.kind))
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(
                                        &mut join.kind,
                                        WallJoinKind::Corner,
                                        "Corner",
                                    )
                                    .changed();
                                changed |= ui
                                    .selectable_value(
                                        &mut join.kind,
                                        WallJoinKind::EndToEnd,
                                        "End-to-end",
                                    )
                                    .changed();
                                changed |= ui
                                    .selectable_value(&mut join.kind, WallJoinKind::Tee, "Tee")
                                    .changed();
                                changed |= ui
                                    .selectable_value(&mut join.kind, WallJoinKind::Cross, "Cross")
                                    .changed();
                            });

                        let mut first_wall = join.first_wall.0.clone();
                        ComboBox::from_label("First wall")
                            .selected_text(wall_display_name(&wall_options, &first_wall))
                            .show_ui(ui, |ui| {
                                for (id, name) in &wall_options {
                                    ui.selectable_value(&mut first_wall, id.clone(), name);
                                }
                            });
                        if first_wall != join.first_wall.0 {
                            join.first_wall = ElementId::new(first_wall);
                            changed = true;
                        }

                        let mut second_wall = join.second_wall.0.clone();
                        ComboBox::from_label("Second wall")
                            .selected_text(wall_display_name(&wall_options, &second_wall))
                            .show_ui(ui, |ui| {
                                for (id, name) in &wall_options {
                                    ui.selectable_value(&mut second_wall, id.clone(), name);
                                }
                            });
                        if second_wall != join.second_wall.0 {
                            join.second_wall = ElementId::new(second_wall);
                            changed = true;
                        }

                        ui.separator();
                        ui.strong("Join point");
                        changed |= coordinate_drag(ui, "X", &mut join.point.x);
                        changed |= coordinate_drag(ui, "Y", &mut join.point.y);
                    } else {
                        join_summary(ui, join, &wall_options);
                    }
                } else {
                    ui.label("Wall join no longer exists");
                }
            }
            Selection::Member { wall_id, member_id } => {
                if let Some(member) = self.selected_member(&wall_id, &member_id) {
                    ui.label(format!("Wall: {wall_id}"));
                    member_inspector(ui, member);
                } else {
                    ui.label("Generated member no longer exists");
                }
            }
        }

        if changed {
            self.rebuild();
        }

        if self.workspace_mode.shows_generated_plan() {
            ui.separator();
            diagnostics_panel(ui, self.error.as_deref(), self.project_plan.as_ref());
            ui.separator();
            bom_panel(ui, self.project_plan.as_ref());
        } else if let Some(error) = self.error.as_deref() {
            ui.separator();
            panel_subheader(ui, "Validation");
            ui.colored_label(Color32::from_rgb(214, 104, 96), error);
        }
    }

    fn workspace_badge(&self) -> &'static str {
        match self.workspace_mode {
            WorkspaceMode::Design => "Design",
            WorkspaceMode::Plan => "Plan",
        }
    }
}

fn command_bar_frame() -> Frame {
    Frame::new()
        .fill(Color32::from_rgb(25, 27, 28))
        .inner_margin(Margin::symmetric(10, 6))
}

fn toolbar_group(ui: &mut Ui, label: &str, add_contents: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(label)
                .size(9.0)
                .strong()
                .color(toolbar_muted_text()),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 2.0);
            add_contents(ui);
        });
    });
}

fn command_button(ui: &mut Ui, label: &str, min_width: f32) -> Response {
    ui.add_sized(
        [min_width, 24.0],
        egui::Button::new(label)
            .fill(Color32::from_rgb(43, 46, 48))
            .stroke(Stroke::new(1.0, Color32::from_rgb(72, 78, 82)))
            .corner_radius(3),
    )
}

fn enabled_command_button(ui: &mut Ui, enabled: bool, label: &str, min_width: f32) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(label)
            .min_size(Vec2::new(min_width, 24.0))
            .fill(Color32::from_rgb(43, 46, 48))
            .stroke(Stroke::new(1.0, Color32::from_rgb(72, 78, 82)))
            .corner_radius(3),
    )
}

fn workspace_segment(ui: &mut Ui, mode: &mut WorkspaceMode, value: WorkspaceMode, label: &str) {
    if segment_button(ui, *mode == value, label, 68.0).clicked() {
        *mode = value;
    }
}

fn view_segment(ui: &mut Ui, mode: &mut ViewportMode, value: ViewportMode, label: &str) {
    if segment_button(ui, *mode == value, label, 70.0).clicked() {
        *mode = value;
    }
}

fn segment_button(ui: &mut Ui, selected: bool, label: &str, min_width: f32) -> Response {
    let fill = if selected {
        Color32::from_rgb(0, 114, 160)
    } else {
        Color32::from_rgb(37, 40, 42)
    };
    let stroke = if selected {
        Stroke::new(1.0, Color32::from_rgb(49, 176, 222))
    } else {
        Stroke::new(1.0, Color32::from_rgb(66, 71, 75))
    };
    let text = if selected {
        RichText::new(label)
            .strong()
            .color(Color32::from_rgb(248, 251, 252))
    } else {
        RichText::new(label).color(Color32::from_rgb(211, 216, 216))
    };

    ui.add_sized(
        [min_width, 24.0],
        egui::Button::new(text)
            .fill(fill)
            .stroke(stroke)
            .corner_radius(3),
    )
}

fn toolbar_divider(ui: &mut Ui) {
    ui.separator();
}

fn panel_header(ui: &mut Ui, title: &str, badge: &str) {
    Frame::new()
        .fill(Color32::from_rgb(31, 34, 35))
        .stroke(Stroke::new(1.0, Color32::from_rgb(55, 60, 62)))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(title)
                        .strong()
                        .size(15.0)
                        .color(Color32::from_rgb(232, 235, 234)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    status_chip(ui, badge, StatusTone::Neutral);
                });
            });
        });
    ui.add_space(6.0);
}

fn panel_subheader(ui: &mut Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(title)
            .strong()
            .size(12.0)
            .color(toolbar_muted_text()),
    );
    ui.add_space(2.0);
}

fn selection_badge(selection: &Selection) -> &'static str {
    match selection {
        Selection::Level(_) => "Level",
        Selection::Wall => "Wall",
        Selection::Opening(_) => "Opening",
        Selection::Dimension(_) => "Dimension",
        Selection::Join(_) => "Join",
        Selection::Member { .. } => "Member",
    }
}

#[derive(Clone, Copy)]
enum StatusTone {
    Neutral,
    Info,
    Success,
    Warning,
}

fn status_chip(ui: &mut Ui, text: &str, tone: StatusTone) {
    let (fill, stroke, text_color) = match tone {
        StatusTone::Neutral => (
            Color32::from_rgb(43, 47, 49),
            Color32::from_rgb(70, 76, 79),
            Color32::from_rgb(206, 212, 212),
        ),
        StatusTone::Info => (
            Color32::from_rgb(22, 61, 78),
            Color32::from_rgb(38, 118, 150),
            Color32::from_rgb(212, 238, 247),
        ),
        StatusTone::Success => (
            Color32::from_rgb(36, 70, 50),
            Color32::from_rgb(75, 136, 96),
            Color32::from_rgb(219, 242, 226),
        ),
        StatusTone::Warning => (
            Color32::from_rgb(82, 65, 30),
            Color32::from_rgb(154, 119, 49),
            Color32::from_rgb(248, 231, 190),
        ),
    };

    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(text_color));
        });
}

fn toolbar_muted_text() -> Color32 {
    Color32::from_rgb(150, 158, 158)
}

fn level_summary(ui: &mut Ui, level: &Level) {
    ui.label(&level.id.0);
    egui::Grid::new("level-summary")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Name", &level.name);
            summary_row(ui, "Elevation", level.elevation.to_string());
        });
}

fn wall_summary(ui: &mut Ui, wall: &Wall, level_options: &[(String, String)]) {
    ui.label(&wall.id.0);
    egui::Grid::new("wall-summary")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Name", &wall.name);
            summary_row(
                ui,
                "Level",
                level_display_name(level_options, &wall.level.0),
            );
            summary_row(ui, "Length", wall.length.to_string());
            summary_row(ui, "Height", wall.height.to_string());
            summary_row(ui, "Stud spacing", wall.stud_spacing.to_string());
            summary_row(ui, "Openings", wall.openings.len().to_string());
            summary_row(ui, "Start", format!("{}, {}", wall.start.x, wall.start.y));
            summary_row(ui, "End", format!("{}, {}", wall.end.x, wall.end.y));
        });
}

fn opening_summary(ui: &mut Ui, opening: &Opening) {
    ui.label(&opening.id.0);
    egui::Grid::new("opening-summary")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Name", &opening.name);
            summary_row(ui, "Kind", kind_label(opening.kind));
            summary_row(ui, "Center", opening.center.to_string());
            summary_row(ui, "Width", opening.width.to_string());
            summary_row(ui, "Height", opening.height.to_string());
            if opening.has_sill() {
                summary_row(ui, "Sill", opening.sill_height.to_string());
            }
        });
}

fn dimension_inspector(
    ui: &mut Ui,
    wall: &mut Wall,
    dimension_index: usize,
    remove: &mut bool,
) -> bool {
    let start_label = dimension_anchor_label(wall, &wall.dimensions[dimension_index].start);
    let end_label = dimension_anchor_label(wall, &wall.dimensions[dimension_index].end);
    let measured = wall
        .dimension_measurement(&wall.dimensions[dimension_index])
        .unwrap_or(Length::ZERO);
    let unsatisfied = wall.dimensions[dimension_index].kind == DimensionKind::Driving
        && !wall.is_driving_dimension_satisfied(&wall.dimensions[dimension_index]);
    let wall_length_inches = wall.length.inches().max(1.0);
    let mut changed = false;
    let mut apply_driving = false;

    {
        let dimension = &mut wall.dimensions[dimension_index];
        ui.label(&dimension.id.0);
        changed |= text_edit(ui, "Name", &mut dimension.name);

        let previous_kind = dimension.kind;
        ComboBox::from_label("Kind")
            .selected_text(dimension_kind_label(dimension.kind))
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut dimension.kind, DimensionKind::Driving, "Driving")
                    .changed();
                changed |= ui
                    .selectable_value(&mut dimension.kind, DimensionKind::Reference, "Reference")
                    .changed();
            });
        if dimension.kind != previous_kind {
            match dimension.kind {
                DimensionKind::Driving => {
                    dimension.value = Some(measured.max(Length::from_whole_inches(1)));
                    apply_driving = true;
                }
                DimensionKind::Reference => {
                    dimension.value = None;
                }
            }
        }

        egui::Grid::new("dimension-inspector")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                summary_row(ui, "From", &start_label);
                summary_row(ui, "To", &end_label);
                summary_row(ui, "Measured", measured.to_string());
            });

        if dimension.kind == DimensionKind::Driving {
            let mut value = dimension.value.unwrap_or(measured);
            if length_drag(ui, "Distance", &mut value, 1.0, wall_length_inches, "in") {
                dimension.value = Some(value);
                changed = true;
                apply_driving = true;
            }
            if unsatisfied {
                ui.colored_label(
                    Color32::from_rgb(214, 104, 96),
                    "Unsatisfied driving dimension",
                );
            }
        }

        ui.separator();
        if ui.button("Remove Dimension").clicked() {
            *remove = true;
        }
    }

    if apply_driving {
        let dimension = wall.dimensions[dimension_index].clone();
        if !wall.apply_driving_dimension(&dimension) {
            changed = true;
        }
    }

    changed
}

fn dimension_summary(ui: &mut Ui, wall: &Wall, dimension: &DimensionConstraint) {
    ui.label(&dimension.id.0);
    let measured = wall
        .dimension_measurement(dimension)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unresolved".to_owned());
    egui::Grid::new("dimension-summary")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Name", &dimension.name);
            summary_row(ui, "Kind", dimension_kind_label(dimension.kind));
            summary_row(ui, "From", dimension_anchor_label(wall, &dimension.start));
            summary_row(ui, "To", dimension_anchor_label(wall, &dimension.end));
            summary_row(ui, "Measured", measured);
            if let Some(value) = dimension.value {
                summary_row(ui, "Target", value.to_string());
            }
            if dimension.kind == DimensionKind::Driving
                && !wall.is_driving_dimension_satisfied(dimension)
            {
                summary_row(ui, "Status", "Unsatisfied");
            }
        });
}

fn join_summary(ui: &mut Ui, join: &WallJoin, wall_options: &[(String, String)]) {
    ui.label(&join.id.0);
    egui::Grid::new("join-summary")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Name", &join.name);
            summary_row(ui, "Kind", join_kind_label(join.kind));
            summary_row(
                ui,
                "First wall",
                wall_display_name(wall_options, &join.first_wall.0),
            );
            summary_row(
                ui,
                "Second wall",
                wall_display_name(wall_options, &join.second_wall.0),
            );
            summary_row(ui, "Point", format!("{}, {}", join.point.x, join.point.y));
        });
}

fn summary_row(ui: &mut Ui, label: &str, value: impl ToString) {
    ui.strong(label);
    ui.label(value.to_string());
    ui.end_row();
}

fn member_inspector(ui: &mut Ui, member: &FrameMember) {
    ui.label(&member.id);
    egui::Grid::new("member-inspector")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.strong("Use");
            ui.label(member.kind.label());
            ui.end_row();
            ui.strong("Profile");
            ui.label(member.profile.label());
            ui.end_row();
            ui.strong("Source");
            ui.label(&member.source.0);
            ui.end_row();
            ui.strong("X");
            ui.label(member.x.to_string());
            ui.end_row();
            ui.strong("Elevation");
            ui.label(member.elevation.to_string());
            ui.end_row();
            ui.strong("Cut length");
            ui.label(member.cut_length.to_string());
            ui.end_row();
            ui.strong("Drawn depth");
            ui.label(member.cross_section_depth.to_string());
            ui.end_row();
            ui.strong("Rule");
            ui.label(&member.provenance.rule_id);
            ui.end_row();
        });
    ui.label(&member.provenance.summary);
}

fn diagnostics_panel(ui: &mut Ui, error: Option<&str>, plan: Option<&ProjectFramePlan>) {
    panel_subheader(ui, "Diagnostics");
    if let Some(error) = error {
        ui.colored_label(Color32::from_rgb(214, 104, 96), error);
    }

    if let Some(plan) = plan {
        let diagnostics = plan
            .diagnostics
            .iter()
            .chain(
                plan.wall_plans
                    .iter()
                    .flat_map(|wall_plan| wall_plan.diagnostics.iter()),
            )
            .collect::<Vec<_>>();

        if diagnostics.is_empty() {
            ui.label("No diagnostics");
            return;
        }

        let unsupported = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Unsupported)
            .count();
        let warnings = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
            .count();
        let info = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Info)
            .count();

        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{unsupported} unsupported"));
            ui.label(format!("{warnings} warnings"));
            ui.label(format!("{info} info"));
        });

        for diagnostic in diagnostics.iter().take(5) {
            diagnostic_row(ui, diagnostic);
        }

        if diagnostics.len() > 5 {
            egui::CollapsingHeader::new(format!("{} more diagnostics", diagnostics.len() - 5))
                .default_open(false)
                .show(ui, |ui| {
                    for diagnostic in diagnostics.iter().skip(5) {
                        diagnostic_row(ui, diagnostic);
                    }
                });
        }
    } else if error.is_none() {
        ui.label("No diagnostics");
    }
}

fn diagnostic_row(ui: &mut Ui, diagnostic: &PlanDiagnostic) {
    let color = match diagnostic.severity {
        DiagnosticSeverity::Info => Color32::from_rgb(74, 92, 112),
        DiagnosticSeverity::Warning => Color32::from_rgb(150, 95, 30),
        DiagnosticSeverity::Unsupported => Color32::from_rgb(155, 60, 58),
    };
    ui.colored_label(
        color,
        format!(
            "{} {}",
            diagnostic_code_prefix(diagnostic.severity),
            diagnostic.code
        ),
    );
    if let Some(source) = &diagnostic.source {
        ui.small(source.0.as_str());
    }
    ui.label(&diagnostic.message);
}

fn bom_panel(ui: &mut Ui, plan: Option<&ProjectFramePlan>) {
    panel_subheader(ui, "BOM");
    if let Some(plan) = plan {
        egui::Grid::new("bom-grid")
            .num_columns(5)
            .spacing([12.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Qty");
                ui.strong("Profile");
                ui.strong("Cut");
                ui.strong("Total");
                ui.strong("Use");
                ui.end_row();

                for item in plan.bom() {
                    ui.label(item.quantity.to_string());
                    ui.label(item.profile.label());
                    ui.label(item.cut_length.to_string());
                    ui.label(item.total_length.to_string());
                    ui.label(item.kind.label());
                    ui.end_row();
                }
            });
    }
}

fn text_edit(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        changed = ui.text_edit_singleline(value).changed();
    });
    changed
}

fn level_display_name(options: &[(String, String)], id: &str) -> String {
    options
        .iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| id.to_owned())
}

fn wall_display_name(options: &[(String, String)], id: &str) -> String {
    options
        .iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| id.to_owned())
}

fn dimension_anchor_label(wall: &Wall, anchor: &DimensionAnchor) -> String {
    match anchor {
        DimensionAnchor::WallStart => "Wall start".to_owned(),
        DimensionAnchor::WallEnd => "Wall end".to_owned(),
        DimensionAnchor::OpeningLeft { opening } => {
            format!("{} left", opening_display_name(wall, &opening.0))
        }
        DimensionAnchor::OpeningCenter { opening } => {
            format!("{} center", opening_display_name(wall, &opening.0))
        }
        DimensionAnchor::OpeningRight { opening } => {
            format!("{} right", opening_display_name(wall, &opening.0))
        }
    }
}

fn opening_display_name(wall: &Wall, id: &str) -> String {
    wall.openings
        .iter()
        .find(|opening| opening.id.0 == id)
        .map(|opening| opening.name.clone())
        .unwrap_or_else(|| id.to_owned())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct OpeningDrivenFields {
    center: Option<DrivenField>,
    width: Option<DrivenField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DrivenField {
    dimension_ids: Vec<String>,
    labels: Vec<String>,
}

impl DrivenField {
    fn hover_text(&self) -> String {
        if self.labels.is_empty() {
            return "Read-only: driven by active dimensions".to_owned();
        }

        let mut lines = vec!["Read-only: driven by dimensions".to_owned()];
        lines.push("Open the Driven menu to choose a source dimension".to_owned());
        if !self.dimension_ids.is_empty() {
            lines.push("Cmd/Ctrl-click the field to select the first driver".to_owned());
        }
        lines.extend(self.labels.iter().map(|label| format!("- {label}")));
        lines.join("\n")
    }

    fn first_dimension_id(&self) -> Option<&str> {
        self.dimension_ids.first().map(String::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DimensionVariableKey {
    WallLength,
    OpeningCenter(String),
    OpeningWidth(String),
}

fn opening_driven_fields(wall: &Wall, opening_id: &str) -> OpeningDrivenFields {
    OpeningDrivenFields {
        center: driven_length_field(
            wall,
            DimensionVariableKey::OpeningCenter(opening_id.to_owned()),
        ),
        width: driven_length_field(
            wall,
            DimensionVariableKey::OpeningWidth(opening_id.to_owned()),
        ),
    }
}

fn driven_length_field(wall: &Wall, key: DimensionVariableKey) -> Option<DrivenField> {
    dimension_variable_is_driven(wall, &key).then(|| driven_field_sources(wall, key))
}

fn dimension_variable_is_driven(wall: &Wall, key: &DimensionVariableKey) -> bool {
    if wall
        .dimensions
        .iter()
        .all(|dimension| dimension.kind != DimensionKind::Driving)
    {
        return false;
    }

    let Some(probe_value) = dimension_variable_probe_value(wall, key) else {
        return false;
    };
    let mut probed_wall = wall.clone();
    if !set_dimension_variable_value(&mut probed_wall, key, probe_value) {
        return false;
    }

    probed_wall.apply_driving_dimensions();
    dimension_variable_value(&probed_wall, key)
        .is_some_and(|solved_value| solved_value != probe_value)
}

fn dimension_variable_probe_value(wall: &Wall, key: &DimensionVariableKey) -> Option<Length> {
    let value = dimension_variable_value(wall, key)?;
    let step = Length::from_whole_inches(12);

    match key {
        DimensionVariableKey::WallLength => {
            alternate_length(value, Length::from_whole_inches(1), value + step, step)
        }
        DimensionVariableKey::OpeningCenter(opening_id) => {
            let opening = wall
                .openings
                .iter()
                .find(|opening| opening.id.0 == *opening_id)?;
            let half_width = opening.width / 2;
            alternate_length(value, half_width, wall.length - half_width, step)
        }
        DimensionVariableKey::OpeningWidth(opening_id) => {
            let opening = wall
                .openings
                .iter()
                .find(|opening| opening.id.0 == *opening_id)?;
            let max_width = opening.center.min(wall.length - opening.center) * 2;
            alternate_length(value, Length::from_whole_inches(12), max_width, step)
        }
    }
}

fn dimension_variable_value(wall: &Wall, key: &DimensionVariableKey) -> Option<Length> {
    match key {
        DimensionVariableKey::WallLength => Some(wall.length),
        DimensionVariableKey::OpeningCenter(opening_id) => wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == *opening_id)
            .map(|opening| opening.center),
        DimensionVariableKey::OpeningWidth(opening_id) => wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == *opening_id)
            .map(|opening| opening.width),
    }
}

fn set_dimension_variable_value(
    wall: &mut Wall,
    key: &DimensionVariableKey,
    value: Length,
) -> bool {
    match key {
        DimensionVariableKey::WallLength => {
            set_wall_length_keep_direction(wall, value);
            true
        }
        DimensionVariableKey::OpeningCenter(opening_id) => {
            let Some(opening) = wall
                .openings
                .iter_mut()
                .find(|opening| opening.id.0 == *opening_id)
            else {
                return false;
            };
            opening.center = value;
            true
        }
        DimensionVariableKey::OpeningWidth(opening_id) => {
            let Some(opening) = wall
                .openings
                .iter_mut()
                .find(|opening| opening.id.0 == *opening_id)
            else {
                return false;
            };
            opening.width = value;
            true
        }
    }
}

fn alternate_length(value: Length, min: Length, max: Length, step: Length) -> Option<Length> {
    if max < min {
        return None;
    }

    let larger = (value + step).min(max);
    if larger != value {
        return Some(larger);
    }

    let smaller = (value - step).max(min);
    (smaller != value).then_some(smaller)
}

fn driven_field_sources(wall: &Wall, target: DimensionVariableKey) -> DrivenField {
    let mut variables = BTreeSet::from([target]);
    let mut dimension_indices = BTreeSet::new();
    let mut changed = true;

    while changed {
        changed = false;
        for (index, dimension) in wall.dimensions.iter().enumerate() {
            if dimension.kind != DimensionKind::Driving {
                continue;
            }

            let dimension_variables = dimension_constraint_variables(dimension);
            if dimension_variables
                .iter()
                .any(|variable| variables.contains(variable))
            {
                changed |= dimension_indices.insert(index);
                for variable in dimension_variables {
                    changed |= variables.insert(variable);
                }
            }
        }
    }

    let dimensions = dimension_indices
        .into_iter()
        .map(|index| &wall.dimensions[index])
        .collect::<Vec<_>>();

    DrivenField {
        dimension_ids: dimensions
            .iter()
            .map(|dimension| dimension.id.0.clone())
            .collect(),
        labels: dimensions
            .iter()
            .map(|dimension| driving_dimension_source_label(wall, dimension))
            .collect(),
    }
}

fn dimension_constraint_variables(
    dimension: &DimensionConstraint,
) -> BTreeSet<DimensionVariableKey> {
    let mut variables = BTreeSet::new();
    add_anchor_variables(&dimension.start, &mut variables);
    add_anchor_variables(&dimension.end, &mut variables);
    variables
}

fn add_anchor_variables(anchor: &DimensionAnchor, variables: &mut BTreeSet<DimensionVariableKey>) {
    match anchor {
        DimensionAnchor::WallStart => {}
        DimensionAnchor::WallEnd => {
            variables.insert(DimensionVariableKey::WallLength);
        }
        DimensionAnchor::OpeningCenter { opening } => {
            variables.insert(DimensionVariableKey::OpeningCenter(opening.0.clone()));
        }
        DimensionAnchor::OpeningLeft { opening } | DimensionAnchor::OpeningRight { opening } => {
            variables.insert(DimensionVariableKey::OpeningCenter(opening.0.clone()));
            variables.insert(DimensionVariableKey::OpeningWidth(opening.0.clone()));
        }
    }
}

fn driving_dimension_source_label(wall: &Wall, dimension: &DimensionConstraint) -> String {
    let mut label = format!(
        "{}: {} to {}",
        dimension.name,
        dimension_anchor_label(wall, &dimension.start),
        dimension_anchor_label(wall, &dimension.end)
    );
    if let Some(value) = dimension.value {
        label.push_str(&format!(" = {value}"));
    }
    label
}

#[derive(Debug, Clone, Copy)]
struct LengthDragSpec {
    min_inches: f64,
    max_inches: f64,
    display_unit: &'static str,
}

fn length_drag_spec(
    min_inches: f64,
    max_inches: f64,
    display_unit: &'static str,
) -> LengthDragSpec {
    LengthDragSpec {
        min_inches,
        max_inches,
        display_unit,
    }
}

fn driven_length_drag(
    ui: &mut Ui,
    label: &str,
    value: &mut Length,
    spec: LengthDragSpec,
    driver: Option<&DrivenField>,
    select_dimension: &mut Option<String>,
) -> bool {
    if let Some(driver) = driver {
        readonly_length_field(
            ui,
            label,
            *value,
            spec.display_unit,
            driver,
            select_dimension,
        );
        false
    } else {
        length_drag(
            ui,
            label,
            value,
            spec.min_inches,
            spec.max_inches,
            spec.display_unit,
        )
    }
}

fn readonly_length_field(
    ui: &mut Ui,
    label: &str,
    value: Length,
    display_unit: &str,
    driver: &DrivenField,
    select_dimension: &mut Option<String>,
) {
    let mut display_value = if display_unit == "ft" {
        value.feet()
    } else {
        value.inches()
    };
    let hover_text = driver.hover_text();

    ui.horizontal(|ui| {
        ui.label(label);
        let value_response = ui.add_enabled(
            false,
            egui::DragValue::new(&mut display_value)
                .speed(if display_unit == "ft" { 0.25 } else { 1.0 })
                .suffix(format!(" {display_unit}")),
        );
        let value_response = ui
            .interact(
                value_response.rect,
                value_response.id.with("driver-shortcut"),
                egui::Sense::click(),
            )
            .on_hover_text(hover_text.clone());
        select_first_driver_on_shortcut(ui, &value_response, driver, select_dimension);

        let chip_response =
            driven_field_menu(ui, driver, select_dimension).on_hover_text(hover_text);
        select_first_driver_on_shortcut(ui, &chip_response, driver, select_dimension);
    });
}

fn driven_field_menu(
    ui: &mut Ui,
    driver: &DrivenField,
    select_dimension: &mut Option<String>,
) -> Response {
    let label = if driver.dimension_ids.len() > 1 {
        format!("Driven ({})", driver.dimension_ids.len())
    } else {
        "Driven".to_owned()
    };

    let menu = ui.menu_button(
        RichText::new(label)
            .size(11.0)
            .color(Color32::from_rgb(248, 231, 190)),
        |ui| {
            ui.set_min_width(220.0);
            if driver.dimension_ids.is_empty() {
                ui.add_enabled(false, egui::Button::new("No active dimensions"));
                return;
            }

            for (dimension_id, label) in driver.dimension_ids.iter().zip(driver.labels.iter()) {
                if ui.button(label).on_hover_text(dimension_id).clicked() {
                    *select_dimension = Some(dimension_id.clone());
                    ui.close();
                }
            }
        },
    );
    menu.response
}

fn select_first_driver_on_shortcut(
    ui: &Ui,
    response: &Response,
    driver: &DrivenField,
    select_dimension: &mut Option<String>,
) {
    let modifier_pressed = ui.input(|input| input.modifiers.command || input.modifiers.ctrl);
    if response.clicked()
        && modifier_pressed
        && let Some(dimension_id) = driver.first_dimension_id()
    {
        *select_dimension = Some(dimension_id.to_owned());
    }
}

fn length_drag(
    ui: &mut Ui,
    label: &str,
    value: &mut Length,
    min_inches: f64,
    max_inches: f64,
    display_unit: &str,
) -> bool {
    let mut display_value = if display_unit == "ft" {
        value.feet()
    } else {
        value.inches()
    };

    let response = ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::DragValue::new(&mut display_value)
                .range(if display_unit == "ft" {
                    min_inches / 12.0..=max_inches / 12.0
                } else {
                    min_inches..=max_inches
                })
                .speed(if display_unit == "ft" { 0.25 } else { 1.0 })
                .suffix(format!(" {display_unit}")),
        )
    });

    if response.inner.changed() {
        let next_inches = if display_unit == "ft" {
            display_value * 12.0
        } else {
            display_value
        };
        *value = Length::from_inches(next_inches.clamp(min_inches, max_inches));
        true
    } else {
        false
    }
}

fn coordinate_drag(ui: &mut Ui, label: &str, value: &mut Length) -> bool {
    let mut display_value = value.feet();
    let response = ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::DragValue::new(&mut display_value)
                .range(-240.0..=240.0)
                .speed(0.25)
                .suffix(" ft"),
        )
    });

    if response.inner.changed() {
        *value = Length::from_feet(display_value.clamp(-240.0, 240.0));
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use framer_core::{CodeProfile, DimensionDirection};

    #[derive(Debug, Clone, Copy)]
    enum WindowAnchor {
        Left,
        Center,
        Right,
    }

    fn wall_with_window(center: Length, width: Length) -> Wall {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            center,
            width,
            Length::from_feet(3.0),
            Length::from_feet(3.0),
        ));
        wall
    }

    fn window_anchor(anchor: WindowAnchor) -> DimensionAnchor {
        let opening = ElementId::new("window");
        match anchor {
            WindowAnchor::Left => DimensionAnchor::OpeningLeft { opening },
            WindowAnchor::Center => DimensionAnchor::OpeningCenter { opening },
            WindowAnchor::Right => DimensionAnchor::OpeningRight { opening },
        }
    }

    fn driving_dimension(
        id: &str,
        start: DimensionAnchor,
        end: DimensionAnchor,
        value: Length,
    ) -> DimensionConstraint {
        DimensionConstraint::new(
            id,
            id.replace('-', " "),
            DimensionKind::Driving,
            start,
            end,
            DimensionDirection::Forward,
            Some(value),
        )
    }

    #[test]
    fn wall_length_dimension_marks_wall_length_driven() {
        let mut wall = wall_with_window(Length::from_feet(6.0), Length::from_feet(3.0));
        wall.dimensions.push(driving_dimension(
            "wall-length",
            DimensionAnchor::WallStart,
            DimensionAnchor::WallEnd,
            Length::from_feet(12.0),
        ));

        let driver = driven_length_field(&wall, DimensionVariableKey::WallLength);

        assert_eq!(
            driver.map(|driver| driver.dimension_ids),
            Some(vec!["wall-length".to_owned()])
        );
    }

    #[test]
    fn paired_edge_offsets_mark_opening_center_and_width_driven() {
        let mut wall = wall_with_window(Length::from_feet(6.0), Length::from_feet(3.0));
        wall.dimensions.push(driving_dimension(
            "left-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Left),
            Length::from_feet(5.0),
        ));
        wall.dimensions.push(driving_dimension(
            "right-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Right),
            Length::from_feet(10.0),
        ));
        wall.apply_driving_dimensions();

        let fields = opening_driven_fields(&wall, "window");

        assert_eq!(
            fields
                .center
                .as_ref()
                .map(|driver| driver.dimension_ids.clone()),
            Some(vec!["left-offset".to_owned(), "right-offset".to_owned()])
        );
        assert_eq!(
            fields
                .width
                .as_ref()
                .map(|driver| driver.dimension_ids.clone()),
            Some(vec!["left-offset".to_owned(), "right-offset".to_owned()])
        );
    }

    #[test]
    fn single_edge_offset_leaves_width_editable() {
        let mut wall = wall_with_window(Length::from_feet(6.0), Length::from_feet(3.0));
        wall.dimensions.push(driving_dimension(
            "left-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Left),
            Length::from_feet(5.0),
        ));
        wall.apply_driving_dimensions();

        let fields = opening_driven_fields(&wall, "window");

        assert_eq!(
            fields
                .center
                .as_ref()
                .map(|driver| driver.dimension_ids.clone()),
            Some(vec!["left-offset".to_owned()])
        );
        assert!(fields.width.is_none());
    }

    #[test]
    fn width_driver_includes_indirect_center_dimension() {
        let mut wall = wall_with_window(Length::from_feet(6.0), Length::from_feet(3.0));
        wall.dimensions.push(driving_dimension(
            "center",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Center),
            Length::from_feet(7.0),
        ));
        wall.dimensions.push(driving_dimension(
            "right-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Right),
            Length::from_feet(9.0),
        ));
        wall.apply_driving_dimensions();

        let fields = opening_driven_fields(&wall, "window");

        assert_eq!(
            fields
                .width
                .as_ref()
                .map(|driver| driver.dimension_ids.clone()),
            Some(vec!["center".to_owned(), "right-offset".to_owned()])
        );
    }
}
