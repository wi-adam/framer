use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{
    self, Align, Color32, ComboBox, Frame, Layout, Margin, PopupCloseBehavior, Pos2, Response,
    RichText, ScrollArea, Stroke, Ui, Vec2,
    containers::menu::{MenuButton, MenuConfig},
};
use framer_core::{
    BuildingModel, CeilingSlope, DimensionAnchor, DimensionAxis, DimensionConstraint,
    DimensionHorizontalReference, DimensionKind, DimensionVerticalReference, ElementId,
    FurnishingInstance, Length, Level, MaterialSource, MepInstance, Opening, OpeningKind, Point2,
    Provenance, QuarterTurn, SeismicDesignCategory, Slope, SurfaceRegion, Wall, WallJoin,
    WallJoinKind,
};
use framer_geometry::{GeometryAudit, GeometryViolation};
use framer_solver::{DiagnosticSeverity, FrameMember, PlanDiagnostic, ProjectFramePlan};
use framer_standards::{ComplianceEntry, ComplianceReport, Outcome};

use super::actions::{self, ActionId, WorkflowTab};
use super::component_visibility::{
    AuthoredComponentKind, ComponentKey, IsolationMode, SelectionOp,
};
use super::design::{Icon, widgets};
use super::labels::{
    diagnostic_code_prefix, dimension_axis_label, dimension_kind_label, geometry_body_label,
    join_kind_label, kind_label,
};
use super::model_edit::{
    next_furnishing_instance_id, next_mep_instance_id, opening_max_bottom, opening_top_clearance,
    set_wall_length_keep_direction,
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
            if header_command_button(
                ui,
                head,
                ActionId::NewProject,
                self.action_enabled(ActionId::NewProject),
                self.action_disabled_reason(ActionId::NewProject),
            )
            .clicked()
            {
                self.execute_action(ActionId::NewProject);
            }
            if header_command_button(
                ui,
                head,
                ActionId::OpenProject,
                self.action_enabled(ActionId::OpenProject),
                self.action_disabled_reason(ActionId::OpenProject),
            )
            .clicked()
            {
                self.execute_action(ActionId::OpenProject);
            }
            if header_command_button(
                ui,
                head,
                ActionId::SaveProject,
                self.action_enabled(ActionId::SaveProject),
                self.action_disabled_reason(ActionId::SaveProject),
            )
            .clicked()
            {
                self.execute_action(ActionId::SaveProject);
            }
            header_divider(ui, head.divider);
            let undo_tip = match self.history.undo_label() {
                Some(label) => format!("Undo {label}  (⌘Z / Ctrl+Z)"),
                None => "Nothing to undo  (⌘Z / Ctrl+Z)".to_owned(),
            };
            if header_command_button(
                ui,
                head,
                ActionId::Undo,
                self.action_enabled(ActionId::Undo),
                Some(undo_tip.as_str()),
            )
            .clicked()
            {
                self.execute_action(ActionId::Undo);
            }
            let redo_tip = match self.history.redo_label() {
                Some(label) => format!("Redo {label}  (⌘⇧Z / Ctrl+Y)"),
                None => "Nothing to redo  (⌘⇧Z / Ctrl+Y)".to_owned(),
            };
            if header_command_button(
                ui,
                head,
                ActionId::Redo,
                self.action_enabled(ActionId::Redo),
                Some(redo_tip.as_str()),
            )
            .clicked()
            {
                self.execute_action(ActionId::Redo);
            }
            header_divider(ui, head.divider);
            self.project_header_menu(ui, head);
            self.examples_header_menu(ui, head);
            if header_command_button(
                ui,
                head,
                ActionId::CommandSearch,
                self.action_enabled(ActionId::CommandSearch),
                self.action_disabled_reason(ActionId::CommandSearch),
            )
            .clicked()
            {
                self.execute_action(ActionId::CommandSearch);
            }
            let path_width = (ui.available_width() * 0.34).clamp(220.0, 460.0);
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
                header_profile(
                    ui,
                    head,
                    self.model
                        .base_standards_name()
                        .unwrap_or("Standards starter pack"),
                );
            });
        });
    }

    fn project_header_menu(&mut self, ui: &mut Ui, head: design::Theme) {
        let (response, _) = MenuButton::from_button(widgets::header_menu_button("Project", head))
            .ui(ui, |ui| {
                ui.set_min_width(176.0);
                if header_menu_action(
                    ui,
                    ActionId::NewProject,
                    self.action_enabled(ActionId::NewProject),
                    self.action_disabled_reason(ActionId::NewProject),
                )
                .clicked()
                {
                    self.execute_action(ActionId::NewProject);
                    ui.close();
                }
                if header_menu_action(
                    ui,
                    ActionId::OpenProject,
                    self.action_enabled(ActionId::OpenProject),
                    self.action_disabled_reason(ActionId::OpenProject),
                )
                .clicked()
                {
                    self.execute_action(ActionId::OpenProject);
                    ui.close();
                }
                if header_menu_action(
                    ui,
                    ActionId::SaveProject,
                    self.action_enabled(ActionId::SaveProject),
                    self.action_disabled_reason(ActionId::SaveProject),
                )
                .clicked()
                {
                    self.execute_action(ActionId::SaveProject);
                    ui.close();
                }
                ui.separator();
                if header_menu_action(
                    ui,
                    ActionId::ExportArtifacts,
                    self.action_enabled(ActionId::ExportArtifacts),
                    self.action_disabled_reason(ActionId::ExportArtifacts),
                )
                .clicked()
                {
                    self.execute_action(ActionId::ExportArtifacts);
                    ui.close();
                }
                if header_menu_action(
                    ui,
                    ActionId::ExportComplianceReport,
                    self.action_enabled(ActionId::ExportComplianceReport),
                    self.action_disabled_reason(ActionId::ExportComplianceReport),
                )
                .clicked()
                {
                    self.execute_action(ActionId::ExportComplianceReport);
                    ui.close();
                }
            });
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Project"));
        response.on_hover_text("Project actions");
    }

    fn examples_header_menu(&mut self, ui: &mut Ui, head: design::Theme) {
        let (response, _) = MenuButton::from_button(widgets::header_menu_button("Examples", head))
            .ui(ui, |ui| {
                ui.set_min_width(176.0);
                if header_menu_action(
                    ui,
                    ActionId::LoadShellDemo,
                    self.action_enabled(ActionId::LoadShellDemo),
                    self.action_disabled_reason(ActionId::LoadShellDemo),
                )
                .clicked()
                {
                    self.execute_action(ActionId::LoadShellDemo);
                    ui.close();
                }
                if header_menu_action(
                    ui,
                    ActionId::LoadWallDemo,
                    self.action_enabled(ActionId::LoadWallDemo),
                    self.action_disabled_reason(ActionId::LoadWallDemo),
                )
                .clicked()
                {
                    self.execute_action(ActionId::LoadWallDemo);
                    ui.close();
                }
            });
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Examples"));
        response.on_hover_text("Example projects");
    }

    pub(super) fn toolbar(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(design::space::SM, design::space::SM);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = design::space::XS;
                for tab in AUTHORING_WORKFLOW_TABS {
                    if widgets::workflow_tab(ui, workflow_tab_label(*tab), self.command_tab == *tab)
                        .clicked()
                    {
                        self.select_workflow_tab(*tab);
                    }
                }
                ui.add_space(design::space::SM);
                toolbar_divider(ui);
                ui.add_space(design::space::SM);
                for tab in OUTPUT_WORKFLOW_TABS {
                    if widgets::workflow_tab(ui, workflow_tab_label(*tab), self.command_tab == *tab)
                        .clicked()
                    {
                        self.select_workflow_tab(*tab);
                    }
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(design::space::SM, design::space::SM);
                self.workflow_command_panels(ui);
            });
        });
    }

    pub(super) fn status_toast_overlay(&mut self, ui: &mut Ui, anchor: Pos2) {
        let Some(signature) = self.status_toast_signature() else {
            self.status_toast_signature = None;
            self.status_toast_until = 0.0;
            return;
        };

        let now = ui.ctx().input(|input| input.time);
        if self.status_toast_signature.as_deref() != Some(signature.as_str()) {
            self.status_toast_signature = Some(signature);
            self.status_toast_until = now + STATUS_TOAST_SECONDS;
        }
        if now > self.status_toast_until {
            return;
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(
                (self.status_toast_until - now).max(0.0),
            ));

        egui::Area::new(egui::Id::new("canvas-status-toast"))
            .fixed_pos(anchor)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                Frame::new()
                    .fill(theme::chrome_mid())
                    .stroke(theme::soft_stroke())
                    .corner_radius(design::radius::SM)
                    .inner_margin(Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.set_max_width(460.0);
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
            });
    }

    fn status_toast_signature(&self) -> Option<String> {
        if self.file_status.is_none()
            && self.artifact_status.is_none()
            && self.dimension_status.is_none()
        {
            return None;
        }

        Some(format!(
            "file={};artifact={};dimension={}",
            self.file_status.as_deref().unwrap_or_default(),
            self.artifact_status.as_deref().unwrap_or_default(),
            self.dimension_status.as_deref().unwrap_or_default()
        ))
    }

    pub(super) fn select_workflow_tab(&mut self, tab: WorkflowTab) {
        let previous_tab = self.command_tab;
        self.command_tab = tab;
        match tab {
            WorkflowTab::Render => self.set_workspace_mode(WorkspaceMode::Render),
            WorkflowTab::Plan => self.set_workspace_mode(WorkspaceMode::Plan),
            WorkflowTab::Design
            | WorkflowTab::Frame
            | WorkflowTab::Openings
            | WorkflowTab::Roofs
            | WorkflowTab::Annotate
            | WorkflowTab::Inspect => {
                self.set_workspace_mode(WorkspaceMode::Design);
                if previous_tab != tab {
                    self.apply_soft_default_view_for_tab(tab);
                }
            }
        }
    }

    fn workflow_command_panels(&mut self, ui: &mut Ui) {
        match self.command_tab {
            WorkflowTab::Design => {
                widgets::command_panel(ui, "Structure", |ui| {
                    let id = ActionId::ToolRoom;
                    if action_tool_button(
                        ui,
                        id,
                        self.room_tool_active,
                        self.action_enabled(id),
                        self.action_disabled_reason(id),
                    )
                    .clicked()
                    {
                        self.execute_action(id);
                    }
                });
            }
            WorkflowTab::Frame => {
                widgets::command_panel(ui, "Structure", |ui| {
                    let id = ActionId::ToolWall;
                    if action_tool_button(
                        ui,
                        id,
                        self.draw_wall_tool.active,
                        self.action_enabled(id),
                        self.action_disabled_reason(id),
                    )
                    .clicked()
                    {
                        self.execute_action(id);
                    }
                    let id = ActionId::ToolCeiling;
                    if action_tool_button(
                        ui,
                        id,
                        self.ceiling_tool_active,
                        self.action_enabled(id),
                        self.action_disabled_reason(id),
                    )
                    .clicked()
                    {
                        self.execute_action(id);
                    }
                    let id = ActionId::ToolVault;
                    if action_tool_button(
                        ui,
                        id,
                        self.vault_tool_active,
                        self.action_enabled(id),
                        self.action_disabled_reason(id),
                    )
                    .clicked()
                    {
                        self.execute_action(id);
                    }
                    let id = ActionId::ToolFloor;
                    if action_tool_button(
                        ui,
                        id,
                        self.floor_tool_active,
                        self.action_enabled(id),
                        self.action_disabled_reason(id),
                    )
                    .clicked()
                    {
                        self.execute_action(id);
                    }
                });
            }
            WorkflowTab::Openings => {
                widgets::command_panel(ui, "", |ui| {
                    self.opening_flyout(ui);
                });
            }
            WorkflowTab::Roofs => {
                widgets::command_panel(ui, "", |ui| {
                    self.roof_flyout(ui);
                });
            }
            WorkflowTab::Annotate => {
                widgets::command_panel(ui, "Dimensions", |ui| {
                    let id = ActionId::ToolDimensionLinear;
                    if action_tool_button(
                        ui,
                        id,
                        self.dimension_tool.active,
                        self.action_enabled(id),
                        self.action_disabled_reason(id),
                    )
                    .clicked()
                    {
                        self.execute_action(id);
                    }
                });
            }
            WorkflowTab::Inspect => {}
            WorkflowTab::Render => {
                self.render_settings_panels(ui);
            }
            WorkflowTab::Plan => {
                widgets::command_panel(ui, "Generated", |ui| {
                    let id = ActionId::ToggleSection;
                    if action_tool_button(
                        ui,
                        id,
                        self.show_section,
                        self.action_enabled(id),
                        self.action_disabled_reason(id),
                    )
                    .clicked()
                    {
                        self.execute_action(id);
                    }
                });
            }
        }
    }

    fn render_settings_panels(&mut self, ui: &mut Ui) {
        widgets::command_panel(ui, "Sun", |ui| {
            render_setting_drag(
                ui,
                "Azimuth",
                &mut self.render_settings.sun_azimuth_deg,
                RenderSettingDrag::degrees(0.0..=360.0, "Set the sun direction around the project"),
            );
            render_setting_drag(
                ui,
                "Elevation",
                &mut self.render_settings.sun_elevation_deg,
                RenderSettingDrag::degrees(0.0..=85.0, "Set the sun height above the horizon"),
            );
        });
        widgets::command_panel(ui, "Environment", |ui| {
            render_setting_drag(
                ui,
                "Exposure",
                &mut self.render_settings.exposure,
                RenderSettingDrag::exposure("Adjust render exposure"),
            );
        });
    }

    fn opening_flyout(&mut self, ui: &mut Ui) {
        command_flyout_button(ui, "Opening", "Add an opening variant", |ui| {
            ui.set_min_width(156.0);
            let id = ActionId::AddDoor;
            if flyout_action(
                ui,
                id,
                self.action_enabled(id),
                self.action_disabled_reason(id),
            )
            .clicked()
            {
                self.execute_action(id);
                ui.close();
            }
            let id = ActionId::AddWindow;
            if flyout_action(
                ui,
                id,
                self.action_enabled(id),
                self.action_disabled_reason(id),
            )
            .clicked()
            {
                self.execute_action(id);
                ui.close();
            }
            let id = ActionId::AddGarageDoor;
            if flyout_action(
                ui,
                id,
                self.action_enabled(id),
                self.action_disabled_reason(id),
            )
            .clicked()
            {
                self.execute_action(id);
                ui.close();
            }
        });
    }

    fn roof_flyout(&mut self, ui: &mut Ui) {
        command_flyout_button(ui, "Roof form", "Generate a roof form", |ui| {
            ui.set_min_width(156.0);
            let id = ActionId::AddGableRoof;
            if flyout_action(
                ui,
                id,
                self.action_enabled(id),
                self.action_disabled_reason(id),
            )
            .clicked()
            {
                self.execute_action(id);
                ui.close();
            }
            let id = ActionId::AddShedRoof;
            if flyout_action(
                ui,
                id,
                self.action_enabled(id),
                self.action_disabled_reason(id),
            )
            .clicked()
            {
                self.execute_action(id);
                ui.close();
            }
            let id = ActionId::AddHipRoof;
            if flyout_action(
                ui,
                id,
                self.action_enabled(id),
                self.action_disabled_reason(id),
            )
            .clicked()
            {
                self.execute_action(id);
                ui.close();
            }
        });
    }

    pub(super) fn command_search_overlay(&mut self, ctx: &egui::Context) {
        if !self.command_search.open {
            return;
        }

        let mut open = self.command_search.open;
        let mut close = false;
        let mut execute = None;

        egui::Window::new("Command Search")
            .id(egui::Id::new("command-search"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .open(&mut open)
            .show(ctx, |ui| {
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                {
                    close = true;
                }

                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.command_search.query)
                        .hint_text("Search commands")
                        .desired_width(ui.available_width()),
                );
                field.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        true,
                        "Command search input",
                    )
                });
                if self.command_search.focus_input {
                    field.request_focus();
                    self.command_search.focus_input = false;
                }

                ui.add_space(design::space::SM);
                let query = self.command_search.query.trim().to_ascii_lowercase();
                let matches: Vec<_> = actions::ACTIONS
                    .iter()
                    .copied()
                    .filter(|action| command_search_matches(*action, &query))
                    .take(12)
                    .collect();
                let enter_pressed = ui
                    .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                if enter_pressed {
                    execute = matches
                        .iter()
                        .find(|action| self.action_enabled(action.id))
                        .map(|action| action.id);
                }

                ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for action in matches {
                        let enabled = self.action_enabled(action.id);
                        if command_search_action(
                            ui,
                            action,
                            enabled,
                            self.action_disabled_reason(action.id),
                        )
                        .clicked()
                        {
                            execute = Some(action.id);
                        }
                    }
                });
            });

        if close {
            open = false;
        }
        self.command_search.open = open;
        if let Some(id) = execute {
            self.command_search.open = false;
            self.command_search.query.clear();
            self.execute_action(id);
        }
    }

    pub(super) fn toggle_dimension_tool(&mut self) {
        self.dimension_tool.active = !self.dimension_tool.active;
        self.dimension_tool.clear_picks();
        if self.dimension_tool.active {
            self.command_tab = WorkflowTab::Annotate;
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
        let visibility_available = self.workspace_mode != WorkspaceMode::Render;
        let id = ActionId::ShowAllComponents;
        let enabled = self.action_enabled(id);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let response =
                    ui.add_enabled(enabled, egui::Button::new("Show all components").small());
                let response = if enabled {
                    response.on_hover_text(actions::metadata(id).tooltip)
                } else {
                    response.on_disabled_hover_text(
                        self.action_disabled_reason(id)
                            .unwrap_or(actions::metadata(id).tooltip),
                    )
                };
                if response.clicked() {
                    self.execute_action(id);
                }
            });
        });
        ui.add_space(design::space::SM);

        // Browser rows are rebuilt every frame. Resolve the ordered selection
        // once so dense generated trees do not clone ids for every row.
        let selected_components = self.selected_components();
        let model_browser_scroll = ScrollArea::vertical().id_salt("model-browser-tree");
        model_browser_scroll.show(ui, |ui| {
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
                    // Roof planes, ceilings, and floor decks are level-owned surfaces,
                    // listed as siblings of rooms under each level: (id, name, level).
                    let roof_planes: Vec<_> = self
                        .model
                        .roof_planes
                        .iter()
                        .map(|plane| {
                            (
                                plane.id.0.clone(),
                                plane.name.clone(),
                                plane.level.0.clone(),
                            )
                        })
                        .collect();
                    let ceilings: Vec<_> = self
                        .model
                        .ceilings
                        .iter()
                        .map(|ceiling| {
                            (
                                ceiling.id.0.clone(),
                                ceiling.name.clone(),
                                ceiling.level.0.clone(),
                                ceiling.slope.is_some(),
                            )
                        })
                        .collect();
                    let floor_decks: Vec<_> = self
                        .model
                        .floor_decks
                        .iter()
                        .map(|deck| (deck.id.0.clone(), deck.name.clone(), deck.level.0.clone()))
                        .collect();
                    let standards_packs: Vec<_> = self
                        .model
                        .standards_packs
                        .iter()
                        .map(|pack| {
                            (
                                pack.id.0.clone(),
                                pack.name.clone(),
                                pack.source.is_some(),
                                self.model
                                    .standards
                                    .iter()
                                    .position(|stack_id| stack_id == &pack.id),
                            )
                        })
                        .collect();

                    let site_selected = matches!(self.selected, Selection::Site);
                    if tree_row(
                        ui,
                        site_selected,
                        Icon::Standards,
                        "Site & standards",
                        "Site and standards",
                    )
                    .clicked()
                    {
                        self.apply_selection(Selection::Site, None, SelectionOp::Replace);
                    }
                    ui.indent("site-standards", |ui| {
                        for (pack_id, pack_name, from_library, stack_index) in &standards_packs {
                            let selected = matches!(
                                &self.selected,
                                Selection::StandardsPack(id) if id == pack_id
                            );
                            let label = match stack_index {
                                Some(index) => format!("{}. {pack_name}", index + 1),
                                None => format!("{pack_name} (inactive)"),
                            };
                            if ui
                                .horizontal(|ui| {
                                    let clicked = tree_row_contents(
                                        ui,
                                        selected,
                                        Icon::Standards,
                                        &label,
                                        "Standards pack",
                                    )
                                    .clicked();
                                    if *from_library {
                                        library_badge(ui);
                                    }
                                    clicked
                                })
                                .inner
                            {
                                self.apply_selection(
                                    Selection::StandardsPack(pack_id.clone()),
                                    None,
                                    SelectionOp::Replace,
                                );
                            }
                        }
                    });

                    for (level_id, level_name) in levels {
                        let level_selected =
                            matches!(&self.selected, Selection::Level(id) if id == &level_id);
                        if tree_row(ui, level_selected, Icon::Level, &level_name, "Level").clicked()
                        {
                            self.set_active_level(ElementId::new(level_id.clone()));
                            self.apply_selection(
                                Selection::Level(level_id.clone()),
                                None,
                                SelectionOp::Replace,
                            );
                        }

                        ui.indent(format!("level-{level_id}"), |ui| {
                            for (index, wall_id, wall_name, wall_level, openings, dimensions) in
                                &walls
                            {
                                if wall_level != &level_id {
                                    continue;
                                }

                                let wall_key = ComponentKey::authored(
                                    AuthoredComponentKind::Wall,
                                    wall_id.clone(),
                                );
                                let wall_selected = selected_components.contains(&wall_key);
                                let (wall_row, wall_eye) = component_tree_row(
                                    ui,
                                    &wall_key,
                                    Icon::Wall,
                                    wall_name,
                                    "Wall segment",
                                    ComponentTreeRowState::new(
                                        wall_selected,
                                        self.component_visibility.is_explicitly_visible(&wall_key),
                                        visibility_available,
                                    ),
                                );
                                if wall_eye.clicked() {
                                    self.toggle_component_visibility(wall_key.clone(), wall_name);
                                }
                                if wall_row.clicked() {
                                    self.apply_selection(
                                        Selection::Wall,
                                        Some(*index),
                                        component_selection_op(ui),
                                    );
                                }

                                ui.indent(format!("wall-{wall_id}"), |ui| {
                                    for (opening_id, opening_kind, opening_name) in openings {
                                        let opening_key = ComponentKey::authored(
                                            AuthoredComponentKind::Opening,
                                            opening_id.clone(),
                                        );
                                        let selected = selected_components.contains(&opening_key);
                                        let (opening_row, opening_eye) = component_tree_row(
                                            ui,
                                            &opening_key,
                                            opening_tree_icon(*opening_kind),
                                            opening_name,
                                            kind_label(*opening_kind),
                                            ComponentTreeRowState::new(
                                                selected,
                                                self.component_visibility
                                                    .is_explicitly_visible(&opening_key),
                                                visibility_available
                                                    && self.workspace_mode.shows_generated_plan(),
                                            ),
                                        );
                                        if opening_eye.clicked() {
                                            self.toggle_component_visibility(
                                                opening_key.clone(),
                                                opening_name,
                                            );
                                        }
                                        if opening_row.clicked() {
                                            self.apply_selection(
                                                Selection::Opening(opening_id.clone()),
                                                Some(*index),
                                                component_selection_op(ui),
                                            );
                                        }
                                    }
                                    for (dimension_id, dimension_kind, dimension_name) in dimensions
                                    {
                                        let key = ComponentKey::authored(
                                            AuthoredComponentKind::Dimension,
                                            dimension_id.clone(),
                                        );
                                        let selected = selected_components.contains(&key);
                                        let kind = format!(
                                            "{} dimension",
                                            dimension_kind_label(*dimension_kind)
                                        );
                                        if tree_row(
                                            ui,
                                            selected,
                                            Icon::Dimension,
                                            dimension_name,
                                            &kind,
                                        )
                                        .clicked()
                                        {
                                            self.apply_selection(
                                                Selection::Dimension(dimension_id.clone()),
                                                Some(*index),
                                                component_selection_op(ui),
                                            );
                                        }
                                    }
                                });
                            }

                            for (room_id, room_name, room_level) in &rooms {
                                if room_level != &level_id {
                                    continue;
                                }
                                let key = ComponentKey::authored(
                                    AuthoredComponentKind::Room,
                                    room_id.clone(),
                                );
                                let selected = selected_components.contains(&key);
                                if tree_row(ui, selected, Icon::Room, room_name, "Room").clicked() {
                                    self.apply_selection(
                                        Selection::Room(room_id.clone()),
                                        None,
                                        component_selection_op(ui),
                                    );
                                }
                            }

                            for (plane_id, plane_name, plane_level) in &roof_planes {
                                if plane_level != &level_id {
                                    continue;
                                }
                                let key = ComponentKey::authored(
                                    AuthoredComponentKind::RoofPlane,
                                    plane_id.clone(),
                                );
                                let selected = selected_components.contains(&key);
                                let (row, eye) = component_tree_row(
                                    ui,
                                    &key,
                                    Icon::Roof,
                                    plane_name,
                                    "Roof plane",
                                    ComponentTreeRowState::new(
                                        selected,
                                        self.component_visibility.is_explicitly_visible(&key),
                                        visibility_available,
                                    ),
                                );
                                if eye.clicked() {
                                    self.toggle_component_visibility(key, plane_name);
                                }
                                if row.clicked() {
                                    self.apply_selection(
                                        Selection::RoofPlane(plane_id.clone()),
                                        None,
                                        component_selection_op(ui),
                                    );
                                }
                            }

                            for (ceiling_id, ceiling_name, ceiling_level, sloped) in &ceilings {
                                if ceiling_level != &level_id {
                                    continue;
                                }
                                let key = ComponentKey::authored(
                                    AuthoredComponentKind::Ceiling,
                                    ceiling_id.clone(),
                                );
                                let selected = selected_components.contains(&key);
                                // Distinguish a sloped (scissor/vault) ceiling from a
                                // flat one in the tree.
                                let kind = if *sloped {
                                    "Sloped ceiling"
                                } else {
                                    "Flat ceiling"
                                };
                                let (row, eye) = component_tree_row(
                                    ui,
                                    &key,
                                    Icon::Ceiling,
                                    ceiling_name,
                                    kind,
                                    ComponentTreeRowState::new(
                                        selected,
                                        self.component_visibility.is_explicitly_visible(&key),
                                        visibility_available,
                                    ),
                                );
                                if eye.clicked() {
                                    self.toggle_component_visibility(key, ceiling_name);
                                }
                                if row.clicked() {
                                    self.apply_selection(
                                        Selection::Ceiling(ceiling_id.clone()),
                                        None,
                                        component_selection_op(ui),
                                    );
                                }
                            }

                            for (deck_id, deck_name, deck_level) in &floor_decks {
                                if deck_level != &level_id {
                                    continue;
                                }
                                let key = ComponentKey::authored(
                                    AuthoredComponentKind::FloorDeck,
                                    deck_id.clone(),
                                );
                                let selected = selected_components.contains(&key);
                                let (row, eye) = component_tree_row(
                                    ui,
                                    &key,
                                    Icon::Floor,
                                    deck_name,
                                    "Floor deck",
                                    ComponentTreeRowState::new(
                                        selected,
                                        self.component_visibility.is_explicitly_visible(&key),
                                        visibility_available,
                                    ),
                                );
                                if eye.clicked() {
                                    self.toggle_component_visibility(key, deck_name);
                                }
                                if row.clicked() {
                                    self.apply_selection(
                                        Selection::FloorDeck(deck_id.clone()),
                                        None,
                                        component_selection_op(ui),
                                    );
                                }
                            }
                        });
                    }

                    if !joins.is_empty() {
                        ui.separator();
                        strong_label(ui, "Corners");
                        for (join_id, join_name, join_kind) in joins {
                            let key = ComponentKey::authored(
                                AuthoredComponentKind::Join,
                                join_id.clone(),
                            );
                            let selected = selected_components.contains(&key);
                            let kind = format!("Corner ({})", join_kind_label(join_kind));
                            let (row, eye) = component_tree_row(
                                ui,
                                &key,
                                Icon::Corner,
                                &join_name,
                                &kind,
                                ComponentTreeRowState::new(
                                    selected,
                                    self.component_visibility.is_explicitly_visible(&key),
                                    visibility_available
                                        && self.workspace_mode.shows_generated_plan(),
                                ),
                            );
                            if eye.clicked() {
                                self.toggle_component_visibility(key, &join_name);
                            }
                            if row.clicked() {
                                self.apply_selection(
                                    Selection::Join(join_id),
                                    None,
                                    component_selection_op(ui),
                                );
                            }
                        }
                    }
                });

            self.library_tree(ui);

            if self.workspace_mode.shows_generated_plan() {
                let has_plan = self.project_plan.is_some();
                let mut generated_groups = Vec::<(
                    String,
                    String,
                    Option<usize>,
                    Vec<(String, framer_solver::MemberKind)>,
                )>::new();
                if let Some(plan) = &self.project_plan {
                    for host in &plan.wall_plans {
                        let wall_index = self
                            .model
                            .walls
                            .iter()
                            .position(|wall| wall.id == host.wall);
                        let name = wall_index
                            .and_then(|index| self.model.walls.get(index))
                            .map(|wall| wall.name.as_str())
                            .unwrap_or(host.wall.0.as_str());
                        generated_groups.push((
                            host.wall.0.clone(),
                            format!("Framing: {name}"),
                            wall_index,
                            host.members
                                .iter()
                                .map(|member| (member.id.clone(), member.kind))
                                .collect(),
                        ));
                    }
                    for host in &plan.roof_plans {
                        let name = self
                            .model
                            .roof_planes
                            .iter()
                            .find(|plane| plane.id == host.roof)
                            .map(|plane| plane.name.as_str())
                            .unwrap_or(host.roof.0.as_str());
                        generated_groups.push((
                            host.roof.0.clone(),
                            format!("Roof framing: {name}"),
                            None,
                            host.members
                                .iter()
                                .map(|member| (member.id.clone(), member.kind))
                                .collect(),
                        ));
                    }
                    for host in &plan.ceiling_plans {
                        let name = self
                            .model
                            .ceilings
                            .iter()
                            .find(|ceiling| ceiling.id == host.ceiling)
                            .map(|ceiling| ceiling.name.as_str())
                            .unwrap_or(host.ceiling.0.as_str());
                        generated_groups.push((
                            host.ceiling.0.clone(),
                            format!("Ceiling framing: {name}"),
                            None,
                            host.members
                                .iter()
                                .map(|member| (member.id.clone(), member.kind))
                                .collect(),
                        ));
                    }
                    for host in &plan.floor_plans {
                        let name = self
                            .model
                            .floor_decks
                            .iter()
                            .find(|deck| deck.id == host.floor)
                            .map(|deck| deck.name.as_str())
                            .unwrap_or(host.floor.0.as_str());
                        generated_groups.push((
                            host.floor.0.clone(),
                            format!("Floor framing: {name}"),
                            None,
                            host.members
                                .iter()
                                .map(|member| (member.id.clone(), member.kind))
                                .collect(),
                        ));
                    }
                }
                let generated_count = generated_groups
                    .iter()
                    .map(|(_, _, _, members)| members.len())
                    .sum::<usize>();
                egui::CollapsingHeader::new(format!("Generated ({generated_count} members)"))
                    .default_open(true)
                    .show(ui, |ui| {
                        if has_plan {
                            for (source_id, label, wall_index, members) in &generated_groups {
                                let source_selected = selected_components.iter().any(|key| {
                                    matches!(
                                        key,
                                        ComponentKey::GeneratedMember { host_id, .. }
                                            if host_id == source_id
                                    )
                                });
                                egui::CollapsingHeader::new(format!(
                                    "{label} ({} members)",
                                    members.len()
                                ))
                                .default_open(source_selected)
                                .show(ui, |ui| {
                                    for (member_id, kind) in members {
                                        let key = ComponentKey::member(
                                            source_id.clone(),
                                            member_id.clone(),
                                        );
                                        let selected = selected_components.contains(&key);
                                        let name = format!("{}: {}", kind.label(), member_id);
                                        let (row, eye) = component_tree_row(
                                            ui,
                                            &key,
                                            member_tree_icon(*kind),
                                            &name,
                                            "Generated member",
                                            ComponentTreeRowState::new(
                                                selected,
                                                self.component_visibility
                                                    .is_explicitly_visible(&key),
                                                visibility_available,
                                            ),
                                        );
                                        if eye.clicked() {
                                            self.toggle_component_visibility(key, &name);
                                        }
                                        if row.clicked() {
                                            self.apply_selection(
                                                Selection::Member {
                                                    source_id: source_id.clone(),
                                                    member_id: member_id.clone(),
                                                },
                                                *wall_index,
                                                component_selection_op(ui),
                                            );
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
                let id = ActionId::AddDoor;
                if widgets::catalog_add_button(
                    ui,
                    "Door",
                    self.action_enabled(id),
                    self.action_disabled_reason(id)
                        .unwrap_or(actions::metadata(id).tooltip),
                )
                .clicked()
                {
                    self.execute_action(id);
                }
                let id = ActionId::AddWindow;
                if widgets::catalog_add_button(
                    ui,
                    "Window",
                    self.action_enabled(id),
                    self.action_disabled_reason(id)
                        .unwrap_or(actions::metadata(id).tooltip),
                )
                .clicked()
                {
                    self.execute_action(id);
                }
                let id = ActionId::AddGarageDoor;
                if widgets::catalog_add_button(
                    ui,
                    "Garage Door",
                    self.action_enabled(id),
                    self.action_disabled_reason(id)
                        .unwrap_or(actions::metadata(id).tooltip),
                )
                .clicked()
                {
                    self.execute_action(id);
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
        let systems: Vec<(String, String, &'static str, bool)> = self
            .model
            .systems
            .iter()
            .map(|system| {
                (
                    system.id.0.clone(),
                    system.name.clone(),
                    system.kind.label(),
                    system.source.is_some(),
                )
            })
            .collect();
        let materials: Vec<(String, String, [u8; 3], bool)> = self
            .model
            .materials
            .iter()
            .map(|material| {
                (
                    material.id.0.clone(),
                    material.name.clone(),
                    material.color(),
                    matches!(&material.source, MaterialSource::Library(_)),
                )
            })
            .collect();
        let furnishings: Vec<(String, String, String, bool)> = self
            .model
            .furnishings
            .iter()
            .map(|furnishing| {
                (
                    furnishing.id.0.clone(),
                    furnishing.name.clone(),
                    object_size_label(&furnishing.size),
                    furnishing.source.is_some(),
                )
            })
            .collect();
        let mep_objects: Vec<(String, String, &'static str, String, bool)> = self
            .model
            .mep_objects
            .iter()
            .map(|object| {
                (
                    object.id.0.clone(),
                    object.name.clone(),
                    object.kind.label(),
                    object_size_label(&object.size),
                    object.source.is_some(),
                )
            })
            .collect();
        let starter = can_edit
            .then(framer_library::starter_library)
            .and_then(Result::ok);
        let starter_systems = starter
            .as_ref()
            .map(|loaded| {
                loaded
                    .library
                    .systems
                    .iter()
                    .map(|system| {
                        (
                            system.id.0.clone(),
                            system.name.clone(),
                            system.kind.label(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let starter_materials = starter
            .as_ref()
            .map(|loaded| {
                loaded
                    .library
                    .materials
                    .iter()
                    .map(|material| {
                        (
                            material.id.0.clone(),
                            material.name.clone(),
                            material.color(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let starter_furnishings = starter
            .as_ref()
            .map(|loaded| {
                loaded
                    .library
                    .furnishings
                    .iter()
                    .map(|furnishing| {
                        (
                            furnishing.id.0.clone(),
                            furnishing.name.clone(),
                            object_size_label(&furnishing.size),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let starter_mep_objects = starter
            .as_ref()
            .map(|loaded| {
                loaded
                    .library
                    .mep_objects
                    .iter()
                    .map(|object| {
                        (
                            object.id.0.clone(),
                            object.name.clone(),
                            object.kind.label(),
                            object_size_label(&object.size),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut insert_system: Option<String> = None;
        let mut insert_material: Option<String> = None;
        let mut place_furnishing: Option<String> = None;
        let mut place_mep_object: Option<String> = None;

        egui::CollapsingHeader::new("Library")
            .default_open(false)
            .show(ui, |ui| {
                strong_label(ui, "Systems");
                for (id, name, kind, from_library) in &systems {
                    let selected = matches!(&self.selected, Selection::System(s) if s == id);
                    if ui
                        .horizontal(|ui| {
                            let clicked =
                                tree_row_contents(ui, selected, Icon::System, name, kind).clicked();
                            if *from_library {
                                library_badge(ui);
                            }
                            clicked
                        })
                        .inner
                    {
                        self.apply_selection(
                            Selection::System(id.clone()),
                            None,
                            SelectionOp::Replace,
                        );
                    }
                }
                if can_edit {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("+ Wall system").clicked() {
                            self.add_wall_system(true);
                        }
                        if ui.button("+ Interior system").clicked() {
                            self.add_wall_system(false);
                        }
                        if ui.button("+ Roof system").clicked() {
                            self.add_surface_system(framer_core::SystemKind::Roof);
                        }
                        if ui.button("+ Floor system").clicked() {
                            self.add_surface_system(framer_core::SystemKind::Floor);
                        }
                        if ui.button("+ Ceiling system").clicked() {
                            self.add_surface_system(framer_core::SystemKind::Ceiling);
                        }
                    });
                }

                ui.separator();
                strong_label(ui, "Materials");
                for (id, name, color, from_library) in &materials {
                    let selected = matches!(&self.selected, Selection::Material(m) if m == id);
                    let [r, g, b] = *color;
                    if ui
                        .horizontal(|ui| {
                            ui.label(
                                design::icon_text(Icon::Material, 13.0)
                                    .color(design::active().text_muted),
                            )
                            .on_hover_text("Material");
                            color_swatch(ui, Color32::from_rgb(r, g, b));
                            let clicked = ui
                                .selectable_label(selected, name)
                                .on_hover_text("Material")
                                .clicked();
                            if *from_library {
                                library_badge(ui);
                            }
                            clicked
                        })
                        .inner
                    {
                        self.apply_selection(
                            Selection::Material(id.clone()),
                            None,
                            SelectionOp::Replace,
                        );
                    }
                }
                if can_edit && ui.button("+ Material").clicked() {
                    self.add_material();
                }

                ui.separator();
                strong_label(ui, "Furnishings");
                for (id, name, size, from_library) in &furnishings {
                    let selected = matches!(&self.selected, Selection::Furnishing(f) if f == id);
                    if ui
                        .horizontal(|ui| {
                            let clicked = tree_row_contents(
                                ui,
                                selected,
                                Icon::Furnishing,
                                name,
                                "Furnishing",
                            )
                            .clicked();
                            ui.label(RichText::new(size).size(design::text_size::LABEL));
                            if *from_library {
                                library_badge(ui);
                            }
                            clicked
                        })
                        .inner
                    {
                        self.apply_selection(
                            Selection::Furnishing(id.clone()),
                            None,
                            SelectionOp::Replace,
                        );
                    }
                }

                ui.separator();
                strong_label(ui, "MEP");
                for (id, name, kind, size, from_library) in &mep_objects {
                    let selected = matches!(&self.selected, Selection::MepObject(m) if m == id);
                    if ui
                        .horizontal(|ui| {
                            let clicked =
                                tree_row_contents(ui, selected, Icon::MepObject, name, kind)
                                    .clicked();
                            ui.label(RichText::new(size).size(design::text_size::LABEL));
                            if *from_library {
                                library_badge(ui);
                            }
                            clicked
                        })
                        .inner
                    {
                        self.apply_selection(
                            Selection::MepObject(id.clone()),
                            None,
                            SelectionOp::Replace,
                        );
                    }
                }

                if can_edit
                    && (!starter_systems.is_empty()
                        || !starter_materials.is_empty()
                        || !starter_furnishings.is_empty()
                        || !starter_mep_objects.is_empty())
                {
                    ui.separator();
                    strong_label(ui, "Starter");
                    for (id, name, kind) in &starter_systems {
                        ui.horizontal(|ui| {
                            tree_static_row_contents(ui, Icon::System, name, kind);
                            if ui.button("Insert").clicked() {
                                insert_system = Some(id.clone());
                            }
                        });
                    }
                    for (id, name, color) in &starter_materials {
                        let [r, g, b] = *color;
                        ui.horizontal(|ui| {
                            ui.label(
                                design::icon_text(Icon::Material, 13.0)
                                    .color(design::active().text_muted),
                            )
                            .on_hover_text("Material");
                            color_swatch(ui, Color32::from_rgb(r, g, b));
                            ui.label(name).on_hover_text("Material");
                            if ui.button("Insert").clicked() {
                                insert_material = Some(id.clone());
                            }
                        });
                    }
                    for (id, name, size) in &starter_furnishings {
                        ui.horizontal(|ui| {
                            tree_static_row_contents(ui, Icon::Furnishing, name, "Furnishing");
                            ui.label(RichText::new(size).size(design::text_size::LABEL));
                            if ui.button("Place").clicked() {
                                place_furnishing = Some(id.clone());
                            }
                        });
                    }
                    for (id, name, kind, size) in &starter_mep_objects {
                        ui.horizontal(|ui| {
                            tree_static_row_contents(ui, Icon::MepObject, name, kind);
                            ui.label(RichText::new(size).size(design::text_size::LABEL));
                            if ui.button("Place").clicked() {
                                place_mep_object = Some(id.clone());
                            }
                        });
                    }
                }
            });

        if let Some(id) = insert_system {
            self.insert_starter_system(id);
        }
        if let Some(id) = insert_material {
            self.insert_starter_material(id);
        }
        if let Some(id) = place_furnishing {
            self.place_starter_furnishing(id);
        }
        if let Some(id) = place_mep_object {
            self.place_starter_mep_object(id);
        }
    }

    fn insert_starter_system(&mut self, id: String) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let loaded = match framer_library::starter_library() {
            Ok(loaded) => loaded,
            Err(error) => {
                self.file_status = Some(format!("Library import failed: {error}"));
                return;
            }
        };

        let mut result = None;
        self.edit("Insert library system", |app| {
            let imported = framer_library::import_system(
                &mut app.model,
                &loaded.library,
                &loaded.content_hash,
                &ElementId::new(&id),
            );
            if let Ok(imported) = &imported
                && let Some(system) = &imported.system
            {
                app.selected = Selection::System(system.0.clone());
            }
            result = Some(imported);
        });
        self.file_status = Some(match result.expect("import closure should run") {
            Ok(imported) => {
                let id = imported
                    .system
                    .map(|id| id.0)
                    .unwrap_or_else(|| "system".to_owned());
                format!("Inserted {id} from starter library")
            }
            Err(error) => format!("Library import failed: {error}"),
        });
    }

    fn insert_starter_material(&mut self, id: String) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let loaded = match framer_library::starter_library() {
            Ok(loaded) => loaded,
            Err(error) => {
                self.file_status = Some(format!("Library import failed: {error}"));
                return;
            }
        };

        let mut result = None;
        self.edit("Insert library material", |app| {
            let imported = framer_library::import_material(
                &mut app.model,
                &loaded.library,
                &loaded.content_hash,
                &ElementId::new(&id),
            );
            if let Ok(imported) = &imported
                && let Some(material) = imported.materials.first()
            {
                app.selected = Selection::Material(material.0.clone());
            }
            result = Some(imported);
        });
        self.file_status = Some(match result.expect("import closure should run") {
            Ok(imported) => {
                let id = imported
                    .materials
                    .first()
                    .map(|id| id.0.clone())
                    .unwrap_or_else(|| "material".to_owned());
                format!("Inserted {id} from starter library")
            }
            Err(error) => format!("Library import failed: {error}"),
        });
    }

    fn place_starter_furnishing(&mut self, id: String) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let loaded = match framer_library::starter_library() {
            Ok(loaded) => loaded,
            Err(error) => {
                self.file_status = Some(format!("Library placement failed: {error}"));
                return;
            }
        };
        let position = self
            .cursor_model
            .unwrap_or(Point2::new(Length::ZERO, Length::ZERO));

        let mut result = None;
        self.edit("Place library furnishing", |app| {
            let source_id = ElementId::new(&id);
            let family = matching_furnishing_family_id(&app.model, &loaded.library, &source_id)
                .map(Ok)
                .unwrap_or_else(|| {
                    framer_library::import_furnishing(
                        &mut app.model,
                        &loaded.library,
                        &loaded.content_hash,
                        &source_id,
                    )
                    .map(|imported| {
                        imported
                            .furnishing
                            .expect("furnishing import should return a family id")
                    })
                });
            match family {
                Ok(family_id) => {
                    let (instance_id, index) = next_furnishing_instance_id(&app.model);
                    let family_name = app
                        .model
                        .furnishings
                        .iter()
                        .find(|furnishing| furnishing.id == family_id)
                        .map(|furnishing| furnishing.name.clone())
                        .unwrap_or_else(|| "Furnishing".to_owned());
                    let level = app.active_level_id();
                    app.model.furnishing_instances.push(FurnishingInstance::new(
                        instance_id.clone(),
                        format!("{family_name} {index}"),
                        family_id.0.clone(),
                        level.0,
                        position,
                    ));
                    app.selected = Selection::FurnishingInstance(instance_id.clone());
                    app.viewport_mode = ViewportMode::Plan;
                    result = Some(Ok(instance_id));
                }
                Err(error) => result = Some(Err(error)),
            }
        });
        self.file_status = Some(match result.expect("placement closure should run") {
            Ok(id) => format!("Placed furnishing {id}"),
            Err(error) => format!("Library placement failed: {error}"),
        });
    }

    fn place_starter_mep_object(&mut self, id: String) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let loaded = match framer_library::starter_library() {
            Ok(loaded) => loaded,
            Err(error) => {
                self.file_status = Some(format!("Library placement failed: {error}"));
                return;
            }
        };
        let position = self
            .cursor_model
            .unwrap_or(Point2::new(Length::ZERO, Length::ZERO));

        let mut result = None;
        self.edit("Place library MEP object", |app| {
            let source_id = ElementId::new(&id);
            let family = matching_mep_object_id(&app.model, &loaded.library, &source_id)
                .map(Ok)
                .unwrap_or_else(|| {
                    framer_library::import_mep_object(
                        &mut app.model,
                        &loaded.library,
                        &loaded.content_hash,
                        &source_id,
                    )
                    .map(|imported| {
                        imported
                            .mep_object
                            .expect("MEP import should return a family id")
                    })
                });
            match family {
                Ok(family_id) => {
                    let (instance_id, index) = next_mep_instance_id(&app.model);
                    let family_name = app
                        .model
                        .mep_objects
                        .iter()
                        .find(|object| object.id == family_id)
                        .map(|object| object.name.clone())
                        .unwrap_or_else(|| "MEP object".to_owned());
                    let level = app.active_level_id();
                    app.model.mep_instances.push(MepInstance::new(
                        instance_id.clone(),
                        format!("{family_name} {index}"),
                        family_id.0.clone(),
                        level.0,
                        position,
                    ));
                    app.selected = Selection::MepInstance(instance_id.clone());
                    app.viewport_mode = ViewportMode::Plan;
                    result = Some(Ok(instance_id));
                }
                Err(error) => result = Some(Err(error)),
            }
        });
        self.file_status = Some(match result.expect("placement closure should run") {
            Ok(id) => format!("Placed MEP object {id}"),
            Err(error) => format!("Library placement failed: {error}"),
        });
    }

    fn resync_library_item(&mut self, item: framer_library::LibraryItem) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let loaded = match framer_library::starter_library() {
            Ok(loaded) => loaded,
            Err(error) => {
                self.file_status = Some(format!("Library re-sync failed: {error}"));
                return;
            }
        };

        let mut result = None;
        self.edit("Re-sync library item", |app| {
            result = Some(framer_library::resync_item(
                &mut app.model,
                &loaded.library,
                &loaded.content_hash,
                item.clone(),
            ));
        });
        self.file_status = Some(match result.expect("re-sync closure should run") {
            Ok(_) => format!(
                "Re-synced library {} {}",
                library_item_kind_label(&item),
                library_item_id(&item).0
            ),
            Err(error) => format!("Library re-sync failed: {error}"),
        });
    }

    fn detach_library_item(&mut self, item: framer_library::LibraryItem) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }

        let mut result = None;
        self.edit("Detach library item", |app| {
            result = Some(framer_library::detach_item(&mut app.model, item.clone()));
        });
        self.file_status = Some(match result.expect("detach closure should run") {
            Ok(true) => format!(
                "Detached library {} {}",
                library_item_kind_label(&item),
                library_item_id(&item).0
            ),
            Ok(false) => format!(
                "Library {} {} was already detached",
                library_item_kind_label(&item),
                library_item_id(&item).0
            ),
            Err(error) => format!("Library detach failed: {error}"),
        });
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui) {
        let selected_component_count = self.selected_component_count();
        if selected_component_count > 1 {
            panel_header(ui, "Inspector", "Multiple");
            multi_selection_inspector(ui, selected_component_count);
            self.inspector_output_sections(ui);
            return;
        }

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
        //   - library lifecycle actions for the selected system/material.
        let mut deferred_resync_library: Option<framer_library::LibraryItem> = None;
        let mut deferred_detach_library: Option<framer_library::LibraryItem> = None;
        let mut deferred_standards = DeferredStandardsActions::default();
        // Connected roof fields must share overhang values so their fixed seam
        // edges intersect at identical endpoints. Apply an inspector change to
        // the whole exact-edge component after the selected-plane borrow ends.
        let mut deferred_roof_overhang_sync: Option<(ElementId, Length, Length)> = None;
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
        // Construction-system picklists and richer summaries (stacked swatch,
        // R-value, layer count) per system kind, collected before the mutable
        // element borrow below so each object's System picker only offers (and
        // summarizes) systems of its own kind. The model member of each summary is
        // borrowed only here.
        let systems_of = |kind: framer_core::SystemKind| {
            self.model
                .systems
                .iter()
                .filter(move |system| system.kind == kind)
                .map(|system| {
                    (
                        system.id.0.clone(),
                        system.name.clone(),
                        system.total_thickness().to_string(),
                        system.exposure().label().to_owned(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let summaries_of = |kind: framer_core::SystemKind| {
            self.model
                .systems
                .iter()
                .filter(move |system| system.kind == kind)
                .map(|system| WallSystemSummary::from_system(system, &self.model))
                .collect::<Vec<_>>()
        };
        let wall_systems = systems_of(framer_core::SystemKind::Wall);
        let roof_systems = systems_of(framer_core::SystemKind::Roof);
        let ceiling_systems = systems_of(framer_core::SystemKind::Ceiling);
        let floor_systems = systems_of(framer_core::SystemKind::Floor);
        let wall_system_summaries = summaries_of(framer_core::SystemKind::Wall);
        let roof_system_summaries = summaries_of(framer_core::SystemKind::Roof);
        let ceiling_system_summaries = summaries_of(framer_core::SystemKind::Ceiling);
        let floor_system_summaries = summaries_of(framer_core::SystemKind::Floor);
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
        let furnishing_options = self
            .model
            .furnishings
            .iter()
            .map(|furnishing| (furnishing.id.0.clone(), furnishing.name.clone()))
            .collect::<Vec<(String, String)>>();
        let mep_options = self
            .model
            .mep_objects
            .iter()
            .map(|object| (object.id.0.clone(), object.name.clone()))
            .collect::<Vec<(String, String)>>();
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
        // Whether the selected system is applied to any wall, roof plane, ceiling,
        // or floor deck. A referenced system must keep its kind (switching, say, a
        // Wall system to Floor would invalidate every wall that uses it), so its
        // Kind row is locked while anything references it.
        let selected_system_referenced = if let Selection::System(id) = &selection {
            self.model.walls.iter().any(|wall| wall.system.0 == *id)
                || self
                    .model
                    .roof_planes
                    .iter()
                    .any(|plane| plane.system.0 == *id)
                || self
                    .model
                    .ceilings
                    .iter()
                    .any(|ceiling| ceiling.system.0 == *id)
                || self
                    .model
                    .floor_decks
                    .iter()
                    .any(|deck| deck.system.0 == *id)
        } else {
            false
        };
        let selected_library_status =
            selected_library_status(&self.model, &selection, &self.library_issues);
        let selected_inspector_id =
            selection_inspector_id(&self.model, self.selected_wall, &selection);

        panel_header(ui, "Inspector", selection_badge(&selection));

        match selection {
            Selection::None => {
                empty_inspector_state(ui);
            }
            Selection::Site => {
                if can_edit {
                    widgets::section(ui, "site-context", "Site context", true, |ui| {
                        changed |= site_context_editor(ui, &mut self.model.site);
                    });
                    widgets::section(ui, "standards-stack", "Standards stack", true, |ui| {
                        standards_stack_panel(
                            ui,
                            &self.model,
                            &mut self.selected,
                            &mut deferred_standards,
                        );
                    });
                } else {
                    site_context_summary(ui, &self.model.site);
                    ui.separator();
                    standards_stack_summary(ui, &self.model);
                }
            }
            Selection::Level(id) => {
                if let Some(level) = self.model.levels.iter_mut().find(|level| level.id.0 == id) {
                    if can_edit {
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
                    .and_then(|room| {
                        framer_core::room_boundary_on_level(&self.model, &room.level, room.seed)
                    });
                if let Some(room) = self.model.rooms.iter_mut().find(|room| room.id.0 == id) {
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
                        changed |= text_edit(ui, "Name", &mut wall.name);

                        let mut level_id = wall.level.0.clone();
                        property_row(ui, "Level", |ui| {
                            ComboBox::from_id_salt("wall-level")
                                .selected_text(level_display_name(&level_options, &level_id))
                                .show_ui(ui, |ui| {
                                    for (id, name) in &level_options {
                                        ui.selectable_value(&mut level_id, id.clone(), name);
                                    }
                                });
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
                                length_drag_spec(24.0, 480.0, DisplayUnit::Feet),
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
                                length_drag_spec(48.0, 168.0, DisplayUnit::Feet),
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
                    self.apply_selection(
                        Selection::Dimension(dimension_id),
                        Some(self.selected_wall),
                        SelectionOp::Replace,
                    );
                }
            }
            Selection::Opening(id) => {
                let top_clearance = opening_top_clearance(&self.model.framing_defaults());
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
                            changed |= text_edit(ui, "Name", &mut opening.name);
                            property_row(ui, "Kind", |ui| {
                                ComboBox::from_id_salt("opening-kind")
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
                            });
                            changed |= driven_length_drag(
                                ui,
                                "Center",
                                &mut opening.center,
                                length_drag_spec(0.0, 480.0, DisplayUnit::Feet),
                                driven_fields.center.as_ref(),
                                &mut select_dimension,
                            );
                            changed |= driven_length_drag(
                                ui,
                                "Width",
                                &mut opening.width,
                                length_drag_spec(12.0, 240.0, DisplayUnit::Inches),
                                driven_fields.width.as_ref(),
                                &mut select_dimension,
                            );
                            changed |= driven_length_drag(
                                ui,
                                "Height",
                                &mut opening.height,
                                length_drag_spec(12.0, 120.0, DisplayUnit::Inches),
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
                                    DisplayUnit::Inches,
                                ),
                                driven_fields.bottom.as_ref(),
                                &mut select_dimension,
                            );

                            ui.separator();
                            if danger_button(ui, "Remove Opening").clicked() {
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
                    self.apply_selection(
                        Selection::Dimension(dimension_id),
                        Some(self.selected_wall),
                        SelectionOp::Replace,
                    );
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
                        changed |= text_edit(ui, "Name", &mut join.name);
                        property_row(ui, "Kind", |ui| {
                            ComboBox::from_id_salt("join-kind")
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
                                        .selectable_value(
                                            &mut join.kind,
                                            WallJoinKind::Cross,
                                            "Cross",
                                        )
                                        .changed();
                                });
                        });

                        let mut first_wall = join.first_wall.0.clone();
                        property_row(ui, "First wall", |ui| {
                            ComboBox::from_id_salt("join-first-wall")
                                .selected_text(wall_display_name(&wall_options, &first_wall))
                                .show_ui(ui, |ui| {
                                    for (id, name) in &wall_options {
                                        ui.selectable_value(&mut first_wall, id.clone(), name);
                                    }
                                });
                        });
                        if first_wall != join.first_wall.0 {
                            join.first_wall = ElementId::new(first_wall);
                            changed = true;
                        }

                        let mut second_wall = join.second_wall.0.clone();
                        property_row(ui, "Second wall", |ui| {
                            ComboBox::from_id_salt("join-second-wall")
                                .selected_text(wall_display_name(&wall_options, &second_wall))
                                .show_ui(ui, |ui| {
                                    for (id, name) in &wall_options {
                                        ui.selectable_value(&mut second_wall, id.clone(), name);
                                    }
                                });
                        });
                        if second_wall != join.second_wall.0 {
                            join.second_wall = ElementId::new(second_wall);
                            changed = true;
                        }

                        ui.separator();
                        strong_label(ui, "Corner point");
                        changed |= coordinate_drag(ui, "X", &mut join.point.x);
                        changed |= coordinate_drag(ui, "Y", &mut join.point.y);
                    } else {
                        join_summary(ui, join, &wall_options);
                    }
                } else {
                    ui.label("Corner no longer exists");
                }
            }
            Selection::Member {
                source_id,
                member_id,
            } => {
                if let Some(member) = self.selected_member(&source_id, &member_id) {
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

                        if let Some(status) = selected_library_status.as_ref() {
                            widgets::section(ui, "system-library-source", "Library", true, |ui| {
                                library_lifecycle_controls(
                                    ui,
                                    status,
                                    &mut deferred_resync_library,
                                    &mut deferred_detach_library,
                                );
                            });
                        }

                        widgets::section(ui, "system-layers", "Layers", true, |ui| {
                            let layer_count = system.layers.len();
                            // Every framed assembly — wall, roof, floor, or ceiling —
                            // requires exactly one positive-thickness framing layer;
                            // the editor uses this count to keep the sole framing
                            // layer un-removable and block adding a second one. The
                            // kind also seeds a newly-framing layer's member family.
                            let system_kind = system.kind;
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
                                    system_kind,
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
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut material.name);
                        if let Some(status) = selected_library_status.as_ref() {
                            widgets::section(
                                ui,
                                "material-library-source",
                                "Library",
                                true,
                                |ui| {
                                    library_lifecycle_controls(
                                        ui,
                                        status,
                                        &mut deferred_resync_library,
                                        &mut deferred_detach_library,
                                    );
                                },
                            );
                        }
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
            Selection::Furnishing(id) => {
                if let Some(furnishing) = self
                    .model
                    .furnishings
                    .iter_mut()
                    .find(|furnishing| furnishing.id.0 == id)
                {
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut furnishing.name);
                        if let Some(status) = selected_library_status.as_ref() {
                            widgets::section(
                                ui,
                                "furnishing-library-source",
                                "Library",
                                true,
                                |ui| {
                                    library_lifecycle_controls(
                                        ui,
                                        status,
                                        &mut deferred_resync_library,
                                        &mut deferred_detach_library,
                                    );
                                },
                            );
                        }
                        widgets::section(ui, "furnishing-size", "Size", true, |ui| {
                            changed |= object_size_editor(ui, &mut furnishing.size);
                        });
                        widgets::section(ui, "furnishing-tags", "Tags", false, |ui| {
                            changed |= tags_editor(ui, &mut furnishing.tags);
                        });
                    } else {
                        object_family_summary(ui, &furnishing.name, &furnishing.size, None);
                    }
                } else {
                    ui.label("Furnishing no longer exists");
                }
            }
            Selection::MepObject(id) => {
                if let Some(object) = self
                    .model
                    .mep_objects
                    .iter_mut()
                    .find(|object| object.id.0 == id)
                {
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut object.name);
                        property_row(ui, "Kind", |ui| {
                            ComboBox::from_id_salt("mep-kind")
                                .selected_text(object.kind.label())
                                .show_ui(ui, |ui| {
                                    for kind in framer_core::MepObjectKind::ALL {
                                        changed |= ui
                                            .selectable_value(&mut object.kind, kind, kind.label())
                                            .changed();
                                    }
                                });
                        });
                        if let Some(status) = selected_library_status.as_ref() {
                            widgets::section(ui, "mep-library-source", "Library", true, |ui| {
                                library_lifecycle_controls(
                                    ui,
                                    status,
                                    &mut deferred_resync_library,
                                    &mut deferred_detach_library,
                                );
                            });
                        }
                        widgets::section(ui, "mep-size", "Size", true, |ui| {
                            changed |= object_size_editor(ui, &mut object.size);
                        });
                        widgets::section(ui, "mep-tags", "Tags", false, |ui| {
                            changed |= tags_editor(ui, &mut object.tags);
                        });
                    } else {
                        object_family_summary(
                            ui,
                            &object.name,
                            &object.size,
                            Some(object.kind.label()),
                        );
                    }
                } else {
                    ui.label("MEP object no longer exists");
                }
            }
            Selection::StandardsPack(id) => {
                let stack_index = self
                    .model
                    .standards
                    .iter()
                    .position(|stack_id| stack_id.0 == id);
                let stack_len = self.model.standards.len();
                let resolved_rules = standards_rule_rows(&self.model);
                if let Some(pack) = self
                    .model
                    .standards_packs
                    .iter_mut()
                    .find(|pack| pack.id.0 == id)
                {
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut pack.name);
                        changed |= text_edit(ui, "Edition", &mut pack.edition);
                        if let Some(status) = selected_library_status.as_ref() {
                            widgets::section(
                                ui,
                                "standards-library-source",
                                "Library",
                                true,
                                |ui| {
                                    library_lifecycle_controls(
                                        ui,
                                        status,
                                        &mut deferred_resync_library,
                                        &mut deferred_detach_library,
                                    );
                                },
                            );
                        }
                        widgets::section(ui, "standards-stack-membership", "Stack", true, |ui| {
                            standards_pack_stack_controls(
                                ui,
                                &id,
                                stack_index,
                                stack_len,
                                &mut deferred_standards,
                            );
                        });
                        widgets::section(ui, "standards-waivers", "Waivers", true, |ui| {
                            standards_waiver_editor(
                                ui,
                                &pack.id.0,
                                &resolved_rules,
                                &mut deferred_standards,
                            );
                        });
                        widgets::section(ui, "standards-tags", "Tags", false, |ui| {
                            changed |= tags_editor(ui, &mut pack.tags);
                        });
                    } else {
                        standards_pack_summary(ui, pack, stack_index);
                    }
                } else {
                    ui.label("Standards pack no longer exists");
                }
            }
            Selection::FurnishingInstance(id) => {
                if let Some(instance) = self
                    .model
                    .furnishing_instances
                    .iter_mut()
                    .find(|instance| instance.id.0 == id)
                {
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut instance.name);
                        changed |=
                            family_picker(ui, "Family", &mut instance.family, &furnishing_options);
                        changed |= level_picker(ui, &mut instance.level, &level_options);
                        changed |= coordinate_drag(ui, "X", &mut instance.position.x);
                        changed |= coordinate_drag(ui, "Y", &mut instance.position.y);
                        changed |= rotation_picker(ui, &mut instance.rotation);
                        widgets::section(ui, "furnishing-instance-tags", "Tags", false, |ui| {
                            changed |= tags_editor(ui, &mut instance.tags);
                        });
                        ui.separator();
                        if danger_button(ui, "Remove Furnishing").clicked() {
                            deferred_remove = Some(DeferredRemove::FurnishingInstance(id.clone()));
                        }
                    } else {
                        placed_object_summary(
                            ui,
                            &instance.name,
                            family_display_name(&furnishing_options, &instance.family.0),
                            &level_options,
                            instance.level.0.as_str(),
                            instance.position,
                            instance.rotation,
                        );
                    }
                } else {
                    ui.label("Furnishing instance no longer exists");
                }
            }
            Selection::MepInstance(id) => {
                if let Some(instance) = self
                    .model
                    .mep_instances
                    .iter_mut()
                    .find(|instance| instance.id.0 == id)
                {
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut instance.name);
                        changed |= family_picker(ui, "Family", &mut instance.family, &mep_options);
                        changed |= level_picker(ui, &mut instance.level, &level_options);
                        changed |= coordinate_drag(ui, "X", &mut instance.position.x);
                        changed |= coordinate_drag(ui, "Y", &mut instance.position.y);
                        changed |= rotation_picker(ui, &mut instance.rotation);
                        widgets::section(ui, "mep-instance-tags", "Tags", false, |ui| {
                            changed |= tags_editor(ui, &mut instance.tags);
                        });
                        ui.separator();
                        if danger_button(ui, "Remove MEP Object").clicked() {
                            deferred_remove = Some(DeferredRemove::MepInstance(id.clone()));
                        }
                    } else {
                        placed_object_summary(
                            ui,
                            &instance.name,
                            family_display_name(&mep_options, &instance.family.0),
                            &level_options,
                            instance.level.0.as_str(),
                            instance.position,
                            instance.rotation,
                        );
                    }
                } else {
                    ui.label("MEP instance no longer exists");
                }
            }
            // Read-only summaries for the roof/ceiling/floor surfaces: Slice 4
            // makes them selectable (in the 3D view); editable inspectors (pitch,
            // height, span, system) land with the authoring tools in a later slice.
            Selection::RoofPlane(id) => {
                if let Some(plane) = self.model.roof_planes.iter_mut().find(|p| p.id.0 == id) {
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut plane.name);

                        let mut level_id = plane.level.0.clone();
                        property_row(ui, "Level", |ui| {
                            ComboBox::from_id_salt("roof-level")
                                .selected_text(level_display_name(&level_options, &level_id))
                                .show_ui(ui, |ui| {
                                    for (lid, name) in &level_options {
                                        ui.selectable_value(&mut level_id, lid.clone(), name);
                                    }
                                });
                        });
                        if level_id != plane.level.0 {
                            plane.level = ElementId::new(level_id);
                            changed = true;
                            deferred_roof_overhang_sync =
                                Some((plane.id.clone(), plane.eave_overhang, plane.rake_overhang));
                        }

                        widgets::section(ui, "roof-geometry", "Geometry", true, |ui| {
                            // Pitch as an explicit rise:run; the run is clamped above
                            // zero (validation requires it) and the ratio is echoed.
                            changed |= length_drag(
                                ui,
                                "Pitch rise",
                                &mut plane.slope.rise,
                                0.0,
                                144.0,
                                DisplayUnit::Inches,
                            );
                            changed |= length_drag(
                                ui,
                                "Pitch run",
                                &mut plane.slope.run,
                                1.0,
                                144.0,
                                DisplayUnit::Inches,
                            );
                            summary_row(
                                ui,
                                "Pitch",
                                format!("{}:{}", plane.slope.rise, plane.slope.run),
                            );
                            property_row(ui, "Eave edge", |ui| {
                                let max = plane.outline.len().saturating_sub(1) as u32;
                                let before = plane.eave_edge;
                                editable_drag_value(
                                    ui,
                                    egui::DragValue::new(&mut plane.eave_edge).range(0..=max),
                                );
                                if plane.eave_edge != before {
                                    changed = true;
                                }
                            });
                            summary_row(ui, "Outline", format!("{} pts", plane.outline.len()));
                            changed |= coordinate_drag(
                                ui,
                                "Springing elevation",
                                &mut plane.reference_elevation,
                            );
                            let eave_changed = length_drag(
                                ui,
                                "Eave overhang",
                                &mut plane.eave_overhang,
                                0.0,
                                48.0,
                                DisplayUnit::Inches,
                            );
                            let rake_changed = length_drag(
                                ui,
                                "Rake overhang",
                                &mut plane.rake_overhang,
                                0.0,
                                48.0,
                                DisplayUnit::Inches,
                            );
                            changed |= eave_changed || rake_changed;
                            if eave_changed || rake_changed {
                                deferred_roof_overhang_sync = Some((
                                    plane.id.clone(),
                                    plane.eave_overhang,
                                    plane.rake_overhang,
                                ));
                            }
                        });

                        changed |= surface_system_picker(
                            ui,
                            "roof-system",
                            &mut plane.system,
                            &roof_systems,
                            &roof_system_summaries,
                            &mut deferred_select_system,
                        );
                    } else {
                        ui.label(&plane.name);
                        ui.label(format!("System: {}", plane.system.0));
                        ui.label(format!("Pitch: {}:{}", plane.slope.rise, plane.slope.run));
                        ui.label(format!("Eave edge: {}", plane.eave_edge));
                        ui.label(format!(
                            "Springing elevation: {}",
                            plane.reference_elevation
                        ));
                        ui.label(format!(
                            "Overhangs: eave {}, rake {}",
                            plane.eave_overhang, plane.rake_overhang
                        ));
                    }
                } else {
                    ui.label("Roof plane no longer exists");
                }
            }
            Selection::Ceiling(id) => {
                // Resolve this ceiling's plan outline (a `Room` region through the wall
                // graph) before the mutable borrow below, so enabling a slope can
                // convert a room-attached ceiling to a fixed polygon — a sloped ceiling
                // needs an explicit outline (validation requires it).
                let ceiling_outline: Option<Vec<Point2>> = self
                    .model
                    .ceilings
                    .iter()
                    .find(|c| c.id.0 == id)
                    .and_then(|c| match &c.region {
                        SurfaceRegion::Polygon(points) => Some(points.clone()),
                        SurfaceRegion::Room(room_id) => self
                            .model
                            .rooms
                            .iter()
                            .find(|room| room.id == *room_id)
                            .and_then(|room| {
                                framer_core::room_boundary_on_level(
                                    &self.model,
                                    &room.level,
                                    room.seed,
                                )
                            })
                            .map(|boundary| boundary.vertices),
                    });
                if let Some(ceiling) = self.model.ceilings.iter_mut().find(|c| c.id.0 == id) {
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut ceiling.name);

                        let mut level_id = ceiling.level.0.clone();
                        property_row(ui, "Level", |ui| {
                            ComboBox::from_id_salt("ceiling-level")
                                .selected_text(level_display_name(&level_options, &level_id))
                                .show_ui(ui, |ui| {
                                    for (lid, name) in &level_options {
                                        ui.selectable_value(&mut level_id, lid.clone(), name);
                                    }
                                });
                        });
                        if level_id != ceiling.level.0 {
                            ceiling.level = ElementId::new(level_id);
                            changed = true;
                        }

                        widgets::section(ui, "ceiling-dimensions", "Geometry", true, |ui| {
                            changed |= length_drag(
                                ui,
                                "Height below top",
                                &mut ceiling.height,
                                0.0,
                                480.0,
                                DisplayUnit::Feet,
                            );
                            summary_row(ui, "Region", surface_region_summary(&ceiling.region));
                            // Slope editor. Flat stays the default; enabling a slope on
                            // a room-attached ceiling converts its region to the fixed
                            // polygon resolved above (a sloped ceiling needs one). The
                            // pitch + low (spring) edge mirror a roof plane.
                            let outline_len = ceiling_outline.as_ref().map_or(0, Vec::len);
                            let can_slope = outline_len >= 3;
                            let mut sloped = ceiling.slope.is_some();
                            let toggle = ui
                                .add_enabled(can_slope, egui::Checkbox::new(&mut sloped, "Sloped"));
                            let toggle_changed = toggle.changed();
                            if !can_slope {
                                toggle.on_hover_text(
                                    "A sloped ceiling needs an enclosed region; close the room loop first.",
                                );
                            }
                            if toggle_changed {
                                if sloped {
                                    if let Some(outline) = &ceiling_outline {
                                        if matches!(ceiling.region, SurfaceRegion::Room(_)) {
                                            ceiling.region =
                                                SurfaceRegion::Polygon(outline.clone());
                                        }
                                        ceiling.slope = Some(CeilingSlope::new(
                                            Slope::new(
                                                Length::from_whole_inches(4),
                                                Length::from_whole_inches(12),
                                            ),
                                            0,
                                        ));
                                    }
                                } else {
                                    ceiling.slope = None;
                                }
                                changed = true;
                            }
                            if let Some(slope) = &mut ceiling.slope {
                                changed |= length_drag(
                                    ui,
                                    "Pitch rise",
                                    &mut slope.pitch.rise,
                                    0.0,
                                    144.0,
                                    DisplayUnit::Inches,
                                );
                                changed |= length_drag(
                                    ui,
                                    "Pitch run",
                                    &mut slope.pitch.run,
                                    1.0,
                                    144.0,
                                    DisplayUnit::Inches,
                                );
                                summary_row(
                                    ui,
                                    "Pitch",
                                    format!("{}:{}", slope.pitch.rise, slope.pitch.run),
                                );
                                property_row(ui, "Low edge", |ui| {
                                    let max = outline_len.saturating_sub(1) as u32;
                                    let before = slope.low_edge;
                                    editable_drag_value(
                                        ui,
                                        egui::DragValue::new(&mut slope.low_edge).range(0..=max),
                                    );
                                    if slope.low_edge != before {
                                        changed = true;
                                    }
                                });
                            }
                        });

                        changed |= surface_system_picker(
                            ui,
                            "ceiling-system",
                            &mut ceiling.system,
                            &ceiling_systems,
                            &ceiling_system_summaries,
                            &mut deferred_select_system,
                        );
                    } else {
                        ui.label(&ceiling.name);
                        ui.label(format!("System: {}", ceiling.system.0));
                        ui.label(format!("Height below level top: {}", ceiling.height));
                        ui.label(format!(
                            "Region: {}",
                            surface_region_summary(&ceiling.region)
                        ));
                        match &ceiling.slope {
                            Some(slope) => ui.label(format!(
                                "Slope: {}:{} (low edge {})",
                                slope.pitch.rise, slope.pitch.run, slope.low_edge
                            )),
                            None => ui.label("Slope: flat"),
                        };
                    }
                } else {
                    ui.label("Ceiling no longer exists");
                }
            }
            Selection::FloorDeck(id) => {
                if let Some(deck) = self.model.floor_decks.iter_mut().find(|d| d.id.0 == id) {
                    if can_edit {
                        changed |= text_edit(ui, "Name", &mut deck.name);

                        let mut level_id = deck.level.0.clone();
                        property_row(ui, "Level", |ui| {
                            ComboBox::from_id_salt("floor-level")
                                .selected_text(level_display_name(&level_options, &level_id))
                                .show_ui(ui, |ui| {
                                    for (lid, name) in &level_options {
                                        ui.selectable_value(&mut level_id, lid.clone(), name);
                                    }
                                });
                        });
                        if level_id != deck.level.0 {
                            deck.level = ElementId::new(level_id);
                            changed = true;
                        }

                        widgets::section(ui, "floor-dimensions", "Geometry", true, |ui| {
                            summary_row(ui, "Region", surface_region_summary(&deck.region));
                            // The joist span direction. `Explicit(..)` is authored
                            // elsewhere; the picker offers the three presets.
                            property_row(ui, "Joist span", |ui| {
                                ComboBox::from_id_salt("floor-span")
                                    .selected_text(span_direction_label(deck.span))
                                    .show_ui(ui, |ui| {
                                        for span in [
                                            framer_core::SpanDirection::Shorter,
                                            framer_core::SpanDirection::Along,
                                            framer_core::SpanDirection::Across,
                                        ] {
                                            changed |= ui
                                                .selectable_value(
                                                    &mut deck.span,
                                                    span,
                                                    span_direction_label(span),
                                                )
                                                .changed();
                                        }
                                    });
                            });
                        });

                        changed |= surface_system_picker(
                            ui,
                            "floor-system",
                            &mut deck.system,
                            &floor_systems,
                            &floor_system_summaries,
                            &mut deferred_select_system,
                        );
                    } else {
                        ui.label(&deck.name);
                        ui.label(format!("System: {}", deck.system.0));
                        ui.label(format!("Region: {}", surface_region_summary(&deck.region)));
                        ui.label(format!("Span: {}", span_direction_label(deck.span)));
                    }
                } else {
                    ui.label("Floor deck no longer exists");
                }
            }
        }

        if let Some(id) = selected_inspector_id.as_deref() {
            inspector_object_id(ui, id);
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
            Some(DeferredRemove::FurnishingInstance(instance_id)) => {
                self.edit("Remove furnishing", |app| {
                    let before = app.model.furnishing_instances.len();
                    app.model
                        .furnishing_instances
                        .retain(|instance| instance.id.0 != instance_id);
                    if app.model.furnishing_instances.len() != before {
                        app.selected = Selection::Wall;
                    }
                });
            }
            Some(DeferredRemove::MepInstance(instance_id)) => {
                self.edit("Remove MEP object", |app| {
                    let before = app.model.mep_instances.len();
                    app.model
                        .mep_instances
                        .retain(|instance| instance.id.0 != instance_id);
                    if app.model.mep_instances.len() != before {
                        app.selected = Selection::Wall;
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
        if let Some(item) = deferred_resync_library {
            self.resync_library_item(item);
        }
        if let Some(item) = deferred_detach_library {
            self.detach_library_item(item);
        }
        if deferred_standards.add_project_pack {
            self.add_project_standards_pack();
        }
        if let Some(id) = deferred_standards.insert_starter_pack {
            self.insert_starter_standards_pack(id);
        }
        if let Some(id) = deferred_standards.add_existing_pack {
            self.add_standards_pack_to_stack(id);
        }
        if let Some((id, dir)) = deferred_standards.move_stack {
            self.move_standards_pack_in_stack(id, dir);
        }
        if let Some(id) = deferred_standards.remove_stack {
            self.remove_standards_pack_from_stack(id);
        }
        if let Some((rule, reason)) = deferred_standards.waive_rule {
            self.waive_standards_rule(rule, reason);
        }
        if let Some(system_id) = deferred_select_system {
            self.apply_selection(Selection::System(system_id), None, SelectionOp::Replace);
        }
        if let Some((roof_id, eave, rake)) = deferred_roof_overhang_sync {
            sync_connected_roof_overhangs(&mut self.model, &roof_id, eave, rake);
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

        self.inspector_output_sections(ui);
    }

    fn inspector_output_sections(&mut self, ui: &mut Ui) {
        if self.workspace_mode.shows_generated_plan() {
            ui.separator();
            if let Some(source) = diagnostics_panel(
                ui,
                self.error.as_deref(),
                self.project_plan.as_ref(),
                &self.geometry_audit,
                &self.model,
            ) {
                self.focus_diagnostic(source);
            }
            ui.separator();
            if let Some(source) = compliance_panel(ui, self.compliance_report.as_ref()) {
                self.focus_compliance_source(source);
            }
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
            WorkspaceMode::Render => "Render",
            WorkspaceMode::Plan => "Plan",
        }
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        let t = design::active();
        let diagnostics = self.plan_diagnostics();
        let error = self.error.clone();
        let diagnostic_counts = diagnostic_counts(
            error.as_deref(),
            &diagnostics,
            self.geometry_audit.violations.len(),
        );
        let zoom_percent = self.status_zoom_percent();

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(design::space::MD, 2.0);
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
                    RichText::new(format!("X {}   Y {}", cursor.x, cursor.y))
                        .monospace()
                        .size(design::text_size::LABEL)
                        .color(t.text_muted),
                );
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = design::space::MD;
                if let Some(zoom_percent) = zoom_percent {
                    ui.label(
                        RichText::new(format!("{zoom_percent}%"))
                            .size(design::text_size::LABEL)
                            .color(t.text_secondary),
                    );
                    toolbar_divider(ui);
                }
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
                            widgets::toggle_switch(ui, &mut self.layers.joins, "Corners");
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
                if let Some(source) = status_diagnostics_menu(
                    ui,
                    &self.model,
                    error.as_deref(),
                    &diagnostics,
                    &self.geometry_audit,
                    diagnostic_counts,
                ) {
                    self.focus_diagnostic(source);
                }
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
        let current = self.active_level_id().0;
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
            self.set_active_level(ElementId::new(chosen.clone()));
            self.apply_selection(Selection::Level(chosen), None, SelectionOp::Replace);
        }
    }

    pub(super) fn selection_status(&self) -> String {
        let selected_component_count = self.selected_component_count();
        let status = if selected_component_count > 1 {
            format!("{selected_component_count} components selected")
        } else {
            match &self.selected {
                Selection::None => "Nothing selected".to_owned(),
                Selection::Site => "Site & standards".to_owned(),
                Selection::Level(id) => format!("Level: {}", self.level_name(id)),
                Selection::Wall => self
                    .model
                    .walls
                    .get(self.selected_wall)
                    .map(|wall| format!("Wall segment: {}", wall.name))
                    .unwrap_or_else(|| "Wall segment".to_owned()),
                Selection::Opening(id) => format!("Opening: {}", self.opening_name(id)),
                Selection::Dimension(id) => format!("Dimension: {}", self.dimension_name(id)),
                Selection::Join(id) => format!("Corner: {}", self.corner_name(id)),
                Selection::Room(id) => format!("Room: {}", self.room_name(id)),
                Selection::Member { member_id, .. } => format!("Member: {member_id}"),
                Selection::RoofPlane(id) => format!("Roof plane: {}", self.roof_plane_name(id)),
                Selection::Ceiling(id) => format!("Ceiling: {}", self.ceiling_name(id)),
                Selection::FloorDeck(id) => format!("Floor deck: {}", self.floor_deck_name(id)),
                Selection::System(id) => format!("System: {}", self.system_name(id)),
                Selection::Material(id) => format!("Material: {}", self.material_name(id)),
                Selection::Furnishing(id) => format!("Furnishing: {}", self.furnishing_name(id)),
                Selection::MepObject(id) => format!("MEP object: {}", self.mep_object_name(id)),
                Selection::StandardsPack(id) => {
                    format!("Standards pack: {}", self.standards_pack_name(id))
                }
                Selection::FurnishingInstance(id) => {
                    format!("Furnishing instance: {}", self.furnishing_instance_name(id))
                }
                Selection::MepInstance(id) => {
                    format!("MEP instance: {}", self.mep_instance_name(id))
                }
            }
        };

        if let Some(mode) = (self.workspace_mode != WorkspaceMode::Render)
            .then(|| self.component_visibility.isolation_mode())
            .flatten()
            .map(IsolationMode::label)
        {
            format!("{status} • Isolated: {mode}")
        } else {
            status
        }
    }

    fn level_name(&self, id: &str) -> String {
        self.model
            .levels
            .iter()
            .find(|level| level.id.0 == id)
            .map(|level| level.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn opening_name(&self, id: &str) -> String {
        self.model
            .walls
            .iter()
            .flat_map(|wall| wall.openings.iter())
            .find(|opening| opening.id.0 == id)
            .map(|opening| opening.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn dimension_name(&self, id: &str) -> String {
        self.model
            .walls
            .iter()
            .flat_map(|wall| wall.dimensions.iter())
            .find(|dimension| dimension.id.0 == id)
            .map(|dimension| dimension.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn corner_name(&self, id: &str) -> String {
        self.model
            .wall_joins
            .iter()
            .find(|join| join.id.0 == id)
            .map(|join| join.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn room_name(&self, id: &str) -> String {
        self.model
            .rooms
            .iter()
            .find(|room| room.id.0 == id)
            .map(|room| room.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn roof_plane_name(&self, id: &str) -> String {
        self.model
            .roof_planes
            .iter()
            .find(|plane| plane.id.0 == id)
            .map(|plane| plane.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn ceiling_name(&self, id: &str) -> String {
        self.model
            .ceilings
            .iter()
            .find(|ceiling| ceiling.id.0 == id)
            .map(|ceiling| ceiling.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn floor_deck_name(&self, id: &str) -> String {
        self.model
            .floor_decks
            .iter()
            .find(|deck| deck.id.0 == id)
            .map(|deck| deck.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn system_name(&self, id: &str) -> String {
        self.model
            .systems
            .iter()
            .find(|system| system.id.0 == id)
            .map(|system| system.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn material_name(&self, id: &str) -> String {
        self.model
            .materials
            .iter()
            .find(|material| material.id.0 == id)
            .map(|material| material.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn furnishing_name(&self, id: &str) -> String {
        self.model
            .furnishings
            .iter()
            .find(|furnishing| furnishing.id.0 == id)
            .map(|furnishing| furnishing.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn mep_object_name(&self, id: &str) -> String {
        self.model
            .mep_objects
            .iter()
            .find(|object| object.id.0 == id)
            .map(|object| object.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn standards_pack_name(&self, id: &str) -> String {
        self.model
            .standards_packs
            .iter()
            .find(|pack| pack.id.0 == id)
            .map(|pack| pack.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn furnishing_instance_name(&self, id: &str) -> String {
        self.model
            .furnishing_instances
            .iter()
            .find(|instance| instance.id.0 == id)
            .map(|instance| instance.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn mep_instance_name(&self, id: &str) -> String {
        self.model
            .mep_instances
            .iter()
            .find(|instance| instance.id.0 == id)
            .map(|instance| instance.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    fn status_zoom_percent(&self) -> Option<u32> {
        match self.viewport_mode {
            ViewportMode::Plan | ViewportMode::RoofPlan => Some(self.plan_view.zoom_percent()),
            ViewportMode::Elevation => Some(
                self.model
                    .walls
                    .get(self.selected_wall)
                    .and_then(|wall| self.elevation_views.get(&wall.id.0))
                    .map_or(100, |camera| camera.zoom_percent()),
            ),
            ViewportMode::Axonometric | ViewportMode::Render => None,
        }
    }

    fn plan_diagnostics(&self) -> Vec<PlanDiagnostic> {
        let Some(plan) = &self.project_plan else {
            return Vec::new();
        };
        plan.diagnostics
            .iter()
            .chain(
                plan.wall_plans
                    .iter()
                    .flat_map(|wall_plan| wall_plan.diagnostics.iter()),
            )
            .cloned()
            .collect()
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

fn header_command_button(
    ui: &mut Ui,
    head: design::Theme,
    id: ActionId,
    enabled: bool,
    tooltip_override: Option<&str>,
) -> Response {
    let action = actions::metadata(id);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(design::control::ICON_BTN), sense);
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, design::radius::SM, Color32::from_white_alpha(18));
    }
    let fg = if enabled { head.text } else { head.text_muted };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        action.icon.glyph().to_string(),
        design::icon_font(design::control::INLINE_ICON),
        fg,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, action.label));
    let tooltip = tooltip_override
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| action_tooltip(*action, None));
    action_response_with_tooltip(response, enabled, tooltip)
}

fn header_menu_action(
    ui: &mut Ui,
    id: ActionId,
    enabled: bool,
    disabled_reason: Option<&str>,
) -> Response {
    let action = actions::metadata(id);
    let response = ui.add_enabled(enabled, egui::Button::new(action.label));
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, action.label));
    action_response_with_tooltip(response, enabled, action_tooltip(*action, disabled_reason))
}

fn strong_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).strong().color(theme::text_primary()));
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
        Selection::None => "Edit selection",
        Selection::Site => "Edit site context",
        Selection::Level(_) => "Edit level",
        Selection::Wall => "Edit wall",
        Selection::Opening(_) => "Edit opening",
        Selection::Dimension(_) => "Edit dimension",
        Selection::Join(_) => "Edit corner",
        Selection::Room(_) => "Edit room",
        Selection::Member { .. } => "Edit",
        Selection::RoofPlane(_) => "Edit roof plane",
        Selection::Ceiling(_) => "Edit ceiling",
        Selection::FloorDeck(_) => "Edit floor deck",
        Selection::System(_) => "Edit system",
        Selection::Material(_) => "Edit material",
        Selection::Furnishing(_) => "Edit furnishing",
        Selection::MepObject(_) => "Edit MEP object",
        Selection::StandardsPack(_) => "Edit standards pack",
        Selection::FurnishingInstance(_) => "Edit furnishing placement",
        Selection::MepInstance(_) => "Edit MEP placement",
    }
}

/// A deletion requested from the inspector this frame, deferred until the
/// inspector's `&mut` borrow of the model ends so it can be replayed through
/// `FramerApp::edit` as one correctly-labelled undo step.
enum DeferredRemove {
    Opening(String),
    Dimension(String),
    FurnishingInstance(String),
    MepInstance(String),
}

#[derive(Default)]
struct DeferredStandardsActions {
    add_project_pack: bool,
    insert_starter_pack: Option<String>,
    add_existing_pack: Option<String>,
    move_stack: Option<(String, isize)>,
    remove_stack: Option<String>,
    waive_rule: Option<(String, String)>,
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

fn command_search_matches(action: actions::ActionMetadata, lowercase_query: &str) -> bool {
    if lowercase_query.is_empty() {
        return true;
    }

    let shortcut = action.shortcut().unwrap_or_default();
    [
        action.label,
        action.tooltip,
        action.search_category(),
        shortcut,
    ]
    .into_iter()
    .any(|value| contains_ascii_case_insensitive(value, lowercase_query))
}

fn contains_ascii_case_insensitive(haystack: &str, lowercase_needle: &str) -> bool {
    let needle = lowercase_needle.as_bytes();
    if needle.is_empty() {
        return true;
    }

    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn command_search_action(
    ui: &mut Ui,
    action: actions::ActionMetadata,
    enabled: bool,
    disabled_reason: Option<&str>,
) -> Response {
    let label = command_search_label(action);
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(label).min_size(Vec2::new(ui.available_width(), 30.0)),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, action.label));
    action_response_with_tooltip(response, enabled, action_tooltip(action, disabled_reason))
}

fn action_tool_button(
    ui: &mut Ui,
    id: ActionId,
    active: bool,
    enabled: bool,
    disabled_reason: Option<&str>,
) -> Response {
    let action = actions::metadata(id);
    let response = widgets::tool_button(ui, action.icon, action.label, active, enabled);
    action_response_with_tooltip(response, enabled, action_tooltip(*action, disabled_reason))
}

fn command_search_label(action: actions::ActionMetadata) -> String {
    let mut label = format!("{} — {}", action.label, action.search_category());
    if let Some(shortcut) = action.shortcut() {
        label.push(' ');
        label.push_str(shortcut);
    }
    label
}

fn action_tooltip(action: actions::ActionMetadata, disabled_reason: Option<&str>) -> String {
    let mut tooltip = disabled_reason.unwrap_or(action.tooltip).to_owned();
    if disabled_reason.is_none()
        && let Some(shortcut) = action.shortcut()
    {
        tooltip.push_str(" (");
        tooltip.push_str(shortcut);
        tooltip.push(')');
    }
    tooltip
}

fn action_response_with_tooltip(response: Response, enabled: bool, tooltip: String) -> Response {
    if enabled {
        response.on_hover_text(tooltip)
    } else {
        response.on_disabled_hover_text(tooltip)
    }
}

fn command_flyout_button(
    ui: &mut Ui,
    label: &'static str,
    tooltip: &'static str,
    add: impl FnOnce(&mut Ui),
) -> Response {
    let t = design::active();
    let button = egui::Button::new(
        RichText::new(label)
            .size(design::text_size::LABEL)
            .color(t.text_secondary),
    )
    .right_text(design::icon_text(Icon::ChevronDown, 11.0).color(t.text_muted))
    .fill(t.control)
    .stroke(t.soft_stroke())
    .corner_radius(design::radius::SM)
    .min_size(design::control::TOOL_BTN);
    let (response, _) = MenuButton::from_button(button)
        .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClick))
        .ui(ui, add);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response.on_hover_text(tooltip)
}

fn flyout_action(
    ui: &mut Ui,
    id: ActionId,
    enabled: bool,
    disabled_reason: Option<&str>,
) -> Response {
    let action = actions::metadata(id);
    let response = ui.add_enabled(enabled, egui::Button::new(action.label));
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, action.label));
    action_response_with_tooltip(response, enabled, action_tooltip(*action, disabled_reason))
}

struct RenderSettingDrag {
    range: std::ops::RangeInclusive<f32>,
    speed: f64,
    fixed_decimals: usize,
    suffix: &'static str,
    tooltip: &'static str,
}

impl RenderSettingDrag {
    fn degrees(range: std::ops::RangeInclusive<f32>, tooltip: &'static str) -> Self {
        Self {
            range,
            speed: 1.0,
            fixed_decimals: 0,
            suffix: "°",
            tooltip,
        }
    }

    fn exposure(tooltip: &'static str) -> Self {
        Self {
            range: 0.1..=4.0,
            speed: 0.05,
            fixed_decimals: 2,
            suffix: "×",
            tooltip,
        }
    }
}

fn render_setting_drag(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    config: RenderSettingDrag,
) -> Response {
    let t = design::active();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(
            RichText::new(label)
                .size(design::text_size::MICRO)
                .strong()
                .color(t.text_muted),
        );
        let response = editable_drag_value(
            ui,
            egui::DragValue::new(value)
                .range(config.range)
                .speed(config.speed)
                .fixed_decimals(config.fixed_decimals)
                .suffix(config.suffix),
        )
        .on_hover_text(config.tooltip);
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, label));
        response
    })
    .inner
}

const AUTHORING_WORKFLOW_TABS: &[WorkflowTab] = &[
    WorkflowTab::Design,
    WorkflowTab::Frame,
    WorkflowTab::Openings,
    WorkflowTab::Roofs,
    WorkflowTab::Annotate,
    // WorkflowTab::Inspect stays in the enum for future command routing, but it
    // stays hidden while it would render an empty command strip.
];

const OUTPUT_WORKFLOW_TABS: &[WorkflowTab] = &[WorkflowTab::Render, WorkflowTab::Plan];
const STATUS_TOAST_SECONDS: f64 = 4.0;

pub(crate) fn workflow_tab_label(tab: WorkflowTab) -> &'static str {
    match tab {
        WorkflowTab::Design => "Design",
        WorkflowTab::Frame => "Frame",
        WorkflowTab::Openings => "Openings",
        WorkflowTab::Roofs => "Roofs",
        WorkflowTab::Annotate => "Annotate",
        WorkflowTab::Inspect => "Inspect",
        WorkflowTab::Render => "Render",
        WorkflowTab::Plan => "Plan",
    }
}

fn toolbar_divider(ui: &mut Ui) {
    ui.separator();
}

fn panel_header(ui: &mut Ui, title: &str, badge: &str) {
    let t = design::active();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .strong()
                .size(design::text_size::HEADING)
                .color(t.text),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(badge)
                    .size(design::text_size::LABEL)
                    .color(t.text_muted),
            );
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

fn selection_inspector_id(
    model: &BuildingModel,
    selected_wall: usize,
    selection: &Selection,
) -> Option<String> {
    match selection {
        Selection::None | Selection::Site => None,
        Selection::Wall => model.walls.get(selected_wall).map(|wall| wall.id.0.clone()),
        Selection::Level(id)
        | Selection::Room(id)
        | Selection::Opening(id)
        | Selection::Dimension(id)
        | Selection::Join(id)
        | Selection::RoofPlane(id)
        | Selection::Ceiling(id)
        | Selection::FloorDeck(id)
        | Selection::System(id)
        | Selection::Material(id)
        | Selection::Furnishing(id)
        | Selection::MepObject(id)
        | Selection::StandardsPack(id)
        | Selection::FurnishingInstance(id)
        | Selection::MepInstance(id) => Some(id.clone()),
        Selection::Member { member_id, .. } => Some(member_id.clone()),
    }
}

fn empty_inspector_state(ui: &mut Ui) {
    ui.add_space(design::space::XL);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new("No selection")
                .strong()
                .color(design::active().text),
        );
        ui.label(
            RichText::new("Select an object to edit its properties.")
                .size(design::text_size::LABEL)
                .color(design::active().text_muted),
        );
    });
}

fn multi_selection_inspector(ui: &mut Ui, count: usize) {
    ui.add_space(design::space::XL);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(format!("{count} components selected"))
                .strong()
                .color(design::active().text),
        );
        ui.label(
            RichText::new("Properties are available when one component is selected.")
                .size(design::text_size::LABEL)
                .color(design::active().text_muted),
        );
    });
}

fn inspector_object_id(ui: &mut Ui, id: &str) {
    ui.add_space(design::space::LG);
    ui.separator();
    ui.add_space(design::space::SM);
    ui.label(
        RichText::new(format!("ID: {id}"))
            .monospace()
            .size(design::text_size::LABEL)
            .color(design::active().text_muted),
    );
}

fn danger_button(ui: &mut Ui, label: &str) -> Response {
    let t = design::active();
    ui.add(
        egui::Button::new(RichText::new(label).color(t.danger))
            .fill(t.control)
            .stroke(Stroke::new(1.0, t.danger))
            .corner_radius(design::radius::SM),
    )
    .on_hover_text(label)
}

fn danger_icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    let t = design::active();
    ui.add_sized(
        Vec2::splat(design::control::ICON_BTN),
        egui::Button::new(design::icon_text(icon, design::control::INLINE_ICON).color(t.danger))
            .fill(t.control)
            .stroke(Stroke::new(1.0, t.danger))
            .corner_radius(design::radius::SM),
    )
    .on_hover_text(tooltip)
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
                        ui.label(design::icon_text(Icon::Delete, 11.0).color(t.danger));
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
            .size(design::text_size::BODY)
            .color(theme::text_muted()),
    );
    ui.add_space(3.0);
}

fn site_context_editor(ui: &mut Ui, site: &mut framer_core::SiteContext) -> bool {
    let mut changed = false;
    changed |= text_edit(ui, "Jurisdiction", &mut site.jurisdiction);
    changed |= property_row(ui, "SDC", |ui| {
        let before = site.seismic;
        ComboBox::from_id_salt("site-seismic")
            .selected_text(
                site.seismic
                    .map(seismic_design_category_label)
                    .unwrap_or("Unknown"),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut site.seismic, None, "Unknown");
                for &category in SEISMIC_DESIGN_CATEGORIES {
                    ui.selectable_value(
                        &mut site.seismic,
                        Some(category),
                        seismic_design_category_label(category),
                    );
                }
            });
        site.seismic != before
    });
    changed |= optional_u32_drag(ui, "Wind", &mut site.wind_speed_mph, 70, 300, 115, "mph");
    changed |= optional_u32_drag(
        ui,
        "Snow",
        &mut site.ground_snow_load_psf,
        0,
        500,
        30,
        "psf",
    );
    changed |= optional_length_drag(
        ui,
        "Frost",
        &mut site.frost_depth,
        0.0,
        120.0,
        24.0,
        DisplayUnit::Inches,
    );
    changed
}

fn site_context_summary(ui: &mut Ui, site: &framer_core::SiteContext) {
    summary_row(
        ui,
        "Jurisdiction",
        if site.jurisdiction.is_empty() {
            "Unknown"
        } else {
            site.jurisdiction.as_str()
        },
    );
    summary_row(
        ui,
        "SDC",
        site.seismic
            .map(seismic_design_category_label)
            .unwrap_or("Unknown"),
    );
    summary_row(
        ui,
        "Wind",
        site.wind_speed_mph
            .map(|value| format!("{value} mph"))
            .unwrap_or_else(|| "Unknown".to_owned()),
    );
    summary_row(
        ui,
        "Snow",
        site.ground_snow_load_psf
            .map(|value| format!("{value} psf"))
            .unwrap_or_else(|| "Unknown".to_owned()),
    );
    summary_row(
        ui,
        "Frost",
        site.frost_depth
            .map(|value| value.to_string())
            .unwrap_or_else(|| "Unknown".to_owned()),
    );
}

fn standards_stack_panel(
    ui: &mut Ui,
    model: &framer_core::BuildingModel,
    selected: &mut Selection,
    actions: &mut DeferredStandardsActions,
) {
    if model.standards.is_empty() {
        ui.label(RichText::new("No active standards packs").color(theme::text_muted()));
    }
    for (index, id) in model.standards.iter().enumerate() {
        let Some(pack) = model.standards_packs.iter().find(|pack| pack.id == *id) else {
            continue;
        };
        ui.horizontal(|ui| {
            let row_selected =
                matches!(selected, Selection::StandardsPack(selected_id) if selected_id == &id.0);
            if ui
                .selectable_label(row_selected, format!("{}. {}", index + 1, pack.name))
                .clicked()
            {
                *selected = Selection::StandardsPack(id.0.clone());
            }
            if pack.source.is_some() {
                library_badge(ui);
            }
            if ui
                .add_enabled(index > 0, |ui: &mut Ui| {
                    widgets::icon_button(ui, Icon::ChevronUp, "Move earlier in stack")
                })
                .clicked()
            {
                actions.move_stack = Some((id.0.clone(), -1));
            }
            if ui
                .add_enabled(index + 1 < model.standards.len(), |ui: &mut Ui| {
                    widgets::icon_button(ui, Icon::ChevronDown, "Move later in stack")
                })
                .clicked()
            {
                actions.move_stack = Some((id.0.clone(), 1));
            }
            if danger_icon_button(ui, Icon::Delete, "Remove from stack").clicked() {
                actions.remove_stack = Some(id.0.clone());
            }
        });
    }

    let inactive = model
        .standards_packs
        .iter()
        .filter(|pack| !model.standards.iter().any(|id| id == &pack.id))
        .collect::<Vec<_>>();
    if !inactive.is_empty() {
        ui.separator();
        strong_label(ui, "Inactive packs");
        for pack in inactive {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(
                        matches!(selected, Selection::StandardsPack(id) if id == &pack.id.0),
                        &pack.name,
                    )
                    .clicked()
                {
                    *selected = Selection::StandardsPack(pack.id.0.clone());
                }
                if pack.source.is_some() {
                    library_badge(ui);
                }
                if widgets::icon_button(ui, Icon::Plus, "Add to stack").clicked() {
                    actions.add_existing_pack = Some(pack.id.0.clone());
                }
            });
        }
    }

    ui.separator();
    ui.horizontal_wrapped(|ui| {
        if widgets::icon_button(ui, Icon::Plus, "New project pack").clicked() {
            actions.add_project_pack = true;
        }
        if let Ok(loaded) = framer_library::starter_library_ref() {
            for pack in &loaded.library.standards {
                if ui.button(format!("Import {}", pack.name)).clicked() {
                    actions.insert_starter_pack = Some(pack.id.0.clone());
                }
            }
        }
    });
}

fn standards_stack_summary(ui: &mut Ui, model: &framer_core::BuildingModel) {
    for (index, id) in model.standards.iter().enumerate() {
        let name = model
            .standards_packs
            .iter()
            .find(|pack| pack.id == *id)
            .map(|pack| pack.name.as_str())
            .unwrap_or(id.0.as_str());
        let label = format!("Pack {}", index + 1);
        summary_row(ui, &label, name);
    }
}

fn standards_pack_stack_controls(
    ui: &mut Ui,
    id: &str,
    stack_index: Option<usize>,
    stack_len: usize,
    actions: &mut DeferredStandardsActions,
) {
    match stack_index {
        Some(index) => {
            summary_row(ui, "Position", format!("{} of {stack_len}", index + 1));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(index > 0, |ui: &mut Ui| {
                        widgets::icon_button(ui, Icon::ChevronUp, "Move earlier in stack")
                    })
                    .clicked()
                {
                    actions.move_stack = Some((id.to_owned(), -1));
                }
                if ui
                    .add_enabled(index + 1 < stack_len, |ui: &mut Ui| {
                        widgets::icon_button(ui, Icon::ChevronDown, "Move later in stack")
                    })
                    .clicked()
                {
                    actions.move_stack = Some((id.to_owned(), 1));
                }
                if danger_icon_button(ui, Icon::Delete, "Remove from stack").clicked() {
                    actions.remove_stack = Some(id.to_owned());
                }
            });
        }
        None => {
            summary_row(ui, "Status", "Inactive");
            if widgets::icon_button(ui, Icon::Plus, "Add to stack").clicked() {
                actions.add_existing_pack = Some(id.to_owned());
            }
        }
    }
}

type StandardsRuleRow = (String, String, String, Option<String>, bool);

#[derive(Debug, Clone, PartialEq, Eq)]
struct StandardsRuleMetadata {
    citation: String,
    pack: String,
    waived: Option<String>,
    has_check: bool,
}

fn standards_rule_rows(model: &framer_core::BuildingModel) -> Vec<StandardsRuleRow> {
    let mut rules = BTreeMap::<String, StandardsRuleMetadata>::new();
    for pack_id in &model.standards {
        let Some(pack) = model
            .standards_packs
            .iter()
            .find(|pack| pack.id == *pack_id)
        else {
            continue;
        };
        for table in &pack.tables.studs {
            insert_standards_rule_metadata(&mut rules, pack, &table.rule, &table.citation, false);
        }
        for table in &pack.tables.headers {
            insert_standards_rule_metadata(&mut rules, pack, &table.rule, &table.citation, false);
        }
        for schedule in &pack.tables.fastening {
            insert_standards_rule_metadata(
                &mut rules,
                pack,
                &schedule.rule,
                &schedule.citation,
                false,
            );
        }
        for table in &pack.tables.bracing {
            insert_standards_rule_metadata(&mut rules, pack, &table.rule, &table.citation, false);
        }
        for check in &pack.checks {
            insert_standards_rule_metadata(&mut rules, pack, &check.rule, &check.citation, true);
        }
        for overlay in &pack.overlays {
            match overlay {
                framer_core::RuleOverlay::Waive { target, reason } => {
                    if let Some(entry) = rules.get_mut(target) {
                        entry.waived = Some(reason.clone());
                    }
                }
                framer_core::RuleOverlay::Severity { .. } => {}
            }
        }
    }

    rules
        .into_iter()
        .map(|(rule, metadata)| {
            (
                rule,
                metadata.citation,
                metadata.pack,
                metadata.waived,
                metadata.has_check,
            )
        })
        .collect()
}

fn insert_standards_rule_metadata(
    rules: &mut BTreeMap<String, StandardsRuleMetadata>,
    pack: &framer_core::StandardsPack,
    rule: &str,
    citation: &str,
    has_check: bool,
) {
    rules.insert(
        rule.to_owned(),
        StandardsRuleMetadata {
            citation: citation.to_owned(),
            pack: pack.id.0.clone(),
            waived: None,
            has_check,
        },
    );
}

fn standards_waiver_editor(
    ui: &mut Ui,
    pack_id: &str,
    rules: &[StandardsRuleRow],
    actions: &mut DeferredStandardsActions,
) {
    if rules.is_empty() {
        ui.label(RichText::new("No resolved rules").color(theme::text_muted()));
        return;
    }

    for (rule, citation, source_pack, waived, has_check) in rules {
        ui.separator();
        ui.label(RichText::new(rule).strong());
        ui.label(
            RichText::new(format!("{citation} · pack {source_pack}"))
                .size(design::text_size::LABEL)
                .color(theme::text_secondary()),
        );
        if let Some(reason) = waived {
            ui.label(
                RichText::new(format!("Waived: {reason}"))
                    .size(design::text_size::LABEL)
                    .color(theme::text_muted()),
            );
        } else if !*has_check {
            ui.label(
                RichText::new("Table-driven rule")
                    .size(design::text_size::LABEL)
                    .color(theme::text_muted()),
            );
        }

        let id = ui.id().with(format!("waive-{pack_id}-{rule}"));
        let mut draft = ui.data_mut(|data| data.get_temp::<String>(id).unwrap_or_default());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut draft)
                    .hint_text("Waiver reason")
                    .desired_width(180.0),
            );
            let enabled = !draft.trim().is_empty();
            if ui
                .add_enabled(enabled, egui::Button::new("Waive"))
                .on_hover_text("Create a project-local waive overlay")
                .clicked()
            {
                actions.waive_rule = Some((rule.clone(), draft.trim().to_owned()));
                draft.clear();
            }
        });
        ui.data_mut(|data| data.insert_temp(id, draft));
    }
}

fn standards_pack_summary(
    ui: &mut Ui,
    pack: &framer_core::StandardsPack,
    stack_index: Option<usize>,
) {
    summary_row(ui, "Name", &pack.name);
    summary_row(ui, "Edition", &pack.edition);
    summary_row(
        ui,
        "Status",
        stack_index
            .map(|index| format!("Stack position {}", index + 1))
            .unwrap_or_else(|| "Inactive".to_owned()),
    );
    summary_row(ui, "Overlays", pack.overlays.len().to_string());
}

fn optional_u32_drag(
    ui: &mut Ui,
    label: &str,
    value: &mut Option<u32>,
    min: u32,
    max: u32,
    default: u32,
    suffix: &str,
) -> bool {
    property_row(ui, label, |ui| {
        let mut changed = false;
        if let Some(current) = value {
            let response = editable_drag_value(
                ui,
                egui::DragValue::new(current)
                    .range(min..=max)
                    .speed(1.0)
                    .suffix(format!(" {suffix}")),
            );
            changed |= response.changed();
            if danger_icon_button(ui, Icon::Delete, "Clear value").clicked() {
                *value = None;
                changed = true;
            }
        } else {
            ui.label(RichText::new("Unknown").color(theme::text_muted()));
            if widgets::icon_button(ui, Icon::Plus, "Set value").clicked() {
                *value = Some(default.clamp(min, max));
                changed = true;
            }
        }
        changed
    })
}

fn optional_length_drag(
    ui: &mut Ui,
    label: &str,
    value: &mut Option<Length>,
    min_inches: f64,
    max_inches: f64,
    default_inches: f64,
    display_unit: DisplayUnit,
) -> bool {
    property_row(ui, label, |ui| {
        let mut changed = false;
        if let Some(current) = value {
            let mut display_value = display_unit.value(*current);
            let response = editable_drag_value(
                ui,
                length_drag_widget(&mut display_value, display_unit, min_inches, max_inches),
            );
            if response.changed() {
                let next_inches = display_unit.length(display_value).inches();
                *current = Length::from_inches(next_inches.clamp(min_inches, max_inches));
                changed = true;
            }
            if danger_icon_button(ui, Icon::Delete, "Clear value").clicked() {
                *value = None;
                changed = true;
            }
        } else {
            ui.label(RichText::new("Unknown").color(theme::text_muted()));
            if widgets::icon_button(ui, Icon::Plus, "Set value").clicked() {
                *value = Some(Length::from_inches(
                    default_inches.clamp(min_inches, max_inches),
                ));
                changed = true;
            }
        }
        changed
    })
}

const SEISMIC_DESIGN_CATEGORIES: &[SeismicDesignCategory] = &[
    SeismicDesignCategory::A,
    SeismicDesignCategory::B,
    SeismicDesignCategory::C,
    SeismicDesignCategory::D0,
    SeismicDesignCategory::D1,
    SeismicDesignCategory::D2,
    SeismicDesignCategory::E,
];

fn seismic_design_category_label(category: SeismicDesignCategory) -> &'static str {
    match category {
        SeismicDesignCategory::A => "A",
        SeismicDesignCategory::B => "B",
        SeismicDesignCategory::C => "C",
        SeismicDesignCategory::D0 => "D0",
        SeismicDesignCategory::D1 => "D1",
        SeismicDesignCategory::D2 => "D2",
        SeismicDesignCategory::E => "E",
    }
}

/// A one-line description of a ceiling/floor-deck surface region for the inspector.
fn surface_region_summary(region: &framer_core::SurfaceRegion) -> String {
    match region {
        framer_core::SurfaceRegion::Room(id) => format!("room {}", id.0),
        framer_core::SurfaceRegion::Polygon(points) => format!("polygon ({} pts)", points.len()),
    }
}

/// The joist span-direction label for the floor-deck inspector.
fn span_direction_label(span: framer_core::SpanDirection) -> &'static str {
    match span {
        framer_core::SpanDirection::Shorter => "shorter clear span",
        framer_core::SpanDirection::Along => "along",
        framer_core::SpanDirection::Across => "across",
        framer_core::SpanDirection::Explicit(_) => "explicit",
    }
}

fn selection_badge(selection: &Selection) -> &'static str {
    match selection {
        Selection::None => "None",
        Selection::Site => "Site",
        Selection::Level(_) => "Level",
        Selection::Wall => "Wall",
        Selection::Opening(_) => "Opening",
        Selection::Dimension(_) => "Dimension",
        Selection::Join(_) => "Corner",
        Selection::Room(_) => "Room",
        Selection::Member { .. } => "Member",
        Selection::RoofPlane(_) => "Roof",
        Selection::Ceiling(_) => "Ceiling",
        Selection::FloorDeck(_) => "Floor",
        Selection::System(_) => "System",
        Selection::Material(_) => "Material",
        Selection::Furnishing(_) => "Furnishing",
        Selection::MepObject(_) => "MEP",
        Selection::StandardsPack(_) => "Standards",
        Selection::FurnishingInstance(_) => "Placed Furnishing",
        Selection::MepInstance(_) => "Placed MEP",
    }
}

#[derive(Clone, Copy)]
enum StatusTone {
    Info,
    Success,
    Warning,
}

fn status_chip(ui: &mut Ui, text: &str, tone: StatusTone) {
    let t = design::active();
    let (fill, stroke, text_color) = match tone {
        StatusTone::Info => (t.accent_soft, t.accent, t.text),
        StatusTone::Success => (t.success_soft, t.success, t.text),
        StatusTone::Warning => (t.warning_soft, t.warning, t.text),
    };

    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(design::text_size::LABEL)
                    .color(text_color),
            );
        });
}

fn tree_row(ui: &mut Ui, selected: bool, icon: Icon, name: &str, kind: &str) -> Response {
    ui.horizontal(|ui| tree_row_contents(ui, selected, icon, name, kind))
        .inner
}

/// A selectable component row with an independent trailing visibility control.
/// The visibility response is separate so hiding a selected component never
/// changes selection and the hidden row remains available to show again.
#[derive(Clone, Copy)]
struct ComponentTreeRowState {
    selected: bool,
    visible: bool,
    visibility_available: bool,
}

impl ComponentTreeRowState {
    fn new(selected: bool, visible: bool, visibility_available: bool) -> Self {
        Self {
            selected,
            visible,
            visibility_available,
        }
    }
}

fn component_tree_row(
    ui: &mut Ui,
    key: &ComponentKey,
    icon: Icon,
    name: &str,
    kind: &str,
    state: ComponentTreeRowState,
) -> (Response, Response) {
    ui.push_id(("component-row", key), |ui| {
        ui.horizontal(|ui| {
            let row = tree_row_contents(ui, state.selected, icon, name, kind);
            let eye = ui
                .with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if state.visible {
                        format!("Hide {name}")
                    } else {
                        format!("Show {name}")
                    };
                    let color = if state.visible {
                        design::active().text_secondary
                    } else {
                        design::active().text_muted
                    };
                    let response = ui
                        .add_enabled_ui(state.visibility_available, |ui| {
                            widgets::ghost_icon_button(ui, Icon::Eye, color, &label)
                        })
                        .inner;
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            state.visibility_available,
                            &label,
                        )
                    });
                    let response = if state.visibility_available {
                        response
                    } else {
                        response.on_disabled_hover_text(
                            "Visibility is unavailable for this component in the active view",
                        )
                    };
                    if !state.visible {
                        let rect = response.rect.shrink(8.0);
                        ui.painter().line_segment(
                            [rect.left_top(), rect.right_bottom()],
                            Stroke::new(1.25, design::active().text_muted),
                        );
                    }
                    response
                })
                .inner;
            (row, eye)
        })
        .inner
    })
    .inner
}

fn component_selection_op(ui: &Ui) -> SelectionOp {
    if ui.input(|input| input.modifiers.contains(egui::Modifiers::COMMAND)) {
        SelectionOp::Toggle
    } else {
        SelectionOp::Replace
    }
}

fn tree_row_contents(ui: &mut Ui, selected: bool, icon: Icon, name: &str, kind: &str) -> Response {
    let tooltip = format!("{kind}: {name}");
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.label(design::icon_text(icon, 13.0).color(design::active().text_muted))
        .on_hover_text(tooltip.clone());
    ui.selectable_label(selected, name).on_hover_text(tooltip)
}

fn tree_static_row_contents(ui: &mut Ui, icon: Icon, name: &str, kind: &str) {
    let tooltip = format!("{kind}: {name}");
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.label(design::icon_text(icon, 13.0).color(design::active().text_muted))
        .on_hover_text(tooltip.clone());
    ui.label(name).on_hover_text(tooltip);
}

fn opening_tree_icon(kind: OpeningKind) -> Icon {
    match kind {
        OpeningKind::Door => Icon::Door,
        OpeningKind::Window | OpeningKind::Skylight => Icon::Window,
        OpeningKind::GarageDoor => Icon::GarageDoor,
        OpeningKind::Stair => Icon::Floor,
    }
}

fn member_tree_icon(kind: framer_solver::MemberKind) -> Icon {
    match kind {
        framer_solver::MemberKind::FloorJoist | framer_solver::MemberKind::RimJoist => Icon::Floor,
        framer_solver::MemberKind::CeilingJoist => Icon::Ceiling,
        framer_solver::MemberKind::Rafter
        | framer_solver::MemberKind::RidgeBoard
        | framer_solver::MemberKind::HipRafter
        | framer_solver::MemberKind::ValleyRafter
        | framer_solver::MemberKind::JackRafter => Icon::Roof,
        _ => Icon::Wall,
    }
}

#[derive(Clone, Copy)]
struct DiagnosticCounts {
    errors: usize,
    unsupported: usize,
    warnings: usize,
    info: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum DiagnosticAction {
    Source(ElementId),
    Geometry(GeometryViolation),
}

const UNSUPPORTED_DIAGNOSTIC_TOOLTIP: &str =
    "Conditions outside the supported prescriptive scope — see diagnostics";

fn count_diagnostics(diagnostics: &[PlanDiagnostic]) -> (usize, usize, usize) {
    diagnostics.iter().fold(
        (0, 0, 0),
        |(unsupported, warnings, info), diagnostic| match diagnostic.severity {
            DiagnosticSeverity::Unsupported | DiagnosticSeverity::Violation => {
                (unsupported + 1, warnings, info)
            }
            DiagnosticSeverity::Warning | DiagnosticSeverity::NeedsReview => {
                (unsupported, warnings + 1, info)
            }
            DiagnosticSeverity::Info => (unsupported, warnings, info + 1),
        },
    )
}

fn diagnostic_counts(
    error: Option<&str>,
    diagnostics: &[PlanDiagnostic],
    geometry_violations: usize,
) -> DiagnosticCounts {
    let (unsupported, warnings, info) = count_diagnostics(diagnostics);
    DiagnosticCounts {
        errors: usize::from(error.is_some()) + geometry_violations,
        unsupported,
        warnings,
        info,
    }
}

fn diagnostic_row_budget(
    geometry_violations: usize,
    plan_diagnostics: usize,
    limit: usize,
) -> (usize, usize, usize) {
    let shown_geometry = geometry_violations.min(limit);
    let shown_plan = plan_diagnostics.min(limit.saturating_sub(shown_geometry));
    let hidden = geometry_violations + plan_diagnostics - shown_geometry - shown_plan;
    (shown_geometry, shown_plan, hidden)
}

fn diagnostics_status_label(counts: DiagnosticCounts) -> String {
    format!(
        "{} errors   {} warnings   {} unsupported   {} info",
        counts.errors, counts.warnings, counts.unsupported, counts.info
    )
}

fn status_diagnostics_menu(
    ui: &mut Ui,
    model: &BuildingModel,
    error: Option<&str>,
    diagnostics: &[PlanDiagnostic],
    geometry_audit: &GeometryAudit,
    counts: DiagnosticCounts,
) -> Option<DiagnosticAction> {
    let t = design::active();
    let text_color = if counts.errors > 0 {
        t.danger
    } else if counts.unsupported > 0 || counts.warnings > 0 {
        t.warning
    } else {
        t.text_secondary
    };
    let label = diagnostics_status_label(counts);
    let button = egui::Button::new(
        RichText::new(label)
            .size(design::text_size::LABEL)
            .color(text_color),
    )
    .frame(false);
    let (response, focused) = MenuButton::from_button(button).ui(ui, |ui| {
        ui.set_min_width(420.0);
        panel_subheader(ui, "Diagnostics");
        ui.horizontal_wrapped(|ui| {
            diagnostic_count_label(
                ui,
                Icon::Error,
                &format!("{} errors", counts.errors),
                if counts.errors == 0 {
                    t.text_muted
                } else {
                    t.danger
                },
                None,
            );
            diagnostic_count_label(
                ui,
                Icon::Warning,
                &format!("{} warnings", counts.warnings),
                if counts.warnings == 0 {
                    t.text_muted
                } else {
                    t.warning
                },
                None,
            );
            diagnostic_count_label(
                ui,
                Icon::Warning,
                &format!("{} unsupported", counts.unsupported),
                if counts.unsupported == 0 {
                    t.text_muted
                } else {
                    t.warning
                },
                Some(UNSUPPORTED_DIAGNOSTIC_TOOLTIP),
            );
            diagnostic_count_label(
                ui,
                Icon::Help,
                &format!("{} info", counts.info),
                t.text_muted,
                None,
            );
        });
        ui.separator();

        let mut focused = None;
        if let Some(error) = error {
            diagnostic_error_row(ui, error);
        }
        if diagnostics.is_empty() && geometry_audit.is_clean() && error.is_none() {
            ui.label(RichText::new("No diagnostics").color(t.text_secondary));
        } else {
            ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for violation in &geometry_audit.violations {
                    if let Some(action) = geometry_diagnostic_row(ui, violation) {
                        focused = Some(action);
                    }
                }
                for diagnostic in diagnostics {
                    if let Some(source) = diagnostic_row(ui, model, diagnostic) {
                        focused = Some(DiagnosticAction::Source(source));
                    }
                }
            });
        }
        focused
    });
    let enabled = response.enabled();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, "Diagnostics")
    });
    let tooltip = if counts.unsupported > 0 {
        format!("Diagnostics\n{UNSUPPORTED_DIAGNOSTIC_TOOLTIP}")
    } else {
        "Diagnostics".to_owned()
    };
    response.on_hover_text(tooltip);
    focused.and_then(|inner| inner.inner)
}

