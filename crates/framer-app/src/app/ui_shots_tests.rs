//! Off-screen UI screenshot suite ("ui-shots"), driven by `egui_kittest`.
//!
//! Renders the real `FramerApp` through a scripted deck of states — every
//! workflow tab, every view, common selections, menus, the command palette,
//! and both themes — and writes PNGs to `target/ui-shots/` (override with
//! `UI_SHOTS_DIR`). This replaces the slow install-the-app-and-drive-it loop
//! for visual review: no app bundle, no window, no screen-capture permissions,
//! and the states are reproduced deterministically via the AccessKit tree.
//!
//! Run it with `scripts/ui-shots.sh`, or directly:
//!
//! ```sh
//! cargo test -p framer-app ui_shots -- --ignored --nocapture
//! ```
//!
//! The test is `#[ignore]`d so the normal `cargo test --workspace` gate stays
//! GPU-free; rendering here needs a wgpu adapter (Metal locally, lavapipe on
//! CI — the same environments as `tests/gpu_parity.rs`).
//!
//! Unlike `ui_harness_tests`, this suite asserts nothing about pixels: it is a
//! camera, not a regression net. Promoting a curated subset to
//! `harness.snapshot()` goldens is future work (see
//! docs/plans/2026-07-07-ui-ux-hardening.md).

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui;
use eframe::wgpu;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use framer_core::{ElementId, Length, OpeningKind, Point2};
use framer_geometry::{BodyKind, GeometryViolation};
use framer_solver::MemberKind;

use super::viewport::{BuiltInPreset, SplitAxis};
use super::{
    AuthoredComponentKind, ComponentKey, FramerApp, RoofForm, Selection, SelectionOp, ViewportMode,
    WallDisplay, actions, design, panels,
};

fn shots_dir() -> PathBuf {
    match std::env::var_os("UI_SHOTS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-shots"),
    }
}

/// A wgpu-rendering harness around the demo-shell app, mirroring
/// `ui_harness_tests::demo_harness` (same font warm-up first frame, same
/// default desktop size) plus a renderer so frames can be rasterized.
fn shots_harness(theme: design::Theme) -> Harness<'static, FramerApp> {
    shots_harness_with_size(theme, egui::vec2(1360.0, 860.0))
}

fn shots_harness_with_size(theme: design::Theme, size: egui::Vec2) -> Harness<'static, FramerApp> {
    let mut fonts_bound = false;
    Harness::builder()
        .with_size(size)
        .with_max_steps(16)
        .wgpu()
        .build_ui_state(
            move |ui, app: &mut FramerApp| {
                if !fonts_bound {
                    design::install(ui.ctx(), theme);
                    fonts_bound = true;
                    return;
                }
                app.handle_keyboard_shortcuts(ui.ctx());
                app.ui_root(ui);
            },
            FramerApp {
                gpu_target_format: Some(wgpu::TextureFormat::Rgba8Unorm),
                ..FramerApp::default()
            },
        )
}

/// Renders the current harness state to `<dir>/<index>-<name>.png`.
///
/// `run_ok` (not `run`) settles pending frames: continuously-repainting views
/// (Render's progressive path tracer) never go idle and would panic `run`.
fn shot(harness: &mut Harness<'_, FramerApp>, dir: &Path, index: &mut u32, name: &str) {
    harness.run_ok();
    let image = harness.render().unwrap_or_else(|error| {
        panic!(
            "ui-shots could not render a frame: {error}. This suite needs a wgpu \
             adapter (Metal locally, lavapipe on CI) — see scripts/ui-shots.sh."
        )
    });
    let path = dir.join(format!("{index:02}-{name}.png"));
    image
        .save(&path)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
    println!("ui-shots: wrote {}", path.display());
    *index += 1;
}

