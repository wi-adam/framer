//! UI-only command metadata for Framer's command surfaces.
//!
//! This is intentionally not a command bus. It records stable ids, labels,
//! tooltips, routing homes, and compact-strip placement so command surfaces can
//! be tested before the toolbar migration moves behavior around.
#![allow(dead_code)]

use super::design::Icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ActionId {
    NewProject,
    OpenProject,
    SaveProject,
    ExportArtifacts,
    Undo,
    Redo,
    LoadShellDemo,
    LoadWallDemo,
    WorkspaceDesign,
    WorkspacePlan,
    ViewPlan,
    ViewElevation,
    ViewRoof,
    View3d,
    ViewRender,
    ToolWall,
    ToolRoom,
    ToolCeiling,
    ToolVault,
    ToolFloor,
    DeleteSelection,
    AddDoor,
    AddWindow,
    AddGarageDoor,
    AddGableRoof,
    AddShedRoof,
    AddHipRoof,
    ToolDimensionLinear,
    DimensionKind,
    DimensionAxis,
    ToggleSection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionOwner {
    Project,
    Edit,
    Samples,
    Workspace,
    View,
    Structure,
    Openings,
    Roofs,
    Dimensions,
    Plan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandSurface {
    AppQuickAccess,
    ProjectMenu,
    ExamplesPicker,
    WorkspaceViewBar,
    WorkflowCommandStrip,
    CommandStripFlyout,
    ContextToolbar,
    ToolOptionsStrip,
    Inspector,
    PlanWorkspace,
    CommandSearch,
    Shortcut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum WorkflowTab {
    Design,
    Frame,
    Openings,
    Roofs,
    Annotate,
    Inspect,
    Plan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandPanel {
    Structure,
    Openings,
    Roofs,
    Dimensions,
    GeneratedPlan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandPresentation {
    TopLevel,
    FlyoutVariant { flyout: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CommandStripRoute {
    pub(crate) tab: WorkflowTab,
    pub(crate) panel: CommandPanel,
    pub(crate) presentation: CommandPresentation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActionMetadata {
    pub(crate) id: ActionId,
    pub(crate) label: &'static str,
    pub(crate) icon: Icon,
    pub(crate) tooltip: &'static str,
    pub(crate) owner: ActionOwner,
    pub(crate) primary_surface: CommandSurface,
    pub(crate) secondary_surfaces: &'static [CommandSurface],
    pub(crate) command_strip: Option<CommandStripRoute>,
    pub(crate) mutates_authored_intent: bool,
}

const SEARCH: &[CommandSurface] = &[CommandSurface::CommandSearch];
const SEARCH_SHORTCUT: &[CommandSurface] =
    &[CommandSurface::CommandSearch, CommandSurface::Shortcut];
const SEARCH_PROJECT: &[CommandSurface] =
    &[CommandSurface::CommandSearch, CommandSurface::ProjectMenu];
const SEARCH_CONTEXT: &[CommandSurface] = &[
    CommandSurface::CommandSearch,
    CommandSurface::ContextToolbar,
];
const SEARCH_INSPECTOR: &[CommandSurface] =
    &[CommandSurface::CommandSearch, CommandSurface::Inspector];

const fn strip_route(
    tab: WorkflowTab,
    panel: CommandPanel,
    presentation: CommandPresentation,
) -> CommandStripRoute {
    CommandStripRoute {
        tab,
        panel,
        presentation,
    }
}

pub(crate) const ACTIONS: &[ActionMetadata] = &[
    ActionMetadata {
        id: ActionId::NewProject,
        label: "New",
        icon: Icon::New,
        tooltip: "Start an empty wall project",
        owner: ActionOwner::Project,
        primary_surface: CommandSurface::AppQuickAccess,
        secondary_surfaces: SEARCH_PROJECT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::OpenProject,
        label: "Open",
        icon: Icon::Open,
        tooltip: "Open the project path",
        owner: ActionOwner::Project,
        primary_surface: CommandSurface::AppQuickAccess,
        secondary_surfaces: SEARCH_PROJECT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::SaveProject,
        label: "Save",
        icon: Icon::Save,
        tooltip: "Save the current model",
        owner: ActionOwner::Project,
        primary_surface: CommandSurface::AppQuickAccess,
        secondary_surfaces: SEARCH_PROJECT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::ExportArtifacts,
        label: "Export",
        icon: Icon::Export,
        tooltip: "Export plan artifacts from Plan workspace",
        owner: ActionOwner::Project,
        primary_surface: CommandSurface::PlanWorkspace,
        secondary_surfaces: SEARCH_PROJECT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::Undo,
        label: "Undo",
        icon: Icon::Undo,
        tooltip: "Undo the last model edit",
        owner: ActionOwner::Edit,
        primary_surface: CommandSurface::AppQuickAccess,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: None,
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::Redo,
        label: "Redo",
        icon: Icon::Redo,
        tooltip: "Redo the last undone model edit",
        owner: ActionOwner::Edit,
        primary_surface: CommandSurface::AppQuickAccess,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: None,
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::LoadShellDemo,
        label: "Shell",
        icon: Icon::Shell,
        tooltip: "Load the multi-wall shell demo",
        owner: ActionOwner::Samples,
        primary_surface: CommandSurface::ExamplesPicker,
        secondary_surfaces: SEARCH_PROJECT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::LoadWallDemo,
        label: "Wall",
        icon: Icon::Wall,
        tooltip: "Load the single-wall demo",
        owner: ActionOwner::Samples,
        primary_surface: CommandSurface::ExamplesPicker,
        secondary_surfaces: SEARCH_PROJECT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::WorkspaceDesign,
        label: "Design",
        icon: Icon::Design,
        tooltip: "Switch to the design workspace",
        owner: ActionOwner::Workspace,
        primary_surface: CommandSurface::WorkspaceViewBar,
        secondary_surfaces: SEARCH,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::WorkspacePlan,
        label: "Plan",
        icon: Icon::Plan,
        tooltip: "Switch to the generated plan workspace",
        owner: ActionOwner::Workspace,
        primary_surface: CommandSurface::WorkspaceViewBar,
        secondary_surfaces: SEARCH,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::ViewPlan,
        label: "Shell / Plan",
        icon: Icon::Shell,
        tooltip: "Show the whole-project plan view",
        owner: ActionOwner::View,
        primary_surface: CommandSurface::WorkspaceViewBar,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::ViewElevation,
        label: "Wall / Elevation",
        icon: Icon::Wall,
        tooltip: "Show the selected-wall elevation view",
        owner: ActionOwner::View,
        primary_surface: CommandSurface::WorkspaceViewBar,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::ViewRoof,
        label: "Roof",
        icon: Icon::Angular,
        tooltip: "Top-down roof plan: view and select roof planes",
        owner: ActionOwner::View,
        primary_surface: CommandSurface::WorkspaceViewBar,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::View3d,
        label: "3D",
        icon: Icon::View3d,
        tooltip: "Show the 3D model view",
        owner: ActionOwner::View,
        primary_surface: CommandSurface::WorkspaceViewBar,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::ViewRender,
        label: "Render",
        icon: Icon::ThemeLight,
        tooltip: "Show the path-traced render view",
        owner: ActionOwner::View,
        primary_surface: CommandSurface::WorkspaceViewBar,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::ToolWall,
        label: "Wall",
        icon: Icon::Wall,
        tooltip: "Draw walls in the plan view (W)",
        owner: ActionOwner::Structure,
        primary_surface: CommandSurface::WorkflowCommandStrip,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: Some(strip_route(
            WorkflowTab::Frame,
            CommandPanel::Structure,
            CommandPresentation::TopLevel,
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::ToolRoom,
        label: "Room",
        icon: Icon::Shell,
        tooltip: "Place a room inside an enclosed area (R)",
        owner: ActionOwner::Structure,
        primary_surface: CommandSurface::WorkflowCommandStrip,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: Some(strip_route(
            WorkflowTab::Design,
            CommandPanel::Structure,
            CommandPresentation::TopLevel,
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::ToolCeiling,
        label: "Ceiling",
        icon: Icon::PanelRight,
        tooltip: "Place a flat ceiling inside an enclosed area (C)",
        owner: ActionOwner::Structure,
        primary_surface: CommandSurface::WorkflowCommandStrip,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: Some(strip_route(
            WorkflowTab::Frame,
            CommandPanel::Structure,
            CommandPresentation::TopLevel,
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::ToolVault,
        label: "Vault",
        icon: Icon::Angular,
        tooltip: "Vault an enclosed area: two opposing sloped ceilings meeting at a ridge (V)",
        owner: ActionOwner::Structure,
        primary_surface: CommandSurface::WorkflowCommandStrip,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: Some(strip_route(
            WorkflowTab::Frame,
            CommandPanel::Structure,
            CommandPresentation::TopLevel,
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::ToolFloor,
        label: "Floor",
        icon: Icon::LayoutGrid,
        tooltip: "Place a floor deck inside an enclosed area (F)",
        owner: ActionOwner::Structure,
        primary_surface: CommandSurface::WorkflowCommandStrip,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: Some(strip_route(
            WorkflowTab::Frame,
            CommandPanel::Structure,
            CommandPresentation::TopLevel,
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::DeleteSelection,
        label: "Delete",
        icon: Icon::Delete,
        tooltip: "Delete the selected object (Del)",
        owner: ActionOwner::Edit,
        primary_surface: CommandSurface::ContextToolbar,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: None,
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::AddDoor,
        label: "Door",
        icon: Icon::Door,
        tooltip: "Add a door to the selected wall",
        owner: ActionOwner::Openings,
        primary_surface: CommandSurface::CommandStripFlyout,
        secondary_surfaces: SEARCH_INSPECTOR,
        command_strip: Some(strip_route(
            WorkflowTab::Openings,
            CommandPanel::Openings,
            CommandPresentation::FlyoutVariant { flyout: "Opening" },
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::AddWindow,
        label: "Window",
        icon: Icon::Window,
        tooltip: "Add a window to the selected wall",
        owner: ActionOwner::Openings,
        primary_surface: CommandSurface::CommandStripFlyout,
        secondary_surfaces: SEARCH_INSPECTOR,
        command_strip: Some(strip_route(
            WorkflowTab::Openings,
            CommandPanel::Openings,
            CommandPresentation::FlyoutVariant { flyout: "Opening" },
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::AddGarageDoor,
        label: "Garage",
        icon: Icon::GarageDoor,
        tooltip: "Add a garage door to the selected wall",
        owner: ActionOwner::Openings,
        primary_surface: CommandSurface::CommandStripFlyout,
        secondary_surfaces: SEARCH_INSPECTOR,
        command_strip: Some(strip_route(
            WorkflowTab::Openings,
            CommandPanel::Openings,
            CommandPresentation::FlyoutVariant { flyout: "Opening" },
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::AddGableRoof,
        label: "Gable",
        icon: Icon::Angular,
        tooltip: "Generate a gable roof over the wall footprint",
        owner: ActionOwner::Roofs,
        primary_surface: CommandSurface::CommandStripFlyout,
        secondary_surfaces: SEARCH_CONTEXT,
        command_strip: Some(strip_route(
            WorkflowTab::Roofs,
            CommandPanel::Roofs,
            CommandPresentation::FlyoutVariant { flyout: "Roof" },
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::AddShedRoof,
        label: "Shed",
        icon: Icon::Angular,
        tooltip: "Generate a shed (mono-pitch) roof over the footprint",
        owner: ActionOwner::Roofs,
        primary_surface: CommandSurface::CommandStripFlyout,
        secondary_surfaces: SEARCH_CONTEXT,
        command_strip: Some(strip_route(
            WorkflowTab::Roofs,
            CommandPanel::Roofs,
            CommandPresentation::FlyoutVariant { flyout: "Roof" },
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::AddHipRoof,
        label: "Hip",
        icon: Icon::Angular,
        tooltip: "Generate a hip roof over a rectangular footprint, or valley planes over a simple L footprint",
        owner: ActionOwner::Roofs,
        primary_surface: CommandSurface::CommandStripFlyout,
        secondary_surfaces: SEARCH_CONTEXT,
        command_strip: Some(strip_route(
            WorkflowTab::Roofs,
            CommandPanel::Roofs,
            CommandPresentation::FlyoutVariant { flyout: "Roof" },
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::ToolDimensionLinear,
        label: "Linear",
        icon: Icon::Linear,
        tooltip: "Place a wall dimension (D)",
        owner: ActionOwner::Dimensions,
        primary_surface: CommandSurface::WorkflowCommandStrip,
        secondary_surfaces: SEARCH_SHORTCUT,
        command_strip: Some(strip_route(
            WorkflowTab::Annotate,
            CommandPanel::Dimensions,
            CommandPresentation::TopLevel,
        )),
        mutates_authored_intent: true,
    },
    ActionMetadata {
        id: ActionId::DimensionKind,
        label: "Dimension Kind",
        icon: Icon::Options,
        tooltip: "Choose whether placed dimensions drive geometry or annotate it",
        owner: ActionOwner::Dimensions,
        primary_surface: CommandSurface::ToolOptionsStrip,
        secondary_surfaces: SEARCH,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::DimensionAxis,
        label: "Dimension Axis",
        icon: Icon::Options,
        tooltip: "Choose horizontal or vertical dimension placement",
        owner: ActionOwner::Dimensions,
        primary_surface: CommandSurface::ToolOptionsStrip,
        secondary_surfaces: SEARCH,
        command_strip: None,
        mutates_authored_intent: false,
    },
    ActionMetadata {
        id: ActionId::ToggleSection,
        label: "Section",
        icon: Icon::LayoutColumns,
        tooltip: "Toggle the wall section preview",
        owner: ActionOwner::Plan,
        primary_surface: CommandSurface::WorkflowCommandStrip,
        secondary_surfaces: SEARCH,
        command_strip: Some(strip_route(
            WorkflowTab::Plan,
            CommandPanel::GeneratedPlan,
            CommandPresentation::TopLevel,
        )),
        mutates_authored_intent: false,
    },
];

pub(crate) fn metadata(id: ActionId) -> &'static ActionMetadata {
    ACTIONS
        .iter()
        .find(|action| action.id == id)
        .expect("all referenced app actions must be registered")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    #[test]
    fn action_ids_are_unique() {
        let mut seen = BTreeSet::new();
        for action in ACTIONS {
            assert!(
                seen.insert(action.id),
                "duplicate action metadata for {:?}",
                action.id
            );
        }
    }

    #[test]
    fn actions_have_accessible_names_tooltips_and_backstops() {
        for action in ACTIONS {
            assert!(
                !action.label.trim().is_empty(),
                "{:?} must have a visible/searchable label",
                action.id
            );
            assert!(
                !action.tooltip.trim().is_empty(),
                "{:?} must have a tooltip/accessibility hint",
                action.id
            );
            assert!(
                !action.secondary_surfaces.is_empty(),
                "{:?} must have a secondary route",
                action.id
            );
            assert!(
                action
                    .secondary_surfaces
                    .contains(&CommandSurface::CommandSearch),
                "{:?} must remain reachable through command search",
                action.id
            );
        }
    }

    #[test]
    fn command_strip_actions_have_complete_routes() {
        for action in ACTIONS {
            match action.primary_surface {
                CommandSurface::WorkflowCommandStrip => {
                    let route = action
                        .command_strip
                        .expect("command-strip action must name its tab and panel");
                    assert_eq!(route.presentation, CommandPresentation::TopLevel);
                }
                CommandSurface::CommandStripFlyout => {
                    let route = action
                        .command_strip
                        .expect("flyout action must name its tab and panel");
                    let CommandPresentation::FlyoutVariant { flyout } = route.presentation else {
                        panic!("{:?} must be represented as a flyout variant", action.id);
                    };
                    assert!(!flyout.trim().is_empty());
                }
                _ => {
                    assert!(
                        action.command_strip.is_none(),
                        "{:?} should not claim a command-strip route from {:?}",
                        action.id,
                        action.primary_surface
                    );
                }
            }
        }
    }

    #[test]
    fn routing_matrix_keeps_non_modeling_actions_out_of_the_strip() {
        for id in [
            ActionId::NewProject,
            ActionId::OpenProject,
            ActionId::SaveProject,
            ActionId::ExportArtifacts,
            ActionId::Undo,
            ActionId::Redo,
            ActionId::LoadShellDemo,
            ActionId::LoadWallDemo,
            ActionId::WorkspaceDesign,
            ActionId::WorkspacePlan,
            ActionId::ViewPlan,
            ActionId::ViewElevation,
            ActionId::ViewRoof,
            ActionId::View3d,
            ActionId::ViewRender,
            ActionId::DeleteSelection,
        ] {
            assert_ne!(
                metadata(id).primary_surface,
                CommandSurface::WorkflowCommandStrip,
                "{id:?} is routed away from permanent command-strip space"
            );
        }
    }

    #[test]
    fn compact_command_strip_budget_is_explicit() {
        let mut top_level_by_tab: BTreeMap<WorkflowTab, usize> = BTreeMap::new();
        for action in ACTIONS {
            if let Some(CommandStripRoute {
                tab,
                presentation: CommandPresentation::TopLevel,
                ..
            }) = action.command_strip
            {
                *top_level_by_tab.entry(tab).or_default() += 1;
            }
        }

        for (tab, count) in top_level_by_tab {
            assert!(
                count <= 5,
                "{tab:?} exposes {count} top-level commands; use flyouts/context before widening"
            );
        }
    }
}