fn diagnostic_count_label(
    ui: &mut Ui,
    icon: Icon,
    text: &str,
    color: Color32,
    tooltip: Option<&str>,
) {
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(design::icon_text(icon, 13.0).color(color));
        ui.label(
            RichText::new(text)
                .size(design::text_size::LABEL)
                .color(design::active().text_secondary),
        );
    });
    if let Some(tooltip) = tooltip {
        row.response.on_hover_text(tooltip);
    }
}

fn snap_label(step: Option<Length>) -> String {
    match step {
        None => "Off".to_owned(),
        Some(step) if step == Length::from_inches(0.5) => "1/2 in".to_owned(),
        Some(step) => format!("{} in", step.inches()),
    }
}

fn level_summary(ui: &mut Ui, level: &Level) {
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

fn library_badge(ui: &mut Ui) {
    ui.label(
        RichText::new("Library")
            .size(11.0)
            .strong()
            .color(theme::active_blue()),
    );
}

fn provenance_label(source: &Provenance) -> String {
    let hash = source
        .content_hash
        .strip_prefix("blake3:")
        .unwrap_or(&source.content_hash);
    let short_hash = hash.get(..8).unwrap_or(hash);
    format!("{} ({short_hash})", source.source_id.0)
}

fn object_size_label(size: &framer_core::ObjectSize) -> String {
    format!("{} x {} x {}", size.width, size.depth, size.height)
}

fn matching_library_source(
    source: &Provenance,
    library: &framer_core::Library,
    id: &ElementId,
) -> bool {
    source.library_uid == library.uid
        && source.version_id == library.version_id
        && source.source_id == *id
}

fn matching_furnishing_family_id(
    model: &framer_core::BuildingModel,
    library: &framer_core::Library,
    source_id: &ElementId,
) -> Option<ElementId> {
    model.furnishings.iter().find_map(|furnishing| {
        furnishing
            .source
            .as_ref()
            .filter(|source| matching_library_source(source, library, source_id))
            .map(|_| furnishing.id.clone())
    })
}

fn matching_mep_object_id(
    model: &framer_core::BuildingModel,
    library: &framer_core::Library,
    source_id: &ElementId,
) -> Option<ElementId> {
    model.mep_objects.iter().find_map(|object| {
        object
            .source
            .as_ref()
            .filter(|source| matching_library_source(source, library, source_id))
            .map(|_| object.id.clone())
    })
}

#[derive(Debug, Clone)]
struct LibrarySelectionStatus {
    item: framer_library::LibraryItem,
    source: Provenance,
    issues: Vec<framer_library::LibraryIssueKind>,
    source_available: bool,
}

fn selected_library_status(
    model: &framer_core::BuildingModel,
    selection: &Selection,
    library_issues: &[framer_library::LibraryIssue],
) -> Option<LibrarySelectionStatus> {
    let (item, source) = match selection {
        Selection::System(id) => {
            let system = model.systems.iter().find(|system| system.id.0 == *id)?;
            let source = system.source.clone()?;
            (
                framer_library::LibraryItem::System(system.id.clone()),
                source,
            )
        }
        Selection::Material(id) => {
            let material = model
                .materials
                .iter()
                .find(|material| material.id.0 == *id)?;
            let MaterialSource::Library(source) = material.source.clone() else {
                return None;
            };
            (
                framer_library::LibraryItem::Material(material.id.clone()),
                source,
            )
        }
        Selection::Furnishing(id) => {
            let furnishing = model
                .furnishings
                .iter()
                .find(|furnishing| furnishing.id.0 == *id)?;
            let source = furnishing.source.clone()?;
            (
                framer_library::LibraryItem::Furnishing(furnishing.id.clone()),
                source,
            )
        }
        Selection::MepObject(id) => {
            let object = model.mep_objects.iter().find(|object| object.id.0 == *id)?;
            let source = object.source.clone()?;
            (
                framer_library::LibraryItem::MepObject(object.id.clone()),
                source,
            )
        }
        Selection::StandardsPack(id) => {
            let pack = model.standards_packs.iter().find(|pack| pack.id.0 == *id)?;
            let source = pack.source.clone()?;
            (
                framer_library::LibraryItem::StandardsPack(pack.id.clone()),
                source,
            )
        }
        _ => return None,
    };

    let loaded = framer_library::starter_library_ref().ok();
    let issues = library_issues
        .iter()
        .filter(|issue| issue.item == item)
        .map(|issue| issue.kind)
        .collect::<Vec<_>>();
    let source_available = loaded.as_ref().is_some_and(|loaded| {
        loaded.library.uid == source.library_uid
            && library_contains_source(&loaded.library, &item, &source.source_id)
    });

    Some(LibrarySelectionStatus {
        item,
        source,
        issues,
        source_available,
    })
}

fn library_contains_source(
    library: &framer_core::Library,
    item: &framer_library::LibraryItem,
    source_id: &ElementId,
) -> bool {
    match item {
        framer_library::LibraryItem::Material(_) => library
            .materials
            .iter()
            .any(|material| material.id == *source_id),
        framer_library::LibraryItem::System(_) => {
            library.systems.iter().any(|system| system.id == *source_id)
        }
        framer_library::LibraryItem::Furnishing(_) => library
            .furnishings
            .iter()
            .any(|furnishing| furnishing.id == *source_id),
        framer_library::LibraryItem::MepObject(_) => library
            .mep_objects
            .iter()
            .any(|object| object.id == *source_id),
        framer_library::LibraryItem::StandardsPack(_) => {
            library.standards.iter().any(|pack| pack.id == *source_id)
        }
    }
}

fn library_lifecycle_controls(
    ui: &mut Ui,
    status: &LibrarySelectionStatus,
    deferred_resync: &mut Option<framer_library::LibraryItem>,
    deferred_detach: &mut Option<framer_library::LibraryItem>,
) {
    summary_row(ui, "Source", provenance_label(&status.source));
    summary_row(ui, "Status", library_status_label(status));
    ui.horizontal(|ui| {
        if ui
            .add_enabled(status.source_available, egui::Button::new("Re-sync"))
            .on_hover_text("Replace the vendored copy from the current source library")
            .clicked()
        {
            *deferred_resync = Some(status.item.clone());
        }
        if ui
            .button("Detach")
            .on_hover_text("Keep this definition as project-owned content")
            .clicked()
        {
            *deferred_detach = Some(status.item.clone());
        }
    });
}

fn library_status_label(status: &LibrarySelectionStatus) -> &'static str {
    let diverged = status
        .issues
        .contains(&framer_library::LibraryIssueKind::Diverged);
    let out_of_date = status
        .issues
        .contains(&framer_library::LibraryIssueKind::OutOfDate);
    let source_missing = status
        .issues
        .contains(&framer_library::LibraryIssueKind::SourceMissing);

    if source_missing {
        "Source missing"
    } else if diverged && out_of_date {
        "Modified, update available"
    } else if diverged {
        "Modified"
    } else if out_of_date {
        "Update available"
    } else if !status.source_available {
        "Source unavailable"
    } else {
        "Current"
    }
}

