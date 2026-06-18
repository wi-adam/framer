//! Integration tests for undo/redo wired into `FramerApp`.
//!
//! These drive a headless `FramerApp::default()` (no GPU needed — the render
//! state is inert until a frame runs) through the real `edit`/`undo`/`redo`
//! methods, asserting that history records, restores model + selection, drops
//! no-op edits, re-solves on restore, and is cleared by load/reset.

use framer_core::{Length, OpeningKind};

use super::{FramerApp, Selection};

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