/// Let the progressive Render view publish at least one frame before capturing.
/// The off-screen deck is visual QA, not a convergence benchmark, so keep the
/// camera in low-resolution preview mode and stop as soon as either renderer has
/// an image.
fn warm_render(harness: &mut Harness<'_, FramerApp>) {
    {
        let runtime = harness.state().viewport_workspace.active_runtime();
        let mut runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime.render.motion_cooldown = 24;
    }
    for _ in 0..80 {
        harness.run_steps(1);
        let has_samples = {
            let runtime = harness.state().viewport_workspace.active_runtime();
            let runtime = runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            runtime.render.cpu.samples() > 0 || runtime.render.gpu.samples() > 0
        };
        if has_samples {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Selects a workflow tab through the app's own toolbar handler (tab + coupled
/// workspace). Clicking tab labels through AccessKit would be ambiguous —
/// "Design" also appears as a panel badge and in the status bar.
fn select_tab(harness: &mut Harness<'_, FramerApp>, tab: actions::WorkflowTab) {
    harness.state_mut().select_workflow_tab(tab);
}

fn sync_active_viewport_mode(harness: &mut Harness<'_, FramerApp>) {
    let mode = harness.state().viewport_workspace.active_mode();
    harness.state_mut().viewport_mode = mode;
}

fn open_3d_component_context_menu(harness: &mut Harness<'_, FramerApp>) {
    let rect = harness.get_by_label("3D viewport").rect();
    for y in [0.25, 0.375, 0.5, 0.625, 0.75] {
        for x in [0.25, 0.375, 0.5, 0.625, 0.75] {
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
            harness.run_ok();
            if harness.state().context_menu_context.is_some() {
                return;
            }
        }
    }
    panic!("the demo-shell 3D viewport should expose at least one pickable component");
}

fn prepare_geometry_overlap(harness: &mut Harness<'_, FramerApp>) {
    let mut wall = harness.state().model.walls[0].clone();
    wall.id = ElementId::new("ui-shot-overlap-wall");
    wall.name = "Overlap wall".to_owned();
    wall.start.x += Length::from_whole_inches(12);
    wall.end.x += Length::from_whole_inches(12);
    wall.openings.clear();
    wall.dimensions.clear();
    harness.state_mut().model.walls.push(wall);
    harness.state_mut().rebuild();
    let violation = harness
        .state()
        .geometry_audit
        .violations
        .iter()
        .find(|violation| {
            matches!(violation, GeometryViolation::Overlap(_))
                && matches!(violation.body_a().kind(), BodyKind::Assembly(_))
        })
        .expect("overlap shot fixture should produce an assembly overlap")
        .clone();
    harness
        .state_mut()
        .focus_diagnostic(panels::DiagnosticAction::Geometry(violation));
    harness.state_mut().layers.wall_display = WallDisplay::Full;
    harness.ctx.request_repaint();
    harness.run_steps(2);
}

/// Reproduce the adjacent-room authoring path that originally left the top and
/// bottom perimeter as separate wall/framing sections. The two collinear draw
/// gestures should now extend the existing demo-shell walls; only the new outer
/// wall is added.
fn prepare_adjacent_room(harness: &mut Harness<'_, FramerApp>) {
    let point = |x: f64, y: f64| Point2::new(Length::from_feet(x), Length::from_feet(y));
    let walls_before = harness.state().model.walls.len();
    harness
        .state_mut()
        .add_wall(point(28.0, 20.0), point(40.0, 20.0));
    harness
        .state_mut()
        .add_wall(point(40.0, 20.0), point(40.0, 0.0));
    harness
        .state_mut()
        .add_wall(point(40.0, 0.0), point(28.0, 0.0));
    harness.state_mut().add_room(point(34.0, 10.0));

    assert_eq!(
        harness.state().model.walls.len(),
        walls_before + 1,
        "only the new outer wall should be authored"
    );
    assert_eq!(framer_core::enclosed_room_count(&harness.state().model), 2);
    harness.state_mut().selected = Selection::None;
    harness.state_mut().layers.wall_display = WallDisplay::Full;
    harness.ctx.request_repaint();
    harness.run_steps(2);
}

#[test]
#[ignore = "renders frames via wgpu; run scripts/ui-shots.sh (or pass -- --ignored)"]
fn capture_ui_shot_deck() {
    let dir = shots_dir();
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", dir.display()));
    // Stale shots from a previous run would silently mix into the new deck
    // (names shift as states are added/removed), so start clean.
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "png") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let mut index = 1u32;

    let mut harness = shots_harness(design::studio_light());
    harness.run_ok();

    // Workflow tabs, including the output workspaces.
    for (tab, name) in [
        (actions::WorkflowTab::Frame, "frame-shell"),
        (actions::WorkflowTab::Design, "design-tab"),
        (actions::WorkflowTab::Openings, "openings-tab"),
        (actions::WorkflowTab::Roofs, "roofs-tab"),
        (actions::WorkflowTab::Annotate, "annotate-tab"),
        (actions::WorkflowTab::Render, "render-workspace"),
        (actions::WorkflowTab::Plan, "plan-workspace"),
    ] {
        select_tab(&mut harness, tab);
        if tab == actions::WorkflowTab::Render {
            warm_render(&mut harness);
        }
        shot(&mut harness, &dir, &mut index, name);
    }

    // Tiled workspace checkpoints: mixed two-up, repeated-type Four Up with
    // distinct canonical 3D angles, and the named-layout menu.
    harness
        .state_mut()
        .viewport_workspace
        .apply_builtin(BuiltInPreset::PlanAnd3d)
        .unwrap();
    sync_active_viewport_mode(&mut harness);
    shot(&mut harness, &dir, &mut index, "viewport-plan-and-3d");
    harness
        .state_mut()
        .viewport_workspace
        .apply_builtin(BuiltInPreset::FourUp)
        .unwrap();
    sync_active_viewport_mode(&mut harness);
    shot(
        &mut harness,
        &dir,
        &mut index,
        "viewport-four-up-distinct-3d",
    );
    harness
        .state_mut()
        .viewport_workspace
        .save_named_preset("Inspection desk")
        .unwrap();
    harness.get_by_label("Layouts").click();
    shot(
        &mut harness,
        &dir,
        &mut index,
        "viewport-layout-presets-menu",
    );
    harness.key_press(egui::Key::Escape);
    harness
        .state_mut()
        .viewport_workspace
        .apply_builtin(BuiltInPreset::Focus)
        .unwrap();
    sync_active_viewport_mode(&mut harness);
    harness.run_ok();

    // Transient statuses are canvas toasts, not toolbar rows, so capture them
    // once before clearing the state for the rest of the deck.
    harness.state_mut().file_status = Some("Reset to multi-wall demo shell".to_owned());
    harness.state_mut().artifact_status = Some("Exported plan artifacts".to_owned());
    harness.state_mut().dimension_status = Some("Pick two anchors, then place".to_owned());
    shot(&mut harness, &dir, &mut index, "status-toast");
    harness.state_mut().file_status = None;
    harness.state_mut().artifact_status = None;
    harness.state_mut().dimension_status = None;
    harness.run_ok();

    // Selection states (back in the Frame tab): wall, opening, corner — the
    // three inspector layouts. Select by state so the deck stays deterministic
    // even when tree names intentionally match canvas labels.
    select_tab(&mut harness, actions::WorkflowTab::Frame);
    harness.run_ok();
    let back_wall_index = harness
        .state()
        .model
        .walls
        .iter()
        .position(|wall| wall.id.0 == "wall-back")
        .expect("demo shell has a back wall");
    for (selection, name) in [
        (Selection::Wall, "wall-selected"),
        (
            Selection::Opening("opening-back-left-window".to_owned()),
            "opening-selected",
        ),
        (
            Selection::Join("join-back-left".to_owned()),
            "corner-selected",
        ),
    ] {
        harness.state_mut().selected_wall = back_wall_index;
        harness.state_mut().selected = selection;
        shot(&mut harness, &dir, &mut index, name);
    }
    harness.state_mut().selected = Selection::None;
    shot(&mut harness, &dir, &mut index, "empty-selection");
    harness.state_mut().layers.joins = true;
    shot(&mut harness, &dir, &mut index, "corner-labels-layer-on");
    harness.state_mut().layers.joins = false;

    // Views, with the back wall selected for the elevation.
    harness.state_mut().selected_wall = back_wall_index;
    harness.state_mut().selected = Selection::Wall;
    harness.run_ok();
    for (mode, name) in [
        (ViewportMode::Elevation, "wall-elevation-view"),
        (ViewportMode::RoofPlan, "roof-view"),
        (ViewportMode::Axonometric, "3d-view"),
    ] {
        harness.state_mut().viewport_mode = mode;
        shot(&mut harness, &dir, &mut index, name);
    }
    harness.state_mut().viewport_mode = ViewportMode::Plan;

    // Component presentation states in generated Plan 3-D: ordered multi-selection,
    // both isolation treatments, the command menu, a reversible hidden browser row,
    // and one semantic rough-opening framing group.
    select_tab(&mut harness, actions::WorkflowTab::Plan);
    harness.state_mut().viewport_mode = ViewportMode::Axonometric;
    let second_wall_index = (back_wall_index + 1) % harness.state().model.walls.len();
    harness.state_mut().apply_selection(
        Selection::Wall,
        Some(back_wall_index),
        SelectionOp::Replace,
    );
    harness.state_mut().apply_selection(
        Selection::Wall,
        Some(second_wall_index),
        SelectionOp::Toggle,
    );
    shot(
        &mut harness,
        &dir,
        &mut index,
        "plan-3d-multi-selected-walls",
    );

    harness
        .state_mut()
        .execute_action(actions::ActionId::IsolateDim);
    shot(&mut harness, &dir, &mut index, "plan-3d-isolate-dim-others");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Component visibility")
        .click();
    shot(
        &mut harness,
        &dir,
        &mut index,
        "plan-3d-component-visibility-menu",
    );
    harness.key_press(egui::Key::Escape);
    harness.run_ok();
    assert_eq!(harness.state().selected_component_count(), 2);
    assert_eq!(
        harness.state().component_visibility.isolation_mode(),
        Some(super::IsolationMode::DimOthers),
        "closing the menu must not clear the selection or isolation"
    );
    assert!(
        harness
            .query_all_by_label("Exit Isolation")
            .next()
            .is_none(),
        "Escape should close the component-visibility popup"
    );

    open_3d_component_context_menu(&mut harness);
    harness.get_by_label("Isolate ⏵").click();
    shot(
        &mut harness,
        &dir,
        &mut index,
        "plan-3d-selection-context-menu",
    );
    harness.get_by_label("Dim Others").click();
    harness.run_ok();

    harness
        .state_mut()
        .execute_action(actions::ActionId::IsolateHide);
    assert_eq!(
        harness.state().component_visibility.isolation_mode(),
        Some(super::IsolationMode::HideOthers)
    );
    shot(
        &mut harness,
        &dir,
        &mut index,
        "plan-3d-isolate-hide-others",
    );
    harness
        .state_mut()
        .execute_action(actions::ActionId::ExitIsolation);

    let hidden_wall = ComponentKey::authored(
        AuthoredComponentKind::Wall,
        harness.state().model.walls[back_wall_index].id.0.clone(),
    );
    harness.state_mut().component_visibility.toggle(hidden_wall);
    shot(
        &mut harness,
        &dir,
        &mut index,
        "plan-3d-hidden-wall-browser-row",
    );
    harness.state_mut().component_visibility.show_all();

    let (door_wall_index, door_id) = harness
        .state()
        .model
        .walls
        .iter()
        .enumerate()
        .find_map(|(wall_index, wall)| {
            wall.openings
                .iter()
                .find(|opening| opening.kind == OpeningKind::Door)
                .map(|opening| (wall_index, opening.id.0.clone()))
        })
        .expect("demo shell has a door rough opening");
    harness.state_mut().apply_selection(
        Selection::Opening(door_id),
        Some(door_wall_index),
        SelectionOp::Replace,
    );
    harness
        .state_mut()
        .execute_action(actions::ActionId::IsolateHide);
    shot(
        &mut harness,
        &dir,
        &mut index,
        "plan-3d-isolated-door-rough-frame",
    );
    harness
        .state_mut()
        .execute_action(actions::ActionId::ExitIsolation);

    // A roof-specific checkpoint: the default shell has no authored roof, so
    // capture the two views that caught regressions only after generating one.
    harness.state_mut().add_roof(RoofForm::Gable);
    harness.state_mut().viewport_mode = ViewportMode::Axonometric;
    shot(&mut harness, &dir, &mut index, "roofed-3d-view");
    select_tab(&mut harness, actions::WorkflowTab::Plan);
    harness.state_mut().viewport_mode = ViewportMode::Axonometric;
    // The Plan cutaway introduces translucent geometry. Force one full repaint
    // before rasterizing so the off-screen target does not retain dirty-region
    // holes from the preceding Design frame.
    harness.ctx.request_repaint();
    harness.run_steps(2);
    shot(&mut harness, &dir, &mut index, "roofed-plan-3d-view");
    let full_roof_plan = harness.state().project_plan.clone();
    let full_wall_display = harness.state().layers.wall_display;
    harness.state_mut().layers.wall_display = WallDisplay::Outline;
    // Isolate one interior common rafter per field for the two construction-detail
    // shots; the full generated plan above remains the product-state checkpoint.
    if let Some(plan) = harness.state_mut().project_plan.as_mut() {
        for roof in &mut plan.roof_plans {
            let rafters: Vec<_> = roof
                .members
                .iter()
                .filter(|member| member.kind == MemberKind::Rafter)
                .map(|member| member.id.clone())
                .collect();
            let middle = rafters.get(rafters.len() / 2).cloned();
            roof.members
                .retain(|member| Some(&member.id) == middle.as_ref());
        }
    }
    {
        let runtime = harness.state().viewport_workspace.active_runtime();
        let mut runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime.view_3d = super::viewport::View3dState::roof_framing_detail_shot();
    }
    harness.ctx.request_repaint();
    harness.run_steps(2);
    shot(
        &mut harness,
        &dir,
        &mut index,
        "roofed-plan-rafter-ridge-cuts-detail",
    );
    {
        let runtime = harness.state().viewport_workspace.active_runtime();
        let mut runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime.view_3d = super::viewport::View3dState::roof_framing_eave_detail_shot();
    }
    harness.ctx.request_repaint();
    harness.run_steps(2);
    shot(
        &mut harness,
        &dir,
        &mut index,
        "roofed-plan-rafter-birdsmouth-detail",
    );
    harness.state_mut().project_plan = full_roof_plan;
    harness.state_mut().layers.wall_display = full_wall_display;
    {
        let runtime = harness.state().viewport_workspace.active_runtime();
        let mut runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime.view_3d = Default::default();
    }
    select_tab(&mut harness, actions::WorkflowTab::Render);
    warm_render(&mut harness);
    shot(&mut harness, &dir, &mut index, "roofed-render-view");
    select_tab(&mut harness, actions::WorkflowTab::Frame);
    harness.state_mut().viewport_mode = ViewportMode::Plan;

    let mut small = shots_harness_with_size(design::studio_light(), egui::vec2(1040.0, 680.0));
    small.run_ok();
    let small_back_wall_index = small
        .state()
        .model
        .walls
        .iter()
        .position(|wall| wall.id.0 == "wall-back")
        .expect("demo shell has a back wall");
    small.state_mut().selected_wall = small_back_wall_index;
    small.state_mut().selected = Selection::Wall;
    small
        .state_mut()
        .viewport_workspace
        .apply_builtin(BuiltInPreset::FourUp)
        .unwrap();
    sync_active_viewport_mode(&mut small);
    shot(&mut small, &dir, &mut index, "small-viewport-four-up");
    let quadrant_ids = small.state().viewport_workspace.layout.pane_ids();
    for id in quadrant_ids {
        small
            .state_mut()
            .viewport_workspace
            .split(id, SplitAxis::Horizontal)
            .unwrap();
    }
    let column_ids = small.state().viewport_workspace.layout.pane_ids();
    for id in column_ids {
        small
            .state_mut()
            .viewport_workspace
            .split(id, SplitAxis::Vertical)
            .unwrap();
    }
    sync_active_viewport_mode(&mut small);
    shot(&mut small, &dir, &mut index, "small-viewport-sixteen-pane");
    small
        .state_mut()
        .viewport_workspace
        .apply_builtin(BuiltInPreset::Focus)
        .unwrap();
    small.state_mut().viewport_mode = ViewportMode::Axonometric;
    shot(&mut small, &dir, &mut index, "small-3d-view");
    select_tab(&mut small, actions::WorkflowTab::Render);
    warm_render(&mut small);
    shot(&mut small, &dir, &mut index, "small-render-view");
    drop(small);

    // Overlay surfaces: the command palette, diagnostics popover, then the
    // Project menu (menus are egui popup state, so open them through real clicks;
    // the palette is closed by resetting its state because Escape handling is
    // part of what the shots exist to review).
    harness.state_mut().open_command_search();
    shot(&mut harness, &dir, &mut index, "command-palette");
    harness.state_mut().command_search = Default::default();
    harness.run_ok();

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Diagnostics")
        .click();
    shot(&mut harness, &dir, &mut index, "diagnostics-popover");

    harness.get_by_label("Project").click();
    shot(&mut harness, &dir, &mut index, "project-menu");
    drop(harness);

    // Regression checkpoint for drawing a room beside an existing shell: the
    // Plan body is one continuous perimeter, and generated 3-D framing does not
    // restart at the former exterior corners.
    let mut adjacent_room = shots_harness(design::studio_light());
    adjacent_room.run_ok();
    prepare_adjacent_room(&mut adjacent_room);
    select_tab(&mut adjacent_room, actions::WorkflowTab::Design);
    adjacent_room.state_mut().viewport_mode = ViewportMode::Plan;
    shot(
        &mut adjacent_room,
        &dir,
        &mut index,
        "adjacent-room-continuous-plan",
    );
    select_tab(&mut adjacent_room, actions::WorkflowTab::Plan);
    adjacent_room.state_mut().viewport_mode = ViewportMode::Axonometric;
    adjacent_room.state_mut().layers.wall_display = WallDisplay::Outline;
    adjacent_room.ctx.request_repaint();
    adjacent_room.run_steps(2);
    shot(
        &mut adjacent_room,
        &dir,
        &mut index,
        "adjacent-room-continuous-framing-3d",
    );
    drop(adjacent_room);

    let mut overlap_light = shots_harness(design::studio_light());
    overlap_light.run_ok();
    prepare_geometry_overlap(&mut overlap_light);
    shot(
        &mut overlap_light,
        &dir,
        &mut index,
        "geometry-overlap-focused-light",
    );
    overlap_light
        .get_by_role_and_label(egui::accesskit::Role::Button, "Diagnostics")
        .click();
    shot(
        &mut overlap_light,
        &dir,
        &mut index,
        "geometry-overlap-diagnostics-light",
    );
    drop(overlap_light);

    // Dark palette spot-checks: default state + an inspector-heavy state.
    let mut dark = shots_harness(design::studio_dark());
    dark.run_ok();
    shot(&mut dark, &dir, &mut index, "dark-frame-shell");
    let dark_back_wall_index = dark
        .state()
        .model
        .walls
        .iter()
        .position(|wall| wall.id.0 == "wall-back")
        .expect("demo shell has a back wall");
    dark.state_mut().selected_wall = dark_back_wall_index;
    dark.state_mut().selected = Selection::Wall;
    shot(&mut dark, &dir, &mut index, "dark-wall-selected");
    select_tab(&mut dark, actions::WorkflowTab::Plan);
    dark.state_mut().viewport_mode = ViewportMode::Axonometric;
    let dark_second_wall_index = (dark_back_wall_index + 1) % dark.state().model.walls.len();
    dark.state_mut().apply_selection(
        Selection::Wall,
        Some(dark_back_wall_index),
        SelectionOp::Replace,
    );
    dark.state_mut().apply_selection(
        Selection::Wall,
        Some(dark_second_wall_index),
        SelectionOp::Toggle,
    );
    dark.state_mut()
        .execute_action(actions::ActionId::IsolateDim);
    shot(
        &mut dark,
        &dir,
        &mut index,
        "dark-plan-3d-isolate-dim-others",
    );
    open_3d_component_context_menu(&mut dark);
    dark.get_by_label("Isolate ⏵").click();
    shot(
        &mut dark,
        &dir,
        &mut index,
        "dark-plan-3d-selection-context-menu",
    );
    dark.get_by_label("Dim Others").click();
    dark.run_ok();
    dark.state_mut()
        .execute_action(actions::ActionId::ExitIsolation);
    select_tab(&mut dark, actions::WorkflowTab::Frame);
    dark.state_mut().viewport_mode = ViewportMode::Plan;
    prepare_geometry_overlap(&mut dark);
    shot(&mut dark, &dir, &mut index, "geometry-overlap-focused-dark");
    dark.get_by_role_and_label(egui::accesskit::Role::Button, "Diagnostics")
        .click();
    shot(
        &mut dark,
        &dir,
        &mut index,
        "geometry-overlap-diagnostics-dark",
    );
    dark.get_by_role_and_label(egui::accesskit::Role::Button, "Diagnostics")
        .click();
    dark.run_ok();
    select_tab(&mut dark, actions::WorkflowTab::Render);
    warm_render(&mut dark);
    shot(&mut dark, &dir, &mut index, "dark-render-view");

    println!(
        "ui-shots: deck complete — {} frames in {}",
        index - 1,
        dir.display()
    );
}