fn library_item_id(item: &framer_library::LibraryItem) -> &ElementId {
    match item {
        framer_library::LibraryItem::Material(id)
        | framer_library::LibraryItem::System(id)
        | framer_library::LibraryItem::Furnishing(id)
        | framer_library::LibraryItem::MepObject(id)
        | framer_library::LibraryItem::StandardsPack(id) => id,
    }
}

fn library_item_kind_label(item: &framer_library::LibraryItem) -> &'static str {
    match item {
        framer_library::LibraryItem::Material(_) => "material",
        framer_library::LibraryItem::System(_) => "system",
        framer_library::LibraryItem::Furnishing(_) => "furnishing",
        framer_library::LibraryItem::MepObject(_) => "MEP object",
        framer_library::LibraryItem::StandardsPack(_) => "standards pack",
    }
}

/// Look up a material's swatch color by id, falling back to a neutral tone.
fn material_color(material_colors: &[(String, [u8; 3])], id: &str) -> Color32 {
    material_colors
        .iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, [r, g, b])| Color32::from_rgb(*r, *g, *b))
        .unwrap_or_else(theme::sheet_ruler)
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
/// A "System" inspector section for a roof/ceiling/floor object: a kind-filtered
/// system ComboBox plus the selected system's stacked swatch and summary, and an
/// "Edit system" jump. Mirrors the Wall inspector's System block. `salt` keys the
/// egui ids (so each object kind's widgets stay distinct). Returns whether the
/// system reference changed. The candidate `systems` / `summaries` lists are
/// already filtered to the object's kind, so picking from them keeps the model
/// valid.
fn surface_system_picker(
    ui: &mut Ui,
    salt: &str,
    current: &mut framer_core::ElementId,
    systems: &[(String, String, String, String)],
    summaries: &[WallSystemSummary],
    deferred_select: &mut Option<String>,
) -> bool {
    widgets::section(ui, salt, "System", true, |ui| {
        let mut changed = false;
        let selected_text = systems
            .iter()
            .find(|(id, ..)| *id == current.0)
            .map(|(_, name, ..)| name.clone())
            .unwrap_or_else(|| current.0.clone());
        changed |= property_row(ui, "System", |ui| {
            let before = current.0.clone();
            ComboBox::from_id_salt(salt)
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for (id, name, _, _) in systems {
                        ui.selectable_value(current, ElementId::new(id.clone()), name);
                    }
                });
            current.0 != before
        });
        ui.add_space(design::space::XS);
        if let Some(summary) = summaries.iter().find(|summary| summary.id == current.0) {
            stacked_swatch(ui, &summary.bands);
            ui.add_space(design::space::XS);
            egui::Grid::new(format!("{salt}-summary"))
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    summary_row(ui, "Name", &summary.name);
                    summary_row(ui, "Layers", summary.layer_count);
                    summary_row(ui, "Total thickness", &summary.total_thickness);
                    summary_row(ui, "Exposure", summary.exposure);
                    summary_row(ui, "R-value", format_r_value(summary.r_value_milli));
                });
            if ui.button("Edit system").clicked() {
                *deferred_select = Some(summary.id.clone());
            }
        } else if systems.is_empty() {
            ui.label(
                RichText::new("No matching system yet — add one in the Library.")
                    .size(design::text_size::LABEL)
                    .color(design::active().text_muted),
            );
        } else {
            ui.label(
                RichText::new("System not found in the model library.")
                    .size(design::text_size::LABEL)
                    .color(design::active().text_muted),
            );
        }
        changed
    })
    .unwrap_or(false)
}

