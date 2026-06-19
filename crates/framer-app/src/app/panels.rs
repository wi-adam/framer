use std::collections::BTreeSet;

use eframe::egui::{
    self, Align, Color32, ComboBox, Frame, Layout, Margin, PopupCloseBehavior, Response, RichText,
    ScrollArea, Stroke, Ui, Vec2,
    containers::menu::{MenuButton, MenuConfig},
};
use framer_core::{
    DimensionAnchor, DimensionAxis, DimensionConstraint, DimensionHorizontalReference,
    DimensionKind, DimensionVerticalReference, ElementId, Length, Level, Opening, OpeningKind,
    Wall, WallJoin, WallJoinKind,
};
use framer_solver::{DiagnosticSeverity, FrameMember, PlanDiagnostic, ProjectFramePlan};

use super::design::{Icon, widgets};
use super::labels::{
    diagnostic_code_prefix, dimension_axis_label, dimension_kind_label, join_kind_label, kind_label,
};
use super::model_edit::{
    opening_max_bottom, opening_top_clearance, set_wall_length_keep_direction,
};
use super::{
    DrawWallToolState, FramerApp, Selection, ViewportMode, WallDisplay, WorkspaceMode, design,
    theme,
};

impl FramerApp {
    pub(super) fn app_header(&mut self, ui: &mut Ui) {
        let head = design::studio_dark();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = design::space::MD;
            ui.add_space(design::space::SM);
            ui.label(
                RichText::new("Framer")
                    .strong()
                    .size(design::text_size::TITLE)
                    .color(head.text),
            );
            header_divider(ui, head.divider);
            ui.label(
                RichText::new("Project")
                    .size(design::text_size::LABEL)
                    .color(head.text_muted),
            );
            let path_width = (ui.available_width() * 0.40).clamp(240.0, 520.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.project_path)
                    .desired_width(path_width)
                    .font(egui::TextStyle::Monospace),
            );
            header_save_pill(ui, head, self.file_status.as_deref());

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(design::space::SM);
                let theme_icon = if design::active().dark {
                    Icon::ThemeDark
                } else {
                    Icon::ThemeLight
                };
                if widgets::ghost_icon_button(
                    ui,
                    theme_icon,
                    head.text,
                    "Toggle light / dark theme",
                )
                .clicked()
                {
                    design::toggle_theme(ui.ctx());
                }
                widgets::ghost_icon_button(ui, Icon::Help, head.text_secondary, "Framer help");
                header_profile(ui, head, self.model.code.display_name.as_str());
            });
        });
    }

    pub(super) fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(design::space::SM, design::space::SM);

            widgets::tool_group(ui, "PROJECT", |ui| {
                if widgets::tool_button(ui, Icon::New, "New", false, true)
                    .on_hover_text("Start an empty wall project")
                    .clicked()
                {
                    self.new_project();
                }
                if widgets::tool_button(ui, Icon::Open, "Open", false, true)
                    .on_hover_text("Open the project path")
                    .clicked()
                {
                    self.load_project_file();
                }
                if widgets::tool_button(ui, Icon::Save, "Save", false, true)
                    .on_hover_text("Save the current model")
                    .clicked()
                {
                    self.save_project_file();
                }
                let can_export = self.workspace_mode.shows_generated_plan();
                if widgets::tool_button(ui, Icon::Export, "Export", false, can_export)
                    .on_hover_text("Export plan artifacts from Plan workspace")
                    .clicked()
                {
                    self.export_current_artifacts();
                }
            });
            widgets::tool_divider(ui);

            widgets::tool_group(ui, "EDIT", |ui| {
                let undo_tip = match self.history.undo_label() {
                    Some(label) => format!("Undo {label}  (⌘Z / Ctrl+Z)"),
                    None => "Nothing to undo  (⌘Z / Ctrl+Z)".to_owned(),
                };
                if widgets::tool_button(ui, Icon::Undo, "Undo", false, self.history.can_undo())
                    .on_hover_text(undo_tip)
                    .clicked()
                {
                    self.undo();
                }
                let redo_tip = match self.history.redo_label() {
                    Some(label) => format!("Redo {label}  (⌘⇧Z / Ctrl+Y)"),
                    None => "Nothing to redo  (⌘⇧Z / Ctrl+Y)".to_owned(),
                };
                if widgets::tool_button(ui, Icon::Redo, "Redo", false, self.history.can_redo())
                    .on_hover_text(redo_tip)
                    .clicked()
                {
                    self.redo();
                }
            });
            widgets::tool_divider(ui);

            widgets::tool_group(ui, "SAMPLES", |ui| {
                if widgets::tool_button(ui, Icon::Shell, "Shell", false, true)
                    .on_hover_text("Load the multi-wall shell demo")
                    .clicked()
                {
                    self.reset_demo();
                }
                if widgets::tool_button(ui, Icon::Wall, "Wall", false, true)
                    .on_hover_text("Load the single-wall demo")
                    .clicked()
                {
                    self.reset_wall_demo();
                }
            });
            widgets::tool_divider(ui);

            widgets::tool_group(ui, "WORKSPACE", |ui| {
                if widgets::tool_button(
                    ui,
                    Icon::Design,
                    "Design",
                    self.workspace_mode == WorkspaceMode::Design,
                    true,
                )
                .clicked()
                {
                    self.set_workspace_mode(WorkspaceMode::Design);
                }
                if widgets::tool_button(
                    ui,
                    Icon::Plan,
                    "Plan",
                    self.workspace_mode == WorkspaceMode::Plan,
                    true,
                )
                .clicked()
                {
                    self.set_workspace_mode(WorkspaceMode::Plan);
                }
            });
            widgets::tool_divider(ui);

            widgets::tool_group(ui, "VIEW", |ui| {
                let design_mode = self.workspace_mode.allows_design_edits();
                let shell_label = if design_mode { "Shell" } else { "Plan" };
                let wall_label = if design_mode { "Wall" } else { "Elevation" };
                if widgets::tool_button(
                    ui,
                    Icon::Shell,
                    shell_label,
                    self.viewport_mode == ViewportMode::Plan,
                    true,
                )
                .clicked()
                {
                    self.viewport_mode = ViewportMode::Plan;
                }
                if widgets::tool_button(
                    ui,
                    Icon::Wall,
                    wall_label,
                    self.viewport_mode == ViewportMode::Elevation,
                    true,
                )
                .clicked()
                {
                    self.viewport_mode = ViewportMode::Elevation;
                }
                if widgets::tool_button(
                    ui,
                    Icon::View3d,
                    "3D",
                    self.viewport_mode == ViewportMode::Axonometric,
                    true,
                )
                .clicked()
                {
                    self.viewport_mode = ViewportMode::Axonometric;
                }
                if widgets::tool_button(
                    ui,
                    Icon::ThemeLight,
                    "Render",
                    self.viewport_mode == ViewportMode::Render,
                    true,
                )
                .clicked()
                {
                    self.viewport_mode = ViewportMode::Render;
                }
            });

            if self.workspace_mode.allows_design_edits() {
                widgets::tool_divider(ui);
                widgets::tool_group(ui, "BUILD", |ui| {
                    if widgets::tool_button(
                        ui,
                        Icon::Wall,
                        "Wall",
                        self.draw_wall_tool.active,
                        true,
                    )
                    .on_hover_text("Draw walls in the plan view (W)")
                    .clicked()
                    {
                        self.toggle_draw_wall_tool();
                    }
                    if widgets::tool_button(ui, Icon::Shell, "Room", self.room_tool_active, true)
                        .on_hover_text("Place a room inside an enclosed area (R)")
                        .clicked()
                    {
                        self.toggle_room_tool();
                    }
                    let can_delete = matches!(
                        self.selected,
                        Selection::Wall | Selection::Opening(_) | Selection::Room(_)
                    );
                    if widgets::tool_button(ui, Icon::Delete, "Delete", false, can_delete)
                        .on_hover_text("Delete the selected wall or opening (Del)")
                        .clicked()
                    {
                        self.delete_selected();
                    }
                    if widgets::tool_button(ui, Icon::Door, "Door", false, true)
                        .on_hover_text("Add a door to the selected wall")
                        .clicked()
                    {
                        self.add_opening(OpeningKind::Door);
                    }
                    if widgets::tool_button(ui, Icon::Window, "Window", false, true)
                        .on_hover_text("Add a window to the selected wall")
                        .clicked()
                    {
                        self.add_opening(OpeningKind::Window);
                    }
                    if widgets::tool_button(ui, Icon::GarageDoor, "Garage", false, true)
                        .on_hover_text("Add a garage door to the selected wall")
                        .clicked()
                    {
                        self.add_opening(OpeningKind::GarageDoor);
                    }
                });
                widgets::tool_divider(ui);
                widgets::tool_group(ui, "DIMENSION", |ui| {
                    if widgets::tool_button(
                        ui,
                        Icon::Linear,
                        "Linear",
                        self.dimension_tool.active,
                        true,
                    )
                    .on_hover_text("Place a wall dimension (D)")
                    .clicked()
                    {
                        self.toggle_dimension_tool();
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
                        ComboBox::from_id_salt("dimension-tool-axis")
                            .selected_text(dimension_axis_label(self.dimension_tool.axis))
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
                            });
                    }
                });
            } else {
                widgets::tool_divider(ui);
                widgets::tool_group(ui, "TOOLS", |ui| {
                    if widgets::tool_button(
                        ui,
                        Icon::LayoutColumns,
                        "Section",
                        self.show_section,
                        true,
                    )
                    .on_hover_text("Toggle the wall section preview")
                    .clicked()
                    {
                        self.show_section = !self.show_section;
                    }
                });
            }
        });

        if self.file_status.is_some()
            || self.artifact_status.is_some()
            || self.dimension_status.is_some()
        {
            Frame::new()
                .fill(theme::chrome_mid())
                .stroke(theme::soft_stroke())
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

    fn toggle_dimension_tool(&mut self) {
        self.dimension_tool.active = !self.dimension_tool.active;
        self.dimension_tool.clear_picks();
        if self.dimension_tool.active {
            self.draw_wall_tool = DrawWallToolState::default();
            self.room_tool_active = false;
        }
        self.opening_drag = None;
        self.dimension_status = if self.dimension_tool.active {
            Some("Pick two anchors, then move the pointer to place the dimension".to_owned())
        } else {
            None
        };
        if self.dimension_tool.active {
            self.viewport_mode = ViewportMode::Elevation;
        }
    }

    pub(super) fn model_tree(&mut self, ui: &mut Ui) {
        panel_header(ui, "Model Browser", self.workspace_badge());

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
                    let rooms: Vec<_> = self
                        .model
                        .rooms
                        .iter()
                        .map(|room| (room.id.0.clone(), room.name.clone(), room.level.0.clone()))
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

                            for (room_id, room_name, room_level) in &rooms {
                                if room_level != &level_id {
                                    continue;
                                }
                                let selected = matches!(
                                    &self.selected,
                                    Selection::Room(id) if id == room_id
                                );
                                if ui
                                    .selectable_label(selected, format!("Room: {room_name}"))
                                    .clicked()
                                {
                                    self.selected = Selection::Room(room_id.clone());
                                }
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

            self.library_tree(ui);

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

    /// The construction-system / material library browser: a collapsible header
    /// with Systems and Materials sub-lists. Rows select the matching library
    /// element; the footer buttons author new systems/materials.
    fn library_tree(&mut self, ui: &mut Ui) {
        // Authoring buttons (add system/material) are only offered when design
        // edits are allowed; selecting library rows stays allowed in Plan mode.
        let can_edit = self.workspace_mode.allows_design_edits();
        let systems: Vec<(String, String, &'static str)> = self
            .model
            .systems
            .iter()
            .map(|system| {
                (
                    system.id.0.clone(),
                    system.name.clone(),
                    system.kind.label(),
                )
            })
            .collect();
        let materials: Vec<(String, String, [u8; 3])> = self
            .model
            .materials
            .iter()
            .map(|material| {
                (
                    material.id.0.clone(),
                    material.name.clone(),
                    material.color(),
                )
            })
            .collect();

        egui::CollapsingHeader::new("Library")
            .default_open(false)
            .show(ui, |ui| {
                ui.strong("Systems");
                for (id, name, kind) in &systems {
                    let selected = matches!(&self.selected, Selection::System(s) if s == id);
                    if ui
                        .selectable_label(selected, format!("{name} ({kind})"))
                        .clicked()
                    {
                        self.selected = Selection::System(id.clone());
                    }
                }
                if can_edit {
                    ui.horizontal(|ui| {
                        if ui.button("+ Wall system").clicked() {
                            self.add_wall_system(true);
                        }
                        if ui.button("+ Interior system").clicked() {
                            self.add_wall_system(false);
                        }
                    });
                }

                ui.separator();
                ui.strong("Materials");
                for (id, name, color) in &materials {
                    let selected = matches!(&self.selected, Selection::Material(m) if m == id);
                    let [r, g, b] = *color;
                    if ui
                        .horizontal(|ui| {
                            color_swatch(ui, Color32::from_rgb(r, g, b));
                            ui.selectable_label(selected, name).clicked()
                        })
                        .inner
                    {
                        self.selected = Selection::Material(id.clone());
                    }
                }
                if can_edit && ui.button("+ Material").clicked() {
                    self.add_material();
                }
            });
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui) {
        let mut changed = false;
        // A Remove click sets this; executed through edit() after the model
        // borrow below ends, so deletions are one labelled, undoable step.
        let mut deferred_remove: Option<DeferredRemove> = None;
        // Library actions deferred past the inspector's `&mut model` borrow, each
        // replayed through edit() as one labelled, undoable step.
        //   - add a layer to the selected system
        let mut deferred_add_layer: Option<String> = None;
        //   - reorder a layer within the selected system: (system id, index, dir)
        let mut deferred_move_layer: Option<(String, usize, isize)> = None;
        //   - remove a layer from the selected system: (system id, index)
        let mut deferred_remove_layer: Option<(String, usize)> = None;
        //   - jump the selection to a construction system (Wall "Edit system").
        let mut deferred_select_system: Option<String> = None;
        // Capture a pre-edit baseline for the undo transaction, but only while
        // an interaction could open one (pointer down or a text field focused)
        // and none is already in flight — so idle frames never clone the model.
        let edit_base = if should_capture_edit_base(
            self.history.is_pending(),
            ui.ctx().input(|input| input.pointer.any_down()),
            ui.ctx().input(|input| input.pointer.any_click()),
            ui.ctx().text_edit_focused(),
        ) {
            Some(self.snapshot())
        } else {
            None
        };
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
        // Snapshot of the wall-kind construction systems for the System picker,
        // collected before the mutable wall borrow: (id, name, thickness, exposure).
        let wall_systems = self
            .model
            .systems
            .iter()
            .filter(|system| system.kind == framer_core::SystemKind::Wall)
            .map(|system| {
                (
                    system.id.0.clone(),
                    system.name.clone(),
                    system.total_thickness().to_string(),
                    system.exposure().label().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        // Richer per-wall-system summaries (stacked swatch, R-value, layer count)
        // for the Wall inspector's read-only System block, collected before the
        // mutable wall borrow.
        let wall_system_summaries = self
            .model
            .systems
            .iter()
            .filter(|system| system.kind == framer_core::SystemKind::Wall)
            .map(|system| WallSystemSummary::from_system(system, &self.model))
            .collect::<Vec<_>>();
        // Material picklist + swatch colors, collected before any `&mut system`
        // borrow so layer ComboBoxes/swatches don't alias the system iter_mut().
        let material_options = self
            .model
            .materials
            .iter()
            .map(|material| (material.id.0.clone(), material.name.clone()))
            .collect::<Vec<(String, String)>>();
        let material_colors = self
            .model
            .materials
            .iter()
            .map(|material| (material.id.0.clone(), material.color()))
            .collect::<Vec<(String, [u8; 3])>>();
        // R-value of the selected system (clear-wall, milli-R), computed before the
        // mutable borrow since it reads the whole material library.
        let selected_system_r_milli = if let Selection::System(id) = &selection {
            self.model
                .systems
                .iter()
                .find(|system| system.id.0 == *id)
                .map(|system| system.r_value_milli(&self.model.materials))
        } else {
            None
        };
        // Whether the selected system is applied to any wall. A referenced
        // Wall-kind system must keep `kind == Wall` (switching it to Floor/Roof
        // would invalidate every wall that uses it), so its Kind row is locked.
        let selected_system_referenced = if let Selection::System(id) = &selection {
            self.model.walls.iter().any(|wall| wall.system.0 == *id)
        } else {
            false
        };

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
            Selection::Room(id) => {
                let boundary = self
                    .model
                    .rooms
                    .iter()
                    .find(|room| room.id.0 == id)
                    .and_then(|room| framer_core::room_boundary(&self.model, room.seed));
                if let Some(room) = self.model.rooms.iter_mut().find(|room| room.id.0 == id) {
                    ui.label(&room.id.0);
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut room.name);
                        ComboBox::from_id_salt("room-usage")
                            .selected_text(room.usage.label())
                            .show_ui(ui, |ui| {
                                for usage in framer_core::RoomUsage::ALL {
                                    changed |= ui
                                        .selectable_value(&mut room.usage, usage, usage.label())
                                        .changed();
                                }
                            });
                    } else {
                        ui.label(&room.name);
                        ui.label(format!("Usage: {}", room.usage.label()));
                    }
                    match &boundary {
                        Some(boundary) => {
                            ui.label(format!("Area: {:.0} sq ft", boundary.area_square_feet()));
                            ui.label(format!("Perimeter: {}", boundary.perimeter));
                        }
                        None => {
                            ui.label("Boundary: open (not enclosed)");
                        }
                    }
                } else {
                    ui.label("Room no longer exists");
                }
            }
            Selection::Wall => {
                let (wall_length_driver, wall_height_driver) = self
                    .model
                    .walls
                    .get(self.selected_wall)
                    .map(|wall| {
                        (
                            driven_length_field(wall, DimensionVariableKey::WallLength),
                            driven_length_field(wall, DimensionVariableKey::WallHeight),
                        )
                    })
                    .unwrap_or_default();
                let mut select_dimension = None;
                if let Some(wall) = self.model.walls.get_mut(self.selected_wall) {
                    if can_edit {
                        inspector_object_id(ui, &wall.id.0);
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

                        widgets::section(ui, "wall-dimensions", "Dimensions", true, |ui| {
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
                                wall_height_driver.as_ref(),
                                &mut select_dimension,
                            );
                        });

                        widgets::section(ui, "wall-placement", "Placement", true, |ui| {
                            let placement_changed =
                                coordinate_drag(ui, "Start X", &mut wall.start.x)
                                    | coordinate_drag(ui, "Start Y", &mut wall.start.y)
                                    | coordinate_drag(ui, "End X", &mut wall.end.x)
                                    | coordinate_drag(ui, "End Y", &mut wall.end.y);
                            if placement_changed {
                                if let Some(length) = wall.placement_length() {
                                    wall.length = length;
                                }
                                changed = true;
                            }
                        });

                        changed |= widgets::section(ui, "wall-system", "System", true, |ui| {
                            let mut c = false;
                            let selected_text = wall_systems
                                .iter()
                                .find(|(id, ..)| *id == wall.system.0)
                                .map(|(_, name, ..)| name.clone())
                                .unwrap_or_else(|| wall.system.0.clone());
                            c |= property_row(ui, "System", |ui| {
                                let before = wall.system.0.clone();
                                ComboBox::from_id_salt("wall-system")
                                    .selected_text(selected_text)
                                    .show_ui(ui, |ui| {
                                        for (id, name, _, _) in &wall_systems {
                                            ui.selectable_value(
                                                &mut wall.system,
                                                ElementId::new(id.clone()),
                                                name,
                                            );
                                        }
                                    });
                                wall.system.0 != before
                            });
                            ui.add_space(design::space::XS);
                            if let Some(summary) = wall_system_summaries
                                .iter()
                                .find(|summary| summary.id == wall.system.0)
                            {
                                stacked_swatch(ui, &summary.bands);
                                ui.add_space(design::space::XS);
                                egui::Grid::new("wall-system-summary")
                                    .num_columns(2)
                                    .spacing([12.0, 4.0])
                                    .show(ui, |ui| {
                                        summary_row(ui, "Name", &summary.name);
                                        summary_row(ui, "Layers", summary.layer_count);
                                        summary_row(
                                            ui,
                                            "Total thickness",
                                            &summary.total_thickness,
                                        );
                                        summary_row(ui, "Exposure", summary.exposure);
                                        summary_row(
                                            ui,
                                            "R-value",
                                            format_r_value(summary.r_value_milli),
                                        );
                                    });
                                if ui.button("Edit system").clicked() {
                                    deferred_select_system = Some(summary.id.clone());
                                }
                            } else {
                                ui.label(
                                    RichText::new("System not found in the model library.")
                                        .size(design::text_size::LABEL)
                                        .color(design::active().text_muted),
                                );
                            }
                            c
                        })
                        .unwrap_or(false);

                        widgets::section(ui, "wall-tags", "Tags", false, |ui| {
                            changed |= tags_editor(ui, &mut wall.tags);
                        });
                    } else {
                        let system_name = wall_systems
                            .iter()
                            .find(|(id, ..)| *id == wall.system.0)
                            .map(|(_, name, ..)| name.as_str())
                            .unwrap_or(wall.system.0.as_str());
                        wall_summary(ui, wall, &level_options, system_name);
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
                                driven_fields.height.as_ref(),
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
                                driven_fields.bottom.as_ref(),
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
                        deferred_remove = Some(DeferredRemove::Opening(id.clone()));
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
                        deferred_remove = Some(DeferredRemove::Dimension(id.clone()));
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
            Selection::System(id) => {
                if let Some(system) = self
                    .model
                    .systems
                    .iter_mut()
                    .find(|system| system.id.0 == id)
                {
                    inspector_object_id(ui, &system.id.0);
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut system.name);

                        // A wall-referenced system cannot change kind (a Floor/Roof
                        // system on a wall is invalid), so its Kind row is a plain
                        // read-only label; unreferenced systems keep the picker.
                        if selected_system_referenced {
                            summary_row(ui, "Kind", system.kind.label());
                        } else {
                            property_row(ui, "Kind", |ui| {
                                ComboBox::from_id_salt("system-kind")
                                    .selected_text(system.kind.label())
                                    .show_ui(ui, |ui| {
                                        for kind in framer_core::SystemKind::ALL {
                                            changed |= ui
                                                .selectable_value(
                                                    &mut system.kind,
                                                    kind,
                                                    kind.label(),
                                                )
                                                .changed();
                                        }
                                    });
                            });
                        }

                        widgets::section(ui, "system-layers", "Layers", true, |ui| {
                            let layer_count = system.layers.len();
                            // Wall systems require exactly one positive-thickness
                            // framing layer; the editor uses this count to keep the
                            // sole framing layer un-removable and block adding a
                            // second one.
                            let is_wall = system.kind == framer_core::SystemKind::Wall;
                            let framing_count = system
                                .layers
                                .iter()
                                .filter(|layer| {
                                    layer.function == framer_core::LayerFunction::Framing
                                })
                                .count();
                            for (index, layer) in system.layers.iter_mut().enumerate() {
                                changed |= system_layer_editor(
                                    ui,
                                    &id,
                                    index,
                                    layer_count,
                                    is_wall,
                                    framing_count,
                                    layer,
                                    &material_options,
                                    &material_colors,
                                    &mut deferred_move_layer,
                                    &mut deferred_remove_layer,
                                );
                                ui.add_space(design::space::XS);
                            }
                            if ui.button("+ Layer").clicked() {
                                deferred_add_layer = Some(id.clone());
                            }
                        });

                        ui.add_space(design::space::XS);
                        egui::Grid::new("system-footer")
                            .num_columns(2)
                            .spacing([12.0, 4.0])
                            .show(ui, |ui| {
                                summary_row(
                                    ui,
                                    "Total thickness",
                                    system.total_thickness().to_string(),
                                );
                                if let Some(milli) = selected_system_r_milli {
                                    summary_row(ui, "R-value", format_r_value(milli));
                                }
                            });
                    } else {
                        system_summary(ui, system, selected_system_r_milli);
                    }
                } else {
                    ui.label("System no longer exists");
                }
            }
            Selection::Material(id) => {
                if let Some(material) = self.model.materials.iter_mut().find(|m| m.id.0 == id) {
                    inspector_object_id(ui, &material.id.0);
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut material.name);
                        changed |= material_appearance_editor(ui, material);
                        widgets::section(ui, "material-props", "Properties", true, |ui| {
                            changed |= material_properties_editor(ui, material);
                        });
                        widgets::section(ui, "material-tags", "Tags", false, |ui| {
                            changed |= tags_editor(ui, &mut material.tags);
                        });
                    } else {
                        material_summary(ui, material);
                    }
                } else {
                    ui.label("Material no longer exists");
                }
            }
        }

        // Replay a deferred Remove as one discrete, labelled undo step now that
        // the model borrow above has ended. These paths leave `changed` false,
        // so they do not also flow through the coalesced inspector transaction.
        match deferred_remove {
            Some(DeferredRemove::Opening(opening_id)) => {
                self.edit("Remove opening", |app| {
                    if let Some(wall) = app.model.walls.get_mut(app.selected_wall)
                        && wall.remove_opening(&ElementId::new(opening_id))
                    {
                        app.selected = Selection::Wall;
                    }
                });
            }
            Some(DeferredRemove::Dimension(dimension_id)) => {
                self.edit("Remove dimension", |app| {
                    if let Some(wall) = app.model.walls.get_mut(app.selected_wall) {
                        let before = wall.dimensions.len();
                        wall.dimensions
                            .retain(|dimension| dimension.id.0 != dimension_id);
                        if wall.dimensions.len() != before {
                            app.selected = Selection::Wall;
                        }
                    }
                });
            }
            None => {}
        }

        // Replay deferred library actions (layer add/reorder/remove and the Wall
        // inspector's "Edit system" jump) now that the model borrow has ended.
        // Each *_layer action is one labelled, undoable edit; the selection jump is
        // pure presentation state, so it is applied directly.
        if let Some(system_id) = deferred_add_layer {
            self.add_layer(&system_id);
        }
        if let Some((system_id, index, dir)) = deferred_move_layer {
            self.move_layer(&system_id, index, dir);
        }
        if let Some((system_id, index)) = deferred_remove_layer {
            self.remove_layer(&system_id, index);
        }
        if let Some(system_id) = deferred_select_system {
            self.selected = Selection::System(system_id);
        }

        if changed {
            // Open one coalesced undo step for this inspector edit run. begin()
            // is a no-op once a transaction is already in flight, so a drag
            // across many frames collapses to a single step; settle_history()
            // (end of frame) commits it when the interaction ends.
            if let Some(base) = edit_base {
                let label = inspector_edit_label(&self.selected);
                self.begin_inspector_edit(base, label);
            }
            self.rebuild();
        }

        if self.workspace_mode.shows_generated_plan() {
            ui.separator();
            diagnostics_panel(ui, self.error.as_deref(), self.project_plan.as_ref());
            ui.separator();
            bom_panel(ui, self.project_plan.as_ref());
            if self
                .project_plan
                .as_ref()
                .is_some_and(|plan| !plan.rooms.is_empty())
            {
                ui.separator();
                room_schedule_panel(ui, self.project_plan.as_ref());
            }
        } else if let Some(error) = self.error.as_deref() {
            ui.separator();
            panel_subheader(ui, "Validation");
            ui.colored_label(theme::danger(), error);
        }
    }

    fn workspace_badge(&self) -> &'static str {
        match self.workspace_mode {
            WorkspaceMode::Design => "Design",
            WorkspaceMode::Plan => "Plan",
        }
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        let t = design::active();
        let (unsupported, warnings, info) = self.diagnostic_counts();
        let error_count = usize::from(self.error.is_some());

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(design::space::MD, 2.0);
            status_metric(ui, Icon::Ready, "Ready", t.success);
            toolbar_divider(ui);
            self.status_level_dropdown(ui);
            toolbar_divider(ui);
            ui.label(
                RichText::new(self.workspace_badge())
                    .strong()
                    .size(design::text_size::LABEL)
                    .color(t.text_secondary),
            );
            if let Some(wall) = self.model.walls.get(self.selected_wall) {
                ui.label(
                    RichText::new(&wall.name)
                        .size(design::text_size::LABEL)
                        .color(t.text_muted),
                );
            }
            toolbar_divider(ui);
            ui.label(
                RichText::new(self.selection_status())
                    .size(design::text_size::LABEL)
                    .color(t.text_secondary),
            );
            if let Some(cursor) = self.cursor_model {
                toolbar_divider(ui);
                ui.label(
                    RichText::new(format!(
                        "X {:.3} ft   Y {:.3} ft   Z 0.000 ft",
                        cursor.x.inches() / 12.0,
                        cursor.y.inches() / 12.0
                    ))
                    .monospace()
                    .size(design::text_size::LABEL)
                    .color(t.text_muted),
                );
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = design::space::MD;
                ui.label(
                    RichText::new("100%")
                        .size(design::text_size::LABEL)
                        .color(t.text_secondary),
                );
                toolbar_divider(ui);
                widgets::toggle_switch(ui, &mut self.ortho, "Ortho");
                // Layers popover: the wall display mode (shared by Plan + 3D) plus
                // per-layer visibility toggles. `CloseOnClickOutside` keeps it open
                // while the user flips several layers — it dismisses on an
                // outside-click or Escape, not on each toggle.
                let (layers_button, _) =
                    MenuButton::new(design::icon_text(Icon::Eye, 14.0).color(t.text_secondary))
                        .config(
                            MenuConfig::new()
                                .close_behavior(PopupCloseBehavior::CloseOnClickOutside),
                        )
                        .ui(ui, |ui| {
                            ui.set_min_width(168.0);
                            ui.label(
                                RichText::new("WALLS")
                                    .size(design::text_size::LABEL)
                                    .color(t.text_muted),
                            );
                            ui.horizontal(|ui| {
                                for mode in
                                    [WallDisplay::Outline, WallDisplay::Width, WallDisplay::Full]
                                {
                                    ui.selectable_value(
                                        &mut self.layers.wall_display,
                                        mode,
                                        mode.label(),
                                    );
                                }
                            });
                            ui.separator();
                            ui.label(
                                RichText::new("SHOW")
                                    .size(design::text_size::LABEL)
                                    .color(t.text_muted),
                            );
                            widgets::toggle_switch(ui, &mut self.layers.grid, "Grid");
                            widgets::toggle_switch(ui, &mut self.layers.rooms, "Rooms");
                            widgets::toggle_switch(ui, &mut self.layers.joins, "Joins");
                            widgets::toggle_switch(ui, &mut self.layers.wall_labels, "Wall labels");
                        });
                // The trigger is icon-only, so give it an explicit accessible name
                // (otherwise a screen reader announces the raw glyph). The tooltip
                // keeps the longer description for sighted users.
                let layers_enabled = layers_button.enabled();
                layers_button.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, layers_enabled, "Layers")
                });
                layers_button.on_hover_text("Layers — wall display mode and element visibility");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(design::icon_text(Icon::Snap, 13.0).color(t.text_secondary));
                    ComboBox::from_id_salt("snap-step")
                        .selected_text(snap_label(self.snap_step))
                        .width(58.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.snap_step, None, "Off");
                            ui.selectable_value(
                                &mut self.snap_step,
                                Some(Length::from_inches(0.5)),
                                "1/2 in",
                            );
                            ui.selectable_value(
                                &mut self.snap_step,
                                Some(Length::from_whole_inches(1)),
                                "1 in",
                            );
                            ui.selectable_value(
                                &mut self.snap_step,
                                Some(Length::from_whole_inches(2)),
                                "2 in",
                            );
                            ui.selectable_value(
                                &mut self.snap_step,
                                Some(Length::from_whole_inches(6)),
                                "6 in",
                            );
                        });
                });
                toolbar_divider(ui);
                let muted = t.text_muted;
                status_metric(ui, Icon::Saved, &format!("{info} info"), muted);
                status_metric(
                    ui,
                    Icon::Warning,
                    &format!("{unsupported} unsupported"),
                    if unsupported == 0 { muted } else { t.warning },
                );
                status_metric(
                    ui,
                    Icon::Warning,
                    &format!("{warnings} warnings"),
                    if warnings == 0 { muted } else { t.warning },
                );
                status_metric(
                    ui,
                    Icon::Error,
                    &format!("{error_count} errors"),
                    if error_count == 0 { muted } else { t.danger },
                );
            });
        });
    }

    fn status_level_dropdown(&mut self, ui: &mut Ui) {
        let levels: Vec<(String, String)> = self
            .model
            .levels
            .iter()
            .map(|level| (level.id.0.clone(), level.name.clone()))
            .collect();
        if levels.is_empty() {
            return;
        }
        let current = self
            .model
            .walls
            .get(self.selected_wall)
            .map(|wall| wall.level.0.clone())
            .filter(|id| levels.iter().any(|(level_id, _)| level_id == id))
            .unwrap_or_else(|| levels[0].0.clone());
        let current_name = levels
            .iter()
            .find(|(id, _)| id == &current)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| current.clone());

        let t = design::active();
        let mut chosen = current.clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(design::icon_text(Icon::LayoutGrid, 13.0).color(t.text_secondary));
            ComboBox::from_id_salt("status-level")
                .selected_text(current_name)
                .width(78.0)
                .show_ui(ui, |ui| {
                    for (id, name) in &levels {
                        ui.selectable_value(&mut chosen, id.clone(), name);
                    }
                });
        });
        if chosen != current {
            self.selected = Selection::Level(chosen);
        }
    }

    fn selection_status(&self) -> String {
        match &self.selected {
            Selection::Level(id) => format!("Level: {id}"),
            Selection::Wall => "Wall segment".to_owned(),
            Selection::Opening(id) => format!("Opening: {id}"),
            Selection::Dimension(id) => format!("Dimension: {id}"),
            Selection::Join(id) => format!("Join: {id}"),
            Selection::Room(id) => format!("Room: {id}"),
            Selection::Member { member_id, .. } => format!("Member: {member_id}"),
            Selection::System(id) => format!("System: {id}"),
            Selection::Material(id) => format!("Material: {id}"),
        }
    }

    fn diagnostic_counts(&self) -> (usize, usize, usize) {
        let Some(plan) = &self.project_plan else {
            return (0, 0, 0);
        };
        plan.diagnostics
            .iter()
            .chain(
                plan.wall_plans
                    .iter()
                    .flat_map(|wall_plan| wall_plan.diagnostics.iter()),
            )
            .fold(
                (0, 0, 0),
                |(unsupported, warnings, info), diagnostic| match diagnostic.severity {
                    DiagnosticSeverity::Unsupported => (unsupported + 1, warnings, info),
                    DiagnosticSeverity::Warning => (unsupported, warnings + 1, info),
                    DiagnosticSeverity::Info => (unsupported, warnings, info + 1),
                },
            )
    }
}

