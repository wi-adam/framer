//! Integration tests for undo/redo wired into `FramerApp`.
//!
//! These drive a headless `FramerApp::default()` (no GPU needed — the render
//! state is inert until a frame runs) through the real `edit`/`undo`/`redo`
//! methods, asserting that history records, restores model + selection, drops
//! no-op edits, re-solves on restore, and is cleared by load/reset.

use eframe::egui;
use framer_core::{CodeProfile, Length, OpeningKind, Point2, Wall};

use super::model_edit::WallEditHandle;
use super::viewport::WallDragEvent;
use super::{FramerApp, Selection};

/// Feed a single key-press through a real egui frame so `handle_keyboard_shortcuts`
/// sees it via `consume_key` exactly as it would at runtime.
fn press_key(app: &mut FramerApp, key: egui::Key, modifiers: egui::Modifiers) {
    let ctx = egui::Context::default();
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    ctx.run(input, |ctx| app.handle_keyboard_shortcuts(ctx));
}

/// A solver-safe authored mutation: `stud_spacing` is never rewritten by
/// `apply_driving_dimensions`, so the change survives `rebuild()` verbatim.
fn bump_stud_spacing(app: &mut FramerApp) {
    let spacing = app.model.walls[0].stud_spacing;
    app.model.walls[0].stud_spacing = spacing + Length::from_inches(1.0);
}

#[test]
fn edit_records_an_undo_step_and_undo_restores_the_model() {
    let mut app = FramerApp::default();
    let original = app.model.clone();

    app.edit("Adjust stud spacing", bump_stud_spacing);

    assert!(app.history.can_undo());
    assert_ne!(app.model, original, "edit should have changed the model");

    app.undo();
    assert_eq!(app.model, original, "undo should restore the prior model");
    assert!(!app.history.can_undo());
    assert!(app.history.can_redo());
}

#[test]
fn no_op_edit_records_nothing() {
    let mut app = FramerApp::default();
    app.edit("Touch nothing", |_app| {});
    assert!(!app.history.can_undo());
}

#[test]
fn undo_then_redo_round_trips_the_model() {
    let mut app = FramerApp::default();
    app.edit("Adjust stud spacing", bump_stud_spacing);
    let edited = app.model.clone();

    app.undo();
    app.redo();

    assert_eq!(app.model, edited, "redo should restore the post-edit model");
}

#[test]
fn undo_restores_the_prior_selection() {
    let mut app = FramerApp::default();
    app.selected = Selection::Wall;
    app.selected_wall = 0;

    app.edit("Adjust and reselect", |app| {
        bump_stud_spacing(app);
        app.selected = Selection::Level("level-1".to_owned());
    });
    assert_eq!(app.selected, Selection::Level("level-1".to_owned()));

    app.undo();
    assert_eq!(
        app.selected,
        Selection::Wall,
        "undo should restore the selection captured before the edit"
    );
}

#[test]
fn reset_demo_clears_history() {
    let mut app = FramerApp::default();
    app.edit("Adjust stud spacing", bump_stud_spacing);
    assert!(app.history.can_undo());

    app.reset_demo();
    assert!(!app.history.can_undo(), "reset should clear undo history");
    assert!(!app.history.can_redo(), "reset should clear redo history");
}

#[test]
fn add_opening_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    let wall = app.selected_wall;
    let before = app.model.walls[wall].openings.len();

    app.add_opening(OpeningKind::Window);
    assert_eq!(app.model.walls[wall].openings.len(), before + 1);
    assert_eq!(app.history.undo_label(), Some("Add opening"));

    app.undo();
    assert_eq!(
        app.model.walls[wall].openings.len(),
        before,
        "undo should remove the added opening"
    );
    // The solver re-ran on restore, so a framing plan is present again.
    assert!(app.project_plan.is_some());
}

#[test]
fn delete_opening_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    app.add_opening(OpeningKind::Window);
    let wall = app.selected_wall;
    let with_opening = app.model.walls[wall].openings.len();
    assert!(with_opening >= 1);

    // add_opening selected the new opening, so delete targets it.
    app.delete_selected_opening();
    assert_eq!(app.model.walls[wall].openings.len(), with_opening - 1);
    assert_eq!(app.history.undo_label(), Some("Delete opening"));

    app.undo();
    assert_eq!(
        app.model.walls[wall].openings.len(),
        with_opening,
        "undo restores the deleted opening"
    );
}

#[test]
fn add_wall_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    let before = app.model.walls.len();

    // Draw a free-floating wall clear of the demo shell (no auto-joins).
    app.add_wall(
        Point2::new(Length::from_feet(0.0), Length::from_feet(30.0)),
        Point2::new(Length::from_feet(10.0), Length::from_feet(30.0)),
    );

    assert_eq!(app.model.walls.len(), before + 1);
    assert_eq!(app.history.undo_label(), Some("Draw wall"));

    app.undo();
    assert_eq!(
        app.model.walls.len(),
        before,
        "undo should remove the drawn wall"
    );
    assert!(app.project_plan.is_some(), "solver re-ran on restore");
}

