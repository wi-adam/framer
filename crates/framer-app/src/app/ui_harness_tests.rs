//! Headless UI smoke tests for the real `FramerApp`, driven by `egui_kittest`.
//!
//! These boot the actual egui UI in a windowless, GPU-less harness, step it
//! through several frames, and assert against both the owned application state
//! and the AccessKit tree that egui exposes for every widget. They are the CI
//! guard that the app still *builds a UI* — every panel lays out without
//! panicking — and that core keyboard interactions still drive the model.
//!
//! The harness drives the app exactly the way `eframe` does: it runs the
//! keyboard logic, then renders the panel layout into a central, margin-less
//! [`egui::Ui`] via [`FramerApp::ui_root`]. The default viewport is the 2D Plan
//! view, which paints purely through egui (no `wgpu` device), so the whole
//! suite is deterministic and runs on every platform with no GPU adapter.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use framer_core::{DimensionAxis, DimensionKind, OpeningKind};

use super::actions::{self, ActionId};
use super::{FramerApp, Selection, ViewportMode, WallDisplay, WorkspaceMode, design};

/// A headless harness wrapping a fully-loaded `FramerApp` (the demo shell).
///
/// `build_ui_state` hands the closure a central, margin-less `Ui` — the same
/// thing `eframe` passes to [`eframe::App::ui`] — so the panel tree lays out
/// identically to the running app.
fn demo_harness<'a>() -> Harness<'a, FramerApp> {
    // `FramerApp::new` installs the design tokens + Lucide icon font on the egui
    // context before the first paint. The harness owns its own context, and
    // `set_fonts` only takes effect at the *next* frame's begin-pass — so the
    // first frame is a no-paint warm-up that just binds the fonts. egui requests
    // repaints during that initial layout/texture upload, so `run()` advances to
    // the real UI frames where `FontFamily::Name("lucide")` icons are laid out.
    let mut fonts_bound = false;
    Harness::builder()
        .with_size(egui::vec2(1360.0, 860.0))
        // Generous headroom so `run()` never trips the default 4-step cap while
        // the first-frame layout settles (the Plan view requests no animation).
        .with_max_steps(16)
        .build_ui_state(
            move |ui, app: &mut FramerApp| {
                if !fonts_bound {
                    design::install(ui.ctx(), design::studio_light());
                    fonts_bound = true;
                    return;
                }
                app.handle_keyboard_shortcuts(ui.ctx());
                app.ui_root(ui);
            },
            FramerApp::default(),
        )
}

fn assert_shortcut_from_annotate_routes_to(
    key: egui::Key,
    expected_tab: actions::WorkflowTab,
    expected_label: &str,
    assert_state: impl FnOnce(&FramerApp),
) {
    let mut harness = demo_harness();
    harness.run();

    harness.get_by_label("Annotate").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Annotate);

    harness.key_press(key);
    harness.run();
    let app = harness.state();
    assert_eq!(app.workspace_mode, WorkspaceMode::Design);
    assert_eq!(app.command_tab, expected_tab);
    assert_state(app);
    assert!(
        harness.query_all_by_label(expected_label).next().is_some(),
        "shortcut should expose the '{expected_label}' command on its owning tab"
    );
}

fn assert_accessible_label(harness: &Harness<FramerApp>, label: &str, surface: &str) {
    assert!(
        harness.query_all_by_label(label).next().is_some(),
        "{surface} should expose '{label}'"
    );
}

fn workflow_tab_test_label(tab: actions::WorkflowTab) -> &'static str {
    match tab {
        actions::WorkflowTab::Design => "Design",
        actions::WorkflowTab::Frame => "Frame",
        actions::WorkflowTab::Openings => "Openings",
        actions::WorkflowTab::Roofs => "Roofs",
        actions::WorkflowTab::Annotate => "Annotate",
        actions::WorkflowTab::Inspect => "Inspect",
        actions::WorkflowTab::Plan => "Plan",
    }
}