fn header_divider(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 20.0), egui::Sense::hover());
    let x = rect.center().x;
    ui.painter().line_segment(
        [
            egui::Pos2::new(x, rect.top()),
            egui::Pos2::new(x, rect.bottom()),
        ],
        Stroke::new(1.0, color),
    );
}

fn header_save_pill(ui: &mut Ui, head: design::Theme, status: Option<&str>) {
    let (icon, text, color) = match status {
        Some(s) if s.to_lowercase().contains("failed") => {
            (Icon::Error, short_status(s), design::active().danger)
        }
        Some(s) => (Icon::Saved, short_status(s), design::active().success),
        None => (Icon::Saved, "Ready".to_owned(), design::active().success),
    };
    Frame::new()
        .fill(Color32::from_white_alpha(16))
        .corner_radius(design::radius::SM)
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(design::icon_text(icon, 12.0).color(color));
                ui.label(
                    RichText::new(text)
                        .size(design::text_size::LABEL)
                        .color(head.text),
                );
            });
        });
}

fn header_profile(ui: &mut Ui, head: design::Theme, name: &str) {
    let font = egui::FontId::proportional(design::text_size::LABEL);
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_owned(), font, head.text);
    let (pad, gap, chev) = (9.0, 7.0, 12.0);
    let width = pad + galley.size().x + gap + chev + pad;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, design::control::ICON_BTN),
        egui::Sense::click(),
    );

    let painter = ui.painter();
    painter.rect(
        rect,
        design::radius::SM,
        Color32::from_white_alpha(16),
        Stroke::new(1.0, head.divider),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        egui::Pos2::new(rect.left() + pad, rect.center().y - galley.size().y / 2.0),
        galley,
        head.text,
    );
    painter.text(
        egui::Pos2::new(rect.right() - pad - chev / 2.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        Icon::ChevronDown.glyph().to_string(),
        design::icon_font(12.0),
        head.text_secondary,
    );
    response.on_hover_text("Code profile");
}

