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
use framer_core::{DimensionAxis, DimensionKind, Length, OpeningKind, Point2};

use super::actions::{self, ActionId};
use super::component_visibility::{
    AuthoredComponentKind, ComponentKey, IsolationMode, SelectionOp,
};
use super::viewport::BuiltInPreset;
use super::{
    FramerApp, RoofForm, Selection, ViewportMode, WallDisplay, WorkspaceMode, design, panels,
};

/// A headless harness wrapping a fully-loaded `FramerApp` (the demo shell).
///
/// `build_ui_state` hands the closure a central, margin-less `Ui` — the same
/// thing `eframe` passes to [`eframe::App::ui`] — so the panel tree lays out
/// identically to the running app.
fn demo_harness<'a>() -> Harness<'a, FramerApp> {
    demo_harness_with_size(egui::vec2(1360.0, 860.0))
}

fn demo_harness_with_size<'a>(size: egui::Vec2) -> Harness<'a, FramerApp> {
    // `FramerApp::new` installs the design tokens + Lucide icon font on the egui
    // context before the first paint. The harness owns its own context, and
    // `set_fonts` only takes effect at the *next* frame's begin-pass — so the
    // first frame is a no-paint warm-up that just binds the fonts. egui requests
    // repaints during that initial layout/texture upload, so `run()` advances to
    // the real UI frames where `FontFamily::Name("lucide")` icons are laid out.
    let mut fonts_bound = false;
    Harness::builder()
        .with_size(size)
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

fn assert_accessible_button(harness: &Harness<FramerApp>, label: &str, surface: &str) {
    assert!(
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, label)
            .next()
            .is_some(),
        "{surface} should expose '{label}' as a button"
    );
}

fn secondary_click_pickable_3d_component(harness: &mut Harness<FramerApp>) {
    secondary_click_pickable_3d_component_at_xs(harness, &[0.25, 0.375, 0.5, 0.625, 0.75]);
}

fn secondary_click_pickable_3d_component_at_xs(
    harness: &mut Harness<FramerApp>,
    x_positions: &[f32],
) {
    let rect = harness.get_by_label("3D viewport").rect();
    for y in [0.25, 0.375, 0.5, 0.625, 0.75] {
        for &x in x_positions {
            let position = egui::pos2(egui::lerp(rect.x_range(), x), egui::lerp(rect.y_range(), y));
            harness.event(egui::Event::PointerMoved(position));
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            harness.run();
            if harness.state().context_menu_context.is_some() {
                return;
            }
        }
    }
    panic!("the demo-shell 3D viewport should expose at least one pickable component");
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

#[test]
fn header_menus_and_tree_sections_are_reachable_on_default_theme() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();

    for label in ["Project", "Examples"] {
        assert!(
            harness
                .query_all_by_role_and_label(Role::Button, label)
                .next()
                .is_some(),
            "the header should expose '{label}' as a visible menu button"
        );
    }

    assert!(
        harness.query_all_by_label("Corners").next().is_some(),
        "the default light model tree should expose the Corners section heading"
    );
}

#[test]
fn model_tree_and_status_use_names_and_corner_language() {
    let mut harness = demo_harness();
    harness.run();

    assert!(
        harness.query_all_by_label("Back wall").next().is_some(),
        "model tree rows should lead with object names"
    );
    assert!(
        harness
            .query_all_by_label("Window: Back left window")
            .next()
            .is_none(),
        "model tree rows should not use Type: Name labels"
    );
    assert!(
        harness.query_all_by_label("Corners").next().is_some(),
        "wall junctions should use Corner as the visible concept name"
    );
    assert!(
        harness.query_all_by_label("Wall joins").next().is_none(),
        "wall junctions should not expose Join in section labels"
    );

    harness.get_by_label("Back left corner").click();
    harness.run();
    assert_eq!(
        harness.state().selected,
        Selection::Join("join-back-left".to_owned())
    );
    assert!(
        harness
            .query_all_by_label("Corner: Back left corner")
            .next()
            .is_some(),
        "status breadcrumb should use the corner display name"
    );
    assert!(
        harness
            .query_all_by_label("Join: join-back-left")
            .next()
            .is_none(),
        "status breadcrumb should not expose the internal join id"
    );

    harness.get_by_label("Back left window").click();
    harness.run();
    assert_eq!(
        harness.state().selected,
        Selection::Opening("opening-back-left-window".to_owned())
    );
    assert!(
        harness
            .query_all_by_label("Opening: Back left window")
            .next()
            .is_some(),
        "status breadcrumb should use opening display names"
    );
    assert!(
        harness
            .query_all_by_label("Opening: opening-back-left-window")
            .next()
            .is_none(),
        "status breadcrumb should not expose opening ids"
    );
}

#[test]
fn authored_component_eye_is_accessible_reversible_and_keeps_hidden_row_selectable() {
    let mut harness = demo_harness();
    harness.run();

    let (wall_id, wall_name) = {
        let wall = &harness.state().model.walls[0];
        (wall.id.0.clone(), wall.name.clone())
    };
    let key = ComponentKey::authored(AuthoredComponentKind::Wall, wall_id);
    let model_before = harness.state().model.clone();
    let plan_before = harness.state().project_plan.clone();

    let hide_label = format!("Hide {wall_name}");
    assert_accessible_button(&harness, &hide_label, "authored component eye");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &hide_label)
        .click();
    harness.run();

    assert!(
        !harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "the eye should hide only the authored component presentation"
    );
    assert_eq!(harness.state().model, model_before);
    assert_eq!(harness.state().project_plan, plan_before);
    assert_eq!(
        harness.state().history.undo_label(),
        Some(hide_label.as_str()),
        "the eye action should identify the hidden component in history"
    );

    harness.state_mut().execute_action(ActionId::Undo);
    harness.run();
    assert!(
        harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "undo should recover an accidentally hidden component"
    );
    assert_eq!(harness.state().model, model_before);
    assert_eq!(harness.state().project_plan, plan_before);

    harness.state_mut().execute_action(ActionId::Redo);
    harness.run();
    assert!(
        !harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "redo should reapply the component visibility action"
    );

    let show_label = format!("Show {wall_name}");
    assert_accessible_button(&harness, &show_label, "hidden authored component eye");
    assert_accessible_button(&harness, &wall_name, "hidden authored component row");

    harness
        .state_mut()
        .apply_selection(Selection::Wall, Some(1), SelectionOp::Replace);
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &wall_name)
        .click();
    harness.run();
    assert!(
        harness.state().component_is_selected(&key),
        "a hidden component row should remain selectable"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &show_label)
        .click();
    harness.run();
    assert!(
        harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "the hidden row eye should restore the component"
    );
    assert_eq!(
        harness.state().history.undo_label(),
        Some(show_label.as_str())
    );
    assert_accessible_button(
        &harness,
        &format!("Hide {wall_name}"),
        "restored authored component eye",
    );
}