/// The framing member family a system of `kind` produces, used to seed a layer's
/// `FramingSpec` when it first becomes a framing layer so the solver dispatches
/// the right member geometry (studs vs. rafters vs. joists).
fn default_member_family(kind: framer_core::SystemKind) -> framer_core::MemberFamily {
    use framer_core::{MemberFamily, SystemKind};
    match kind {
        SystemKind::Wall => MemberFamily::Stud,
        SystemKind::Roof => MemberFamily::Rafter,
        SystemKind::Floor => MemberFamily::FloorJoist,
        SystemKind::Ceiling => MemberFamily::CeilingJoist,
    }
}

#[allow(clippy::too_many_arguments)]
fn system_layer_editor(
    ui: &mut Ui,
    system_id: &str,
    index: usize,
    layer_count: usize,
    system_kind: framer_core::SystemKind,
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
    // Every framed assembly must keep exactly one framing layer, so its sole
    // framing layer cannot be removed (in addition to the general "keep one
    // layer" guard) and the Function picker may not introduce a second one.
    let is_only_framing = is_framing && framing_count <= 1;
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
                            danger_icon_button(ui, Icon::Delete, "Remove layer")
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
                changed |= length_drag(
                    ui,
                    "Thickness",
                    &mut layer.thickness,
                    0.0625,
                    48.0,
                    DisplayUnit::Inches,
                );
            }

            // Function. Switching to/from Framing keeps `framing` consistent so the
            // model stays valid (framing.is_some() iff function == Framing). Every
            // framed system must keep exactly one framing layer, so the picker never
            // offers a SECOND Framing layer (`another_layer_is_framing`) and never
            // lets the SOLE framing layer drop Framing (`is_only_framing`) — either
            // would leave the system with the wrong framing-layer count.
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
                            member_family: default_member_family(system_kind),
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
                changed |= length_drag(
                    ui,
                    "Spacing",
                    &mut framing.spacing,
                    1.0,
                    48.0,
                    DisplayUnit::Inches,
                );
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
            if let Some(source) = &system.source {
                summary_row(ui, "Source", provenance_label(source));
            }
        });
}