fn short_status(status: &str) -> String {
    status
        .split_whitespace()
        .next()
        .unwrap_or("Ready")
        .to_owned()
}

/// A coarse, selection-derived label for an inspector edit, shown on the Undo
/// button (e.g. "Undo Edit wall"). Inspector edits are coalesced per selection,
/// so a single label per selected-object kind is the right granularity.
fn inspector_edit_label(selection: &Selection) -> &'static str {
    match selection {
        Selection::Level(_) => "Edit level",
        Selection::Wall => "Edit wall",
        Selection::Opening(_) => "Edit opening",
        Selection::Dimension(_) => "Edit dimension",
        Selection::Join(_) => "Edit join",
        Selection::Room(_) => "Edit room",
        Selection::Member { .. } => "Edit",
        Selection::System(_) => "Edit system",
        Selection::Material(_) => "Edit material",
    }
}

/// A deletion requested from the inspector this frame, deferred until the
/// inspector's `&mut` borrow of the model ends so it can be replayed through
/// `FramerApp::edit` as one correctly-labelled undo step.
enum DeferredRemove {
    Opening(String),
    Dimension(String),
}

/// Whether to snapshot a pre-edit baseline this frame for the inspector's undo
/// transaction. We must capture on any frame where an inspector edit could
/// *commit*: while dragging a value (`pointer_down`), on the click-release that
/// commits a ComboBox selection or button (`any_click` — egui fires the click
/// on release, when `pointer_down` is already false), or while a text field is
/// focused. Only when no transaction is already in flight, so idle frames never
/// clone the model.
fn should_capture_edit_base(
    is_pending: bool,
    pointer_down: bool,
    any_click: bool,
    text_focused: bool,
) -> bool {
    !is_pending && (pointer_down || any_click || text_focused)
}

