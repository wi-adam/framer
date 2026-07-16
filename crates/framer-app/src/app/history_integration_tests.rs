//! Integration tests for undo/redo wired into `FramerApp`.
//!
//! These drive a headless `FramerApp::default()` (no GPU needed — the render
//! state is inert until a frame runs) through the real `edit`/`undo`/`redo`
//! methods, asserting that history records, restores model + selection, drops
//! no-op edits, re-solves on restore, and is cleared by load/reset.

use eframe::egui;
use framer_analysis::{AssertionParticipantRole, AssertionRef, AssertionSource};
use framer_core::{
    Applicability, AuthoredIntentMode, BuildingModel, CheckScope, CheckSeverity, ClearanceDatum,
    ClearanceDirection, CompareOp, ComplianceCheck, ElementId, Fact, FactOperand, FramingDefaults,
    Furnishing, FurnishingInstance, IntentDomain, Length, Level, MepInstance, MepObject,
    MepObjectKind, OpeningKind, Point2, Predicate, PreferencePriority, Room, RoomUsage, Wall,
};

use super::actions::ActionId;
use super::model_edit::WallEditHandle;
use super::viewport::WallDragEvent;
use super::{
    AuthoredComponentKind, ComponentKey, FramerApp, IntentAuthoringDraft, IntentDraftExpression,
    RoofForm, Selection, SelectionOp, ViewClick, ViewportMode, WorkspaceMode,
};

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
    let _ = ctx.run_ui(input, |ui| app.handle_keyboard_shortcuts(ui.ctx()));
}

/// A solver-safe authored mutation: a wall's `name` is never rewritten by
/// `apply_driving_dimensions`, so the change survives `rebuild()` verbatim.
fn rename_first_wall(app: &mut FramerApp) {
    app.model.walls[0].name.push('*');
}

fn replace_walls_with_rectangular_footprint(app: &mut FramerApp, width_ft: f64, depth_ft: f64) {
    while !app.model.walls.is_empty() {
        app.selected = Selection::Wall;
        app.selected_wall = 0;
        app.delete_selected_wall();
    }
    let corners = [
        (0.0, 0.0),
        (width_ft, 0.0),
        (width_ft, depth_ft),
        (0.0, depth_ft),
        (0.0, 0.0),
    ];
    for pair in corners.windows(2) {
        app.add_wall(
            Point2::new(Length::from_feet(pair[0].0), Length::from_feet(pair[0].1)),
            Point2::new(Length::from_feet(pair[1].0), Length::from_feet(pair[1].1)),
        );
    }
    app.history.clear();
}

fn replace_walls_with_l_footprint(app: &mut FramerApp) {
    while !app.model.walls.is_empty() {
        app.selected = Selection::Wall;
        app.selected_wall = 0;
        app.delete_selected_wall();
    }
    let corners = [
        (0.0, 0.0),
        (24.0, 0.0),
        (24.0, 12.0),
        (12.0, 12.0),
        (12.0, 24.0),
        (0.0, 24.0),
        (0.0, 0.0),
    ];
    for pair in corners.windows(2) {
        app.add_wall(
            Point2::new(Length::from_feet(pair[0].0), Length::from_feet(pair[0].1)),
            Point2::new(Length::from_feet(pair[1].0), Length::from_feet(pair[1].1)),
        );
    }
    app.history.clear();
}

fn prepare_intent_fixture(app: &mut FramerApp) -> (ElementId, ElementId) {
    let mut model = BuildingModel::demo_shell();
    let room = Room::new(
        "room-intent",
        "Intent room",
        RoomUsage::Living,
        "level-1",
        Point2::new(Length::from_feet(10.0), Length::from_feet(10.0)),
    );
    let subject_id = ElementId::new("furnishing-instance-intent");
    model.rooms.push(room.clone());
    model.furnishings.push(Furnishing::new(
        "furnishing-intent",
        "Intent furnishing",
        Length::from_whole_inches(24),
        Length::from_whole_inches(24),
        Length::from_whole_inches(30),
    ));
    model.furnishing_instances.push(FurnishingInstance::new(
        subject_id.0.clone(),
        "Placed intent furnishing",
        "furnishing-intent",
        "level-1",
        Point2::new(Length::from_feet(10.0), Length::from_feet(10.0)),
    ));
    model.sort_deterministically();
    model.validate().expect("intent fixture model");
    app.model = model;
    app.selected = Selection::FurnishingInstance(subject_id.0.clone());
    app.component_selection.replace(Some(ComponentKey::authored(
        AuthoredComponentKind::FurnishingInstance,
        subject_id.0.clone(),
    )));
    app.history.clear();
    app.rebuild();
    (subject_id, room.id)
}

fn create_intent(
    app: &mut FramerApp,
    expression: IntentDraftExpression,
) -> framer_core::AuthoredIntentId {
    assert!(app.begin_intent_assertion(expression));
    let Some(IntentAuthoringDraft::Assertion(draft)) = app.intent_authoring_draft.as_mut() else {
        panic!("assertion draft");
    };
    draft.rationale = "Keep the placement usable".to_owned();
    assert!(app.accept_intent_authoring());
    app.model.intents.last().expect("created intent").id.clone()
}

fn waive_intent(app: &mut FramerApp, id: &framer_core::AuthoredIntentId) {
    assert!(app.begin_waive_intent(id));
    let Some(IntentAuthoringDraft::Waiver(draft)) = app.intent_authoring_draft.as_mut() else {
        panic!("waiver draft");
    };
    draft.reason = "Approved project exception".to_owned();
    assert!(app.accept_intent_authoring());
}

#[test]
fn cancelling_or_changing_selection_drops_intent_draft_without_history() {
    let mut app = FramerApp::default();
    let (_, room_id) = prepare_intent_fixture(&mut app);
    let before = app.model.clone();

    assert!(app.begin_intent_assertion(IntentDraftExpression::Containment));
    assert!(app.intent_authoring_draft.is_some());
    app.cancel_intent_authoring();
    assert_eq!(app.model, before);
    assert!(!app.history.can_undo());

    assert!(app.begin_intent_assertion(IntentDraftExpression::Containment));
    app.apply_selection(Selection::Room(room_id.0), None, SelectionOp::Replace);
    assert!(app.intent_authoring_draft.is_none());
    assert_eq!(app.model, before);
    assert!(!app.history.can_undo());

    app.apply_selection(
        Selection::FurnishingInstance("furnishing-instance-intent".to_owned()),
        None,
        SelectionOp::Replace,
    );
    assert!(app.begin_intent_assertion(IntentDraftExpression::Containment));
    app.reset_demo();
    assert!(app.intent_authoring_draft.is_none());
    assert!(!app.history.can_undo());
}

#[test]
fn create_edit_waive_and_delete_intent_each_round_trip_through_undo_redo() {
    let mut app = FramerApp::default();
    prepare_intent_fixture(&mut app);

    let id = create_intent(&mut app, IntentDraftExpression::Containment);
    assert_eq!(app.history.undo_label(), Some("Add intent"));
    app.undo();
    assert!(app.model.intents.is_empty());
    app.redo();
    assert_eq!(app.model.intents.len(), 1);

    assert!(app.begin_edit_intent(&id));
    let Some(IntentAuthoringDraft::Assertion(draft)) = app.intent_authoring_draft.as_mut() else {
        panic!("edit draft");
    };
    draft.mode = AuthoredIntentMode::Preference {
        priority: PreferencePriority(7),
    };
    draft.rationale = "Prefer this exact room".to_owned();
    assert!(app.accept_intent_authoring());
    assert_eq!(app.history.undo_label(), Some("Edit intent"));
    app.undo();
    assert_eq!(app.model.intents[0].mode, AuthoredIntentMode::Requirement);
    app.redo();
    assert_eq!(
        app.model.intents[0].mode,
        AuthoredIntentMode::Preference {
            priority: PreferencePriority(7)
        }
    );

    waive_intent(&mut app, &id);
    assert_eq!(app.history.undo_label(), Some("Waive intent"));
    app.undo();
    assert!(app.model.intent_overrides.is_empty());
    app.redo();
    assert_eq!(app.model.intent_overrides.len(), 1);

    assert!(app.delete_intent(&id));
    assert_eq!(app.history.undo_label(), Some("Delete intent"));
    assert!(app.model.intents.is_empty());
    assert!(app.model.intent_overrides.is_empty());
    app.undo();
    assert_eq!(app.model.intents.len(), 1);
    assert_eq!(app.model.intent_overrides.len(), 1);
    app.redo();
    assert!(app.model.intents.is_empty());
    assert!(app.model.intent_overrides.is_empty());
}