/// The app boots, the demo shell loads, the framing plan regenerates, and the
/// full panel tree lays out — all without panicking — and the window title is
/// present in the accessibility tree.
#[test]
fn boots_and_lays_out_demo_shell() {
    let mut harness = demo_harness();
    harness.run();

    // The default project is the multi-wall demo shell; `Default` calls
    // `rebuild()`, so a successful plan must have been generated.
    let app = harness.state();
    assert!(
        !app.model.walls.is_empty(),
        "demo shell should load with walls"
    );
    assert!(
        app.project_plan.is_some(),
        "framing plan should regenerate on load: {:?}",
        app.error
    );

    // The header renders `ui.label("Framer")`, which egui exposes to AccessKit
    // as a labelled node — proof the chrome actually built, not just the state.
    // `query_all` (not `query_by`) so a future second "Framer" node — an About
    // box, a tooltip — wouldn't turn this into a uniqueness panic.
    assert!(
        harness.query_all_by_label("Framer").next().is_some(),
        "the 'Framer' title label should be in the accessibility tree"
    );
}

/// The workflow command buttons are custom-painted, so their visible text is not
/// automatically exposed to AccessKit. The command metadata should still give
/// them accessible/searchable names.
#[test]
fn command_buttons_expose_metadata_labels() {
    let mut harness = demo_harness();
    harness.run();

    for id in [
        ActionId::NewProject,
        ActionId::SaveProject,
        ActionId::Undo,
        ActionId::Redo,
        ActionId::View3d,
        ActionId::ToolWall,
    ] {
        let action = actions::metadata(id);
        assert!(
            harness.query_all_by_label(action.label).next().is_some(),
            "{id:?} should expose '{}' as an accessible command label",
            action.label
        );
    }

    harness.get_by_label("Annotate").click();
    harness.run();
    let action = actions::metadata(ActionId::ToolDimensionLinear);
    assert!(
        harness.query_all_by_label(action.label).next().is_some(),
        "{:?} should expose '{}' when its workflow tab is active",
        action.id,
        action.label
    );
}

/// Project, edit, and sample loading commands live in the app header/menu
/// surfaces, not in the workflow command strip.
#[test]
fn header_owns_non_modeling_command_surfaces() {
    let mut harness = demo_harness();
    harness.run();

    for label in ["Project", "Examples", "New", "Open", "Save", "Undo", "Redo"] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "the app header should expose '{label}'"
        );
    }

    for strip_group in ["PROJECT", "EDIT", "SAMPLES"] {
        assert!(
            harness.query_all_by_label(strip_group).next().is_none(),
            "'{strip_group}' should not be a workflow command-strip group"
        );
    }
}

/// The workflow command strip is tabbed by process instead of exposing every
/// authoring button in one permanent row.
#[test]
fn workflow_command_strip_routes_tabbed_panels() {
    let mut harness = demo_harness();
    harness.run();

    for tab in [
        "Design", "Frame", "Openings", "Roofs", "Annotate", "Inspect", "Plan",
    ] {
        assert!(
            harness.query_all_by_label(tab).next().is_some(),
            "workflow tab '{tab}' should be visible"
        );
    }

    for old_group in ["WORKSPACE", "BUILD", "DIMENSION", "TOOLS"] {
        assert!(
            harness.query_all_by_label(old_group).next().is_none(),
            "'{old_group}' should not be a broad command-strip group"
        );
    }

    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Frame);
    assert!(
        harness.query_all_by_label("Wall").next().is_some(),
        "Frame tab should expose the Wall tool by default"
    );

    harness.get_by_label("Openings").click();
    harness.run();
    assert!(
        harness.query_all_by_label("Opening").next().is_some(),
        "Openings tab should expose the opening flyout"
    );
    assert!(
        harness.query_all_by_label("Garage").next().is_none(),
        "opening variants should not be permanent top-level command buttons"
    );
    harness.get_by_label("Opening").click();
    harness.run();
    assert!(
        harness.query_all_by_label("Garage").next().is_some(),
        "opening flyout should expose opening variants"
    );

    harness.get_by_label("Roofs").click();
    harness.run();
    assert!(
        harness.query_all_by_label("Roof form").next().is_some(),
        "Roofs tab should expose the roof flyout"
    );
    assert!(
        harness.query_all_by_label("Gable").next().is_none(),
        "roof forms should not be permanent top-level command buttons"
    );
    harness.get_by_label("Roof form").click();
    harness.run();
    assert!(
        harness.query_all_by_label("Gable").next().is_some(),
        "roof flyout should expose roof forms"
    );

    harness.get_by_label("Plan").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert!(
        harness.query_all_by_label("Section").next().is_some(),
        "Plan tab should expose generated-plan tools"
    );

    harness.get_by_label("Frame").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
}