fn toolbar_divider(ui: &mut Ui) {
    ui.separator();
}

fn panel_header(ui: &mut Ui, title: &str, badge: &str) {
    let t = design::active();
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().size(17.0).color(t.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            Frame::new()
                .fill(t.control)
                .stroke(t.soft_stroke())
                .corner_radius(design::radius::SM)
                .inner_margin(Margin::symmetric(8, 2))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(badge)
                            .size(design::text_size::LABEL)
                            .color(t.text_secondary),
                    );
                });
        });
    });
    ui.add_space(design::space::SM);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        t.divider_stroke(),
    );
    ui.add_space(design::space::MD);
}

fn inspector_object_id(ui: &mut Ui, id: &str) {
    ui.label(
        RichText::new(id)
            .size(design::text_size::LABEL)
            .color(design::active().text_muted),
    );
    ui.add_space(2.0);
}

/// Removable tag chips plus an add field. Returns whether `tags` changed.
fn tags_editor(ui: &mut Ui, tags: &mut Vec<String>) -> bool {
    let t = design::active();
    let mut changed = false;
    let mut remove = None;

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (index, tag) in tags.iter().enumerate() {
            let response = Frame::new()
                .fill(t.control)
                .stroke(t.soft_stroke())
                .corner_radius(design::radius::SM)
                .inner_margin(Margin::symmetric(7, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(
                            RichText::new(tag)
                                .size(design::text_size::LABEL)
                                .color(t.text_secondary),
                        );
                        ui.label(design::icon_text(Icon::Delete, 11.0).color(t.text_muted));
                    });
                })
                .response
                .interact(egui::Sense::click())
                .on_hover_text("Remove tag");
            if response.clicked() {
                remove = Some(index);
            }
        }
    });

    if let Some(index) = remove {
        tags.remove(index);
        changed = true;
    }

    let id = ui.id().with("tag-draft");
    let mut draft = ui.data_mut(|data| data.get_temp::<String>(id).unwrap_or_default());
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut draft)
                .hint_text("Add tag…")
                .desired_width(150.0),
        );
        let submit = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let add = widgets::icon_button(ui, Icon::Plus, "Add tag").clicked();
        if (submit || add) && !draft.trim().is_empty() {
            tags.push(draft.trim().to_owned());
            draft.clear();
            changed = true;
        }
    });
    ui.data_mut(|data| data.insert_temp(id, draft));

    changed
}