#[test]
fn plan_workspace_rejects_direct_intent_mutations_and_drops_live_drafts() {
    let mut app = FramerApp::default();
    prepare_intent_fixture(&mut app);
    let id = create_intent(&mut app, IntentDraftExpression::Containment);
    app.history.clear();
    let before = app.model.clone();

    assert!(app.begin_edit_intent(&id));
    let stale_draft = app
        .intent_authoring_draft
        .clone()
        .expect("Design should open the edit draft");
    app.set_workspace_mode(WorkspaceMode::Plan);
    assert!(app.intent_authoring_draft.is_none());

    // Even a stale draft injected by a direct caller cannot bypass the output-workspace guard.
    app.intent_authoring_draft = Some(stale_draft);
    assert!(!app.accept_intent_authoring());
    assert!(app.intent_authoring_draft.is_none());
    assert!(!app.begin_intent_assertion(IntentDraftExpression::Containment));
    assert!(!app.begin_edit_intent(&id));
    assert!(!app.begin_waive_intent(&id));
    assert!(!app.delete_intent(&id));

    assert_eq!(app.model, before);
    assert!(!app.history.can_undo());
    assert!(app.focus_all_intent_participants(&AssertionRef::Authored(id)));
    assert_eq!(app.selected_component_count(), 2);
    assert_eq!(app.model, before);
    assert!(!app.history.can_undo());
}

#[test]
fn editing_intent_preserves_its_valid_persisted_domain() {
    let mut app = FramerApp::default();
    prepare_intent_fixture(&mut app);
    let id = create_intent(&mut app, IntentDraftExpression::Containment);
    app.model.intents[0].domain = IntentDomain::Compliance;
    app.model.validate().expect("Compliance-domain fixture");
    app.rebuild();
    app.history.clear();

    assert!(app.begin_edit_intent(&id));
    let Some(IntentAuthoringDraft::Assertion(draft)) = app.intent_authoring_draft.as_mut() else {
        panic!("edit draft");
    };
    draft.rationale = "Retain the authored compliance classification".to_owned();
    assert!(app.accept_intent_authoring());

    assert_eq!(app.model.intents[0].domain, IntentDomain::Compliance);
    framer_core::save_project(&app.model).expect("edited intent remains saveable");
    app.undo();
    assert_eq!(app.model.intents[0].domain, IntentDomain::Compliance);
    app.redo();
    assert_eq!(app.model.intents[0].domain, IntentDomain::Compliance);
}

#[test]
fn scoped_placed_object_level_changes_reject_without_history_or_invalid_state() {
    let mut app = FramerApp::default();
    let (furnishing_id, _) = prepare_intent_fixture(&mut app);
    let upper = Level::new("level-2", "Level 2", Length::from_feet(10.0));
    app.model.levels.push(upper.clone());
    app.model.sort_deterministically();
    app.rebuild();
    let intent_id = create_intent(&mut app, IntentDraftExpression::Containment);
    app.history.clear();
    let before = app.model.clone();
    let furnishing_ref = framer_core::AuthoredEntityRef::FurnishingInstance(furnishing_id.clone());

    assert!(!app.change_placed_object_level(furnishing_ref.clone(), upper.id.clone()));
    assert_eq!(app.model, before);
    assert!(!app.history.can_undo());
    assert!(
        app.file_status
            .as_deref()
            .is_some_and(|status| status.contains("participates in project intent"))
    );
    framer_core::save_project(&app.model).expect("rejected furnishing move remains saveable");

    assert!(app.delete_intent(&intent_id));
    app.history.clear();
    assert!(app.change_placed_object_level(furnishing_ref, upper.id.clone()));
    assert_eq!(
        app.history.undo_label(),
        Some("Change furnishing placement level")
    );
    assert_eq!(app.model.furnishing_instances[0].level, upper.id);
    framer_core::save_project(&app.model).expect("accepted unscoped move remains saveable");
    app.undo();
    assert_eq!(app.model.furnishing_instances[0].level.0, "level-1");

    app.model.mep_objects.push(MepObject::new(
        "mep-intent-level",
        "Intent level fixture",
        MepObjectKind::Plumbing,
        Length::from_whole_inches(18),
        Length::from_whole_inches(24),
        Length::from_whole_inches(30),
    ));
    let mep_id = ElementId::new("mep-instance-intent-level");
    app.model.mep_instances.push(MepInstance::new(
        mep_id.0.clone(),
        "Placed intent level fixture",
        "mep-intent-level",
        "level-1",
        Point2::new(Length::from_feet(12.0), Length::from_feet(12.0)),
    ));
    app.model.sort_deterministically();
    app.model.validate().expect("MEP fixture");
    app.selected = Selection::MepInstance(mep_id.0.clone());
    app.component_selection.replace(Some(ComponentKey::authored(
        AuthoredComponentKind::MepInstance,
        mep_id.0.clone(),
    )));
    app.rebuild();
    create_intent(&mut app, IntentDraftExpression::Containment);
    app.history.clear();
    let before = app.model.clone();

    assert!(!app.change_placed_object_level(
        framer_core::AuthoredEntityRef::MepInstance(mep_id),
        upper.id
    ));
    assert_eq!(app.model, before);
    assert!(!app.history.can_undo());
    framer_core::save_project(&app.model).expect("rejected MEP move remains saveable");
}

#[test]
fn same_frame_placement_field_then_level_change_settles_in_order() {
    const ORIGINAL_NAME: &str = "Placed intent furnishing";
    const EDITED_NAME: &str = "Edited intent furnishing";

    let mut app = FramerApp::default();
    let (furnishing_id, _) = prepare_intent_fixture(&mut app);
    let upper = Level::new("level-2", "Level 2", Length::from_feet(10.0));
    app.model.levels.push(upper.clone());
    app.model.sort_deterministically();
    app.rebuild();
    app.history.clear();

    let base = app.snapshot();
    app.model
        .furnishing_instances
        .iter_mut()
        .find(|instance| instance.id == furnishing_id)
        .expect("furnishing fixture")
        .name = EDITED_NAME.to_owned();
    let mut changed = true;
    let mut edit_base = Some(base);

    app.settle_inspector_history_before_discrete_edit(
        &mut changed,
        &mut edit_base,
        "Edit furnishing placement",
    );
    assert!(!changed);
    assert!(edit_base.is_none());
    assert!(!app.history.is_pending());
    assert_eq!(app.history.undo_label(), Some("Edit furnishing placement"));

    assert!(app.change_placed_object_level(
        framer_core::AuthoredEntityRef::FurnishingInstance(furnishing_id),
        upper.id
    ));
    let assert_state = |app: &FramerApp, expected_name: &str, expected_level: &str| {
        let instance = &app.model.furnishing_instances[0];
        assert_eq!(instance.name, expected_name);
        assert_eq!(instance.level.0, expected_level);
        app.model.validate().expect("placement history model");
        framer_core::save_project(&app.model).expect("placement history model remains saveable");
    };

    assert_state(&app, EDITED_NAME, "level-2");
    assert_eq!(
        app.history.undo_label(),
        Some("Change furnishing placement level")
    );

    app.undo();
    assert_state(&app, EDITED_NAME, "level-1");
    assert_eq!(app.history.undo_label(), Some("Edit furnishing placement"));
    assert_eq!(
        app.history.redo_label(),
        Some("Change furnishing placement level")
    );

    app.undo();
    assert_state(&app, ORIGINAL_NAME, "level-1");
    assert_eq!(app.history.undo_label(), None);
    assert_eq!(app.history.redo_label(), Some("Edit furnishing placement"));

    app.redo();
    assert_state(&app, EDITED_NAME, "level-1");
    assert_eq!(
        app.history.redo_label(),
        Some("Change furnishing placement level")
    );
    app.redo();
    assert_state(&app, EDITED_NAME, "level-2");
    assert_eq!(app.history.redo_label(), None);
}