#[test]
fn generated_component_eye_uses_host_and_member_identity() {
    let mut harness = demo_harness();
    harness.run();

    let (host_id, member_id, member_name, wall_index) = {
        let app = harness.state();
        let wall_plan = app
            .project_plan
            .as_ref()
            .and_then(|plan| plan.wall_plans.first())
            .expect("demo shell wall framing");
        let member = wall_plan.members.first().expect("generated wall member");
        let wall_index = app
            .model
            .walls
            .iter()
            .position(|wall| wall.id == wall_plan.wall)
            .expect("generated wall host should be authored");
        (
            wall_plan.wall.0.clone(),
            member.id.clone(),
            format!("{}: {}", member.kind.label(), member.id),
            wall_index,
        )
    };
    let key = ComponentKey::member(host_id.clone(), member_id.clone());

    harness.state_mut().apply_selection(
        Selection::Member {
            source_id: host_id,
            member_id,
        },
        Some(wall_index),
        SelectionOp::Replace,
    );
    harness.state_mut().workspace_mode = WorkspaceMode::Plan;
    harness.state_mut().command_tab = actions::WorkflowTab::Plan;
    harness.run();

    let hide_label = format!("Hide {member_name}");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &hide_label)
        .scroll_to_me();
    harness.run();
    assert_accessible_button(&harness, &hide_label, "generated component eye");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &hide_label)
        .click();
    harness.run();

    assert!(
        !harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "the generated eye should address the exact host + member leaf"
    );
    let show_label = format!("Show {member_name}");
    assert_accessible_button(&harness, &show_label, "hidden generated component eye");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &show_label)
        .click();
    harness.run();
    assert!(
        harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key)
    );
}

#[test]
fn model_browser_show_all_restores_hidden_component_overrides() {
    let mut harness = demo_harness();
    harness.run();
    let (wall_id, wall_name) = {
        let wall = &harness.state().model.walls[0];
        (wall.id.0.clone(), wall.name.clone())
    };
    let key = ComponentKey::authored(AuthoredComponentKind::Wall, wall_id);

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &format!("Hide {wall_name}"))
        .click();
    harness.run();
    assert_accessible_button(&harness, "Show all components", "Model Browser");

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Show all components")
        .click();
    harness.run();
    assert!(
        harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key)
    );
    assert_eq!(
        harness.state().history.undo_label(),
        Some("Show All Components")
    );

    harness.state_mut().execute_action(ActionId::Undo);
    harness.run();
    assert!(
        !harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "undoing Show All should restore the hidden override set"
    );

    harness.state_mut().execute_action(ActionId::Redo);
    harness.run();
    assert!(
        harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "redoing Show All should clear the hidden override set again"
    );
    assert_accessible_button(
        &harness,
        "Show all components",
        "disabled Model Browser recovery action",
    );
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Show all components")
        .click();
    harness.run_ok();
    assert!(
        harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "the disabled recovery action must not change presentation state"
    );
}

#[test]
fn render_workspace_suppresses_isolation_status_and_disables_browser_eyes() {
    let mut harness = demo_harness();
    harness.run();
    let (wall_id, wall_name) = {
        let wall = &harness.state().model.walls[0];
        (wall.id.0.clone(), wall.name.clone())
    };
    let key = ComponentKey::authored(AuthoredComponentKind::Wall, wall_id);
    harness
        .state_mut()
        .component_visibility
        .isolate(IsolationMode::DimOthers, vec![key.clone()]);
    harness
        .state_mut()
        .set_workspace_mode(WorkspaceMode::Render);
    harness.run_ok();

    assert!(!harness.state().selection_status().contains("Isolated"));
    let hide_label = format!("Hide {wall_name}");
    assert_accessible_button(&harness, &hide_label, "disabled Render browser eye");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &hide_label)
        .click();
    harness.run_ok();
    assert!(
        harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "Render browser eyes must not mutate interactive-only presentation state"
    );
}

#[test]
fn design_disables_plan_only_opening_visibility_eye() {
    let mut harness = demo_harness();
    harness.run();
    let (opening_id, opening_name) = harness
        .state()
        .model
        .walls
        .iter()
        .flat_map(|wall| &wall.openings)
        .map(|opening| (opening.id.0.clone(), opening.name.clone()))
        .next()
        .expect("demo shell opening");
    let key = ComponentKey::authored(AuthoredComponentKind::Opening, opening_id);
    let hide_label = format!("Hide {opening_name}");

    assert_accessible_button(&harness, &hide_label, "disabled Design opening eye");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &hide_label)
        .click();
    harness.run_ok();
    assert!(
        harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "Design omits rough-opening framing, so its eye must not create an invisible override"
    );

    harness.state_mut().set_workspace_mode(WorkspaceMode::Plan);
    harness.run_ok();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &hide_label)
        .click();
    harness.run_ok();
    assert!(
        !harness
            .state()
            .component_visibility
            .is_explicitly_visible(&key),
        "Plan renders the opening's generated frame, so its eye should be enabled"
    );
}