/// The command metadata seam and rendered command strip should stay in lockstep:
/// every top-level command-strip action must be reachable on its owning workflow
/// tab, or future commands can be documented without actually being surfaced.
#[test]
fn workflow_command_strip_renders_metadata_top_level_actions() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();

    for tab in [
        actions::WorkflowTab::Design,
        actions::WorkflowTab::Frame,
        actions::WorkflowTab::Openings,
        actions::WorkflowTab::Roofs,
        actions::WorkflowTab::Annotate,
        actions::WorkflowTab::Inspect,
        actions::WorkflowTab::Plan,
    ] {
        harness
            .get_by_role_and_label(Role::Button, workflow_tab_test_label(tab))
            .click();
        harness.run();

        for action in actions::ACTIONS.iter().filter(|action| {
            matches!(
                action.command_strip,
                Some(actions::CommandStripRoute {
                    tab: action_tab,
                    presentation: actions::CommandPresentation::TopLevel,
                    ..
                }) if action_tab == tab
            )
        }) {
            assert_accessible_label(&harness, action.label, workflow_tab_test_label(tab));
        }
    }
}

/// Insertion variants live in command-strip flyouts, but still execute the same
/// authored-model mutations and undo labels as the old top-level buttons.
#[test]
fn insertion_flyouts_execute_opening_and_roof_variants() {
    let mut harness = demo_harness();
    harness.run();

    let wall = harness.state().selected_wall;
    let openings_before = harness.state().model.walls[wall].openings.len();
    harness.get_by_label("Openings").click();
    harness.run();
    harness.get_by_label("Opening").click();
    harness.run();
    harness.get_by_label("Garage").click();
    harness.run();

    let wall = harness.state().selected_wall;
    assert_eq!(
        harness.state().model.walls[wall].openings.len(),
        openings_before + 1
    );
    assert!(
        harness.state().model.walls[wall]
            .openings
            .iter()
            .any(|opening| opening.kind == OpeningKind::GarageDoor),
        "garage variant should insert a garage-door opening"
    );
    assert_eq!(harness.state().history.undo_label(), Some("Add opening"));

    let roofs_before = harness.state().model.roof_planes.len();
    harness.get_by_label("Roofs").click();
    harness.run();
    harness.get_by_label("Roof form").click();
    harness.run();
    harness.get_by_label("Hip").click();
    harness.run();

    assert!(
        harness.state().model.roof_planes.len() > roofs_before,
        "hip roof variant should author roof planes"
    );
    assert_eq!(harness.state().history.undo_label(), Some("Add roof"));
}

/// Command search is the universal backstop for commands that moved out of
/// permanent chrome. Hidden flyout variants remain searchable and executable.
#[test]
fn command_search_header_button_opens_modal() {
    let mut harness = demo_harness();
    harness.run();

    harness.get_by_label("Commands").click();
    harness.run();

    assert!(harness.state().command_search.open);
    assert!(
        harness
            .query_all_by_label("Command Search")
            .next()
            .is_some(),
        "header command button should open command search through action dispatch"
    );
}