#[test]
fn invalid_intent_participant_never_mutates_model_or_history() {
    let mut app = FramerApp::default();
    prepare_intent_fixture(&mut app);
    let before = app.model.clone();
    assert!(app.begin_intent_assertion(IntentDraftExpression::Containment));
    let Some(IntentAuthoringDraft::Assertion(draft)) = app.intent_authoring_draft.as_mut() else {
        panic!("assertion draft");
    };
    draft.room = Some(ElementId::new("room-does-not-exist"));
    draft.rationale = "Invalid participant fixture".to_owned();

    assert!(!app.accept_intent_authoring());
    assert_eq!(app.model, before);
    assert!(!app.history.can_undo());
    assert!(app.intent_authoring_draft.is_some());
}

#[test]
fn save_load_recomputes_intent_and_clears_incomplete_draft() {
    let mut app = FramerApp::default();
    prepare_intent_fixture(&mut app);
    let id = create_intent(&mut app, IntentDraftExpression::Containment);
    let document = framer_core::save_project(&app.model).expect("save intent project");
    let path =
        std::env::temp_dir().join(format!("framer-intent-load-{}.framer", std::process::id()));
    std::fs::write(&path, document).expect("write project");

    assert!(app.begin_intent_assertion(IntentDraftExpression::Containment));
    app.project_path = path.display().to_string();
    app.load_project_file();
    let _ = std::fs::remove_file(path);

    assert!(app.intent_authoring_draft.is_none());
    assert!(!app.history.can_undo());
    let record = app
        .intent_report
        .as_ref()
        .and_then(|report| report.as_ref().ok())
        .and_then(|report| report.record(&AssertionRef::Authored(id)));
    assert!(
        record.is_some(),
        "load must rebuild the compiled intent report"
    );
}

#[test]
fn focus_all_selects_object_and_room_while_keeping_object_primary() {
    let mut app = FramerApp::default();
    let (subject_id, room_id) = prepare_intent_fixture(&mut app);
    let id = create_intent(&mut app, IntentDraftExpression::Containment);
    let reference = AssertionRef::Authored(id);

    assert!(app.focus_all_intent_participants(&reference));
    assert_eq!(
        app.selected,
        Selection::FurnishingInstance(subject_id.0.clone())
    );
    assert_eq!(app.selected_component_count(), 2);
    assert!(app.component_is_selected(&ComponentKey::authored(
        AuthoredComponentKind::Room,
        room_id.0
    )));
    assert!(app.component_is_selected(&ComponentKey::authored(
        AuthoredComponentKind::FurnishingInstance,
        subject_id.0
    )));
    assert_eq!(app.focused_intent, Some(reference));
}

#[test]
fn focus_all_uses_evaluated_entity_for_standards_placement_records() {
    let mut app = FramerApp::default();
    let (subject_id, room_id) = prepare_intent_fixture(&mut app);
    let rule = "fixture.standards-placement-focus";
    app.model.standards_packs[0].checks.push(ComplianceCheck {
        rule: rule.to_owned(),
        citation: "Intent focus fixture".to_owned(),
        title: "Placed object remains in its room".to_owned(),
        severity: CheckSeverity::Required,
        applies: Applicability::Always,
        scope: CheckScope::PlacedObjects { tags: Vec::new() },
        requirement: Predicate::Compare {
            fact: Fact::PlacedObjectContainedInRoom,
            op: CompareOp::Eq,
            value: FactOperand::FlagLiteral(true),
        },
    });
    app.model.sort_deterministically();
    app.model.validate().expect("standards focus fixture");
    app.rebuild();

    let reference = app
        .intent_report
        .as_ref()
        .and_then(|report| report.as_ref().ok())
        .and_then(|report| {
            report.records().iter().find_map(|record| {
                let assertion = record.assertion();
                match &assertion.source {
                    AssertionSource::StandardsRule(source) if source.rule == rule => {
                        assert!(assertion.participants.iter().all(|participant| {
                            participant.role == AssertionParticipantRole::EvaluatedEntity
                        }));
                        Some(assertion.reference.clone())
                    }
                    _ => None,
                }
            })
        })
        .expect("standards placement assertion");

    assert!(app.focus_all_intent_participants(&reference));
    assert_eq!(
        app.selected,
        Selection::FurnishingInstance(subject_id.0.clone())
    );
    assert_eq!(app.selected_component_count(), 2);
    assert!(app.component_is_selected(&ComponentKey::authored(
        AuthoredComponentKind::Room,
        room_id.0
    )));
    assert_eq!(app.focused_intent, Some(reference));
}

#[test]
fn intent_diagnostic_focus_selects_cross_object_primary_subject() {
    let mut app = FramerApp::default();
    let (subject_id, _) = prepare_intent_fixture(&mut app);
    create_intent(
        &mut app,
        IntentDraftExpression::Clearance {
            direction: ClearanceDirection::Front,
            datum: ClearanceDatum::FootprintFace,
            threshold: Length::from_feet(100.0),
        },
    );
    let diagnostic = app
        .project_plan
        .as_ref()
        .expect("plan")
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.starts_with("intent.assertion"))
        .expect("violated intent diagnostic");
    assert_eq!(diagnostic.source.as_ref(), Some(&subject_id));

    app.focus_diagnostic(super::panels::DiagnosticAction::Source(subject_id.clone()));
    assert_eq!(app.selected, Selection::FurnishingInstance(subject_id.0));
}

#[test]
fn deleting_subject_or_room_cascades_intent_and_override_atomically() {
    let mut app = FramerApp::default();
    let (subject_id, room_id) = prepare_intent_fixture(&mut app);
    let id = create_intent(&mut app, IntentDraftExpression::Containment);
    waive_intent(&mut app, &id);
    app.history.clear();

    app.apply_selection(
        Selection::FurnishingInstance(subject_id.0.clone()),
        None,
        SelectionOp::Replace,
    );
    app.delete_selected_furnishing_instance();
    assert!(app.model.furnishing_instances.is_empty());
    assert!(app.model.intents.is_empty());
    assert!(app.model.intent_overrides.is_empty());
    assert_eq!(app.history.undo_label(), Some("Delete furnishing instance"));
    app.undo();
    assert_eq!(app.model.furnishing_instances.len(), 1);
    assert_eq!(app.model.intents.len(), 1);
    assert_eq!(app.model.intent_overrides.len(), 1);

    app.history.clear();
    app.apply_selection(Selection::Room(room_id.0), None, SelectionOp::Replace);
    app.delete_selected_room();
    assert!(app.model.rooms.is_empty());
    assert!(app.model.intents.is_empty());
    assert!(app.model.intent_overrides.is_empty());
    assert_eq!(app.history.undo_label(), Some("Delete room"));
    app.undo();
    assert_eq!(app.model.rooms.len(), 1);
    assert_eq!(app.model.intents.len(), 1);
    assert_eq!(app.model.intent_overrides.len(), 1);
}