#[test]
fn model_tree_command_or_ctrl_click_toggles_multi_selection() {
    let mut harness = demo_harness();
    harness.run();
    let room_level = harness.state().model.levels[0].id.0.clone();
    harness.state_mut().model.rooms.push(framer_core::Room::new(
        "room-multi-select",
        "Multi-select room",
        framer_core::RoomUsage::default(),
        room_level,
        Point2::new(Length::from_feet(1.0), Length::from_feet(1.0)),
    ));
    harness.run();

    let walls = harness
        .state()
        .model
        .walls
        .iter()
        .take(2)
        .map(|wall| (wall.id.0.clone(), wall.name.clone()))
        .collect::<Vec<_>>();
    assert_eq!(walls.len(), 2, "demo shell should expose two wall rows");
    let first_key = ComponentKey::authored(AuthoredComponentKind::Wall, walls[0].0.clone());
    let second_key = ComponentKey::authored(AuthoredComponentKind::Wall, walls[1].0.clone());

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &walls[0].1)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &walls[1].1)
        .click_modifiers(egui::Modifiers::CTRL);
    harness.run();

    assert_eq!(harness.state().selected_component_count(), 2);
    assert!(harness.state().component_is_selected(&first_key));
    assert!(harness.state().component_is_selected(&second_key));
    assert_accessible_label(
        &harness,
        "2 components selected",
        "multi-selection inspector and status",
    );
    let primary_id_label = format!("ID: {}", walls[1].0);
    assert!(
        harness
            .query_all_by_label(&primary_id_label)
            .next()
            .is_none(),
        "multi-selection should suppress the primary component's single-object editor"
    );
    let isolated = harness.state().selected_components();
    harness
        .state_mut()
        .component_visibility
        .isolate(IsolationMode::DimOthers, isolated);
    harness.run();
    assert_accessible_label(
        &harness,
        "2 components selected • Isolated: Dim others",
        "active isolation status",
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &walls[1].1)
        .click_modifiers(egui::Modifiers::COMMAND);
    harness.run();
    assert_eq!(harness.state().selected_component_count(), 1);
    assert!(harness.state().component_is_selected(&first_key));
    assert!(!harness.state().component_is_selected(&second_key));

    let (room_id, room_name) = {
        let room = harness
            .state()
            .model
            .rooms
            .first()
            .expect("demo shell should expose a room row");
        (room.id.0.clone(), room.name.clone())
    };
    let room_key = ComponentKey::authored(AuthoredComponentKind::Room, room_id);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &room_name)
        .scroll_to_me();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &room_name)
        .click_modifiers(egui::Modifiers::CTRL);
    harness.run();
    assert_eq!(harness.state().selected_component_count(), 2);
    assert!(harness.state().component_is_selected(&first_key));
    assert!(harness.state().component_is_selected(&room_key));
}

#[test]
fn plan_multi_selection_keeps_generated_output_sections_visible() {
    let mut harness = demo_harness();
    harness.run();
    let wall_names = harness
        .state()
        .model
        .walls
        .iter()
        .take(2)
        .map(|wall| wall.name.clone())
        .collect::<Vec<_>>();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &wall_names[0])
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &wall_names[1])
        .click_modifiers(egui::Modifiers::COMMAND);
    harness.run();
    harness.state_mut().set_workspace_mode(WorkspaceMode::Plan);
    harness.run();

    for section in ["Diagnostics", "Compliance", "BOM"] {
        assert_accessible_label(&harness, section, "Plan multi-selection inspector");
    }
}

#[test]
fn status_bar_reports_canonical_cursor_units_and_zoom() {
    let mut harness = demo_harness();
    harness.run();
    harness.state_mut().cursor_model = Some(Point2::new(
        Length::from_ticks(14 * 12 * Length::TICKS_PER_INCH + 3 * Length::TICKS_PER_INCH + 10),
        Length::from_feet(24.0),
    ));
    harness.run_steps(1);

    assert!(
        harness
            .query_all_by_label("X 14' 3 5/8\"   Y 24' 0\"")
            .next()
            .is_some(),
        "status bar should render cursor coordinates with the core Length display"
    );
    assert!(
        harness.query_all_by_label("100%").next().is_some(),
        "fit-to-bounds plan view should report 100% zoom"
    );
    assert!(
        harness
            .query_all_by_label("X 14.302 ft   Y 24.000 ft   Z 0.000 ft")
            .next()
            .is_none(),
        "status bar should not render the old decimal-feet cursor readout"
    );
}

#[test]
fn status_bar_diagnostics_popover_selects_source_from_design() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);

    let diagnostic = {
        let app = harness.state();
        let plan = app
            .project_plan
            .as_ref()
            .expect("demo shell should solve to a framing plan");
        plan.diagnostics
            .iter()
            .chain(
                plan.wall_plans
                    .iter()
                    .flat_map(|wall_plan| wall_plan.diagnostics.iter()),
            )
            .find(|diagnostic| {
                diagnostic.source.as_ref().is_some_and(|source| {
                    app.model
                        .walls
                        .iter()
                        .any(|wall| wall.openings.iter().any(|opening| opening.id == *source))
                })
            })
            .cloned()
            .expect("demo shell should expose a source-backed opening diagnostic")
    };
    let source = diagnostic.source.clone().expect("diagnostic has a source");

    harness
        .get_by_role_and_label(Role::Button, "Diagnostics")
        .click();
    harness.run();

    assert!(
        harness.query_all_by_label("3 warnings").next().is_some(),
        "diagnostics popover should keep the visible warning count"
    );
    assert!(
        harness
            .query_all_by_label(diagnostic.message.as_str())
            .next()
            .is_some(),
        "diagnostics popover should list diagnostic messages"
    );

    let action_label = panels::diagnostic_row_action_label(&source);
    harness
        .get_by_role_and_label(Role::Button, &action_label)
        .click();
    harness.run();

    assert_eq!(harness.state().selected, Selection::Opening(source.0));
}

/// The workflow command strip is tabbed by process instead of exposing every
/// authoring button in one permanent row.
#[test]
fn workflow_command_strip_routes_tabbed_panels() {
    let mut harness = demo_harness();
    harness.run();

    for tab in [
        "Design", "Frame", "Openings", "Roofs", "Annotate", "Render", "Plan",
    ] {
        assert!(
            harness.query_all_by_label(tab).next().is_some(),
            "workflow tab '{tab}' should be visible"
        );
    }
    assert!(
        harness.query_all_by_label("Inspect").next().is_none(),
        "empty Inspect workflow tab should stay hidden until it owns commands"
    );

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

    harness.get_by_label("Render").click();
    harness.run_steps(1);
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Render);
    assert_eq!(harness.state().viewport_mode, ViewportMode::Render);

    harness.get_by_label("Frame").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
}

#[test]
fn render_workflow_exposes_session_render_settings() {
    let mut harness = demo_harness();
    harness.run();

    harness.get_by_label("Render").click();
    harness.run_steps(1);

    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Render);
    for label in ["Sun", "Azimuth", "Elevation", "Environment", "Exposure"] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "Render tab should expose the '{label}' setting"
        );
    }

    harness.state_mut().render_settings.sun_azimuth_deg = 90.0;
    harness.state_mut().render_settings.sun_elevation_deg = 0.0;
    harness.state_mut().render_settings.exposure = 1.75;

    let mut opts = framer_render::RenderOptions::default();
    harness.state().render_settings.apply_to_options(&mut opts);

    assert!((opts.sun.dir.x - 0.0).abs() < 1.0e-5);
    assert!((opts.sun.dir.y - 1.0).abs() < 1.0e-5);
    assert!((opts.sun.dir.z - 0.0).abs() < 1.0e-5);
    assert_eq!(opts.exposure.to_bits(), 1.75_f32.to_bits());
}

