use eframe::egui::{self, Color32, ComboBox, ScrollArea, Ui};
use framer_core::{ElementId, Length, OpeningKind, WallJoinKind};
use framer_solver::{DiagnosticSeverity, FrameMember, PlanDiagnostic, ProjectFramePlan};

use super::labels::{diagnostic_code_prefix, join_kind_label, kind_label};
use super::model_edit::set_wall_length_keep_direction;
use super::{FramerApp, Selection, ViewportMode};

impl FramerApp {
    pub(super) fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("Framer");
            ui.separator();
            if ui.button("New").clicked() {
                self.new_project();
            }
            if ui.button("Shell Demo").clicked() {
                self.reset_demo();
            }
            if ui.button("Wall Demo").clicked() {
                self.reset_wall_demo();
            }
            if ui.button("Open").clicked() {
                self.load_project_file();
            }
            if ui.button("Save").clicked() {
                self.save_project_file();
            }
            if ui.button("Export").clicked() {
                self.export_current_artifacts();
            }
            ui.add(egui::TextEdit::singleline(&mut self.project_path).desired_width(340.0));
            ui.separator();
            ui.selectable_value(&mut self.viewport_mode, ViewportMode::Plan, "Plan");
            ui.selectable_value(
                &mut self.viewport_mode,
                ViewportMode::Elevation,
                "Elevation",
            );
            ui.selectable_value(&mut self.viewport_mode, ViewportMode::Axonometric, "3D");
            ui.checkbox(&mut self.show_section, "Section");
        });

        if self.file_status.is_some() || self.artifact_status.is_some() {
            ui.horizontal_wrapped(|ui| {
                if let Some(status) = &self.file_status {
                    ui.label(status);
                }
                if let Some(status) = &self.artifact_status {
                    ui.label(status);
                }
            });
        }
    }

    pub(super) fn model_tree(&mut self, ui: &mut Ui) {
        ui.heading("Model Tree");
        ui.separator();

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
                            for (index, wall_id, wall_name, wall_level, openings) in &walls {
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
                .default_open(false)
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

            ui.separator();
            ui.heading("Catalog");
            if ui.button("+ Door").clicked() {
                self.add_opening(OpeningKind::Door);
            }
            if ui.button("+ Window").clicked() {
                self.add_opening(OpeningKind::Window);
            }
            if ui.button("+ Garage Door").clicked() {
                self.add_opening(OpeningKind::GarageDoor);
            }
        });
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui) {
        let mut changed = false;
        let selection = self.selected.clone();
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

        ui.heading("Inspector");
        ui.separator();

        match selection {
            Selection::Level(id) => {
                if let Some(level) = self.model.levels.iter_mut().find(|level| level.id.0 == id) {
                    ui.label(&level.id.0);
                    changed |= text_edit(ui, "Name", &mut level.name);
                    changed |= coordinate_drag(ui, "Elevation", &mut level.elevation);
                } else {
                    ui.label("Level no longer exists");
                }
            }
            Selection::Wall => {
                if let Some(wall) = self.model.walls.get_mut(self.selected_wall) {
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
                    if length_drag(ui, "Length", &mut wall_length, 24.0, 480.0, "ft") {
                        set_wall_length_keep_direction(wall, wall_length);
                        changed = true;
                    }
                    changed |= length_drag(ui, "Height", &mut wall.height, 48.0, 168.0, "ft");
                    changed |=
                        length_drag(ui, "Stud spacing", &mut wall.stud_spacing, 8.0, 32.0, "in");
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
                }
            }
            Selection::Opening(id) => {
                if let Some(wall) = self.model.walls.get_mut(self.selected_wall) {
                    let mut remove = false;
                    if let Some(opening) =
                        wall.openings.iter_mut().find(|opening| opening.id.0 == id)
                    {
                        ui.label(&opening.id.0);
                        changed |= text_edit(ui, "Name", &mut opening.name);
                        ComboBox::from_label("Kind")
                            .selected_text(kind_label(opening.kind))
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(&mut opening.kind, OpeningKind::Door, "Door")
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
                        changed |= length_drag(ui, "Center", &mut opening.center, 0.0, 480.0, "ft");
                        changed |= length_drag(ui, "Width", &mut opening.width, 12.0, 240.0, "in");
                        changed |=
                            length_drag(ui, "Height", &mut opening.height, 12.0, 120.0, "in");
                        if opening.has_sill() {
                            changed |=
                                length_drag(ui, "Sill", &mut opening.sill_height, 0.0, 96.0, "in");
                        } else if opening.sill_height != Length::ZERO {
                            opening.sill_height = Length::ZERO;
                            changed = true;
                        }

                        ui.separator();
                        if ui.button("Remove Opening").clicked() {
                            remove = true;
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
            }
            Selection::Join(id) => {
                if let Some(join) = self
                    .model
                    .wall_joins
                    .iter_mut()
                    .find(|join| join.id.0 == id)
                {
                    ui.label(&join.id.0);
                    changed |= text_edit(ui, "Name", &mut join.name);
                    ComboBox::from_label("Kind")
                        .selected_text(join_kind_label(join.kind))
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(&mut join.kind, WallJoinKind::Corner, "Corner")
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

        ui.separator();
        diagnostics_panel(ui, self.error.as_deref(), self.project_plan.as_ref());
        ui.separator();
        bom_panel(ui, self.project_plan.as_ref());
    }
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
    ui.heading("Diagnostics");
    if let Some(error) = error {
        ui.colored_label(Color32::from_rgb(185, 65, 65), error);
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
    ui.heading("BOM");
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