#[test]
fn edit_records_an_undo_step_and_undo_restores_the_model() {
    let mut app = FramerApp::default();
    let original = app.model.clone();

    app.edit("Rename wall", rename_first_wall);

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
    app.edit("Rename wall", rename_first_wall);
    let edited = app.model.clone();

    app.undo();
    app.redo();

    assert_eq!(app.model, edited, "redo should restore the post-edit model");
}

#[test]
fn interleaved_visibility_and_model_undos_rebuild_only_the_model_step() {
    let mut app = FramerApp::default();
    let original = app.model.clone();

    app.edit("Rename wall", rename_first_wall);
    let edited = app.model.clone();
    app.viewport_mode = ViewportMode::Axonometric;
    app.execute_action(ActionId::IsolateDim);
    assert!(app.component_visibility.isolation_mode().is_some());

    // Make solver activity observable instead of relying on deterministic plan
    // equality, which would pass whether or not `rebuild()` ran.
    app.project_plan = None;

    app.undo();
    assert_eq!(app.model, edited);
    assert_eq!(app.component_visibility.isolation_mode(), None);
    assert!(
        app.project_plan.is_none(),
        "presentation-only undo must not invoke the solver"
    );

    app.undo();
    assert_eq!(app.model, original);
    assert!(
        app.project_plan.is_some(),
        "the interleaved model undo must regenerate derived state"
    );
}