#[test]
fn generated_roof_tree_row_selects_the_member_by_owning_plan() {
    let mut harness = demo_harness();
    harness.run();
    harness.state_mut().add_roof(RoofForm::Gable);

    let (source_id, group_label, member_id, member_label) = {
        let app = harness.state();
        let roof_plan = app
            .project_plan
            .as_ref()
            .and_then(|plan| plan.roof_plans.first())
            .expect("demo shell roof framing");
        let roof_name = app
            .model
            .roof_planes
            .iter()
            .find(|plane| plane.id == roof_plan.roof)
            .map(|plane| plane.name.as_str())
            .unwrap_or(roof_plan.roof.0.as_str());
        let member = roof_plan.members.first().expect("generated roof member");
        (
            roof_plan.roof.0.clone(),
            format!(
                "Roof framing: {roof_name} ({} members)",
                roof_plan.members.len()
            ),
            member.id.clone(),
            format!("{}: {}", member.kind.label(), member.id),
        )
    };

    // Selecting the host before Plan first renders opens that generated group;
    // then replace the selection without another frame so clicking the visible
    // row must perform the actual dispatch under test.
    harness.state_mut().selected = Selection::Member {
        source_id: source_id.clone(),
        member_id: member_id.clone(),
    };
    harness.get_by_label("Plan").click();
    harness.run();
    assert!(harness.query_all_by_label(&group_label).next().is_some());
    harness.state_mut().selected = Selection::RoofPlane(source_id.clone());
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &member_label)
        .scroll_to_me();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &member_label)
        .click();
    harness.run();
    assert_eq!(
        harness.state().selected,
        Selection::Member {
            source_id,
            member_id,
        }
    );
}

/// The command metadata seam and rendered command strip should stay in lockstep:
/// every top-level command-strip action must be reachable on its owning workflow
/// tab, or future commands can be documented without actually being surfaced.
#[test]
fn workflow_command_strip_renders_metadata_top_level_actions() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();
    let mut checked = 0;

    for tab in [
        actions::WorkflowTab::Design,
        actions::WorkflowTab::Frame,
        actions::WorkflowTab::Openings,
        actions::WorkflowTab::Roofs,
        actions::WorkflowTab::Annotate,
        actions::WorkflowTab::Plan,
    ] {
        harness
            .get_by_role_and_label(Role::Button, panels::workflow_tab_label(tab))
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
            assert_accessible_button(&harness, action.label, panels::workflow_tab_label(tab));
            checked += 1;
        }
    }

    let expected = actions::ACTIONS
        .iter()
        .filter(|action| {
            matches!(
                action.command_strip,
                Some(actions::CommandStripRoute {
                    presentation: actions::CommandPresentation::TopLevel,
                    ..
                })
            )
        })
        .count();
    assert!(
        expected > 0,
        "metadata must expose at least one TopLevel command-strip action"
    );
    assert_eq!(
        checked, expected,
        "every TopLevel command-strip action should be reachable"
    );
}

/// The native window's minimum size is the documented narrow budget for command
/// surfaces. At that width the command strip can wrap panels, but the primary
/// tabs, panel and flyout trigger buttons, and command-search backstop must
/// remain reachable.
#[test]
fn command_surfaces_remain_reachable_at_minimum_window_size() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness_with_size(egui::vec2(1040.0, 680.0));
    harness.run();

    for label in ["Framer", "Project", "Examples", "Commands"] {
        assert_accessible_label(&harness, label, "minimum-width app header");
    }
    for label in ["Shell", "Wall", "Roof", "3D"] {
        assert_accessible_button(&harness, label, "minimum-width segmented view control");
    }

    for tab in [
        (actions::WorkflowTab::Design, "Room"),
        (actions::WorkflowTab::Frame, "Wall"),
        (actions::WorkflowTab::Openings, "Opening"),
        (actions::WorkflowTab::Roofs, "Roof form"),
        (actions::WorkflowTab::Annotate, "Linear"),
        (actions::WorkflowTab::Plan, "Section"),
    ] {
        let (tab, expected_label) = tab;
        harness
            .get_by_role_and_label(Role::Button, panels::workflow_tab_label(tab))
            .click();
        harness.run();
        assert_accessible_button(&harness, expected_label, panels::workflow_tab_label(tab));
    }
    assert_accessible_label(&harness, "Render", "minimum-width workflow tabs");

    harness.get_by_label("Commands").click();
    harness.run();
    assert_accessible_label(&harness, "Command Search", "minimum-width command search");
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
fn command_search_does_not_leave_output_workspace_for_disabled_authoring_action() {
    let mut harness = demo_harness();
    harness.run();

    harness.get_by_label("Plan").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert!(!harness.state().action_enabled(ActionId::ToolWall));

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run();
    harness.state_mut().command_search.query = "draw walls".to_owned();
    harness.run();
    assert!(
        harness.query_all_by_label("Wall").next().is_some(),
        "disabled authoring command should stay discoverable"
    );

    harness.key_press(egui::Key::Enter);
    harness.run();

    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Plan);
    assert!(!harness.state().draw_wall_tool.active);
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

#[test]
fn command_search_escape_closes_before_active_wall_tool() {
    let mut harness = demo_harness();
    harness.run();

    harness.key_press(egui::Key::W);
    harness.run();
    assert!(harness.state().draw_wall_tool.active);

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run();
    assert!(harness.state().command_search.open);

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().command_search.open);
    assert!(
        harness.state().draw_wall_tool.active,
        "Escape should dismiss the topmost command palette before cancelling tools"
    );

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(
        !harness.state().draw_wall_tool.active,
        "Escape should still cancel the wall tool once the palette is closed"
    );
}