#[test]
fn command_search_executes_hidden_insertion_variant() {
    let mut harness = demo_harness();
    harness.run();

    assert!(
        harness.query_all_by_label("Commands").next().is_some(),
        "the app header should expose command search"
    );
    assert!(
        harness.query_all_by_label("Garage").next().is_none(),
        "garage should not be visible before opening its flyout or search"
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run();
    assert!(
        harness
            .query_all_by_label("Command Search")
            .next()
            .is_some(),
        "Cmd/Ctrl+K should open command search"
    );

    let wall = harness.state().selected_wall;
    let openings_before = harness.state().model.walls[wall].openings.len();
    harness.state_mut().command_search.query = "garage".to_owned();
    harness.run();
    assert!(
        harness.query_all_by_label("Door").next().is_none(),
        "search should filter non-matching opening variants"
    );
    harness.get_by_label("Garage").click();
    harness.run();

    let wall = harness.state().selected_wall;
    assert_eq!(
        harness.state().model.walls[wall].openings.len(),
        openings_before + 1
    );
    assert!(
        harness.state().model.walls[wall]
            .openings
            .iter()
            .any(|opening| opening.kind == OpeningKind::GarageDoor),
        "search result should execute the garage-door insertion"
    );
    assert!(!harness.state().command_search.open);
    assert_eq!(harness.state().history.undo_label(), Some("Add opening"));
}

#[test]
fn command_search_enter_executes_first_enabled_match() {
    let mut harness = demo_harness();
    harness.run();

    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert!(!harness.state().action_enabled(ActionId::ExportArtifacts));

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run();
    harness.state_mut().command_search.query = "plan".to_owned();
    harness.run();

    harness.key_press(egui::Key::Enter);
    harness.run();

    assert_eq!(
        harness.state().workspace_mode,
        WorkspaceMode::Plan,
        "Enter should skip the disabled Export match and execute Plan"
    );
    assert!(!harness.state().command_search.open);
}

#[test]
fn command_search_does_not_execute_disabled_action() {
    let mut harness = demo_harness();
    harness.run();

    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert!(harness.state().artifact_status.is_none());
    assert!(!harness.state().action_enabled(ActionId::ExportArtifacts));

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run();
    harness.state_mut().command_search.query = "export".to_owned();
    harness.run();
    assert!(
        harness.query_all_by_label("Export").next().is_some(),
        "disabled commands should still be discoverable"
    );

    harness.key_press(egui::Key::Enter);
    harness.run();

    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert!(
        harness.state().artifact_status.is_none(),
        "disabled Export must not run from command search"
    );
    assert!(
        harness.state().command_search.open,
        "search should stay open when no enabled match can execute"
    );
}

#[test]
fn command_search_escape_closes_without_executing() {
    let mut harness = demo_harness();
    harness.run();

    let wall = harness.state().selected_wall;
    let openings_before = harness.state().model.walls[wall].openings.len();

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run();
    harness.state_mut().command_search.query = "garage".to_owned();
    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.run();

    let wall = harness.state().selected_wall;
    assert!(!harness.state().command_search.open);
    assert_eq!(
        harness.state().model.walls[wall].openings.len(),
        openings_before,
        "Escape should dismiss search without executing the visible match"
    );
}

/// Workspace mode and view switching live in the workspace/view bar, not inside
/// the workflow command strip's modeling panels.
#[test]
fn workspace_view_bar_owns_workspace_and_view_controls() {
    let mut harness = demo_harness();
    harness.run();

    for label in [
        "Design Workspace",
        "Plan Workspace",
        "Shell",
        "Wall",
        "Roof",
        "3D",
        "Render",
    ] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "workspace/view bar should expose '{label}'"
        );
    }
    assert!(
        harness.query_all_by_label("View").next().is_none(),
        "workflow command strip should no longer expose a View panel"
    );

    harness.get_by_label("Plan Workspace").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Plan);
    assert!(
        harness.query_all_by_label("Plan").next().is_some(),
        "Plan workspace should relabel the plan view tab"
    );
    assert!(
        harness.query_all_by_label("Elevation").next().is_some(),
        "Plan workspace should relabel the elevation view tab"
    );
    harness.get_by_label("Elevation").click();
    harness.run();
    assert_eq!(harness.state().viewport_mode, ViewportMode::Elevation);

    harness.get_by_label("Design Workspace").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Frame);

    harness.get_by_label("Shell").click();
    harness.run();
    assert_eq!(harness.state().viewport_mode, ViewportMode::Plan);

    harness.get_by_label("Render").click();
    harness.run_steps(1);
    assert_eq!(harness.state().viewport_mode, ViewportMode::Render);

    harness.get_by_label("Roof").click();
    harness.run_steps(1);
    assert_eq!(harness.state().viewport_mode, ViewportMode::RoofPlan);

    harness.get_by_label("3D").click();
    harness.run_steps(1);
    assert_eq!(harness.state().viewport_mode, ViewportMode::Axonometric);
}

