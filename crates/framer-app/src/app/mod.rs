mod labels;
mod model_edit;
mod panels;
mod project_io;
mod viewport;

use std::fs;
use std::path::PathBuf;

use eframe::egui::{self, CentralPanel, Panel, ScrollArea};
use framer_core::{
    BuildingModel, Length, Opening, OpeningKind, Wall, load_project as load_project_document,
    save_project as save_project_document,
};
use framer_solver::{
    FrameMember, ProjectFramePlan, export_bom_csv, export_project_svg, generate_project_plan,
};

use model_edit::next_opening_id;
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
    viewport_mode: ViewportMode,
    view_3d: View3dState,
    show_section: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Level(String),
    Wall,
    Opening(String),
    Join(String),
    Member { wall_id: String, member_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewportMode {
    Plan,
    Elevation,
    Axonometric,
}

enum ViewClick {
    Wall(usize),
    Opening {
        wall_index: usize,
        opening_id: String,
    },
    Member {
        wall_id: String,
        member_id: String,
    },
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
            viewport_mode: ViewportMode::Plan,
            view_3d: View3dState::default(),
            show_section: true,
        };
        app.rebuild();
        app
    }
}

impl FramerApp {
    fn rebuild(&mut self) {
        if self.selected_wall >= self.model.walls.len() {
            self.selected_wall = 0;
            self.selected = Selection::Wall;
        }

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
        self.rebuild();
    }

    fn reset_demo(&mut self) {
        self.model = BuildingModel::demo_shell();
        self.selected_wall = 0;
        self.selected = Selection::Wall;
        self.project_path = DEFAULT_PROJECT_PATH.to_owned();
        self.file_status = Some("Reset to multi-wall demo shell".to_owned());
        self.artifact_status = None;
        self.rebuild();
    }

    fn reset_wall_demo(&mut self) {
        self.model = BuildingModel::demo_wall();
        self.selected_wall = 0;
        self.selected = Selection::Wall;
        self.project_path = "examples/projects/demo-wall.framer".to_owned();
        self.file_status = Some("Reset to Phase 1 demo wall".to_owned());
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

    use super::*;

    #[test]
    fn app_saves_and_reopens_demo_project() {
        let path = std::env::temp_dir().join(format!("framer-demo-wall-{}.framer", process::id()));
        let mut app = FramerApp::default();
        app.project_path = path.display().to_string();

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
        let mut app = FramerApp::default();
        app.project_path = path.display().to_string();

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
}