#[test]
fn undo_restores_the_prior_selection() {
    // `FramerApp::default()` already starts with the first wall selected — the
    // selection this test expects `undo` to restore after re-selecting a level.
    let mut app = FramerApp::default();
    assert_eq!(app.selected, Selection::Wall);

    app.edit("Adjust and reselect", |app| {
        rename_first_wall(app);
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
    app.edit("Rename wall", rename_first_wall);
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
fn add_wall_collinear_continuation_extends_existing_wall_in_one_undo_step() {
    let mut app = FramerApp::default();
    replace_walls_with_rectangular_footprint(&mut app, 10.0, 8.0);
    let before = app.model.clone();
    let walls_before = app.model.walls.len();

    // The authored top wall runs from (10,8) back to (0,8). Drawing outward
    // from its local start exercises the offset-shifting extension direction.
    app.add_wall(
        Point2::new(Length::from_feet(10.0), Length::from_feet(8.0)),
        Point2::new(Length::from_feet(20.0), Length::from_feet(8.0)),
    );

    assert_eq!(
        app.model.walls.len(),
        walls_before,
        "a straight continuation must not author a second physical wall section"
    );
    let extended = app
        .model
        .walls
        .iter()
        .find(|wall| wall.start.y == Length::from_feet(8.0) && wall.end.y == wall.start.y)
        .unwrap();
    assert_eq!(extended.length, Length::from_feet(20.0));
    assert_eq!(app.history.undo_label(), Some("Draw wall"));
    assert_eq!(
        app.project_plan.as_ref().unwrap().wall_plans.len(),
        walls_before,
        "regeneration must frame one extended wall rather than two restarted sections"
    );

    app.undo();
    assert_eq!(app.model, before, "undo restores the pre-extension wall");
}

#[test]
fn drawing_adjacent_room_extends_perimeter_and_reclassifies_old_corners_as_tees() {
    let mut app = FramerApp::default();
    replace_walls_with_rectangular_footprint(&mut app, 10.0, 8.0);
    assert_eq!(
        framer_core::enclosed_room_count(&app.model),
        1,
        "the starting footprint encloses one room"
    );

    // Add the three missing sides of a room to the right. The top and bottom
    // gestures continue existing perimeter walls; only the new outer wall is a
    // new authored wall. The old right wall becomes the shared partition.
    app.add_wall(
        Point2::new(Length::from_feet(10.0), Length::from_feet(8.0)),
        Point2::new(Length::from_feet(20.0), Length::from_feet(8.0)),
    );
    app.add_wall(
        Point2::new(Length::from_feet(20.0), Length::from_feet(8.0)),
        Point2::new(Length::from_feet(20.0), Length::ZERO),
    );
    app.add_wall(
        Point2::new(Length::from_feet(20.0), Length::ZERO),
        Point2::new(Length::from_feet(10.0), Length::ZERO),
    );

    assert_eq!(app.model.walls.len(), 5);
    assert_eq!(framer_core::enclosed_room_count(&app.model), 2);
    let tee_count = app
        .model
        .wall_joins
        .iter()
        .filter(|join| join.kind == framer_core::WallJoinKind::Tee)
        .count();
    assert_eq!(
        tee_count, 2,
        "the former outside corners become partition tees on the extended runs"
    );
    assert_eq!(app.project_plan.as_ref().unwrap().wall_plans.len(), 5);
}

#[test]
fn add_wall_auto_creates_corner_join_at_shared_endpoint() {
    let mut app = FramerApp::default();
    // Seed a free wall with only one wall at its far endpoint. Demo-shell
    // corners have two perpendicular walls, so any outward ortho gesture there
    // is correctly interpreted as continuing one of those perimeter walls.
    app.add_wall(
        Point2::new(Length::ZERO, Length::from_feet(30.0)),
        Point2::new(Length::from_feet(10.0), Length::from_feet(30.0)),
    );
    let joins_before = app.model.wall_joins.len();

    // A perpendicular wall drawn from the free wall's endpoint is a true corner,
    // not a straight continuation.
    app.add_wall(
        Point2::new(Length::from_feet(10.0), Length::from_feet(30.0)),
        Point2::new(Length::from_feet(10.0), Length::from_feet(40.0)),
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
fn add_ceiling_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    let before = app.model.ceilings.len();
    let systems_before = app.model.systems.len();

    // Seed inside the demo shell (28ft × 20ft); the region resolves to the
    // enclosed wall loop.
    let region = app
        .surface_region_at(Point2::new(
            Length::from_feet(14.0),
            Length::from_feet(10.0),
        ))
        .expect("the demo shell is an enclosed loop");
    app.add_ceiling(region);

    assert_eq!(app.model.ceilings.len(), before + 1);
    // The whole change is one undo step, the ceiling references a Ceiling-kind
    // system (reused if present, else seeded), and the model validates.
    let ceiling = app.model.ceilings.last().unwrap();
    let system = app
        .model
        .systems
        .iter()
        .find(|system| system.id == ceiling.system)
        .expect("the ceiling references an existing system");
    assert_eq!(system.kind, framer_core::SystemKind::Ceiling);
    assert!(
        app.model.validate().is_ok(),
        "authored ceiling must validate"
    );
    assert_eq!(app.history.undo_label(), Some("Add ceiling"));

    app.undo();
    assert_eq!(app.model.ceilings.len(), before, "undo removes the ceiling");
    assert_eq!(
        app.model.systems.len(),
        systems_before,
        "undo restores the system list"
    );
}

#[test]
fn add_ceiling_seeds_a_ceiling_system_when_absent() {
    let mut app = FramerApp::default();
    // Drop every ceiling system so the next placement must seed one.
    app.model
        .systems
        .retain(|system| system.kind != framer_core::SystemKind::Ceiling);
    let systems_before = app.model.systems.len();

    let region = app
        .surface_region_at(Point2::new(
            Length::from_feet(14.0),
            Length::from_feet(10.0),
        ))
        .expect("the demo shell is an enclosed loop");
    app.add_ceiling(region);

    assert_eq!(
        app.model.systems.len(),
        systems_before + 1,
        "a Ceiling system is seeded in the same step"
    );
    assert!(app.model.validate().is_ok(), "seeded ceiling must validate");
}

#[test]
fn add_floor_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    let before = app.model.floor_decks.len();

    let region = app
        .surface_region_at(Point2::new(
            Length::from_feet(14.0),
            Length::from_feet(10.0),
        ))
        .expect("the demo shell is an enclosed loop");
    app.add_floor(region);

    assert_eq!(app.model.floor_decks.len(), before + 1);
    assert!(app.model.validate().is_ok(), "authored floor must validate");
    assert_eq!(app.history.undo_label(), Some("Add floor"));

    app.undo();
    assert_eq!(app.model.floor_decks.len(), before, "undo removes the deck");
}

#[test]
fn delete_ceiling_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    let region = app
        .surface_region_at(Point2::new(
            Length::from_feet(14.0),
            Length::from_feet(10.0),
        ))
        .expect("the demo shell is an enclosed loop");
    app.add_ceiling(region);
    let id = match &app.selected {
        Selection::Ceiling(id) => id.clone(),
        other => panic!("expected the new ceiling selected, got {other:?}"),
    };

    app.delete_selected_ceiling();
    assert!(app.model.ceilings.iter().all(|ceiling| ceiling.id.0 != id));
    assert_eq!(app.history.undo_label(), Some("Delete ceiling"));

    app.undo();
    assert_eq!(app.model.ceilings.len(), 1, "undo restores the ceiling");
}

#[test]
fn add_gable_roof_generates_two_valid_planes_in_one_step() {
    let mut app = FramerApp::default();
    let before = app.model.roof_planes.len();

    app.add_roof(RoofForm::Gable);

    assert_eq!(
        app.model.roof_planes.len(),
        before + 2,
        "a gable is two opposing planes"
    );
    assert!(app.model.validate().is_ok(), "generated roof must validate");
    // Each plane references a Roof system, has a positive run, and an in-range
    // eave edge.
    for plane in &app.model.roof_planes {
        let system = app
            .model
            .systems
            .iter()
            .find(|system| system.id == plane.system)
            .expect("plane references an existing system");
        assert_eq!(system.kind, framer_core::SystemKind::Roof);
        assert!(plane.slope.run > Length::ZERO);
        assert!((plane.eave_edge as usize) < plane.outline.len());
        assert!(plane.frame().is_some(), "plane geometry is non-degenerate");
    }
    assert_eq!(app.history.undo_label(), Some("Add roof"));
    assert_eq!(
        app.viewport_mode,
        ViewportMode::RoofPlan,
        "adding a roof switches to the roof-plan view"
    );

    app.undo();
    assert_eq!(
        app.model.roof_planes.len(),
        before,
        "undo removes both planes in one step"
    );
}

#[test]
fn add_shed_roof_generates_one_plane() {
    let mut app = FramerApp::default();
    app.add_roof(RoofForm::Shed);
    assert_eq!(app.model.roof_planes.len(), 1, "a shed is a single plane");
    assert!(app.model.validate().is_ok(), "generated roof must validate");
}

#[test]
fn add_hip_roof_generates_four_valid_planes() {
    let mut app = FramerApp::default();
    let before = app.model.roof_planes.len();

    app.add_roof(RoofForm::Hip);

    let new_planes = &app.model.roof_planes[before..];
    assert_eq!(new_planes.len(), 4, "a hip roof is four stored planes");
    assert_eq!(
        new_planes
            .iter()
            .filter(|plane| plane.outline.len() == 4)
            .count(),
        2,
        "the long sides are trapezoids"
    );
    assert_eq!(
        new_planes
            .iter()
            .filter(|plane| plane.outline.len() == 3)
            .count(),
        2,
        "the short ends are hip triangles"
    );
    assert!(
        app.model.validate().is_ok(),
        "generated hip roof must validate"
    );
    assert!(new_planes.iter().all(|plane| plane.frame().is_some()));
    assert_eq!(app.history.undo_label(), Some("Add roof"));

    app.undo();
    assert_eq!(
        app.model.roof_planes.len(),
        before,
        "undo removes all four hip planes in one step"
    );
}

#[test]
fn add_hip_roof_on_l_footprint_generates_valley_planes() {
    let mut app = FramerApp::default();
    replace_walls_with_l_footprint(&mut app);

    app.add_roof(RoofForm::Hip);

    assert_eq!(
        app.model.roof_planes.len(),
        2,
        "a simple L footprint starts with the two stored valley planes"
    );
    assert!(
        app.model.validate().is_ok(),
        "generated L-footprint valley roof must validate"
    );
    let plan = framer_solver::generate_project_plan(&app.model).unwrap();
    let valleys: Vec<_> = plan
        .roof_plans
        .iter()
        .flat_map(|roof| roof.members.iter())
        .filter(|member| member.kind == framer_solver::MemberKind::ValleyRafter)
        .collect();
    assert_eq!(valleys.len(), 1, "the generated planes share one valley");
    for roof in &plan.roof_plans {
        assert!(
            roof.members
                .iter()
                .any(|member| member.kind == framer_solver::MemberKind::JackRafter),
            "each generated valley plane has jack rafters clipped to the diagonal"
        );
    }
    assert_eq!(app.history.undo_label(), Some("Add roof"));

    app.undo();
    assert!(
        app.model.roof_planes.is_empty(),
        "undo removes the generated L-footprint valley planes"
    );
}

#[test]
fn add_square_hip_roof_generates_four_triangular_planes() {
    let mut app = FramerApp::default();
    replace_walls_with_rectangular_footprint(&mut app, 20.0, 20.0);

    app.add_roof(RoofForm::Hip);

    assert_eq!(
        app.model.roof_planes.len(),
        4,
        "a square hip has four planes"
    );
    assert!(
        app.model
            .roof_planes
            .iter()
            .all(|plane| plane.outline.len() == 3),
        "a square hip degenerates to four triangles meeting at one peak"
    );
    assert!(
        app.model.validate().is_ok(),
        "generated square hip roof must validate"
    );
    assert!(
        app.model
            .roof_planes
            .iter()
            .all(|plane| plane.frame().is_some()),
        "every square hip plane has a usable frame"
    );
}

#[test]
fn add_portrait_hip_roof_generates_four_valid_planes() {
    let mut app = FramerApp::default();
    replace_walls_with_rectangular_footprint(&mut app, 20.0, 28.0);

    app.add_roof(RoofForm::Hip);

    assert_eq!(
        app.model.roof_planes.len(),
        4,
        "a portrait hip has four planes"
    );
    assert_eq!(
        app.model
            .roof_planes
            .iter()
            .filter(|plane| plane.outline.len() == 4)
            .count(),
        2,
        "the long sides are trapezoids"
    );
    assert_eq!(
        app.model
            .roof_planes
            .iter()
            .filter(|plane| plane.outline.len() == 3)
            .count(),
        2,
        "the short ends are hip triangles"
    );
    assert!(
        app.model.validate().is_ok(),
        "generated portrait hip roof must validate"
    );
    assert!(
        app.model
            .roof_planes
            .iter()
            .all(|plane| plane.frame().is_some()),
        "every portrait hip plane has a usable frame"
    );
}

#[test]
fn add_roof_without_walls_is_a_no_op() {
    let mut app = FramerApp::default();
    while !app.model.walls.is_empty() {
        app.selected = Selection::Wall;
        app.selected_wall = 0;
        app.delete_selected_wall();
    }
    let undo_before = app.history.can_undo();

    app.add_roof(RoofForm::Gable);

    assert!(
        app.model.roof_planes.is_empty(),
        "no footprint -> no roof planes"
    );
    assert_eq!(
        app.history.can_undo(),
        undo_before,
        "a no-op roof records no undo step"
    );
}

#[test]
fn surface_placed_in_a_room_attaches_to_that_room() {
    let mut app = FramerApp::default();
    let seed = Point2::new(Length::from_feet(14.0), Length::from_feet(10.0));
    app.add_room(seed);
    let room_id = match &app.selected {
        Selection::Room(id) => id.clone(),
        other => panic!("expected the new room selected, got {other:?}"),
    };

    // A ceiling/floor placed over a loop that already holds a room references
    // that room (so the surface tracks the room as walls move), not a frozen
    // polygon.
    let region = app
        .surface_region_at(seed)
        .expect("the demo shell is an enclosed loop");
    assert!(
        matches!(&region, framer_core::SurfaceRegion::Room(id) if id.0 == room_id),
        "region should reference the room, got {region:?}"
    );
    app.add_ceiling(region);
    let ceiling = app.model.ceilings.last().unwrap();
    assert!(
        matches!(&ceiling.region, framer_core::SurfaceRegion::Room(id) if id.0 == room_id),
        "ceiling should attach to the room"
    );
    assert!(app.model.validate().is_ok());
}

#[test]
fn delete_floor_deck_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    let region = app
        .surface_region_at(Point2::new(
            Length::from_feet(14.0),
            Length::from_feet(10.0),
        ))
        .expect("the demo shell is an enclosed loop");
    app.add_floor(region);
    let id = match &app.selected {
        Selection::FloorDeck(id) => id.clone(),
        other => panic!("expected the new floor deck selected, got {other:?}"),
    };

    app.delete_selected_floor_deck();
    assert!(app.model.floor_decks.iter().all(|deck| deck.id.0 != id));
    assert_eq!(app.history.undo_label(), Some("Delete floor deck"));

    app.undo();
    assert_eq!(app.model.floor_decks.len(), 1, "undo restores the deck");
}

#[test]
fn delete_roof_plane_is_a_single_undoable_step() {
    let mut app = FramerApp::default();
    app.add_roof(RoofForm::Gable);
    let id = match &app.selected {
        Selection::RoofPlane(id) => id.clone(),
        other => panic!("expected the new roof plane selected, got {other:?}"),
    };
    let before = app.model.roof_planes.len();

    app.delete_selected_roof_plane();
    assert_eq!(app.model.roof_planes.len(), before - 1);
    assert!(app.model.roof_planes.iter().all(|plane| plane.id.0 != id));
    assert_eq!(app.history.undo_label(), Some("Delete roof plane"));

    app.undo();
    assert_eq!(
        app.model.roof_planes.len(),
        before,
        "undo restores the deleted roof plane"
    );
}

#[test]
fn referenced_surface_systems_cannot_be_deleted() {
    let mut app = FramerApp::default();
    let region = app
        .surface_region_at(Point2::new(
            Length::from_feet(14.0),
            Length::from_feet(10.0),
        ))
        .expect("the demo shell is an enclosed loop");
    app.add_ceiling(region.clone());
    app.add_floor(region);
    app.add_roof(RoofForm::Shed);

    // Each surface's referenced system must be undeletable (deleting it would
    // dangle the surface's `system`) — the widened deletion guard covers roofs,
    // ceilings, and floors, not just walls.
    let referenced: Vec<String> = [
        app.model.ceilings.last().map(|c| c.system.0.clone()),
        app.model.floor_decks.last().map(|d| d.system.0.clone()),
        app.model.roof_planes.last().map(|p| p.system.0.clone()),
    ]
    .into_iter()
    .flatten()
    .collect();
    assert_eq!(referenced.len(), 3, "all three surfaces were placed");

    for system_id in referenced {
        app.selected = Selection::System(system_id.clone());
        let before = app.model.systems.len();
        app.delete_selected_system();
        assert_eq!(
            app.model.systems.len(),
            before,
            "a referenced surface system must not be deleted"
        );
        assert!(
            app.model
                .systems
                .iter()
                .any(|system| system.id.0 == system_id),
            "the referenced system {system_id} is still present"
        );
    }
}

#[test]
fn roof_springing_falls_back_to_tallest_wall_without_level_height() {
    let mut app = FramerApp::default();
    // The demo level has no authored height, so the springing line falls back to
    // the tallest wall top (elevation + max wall height), never `Length::ZERO`.
    assert_eq!(app.model.levels[0].height, Length::ZERO);
    let elevation = app.model.levels[0].elevation;
    app.model.walls[0].height = Length::from_feet(12.0); // make one wall clearly tallest

    app.add_roof(RoofForm::Shed);

    let plane = app.model.roof_planes.last().unwrap();
    assert_eq!(
        plane.reference_elevation,
        elevation + Length::from_feet(12.0),
        "springing follows the tallest wall when the level has no height"
    );
    assert!(plane.reference_elevation > Length::ZERO);
}

#[test]
fn roof_springing_uses_level_top_when_height_authored() {
    let mut app = FramerApp::default();
    // An authored level height defines the top plane directly, taking precedence
    // over the wall-height fallback.
    app.model.levels[0].height = Length::from_feet(10.0);
    let elevation = app.model.levels[0].elevation;

    app.add_roof(RoofForm::Shed);

    let plane = app.model.roof_planes.last().unwrap();
    assert_eq!(
        plane.reference_elevation,
        elevation + Length::from_feet(10.0),
        "springing is the authored level top"
    );
}

#[test]
fn gable_over_a_tall_footprint_uses_the_y_longer_branch() {
    let mut app = FramerApp::default();
    // Clear the wider-than-tall demo shell and lay a footprint that is taller than
    // it is wide (10ft × 30ft), so the ridge runs along x and the eave edges are
    // 3 and 1 (the branch the demo shell never exercises).
    while !app.model.walls.is_empty() {
        app.selected = Selection::Wall;
        app.selected_wall = 0;
        app.delete_selected_wall();
    }
    let corners = [
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 30.0),
        (0.0, 30.0),
        (0.0, 0.0),
    ];
    for pair in corners.windows(2) {
        app.add_wall(
            Point2::new(Length::from_feet(pair[0].0), Length::from_feet(pair[0].1)),
            Point2::new(Length::from_feet(pair[1].0), Length::from_feet(pair[1].1)),
        );
    }

    app.add_roof(RoofForm::Gable);

    assert_eq!(app.model.roof_planes.len(), 2, "a gable is two planes");
    let mut eaves: Vec<u32> = app
        .model
        .roof_planes
        .iter()
        .map(|plane| plane.eave_edge)
        .collect();
    eaves.sort_unstable();
    assert_eq!(
        eaves,
        vec![1, 3],
        "a y-longer gable's planes eave on edges 1 and 3"
    );
    assert!(app.model.validate().is_ok());
    for plane in &app.model.roof_planes {
        assert!(
            (plane.eave_edge as usize) < plane.outline.len(),
            "eave edge in range"
        );
        assert!(plane.frame().is_some(), "plane geometry is non-degenerate");
    }
}