/// Active tool settings live in the contextual options strip beside the canvas
/// instead of expanding the workflow command strip's permanent panels.
#[test]
fn contextual_tool_options_follow_active_authoring_tools() {
    let mut harness = demo_harness();
    harness.run();
    assert!(
        harness.query_all_by_label("Wall options").next().is_none(),
        "inactive tools should not show contextual options"
    );

    harness.key_press(egui::Key::W);
    harness.run();
    assert!(harness.state().draw_wall_tool.active);
    for label in [
        "Wall options",
        "Baseline",
        "Centerline",
        "Height",
        "Level",
        "Placement",
        "First endpoint",
    ] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "wall tool options should expose '{label}'"
        );
    }

    harness.key_press(egui::Key::W);
    harness.run();
    assert!(!harness.state().draw_wall_tool.active);
    assert!(
        harness.query_all_by_label("Wall options").next().is_none(),
        "wall options should leave with the wall tool"
    );

    harness.key_press(egui::Key::D);
    harness.run();
    assert!(harness.state().dimension_tool.active);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Annotate);
    for label in ["Dimension options", "Dimension Kind", "Dimension Axis"] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "dimension tool options should expose '{label}'"
        );
    }
    assert!(
        harness.query_all_by_value("Driving").next().is_some(),
        "dimension kind combo should expose its current value"
    );
    assert!(
        harness.query_all_by_value("Horizontal").next().is_some(),
        "dimension axis combo should expose its current value"
    );

    harness.get_by_value("Driving").click();
    harness.run();
    harness.get_by_label("Reference").click();
    harness.run();
    assert_eq!(
        harness.state().dimension_tool.kind,
        DimensionKind::Reference
    );

    harness.get_by_value("Horizontal").click();
    harness.run();
    harness.get_by_label("Vertical").click();
    harness.run();
    assert_eq!(harness.state().dimension_tool.axis, DimensionAxis::Vertical);
}

/// Selection lifecycle actions live on canvas context chrome instead of taking a
/// permanent workflow command-strip slot.
#[test]
fn selection_context_toolbar_deletes_selected_wall() {
    let mut harness = demo_harness();
    harness.run();

    assert_eq!(harness.state().selected, Selection::Wall);
    let before = harness.state().model.walls.len();
    assert!(
        harness.query_all_by_label("Delete").next().is_some(),
        "a selected wall should expose contextual delete"
    );

    harness.get_by_label("Delete").click();
    harness.run();

    assert_eq!(harness.state().model.walls.len(), before - 1);
    assert_eq!(harness.state().selected, Selection::Wall);
    assert_eq!(harness.state().history.undo_label(), Some("Delete wall"));
}

/// Opening-specific lifecycle actions share the canvas context toolbar with
/// Delete. Duplicating from that surface must keep using the existing edit/undo
/// path instead of becoming decorative chrome.
#[test]
fn selection_context_toolbar_duplicates_selected_opening() {
    let mut harness = demo_harness();
    harness.run();

    let wall_index = harness.state().selected_wall;
    let opening_id = harness.state().model.walls[wall_index].openings[0]
        .id
        .0
        .clone();
    let openings_before = harness.state().model.walls[wall_index].openings.len();
    harness.state_mut().selected = Selection::Opening(opening_id);
    harness.run();

    assert_accessible_label(&harness, "Duplicate opening", "selection context toolbar");
    assert_accessible_label(&harness, "Delete", "selection context toolbar");

    harness.get_by_label("Duplicate opening").click();
    harness.run();

    let wall = &harness.state().model.walls[wall_index];
    assert_eq!(wall.openings.len(), openings_before + 1);
    assert!(
        wall.openings
            .iter()
            .any(|opening| opening.name.ends_with(" copy")),
        "duplicating from the context toolbar should author a copied opening"
    );
    assert_eq!(
        harness.state().history.undo_label(),
        Some("Duplicate opening")
    );
    assert!(matches!(harness.state().selected, Selection::Opening(_)));
}