fn panel_subheader(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(title)
            .strong()
            .size(12.0)
            .color(theme::text_muted()),
    );
    ui.add_space(3.0);
}

fn selection_badge(selection: &Selection) -> &'static str {
    match selection {
        Selection::Level(_) => "Level",
        Selection::Wall => "Wall",
        Selection::Opening(_) => "Opening",
        Selection::Dimension(_) => "Dimension",
        Selection::Join(_) => "Join",
        Selection::Room(_) => "Room",
        Selection::Member { .. } => "Member",
        Selection::System(_) => "System",
        Selection::Material(_) => "Material",
    }
}

#[derive(Clone, Copy)]
enum StatusTone {
    Info,
    Success,
    Warning,
}

fn status_chip(ui: &mut Ui, text: &str, tone: StatusTone) {
    let (fill, stroke, text_color) = match tone {
        StatusTone::Info => (
            theme::active_blue_soft(),
            theme::active_blue(),
            theme::text_primary(),
        ),
        StatusTone::Success => (
            Color32::from_rgb(28, 67, 45),
            theme::success(),
            Color32::from_rgb(217, 245, 225),
        ),
        StatusTone::Warning => (
            Color32::from_rgb(82, 63, 25),
            theme::warning(),
            Color32::from_rgb(250, 232, 188),
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

/// An icon + text status-bar metric (used for Ready, diagnostics, snap).
fn status_metric(ui: &mut Ui, icon: Icon, text: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(design::icon_text(icon, 13.0).color(color));
        ui.label(
            RichText::new(text)
                .size(design::text_size::LABEL)
                .color(design::active().text_secondary),
        );
    });
}

fn snap_label(step: Option<Length>) -> String {
    match step {
        None => "Off".to_owned(),
        Some(step) if step == Length::from_inches(0.5) => "1/2 in".to_owned(),
        Some(step) => format!("{} in", step.inches()),
    }
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

/// A read-only summary of a wall-kind construction system, prepared before the
/// inspector's mutable wall borrow so the Wall inspector can show the applied
/// system's layer build-up (stacked swatch + totals) without re-borrowing.
struct WallSystemSummary {
    id: String,
    name: String,
    total_thickness: String,
    exposure: &'static str,
    r_value_milli: i64,
    layer_count: usize,
    /// Per-layer (interior -> exterior) swatch color and thickness weight in
    /// ticks, for the stacked mini-swatch.
    bands: Vec<(Color32, i64)>,
}

impl WallSystemSummary {
    fn from_system(
        system: &framer_core::ConstructionSystem,
        model: &framer_core::BuildingModel,
    ) -> Self {
        let bands = system
            .layers
            .iter()
            .map(|layer| {
                let [r, g, b] = model
                    .material(&layer.material)
                    .map(|material| material.color())
                    .unwrap_or([188, 179, 158]);
                (Color32::from_rgb(r, g, b), layer.thickness.ticks().max(1))
            })
            .collect();
        Self {
            id: system.id.0.clone(),
            name: system.name.clone(),
            total_thickness: system.total_thickness().to_string(),
            exposure: system.exposure().label(),
            r_value_milli: system.r_value_milli(&model.materials),
            layer_count: system.layers.len(),
            bands,
        }
    }
}

/// Format a milli-R value (R x 1000) as a one-decimal "R-#.#" string.
fn format_r_value(milli: i64) -> String {
    format!("R-{:.1}", milli as f64 / 1000.0)
}

/// A small horizontal stacked swatch: one band per construction layer
/// (interior -> exterior), each width-weighted by its thickness.
fn stacked_swatch(ui: &mut Ui, bands: &[(Color32, i64)]) {
    let height = 18.0;
    let width = ui.available_width().clamp(60.0, 220.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let total: i64 = bands.iter().map(|(_, weight)| *weight).sum::<i64>().max(1);
    let mut x = rect.left();
    for (index, (color, weight)) in bands.iter().enumerate() {
        let band_width = if index + 1 == bands.len() {
            rect.right() - x
        } else {
            width * (*weight as f32 / total as f32)
        };
        let band = egui::Rect::from_min_max(
            egui::pos2(x, rect.top()),
            egui::pos2(x + band_width, rect.bottom()),
        );
        ui.painter().rect_filled(band, 0.0, *color);
        x += band_width;
    }
    ui.painter()
        .rect_stroke(rect, 2.0, theme::soft_stroke(), egui::StrokeKind::Inside);
}

/// Board profiles offered in the framing-member picker (interior -> exterior is
/// irrelevant here; this is the size menu).
const BOARD_PROFILES: [framer_core::BoardProfile; 5] = [
    framer_core::BoardProfile::TwoByFour,
    framer_core::BoardProfile::TwoBySix,
    framer_core::BoardProfile::TwoByEight,
    framer_core::BoardProfile::TwoByTen,
    framer_core::BoardProfile::TwoByTwelve,
];

/// Paint a small solid color swatch (used to preview a layer's material).
fn color_swatch(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, color);
    ui.painter()
        .rect_stroke(rect, 2.0, theme::soft_stroke(), egui::StrokeKind::Inside);
}

/// Look up a material's swatch color by id, falling back to a neutral tone.
fn material_color(material_colors: &[(String, [u8; 3])], id: &str) -> Color32 {
    material_colors
        .iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, [r, g, b])| Color32::from_rgb(*r, *g, *b))
        .unwrap_or_else(|| Color32::from_rgb(188, 179, 158))
}