#[test]
fn add_opening_preserves_skylight_and_stair_kinds() {
    // The un-fork: Skylight/Stair openings keep their kind instead of being
    // coerced to Window (BOM and render dispatch on opening.kind). A fresh app +
    // free wall per kind keeps the single opening clear of the demo openings.
    for kind in [OpeningKind::Skylight, OpeningKind::Stair] {
        let mut app = FramerApp::default();
        app.add_wall(
            Point2::new(Length::from_feet(0.0), Length::from_feet(40.0)),
            Point2::new(Length::from_feet(20.0), Length::from_feet(40.0)),
        );
        app.selected_wall = app.model.walls.len() - 1;
        app.selected = Selection::Wall;

        app.add_opening(kind);
        let id = match &app.selected {
            Selection::Opening(id) => id.clone(),
            other => panic!("expected the new opening selected, got {other:?}"),
        };
        let opening = app.model.walls[app.selected_wall]
            .openings
            .iter()
            .find(|opening| opening.id.0 == id)
            .expect("the new opening exists");
        assert_eq!(opening.kind, kind, "{kind:?} opening keeps its kind");
        assert!(app.model.validate().is_ok(), "{kind:?} opening validates");
    }
}

#[test]
fn region_tools_are_mutually_exclusive() {
    let mut app = FramerApp::default();

    app.toggle_ceiling_tool();
    assert!(app.ceiling_tool_active, "ceiling tool activates");
    assert!(
        !app.floor_tool_active
            && !app.room_tool_active
            && !app.draw_wall_tool.active
            && !app.dimension_tool.active,
        "activating the ceiling tool cancels every other placement tool"
    );

    app.toggle_floor_tool();
    assert!(app.floor_tool_active, "floor tool activates");
    assert!(!app.ceiling_tool_active, "activating floor cancels ceiling");

    app.toggle_room_tool();
    assert!(app.room_tool_active, "room tool activates");
    assert!(!app.floor_tool_active, "activating room cancels floor");

    // Toggling the active tool off leaves nothing active.
    app.toggle_room_tool();
    assert!(!app.room_tool_active, "toggling off deactivates the tool");
}