#[test]
fn add_wall_auto_creates_corner_join_at_shared_endpoint() {
    let mut app = FramerApp::default();
    let joins_before = app.model.wall_joins.len();

    // The demo shell has an endpoint at the origin; a wall drawn from there
    // should auto-join the walls meeting at (0,0).
    app.add_wall(
        Point2::new(Length::from_feet(0.0), Length::from_feet(0.0)),
        Point2::new(Length::from_feet(0.0), Length::from_feet(-10.0)),
    );

    assert!(
        app.model.wall_joins.len() > joins_before,
        "drawing onto a shared endpoint should record a corner join"
    );
}

#[test]
fn add_wall_rejects_non_ortho_segment() {
    let mut app = FramerApp::default();
    let before = app.model.walls.len();

    // A diagonal segment is not a valid axis-aligned wall; it must not be added
    // (and must not pollute undo history with an invalid model).
    app.add_wall(
        Point2::new(Length::from_feet(0.0), Length::from_feet(40.0)),
        Point2::new(Length::from_feet(10.0), Length::from_feet(50.0)),
    );

    assert_eq!(
        app.model.walls.len(),
        before,
        "diagonal wall must be rejected"
    );
    assert!(
        !app.history.can_undo(),
        "rejected wall records no undo step"
    );
}

#[test]
fn add_room_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    let before = app.model.rooms.len();

    // Seed inside the demo shell (28ft × 20ft).
    app.add_room(Point2::new(
        Length::from_feet(14.0),
        Length::from_feet(10.0),
    ));

    assert_eq!(app.model.rooms.len(), before + 1);
    assert_eq!(app.history.undo_label(), Some("Add room"));

    app.undo();
    assert_eq!(app.model.rooms.len(), before, "undo removes the added room");
}

#[test]
fn delete_room_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    app.add_room(Point2::new(
        Length::from_feet(14.0),
        Length::from_feet(10.0),
    ));
    let id = match &app.selected {
        Selection::Room(id) => id.clone(),
        other => panic!("expected the new room selected, got {other:?}"),
    };

    app.delete_selected_room();
    assert!(app.model.rooms.iter().all(|room| room.id.0 != id));
    assert_eq!(app.history.undo_label(), Some("Delete room"));

    app.undo();
    assert_eq!(app.model.rooms.len(), 1, "undo restores the deleted room");
}

#[test]
fn delete_last_wall_leaves_consistent_state() {
    let mut app = FramerApp::default();
    // Delete every wall; this must not panic and must leave an empty model.
    while !app.model.walls.is_empty() {
        app.selected = Selection::Wall;
        app.selected_wall = 0;
        app.delete_selected_wall();
    }
    assert!(app.model.walls.is_empty());
    assert!(app.model.wall_joins.is_empty(), "all joins cascade away");
}

#[test]
fn delete_wall_drops_referencing_joins_in_one_step() {
    let mut app = FramerApp::default();
    let walls_before = app.model.walls.len();
    let joins_before = app.model.wall_joins.len();
    app.selected = Selection::Wall;
    app.selected_wall = 0;
    let removed_id = app.model.walls[0].id.0.clone();

    app.delete_selected_wall();

    assert_eq!(app.model.walls.len(), walls_before - 1);
    assert!(app.model.walls.iter().all(|wall| wall.id.0 != removed_id));
    assert!(
        app.model.wall_joins.len() < joins_before,
        "deleting a wall should drop the joins that reference it"
    );
    assert_eq!(app.history.undo_label(), Some("Delete wall"));

    app.undo();
    assert_eq!(app.model.walls.len(), walls_before);
    assert_eq!(app.model.wall_joins.len(), joins_before);
}