#[test]
fn command_search_suppresses_canvas_context_toolbar() {
    let mut harness = demo_harness();
    harness.run();
    assert!(
        harness.query_all_by_label("Delete").next().is_some(),
        "selected wall should expose its canvas context toolbar"
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run();

    assert!(harness.state().command_search.open);
    assert!(
        harness.query_all_by_label("Delete").next().is_none(),
        "global command search should hide canvas-local context actions underneath it"
    );
}

#[test]
fn visibility_popup_escape_preserves_selection_and_isolation_then_exit_remains_reachable() {
    let mut harness = demo_harness();
    harness.run();
    harness.state_mut().set_workspace_mode(WorkspaceMode::Plan);
    harness.state_mut().viewport_mode = ViewportMode::Axonometric;
    harness.state_mut().execute_action(ActionId::IsolateDim);
    harness.run();

    assert_accessible_button(&harness, "Component visibility", "3D context toolbar");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Component visibility")
        .click();
    harness.run();
    assert_accessible_button(&harness, "Exit Isolation", "component visibility popup");

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(harness.state().selected_component_count(), 1);
    assert_eq!(
        harness.state().component_visibility.isolation_mode(),
        Some(IsolationMode::DimOthers)
    );
    assert!(
        harness
            .query_all_by_label("Exit Isolation")
            .next()
            .is_none()
    );

    harness.state_mut().clear_selection();
    harness.run();
    assert_accessible_button(
        &harness,
        "Component visibility",
        "isolation recovery context toolbar",
    );
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Component visibility")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Exit Isolation")
        .click();
    harness.run();
    assert_eq!(harness.state().component_visibility.isolation_mode(), None);
}

#[test]
fn disabled_command_reasons_follow_enabled_context() {
    let mut harness = demo_harness();
    harness.run();

    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert_eq!(
        harness
            .state()
            .action_disabled_reason(ActionId::ExportArtifacts),
        Some("Available in the Plan workspace")
    );
    assert_eq!(
        harness
            .state()
            .action_disabled_reason(ActionId::ExportComplianceReport),
        Some("Available in the Plan workspace")
    );

    harness.get_by_label("Plan").click();
    harness.run();
    assert!(harness.state().action_enabled(ActionId::ExportArtifacts));
    assert_eq!(
        harness
            .state()
            .action_disabled_reason(ActionId::ExportArtifacts),
        None
    );
    assert!(!harness.state().action_enabled(ActionId::ToolWall));
    assert_eq!(
        harness.state().action_disabled_reason(ActionId::ToolWall),
        Some("Available in an authoring workflow tab; Render and Plan are output workspaces")
    );
}

#[test]
fn command_strip_buttons_follow_enabled_context() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();

    harness.state_mut().workspace_mode = WorkspaceMode::Plan;
    harness.state_mut().command_tab = actions::WorkflowTab::Frame;
    harness.run();

    assert!(!harness.state().action_enabled(ActionId::ToolWall));
    assert_eq!(
        harness.state().action_disabled_reason(ActionId::ToolWall),
        Some("Available in an authoring workflow tab; Render and Plan are output workspaces")
    );

    let wall_button = harness
        .query_all_by_role_and_label(Role::Button, "Wall")
        .next()
        .expect("forced Frame command strip should render the Wall command");
    wall_button.click();
    harness.run();

    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Frame);
    assert!(
        !harness.state().draw_wall_tool.active,
        "disabled Wall command-strip button must not activate"
    );
}

/// The visible workspace switch lives in the workflow strip's generated output
/// tab, while view switching stays in the workspace/view bar.
#[test]
fn workspace_view_bar_owns_workspace_and_view_controls() {
    let mut harness = demo_harness();
    harness.run();

    for label in ["Shell", "Wall", "Roof", "3D"] {
        assert!(
            harness.query_all_by_label(label).next().is_some(),
            "workspace/view bar should expose '{label}'"
        );
    }
    for removed_switcher in ["Design Workspace", "Plan Workspace"] {
        assert!(
            harness
                .query_all_by_label(removed_switcher)
                .next()
                .is_none(),
            "workspace/view bar should not expose the removed '{removed_switcher}' switcher"
        );
    }
    assert!(
        harness.query_all_by_label("View").next().is_none(),
        "workflow command strip should no longer expose a View panel"
    );

    harness.get_by_label("Plan").click();
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

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    harness.run();
    harness.state_mut().command_search.query = "design workspace".to_owned();
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Frame);

    harness.get_by_label("Shell").click();
    harness.run();
    assert_eq!(harness.state().viewport_mode, ViewportMode::Plan);

    harness.get_by_label("Render").click();
    harness.run_steps(1);
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Render);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Render);
    assert_eq!(harness.state().viewport_mode, ViewportMode::Render);
    for hidden_view in ["Shell", "Roof", "3D"] {
        assert!(
            harness.query_all_by_label(hidden_view).next().is_none(),
            "Render workspace should hide the authoring view tab '{hidden_view}'"
        );
    }

    harness.state_mut().execute_action(ActionId::ViewRoof);
    harness.run_steps(1);
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Design);
    assert_eq!(harness.state().viewport_mode, ViewportMode::RoofPlan);

    harness.get_by_label("3D").click();
    harness.run_steps(1);
    assert_eq!(harness.state().viewport_mode, ViewportMode::Axonometric);
}