#[test]
fn c_and_f_keys_toggle_ceiling_and_floor_tools() {
    let mut app = FramerApp::default();

    press_key(&mut app, egui::Key::C, egui::Modifiers::NONE);
    assert!(
        app.ceiling_tool_active,
        "C routes through to the ceiling tool"
    );

    press_key(&mut app, egui::Key::F, egui::Modifiers::NONE);
    assert!(app.floor_tool_active, "F routes through to the floor tool");
    assert!(!app.ceiling_tool_active, "F cancels the ceiling tool");

    press_key(&mut app, egui::Key::F, egui::Modifiers::NONE);
    assert!(!app.floor_tool_active, "F again deactivates the floor tool");
}

#[test]
fn v_key_toggles_the_vault_tool() {
    let mut app = FramerApp::default();

    press_key(&mut app, egui::Key::V, egui::Modifiers::NONE);
    assert!(app.vault_tool_active, "V routes through to the vault tool");

    // A sibling region-tool key cancels it, and V again deactivates.
    press_key(&mut app, egui::Key::C, egui::Modifiers::NONE);
    assert!(!app.vault_tool_active, "C cancels the vault tool");
    press_key(&mut app, egui::Key::V, egui::Modifiers::NONE);
    assert!(app.vault_tool_active);
    press_key(&mut app, egui::Key::V, egui::Modifiers::NONE);
    assert!(!app.vault_tool_active, "V again deactivates the vault tool");
}

#[test]
fn scissor_halves_rejects_a_degenerate_region() {
    // A zero-area region (collinear points → zero depth) cannot be vaulted; the guard
    // returns None so add_vault's "too small to vault" early-return fires instead of
    // authoring a degenerate ceiling.
    let degenerate = vec![
        Point2::new(Length::ZERO, Length::ZERO),
        Point2::new(Length::from_feet(10.0), Length::ZERO),
        Point2::new(Length::from_feet(20.0), Length::ZERO),
    ];
    assert!(super::scissor_halves(&degenerate).is_none());

    // add_vault on a degenerate outline authors nothing and records no undo step.
    let mut app = FramerApp::default();
    let before = app.model.ceilings.len();
    app.add_vault(&degenerate);
    assert_eq!(
        app.model.ceilings.len(),
        before,
        "a degenerate region is not vaulted"
    );
    assert!(
        !app.history.can_undo(),
        "a rejected vault records no undo step"
    );
}