/// Look up a material's display name by id, falling back to the raw id.
fn material_name(material_options: &[(String, String)], id: &str) -> String {
    material_options
        .iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| id.to_owned())
}

/// Inline editor for one construction layer (interior -> exterior). Returns
/// whether the layer's data changed; reorder/remove are deferred to the caller.
#[allow(clippy::too_many_arguments)]
fn system_layer_editor(
    ui: &mut Ui,
    system_id: &str,
    index: usize,
    layer_count: usize,
    is_wall: bool,
    framing_count: usize,
    layer: &mut framer_core::ConstructionLayer,
    material_options: &[(String, String)],
    material_colors: &[(String, [u8; 3])],
    deferred_move: &mut Option<(String, usize, isize)>,
    deferred_remove: &mut Option<(String, usize)>,
) -> bool {
    use framer_core::{BoardProfile, ElementId, FramingPattern, LayerFunction};

    let mut changed = false;
    let is_framing = layer.function == LayerFunction::Framing;
    // A Wall system must keep exactly one framing layer, so its sole framing
    // layer cannot be removed (in addition to the general "keep one layer"
    // guard) and the Function picker may not introduce a second one.
    let is_only_framing = is_framing && is_wall && framing_count <= 1;
    Frame::new()
        .fill(design::active().control)
        .stroke(theme::soft_stroke())
        .corner_radius(design::radius::SM)
        .inner_margin(Margin::same(6))
        .show(ui, |ui| {
            // Header row: material swatch, ordinal, and reorder/remove controls.
            ui.horizontal(|ui| {
                color_swatch(ui, material_color(material_colors, &layer.material.0));
                ui.label(
                    RichText::new(format!("Layer {}", index + 1))
                        .strong()
                        .size(design::text_size::LABEL)
                        .color(theme::text_secondary()),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(layer_count > 1 && !is_only_framing, |ui: &mut Ui| {
                            widgets::icon_button(ui, Icon::Delete, "Remove layer")
                        })
                        .clicked()
                    {
                        *deferred_remove = Some((system_id.to_owned(), index));
                    }
                    if ui
                        .add_enabled(index + 1 < layer_count, |ui: &mut Ui| {
                            widgets::icon_button(ui, Icon::ChevronDown, "Move toward exterior")
                        })
                        .clicked()
                    {
                        *deferred_move = Some((system_id.to_owned(), index, 1));
                    }
                    if ui
                        .add_enabled(index > 0, |ui: &mut Ui| {
                            widgets::icon_button(ui, Icon::ChevronUp, "Move toward interior")
                        })
                        .clicked()
                    {
                        *deferred_move = Some((system_id.to_owned(), index, -1));
                    }
                });
            });

            // Material picker.
            changed |= property_row(ui, "Material", |ui| {
                let before = layer.material.0.clone();
                ComboBox::from_id_salt(("layer-material", index))
                    .selected_text(material_name(material_options, &layer.material.0))
                    .show_ui(ui, |ui| {
                        for (id, name) in material_options {
                            ui.selectable_value(
                                &mut layer.material,
                                ElementId::new(id.clone()),
                                name,
                            );
                        }
                    });
                layer.material.0 != before
            });

            // Thickness (inches; layers are small so an inch display reads best).
            // A framing layer's depth is the member's nominal depth, so geometry
            // and BOM agree; show it read-only. Other layers stay editable, with a
            // one-tick minimum so a layer can never be zero-thickness.
            if is_framing {
                let depth = layer
                    .framing
                    .as_ref()
                    .map(|framing| framing.member)
                    .unwrap_or(BoardProfile::TwoByFour);
                summary_row(
                    ui,
                    "Thickness",
                    format!("follows {} = {}", depth.label(), layer.thickness),
                );
            } else {
                changed |= length_drag(ui, "Thickness", &mut layer.thickness, 0.0625, 48.0, "in");
            }

            // Function. Switching to/from Framing keeps `framing` consistent so the
            // model stays valid (framing.is_some() iff function == Framing). A Wall
            // system must keep exactly one framing layer, so the picker never offers
            // a SECOND Framing layer (`another_layer_is_framing`) and never lets the
            // SOLE framing layer drop Framing (`is_only_framing`) — either would
            // leave a Wall system with the wrong framing-layer count.
            let another_layer_is_framing = !is_framing && framing_count >= 1;
            changed |= property_row(ui, "Function", |ui| {
                let before = layer.function;
                ComboBox::from_id_salt(("layer-function", index))
                    .selected_text(layer.function.label())
                    .show_ui(ui, |ui| {
                        for function in LayerFunction::ALL {
                            let is_framing_option = function == LayerFunction::Framing;
                            if is_framing_option && another_layer_is_framing {
                                continue;
                            }
                            // The sole framing layer of a Wall system can only stay
                            // Framing; non-Framing choices would zero out the count.
                            if !is_framing_option && is_only_framing {
                                continue;
                            }
                            ui.selectable_value(&mut layer.function, function, function.label());
                        }
                    });
                if layer.function != before {
                    if layer.function == LayerFunction::Framing && layer.framing.is_none() {
                        let member = BoardProfile::TwoByFour;
                        layer.framing = Some(framer_core::FramingSpec {
                            member,
                            spacing: Length::from_whole_inches(16),
                            pattern: FramingPattern::Single,
                            cavity_material: None,
                        });
                        // Depth follows the member so geometry and BOM agree.
                        layer.thickness = member.nominal_depth();
                    } else if layer.function != LayerFunction::Framing {
                        layer.framing = None;
                    }
                    true
                } else {
                    false
                }
            });

            // Framing detail, revealed only for the Framing layer. A member change
            // also re-syncs the layer depth (applied after the `framing` borrow
            // ends) so geometry and BOM agree.
            let mut new_member_depth = None;
            if let Some(framing) = layer.framing.as_mut() {
                changed |= property_row(ui, "Member", |ui| {
                    let before = framing.member;
                    ComboBox::from_id_salt(("layer-member", index))
                        .selected_text(framing.member.label())
                        .show_ui(ui, |ui| {
                            for profile in BOARD_PROFILES {
                                ui.selectable_value(&mut framing.member, profile, profile.label());
                            }
                        });
                    if framing.member != before {
                        new_member_depth = Some(framing.member.nominal_depth());
                        true
                    } else {
                        false
                    }
                });
                changed |= length_drag(ui, "Spacing", &mut framing.spacing, 1.0, 48.0, "in");
                changed |= property_row(ui, "Pattern", |ui| {
                    let before = framing.pattern;
                    ComboBox::from_id_salt(("layer-pattern", index))
                        .selected_text(framing.pattern.label())
                        .show_ui(ui, |ui| {
                            for pattern in FramingPattern::ALL {
                                ui.selectable_value(&mut framing.pattern, pattern, pattern.label());
                            }
                        });
                    framing.pattern != before
                });
                changed |= property_row(ui, "Cavity fill", |ui| {
                    let selected = framing
                        .cavity_material
                        .as_ref()
                        .map(|id| material_name(material_options, &id.0))
                        .unwrap_or_else(|| "None".to_owned());
                    let before = framing.cavity_material.clone();
                    ComboBox::from_id_salt(("layer-cavity", index))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut framing.cavity_material, None, "None");
                            for (id, name) in material_options {
                                ui.selectable_value(
                                    &mut framing.cavity_material,
                                    Some(ElementId::new(id.clone())),
                                    name,
                                );
                            }
                        });
                    framing.cavity_material != before
                });
            }
            // Re-sync the framing layer depth to the chosen member's nominal depth.
            if let Some(depth) = new_member_depth {
                layer.thickness = depth;
            }
        });
    changed
}

/// Read-only summary of a construction system (Plan workspace).
fn system_summary(
    ui: &mut Ui,
    system: &framer_core::ConstructionSystem,
    r_value_milli: Option<i64>,
) {
    egui::Grid::new("system-summary")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Name", &system.name);
            summary_row(ui, "Kind", system.kind.label());
            summary_row(ui, "Layers", system.layers.len());
            summary_row(ui, "Total thickness", system.total_thickness().to_string());
            summary_row(ui, "Exposure", system.exposure().label());
            if let Some(milli) = r_value_milli {
                summary_row(ui, "R-value", format_r_value(milli));
            }
        });
}