/// Tool shortcuts entered from the generated-plan workflow must return to a
/// Design-compatible command tab before activating the tool.
#[test]
fn tool_shortcut_from_plan_tab_returns_to_frame_commands() {
    let mut harness = demo_harness();
    harness.run();

    harness.get_by_label("Plan").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Plan);
    assert!(
        harness.query_all_by_label("Section").next().is_some(),
        "Plan tab should expose generated-plan commands before the shortcut"
    );

    harness.key_press(egui::Key::W);
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Frame);
    assert!(
        harness.state().draw_wall_tool.active,
        "W should activate the wall tool"
    );
    assert!(
        harness.query_all_by_label("Wall").next().is_some(),
        "shortcut should route back to Frame commands"
    );
}

/// Dimension shortcuts entered from generated-plan workflow must reveal the
/// annotation controls that configure the active dimension tool.
#[test]
fn dimension_shortcut_from_plan_tab_returns_to_annotate_commands() {
    let mut harness = demo_harness();
    harness.run();

    harness.get_by_label("Plan").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Plan);

    harness.key_press(egui::Key::D);
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Annotate);
    assert!(
        harness.state().dimension_tool.active,
        "D should activate the dimension tool"
    );
    assert!(
        harness.query_all_by_label("Linear").next().is_some(),
        "shortcut should route back to Annotate commands"
    );
}

/// Keyboard shortcuts should route the command strip to the tab that owns the
/// newly active tool, even when launched from another workflow tab.
#[test]
fn authoring_shortcuts_route_to_owning_command_tabs() {
    assert_shortcut_from_annotate_routes_to(
        egui::Key::R,
        actions::WorkflowTab::Design,
        "Room",
        |app| assert!(app.room_tool_active, "R should activate the room tool"),
    );
    assert_shortcut_from_annotate_routes_to(
        egui::Key::C,
        actions::WorkflowTab::Frame,
        "Ceiling",
        |app| {
            assert!(
                app.ceiling_tool_active,
                "C should activate the ceiling tool"
            )
        },
    );
    assert_shortcut_from_annotate_routes_to(
        egui::Key::F,
        actions::WorkflowTab::Frame,
        "Floor",
        |app| assert!(app.floor_tool_active, "F should activate the floor tool"),
    );
    assert_shortcut_from_annotate_routes_to(
        egui::Key::V,
        actions::WorkflowTab::Frame,
        "Vault",
        |app| assert!(app.vault_tool_active, "V should activate the vault tool"),
    );
}

/// The `W` keyboard shortcut toggles the draw-wall tool, proving keyboard input
/// flows through `handle_keyboard_shortcuts` and mutates the model — driven
/// entirely headlessly.
#[test]
fn w_key_toggles_draw_wall_tool() {
    let mut harness = demo_harness();
    harness.run();
    assert!(
        !harness.state().draw_wall_tool.active,
        "draw-wall tool should start inactive"
    );

    harness.key_press(egui::Key::W);
    harness.run();
    assert!(
        harness.state().draw_wall_tool.active,
        "pressing W should activate the draw-wall tool"
    );

    harness.key_press(egui::Key::W);
    harness.run();
    assert!(
        !harness.state().draw_wall_tool.active,
        "pressing W again should deactivate the draw-wall tool"
    );
}