#[test]
fn place_ceiling_and_floor_clicks_route_and_gate_on_the_tool() {
    let mut app = FramerApp::default();
    let seed = Point2::new(Length::from_feet(14.0), Length::from_feet(10.0));

    // Gate: a PlaceCeiling/PlaceFloor click with the tool inactive is a no-op.
    app.handle_view_click(ViewClick::PlaceCeiling { point: seed });
    app.handle_view_click(ViewClick::PlaceFloor { point: seed });
    assert!(
        app.model.ceilings.is_empty() && app.model.floor_decks.is_empty(),
        "placement clicks do nothing while the tool is inactive"
    );

    // Routing: with the matching tool active, the click drops the surface.
    app.toggle_ceiling_tool();
    app.handle_view_click(ViewClick::PlaceCeiling { point: seed });
    assert_eq!(
        app.model.ceilings.len(),
        1,
        "PlaceCeiling routes to add_ceiling"
    );

    app.toggle_floor_tool();
    app.handle_view_click(ViewClick::PlaceFloor { point: seed });
    assert_eq!(
        app.model.floor_decks.len(),
        1,
        "PlaceFloor routes to add_floor"
    );
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
    app.edit("Rename wall", rename_first_wall);
    assert_ne!(app.model, original);

    press_key(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
    assert_eq!(app.model, original, "Cmd/Ctrl+Z must undo");
}

#[test]
fn cmd_shift_z_redoes_via_keyboard() {
    let mut app = FramerApp::default();
    let original = app.model.clone();
    app.edit("Rename wall", rename_first_wall);
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
    let code = FramingDefaults::irc_2021_starter();
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
    assert_eq!(
        app.model.walls[0].end,
        Point2::new(Length::from_feet(12.0), Length::ZERO)
    );
    assert_eq!(app.model.walls[0].length, Length::from_feet(12.0));
    // The shared corner and its join are untouched.
    assert_eq!(
        app.model.walls[1].start,
        Point2::new(Length::ZERO, Length::ZERO)
    );
    assert_eq!(app.model.wall_joins.len(), 1);

    // The whole drag undoes in a single step.
    app.undo();
    assert_eq!(app.model, original);
    assert!(!app.history.can_undo());
}

#[test]
fn wall_endpoint_drag_drags_shared_node_along_a_collinear_run() {
    let mut app = FramerApp::default();
    let code = FramingDefaults::irc_2021_starter();
    let pt = |x: f64, y: f64| Point2::new(Length::from_feet(x), Length::from_feet(y));
    let wall = |id: &str, start, end| {
        Wall::new(id, id, Length::from_feet(1.0), &code).with_placement("level-1", start, end)
    };
    app.model.walls.clear();
    app.model.wall_joins.clear();
    app.model.rooms.clear();
    app.model.walls.push(wall("a", pt(0.0, 0.0), pt(10.0, 0.0)));
    app.model
        .walls
        .push(wall("b", pt(10.0, 0.0), pt(20.0, 0.0)));
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
    assert_eq!(
        app.model.walls[0].end,
        Point2::new(Length::from_feet(12.0), Length::ZERO)
    );
    assert_eq!(
        app.model.walls[1].start,
        Point2::new(Length::from_feet(12.0), Length::ZERO)
    );
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
    assert_eq!(
        app.model.walls[0].start,
        Point2::new(Length::ZERO, Length::from_feet(2.0))
    );
    assert_eq!(
        app.model.walls[0].end,
        Point2::new(Length::from_feet(10.0), Length::from_feet(2.0))
    );
    assert_eq!(
        app.model.walls[1].start,
        Point2::new(Length::ZERO, Length::from_feet(2.0))
    );
    assert_eq!(app.model.walls[1].length, Length::from_feet(6.0));

    app.undo();
    assert_eq!(app.model, original);
}

/// The vault tool authors a scissor/vault as two opposing sloped ceilings in one
/// undoable step, and the result round-trips with its slopes intact.
#[test]
fn add_vault_authors_two_opposing_sloped_ceilings() {
    let mut app = FramerApp::default();
    let before = app.model.ceilings.len();
    let outline = framer_core::room_boundary(
        &app.model,
        Point2::new(Length::from_feet(14.0), Length::from_feet(10.0)),
    )
    .expect("the demo shell is an enclosed loop")
    .vertices;
    app.add_vault(&outline);

    assert_eq!(
        app.model.ceilings.len(),
        before + 2,
        "a vault is two opposing ceilings"
    );
    let vault: Vec<_> = app.model.ceilings.iter().rev().take(2).collect();
    assert!(
        vault.iter().all(|c| c.slope.is_some()),
        "both halves are sloped"
    );
    assert!(
        vault
            .iter()
            .all(|c| matches!(c.region, framer_core::SurfaceRegion::Polygon(_))),
        "both halves are explicit polygons (a sloped ceiling needs one)"
    );
    assert_ne!(
        vault[0].region, vault[1].region,
        "the two halves are opposing, not identical"
    );
    assert!(
        app.model.validate().is_ok(),
        "the authored vault must validate"
    );

    // Round-trips through save/load with both slopes intact.
    let json = framer_core::save_project(&app.model).expect("the vaulted model saves");
    let reloaded = framer_core::load_project(&json).expect("the vaulted model loads");
    assert_eq!(
        reloaded
            .ceilings
            .iter()
            .filter(|c| c.slope.is_some())
            .count(),
        2,
        "both sloped ceilings survive a round-trip"
    );

    // One undoable step.
    assert_eq!(app.history.undo_label(), Some("Add vault"));
    app.undo();
    assert_eq!(
        app.model.ceilings.len(),
        before,
        "undo removes both vault halves in one step"
    );
}

/// The vault frames a *rising tent*: each half rises from its spring wall to a
/// shared, higher ridge — not an inverted vault. Pinned through the solver, since
/// validation only checks `low_edge` bounds and would pass an inverted half.
#[test]
fn a_vault_frames_a_rising_tent() {
    let mut app = FramerApp::default();
    let outline = framer_core::room_boundary(
        &app.model,
        Point2::new(Length::from_feet(14.0), Length::from_feet(10.0)),
    )
    .expect("the demo shell is an enclosed loop")
    .vertices;
    app.add_vault(&outline);
    let plan = app.project_plan.as_ref().expect("the vault re-solves");

    let mut ridge_elevations = Vec::new();
    for ceiling in &app.model.ceilings {
        let ceiling_plan = plan
            .ceiling_plan(&framer_core::ElementId::new(ceiling.id.0.clone()))
            .expect("each vault half has a ceiling plan");
        // The common ceiling joists run up the slope (the band joists/blocking are
        // level at their edge, so they are excluded).
        let joists: Vec<_> = ceiling_plan
            .members
            .iter()
            .filter(|member| member.kind == framer_solver::MemberKind::CeilingJoist)
            .filter_map(|member| member.sloped)
            .collect();
        assert!(!joists.is_empty(), "a sloped vault half frames joists");
        // Every joist rises: its ridge (high) end is above its spring (low) end.
        assert!(
            joists.iter().all(|s| s.high_elevation > s.low_elevation),
            "the vault rises from its spring wall; it does not invert"
        );
        ridge_elevations.push(joists[0].high_elevation);
    }
    // The two opposing halves meet at one shared ridge elevation.
    assert_eq!(ridge_elevations.len(), 2);
    assert_eq!(
        ridge_elevations[0], ridge_elevations[1],
        "the two halves meet at the same ridge elevation"
    );
}

/// A vault click does nothing unless the tool is active and the point is inside a
/// closed wall loop.
#[test]
fn vault_click_gates_on_the_tool_and_an_enclosed_loop() {
    let mut app = FramerApp::default();
    let before = app.model.ceilings.len();
    let inside = Point2::new(Length::from_feet(14.0), Length::from_feet(10.0));

    // Tool inactive → ignored.
    app.handle_place_vault(inside);
    assert_eq!(
        app.model.ceilings.len(),
        before,
        "no vault while the tool is inactive"
    );

    app.toggle_vault_tool();
    assert!(app.vault_tool_active, "the vault tool activates");

    // Active but outside any loop → no vault.
    app.handle_place_vault(Point2::new(
        Length::from_feet(500.0),
        Length::from_feet(500.0),
    ));
    assert_eq!(
        app.model.ceilings.len(),
        before,
        "no vault outside an enclosed loop"
    );

    // Active and inside the loop → authors the vault.
    app.handle_place_vault(inside);
    assert_eq!(
        app.model.ceilings.len(),
        before + 2,
        "a click inside the loop vaults it"
    );
}

/// The scissor split divides the bounding box along its longer span: a 28×20 region
/// ridges along x at the mid-depth, with halves springing from the y=0 / y=20 walls
/// (edge 0) and meeting at the ridge.
#[test]
fn scissor_halves_splits_along_the_longer_span() {
    let ft = Length::from_feet;
    let outline = vec![
        Point2::new(Length::ZERO, Length::ZERO),
        Point2::new(ft(28.0), Length::ZERO),
        Point2::new(ft(28.0), ft(20.0)),
        Point2::new(Length::ZERO, ft(20.0)),
    ];
    let (low, high) = super::scissor_halves(&outline).expect("a vaultable rectangle");

    // Edge 0 (verts 0..1) of each half is its outer spring wall.
    assert_eq!(low[0].y, Length::ZERO, "low half springs from the y=0 wall");
    assert_eq!(low[1].y, Length::ZERO);
    assert_eq!(high[0].y, ft(20.0), "high half springs from the y=20 wall");
    assert_eq!(high[1].y, ft(20.0));
    // Both reach the shared ridge at y = 10ft.
    let mid = ft(10.0);
    assert!(
        low.iter().any(|p| p.y == mid) && high.iter().any(|p| p.y == mid),
        "the halves meet at the ridge"
    );
}

/// The portrait branch: a 20×28 region (depth > width) ridges along y at the
/// mid-width, with halves springing from the x=0 / x=20 walls (edge 0). Pins the
/// second `scissor_halves` branch — model validation only checks `low_edge` bounds,
/// so an inverted vault here would otherwise validate silently.
#[test]
fn scissor_halves_splits_a_portrait_region_along_y() {
    let ft = Length::from_feet;
    let outline = vec![
        Point2::new(Length::ZERO, Length::ZERO),
        Point2::new(ft(20.0), Length::ZERO),
        Point2::new(ft(20.0), ft(28.0)),
        Point2::new(Length::ZERO, ft(28.0)),
    ];
    let (low, high) = super::scissor_halves(&outline).expect("a vaultable rectangle");

    // Edge 0 (verts 0..1) of each half is its outer spring wall (x = min / max).
    assert_eq!(low[0].x, Length::ZERO, "low half springs from the x=0 wall");
    assert_eq!(low[1].x, Length::ZERO);
    assert_eq!(high[0].x, ft(20.0), "high half springs from the x=20 wall");
    assert_eq!(high[1].x, ft(20.0));
    // Both reach the shared ridge at x = 10ft.
    let mid = ft(10.0);
    assert!(
        low.iter().any(|p| p.x == mid) && high.iter().any(|p| p.x == mid),
        "the halves meet at the ridge"
    );
}

/// Arming the vault tool and then any sibling region tool (or vice versa) leaves
/// exactly one active — so a click never authors a vault under the wrong tool.
#[test]
fn the_vault_tool_is_mutually_exclusive_with_the_other_region_tools() {
    let mut app = FramerApp::default();

    // Each sibling cancels an armed vault tool.
    for activate_sibling in [
        FramerApp::toggle_room_tool as fn(&mut FramerApp),
        FramerApp::toggle_ceiling_tool,
        FramerApp::toggle_floor_tool,
    ] {
        app.toggle_vault_tool();
        assert!(app.vault_tool_active, "vault tool armed");
        activate_sibling(&mut app);
        assert!(
            !app.vault_tool_active,
            "arming another region tool must cancel the vault tool"
        );
    }

    // And arming the vault tool cancels every other region tool.
    app.toggle_room_tool();
    app.toggle_vault_tool();
    assert!(
        app.vault_tool_active
            && !app.room_tool_active
            && !app.ceiling_tool_active
            && !app.floor_tool_active,
        "the vault tool cancels the other region tools"
    );
}