#[test]
fn tiled_viewport_builtins_expose_bounded_pane_chrome_at_narrow_size() {
    use eframe::egui::accesskit::Role;

    let size = egui::vec2(1040.0, 680.0);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    for (preset, expected_modes) in [
        (
            BuiltInPreset::PlanAnd3d,
            vec![ViewportMode::Plan, ViewportMode::Axonometric],
        ),
        (
            BuiltInPreset::FourUp,
            vec![
                ViewportMode::Plan,
                ViewportMode::Elevation,
                ViewportMode::Axonometric,
                ViewportMode::Axonometric,
            ],
        ),
    ] {
        let mut harness = demo_harness_with_size(size);
        harness.run();
        harness
            .state_mut()
            .viewport_workspace
            .apply_builtin(preset)
            .expect("built-in viewport layout should instantiate");
        harness.run();

        let panes = {
            let layout = &harness.state().viewport_workspace.layout;
            layout
                .pane_ids()
                .into_iter()
                .map(|id| {
                    (
                        id,
                        layout
                            .pane(id)
                            .expect("layout pane ID should resolve")
                            .config()
                            .mode(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            panes.iter().map(|(_, mode)| *mode).collect::<Vec<_>>(),
            expected_modes,
            "{} should expose its locked pane modes",
            preset.name()
        );

        for (id, _) in &panes {
            let label = format!("View {}", id.get());
            let rect = harness.get_by_label(&label).rect();
            assert!(
                rect.left() >= window.left()
                    && rect.top() >= window.top()
                    && rect.right() <= window.right()
                    && rect.bottom() <= window.bottom(),
                "{} pane header '{label}' should stay inside {size:?}, got {rect:?}",
                preset.name()
            );
        }

        for (label, expected_count) in [
            (
                "Plan",
                expected_modes
                    .iter()
                    .filter(|mode| **mode == ViewportMode::Plan)
                    .count(),
            ),
            (
                "Elevation",
                expected_modes
                    .iter()
                    .filter(|mode| **mode == ViewportMode::Elevation)
                    .count(),
            ),
            (
                "3D",
                expected_modes
                    .iter()
                    .filter(|mode| **mode == ViewportMode::Axonometric)
                    .count(),
            ),
        ] {
            let mode_controls = harness
                .query_all_by_role(Role::ComboBox)
                .filter(|node| node.value().as_deref() == Some(label))
                .collect::<Vec<_>>();
            assert_eq!(
                mode_controls.len(),
                expected_count,
                "{} should expose one mode control valued '{label}' per matching pane",
                preset.name()
            );
            for control in mode_controls {
                let rect = control.rect();
                assert!(
                    rect.left() >= window.left()
                        && rect.top() >= window.top()
                        && rect.right() <= window.right()
                        && rect.bottom() <= window.bottom(),
                    "{} mode control '{label}' should stay inside {size:?}, got {rect:?}",
                    preset.name()
                );
            }
        }

        for (id, _) in &panes {
            let action_menu_label = format!("View {} viewport actions", id.get());
            let menu = harness.get_by_role_and_label(Role::Button, &action_menu_label);
            let rect = menu.rect();
            assert!(
                rect.left() >= window.left()
                    && rect.top() >= window.top()
                    && rect.right() <= window.right()
                    && rect.bottom() <= window.bottom(),
                "{} viewport action menu should stay inside {size:?}, got {rect:?}",
                preset.name()
            );
        }
    }
}

#[test]
fn render_workspace_applying_design_and_render_keeps_render_command_context() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();

    harness.get_by_label("Render").click();
    harness.run_steps(1);
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Render);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Render);

    harness
        .get_by_role_and_label(Role::Button, "Layouts")
        .click();
    harness.run_steps(1);
    harness
        .get_by_role_and_label(Role::Button, "Design + Render")
        .click();
    harness.run_steps(1);

    let app = harness.state();
    let modes = app
        .viewport_workspace
        .layout
        .pane_ids()
        .into_iter()
        .map(|id| {
            app.viewport_workspace
                .layout
                .pane(id)
                .expect("layout pane ID should resolve")
                .config()
                .mode()
        })
        .collect::<Vec<_>>();
    assert_eq!(modes, vec![ViewportMode::Axonometric, ViewportMode::Render]);
    assert_eq!(app.workspace_mode, WorkspaceMode::Render);
    assert_eq!(app.command_tab, actions::WorkflowTab::Render);
    assert_eq!(app.viewport_workspace.active_mode(), ViewportMode::Render);
    assert_eq!(app.viewport_mode, ViewportMode::Render);
}

#[test]
fn render_workspace_applying_saved_authoring_layout_restores_it_on_exit() {
    use eframe::egui::accesskit::Role;

    const PRESET_NAME: &str = "Plan focus";

    let mut harness = demo_harness();
    harness.run();
    assert_eq!(
        harness.state().viewport_workspace.active_mode(),
        ViewportMode::Plan
    );

    harness
        .get_by_role_and_label(Role::Button, "Layouts")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, "Save current layout…")
        .click();
    harness.run_steps(1);

    let dialog = harness.get_by_role_and_label(Role::Window, "Save viewport layout");
    dialog.get_by_role(Role::TextInput).focus();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(PRESET_NAME.to_owned()));
    harness.run_steps(1);
    let dialog = harness.get_by_role_and_label(Role::Window, "Save viewport layout");
    assert_eq!(
        dialog.get_by_role(Role::TextInput).value().as_deref(),
        Some(PRESET_NAME)
    );
    harness
        .get_by_role_and_label(Role::Window, "Save viewport layout")
        .get_by_role_and_label(Role::Button, "Save")
        .click();
    harness.run_steps(1);
    assert!(
        !harness.state().viewport_workspace.save_preset_open,
        "Save should close the custom-preset dialog"
    );

    harness.get_by_label("Render").click();
    harness.run_steps(1);
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Render);

    harness
        .get_by_role_and_label(Role::Button, "Layouts")
        .click();
    harness.run_steps(1);
    harness
        .get_by_role_and_label(Role::Button, PRESET_NAME)
        .click();
    harness.run_steps(1);

    let app = harness.state();
    assert_eq!(app.viewport_workspace.layout.pane_count(), 1);
    assert_eq!(app.workspace_mode, WorkspaceMode::Render);
    assert_eq!(app.command_tab, actions::WorkflowTab::Render);
    assert_eq!(app.viewport_workspace.active_mode(), ViewportMode::Render);
    assert_eq!(app.viewport_mode, ViewportMode::Render);

    harness.get_by_label("Frame").click();
    harness.run_steps(1);
    let app = harness.state();
    assert_eq!(app.workspace_mode, WorkspaceMode::Design);
    assert_eq!(app.command_tab, actions::WorkflowTab::Frame);
    assert_eq!(app.viewport_workspace.active_mode(), ViewportMode::Plan);
    assert_eq!(app.viewport_mode, ViewportMode::Plan);
}

#[test]
fn tiled_viewport_maximum_balanced_layout_keeps_compact_chrome_reachable() {
    use eframe::egui::accesskit::Role;

    let size = egui::vec2(1040.0, 680.0);
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let mut harness = demo_harness_with_size(size);
    harness.run();
    harness
        .state_mut()
        .viewport_workspace
        .apply_builtin(BuiltInPreset::FourUp)
        .unwrap();
    let quadrant_ids = harness.state().viewport_workspace.layout.pane_ids();
    for id in quadrant_ids {
        harness
            .state_mut()
            .viewport_workspace
            .split(id, super::viewport::SplitAxis::Horizontal)
            .unwrap();
    }
    let column_ids = harness.state().viewport_workspace.layout.pane_ids();
    for id in column_ids {
        harness
            .state_mut()
            .viewport_workspace
            .split(id, super::viewport::SplitAxis::Vertical)
            .unwrap();
    }
    harness.run();

    let pane_ids = harness.state().viewport_workspace.layout.pane_ids();
    assert_eq!(pane_ids.len(), 16);
    for id in pane_ids {
        for (role, label) in [
            (Role::ComboBox, format!("View {} mode", id.get())),
            (Role::Button, format!("View {} viewport actions", id.get())),
        ] {
            let rect = harness.get_by_role_and_label(role, &label).rect();
            assert!(
                rect.left() >= window.left()
                    && rect.top() >= window.top()
                    && rect.right() <= window.right()
                    && rect.bottom() <= window.bottom(),
                "compact pane control '{label}' should stay inside {size:?}, got {rect:?}"
            );
        }
    }

    let splitters = harness
        .query_all_by_role_and_label(Role::Slider, "Horizontal viewport split divider")
        .count()
        + harness
            .query_all_by_role_and_label(Role::Slider, "Vertical viewport split divider")
            .count();
    assert_eq!(
        splitters, 15,
        "every split should expose an accessible divider"
    );
}