/// A fresh app shows walls as outlines with every layer visible — the cleanest
/// default so a loaded shell reads as a line drawing first.
#[test]
fn layers_default_to_outline_with_everything_visible() {
    let mut harness = demo_harness();
    harness.run();
    let layers = harness.state().layers;
    assert_eq!(layers.wall_display, WallDisplay::Outline);
    assert!(
        layers.grid && layers.rooms && layers.joins && layers.wall_labels,
        "all visibility layers should default on"
    );
}

/// The plan view lays out in every wall display mode without panicking —
/// exercising the outline/width/full branches and the per-mode opening handling
/// against the demo shell (which has openings).
#[test]
fn plan_view_lays_out_in_every_wall_display_mode() {
    for mode in [WallDisplay::Outline, WallDisplay::Width, WallDisplay::Full] {
        let mut harness = demo_harness();
        harness.run();
        harness.state_mut().layers.wall_display = mode;
        harness.run();
        // Smoke check: the frame laid out without panicking (state survives is a
        // sanity guard — render output isn't observable headlessly; the
        // `scene_3d_*` unit tests pin the actual per-mode geometry).
        assert!(
            !harness.state().model.walls.is_empty(),
            "plan view in {mode:?} mode should lay out without panicking"
        );
    }
}

/// Hiding every visibility layer still lays out the plan view (the guard branches
/// are all exercised and none panics).
#[test]
fn plan_view_lays_out_with_all_layers_hidden() {
    let mut harness = demo_harness();
    harness.run();
    {
        let layers = &mut harness.state_mut().layers;
        layers.grid = false;
        layers.rooms = false;
        layers.joins = false;
        layers.wall_labels = false;
    }
    harness.run();
    assert!(
        !harness.state().model.walls.is_empty(),
        "plan view should lay out without panicking with all layers hidden"
    );
}

/// Smoke test: the 3D (axonometric) view lays out headlessly in every wall display
/// mode without panicking. This is the only thing that runs `axonometric.rs`'s
/// per-mode paths — the outline painter overlay (Outline) and the empty-fill
/// callback guard — which execute only when the 3D view actually draws. The harness
/// has no GPU, so the CPU paths (scene build, orbit projector, outline overlay) run
/// without the wgpu callback. Observable per-mode geometry is pinned by the
/// `scene_3d_*` unit tests instead.
#[test]
fn axonometric_view_lays_out_in_every_wall_display_mode() {
    for mode in [WallDisplay::Outline, WallDisplay::Width, WallDisplay::Full] {
        let mut harness = demo_harness();
        harness.run();
        {
            let app = harness.state_mut();
            app.viewport_mode = ViewportMode::Axonometric;
            app.layers.wall_display = mode;
        }
        harness.run();
        assert!(
            harness.state().project_plan.is_some(),
            "3D view in {mode:?} mode should lay out without panicking"
        );
    }
}

/// Drive the ceiling inspector's slope editor through the real UI: select the demo
/// shell's flat (polygon) ceiling, toggle the "Sloped" checkbox, and confirm the
/// model gains then loses a slope — proving the editor is wired through the inspector
/// edit path (Slice A5.1). The ceiling is a `Polygon` region, so the toggle is
/// enabled.
#[test]
fn ceiling_inspector_toggles_slope_through_the_ui() {
    use framer_core::{Ceiling, Length, Point2, SurfaceRegion};

    let mut harness = demo_harness();
    harness.run();
    // The default demo-shell model carries no ceiling, so add a polygon-region one
    // (a sloped ceiling needs an explicit outline anyway) over the 28×20ft footprint,
    // referencing the starter ceiling system so the model stays valid, and select it.
    {
        let ft = Length::from_feet;
        harness.state_mut().model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling-1",
            SurfaceRegion::Polygon(vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(ft(28.0), Length::ZERO),
                Point2::new(ft(28.0), ft(20.0)),
                Point2::new(Length::ZERO, ft(20.0)),
            ]),
            ft(8.0),
        ));
        harness.state_mut().selected = Selection::Ceiling("ceiling-1".to_owned());
    }
    harness.run();
    let slope_of = |h: &Harness<FramerApp>| {
        h.state()
            .model
            .ceilings
            .iter()
            .find(|c| c.id.0 == "ceiling-1")
            .and_then(|c| c.slope)
    };
    assert!(slope_of(&harness).is_none(), "the ceiling starts flat");
    assert!(harness.state().error.is_none(), "the flat ceiling is valid");

    // Enable the slope.
    harness.get_by_label("Sloped").click();
    harness.run();
    assert!(
        slope_of(&harness).is_some(),
        "toggling Sloped on makes the ceiling sloped"
    );
    // The authored slope produces a model that still validates (the framing plan
    // regenerates without error).
    assert!(
        harness.state().error.is_none() && harness.state().project_plan.is_some(),
        "a sloped polygon ceiling is a valid model: {:?}",
        harness.state().error
    );

    // Disable it again.
    harness.get_by_label("Sloped").click();
    harness.run();
    assert!(
        slope_of(&harness).is_none(),
        "toggling Sloped off makes the ceiling flat again"
    );
}