/// Appearance editor for a material. Returns whether it changed. Currently the
/// only `Appearance` variant is `SolidColor`, edited via an sRGB color button.
fn material_appearance_editor(ui: &mut Ui, material: &mut framer_core::Material) -> bool {
    let mut changed = false;
    property_row(ui, "Appearance", |ui| match &mut material.appearance {
        framer_core::Appearance::SolidColor(rgb) => {
            changed |= ui.color_edit_button_srgb(rgb).changed();
        }
    });
    changed
}

/// Editor for the well-known material property keys surfaced as labelled
/// `i64` drag values. Returns whether any property changed.
fn material_properties_editor(ui: &mut Ui, material: &mut framer_core::Material) -> bool {
    use framer_core::PropertyValue;
    let mut changed = false;
    for (key, label) in [
        ("r_per_inch_milli", "R / inch (milli-R)"),
        ("cost_cents", "Cost (cents)"),
    ] {
        let mut value = match material.properties.get(key) {
            Some(PropertyValue::Int(v)) => *v,
            _ => 0,
        };
        let response = property_row(ui, label, |ui| {
            ui.add(
                egui::DragValue::new(&mut value)
                    .speed(1.0)
                    .range(0..=i64::MAX),
            )
        });
        if response.changed() {
            material
                .properties
                .insert(key.to_owned(), PropertyValue::Int(value));
            changed = true;
        }
    }
    changed
}

/// Read-only summary of a material (Plan workspace).
fn material_summary(ui: &mut Ui, material: &framer_core::Material) {
    ui.horizontal(|ui| {
        let [r, g, b] = material.color();
        color_swatch(ui, Color32::from_rgb(r, g, b));
        ui.label(
            RichText::new(&material.name)
                .strong()
                .color(theme::text_primary()),
        );
    });
    egui::Grid::new("material-summary")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "R / inch", format_r_value(material.r_per_inch_milli()));
            if !material.tags.is_empty() {
                summary_row(ui, "Tags", material.tags.join(", "));
            }
        });
}

fn wall_summary(ui: &mut Ui, wall: &Wall, level_options: &[(String, String)], system_name: &str) {
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
            summary_row(ui, "System", system_name);
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
    let wall_height_inches = wall.height.inches().max(1.0);
    let mut changed = false;
    let mut apply_driving = false;
    let axis_changed;

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

        let previous_axis = dimension.axis;
        ComboBox::from_label("Axis")
            .selected_text(dimension_axis_label(dimension.axis))
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(&mut dimension.axis, DimensionAxis::Horizontal, "Horizontal")
                    .changed();
                changed |= ui
                    .selectable_value(&mut dimension.axis, DimensionAxis::Vertical, "Vertical")
                    .changed();
            });
        axis_changed = dimension.axis != previous_axis;

        egui::Grid::new("dimension-inspector")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                summary_row(ui, "Axis", dimension_axis_label(dimension.axis));
                summary_row(ui, "From", &start_label);
                summary_row(ui, "To", &end_label);
                summary_row(ui, "Measured", measured.to_string());
            });

        if dimension.kind == DimensionKind::Driving {
            let mut value = dimension.value.unwrap_or(measured);
            let axis_bound_inches = match dimension.axis {
                DimensionAxis::Horizontal => wall_length_inches,
                DimensionAxis::Vertical => wall_height_inches,
            };
            if length_drag(ui, "Distance", &mut value, 1.0, axis_bound_inches, "in") {
                dimension.value = Some(value);
                changed = true;
                apply_driving = true;
            }
            if unsatisfied {
                ui.colored_label(theme::danger(), "Unsatisfied driving dimension");
            }
        }

        ui.separator();
        if ui.button("Remove Dimension").clicked() {
            *remove = true;
        }
    }

    if axis_changed {
        changed = true;
        if wall.dimensions[dimension_index].kind == DimensionKind::Driving {
            let measured = wall
                .dimension_measurement(&wall.dimensions[dimension_index])
                .unwrap_or(Length::ZERO)
                .max(Length::from_whole_inches(1));
            wall.dimensions[dimension_index].value = Some(measured);
            apply_driving = true;
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
            summary_row(ui, "Axis", dimension_axis_label(dimension.axis));
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
    ui.label(RichText::new(label).strong().color(theme::text_secondary()));
    ui.label(RichText::new(value.to_string()).color(theme::text_primary()));
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
        ui.colored_label(theme::danger(), error);
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
        DiagnosticSeverity::Info => theme::active_blue(),
        DiagnosticSeverity::Warning => theme::warning(),
        DiagnosticSeverity::Unsupported => theme::danger(),
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
        ui.label(RichText::new("Lumber").color(theme::text_secondary()));
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

        let layers = plan.layer_bom();
        if !layers.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new("Materials").color(theme::text_secondary()));
            egui::Grid::new("layer-bom-grid")
                .num_columns(5)
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Material");
                    ui.strong("Function");
                    ui.strong("Thickness");
                    ui.strong("Area");
                    ui.strong("Volume");
                    ui.end_row();

                    for item in layers {
                        ui.label(&item.material_name);
                        ui.label(item.function.label());
                        ui.label(item.thickness.to_string());
                        if item.area_sq_in > 0 {
                            ui.label(format!("{:.0} sq ft", item.area_sq_in as f64 / 144.0));
                        } else {
                            ui.label("—");
                        }
                        if item.volume_bd_in > 0 {
                            ui.label(format!("{:.1} cu ft", item.volume_bd_in as f64 / 1728.0));
                        } else {
                            ui.label("—");
                        }
                        ui.end_row();
                    }
                });
        }
    }
}

fn room_schedule_panel(ui: &mut Ui, plan: Option<&ProjectFramePlan>) {
    panel_subheader(ui, "Room schedule");
    if let Some(plan) = plan {
        egui::Grid::new("room-schedule-grid")
            .num_columns(4)
            .spacing([12.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Room");
                ui.strong("Usage");
                ui.strong("Area");
                ui.strong("Perimeter");
                ui.end_row();

                for room in &plan.rooms {
                    ui.label(&room.name);
                    ui.label(&room.usage);
                    if room.closed {
                        ui.label(format!("{:.0} sq ft", room.area_square_feet()));
                        ui.label(room.perimeter.to_string());
                    } else {
                        ui.colored_label(theme::text_secondary(), "open");
                        ui.label("—");
                    }
                    ui.end_row();
                }
            });
    }
}

const PROPERTY_LABEL_WIDTH: f32 = 92.0;

fn property_row<R>(ui: &mut Ui, label: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized(
            [PROPERTY_LABEL_WIDTH, 22.0],
            egui::Label::new(RichText::new(label).color(theme::text_secondary())),
        );
        add_contents(ui)
    })
    .inner
}

fn text_edit(ui: &mut Ui, label: &str, value: &mut String) -> bool {
    property_row(ui, label, |ui| {
        let width = ui.available_width().max(90.0);
        ui.add_sized([width, 24.0], egui::TextEdit::singleline(value))
    })
    .changed()
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
        DimensionAnchor::WallPoint {
            horizontal,
            vertical,
        } => format!(
            "Wall {}",
            point_anchor_label(*horizontal, *vertical).to_ascii_lowercase()
        ),
        DimensionAnchor::OpeningPoint {
            opening,
            horizontal,
            vertical,
        } => format!(
            "{} {}",
            opening_display_name(wall, &opening.0),
            point_anchor_label(*horizontal, *vertical).to_ascii_lowercase()
        ),
    }
}

