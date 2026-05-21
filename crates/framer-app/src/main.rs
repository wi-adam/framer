use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align2, CentralPanel, Color32, ComboBox, FontId, Panel, Pos2, Rect, ScrollArea, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};
use framer_core::{
    BuildingModel, Length, Opening, OpeningKind, Wall, load_project as load_project_document,
    save_project as save_project_document,
};
use framer_solver::{
    DiagnosticSeverity, FrameMember, MemberKind, MemberOrientation, PlanDiagnostic, WallFramePlan,
    export_bom_csv, export_wall_elevation_svg, generate_wall_plan,
};

const DEFAULT_PROJECT_PATH: &str = "examples/projects/demo-wall.framer";

fn main() -> eframe::Result {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([1040.0, 680.0])
            .with_title("Framer"),
        ..Default::default()
    };

    eframe::run_native(
        "Framer",
        options,
        Box::new(|_cc| Ok(Box::<FramerApp>::default())),
    )
}

struct FramerApp {
    model: BuildingModel,
    selected_wall: usize,
    selected: Selection,
    wall_plan: Option<WallFramePlan>,
    error: Option<String>,
    project_path: String,
    file_status: Option<String>,
    artifact_status: Option<String>,
    viewport_mode: ViewportMode,
    show_section: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Wall,
    Opening(String),
    Member(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewportMode {
    Elevation,
    Axonometric,
}

impl Default for FramerApp {
    fn default() -> Self {
        let mut app = Self {
            model: BuildingModel::demo_wall(),
            selected_wall: 0,
            selected: Selection::Wall,
            wall_plan: None,
            error: None,
            project_path: DEFAULT_PROJECT_PATH.to_owned(),
            file_status: None,
            artifact_status: None,
            viewport_mode: ViewportMode::Elevation,
            show_section: true,
        };
        app.rebuild();
        app
    }
}

impl FramerApp {
    fn rebuild(&mut self) {
        let Some(wall) = self.model.walls.get(self.selected_wall) else {
            self.wall_plan = None;
            self.error = Some("No wall selected".to_owned());
            return;
        };

        match generate_wall_plan(wall, &self.model.code) {
            Ok(plan) => {
                self.wall_plan = Some(plan);
                self.error = None;
            }
            Err(error) => {
                self.wall_plan = None;
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
        self.rebuild();
    }

    fn reset_demo(&mut self) {
        self.model = BuildingModel::demo_wall();
        self.selected_wall = 0;
        self.selected = Selection::Wall;
        self.project_path = DEFAULT_PROJECT_PATH.to_owned();
        self.file_status = Some("Reset to demo wall".to_owned());
        self.artifact_status = None;
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
                self.rebuild();
                self.file_status = Some(format!("Opened {}", path.display()));
                self.artifact_status = None;
            }
            Err(error) => {
                self.file_status = Some(format!("Open failed: {error}"));
            }
        }
    }

    fn export_current_artifacts(&mut self) {
        let Some(wall) = self.model.walls.get(self.selected_wall) else {
            self.artifact_status = Some("Export failed: no wall selected".to_owned());
            return;
        };

        let Some(plan) = &self.wall_plan else {
            self.artifact_status =
                Some("Export failed: regenerate a valid framing plan first".to_owned());
            return;
        };

        let (svg_path, csv_path) = export_paths(&self.project_path);
        let svg = export_wall_elevation_svg(wall, plan);
        let csv = export_bom_csv(&plan.bom());

        let result = write_text_file(&svg_path, svg).and_then(|()| write_text_file(&csv_path, csv));
        self.artifact_status = Some(match result {
            Ok(()) => format!("Exported {} and {}", svg_path.display(), csv_path.display()),
            Err(error) => format!("Export failed: {error}"),
        });
    }

    fn add_opening(&mut self, kind: OpeningKind) {
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

    fn selected_member(&self, id: &str) -> Option<&FrameMember> {
        self.wall_plan
            .as_ref()?
            .members
            .iter()
            .find(|member| member.id == id)
    }
}

impl eframe::App for FramerApp {
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
            .show_inside(ui, |ui| self.inspector(ui));
        CentralPanel::default().show_inside(ui, |ui| self.workspace(ui));
    }
}

impl FramerApp {
    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("Framer");
            ui.separator();
            if ui.button("New").clicked() {
                self.new_project();
            }
            if ui.button("Demo").clicked() {
                self.reset_demo();
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

    fn model_tree(&mut self, ui: &mut Ui) {
        ui.heading("Model Tree");
        ui.separator();

        ScrollArea::vertical().show(ui, |ui| {
            egui::CollapsingHeader::new("Authored")
                .default_open(true)
                .show(ui, |ui| {
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
                                wall.openings
                                    .iter()
                                    .map(|opening| {
                                        (opening.id.0.clone(), opening.kind, opening.name.clone())
                                    })
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect();

                    for (index, wall_id, wall_name, openings) in walls {
                        let wall_selected =
                            self.selected_wall == index && matches!(self.selected, Selection::Wall);
                        if ui
                            .selectable_label(wall_selected, format!("Wall: {wall_name}"))
                            .clicked()
                        {
                            self.selected_wall = index;
                            self.selected = Selection::Wall;
                            self.rebuild();
                        }

                        ui.indent(format!("wall-{wall_id}"), |ui| {
                            for (opening_id, opening_kind, opening_name) in openings {
                                let selected = matches!(
                                    &self.selected,
                                    Selection::Opening(id) if id == &opening_id
                                );
                                if ui
                                    .selectable_label(
                                        selected,
                                        format!("{}: {}", kind_label(opening_kind), opening_name),
                                    )
                                    .clicked()
                                {
                                    self.selected_wall = index;
                                    self.selected = Selection::Opening(opening_id);
                                    self.rebuild();
                                }
                            }
                        });
                    }
                });

            egui::CollapsingHeader::new("Generated")
                .default_open(true)
                .show(ui, |ui| {
                    if let Some(plan) = &self.wall_plan {
                        for member in &plan.members {
                            let selected = matches!(
                                &self.selected,
                                Selection::Member(id) if id == &member.id
                            );
                            if ui
                                .selectable_label(
                                    selected,
                                    format!("{}: {}", member.kind.label(), member.id),
                                )
                                .clicked()
                            {
                                self.selected = Selection::Member(member.id.clone());
                            }
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

    fn inspector(&mut self, ui: &mut Ui) {
        let mut changed = false;
        let selection = self.selected.clone();

        ui.heading("Inspector");
        ui.separator();

        match selection {
            Selection::Wall => {
                if let Some(wall) = self.model.walls.get_mut(self.selected_wall) {
                    ui.label(&wall.id.0);
                    changed |= text_edit(ui, "Name", &mut wall.name);
                    changed |= length_drag(ui, "Length", &mut wall.length, 24.0, 480.0, "ft");
                    changed |= length_drag(ui, "Height", &mut wall.height, 48.0, 168.0, "ft");
                    changed |=
                        length_drag(ui, "Stud spacing", &mut wall.stud_spacing, 8.0, 32.0, "in");
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
            Selection::Member(id) => {
                if let Some(member) = self.selected_member(&id) {
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
        diagnostics_panel(ui, self.error.as_deref(), self.wall_plan.as_ref());
        ui.separator();
        bom_panel(ui, self.wall_plan.as_ref());
    }

    fn workspace(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("CAD Workspace");
            ui.separator();
            ui.label(self.model.code.display_name.as_str());
        });
        ui.add_space(8.0);

        let Some(wall) = self.model.walls.get(self.selected_wall) else {
            ui.label("No wall selected");
            return;
        };
        let Some(plan) = &self.wall_plan else {
            ui.label("No valid framing plan");
            return;
        };

        let selected_member = match &self.selected {
            Selection::Member(id) => Some(id.as_str()),
            _ => None,
        };
        let section_x = if self.show_section {
            section_position(wall, &self.selected)
        } else {
            None
        };

        let clicked = match self.viewport_mode {
            ViewportMode::Elevation => {
                draw_wall_elevation(ui, wall, &plan.members, selected_member, section_x)
            }
            ViewportMode::Axonometric => {
                draw_wall_axonometric(ui, wall, &plan.members, selected_member, section_x)
            }
        };

        if let Some(member_id) = clicked {
            self.selected = Selection::Member(member_id);
        }
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
            ui.strong("Rule");
            ui.label(&member.provenance.rule_id);
            ui.end_row();
        });
    ui.label(&member.provenance.summary);
}

fn diagnostics_panel(ui: &mut Ui, error: Option<&str>, plan: Option<&WallFramePlan>) {
    ui.heading("Diagnostics");
    if let Some(error) = error {
        ui.colored_label(Color32::from_rgb(185, 65, 65), error);
    }

    if let Some(plan) = plan {
        for diagnostic in &plan.diagnostics {
            diagnostic_row(ui, diagnostic);
        }
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

fn bom_panel(ui: &mut Ui, plan: Option<&WallFramePlan>) {
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

fn next_opening_id(wall: &Wall, prefix: &str) -> (String, usize) {
    let mut index = wall.openings.len() + 1;
    loop {
        let id = format!("{prefix}-{index}");
        if wall.openings.iter().all(|opening| opening.id.0 != id) {
            return (id, index);
        }
        index += 1;
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

    painter.rect_filled(rect, 0.0, Color32::from_rgb(246, 244, 239));
    painter.rect_stroke(
        drawing,
        0.0,
        Stroke::new(1.0, Color32::from_rgb(190, 184, 172)),
        StrokeKind::Outside,
    );

    let sx = drawing.width() / wall.length.inches().max(1.0) as f32;
    let sy = drawing.height() / wall.height.inches().max(1.0) as f32;
    let pointer = response.interact_pointer_pos();
    let mut clicked = None;

    for member in members {
        let member_rect = member_rect(drawing, sx, sy, member);
        let hovered = pointer.is_some_and(|position| member_rect.contains(position));
        let selected = selected_member == Some(member.id.as_str());
        draw_member_rect(&painter, member_rect, member.kind, selected, hovered);
        if hovered && response.clicked() {
            clicked = Some(member.id.clone());
        }
    }

    if let Some(section_x) = section_x {
        draw_section_line(&painter, drawing, sx, section_x);
    }

    painter.text(
        Pos2::new(drawing.left(), drawing.bottom() + 20.0),
        Align2::LEFT_CENTER,
        format!("{} x {}", wall.length, wall.height),
        FontId::proportional(13.0),
        Color32::from_rgb(70, 67, 61),
    );

    clicked
}

fn draw_wall_axonometric(
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

    painter.rect_filled(rect, 0.0, Color32::from_rgb(241, 244, 241));
    let margin = 62.0;
    let depth = Vec2::new(38.0, -28.0);
    let drawing = Rect::from_min_max(
        rect.min + Vec2::new(margin, margin + 28.0),
        rect.max - Vec2::new(margin + depth.x, margin),
    );
    let back = drawing.translate(depth);

    painter.rect_filled(back, 0.0, Color32::from_rgb(226, 229, 222));
    for (front, rear) in [
        (drawing.left_top(), back.left_top()),
        (drawing.right_top(), back.right_top()),
        (drawing.left_bottom(), back.left_bottom()),
        (drawing.right_bottom(), back.right_bottom()),
    ] {
        painter.line_segment(
            [front, rear],
            Stroke::new(1.0, Color32::from_rgb(174, 181, 171)),
        );
    }
    painter.rect_stroke(
        drawing,
        0.0,
        Stroke::new(1.0, Color32::from_rgb(150, 158, 147)),
        StrokeKind::Outside,
    );

    let sx = drawing.width() / wall.length.inches().max(1.0) as f32;
    let sy = drawing.height() / wall.height.inches().max(1.0) as f32;
    let pointer = response.interact_pointer_pos();
    let mut clicked = None;

    for member in members {
        let member_rect = member_rect(drawing, sx, sy, member);
        let shadow = member_rect.translate(depth * 0.45);
        painter.rect_filled(
            shadow,
            1.0,
            Color32::from_rgba_unmultiplied(90, 100, 90, 48),
        );
        let hovered = pointer.is_some_and(|position| member_rect.contains(position));
        let selected = selected_member == Some(member.id.as_str());
        draw_member_rect(&painter, member_rect, member.kind, selected, hovered);
        if hovered && response.clicked() {
            clicked = Some(member.id.clone());
        }
    }

    if let Some(section_x) = section_x {
        draw_section_line(&painter, drawing, sx, section_x);
    }

    clicked
}

fn member_rect(drawing: Rect, sx: f32, sy: f32, member: &FrameMember) -> Rect {
    let start_x = drawing.left() + member.x.inches() as f32 * sx;
    let start_y = drawing.bottom() - member.elevation.inches() as f32 * sy;

    match member.orientation {
        MemberOrientation::Horizontal => {
            let width = (member.cut_length.inches() as f32 * sx).max(2.0);
            let height = (member.profile.thickness().inches() as f32 * sy).max(3.0);
            Rect::from_min_size(
                Pos2::new(start_x, start_y - height),
                Vec2::new(width, height),
            )
        }
        MemberOrientation::Vertical => {
            let width = (member.profile.thickness().inches() as f32 * sx).max(3.0);
            let height = (member.cut_length.inches() as f32 * sy).max(2.0);
            Rect::from_min_size(
                Pos2::new(start_x - width / 2.0, start_y - height),
                Vec2::new(width, height),
            )
        }
    }
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
        MemberKind::CommonStud => Color32::from_rgb(186, 145, 94),
        MemberKind::KingStud => Color32::from_rgb(151, 100, 61),
        MemberKind::JackStud => Color32::from_rgb(211, 168, 95),
        MemberKind::Header => Color32::from_rgb(115, 130, 99),
        MemberKind::RoughSill => Color32::from_rgb(92, 121, 144),
        MemberKind::CrippleStud => Color32::from_rgb(218, 190, 139),
    }
}

fn kind_label(kind: OpeningKind) -> &'static str {
    match kind {
        OpeningKind::Door => "Door",
        OpeningKind::Window => "Window",
        OpeningKind::GarageDoor => "Garage door",
        OpeningKind::Skylight => "Skylight",
        OpeningKind::Stair => "Stair",
    }
}

fn diagnostic_code_prefix(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Info => "Info",
        DiagnosticSeverity::Warning => "Warning",
        DiagnosticSeverity::Unsupported => "Unsupported",
    }
}

fn section_position(wall: &Wall, selection: &Selection) -> Option<Length> {
    match selection {
        Selection::Opening(id) => wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == *id)
            .map(|opening| opening.center),
        Selection::Member(_) | Selection::Wall => Some(wall.length / 2),
    }
}

fn write_text_file(path: &Path, contents: String) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, contents).map_err(|error| error.to_string())
}

fn export_paths(project_path: &str) -> (PathBuf, PathBuf) {
    let trimmed = project_path.trim();
    let base = if trimmed.is_empty() {
        PathBuf::from("framer-export.framer")
    } else {
        PathBuf::from(trimmed)
    };
    (base.with_extension("svg"), base.with_extension("csv"))
}

#[cfg(test)]
mod tests {
    use std::process;

    use super::*;

    #[test]
    fn app_saves_and_reopens_demo_project() {
        let path = std::env::temp_dir().join(format!("framer-demo-wall-{}.framer", process::id()));
        let mut app = FramerApp::default();
        app.project_path = path.display().to_string();

        app.save_project_file();
        assert!(matches!(app.file_status.as_deref(), Some(status) if status.starts_with("Saved ")));

        app.model.walls[0].length = Length::from_feet(10.0);
        app.load_project_file();

        assert!(
            matches!(app.file_status.as_deref(), Some(status) if status.starts_with("Opened "))
        );
        assert_eq!(app.model, BuildingModel::demo_wall());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn app_exports_svg_and_csv_artifacts() {
        let path =
            std::env::temp_dir().join(format!("framer-demo-export-{}.framer", process::id()));
        let svg_path = path.with_extension("svg");
        let csv_path = path.with_extension("csv");
        let mut app = FramerApp::default();
        app.project_path = path.display().to_string();

        app.export_current_artifacts();

        assert!(
            matches!(app.artifact_status.as_deref(), Some(status) if status.starts_with("Exported "))
        );
        assert!(fs::read_to_string(&svg_path).unwrap().contains("<svg"));
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
}