/// Appearance editor for a material. Returns whether it changed. Asset-backed
/// appearances keep their binary refs read-only here; the fallback color remains
/// editable.
fn material_appearance_editor(ui: &mut Ui, material: &mut framer_core::Material) -> bool {
    let mut changed = false;
    property_row(ui, "Appearance", |ui| match &mut material.appearance {
        framer_core::Appearance::SolidColor(rgb) => {
            changed |= ui.color_edit_button_srgb(rgb).changed();
        }
        framer_core::Appearance::Textured {
            color,
            texture,
            scale,
        } => {
            changed |= ui.color_edit_button_srgb(color).changed();
            ui.label(format!("texture · {} · {}", texture.media_type, scale));
            ui.monospace(&texture.hash);
        }
        framer_core::Appearance::DepthMapped {
            color,
            height,
            scale,
        } => {
            changed |= ui.color_edit_button_srgb(color).changed();
            ui.label(format!("height · {} · {}", height.media_type, scale));
            ui.monospace(&height.hash);
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
            editable_drag_value(
                ui,
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
            if let MaterialSource::Library(source) = &material.source {
                summary_row(ui, "Source", provenance_label(source));
            }
        });
}

fn wall_summary(ui: &mut Ui, wall: &Wall, level_options: &[(String, String)], system_name: &str) {
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
        changed |= text_edit(ui, "Name", &mut dimension.name);

        let previous_kind = dimension.kind;
        property_row(ui, "Kind", |ui| {
            ComboBox::from_id_salt("dimension-kind")
                .selected_text(dimension_kind_label(dimension.kind))
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut dimension.kind, DimensionKind::Driving, "Driving")
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut dimension.kind,
                            DimensionKind::Reference,
                            "Reference",
                        )
                        .changed();
                });
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
        property_row(ui, "Axis", |ui| {
            ComboBox::from_id_salt("dimension-axis")
                .selected_text(dimension_axis_label(dimension.axis))
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut dimension.axis,
                            DimensionAxis::Horizontal,
                            "Horizontal",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(&mut dimension.axis, DimensionAxis::Vertical, "Vertical")
                        .changed();
                });
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
            if length_drag(
                ui,
                "Distance",
                &mut value,
                1.0,
                axis_bound_inches,
                DisplayUnit::Inches,
            ) {
                dimension.value = Some(value);
                changed = true;
                apply_driving = true;
            }
            if unsatisfied {
                ui.colored_label(theme::danger(), "Unsatisfied driving dimension");
            }
        }

        ui.separator();
        if danger_button(ui, "Remove Dimension").clicked() {
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
    egui::Grid::new("member-inspector")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            strong_label(ui, "Use");
            ui.label(member.kind.label());
            ui.end_row();
            strong_label(ui, "Profile");
            ui.label(member.profile.label());
            ui.end_row();
            strong_label(ui, "Source");
            ui.label(&member.source.0);
            ui.end_row();
            strong_label(ui, "X");
            ui.label(member.x.to_string());
            ui.end_row();
            strong_label(ui, "Elevation");
            ui.label(member.elevation.to_string());
            ui.end_row();
            strong_label(ui, "Cut length");
            ui.label(member.cut_length.to_string());
            ui.end_row();
            strong_label(ui, "Drawn depth");
            ui.label(member.cross_section_depth.to_string());
            ui.end_row();
            strong_label(ui, "Rule");
            ui.label(&member.provenance.rule_id);
            ui.end_row();
        });
    ui.label(&member.provenance.summary);
}