/// Enabling a slope on a **room-attached** ceiling converts its region to an explicit
/// polygon (the resolved wall-loop boundary) — the load-bearing A5.1 logic, since a
/// sloped ceiling needs a fixed outline. Driven through the real inspector.
#[test]
fn ceiling_inspector_slope_converts_a_room_region_to_a_polygon() {
    use framer_core::{Ceiling, ElementId, Length, Point2, Room, RoomUsage, SurfaceRegion};

    let mut harness = demo_harness();
    harness.run();
    {
        let model = &mut harness.state_mut().model;
        // A room seeded inside the demo shell's closed 28×20ft wall loop, plus a
        // room-attached (flat) ceiling over it.
        model.rooms.push(Room::new(
            "room-test",
            "Room",
            RoomUsage::default(),
            "level-1",
            Point2::new(Length::from_feet(14.0), Length::from_feet(10.0)),
        ));
        model.ceilings.push(Ceiling::new(
            "ceiling-room",
            "Room ceiling",
            "level-1",
            "system-ceiling-1",
            SurfaceRegion::Room(ElementId::new("room-test")),
            Length::from_feet(8.0),
        ));
    }
    harness.state_mut().selected = Selection::Ceiling("ceiling-room".to_owned());
    harness.run();
    let ceiling = |h: &Harness<FramerApp>| {
        h.state()
            .model
            .ceilings
            .iter()
            .find(|c| c.id.0 == "ceiling-room")
            .cloned()
            .expect("the room ceiling exists")
    };
    assert!(
        matches!(ceiling(&harness).region, SurfaceRegion::Room(_)),
        "starts as a room region"
    );
    assert!(
        harness.state().error.is_none(),
        "the flat room ceiling is valid"
    );

    // Enable the slope: the region must become an explicit polygon, and the model
    // must still validate (a sloped Room region would be rejected).
    harness.get_by_label("Sloped").click();
    harness.run();
    let after = ceiling(&harness);
    assert!(after.slope.is_some(), "the ceiling became sloped");
    assert!(
        matches!(after.region, SurfaceRegion::Polygon(_)),
        "sloping a room ceiling converts its region to a polygon"
    );
    assert!(
        harness.state().error.is_none() && harness.state().project_plan.is_some(),
        "the converted sloped ceiling is a valid model: {:?}",
        harness.state().error
    );
}

/// Drive the Layers popover through the real UI: open it and pick a wall display
/// mode, confirming the selection flows into app state. This is the only test that
/// exercises the popover wiring end-to-end (the others set the field directly). The
/// icon-only trigger carries an explicit "Layers" accessible name, so the test
/// locates the button by that name (the `Button` role disambiguates it from egui's
/// generated text-label node of the same name).
#[test]
fn layers_popover_selects_wall_display_mode() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();
    assert_eq!(harness.state().layers.wall_display, WallDisplay::Outline);

    harness
        .get_by_role_and_label(Role::Button, "Layers")
        .click();
    harness.run();

    // The selector items render their mode label; clicking "Full" must flip state.
    harness.get_by_label("Full").click();
    harness.run();
    assert_eq!(harness.state().layers.wall_display, WallDisplay::Full);
}