#[test]
fn tiled_viewport_mode_control_activates_its_pane_and_switches_only_that_pane() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();
    harness
        .state_mut()
        .viewport_workspace
        .apply_builtin(BuiltInPreset::PlanAnd3d)
        .unwrap();
    harness.run();

    let pane_ids = harness.state().viewport_workspace.layout.pane_ids();
    let [plan_pane, view_3d_pane] = pane_ids.as_slice() else {
        panic!("Plan + 3D should produce exactly two panes");
    };
    assert_eq!(harness.state().viewport_workspace.active_id(), *plan_pane);

    harness
        .query_all_by_role(Role::ComboBox)
        .find(|node| node.value().as_deref() == Some("3D"))
        .expect("3D pane should expose its current mode through the combobox value")
        .click();
    harness.run();
    assert_eq!(
        harness.state().viewport_workspace.active_id(),
        *view_3d_pane,
        "opening a pane-local mode control should activate its source pane"
    );

    harness
        .get_by_role_and_label(Role::Button, "Elevation")
        .click();
    harness.run();

    let app = harness.state();
    assert_eq!(app.viewport_workspace.active_id(), *view_3d_pane);
    assert_eq!(app.viewport_mode, ViewportMode::Elevation);
    assert_eq!(
        app.viewport_workspace
            .layout
            .pane(*view_3d_pane)
            .unwrap()
            .config()
            .mode(),
        ViewportMode::Elevation
    );
    assert_eq!(
        app.viewport_workspace
            .layout
            .pane(*plan_pane)
            .unwrap()
            .config()
            .mode(),
        ViewportMode::Plan,
        "switching one pane must not replace its sibling's mode"
    );
}

#[test]
fn tiled_viewport_split_control_and_last_pane_close_guard_drive_layout_state() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();
    let only_pane = harness.state().viewport_workspace.active_id();
    let only_pane_actions = format!("View {} viewport actions", only_pane.get());

    harness
        .get_by_role_and_label(Role::Button, &only_pane_actions)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, "Close viewport")
        .click();
    harness.run_steps(1);
    assert_eq!(harness.state().viewport_workspace.layout.pane_count(), 1);
    assert_eq!(harness.state().viewport_workspace.active_id(), only_pane);
    assert_eq!(
        harness.state().file_status.as_deref(),
        Some("the last viewport pane cannot close")
    );

    harness
        .get_by_role_and_label(Role::Button, &only_pane_actions)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, "Split left / right")
        .click();
    harness.run_steps(1);
    assert_eq!(harness.state().viewport_workspace.layout.pane_count(), 2);
    harness.run_steps(1);
    let pane_ids = harness.state().viewport_workspace.layout.pane_ids();
    assert_eq!(pane_ids.len(), 2);
    for id in pane_ids {
        harness.get_by_role_and_label(Role::Button, &format!("View {} viewport actions", id.get()));
    }
}

#[test]
fn tiled_viewport_closing_the_active_pane_preserves_the_surviving_view_type() {
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();
    harness
        .state_mut()
        .viewport_workspace
        .apply_builtin(BuiltInPreset::PlanAnd3d)
        .unwrap();
    harness.run();

    let plan_pane = harness.state().viewport_workspace.active_id();
    assert_eq!(
        harness.state().viewport_workspace.active_mode(),
        ViewportMode::Plan
    );
    harness
        .get_by_role_and_label(
            Role::Button,
            &format!("View {} viewport actions", plan_pane.get()),
        )
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, "Close viewport")
        .click();
    harness.run_steps(2);

    let app = harness.state();
    assert_eq!(app.viewport_workspace.layout.pane_count(), 1);
    assert_eq!(
        app.viewport_workspace.active_mode(),
        ViewportMode::Axonometric
    );
    assert_eq!(app.viewport_mode, ViewportMode::Axonometric);
}

#[test]
fn tiled_viewport_pop_out_control_registers_a_deferred_viewport_output() {
    use eframe::egui::ViewportId;
    use eframe::egui::accesskit::Role;

    let mut harness = demo_harness();
    harness.run();
    let pane_id = harness.state().viewport_workspace.active_id();
    let pane_actions = format!("View {} viewport actions", pane_id.get());

    harness
        .get_by_role_and_label(Role::Button, &pane_actions)
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, "Pop out viewport")
        .click();
    harness.run_steps(1);

    assert!(
        harness
            .state()
            .viewport_workspace
            .layout
            .pane(pane_id)
            .unwrap()
            .config()
            .is_popped_out()
    );
    let has_native_deferred_output = harness
        .output()
        .viewport_output
        .keys()
        .any(|id| *id != ViewportId::ROOT);
    let embedded_header = format!("Pane {}", pane_id.get());
    let has_embedded_callback = harness
        .query_all_by_label(&embedded_header)
        .next()
        .is_some();
    assert!(
        has_native_deferred_output || has_embedded_callback,
        "pop-out should produce a native deferred viewport or egui's embedded callback fallback"
    );
    if has_embedded_callback {
        let dock_label = format!("Dock pane {}", pane_id.get());
        assert_accessible_button(&harness, &dock_label, "embedded deferred viewport");
        harness
            .get_by_role_and_label(Role::Button, &dock_label)
            .click();
        harness.run_steps(2);
        assert!(
            !harness
                .state()
                .viewport_workspace
                .layout
                .pane(pane_id)
                .unwrap()
                .config()
                .is_popped_out(),
            "the embedded deferred Dock control should reduce through the root event queue"
        );
    }
}

