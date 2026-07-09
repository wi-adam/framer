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

use super::{FramerApp, RoofForm, Selection, ViewportMode, actions, design};

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
    harness.state_mut().render_motion_cooldown = 24;
    for _ in 0..80 {
        harness.run_steps(1);
        let state = harness.state();
        if state.render_view.samples() > 0 || state.render_gpu.samples() > 0 {
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

    // Workflow tabs (Design workspace), default Shell/Plan view.
    for (tab, name) in [
        (actions::WorkflowTab::Frame, "frame-shell"),
        (actions::WorkflowTab::Design, "design-tab"),
        (actions::WorkflowTab::Openings, "openings-tab"),
        (actions::WorkflowTab::Roofs, "roofs-tab"),
        (actions::WorkflowTab::Annotate, "annotate-tab"),
        (actions::WorkflowTab::Plan, "plan-workspace"),
    ] {
        select_tab(&mut harness, tab);
        shot(&mut harness, &dir, &mut index, name);
    }

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
        (ViewportMode::Render, "render-view"),
    ] {
        harness.state_mut().viewport_mode = mode;
        if mode == ViewportMode::Render {
            warm_render(&mut harness);
        }
        shot(&mut harness, &dir, &mut index, name);
    }
    harness.state_mut().viewport_mode = ViewportMode::Plan;

    // A roof-specific checkpoint: the default shell has no authored roof, so
    // capture the two views that caught regressions only after generating one.
    harness.state_mut().add_roof(RoofForm::Gable);
    for (mode, name) in [
        (ViewportMode::Axonometric, "roofed-3d-view"),
        (ViewportMode::Render, "roofed-render-view"),
    ] {
        harness.state_mut().viewport_mode = mode;
        if mode == ViewportMode::Render {
            warm_render(&mut harness);
        }
        shot(&mut harness, &dir, &mut index, name);
    }
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
    for (mode, name) in [
        (ViewportMode::Axonometric, "small-3d-view"),
        (ViewportMode::Render, "small-render-view"),
    ] {
        small.state_mut().viewport_mode = mode;
        if mode == ViewportMode::Render {
            warm_render(&mut small);
        }
        shot(&mut small, &dir, &mut index, name);
    }
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
    dark.state_mut().viewport_mode = ViewportMode::Render;
    warm_render(&mut dark);
    shot(&mut dark, &dir, &mut index, "dark-render-view");

    println!(
        "ui-shots: deck complete — {} frames in {}",
        index - 1,
        dir.display()
    );
}