fn point_anchor_label(
    horizontal: DimensionHorizontalReference,
    vertical: DimensionVerticalReference,
) -> &'static str {
    match (horizontal, vertical) {
        (DimensionHorizontalReference::Left, DimensionVerticalReference::Bottom) => "Bottom left",
        (DimensionHorizontalReference::Center, DimensionVerticalReference::Bottom) => "Bottom edge",
        (DimensionHorizontalReference::Right, DimensionVerticalReference::Bottom) => "Bottom right",
        (DimensionHorizontalReference::Left, DimensionVerticalReference::Center) => "Left edge",
        (DimensionHorizontalReference::Center, DimensionVerticalReference::Center) => "Center",
        (DimensionHorizontalReference::Right, DimensionVerticalReference::Center) => "Right edge",
        (DimensionHorizontalReference::Left, DimensionVerticalReference::Top) => "Top left",
        (DimensionHorizontalReference::Center, DimensionVerticalReference::Top) => "Top edge",
        (DimensionHorizontalReference::Right, DimensionVerticalReference::Top) => "Top right",
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
    bottom: Option<DrivenField>,
    height: Option<DrivenField>,
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
    WallHeight,
    OpeningCenter(String),
    OpeningWidth(String),
    OpeningBottom(String),
    OpeningHeight(String),
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
        bottom: driven_length_field(
            wall,
            DimensionVariableKey::OpeningBottom(opening_id.to_owned()),
        ),
        height: driven_length_field(
            wall,
            DimensionVariableKey::OpeningHeight(opening_id.to_owned()),
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
        DimensionVariableKey::WallHeight => {
            alternate_length(value, Length::from_whole_inches(48), value + step, step)
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
        DimensionVariableKey::OpeningBottom(opening_id) => {
            let opening = wall
                .openings
                .iter()
                .find(|opening| opening.id.0 == *opening_id)?;
            alternate_length(value, Length::ZERO, wall.height - opening.height, step)
        }
        DimensionVariableKey::OpeningHeight(opening_id) => {
            let opening = wall
                .openings
                .iter()
                .find(|opening| opening.id.0 == *opening_id)?;
            alternate_length(
                value,
                Length::from_whole_inches(12),
                wall.height - opening.sill_height,
                step,
            )
        }
    }
}

fn dimension_variable_value(wall: &Wall, key: &DimensionVariableKey) -> Option<Length> {
    match key {
        DimensionVariableKey::WallLength => Some(wall.length),
        DimensionVariableKey::WallHeight => Some(wall.height),
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
        DimensionVariableKey::OpeningBottom(opening_id) => wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == *opening_id)
            .map(|opening| opening.sill_height),
        DimensionVariableKey::OpeningHeight(opening_id) => wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == *opening_id)
            .map(|opening| opening.height),
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
        DimensionVariableKey::WallHeight => {
            wall.height = value;
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
        DimensionVariableKey::OpeningBottom(opening_id) => {
            let Some(opening) = wall
                .openings
                .iter_mut()
                .find(|opening| opening.id.0 == *opening_id)
            else {
                return false;
            };
            opening.sill_height = value;
            true
        }
        DimensionVariableKey::OpeningHeight(opening_id) => {
            let Some(opening) = wall
                .openings
                .iter_mut()
                .find(|opening| opening.id.0 == *opening_id)
            else {
                return false;
            };
            opening.height = value;
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
    add_anchor_variables(&dimension.start, dimension.axis, &mut variables);
    add_anchor_variables(&dimension.end, dimension.axis, &mut variables);
    variables
}

fn add_anchor_variables(
    anchor: &DimensionAnchor,
    axis: DimensionAxis,
    variables: &mut BTreeSet<DimensionVariableKey>,
) {
    match axis {
        DimensionAxis::Horizontal => match anchor {
            DimensionAnchor::WallStart => {}
            DimensionAnchor::WallEnd => {
                variables.insert(DimensionVariableKey::WallLength);
            }
            DimensionAnchor::WallPoint { horizontal, .. } => {
                add_horizontal_wall_variables(*horizontal, variables);
            }
            DimensionAnchor::OpeningCenter { opening } => {
                variables.insert(DimensionVariableKey::OpeningCenter(opening.0.clone()));
            }
            DimensionAnchor::OpeningLeft { opening }
            | DimensionAnchor::OpeningRight { opening } => {
                add_horizontal_opening_variables(&opening.0, variables);
            }
            DimensionAnchor::OpeningPoint {
                opening,
                horizontal,
                ..
            } => {
                add_horizontal_opening_variables_for_reference(&opening.0, *horizontal, variables);
            }
        },
        DimensionAxis::Vertical => match anchor {
            DimensionAnchor::WallStart | DimensionAnchor::WallEnd => {}
            DimensionAnchor::WallPoint { vertical, .. } => {
                add_vertical_wall_variables(*vertical, variables);
            }
            DimensionAnchor::OpeningLeft { opening }
            | DimensionAnchor::OpeningCenter { opening }
            | DimensionAnchor::OpeningRight { opening } => {
                add_vertical_opening_variables_for_reference(
                    &opening.0,
                    DimensionVerticalReference::Center,
                    variables,
                );
            }
            DimensionAnchor::OpeningPoint {
                opening, vertical, ..
            } => {
                add_vertical_opening_variables_for_reference(&opening.0, *vertical, variables);
            }
        },
    }
}

fn add_horizontal_wall_variables(
    horizontal: DimensionHorizontalReference,
    variables: &mut BTreeSet<DimensionVariableKey>,
) {
    match horizontal {
        DimensionHorizontalReference::Left => {}
        DimensionHorizontalReference::Center | DimensionHorizontalReference::Right => {
            variables.insert(DimensionVariableKey::WallLength);
        }
    }
}

fn add_vertical_wall_variables(
    vertical: DimensionVerticalReference,
    variables: &mut BTreeSet<DimensionVariableKey>,
) {
    match vertical {
        DimensionVerticalReference::Bottom => {}
        DimensionVerticalReference::Center | DimensionVerticalReference::Top => {
            variables.insert(DimensionVariableKey::WallHeight);
        }
    }
}

fn add_horizontal_opening_variables(id: &str, variables: &mut BTreeSet<DimensionVariableKey>) {
    variables.insert(DimensionVariableKey::OpeningCenter(id.to_owned()));
    variables.insert(DimensionVariableKey::OpeningWidth(id.to_owned()));
}

fn add_horizontal_opening_variables_for_reference(
    id: &str,
    horizontal: DimensionHorizontalReference,
    variables: &mut BTreeSet<DimensionVariableKey>,
) {
    variables.insert(DimensionVariableKey::OpeningCenter(id.to_owned()));
    if !matches!(horizontal, DimensionHorizontalReference::Center) {
        variables.insert(DimensionVariableKey::OpeningWidth(id.to_owned()));
    }
}

fn add_vertical_opening_variables_for_reference(
    id: &str,
    vertical: DimensionVerticalReference,
    variables: &mut BTreeSet<DimensionVariableKey>,
) {
    variables.insert(DimensionVariableKey::OpeningBottom(id.to_owned()));
    if !matches!(vertical, DimensionVerticalReference::Bottom) {
        variables.insert(DimensionVariableKey::OpeningHeight(id.to_owned()));
    }
}

fn driving_dimension_source_label(wall: &Wall, dimension: &DimensionConstraint) -> String {
    let mut label = format!(
        "{}: {} {} to {}",
        dimension.name,
        dimension_axis_label(dimension.axis).to_ascii_lowercase(),
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

    property_row(ui, label, |ui| {
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
        RichText::new(label).size(11.0).color(theme::warning()),
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

    let response = property_row(ui, label, |ui| {
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

    if response.changed() {
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
    let response = property_row(ui, label, |ui| {
        ui.add(
            egui::DragValue::new(&mut display_value)
                .range(-240.0..=240.0)
                .speed(0.25)
                .suffix(" ft"),
        )
    });

    if response.changed() {
        *value = Length::from_feet(display_value.clamp(-240.0, 240.0));
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use framer_core::{
        CodeProfile, DimensionAxis, DimensionDirection, DimensionHorizontalReference,
        DimensionVerticalReference,
    };

    #[test]
    fn captures_edit_base_on_click_release_frame() {
        // A ComboBox selection or button commits on the pointer-release frame:
        // pointer is already up, but a click occurred. Without this, the whole
        // class of dropdown/button inspector edits records no undo step.
        assert!(should_capture_edit_base(false, false, true, false));
    }

    #[test]
    fn captures_edit_base_while_dragging_or_focused() {
        assert!(should_capture_edit_base(false, true, false, false), "drag");
        assert!(
            should_capture_edit_base(false, false, false, true),
            "focused text field"
        );
    }

    #[test]
    fn skips_edit_base_when_idle_or_pending() {
        assert!(
            !should_capture_edit_base(false, false, false, false),
            "idle frame: no clone"
        );
        assert!(
            !should_capture_edit_base(true, true, true, true),
            "transaction already open: keep the original base"
        );
    }

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

    #[test]
    fn vertical_height_dimension_marks_opening_height_driven() {
        let mut wall = wall_with_window(Length::from_feet(6.0), Length::from_feet(3.0));
        wall.dimensions.push(
            driving_dimension(
                "height",
                DimensionAnchor::OpeningPoint {
                    opening: ElementId::new("window"),
                    horizontal: DimensionHorizontalReference::Center,
                    vertical: DimensionVerticalReference::Bottom,
                },
                DimensionAnchor::OpeningPoint {
                    opening: ElementId::new("window"),
                    horizontal: DimensionHorizontalReference::Center,
                    vertical: DimensionVerticalReference::Top,
                },
                Length::from_feet(4.0),
            )
            .with_axis(DimensionAxis::Vertical),
        );
        wall.apply_driving_dimensions();

        let fields = opening_driven_fields(&wall, "window");

        assert_eq!(
            fields
                .height
                .as_ref()
                .map(|driver| driver.dimension_ids.clone()),
            Some(vec!["height".to_owned()])
        );
        assert!(fields.bottom.is_none());
    }

    #[test]
    fn vertical_bottom_offset_marks_opening_bottom_driven() {
        let mut wall = wall_with_window(Length::from_feet(6.0), Length::from_feet(3.0));
        wall.dimensions.push(
            driving_dimension(
                "bottom-offset",
                DimensionAnchor::WallPoint {
                    horizontal: DimensionHorizontalReference::Left,
                    vertical: DimensionVerticalReference::Bottom,
                },
                DimensionAnchor::OpeningPoint {
                    opening: ElementId::new("window"),
                    horizontal: DimensionHorizontalReference::Center,
                    vertical: DimensionVerticalReference::Bottom,
                },
                Length::from_feet(4.0),
            )
            .with_axis(DimensionAxis::Vertical),
        );
        wall.apply_driving_dimensions();

        let fields = opening_driven_fields(&wall, "window");

        assert_eq!(
            fields
                .bottom
                .as_ref()
                .map(|driver| driver.dimension_ids.clone()),
            Some(vec!["bottom-offset".to_owned()])
        );
        assert!(fields.height.is_none());
    }
}