fn object_size_editor(ui: &mut Ui, size: &mut framer_core::ObjectSize) -> bool {
    length_drag(
        ui,
        "Width",
        &mut size.width,
        1.0,
        240.0,
        DisplayUnit::Inches,
    ) | length_drag(
        ui,
        "Depth",
        &mut size.depth,
        1.0,
        240.0,
        DisplayUnit::Inches,
    ) | length_drag(
        ui,
        "Height",
        &mut size.height,
        1.0,
        144.0,
        DisplayUnit::Inches,
    )
}

fn family_picker(
    ui: &mut Ui,
    label: &str,
    selected: &mut ElementId,
    options: &[(String, String)],
) -> bool {
    property_row(ui, label, |ui| {
        let before = selected.0.clone();
        ComboBox::from_id_salt(label)
            .selected_text(family_display_name(options, &selected.0))
            .show_ui(ui, |ui| {
                for (id, name) in options {
                    ui.selectable_value(selected, ElementId::new(id.clone()), name);
                }
            });
        selected.0 != before
    })
}

fn level_picker(ui: &mut Ui, selected: &mut ElementId, options: &[(String, String)]) -> bool {
    property_row(ui, "Level", |ui| {
        let before = selected.0.clone();
        ComboBox::from_id_salt("object-level")
            .selected_text(level_display_name(options, &selected.0))
            .show_ui(ui, |ui| {
                for (id, name) in options {
                    ui.selectable_value(selected, ElementId::new(id.clone()), name);
                }
            });
        selected.0 != before
    })
}

