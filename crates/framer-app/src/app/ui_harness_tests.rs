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

use super::{FramerApp, ViewportMode, WallDisplay, design};

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
