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

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::{FramerApp, ViewportMode, actions, design};

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
    let mut fonts_bound = false;
    Harness::builder()
        .with_size(egui::vec2(1360.0, 860.0))
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
            FramerApp::default(),
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
        (actions::WorkflowTab::Inspect, "inspect-tab"),
        (actions::WorkflowTab::Plan, "plan-workspace"),
    ] {
        select_tab(&mut harness, tab);
        shot(&mut harness, &dir, &mut index, name);
    }

    // Selection states (back in the Frame tab): wall, opening, corner — the
    // three inspector layouts. Tree row labels are unique in the AccessKit
    // tree, so clicks are unambiguous.
    select_tab(&mut harness, actions::WorkflowTab::Frame);
    harness.run_ok();
    for (label, name) in [
        ("Wall segment: Back wall", "wall-selected"),
        ("Window: Back left window", "opening-selected"),
        ("Corner: Back left corner", "corner-selected"),
    ] {
        harness.get_by_label(label).click();
        shot(&mut harness, &dir, &mut index, name);
    }

    // Views, with the back wall selected for the elevation.
    harness.get_by_label("Wall segment: Back wall").click();
    harness.run_ok();
    for (mode, name) in [
        (ViewportMode::Elevation, "wall-elevation-view"),
        (ViewportMode::RoofPlan, "roof-view"),
        (ViewportMode::Axonometric, "3d-view"),
        (ViewportMode::Render, "render-view"),
    ] {
        harness.state_mut().viewport_mode = mode;
        if mode == ViewportMode::Render {
            // The progressive path tracer never reports idle; give it a few
            // frames to produce a first image instead of settling.
            harness.run_steps(8);
        }
        shot(&mut harness, &dir, &mut index, name);
    }
    harness.state_mut().viewport_mode = ViewportMode::Plan;

    // Overlay surfaces: the command palette, then the Project menu (menus are
    // egui popup state, so open it through a real click; the palette is closed
    // by resetting its state because Escape handling is part of what the shots
    // exist to review).
    harness.state_mut().open_command_search();
    shot(&mut harness, &dir, &mut index, "command-palette");
    harness.state_mut().command_search = Default::default();
    harness.run_ok();

    harness.get_by_label("Project").click();
    shot(&mut harness, &dir, &mut index, "project-menu");
    drop(harness);

    // Dark palette spot-checks: default state + an inspector-heavy state.
    let mut dark = shots_harness(design::studio_dark());
    dark.run_ok();
    shot(&mut dark, &dir, &mut index, "dark-frame-shell");
    dark.get_by_label("Wall segment: Back wall").click();
    shot(&mut dark, &dir, &mut index, "dark-wall-selected");

    println!(
        "ui-shots: deck complete — {} frames in {}",
        index - 1,
        dir.display()
    );
}