fn rotation_picker(ui: &mut Ui, selected: &mut QuarterTurn) -> bool {
    property_row(ui, "Rotation", |ui| {
        let before = *selected;
        ComboBox::from_id_salt("object-rotation")
            .selected_text(selected.label())
            .show_ui(ui, |ui| {
                for turn in QuarterTurn::ALL {
                    ui.selectable_value(selected, turn, turn.label());
                }
            });
        *selected != before
    })
}

fn family_display_name(options: &[(String, String)], id: &str) -> String {
    options
        .iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| id.to_owned())
}

fn object_family_summary(
    ui: &mut Ui,
    name: &str,
    size: &framer_core::ObjectSize,
    kind: Option<&str>,
) {
    egui::Grid::new("object-family-summary")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Name", name);
            if let Some(kind) = kind {
                summary_row(ui, "Kind", kind);
            }
            summary_row(ui, "Size", object_size_label(size));
        });
}

fn placed_object_summary(
    ui: &mut Ui,
    name: &str,
    family: String,
    level_options: &[(String, String)],
    level: &str,
    position: Point2,
    rotation: QuarterTurn,
) {
    egui::Grid::new("placed-object-summary")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Name", name);
            summary_row(ui, "Family", family);
            summary_row(ui, "Level", level_display_name(level_options, level));
            summary_row(ui, "Position", format!("{}, {}", position.x, position.y));
            summary_row(ui, "Rotation", rotation.label());
        });
}

fn diagnostics_panel(
    ui: &mut Ui,
    error: Option<&str>,
    plan: Option<&ProjectFramePlan>,
    geometry_audit: &GeometryAudit,
    model: &BuildingModel,
) -> Option<DiagnosticAction> {
    panel_subheader(ui, "Diagnostics");
    let mut focused = None;
    if let Some(error) = error {
        diagnostic_error_row(ui, error);
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
            .cloned()
            .collect::<Vec<_>>();

        if diagnostics.is_empty() && geometry_audit.is_clean() {
            ui.label("No diagnostics");
            return focused;
        }

        let (unsupported, warnings, info) = count_diagnostics(&diagnostics);

        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "{} geometry violations",
                geometry_audit.violations.len()
            ));
            ui.label(format!("{unsupported} unsupported"))
                .on_hover_text(UNSUPPORTED_DIAGNOSTIC_TOOLTIP);
            ui.label(format!("{warnings} warnings"));
            ui.label(format!("{info} info"));
        });

        let (shown_geometry, shown_plan, hidden) =
            diagnostic_row_budget(geometry_audit.violations.len(), diagnostics.len(), 5);
        for violation in geometry_audit.violations.iter().take(shown_geometry) {
            if let Some(action) = geometry_diagnostic_row(ui, violation) {
                focused = Some(action);
            }
        }

        for diagnostic in diagnostics.iter().take(shown_plan) {
            if let Some(source) = diagnostic_row(ui, model, diagnostic) {
                focused = Some(DiagnosticAction::Source(source));
            }
        }

        if hidden > 0 {
            egui::CollapsingHeader::new(format!("{hidden} more diagnostics"))
                .default_open(false)
                .show(ui, |ui| {
                    for violation in geometry_audit.violations.iter().skip(shown_geometry) {
                        if let Some(action) = geometry_diagnostic_row(ui, violation) {
                            focused = Some(action);
                        }
                    }
                    for diagnostic in diagnostics.iter().skip(shown_plan) {
                        if let Some(source) = diagnostic_row(ui, model, diagnostic) {
                            focused = Some(DiagnosticAction::Source(source));
                        }
                    }
                });
        }
    } else if error.is_none() && geometry_audit.is_clean() {
        ui.label("No diagnostics");
    }
    focused
}

fn diagnostic_error_row(ui: &mut Ui, error: &str) {
    let t = design::active();
    ui.horizontal_wrapped(|ui| {
        ui.label(design::icon_text(Icon::Error, 13.0).color(t.danger));
        ui.label(RichText::new("Error").strong().color(t.danger));
        ui.label(RichText::new(error).color(t.text));
    });
    ui.add_space(4.0);
}

fn geometry_diagnostic_row(ui: &mut Ui, violation: &GeometryViolation) -> Option<DiagnosticAction> {
    let t = design::active();
    let body_a = geometry_body_label(violation.body_a());
    let body_b = violation.body_b().map(geometry_body_label);
    let row = ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(design::icon_text(Icon::Error, 13.0).color(t.danger));
            ui.label(RichText::new("Violation").strong().color(t.danger));
            ui.label(
                RichText::new(violation.code())
                    .size(design::text_size::LABEL)
                    .color(t.text_secondary),
            );
        });
        ui.label(
            RichText::new(format!("Body A: {body_a}"))
                .size(design::text_size::LABEL)
                .color(t.text_muted),
        );
        if let Some(body_b) = &body_b {
            ui.label(
                RichText::new(format!("Body B: {body_b}"))
                    .size(design::text_size::LABEL)
                    .color(t.text_muted),
            );
        }
        ui.label(
            RichText::new(geometry_violation_message(violation))
                .size(design::text_size::BODY)
                .color(t.text),
        );
    });
    let Some(label) = geometry_diagnostic_row_action_label(violation) else {
        ui.add_space(4.0);
        return None;
    };
    let response = ui
        .interact(
            row.response.rect,
            ui.id().with(format!(
                "geometry-diagnostic-row-{}-{}-{}",
                violation.code(),
                violation.body_a(),
                violation
                    .body_b()
                    .map_or_else(|| "none".to_owned(), ToString::to_string)
            )),
            egui::Sense::click(),
        )
        .on_hover_text("Focus physical geometry violation");
    let enabled = response.enabled();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.clone())
    });
    ui.add_space(4.0);
    response
        .clicked()
        .then(|| DiagnosticAction::Geometry(violation.clone()))
}

fn geometry_violation_message(violation: &GeometryViolation) -> String {
    match violation {
        GeometryViolation::BodyUnbuildable(diagnostic) => diagnostic.message.clone(),
        GeometryViolation::QueryUnsupported(diagnostic) => diagnostic.message.clone(),
        GeometryViolation::Overlap(diagnostic) => format!(
            "Penetration {:.6} in at witness ({:.6}, {:.6}, {:.6}) in",
            diagnostic.penetration_depth,
            diagnostic.witness.x,
            diagnostic.witness.y,
            diagnostic.witness.z
        ),
    }
}

pub(super) fn geometry_diagnostic_row_action_label(
    violation: &GeometryViolation,
) -> Option<String> {
    violation.body_b().map(|body_b| {
        format!(
            "Focus geometry violation {} between {} and {}",
            violation.code(),
            violation.body_a(),
            body_b
        )
    })
}

fn diagnostic_row(
    ui: &mut Ui,
    model: &BuildingModel,
    diagnostic: &PlanDiagnostic,
) -> Option<ElementId> {
    let t = design::active();
    let color = diagnostic_severity_color(diagnostic.severity);
    let source_label = diagnostic
        .source
        .as_ref()
        .map(|source| diagnostic_source_label(model, source));
    let row = ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(design::icon_text(diagnostic_icon(diagnostic.severity), 13.0).color(color));
            ui.label(
                RichText::new(diagnostic_code_prefix(diagnostic.severity))
                    .strong()
                    .color(color),
            );
            ui.label(
                RichText::new(&diagnostic.code)
                    .size(design::text_size::LABEL)
                    .color(t.text_secondary),
            );
            if let Some(source_label) = &source_label {
                ui.label(
                    RichText::new(source_label)
                        .size(design::text_size::LABEL)
                        .color(t.text_muted),
                );
            }
        });
        ui.label(
            RichText::new(&diagnostic.message)
                .size(design::text_size::BODY)
                .color(t.text),
        );
    });
    let mut focused = None;
    if let Some(source) = &diagnostic.source {
        let response = ui
            .interact(
                row.response.rect,
                ui.id()
                    .with(format!("diagnostic-row-{}-{}", diagnostic.code, source.0)),
                egui::Sense::click(),
            )
            .on_hover_text("Select source element");
        let enabled = response.enabled();
        let label = diagnostic_row_action_label(source);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.clone())
        });
        if response.clicked() {
            focused = Some(source.clone());
        }
    }
    ui.add_space(4.0);
    focused
}

pub(super) fn diagnostic_row_action_label(source: &ElementId) -> String {
    format!("Select diagnostic source {}", source.0)
}

fn diagnostic_icon(severity: DiagnosticSeverity) -> Icon {
    match severity {
        DiagnosticSeverity::Info => Icon::Help,
        DiagnosticSeverity::Warning | DiagnosticSeverity::NeedsReview => Icon::Warning,
        DiagnosticSeverity::Unsupported | DiagnosticSeverity::Violation => Icon::Error,
    }
}

fn diagnostic_severity_color(severity: DiagnosticSeverity) -> Color32 {
    match severity {
        DiagnosticSeverity::Info => theme::active_blue(),
        DiagnosticSeverity::Warning | DiagnosticSeverity::NeedsReview => theme::warning(),
        DiagnosticSeverity::Unsupported | DiagnosticSeverity::Violation => theme::danger(),
    }
}

fn diagnostic_source_label(model: &BuildingModel, source: &ElementId) -> String {
    let name = diagnostic_source_name(model, source);
    match name {
        Some(name) if name != source.0 => format!("{name} ({})", source.0),
        Some(name) => name,
        None => source.0.clone(),
    }
}

fn diagnostic_source_name(model: &BuildingModel, source: &ElementId) -> Option<String> {
    if let Some(wall) = model.walls.iter().find(|wall| wall.id == *source) {
        return Some(wall.name.clone());
    }
    for wall in &model.walls {
        if let Some(opening) = wall.openings.iter().find(|opening| opening.id == *source) {
            return Some(opening.name.clone());
        }
        if let Some(dimension) = wall
            .dimensions
            .iter()
            .find(|dimension| dimension.id == *source)
        {
            return Some(dimension.name.clone());
        }
    }
    if let Some(join) = model.wall_joins.iter().find(|join| join.id == *source) {
        return Some(join.name.clone());
    }
    if let Some(level) = model.levels.iter().find(|level| level.id == *source) {
        return Some(level.name.clone());
    }
    if let Some(room) = model.rooms.iter().find(|room| room.id == *source) {
        return Some(room.name.clone());
    }
    if let Some(plane) = model.roof_planes.iter().find(|plane| plane.id == *source) {
        return Some(plane.name.clone());
    }
    if let Some(ceiling) = model.ceilings.iter().find(|ceiling| ceiling.id == *source) {
        return Some(ceiling.name.clone());
    }
    if let Some(deck) = model.floor_decks.iter().find(|deck| deck.id == *source) {
        return Some(deck.name.clone());
    }
    if let Some(system) = model.systems.iter().find(|system| system.id == *source) {
        return Some(system.name.clone());
    }
    if let Some(material) = model
        .materials
        .iter()
        .find(|material| material.id == *source)
    {
        return Some(material.name.clone());
    }
    if let Some(furnishing) = model
        .furnishings
        .iter()
        .find(|furnishing| furnishing.id == *source)
    {
        return Some(furnishing.name.clone());
    }
    if let Some(object) = model.mep_objects.iter().find(|object| object.id == *source) {
        return Some(object.name.clone());
    }
    if let Some(pack) = model.standards_packs.iter().find(|pack| pack.id == *source) {
        return Some(pack.name.clone());
    }
    if let Some(instance) = model
        .furnishing_instances
        .iter()
        .find(|instance| instance.id == *source)
    {
        return Some(instance.name.clone());
    }
    if let Some(instance) = model
        .mep_instances
        .iter()
        .find(|instance| instance.id == *source)
    {
        return Some(instance.name.clone());
    }
    model
        .braced_wall_lines
        .iter()
        .find(|line| line.id == *source)
        .map(|line| line.name.clone())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComplianceGroup {
    Violations,
    NeedsReview,
    Advisories,
    Waived,
    Passed,
    NotApplicable,
}

impl ComplianceGroup {
    fn label(self) -> &'static str {
        match self {
            Self::Violations => "Violations",
            Self::NeedsReview => "Needs review",
            Self::Advisories => "Advisories",
            Self::Waived => "Waived",
            Self::Passed => "Passed",
            Self::NotApplicable => "Not applicable",
        }
    }

    fn default_open(self) -> bool {
        !matches!(self, Self::Passed | Self::NotApplicable)
    }

    fn contains(self, outcome: &Outcome) -> bool {
        self == compliance_outcome_group(outcome)
    }
}

fn compliance_outcome_group(outcome: &Outcome) -> ComplianceGroup {
    match outcome {
        Outcome::Violation => ComplianceGroup::Violations,
        Outcome::NeedsReview => ComplianceGroup::NeedsReview,
        Outcome::Advisory => ComplianceGroup::Advisories,
        Outcome::Waived { .. } => ComplianceGroup::Waived,
        Outcome::Pass => ComplianceGroup::Passed,
        Outcome::NotApplicable => ComplianceGroup::NotApplicable,
    }
}

const COMPLIANCE_GROUPS: &[ComplianceGroup] = &[
    ComplianceGroup::Violations,
    ComplianceGroup::NeedsReview,
    ComplianceGroup::Advisories,
    ComplianceGroup::Waived,
    ComplianceGroup::Passed,
    ComplianceGroup::NotApplicable,
];

fn compliance_panel(ui: &mut Ui, report: Option<&ComplianceReport>) -> Option<ElementId> {
    panel_subheader(ui, "Compliance");
    let Some(report) = report else {
        ui.label("No compliance report");
        return None;
    };
    if report.entries.is_empty() {
        ui.label("No compliance entries");
        return None;
    }

    let mut focused = None;
    for group in COMPLIANCE_GROUPS {
        let entries = report
            .entries
            .iter()
            .filter(|entry| group.contains(&entry.outcome))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            continue;
        }

        egui::CollapsingHeader::new(format!("{} ({})", group.label(), entries.len()))
            .default_open(group.default_open())
            .show(ui, |ui| {
                for entry in entries {
                    if let Some(source) = compliance_entry_row(ui, entry) {
                        focused = Some(source);
                    }
                }
            });
    }
    focused
}

fn compliance_entry_row(ui: &mut Ui, entry: &ComplianceEntry) -> Option<ElementId> {
    let mut focused = None;
    let row = ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                compliance_outcome_color(&entry.outcome),
                compliance_outcome_label(&entry.outcome),
            );
            ui.label(
                RichText::new(&entry.rule)
                    .strong()
                    .color(if entry.element.is_some() {
                        theme::active_blue()
                    } else {
                        theme::text_primary()
                    }),
            );
            ui.label(
                RichText::new(&entry.citation)
                    .size(design::text_size::LABEL)
                    .color(theme::text_secondary()),
            );
            if let Some(source) = &entry.element {
                ui.small(source.0.as_str());
            }
        });
        ui.label(
            RichText::new(&entry.message)
                .size(design::text_size::BODY)
                .color(theme::text_primary()),
        );
    });
    if let Some(source) = &entry.element {
        let response = ui
            .interact(
                row.response.rect,
                ui.id()
                    .with(format!("compliance-entry-{}-{}", entry.rule, source.0)),
                egui::Sense::click(),
            )
            .on_hover_text("Select source element");
        if response.clicked() {
            focused = Some(source.clone());
        }
    }
    ui.add_space(4.0);
    focused
}

fn compliance_outcome_label(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Pass => "Pass",
        Outcome::Violation => "Violation",
        Outcome::Advisory => "Advisory",
        Outcome::NeedsReview => "Needs review",
        Outcome::NotApplicable => "Not applicable",
        Outcome::Waived { .. } => "Waived",
    }
}

fn compliance_outcome_color(outcome: &Outcome) -> Color32 {
    match outcome {
        Outcome::Pass => theme::success(),
        Outcome::Violation => theme::danger(),
        Outcome::Advisory | Outcome::NeedsReview => theme::warning(),
        Outcome::NotApplicable | Outcome::Waived { .. } => theme::text_muted(),
    }
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
                strong_label(ui, "Qty");
                strong_label(ui, "Profile");
                strong_label(ui, "Cut");
                strong_label(ui, "Total");
                strong_label(ui, "Use");
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
                    strong_label(ui, "Material");
                    strong_label(ui, "Function");
                    strong_label(ui, "Thickness");
                    strong_label(ui, "Area");
                    strong_label(ui, "Volume");
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
                strong_label(ui, "Room");
                strong_label(ui, "Usage");
                strong_label(ui, "Area");
                strong_label(ui, "Perimeter");
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
    display_unit: DisplayUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayUnit {
    Feet,
    Inches,
}

impl DisplayUnit {
    fn value(self, length: Length) -> f64 {
        match self {
            Self::Feet => length.feet(),
            Self::Inches => length.inches(),
        }
    }

    fn length(self, value: f64) -> Length {
        match self {
            Self::Feet => Length::from_feet(value),
            Self::Inches => Length::from_inches(value),
        }
    }

    fn range(self, min_inches: f64, max_inches: f64) -> std::ops::RangeInclusive<f64> {
        match self {
            Self::Feet => min_inches / 12.0..=max_inches / 12.0,
            Self::Inches => min_inches..=max_inches,
        }
    }

    fn speed(self) -> f64 {
        match self {
            Self::Feet => 0.25,
            Self::Inches => 1.0,
        }
    }
}

fn length_drag_spec(min_inches: f64, max_inches: f64, display_unit: DisplayUnit) -> LengthDragSpec {
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
    display_unit: DisplayUnit,
    driver: &DrivenField,
    select_dimension: &mut Option<String>,
) {
    let mut display_value = display_unit.value(value);
    let hover_text = driver.hover_text();

    property_row(ui, label, |ui| {
        let value_response = ui.add_enabled(
            false,
            length_drag_widget(&mut display_value, display_unit, f64::MIN, f64::MAX),
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
    display_unit: DisplayUnit,
) -> bool {
    let mut display_value = display_unit.value(*value);

    let response = property_row(ui, label, |ui| {
        editable_drag_value(
            ui,
            length_drag_widget(&mut display_value, display_unit, min_inches, max_inches),
        )
    });

    if response.changed() {
        let next_inches = display_unit.length(display_value).inches();
        *value = Length::from_inches(next_inches.clamp(min_inches, max_inches));
        true
    } else {
        false
    }
}

fn coordinate_drag(ui: &mut Ui, label: &str, value: &mut Length) -> bool {
    let mut display_value = DisplayUnit::Feet.value(*value);
    let response = property_row(ui, label, |ui| {
        editable_drag_value(
            ui,
            length_drag_widget(
                &mut display_value,
                DisplayUnit::Feet,
                Length::from_feet(-240.0).inches(),
                Length::from_feet(240.0).inches(),
            ),
        )
    });

    if response.changed() {
        let next_inches = DisplayUnit::Feet.length(display_value).inches();
        *value = Length::from_inches(next_inches.clamp(
            Length::from_feet(-240.0).inches(),
            Length::from_feet(240.0).inches(),
        ));
        true
    } else {
        false
    }
}

fn editable_drag_value(ui: &mut Ui, widget: egui::DragValue<'_>) -> Response {
    let t = design::active();
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.bg_fill = t.field;
        widgets.inactive.weak_bg_fill = t.field;
        widgets.inactive.bg_stroke = t.border_stroke();
        widgets.hovered.bg_fill = t.control_hover;
        widgets.hovered.weak_bg_fill = t.control_hover;
        widgets.hovered.bg_stroke = t.accent_stroke();
        ui.add(widget)
    })
    .inner
    .on_hover_cursor(egui::CursorIcon::Text)
}

fn length_drag_widget<'a>(
    display_value: &'a mut f64,
    display_unit: DisplayUnit,
    min_inches: f64,
    max_inches: f64,
) -> egui::DragValue<'a> {
    egui::DragValue::new(display_value)
        .range(display_unit.range(min_inches, max_inches))
        .speed(display_unit.speed())
        .custom_formatter(move |value, _| format_length_display_value(value, display_unit))
        .custom_parser(move |text| parse_length_display_value(text, display_unit))
}