#[test]
fn transient_status_toasts_do_not_reflow_main_panels() {
    let mut harness = demo_harness();
    harness.run();

    let model_browser_y = harness.get_by_label("Model Browser").rect().top();
    harness.state_mut().file_status = Some("Created new project".to_owned());
    harness.state_mut().artifact_status = Some("Exported plan artifacts".to_owned());
    harness.state_mut().dimension_status = Some("Pick two anchors, then place".to_owned());
    harness.run();

    assert!(
        harness
            .query_all_by_label("Created new project")
            .next()
            .is_some(),
        "full file status should remain visible as a canvas toast"
    );
    assert!(
        harness
            .query_all_by_label("Exported plan artifacts")
            .next()
            .is_some(),
        "artifact status should remain visible as a canvas toast"
    );
    assert!(
        harness
            .query_all_by_label("Pick two anchors, then place")
            .next()
            .is_some(),
        "tool guidance should remain visible as a canvas toast"
    );

    let shifted_y = harness.get_by_label("Model Browser").rect().top();
    assert!(
        (shifted_y - model_browser_y).abs() <= 0.5,
        "transient statuses must not change the main panel top edge"
    );

    harness.state_mut().status_toast_until = -1.0;
    harness.run_steps(1);
    assert!(
        harness
            .query_all_by_label("Created new project")
            .next()
            .is_none(),
        "status toast should auto-dismiss without clearing the underlying status"
    );
    assert_eq!(
        harness.state().file_status.as_deref(),
        Some("Created new project")
    );
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

#[test]
fn inspector_has_friendly_empty_selection_state() {
    let mut harness = demo_harness();
    harness.run();

    harness.state_mut().selected = Selection::None;
    harness.run();

    assert_accessible_label(&harness, "No selection", "inspector empty state");
    assert_accessible_label(
        &harness,
        "Select an object to edit its properties.",
        "inspector empty state",
    );
}

/// Tool shortcuts entered from an output workspace must stay disabled; leaving
/// an output workspace is an explicit workflow-tab choice.
#[test]
fn tool_shortcut_from_plan_tab_does_not_activate_authoring_tool() {
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
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Plan);
    assert!(
        !harness.state().draw_wall_tool.active,
        "W should stay disabled from output workspaces"
    );
}

/// Dimension shortcuts entered from generated-plan workflow must remain disabled
/// instead of leaving the output workspace implicitly.
#[test]
fn dimension_shortcut_from_plan_tab_does_not_activate_authoring_tool() {
    let mut harness = demo_harness();
    harness.run();

    harness.get_by_label("Plan").click();
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Plan);

    harness.key_press(egui::Key::D);
    harness.run();
    assert_eq!(harness.state().workspace_mode, WorkspaceMode::Plan);
    assert_eq!(harness.state().command_tab, actions::WorkflowTab::Plan);
    assert!(
        !harness.state().dimension_tool.active,
        "D should stay disabled from output workspaces"
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

/// A fresh app shows walls as outlines with noisy corner labels off by default
/// so a loaded shell reads as a clean line drawing first.
#[test]
fn layers_default_to_outline_with_corner_labels_quiet() {
    let mut harness = demo_harness();
    harness.run();
    let layers = harness.state().layers;
    assert_eq!(layers.wall_display, WallDisplay::Outline);
    assert!(
        layers.grid && layers.rooms && layers.wall_labels,
        "primary drafting layers should default on"
    );
    assert!(
        !layers.joins,
        "corner labels should be opt-in unless hovered or selected"
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

#[test]
fn axonometric_secondary_click_opens_context_menu_and_dispatches_isolation() {
    let mut harness = demo_harness();
    harness.run();
    harness.state_mut().set_workspace_mode(WorkspaceMode::Plan);
    harness.state_mut().viewport_mode = ViewportMode::Axonometric;
    harness.run();

    secondary_click_pickable_3d_component(&mut harness);

    assert_accessible_button(&harness, "Isolate ⏵", "3D selection context menu");
    harness.get_by_label("Isolate ⏵").click();
    harness.run();
    assert_accessible_button(&harness, "Dim Others", "Isolate submenu");
    assert_accessible_button(&harness, "Hide Others", "Isolate submenu");

    harness.get_by_label("Dim Others").click();
    harness.run();
    assert_eq!(
        harness.state().component_visibility.isolation_mode(),
        Some(IsolationMode::DimOthers)
    );

    let selected = harness.state().selected_components();
    secondary_click_pickable_3d_component(&mut harness);
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(harness.state().selected_components(), selected);
    assert_eq!(
        harness.state().component_visibility.isolation_mode(),
        Some(IsolationMode::DimOthers),
        "Escape should dismiss the popup before changing isolation or selection"
    );
    assert_eq!(harness.state().context_menu_context, None);
    assert!(harness.query_all_by_label("Isolate ⏵").next().is_none());

    let viewport = harness.get_by_label("3D viewport").rect();
    let empty = viewport.left_top() + egui::vec2(2.0, 2.0);
    harness.event(egui::Event::PointerMoved(empty));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: empty,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run();
    assert!(harness.query_all_by_label("Isolate ⏵").next().is_none());
    assert_eq!(harness.state().selected_components(), selected);
}

#[test]
fn tiled_context_menu_click_over_sibling_keeps_source_pane_active() {
    let mut harness = demo_harness();
    harness.run();
    let source = {
        let app = harness.state_mut();
        app.set_workspace_mode(WorkspaceMode::Plan);
        app.viewport_workspace
            .apply_builtin(BuiltInPreset::PlanAnd3d)
            .unwrap();
        let panes = app.viewport_workspace.layout.pane_ids();
        let source = panes[0];
        let sibling = panes[1];
        app.viewport_workspace
            .set_mode(source, ViewportMode::Axonometric)
            .unwrap();
        app.viewport_workspace
            .set_mode(sibling, ViewportMode::Plan)
            .unwrap();
        app.viewport_workspace.set_active(source).unwrap();
        app.viewport_workspace
            .layout
            .set_split_ratio(&[], 0.18)
            .unwrap();
        app.viewport_mode = ViewportMode::Axonometric;
        source
    };
    harness.run();

    secondary_click_pickable_3d_component_at_xs(
        &mut harness,
        &[0.95, 0.875, 0.75, 0.625, 0.5, 0.375, 0.25],
    );
    let source_rect = harness.get_by_label("3D viewport").rect();
    let isolate = harness.get_by_label("Isolate ⏵");
    assert!(
        isolate.rect().center().x > source_rect.right(),
        "the regression requires the popup action to overlap the sibling pane"
    );

    isolate.click();
    harness.run();
    assert_eq!(harness.state().viewport_workspace.active_id(), source);
    harness.get_by_label("Dim Others").click();
    harness.run();
    assert_eq!(harness.state().viewport_workspace.active_id(), source);
    assert_eq!(
        harness.state().component_visibility.isolation_mode(),
        Some(IsolationMode::DimOthers)
    );
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