#[test]
fn cmd_z_undoes_via_keyboard() {
    let mut app = FramerApp::default();
    let original = app.model.clone();
    app.edit("Adjust stud spacing", bump_stud_spacing);
    assert_ne!(app.model, original);

    press_key(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(app.model, original, "Cmd/Ctrl+Z must undo");
}

#[test]
fn cmd_shift_z_redoes_via_keyboard() {
    let mut app = FramerApp::default();
    let original = app.model.clone();
    app.edit("Adjust stud spacing", bump_stud_spacing);
    let edited = app.model.clone();
    app.undo();
    assert_eq!(app.model, original);
    assert!(app.history.can_redo());

    // Regression: egui's consume_key matches modifiers logically, so a naive
    // Cmd+Z undo check would also swallow Cmd+Shift+Z. Redo must win the chord.
    press_key(
        &mut app,
        egui::Key::Z,
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
    );
    assert_eq!(app.model, edited, "Cmd/Ctrl+Shift+Z must redo, not undo");
    assert!(!app.history.can_redo());
}

/// Replace the app's model with a minimal L of two walls sharing a corner at the
/// origin, so wall `a` has a *free* end at (10ft, 0) that can be dragged.
fn install_corner_model(app: &mut FramerApp) {
    let code = CodeProfile::irc_2021_prescriptive();
    let pt = |x: f64, y: f64| Point2::new(Length::from_feet(x), Length::from_feet(y));
    let wall = |id: &str, start, end| {
        Wall::new(id, id, Length::from_feet(1.0), &code).with_placement("level-1", start, end)
    };
    app.model.walls.clear();
    app.model.wall_joins.clear();
    app.model.rooms.clear();
    app.model.walls.push(wall("a", pt(0.0, 0.0), pt(10.0, 0.0)));
    app.model.walls.push(wall("b", pt(0.0, 0.0), pt(0.0, 8.0)));
    app.model.reconcile_joins();
    app.rebuild();
}

#[test]
fn wall_endpoint_drag_extends_a_free_end_as_one_undo_step() {
    let mut app = FramerApp::default();
    install_corner_model(&mut app);
    app.selected = Selection::Wall;
    app.selected_wall = 0;
    let original = app.model.clone();

    // Drag wall a's free end from (10,0) out to (12,0): one coalesced gesture.
    app.handle_wall_drag_event(WallDragEvent::Started {
        wall_index: 0,
        handle: WallEditHandle::End,
    });
    app.handle_wall_drag_event(WallDragEvent::Updated {
        point: Point2::new(Length::from_feet(12.0), Length::ZERO),
    });
    app.handle_wall_drag_event(WallDragEvent::Stopped);

    assert!(app.history.can_undo());
    assert_eq!(app.model.walls[0].end, Point2::new(Length::from_feet(12.0), Length::ZERO));
    assert_eq!(app.model.walls[0].length, Length::from_feet(12.0));
    // The shared corner and its join are untouched.
    assert_eq!(app.model.walls[1].start, Point2::new(Length::ZERO, Length::ZERO));
    assert_eq!(app.model.wall_joins.len(), 1);

    // The whole drag undoes in a single step.
    app.undo();
    assert_eq!(app.model, original);
    assert!(!app.history.can_undo());
}

#[test]
fn wall_endpoint_drag_drags_shared_node_along_a_collinear_run() {
    let mut app = FramerApp::default();
    let code = CodeProfile::irc_2021_prescriptive();
    let pt = |x: f64, y: f64| Point2::new(Length::from_feet(x), Length::from_feet(y));
    let wall = |id: &str, start, end| {
        Wall::new(id, id, Length::from_feet(1.0), &code).with_placement("level-1", start, end)
    };
    app.model.walls.clear();
    app.model.wall_joins.clear();
    app.model.rooms.clear();
    app.model.walls.push(wall("a", pt(0.0, 0.0), pt(10.0, 0.0)));
    app.model.walls.push(wall("b", pt(10.0, 0.0), pt(20.0, 0.0)));
    app.model.reconcile_joins();
    app.rebuild();
    app.selected = Selection::Wall;
    app.selected_wall = 0;

    app.handle_wall_drag_event(WallDragEvent::Started {
        wall_index: 0,
        handle: WallEditHandle::End,
    });
    app.handle_wall_drag_event(WallDragEvent::Updated {
        point: Point2::new(Length::from_feet(12.0), Length::ZERO),
    });
    app.handle_wall_drag_event(WallDragEvent::Stopped);

    // The shared node moved on both walls (node-follow): a grew, b shrank.
    assert_eq!(app.model.walls[0].end, Point2::new(Length::from_feet(12.0), Length::ZERO));
    assert_eq!(app.model.walls[1].start, Point2::new(Length::from_feet(12.0), Length::ZERO));
    assert_eq!(app.model.walls[0].length, Length::from_feet(12.0));
    assert_eq!(app.model.walls[1].length, Length::from_feet(8.0));
}

#[test]
fn whole_wall_translate_slides_perpendicular_and_stretches_neighbour() {
    let mut app = FramerApp::default();
    install_corner_model(&mut app);
    app.selected = Selection::Wall;
    app.selected_wall = 0;
    let original = app.model.clone();

    // Body-drag wall a (horizontal at y=0). The vertical component slides it up;
    // any horizontal component is projected out (perpendicular-only translate).
    app.handle_wall_drag_event(WallDragEvent::Started {
        wall_index: 0,
        handle: WallEditHandle::Body,
    });
    app.handle_wall_drag_event(WallDragEvent::Translated {
        dx: Length::from_feet(5.0),
        dy: Length::from_feet(2.0),
    });
    app.handle_wall_drag_event(WallDragEvent::Stopped);

    // a slid up 2ft (x unchanged); b's shared corner followed up, shortening it.
    assert_eq!(app.model.walls[0].start, Point2::new(Length::ZERO, Length::from_feet(2.0)));
    assert_eq!(app.model.walls[0].end, Point2::new(Length::from_feet(10.0), Length::from_feet(2.0)));
    assert_eq!(app.model.walls[1].start, Point2::new(Length::ZERO, Length::from_feet(2.0)));
    assert_eq!(app.model.walls[1].length, Length::from_feet(6.0));

    app.undo();
    assert_eq!(app.model, original);
}