fn format_length_display_value(value: f64, display_unit: DisplayUnit) -> String {
    display_unit.length(value).to_string()
}

fn parse_length_display_value(text: &str, display_unit: DisplayUnit) -> Option<f64> {
    let length = parse_length_expression(text).or_else(|| {
        text.trim()
            .parse::<f64>()
            .ok()
            .map(|value| display_unit.length(value))
    })?;
    Some(display_unit.value(length))
}

fn parse_length_expression(text: &str) -> Option<Length> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    if let Some(value) = lower
        .strip_suffix("feet")
        .or_else(|| lower.strip_suffix("foot"))
        .or_else(|| lower.strip_suffix("ft"))
        .and_then(|value| value.trim().parse::<f64>().ok())
    {
        return Some(Length::from_feet(value));
    }
    if let Some(value) = lower
        .strip_suffix("inches")
        .or_else(|| lower.strip_suffix("inch"))
        .or_else(|| lower.strip_suffix("in"))
        .and_then(|value| value.trim().parse::<f64>().ok())
    {
        return Some(Length::from_inches(value));
    }

    if !(trimmed.contains('\'') || trimmed.contains('"')) {
        return None;
    }

    let (sign, unsigned) = match trimmed.strip_prefix('-') {
        Some(rest) => (-1.0, rest.trim()),
        None => (1.0, trimmed),
    };
    let (feet, inches_text) = match unsigned.split_once('\'') {
        Some((feet, rest)) => (feet.trim().parse::<f64>().ok()?, rest),
        None => (0.0, unsigned),
    };
    let inches_text = inches_text.trim().trim_end_matches('"').trim();
    let inches = if inches_text.is_empty() {
        0.0
    } else {
        parse_inches_with_optional_fraction(inches_text)?
    };

    Some(Length::from_inches(sign * (feet * 12.0 + inches)))
}

fn parse_inches_with_optional_fraction(text: &str) -> Option<f64> {
    let parts = text.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        [whole] => parse_number_or_fraction(whole),
        [whole, fraction] => Some(whole.parse::<f64>().ok()? + parse_fraction(fraction)?),
        _ => None,
    }
}

fn parse_number_or_fraction(text: &str) -> Option<f64> {
    if text.contains('/') {
        parse_fraction(text)
    } else {
        text.parse::<f64>().ok()
    }
}

fn parse_fraction(text: &str) -> Option<f64> {
    let (numerator, denominator) = text.split_once('/')?;
    let numerator = numerator.trim().parse::<f64>().ok()?;
    let denominator = denominator.trim().parse::<f64>().ok()?;
    if denominator == 0.0 {
        None
    } else {
        Some(numerator / denominator)
    }
}

fn sync_connected_roof_overhangs(
    model: &mut BuildingModel,
    roof_id: &ElementId,
    eave: Length,
    rake: Length,
) {
    let connected = model.connected_roof_plane_ids(roof_id);
    for plane in &mut model.roof_planes {
        if connected.contains(&plane.id) {
            plane.eave_overhang = eave;
            plane.rake_overhang = rake;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use framer_core::{
        DimensionAxis, DimensionDirection, DimensionHorizontalReference,
        DimensionVerticalReference, FramingDefaults, RoofPlane,
    };
    use framer_geometry::{AssemblyKind, BodyRef, GeometryBuildDiagnostic, GeometryQueryViolation};

    #[test]
    fn geometry_violations_contribute_to_status_error_count() {
        let counts = diagnostic_counts(None, &[], 3);

        assert_eq!(counts.errors, 3);
        assert_eq!(
            diagnostics_status_label(counts),
            "3 errors   0 warnings   0 unsupported   0 info"
        );
    }

    #[test]
    fn diagnostic_row_budget_covers_mixed_and_overflowing_geometry_rows() {
        assert_eq!(diagnostic_row_budget(2, 6, 5), (2, 3, 3));
        assert_eq!(diagnostic_row_budget(7, 2, 5), (5, 0, 4));
        assert_eq!(diagnostic_row_budget(0, 0, 5), (0, 0, 0));
    }

    #[test]
    fn geometry_diagnostic_text_retains_pair_depth_and_witness() {
        let mut app = FramerApp::default();
        let mut wall = app.model.walls[0].clone();
        wall.id = ElementId::new("diagnostic-overlap-wall");
        wall.name = "Diagnostic overlap wall".to_owned();
        wall.start.x += Length::from_whole_inches(12);
        wall.end.x += Length::from_whole_inches(12);
        wall.openings.clear();
        wall.dimensions.clear();
        app.model.walls.push(wall);
        app.rebuild();

        let overlap = app
            .geometry_audit
            .violations
            .iter()
            .find(|violation| matches!(violation, GeometryViolation::Overlap(_)))
            .expect("fixture should produce an overlap");
        let body_b = overlap.body_b().expect("overlap retains both bodies");
        let message = geometry_violation_message(overlap);
        let action_label = geometry_diagnostic_row_action_label(overlap).unwrap();

        assert_eq!(overlap.code(), "geometry.overlap");
        assert!(message.contains("Penetration"));
        assert!(message.contains("witness"));
        assert!(action_label.contains(&overlap.body_a().to_string()));
        assert!(action_label.contains(&body_b.to_string()));
    }

    #[test]
    fn unbuildable_geometry_diagnostic_is_violation_styled_without_focus_action() {
        let body = BodyRef::assembly(ElementId::new("bad-wall"), AssemblyKind::Wall);
        let violation = GeometryViolation::BodyUnbuildable(GeometryBuildDiagnostic::unbuildable(
            body.clone(),
            "outline did not triangulate",
        ));

        assert_eq!(violation.code(), "geometry.body.unbuildable");
        assert_eq!(
            geometry_violation_message(&violation),
            "outline did not triangulate"
        );
        assert_eq!(geometry_diagnostic_row_action_label(&violation), None);
        assert!(geometry_body_label(&body).contains("Wall assembly"));
    }

    #[test]
    fn unsupported_query_diagnostic_retains_both_bodies_and_message() {
        let body_a = BodyRef::assembly(ElementId::new("wall-a"), AssemblyKind::Wall);
        let body_b = BodyRef::assembly(ElementId::new("wall-b"), AssemblyKind::Wall);
        let violation = GeometryViolation::QueryUnsupported(GeometryQueryViolation {
            body_a: body_a.clone(),
            body_b: body_b.clone(),
            message: "shape pair is not supported".to_owned(),
        });

        assert_eq!(violation.code(), "geometry.query.unsupported");
        assert_eq!(
            geometry_violation_message(&violation),
            "shape pair is not supported"
        );
        let label = geometry_diagnostic_row_action_label(&violation).unwrap();
        assert!(label.contains(&body_a.to_string()));
        assert!(label.contains(&body_b.to_string()));
    }

    #[test]
    fn reconnecting_a_roof_plane_reconciles_the_component_overhangs() {
        let mut model = BuildingModel::new();
        let p = |x, y| Point2::new(Length::from_feet(x), Length::from_feet(y));
        let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
        model.roof_planes = vec![
            RoofPlane::new(
                "roof-south",
                "South",
                "level-1",
                "system-roof-1",
                vec![p(0.0, 0.0), p(12.0, 0.0), p(12.0, 4.0), p(0.0, 4.0)],
                slope,
                0,
                Length::from_feet(8.0),
            )
            .with_eave_overhang(Length::from_whole_inches(12))
            .with_rake_overhang(Length::from_whole_inches(8)),
            RoofPlane::new(
                "roof-north",
                "North",
                "detached-level",
                "system-roof-1",
                vec![p(0.0, 4.0), p(12.0, 4.0), p(12.0, 8.0), p(0.0, 8.0)],
                slope,
                2,
                Length::from_feet(8.0),
            )
            .with_eave_overhang(Length::from_whole_inches(6))
            .with_rake_overhang(Length::from_whole_inches(4)),
        ];

        let moved = model.roof_planes[1].id.clone();
        model.roof_planes[1].level = ElementId::new("level-1");
        sync_connected_roof_overhangs(
            &mut model,
            &moved,
            Length::from_whole_inches(6),
            Length::from_whole_inches(4),
        );

        assert!(model.roof_planes.iter().all(|plane| {
            plane.eave_overhang == Length::from_whole_inches(6)
                && plane.rake_overhang == Length::from_whole_inches(4)
        }));
        model.validate().unwrap();
    }

    #[test]
    fn inspector_length_fields_format_with_canonical_length_display() {
        assert_eq!(
            format_length_display_value(28.0, DisplayUnit::Feet),
            "28' 0\""
        );
        assert_eq!(
            format_length_display_value(48.0, DisplayUnit::Inches),
            "4' 0\""
        );
        assert_eq!(
            format_length_display_value(8.1875, DisplayUnit::Inches),
            "0' 8 3/16\""
        );
    }

    #[test]
    fn inspector_length_fields_accept_plain_native_unit_entry() {
        assert_eq!(
            parse_length_display_value("4", DisplayUnit::Feet),
            Some(4.0)
        );
        assert_eq!(
            parse_length_display_value("48", DisplayUnit::Inches),
            Some(48.0)
        );
    }

    #[test]
    fn inspector_length_fields_accept_canonical_length_entry() {
        assert_eq!(
            parse_length_display_value("4' 0\"", DisplayUnit::Feet),
            Some(4.0)
        );
        assert_eq!(
            parse_length_display_value("4' 0\"", DisplayUnit::Inches),
            Some(48.0)
        );
        assert_eq!(
            parse_length_display_value("0' 8 3/16\"", DisplayUnit::Inches),
            Some(8.1875)
        );
        assert_eq!(
            parse_length_display_value("-4' 6\"", DisplayUnit::Feet),
            Some(-4.5)
        );
        assert_eq!(
            parse_length_display_value("6 feet", DisplayUnit::Feet),
            Some(6.0)
        );
        assert_eq!(
            parse_length_display_value("48 in", DisplayUnit::Inches),
            Some(48.0)
        );
        assert_eq!(
            parse_length_display_value("4' 6 3/16\"", DisplayUnit::Inches),
            Some(54.1875)
        );
    }

    #[test]
    fn command_search_labels_use_categories_and_shortcuts() {
        assert_eq!(
            command_search_label(*actions::metadata(ActionId::NewProject)),
            "New — Project"
        );
        assert_eq!(
            command_search_label(*actions::metadata(ActionId::Undo)),
            "Undo — Edit ⌘Z"
        );
        assert_eq!(
            command_search_label(*actions::metadata(ActionId::ToolWall)),
            "Wall — Structure W"
        );

        for action in actions::ACTIONS {
            let label = command_search_label(*action);
            assert!(
                !label.contains("App header") && !label.contains("Workflow strip"),
                "command search label should not expose internal routes: {label}"
            );
        }
    }

    #[test]
    fn command_search_matching_ignores_internal_surface_names() {
        assert!(!command_search_matches(
            *actions::metadata(ActionId::NewProject),
            "app header"
        ));
        assert!(command_search_matches(
            *actions::metadata(ActionId::NewProject),
            "project"
        ));
    }

    #[test]
    fn action_tooltips_append_shortcuts_or_disabled_reason() {
        assert_eq!(
            action_tooltip(*actions::metadata(ActionId::ToolWall), None),
            "Draw walls in the plan view (W)"
        );
        assert_eq!(
            action_tooltip(
                *actions::metadata(ActionId::ExportArtifacts),
                Some("Available in the Plan workspace"),
            ),
            "Available in the Plan workspace"
        );
    }

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

    fn library_status(
        issues: Vec<framer_library::LibraryIssueKind>,
        source_available: bool,
    ) -> LibrarySelectionStatus {
        LibrarySelectionStatus {
            item: framer_library::LibraryItem::Material(ElementId::new("local-material")),
            source: Provenance {
                library_uid: "11111111-1111-4111-8111-111111111111".to_owned(),
                version_id: "2026.06".to_owned(),
                source_id: ElementId::new("source-material"),
                content_hash: "blake3:abc123".to_owned(),
            },
            issues,
            source_available,
        }
    }

    #[test]
    fn library_status_label_covers_states_and_precedence() {
        let cases = vec![
            (
                vec![framer_library::LibraryIssueKind::SourceMissing],
                true,
                "Source missing",
            ),
            (
                vec![
                    framer_library::LibraryIssueKind::Diverged,
                    framer_library::LibraryIssueKind::OutOfDate,
                    framer_library::LibraryIssueKind::SourceMissing,
                ],
                false,
                "Source missing",
            ),
            (
                vec![
                    framer_library::LibraryIssueKind::Diverged,
                    framer_library::LibraryIssueKind::OutOfDate,
                ],
                true,
                "Modified, update available",
            ),
            (
                vec![framer_library::LibraryIssueKind::Diverged],
                true,
                "Modified",
            ),
            (
                vec![framer_library::LibraryIssueKind::OutOfDate],
                true,
                "Update available",
            ),
            (Vec::new(), false, "Source unavailable"),
            (Vec::new(), true, "Current"),
        ];

        for (issues, source_available, expected) in cases {
            assert_eq!(
                library_status_label(&library_status(issues, source_available)),
                expected
            );
        }
    }

    #[test]
    fn standards_rule_rows_match_full_resolution_metadata() {
        let mut app = FramerApp::default();
        app.waive_standards_rule(
            "irc2021.r602.3-5.studs".to_owned(),
            "accepted by AHJ".to_owned(),
        );

        let expected = app
            .model
            .resolved_standards()
            .rules
            .into_iter()
            .map(|rule| {
                (
                    rule.rule,
                    rule.citation,
                    rule.pack.0,
                    rule.waived,
                    rule.severity.is_some(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(standards_rule_rows(&app.model), expected);
    }

    #[test]
    fn compliance_groups_cover_every_outcome_once() {
        let outcomes = [
            Outcome::Violation,
            Outcome::NeedsReview,
            Outcome::Advisory,
            Outcome::Waived {
                reason: "accepted by AHJ".to_owned(),
            },
            Outcome::Pass,
            Outcome::NotApplicable,
        ];

        for outcome in outcomes {
            let matches = COMPLIANCE_GROUPS
                .iter()
                .filter(|group| group.contains(&outcome))
                .count();
            assert_eq!(matches, 1, "{outcome:?} should map to exactly one group");
        }
        assert!(!ComplianceGroup::Passed.default_open());
        assert!(!ComplianceGroup::NotApplicable.default_open());
    }

    #[test]
    fn inserting_starter_system_vendors_provenance_and_selects_it() {
        let mut app = FramerApp::default();
        let system_count = app.model.systems.len();
        let material_count = app.model.materials.len();

        app.insert_starter_system("system-wall-exterior-1".to_owned());

        assert_eq!(app.model.libraries.len(), 1);
        assert!(app.model.systems.len() > system_count);
        assert!(app.model.materials.len() > material_count);
        let Selection::System(system_id) = &app.selected else {
            panic!("imported system should be selected");
        };
        let system = app
            .model
            .systems
            .iter()
            .find(|system| system.id.0 == *system_id)
            .expect("selected imported system should exist");
        let source = system
            .source
            .as_ref()
            .expect("system should have provenance");
        assert_eq!(source.source_id, ElementId::new("system-wall-exterior-1"));
        assert!(source.content_hash.starts_with("blake3:"));
        assert!(
            app.model
                .materials
                .iter()
                .any(|material| matches!(&material.source, MaterialSource::Library(_)))
        );
        app.model.validate().unwrap();
    }

    #[test]
    fn inserting_starter_material_vendors_provenance_and_selects_it() {
        let mut app = FramerApp::default();
        let material_count = app.model.materials.len();

        app.insert_starter_material("mat-fiber-cement".to_owned());

        assert_eq!(app.model.libraries.len(), 1);
        assert!(app.model.materials.len() > material_count);
        let Selection::Material(material_id) = &app.selected else {
            panic!("imported material should be selected");
        };
        let material = app
            .model
            .materials
            .iter()
            .find(|material| material.id.0 == *material_id)
            .expect("selected imported material should exist");
        let MaterialSource::Library(source) = &material.source else {
            panic!("material should have provenance");
        };
        assert_eq!(source.source_id, ElementId::new("mat-fiber-cement"));
        assert!(source.content_hash.starts_with("blake3:"));
        app.model.validate().unwrap();
    }

    #[test]
    fn placing_starter_objects_vendors_families_and_instances() {
        let mut app = FramerApp::default();
        let placement = Point2::new(Length::from_inches(24.0), Length::from_inches(36.0));
        let active_level = Level::new("level-2", "Level 2", Length::from_feet(10.0));
        let active_level_id = active_level.id.clone();
        app.model.levels.push(active_level);
        app.set_active_level(active_level_id.clone());
        app.cursor_model = Some(placement);

        app.place_starter_furnishing("furnishing-workbench".to_owned());

        assert_eq!(app.model.furnishings.len(), 1);
        assert_eq!(app.model.furnishing_instances.len(), 1);
        assert_eq!(app.model.libraries.len(), 1);
        assert_eq!(
            app.model.furnishings[0].source.as_ref().unwrap().source_id,
            ElementId::new("furnishing-workbench")
        );
        let furnishing = &app.model.furnishing_instances[0];
        assert_eq!(furnishing.family, app.model.furnishings[0].id);
        assert_eq!(furnishing.position, placement);
        assert_eq!(furnishing.level, active_level_id);
        assert_eq!(
            app.selected,
            Selection::FurnishingInstance(furnishing.id.0.clone())
        );
        assert_eq!(app.viewport_mode, ViewportMode::Plan);
        assert!(
            app.file_status
                .as_deref()
                .is_some_and(|status| status.starts_with("Placed furnishing "))
        );

        app.cursor_model = None;
        app.place_starter_mep_object("mep-load-center".to_owned());

        assert_eq!(app.model.mep_objects.len(), 1);
        assert_eq!(app.model.mep_instances.len(), 1);
        assert_eq!(app.model.libraries.len(), 1);
        assert_eq!(
            app.model.mep_objects[0].source.as_ref().unwrap().source_id,
            ElementId::new("mep-load-center")
        );
        let object = &app.model.mep_instances[0];
        assert_eq!(object.family, app.model.mep_objects[0].id);
        assert_eq!(object.position, Point2::new(Length::ZERO, Length::ZERO));
        assert_eq!(object.level, active_level_id);
        assert_eq!(app.selected, Selection::MepInstance(object.id.0.clone()));
        assert!(
            app.file_status
                .as_deref()
                .is_some_and(|status| status.starts_with("Placed MEP object "))
        );
        app.model.validate().unwrap();
    }

    #[test]
    fn repeated_starter_object_placement_reuses_imported_family() {
        let mut app = FramerApp::default();

        app.place_starter_furnishing("furnishing-workbench".to_owned());
        app.place_starter_furnishing("furnishing-workbench".to_owned());

        assert_eq!(app.model.furnishings.len(), 1);
        assert_eq!(app.model.furnishing_instances.len(), 2);
        assert_eq!(app.model.libraries.len(), 1);
        let family = app.model.furnishings[0].id.clone();
        assert!(
            app.model
                .furnishing_instances
                .iter()
                .all(|instance| instance.family == family)
        );
        app.model.validate().unwrap();
    }

    #[test]
    fn plan_mode_does_not_place_starter_objects() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Plan);

        app.place_starter_furnishing("furnishing-workbench".to_owned());
        app.place_starter_mep_object("mep-load-center".to_owned());

        assert!(app.model.furnishings.is_empty());
        assert!(app.model.furnishing_instances.is_empty());
        assert!(app.model.mep_objects.is_empty());
        assert!(app.model.mep_instances.is_empty());
        assert!(app.model.libraries.is_empty());
    }

    #[test]
    fn editing_imported_material_surfaces_divergence_diagnostic() {
        let mut app = FramerApp::default();
        app.insert_starter_material("mat-fiber-cement".to_owned());
        let Selection::Material(material_id) = app.selected.clone() else {
            panic!("imported material should be selected");
        };
        app.model
            .materials
            .iter_mut()
            .find(|material| material.id.0 == material_id)
            .unwrap()
            .name = "Local fiber cement".to_owned();

        app.rebuild();

        let diagnostics = &app.project_plan.as_ref().unwrap().diagnostics;
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "library.item.diverged"
                && diagnostic.source == Some(ElementId::new(material_id.clone()))
        }));
    }

    #[test]
    fn detaching_imported_material_clears_provenance_and_diagnostics() {
        let mut app = FramerApp::default();
        app.insert_starter_material("mat-fiber-cement".to_owned());
        let Selection::Material(material_id) = app.selected.clone() else {
            panic!("imported material should be selected");
        };
        app.model
            .materials
            .iter_mut()
            .find(|material| material.id.0 == material_id)
            .unwrap()
            .name = "Local fiber cement".to_owned();

        app.detach_library_item(framer_library::LibraryItem::Material(ElementId::new(
            material_id.clone(),
        )));

        let material = app
            .model
            .materials
            .iter()
            .find(|material| material.id.0 == material_id)
            .unwrap();
        assert!(matches!(material.source, MaterialSource::Project));
        assert!(app.model.libraries.is_empty());
        let diagnostics = &app.project_plan.as_ref().unwrap().diagnostics;
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "library.item.diverged")
        );
    }

    #[test]
    fn resyncing_imported_material_restores_source_content() {
        let mut app = FramerApp::default();
        app.insert_starter_material("mat-fiber-cement".to_owned());
        let Selection::Material(material_id) = app.selected.clone() else {
            panic!("imported material should be selected");
        };
        app.model
            .materials
            .iter_mut()
            .find(|material| material.id.0 == material_id)
            .unwrap()
            .name = "Local fiber cement".to_owned();

        app.resync_library_item(framer_library::LibraryItem::Material(ElementId::new(
            material_id.clone(),
        )));

        let starter = framer_library::starter_library().unwrap();
        let source_name = starter
            .library
            .materials
            .iter()
            .find(|material| material.id == ElementId::new("mat-fiber-cement"))
            .unwrap()
            .name
            .clone();
        let material = app
            .model
            .materials
            .iter()
            .find(|material| material.id.0 == material_id)
            .unwrap();
        assert_eq!(material.name, source_name);
        assert!(matches!(&material.source, MaterialSource::Library(_)));
        let diagnostics = &app.project_plan.as_ref().unwrap().diagnostics;
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "library.item.diverged")
        );
    }

    #[derive(Debug, Clone, Copy)]
    enum WindowAnchor {
        Left,
        Center,
        Right,
    }

    fn wall_with_window(center: Length, width: Length) -> Wall {
        let code = FramingDefaults::irc_2021_starter();
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
