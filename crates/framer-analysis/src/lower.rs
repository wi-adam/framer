use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use framer_core::{
    Applicability, AuthoredEntityRef, AuthoredIntentMode, BuildingModel, CheckSeverity,
    DimensionKind, ElementId, ExactIntentScope, Fact, IntentAssertion, IntentExpression,
    IntentOverride, IntentSource, ProjectIntentScope, PropertyValue, ResolvedStandards,
};
use framer_geometry::{GeometryAudit, GeometryViolation};
use framer_solver::{DiagnosticSeverity, PlanDiagnostic, ProjectFramePlan, RuleRef};
use framer_standards::{
    ComplianceEntry, ComplianceReport, FactSnapshot, FactSubject, FactUnknownKind, Outcome,
    PlacedObjectRef, PredicateObservation, StandardsEvaluation, StandardsEvaluationDetail,
    SyntheticEntryKind, Tri,
};

use crate::{
    AssertionParticipant, AssertionParticipantRole, AssertionRef, AssertionScope, AssertionSource,
    AssumptionEvidence, AssumptionIntentRecord, AssumptionPremise, BooleanExpression,
    BooleanIntentMode, BooleanIntentRecord, CompiledAssertion, ComplianceEntryRef,
    DerivedAssertionId, DerivedAssertionProvider, DerivedAssertionRole, DerivedAssertionSource,
    DiagnosticProvider, DiagnosticRef, GraphRevision, IntentDomain, IntentEvidenceRef,
    IntentOutcome, IntentRecord, IntentReport, IntentUnknown, IntentUnknownKind, IntentValue,
    PhysicalBodyRef, PreferencePriority, SelectionAttribute, SiteAssumptionKey, StandardsRuleRef,
    WaiverRecord, WaiverRef,
};

#[cfg(test)]
pub(crate) fn compile_current_intent(
    model: &BuildingModel,
    revision: GraphRevision,
) -> IntentReport {
    IntentReport::from_parts(
        revision,
        current_intent_records(model, revision, &BTreeMap::new(), &BTreeSet::new()),
        Vec::new(),
    )
}

type DiagnosticEvidenceIndex =
    BTreeMap<(DiagnosticProvider, String, Option<AuthoredEntityRef>), Vec<DiagnosticRef>>;

/// One exact, revision-neutral evaluation of a persisted authored assertion.
///
/// The predicate result is produced by `framer-standards::FactSnapshot`; analysis only adapts it
/// to the common outcome/evidence and diagnostics protocols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthoredIntentEvaluation {
    assertion: IntentAssertion,
    subject: FactSubject,
    predicate: PredicateObservation,
    outcome: IntentOutcome,
    project_waiver: Option<(framer_core::IntentOverrideId, IntentSource, String)>,
}

pub(crate) fn evaluate_authored_intent(
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> Vec<AuthoredIntentEvaluation> {
    let snapshot = FactSnapshot::new(model, resolved, plan);
    let mut evaluations = model
        .intents
        .iter()
        .filter_map(|assertion| {
            let ProjectIntentScope::Exact(ExactIntentScope {
                subject,
                participants,
            }) = &assertion.scope;
            let object = PlacedObjectRef::from_authored(subject)?;
            let room = participants
                .iter()
                .find_map(|participant| match participant {
                    AuthoredEntityRef::Room(room) => Some(room.clone()),
                    _ => None,
                })?;
            let subject = FactSubject::placed_object_exact(object, room);
            let IntentExpression::FactPredicate(expression) = &assertion.expression;
            let predicate = snapshot.evaluate_predicate(expression, &subject);
            let project_waiver = model.intent_overrides.iter().find_map(|intent_override| {
                let IntentOverride::Waive {
                    id,
                    target,
                    reason,
                    source,
                } = intent_override;
                (target == &assertion.id).then(|| (id.clone(), *source, reason.clone()))
            });
            let outcome = if let Some((override_id, _, reason)) = &project_waiver {
                IntentOutcome::Waived {
                    waiver: WaiverRef::Project {
                        override_id: override_id.clone(),
                    },
                    reason: reason.clone(),
                }
            } else {
                match predicate.result {
                    Tri::True => IntentOutcome::Satisfied,
                    Tri::False => IntentOutcome::Violated,
                    Tri::Unknown => IntentOutcome::Unknown(unknown_from_predicate(&predicate)),
                }
            };
            Some(AuthoredIntentEvaluation {
                assertion: assertion.clone(),
                subject,
                predicate,
                outcome,
                project_waiver,
            })
        })
        .collect::<Vec<_>>();
    evaluations.sort_by(|left, right| left.assertion.id.cmp(&right.assertion.id));
    evaluations
}

fn unknown_from_predicate(predicate: &PredicateObservation) -> IntentUnknown {
    let unknown = predicate.observed_facts.iter().find_map(|observed| {
        let framer_standards::FactObservation::Unknown(unknown) = &observed.observation else {
            return None;
        };
        Some(unknown)
    });
    let Some(unknown) = unknown else {
        return IntentUnknown {
            kind: IntentUnknownKind::EvaluationUnavailable,
            detail: "The shared predicate evaluator returned unknown without an unavailable fact."
                .to_owned(),
        };
    };
    IntentUnknown {
        kind: match unknown.kind {
            FactUnknownKind::MissingInput => IntentUnknownKind::MissingInput,
            FactUnknownKind::UnresolvedSubject => IntentUnknownKind::UnresolvedSubject,
            FactUnknownKind::WrongSubjectKind => IntentUnknownKind::WrongSubjectKind,
            FactUnknownKind::UnsupportedCondition => IntentUnknownKind::UnsupportedCondition,
        },
        detail: format!(
            "Fact {:?} is unavailable for {:?} ({:?}).",
            unknown.fact, unknown.subject, unknown.kind
        ),
    }
}

fn current_intent_records(
    model: &BuildingModel,
    revision: GraphRevision,
    diagnostics: &DiagnosticEvidenceIndex,
    referenced_site_properties: &BTreeSet<String>,
) -> Vec<IntentRecord> {
    let mut records = Vec::new();
    compile_driving_dimensions(model, revision, diagnostics, &mut records);
    compile_construction_selections(model, revision, &mut records);
    compile_site_premises(model, revision, referenced_site_properties, &mut records);
    records
}

fn compile_driving_dimensions(
    model: &BuildingModel,
    revision: GraphRevision,
    diagnostics: &DiagnosticEvidenceIndex,
    records: &mut Vec<IntentRecord>,
) {
    for wall in &model.walls {
        for dimension in &wall.dimensions {
            if dimension.kind != DimensionKind::Driving {
                continue;
            }
            let wall_ref = AuthoredEntityRef::Wall(wall.id.clone());
            let dimension_ref = AuthoredEntityRef::Dimension(dimension.id.clone());
            let observed = wall.dimension_measurement(dimension);
            let outcome = match dimension.value {
                Some(_) if wall.is_driving_dimension_satisfied(dimension) => {
                    IntentOutcome::Satisfied
                }
                Some(_) => IntentOutcome::Violated,
                None => IntentOutcome::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::MissingInput,
                    detail: "Driving dimension has no target value.".to_owned(),
                }),
            };
            let expression = match dimension.value {
                Some(expected) => BooleanExpression::ExactLength {
                    label: dimension.name.clone(),
                    expected,
                    observed,
                },
                None => BooleanExpression::Finding {
                    code: "intent.dimension.missing-value".to_owned(),
                },
            };
            let mut evidence = vec![
                IntentEvidenceRef::Authored(dimension_ref.clone()),
                IntentEvidenceRef::Authored(wall_ref.clone()),
            ];
            let diagnostic_code = match (&outcome, dimension.value) {
                (IntentOutcome::Violated, Some(_)) => Some("intent.dimension.unsatisfied"),
                (IntentOutcome::Unknown(_), None) => Some("intent.dimension.missing-value"),
                _ => None,
            };
            if let Some(reference) = diagnostic_code.and_then(|code| {
                diagnostics
                    .get(&(
                        DiagnosticProvider::Analysis,
                        code.to_owned(),
                        Some(dimension_ref.clone()),
                    ))
                    .and_then(|matches| matches.first())
            }) {
                evidence.push(IntentEvidenceRef::Diagnostic(reference.clone()));
            }
            records.push(IntentRecord::Boolean(BooleanIntentRecord {
                assertion: CompiledAssertion {
                    reference: AssertionRef::Derived(DerivedAssertionId::new(
                        revision,
                        DerivedAssertionProvider::Core,
                        DerivedAssertionSource::Authored(dimension_ref.clone()),
                        DerivedAssertionRole::DrivingDimension,
                    )),
                    domain: IntentDomain::SpatialProgram,
                    scope: AssertionScope::Exact(vec![wall_ref.clone(), dimension_ref.clone()]),
                    participants: vec![
                        AssertionParticipant::new(
                            wall_ref.clone(),
                            AssertionParticipantRole::Host,
                            0,
                        ),
                        AssertionParticipant::new(
                            dimension_ref.clone(),
                            AssertionParticipantRole::Constraint,
                            1,
                        ),
                    ],
                    source: AssertionSource::Authored(dimension_ref.clone()),
                    rationale: "Authored driving dimensions constrain current wall geometry."
                        .to_owned(),
                },
                mode: BooleanIntentMode::Requirement,
                expression,
                outcome,
                predicate_observation: None,
                evidence,
            }));
        }
    }
}

fn compile_construction_selections(
    model: &BuildingModel,
    revision: GraphRevision,
    records: &mut Vec<IntentRecord>,
) {
    for wall in &model.walls {
        compile_construction_selection(
            model,
            revision,
            AuthoredEntityRef::Wall(wall.id.clone()),
            &wall.system,
            records,
        );
    }
    for roof in &model.roof_planes {
        compile_construction_selection(
            model,
            revision,
            AuthoredEntityRef::RoofPlane(roof.id.clone()),
            &roof.system,
            records,
        );
    }
    for ceiling in &model.ceilings {
        compile_construction_selection(
            model,
            revision,
            AuthoredEntityRef::Ceiling(ceiling.id.clone()),
            &ceiling.system,
            records,
        );
    }
    for floor in &model.floor_decks {
        compile_construction_selection(
            model,
            revision,
            AuthoredEntityRef::FloorDeck(floor.id.clone()),
            &floor.system,
            records,
        );
    }
}

fn compile_construction_selection(
    model: &BuildingModel,
    revision: GraphRevision,
    host: AuthoredEntityRef,
    selected: &ElementId,
    records: &mut Vec<IntentRecord>,
) {
    let system = AuthoredEntityRef::ConstructionSystem(selected.clone());
    let outcome = if model
        .systems
        .iter()
        .any(|candidate| candidate.id == *selected)
    {
        IntentOutcome::Satisfied
    } else {
        IntentOutcome::Unknown(IntentUnknown {
            kind: IntentUnknownKind::UnresolvedReference,
            detail: format!("Construction system '{}' is not resolved.", selected.0),
        })
    };
    records.push(IntentRecord::Boolean(BooleanIntentRecord {
        assertion: CompiledAssertion {
            reference: AssertionRef::Derived(DerivedAssertionId::new(
                revision,
                DerivedAssertionProvider::Core,
                DerivedAssertionSource::Authored(host.clone()),
                DerivedAssertionRole::ConstructionSystemSelection {
                    selected: selected.clone(),
                },
            )),
            domain: IntentDomain::Construction,
            scope: AssertionScope::Exact(vec![host.clone(), system.clone()]),
            participants: vec![
                AssertionParticipant::new(host.clone(), AssertionParticipantRole::Host, 0),
                AssertionParticipant::new(
                    system.clone(),
                    AssertionParticipantRole::SelectedSystem,
                    1,
                ),
            ],
            source: AssertionSource::Authored(host.clone()),
            rationale: "The authored host explicitly selects its construction system.".to_owned(),
        },
        mode: BooleanIntentMode::Requirement,
        expression: BooleanExpression::SelectedEntity {
            attribute: SelectionAttribute::ConstructionSystem,
            selected: selected.clone(),
        },
        outcome,
        predicate_observation: None,
        evidence: vec![
            IntentEvidenceRef::Authored(host),
            IntentEvidenceRef::Authored(system),
        ],
    }));
}

fn compile_site_premises(
    model: &BuildingModel,
    revision: GraphRevision,
    referenced_properties: &BTreeSet<String>,
    records: &mut Vec<IntentRecord>,
) {
    let site = &model.site;
    site_assumption(
        revision,
        SiteAssumptionKey::Jurisdiction,
        "Jurisdiction",
        (!site.jurisdiction.trim().is_empty())
            .then(|| IntentValue::Text(site.jurisdiction.clone())),
        records,
    );
    site_assumption(
        revision,
        SiteAssumptionKey::SeismicDesignCategory,
        "Seismic design category",
        site.seismic
            .map(|value| IntentValue::Text(format!("{value:?}"))),
        records,
    );
    site_assumption(
        revision,
        SiteAssumptionKey::WindSpeed,
        "Wind speed (mph)",
        site.wind_speed_mph
            .map(|value| IntentValue::Int(i64::from(value))),
        records,
    );
    site_assumption(
        revision,
        SiteAssumptionKey::GroundSnowLoad,
        "Ground snow load (psf)",
        site.ground_snow_load_psf
            .map(|value| IntentValue::Int(i64::from(value))),
        records,
    );
    site_assumption(
        revision,
        SiteAssumptionKey::FrostDepth,
        "Frost depth",
        site.frost_depth.map(IntentValue::Length),
        records,
    );
    let property_keys = site
        .properties
        .keys()
        .chain(referenced_properties)
        .cloned()
        .collect::<BTreeSet<_>>();
    for key in property_keys {
        site_assumption(
            revision,
            SiteAssumptionKey::Property(key.clone()),
            &key,
            site.properties.get(&key).map(property_value),
            records,
        );
    }
}

fn site_assumption(
    revision: GraphRevision,
    key: SiteAssumptionKey,
    label: &str,
    value: Option<IntentValue>,
    records: &mut Vec<IntentRecord>,
) {
    let site = AuthoredEntityRef::Site;
    records.push(IntentRecord::Assumption(AssumptionIntentRecord {
        assertion: CompiledAssertion {
            reference: site_assumption_assertion_ref(revision, key),
            domain: IntentDomain::Compliance,
            scope: AssertionScope::Exact(vec![site.clone()]),
            participants: vec![AssertionParticipant::new(
                site.clone(),
                AssertionParticipantRole::SitePremise,
                0,
            )],
            source: AssertionSource::Authored(site.clone()),
            rationale: "Current site inputs are premises for standards and engineering analysis."
                .to_owned(),
        },
        premise: AssumptionPremise {
            label: label.to_owned(),
        },
        evidence: match value {
            Some(value) => AssumptionEvidence::Known(value),
            None => AssumptionEvidence::Unavailable(IntentUnknown {
                kind: IntentUnknownKind::MissingInput,
                detail: format!("{label} is not provided."),
            }),
        },
        provenance: vec![IntentEvidenceRef::Authored(site)],
    }));
}

fn site_assumption_assertion_ref(revision: GraphRevision, key: SiteAssumptionKey) -> AssertionRef {
    AssertionRef::Derived(DerivedAssertionId::new(
        revision,
        DerivedAssertionProvider::Core,
        DerivedAssertionSource::Authored(AuthoredEntityRef::Site),
        DerivedAssertionRole::SiteAssumption(key),
    ))
}

fn property_value(value: &PropertyValue) -> IntentValue {
    match value {
        PropertyValue::Int(value) => IntentValue::Int(*value),
        PropertyValue::Length(value) => IntentValue::Length(*value),
        PropertyValue::Text(value) => IntentValue::Text(value.clone()),
        PropertyValue::Flag(value) => IntentValue::Flag(*value),
    }
}

pub(crate) fn compile_project_intent(
    model: &BuildingModel,
    plan: &ProjectFramePlan,
    geometry_audit: &GeometryAudit,
    standards: &StandardsEvaluation,
    authored_intent: &[AuthoredIntentEvaluation],
    revision: GraphRevision,
) -> IntentReport {
    let diagnostics = canonical_plan_diagnostic_records(model, plan, revision);
    let mut diagnostic_evidence = DiagnosticEvidenceIndex::new();
    for (_, reference) in &diagnostics {
        diagnostic_evidence
            .entry((
                reference.provider,
                reference.code.clone(),
                reference.source.clone(),
            ))
            .or_default()
            .push(reference.clone());
    }
    let referenced_site_properties = referenced_site_property_keys(standards);
    let mut records = current_intent_records(
        model,
        revision,
        &diagnostic_evidence,
        &referenced_site_properties,
    );
    let mut waivers = Vec::new();
    let diagnostic_payloads = diagnostics
        .iter()
        .map(|(diagnostic, reference)| (reference.clone(), diagnostic.clone()))
        .collect();
    let mut diagnostics_by_key =
        BTreeMap::<(String, Option<ElementId>, String), Vec<DiagnosticRef>>::new();
    for (diagnostic, reference) in &diagnostics {
        diagnostics_by_key
            .entry((
                diagnostic.code.clone(),
                diagnostic.source.clone(),
                diagnostic.message.clone(),
            ))
            .or_default()
            .push(reference.clone());
    }

    lower_authored_intent_records(
        authored_intent,
        &diagnostics_by_key,
        &mut records,
        &mut waivers,
    );

    lower_standards_records(
        model,
        standards,
        revision,
        &diagnostics_by_key,
        &mut records,
        &mut waivers,
    );
    lower_nonstandards_diagnostics(diagnostics, revision, &mut records);
    lower_geometry_records(model, geometry_audit, revision, &mut records);

    IntentReport::from_parts_with_diagnostics(revision, records, waivers, diagnostic_payloads)
}

fn lower_authored_intent_records(
    evaluations: &[AuthoredIntentEvaluation],
    diagnostics_by_key: &BTreeMap<(String, Option<ElementId>, String), Vec<DiagnosticRef>>,
    records: &mut Vec<IntentRecord>,
    waivers: &mut Vec<WaiverRecord>,
) {
    for evaluation in evaluations {
        let assertion = &evaluation.assertion;
        let participants = evaluation.subject.authored_participants();
        let mut qualified = Vec::with_capacity(participants.len());
        for (semantic_order, participant) in participants.iter().cloned().enumerate() {
            qualified.push(AssertionParticipant::new(
                participant,
                if semantic_order == 0 {
                    AssertionParticipantRole::Subject
                } else {
                    AssertionParticipantRole::Host
                },
                semantic_order as u32,
            ));
        }
        let diagnostic = authored_intent_plan_diagnostic(evaluation).and_then(|diagnostic| {
            diagnostics_by_key
                .get(&(diagnostic.code, diagnostic.source, diagnostic.message))
                .and_then(|matches| matches.first())
                .cloned()
        });
        let mut evidence = participants
            .iter()
            .cloned()
            .map(IntentEvidenceRef::Authored)
            .collect::<Vec<_>>();
        if let Some(diagnostic) = diagnostic {
            evidence.push(IntentEvidenceRef::Diagnostic(diagnostic));
        }
        let reference = AssertionRef::Authored(assertion.id.clone());
        records.push(IntentRecord::Boolean(BooleanIntentRecord {
            assertion: CompiledAssertion {
                reference: reference.clone(),
                domain: assertion.domain,
                scope: AssertionScope::Exact(participants.clone()),
                participants: qualified,
                source: match assertion.source {
                    IntentSource::User => AssertionSource::User,
                },
                rationale: assertion.rationale.clone().unwrap_or_else(|| {
                    "Project-authored placed-object containment or clearance intent.".to_owned()
                }),
            },
            mode: assertion.mode,
            expression: match &assertion.expression {
                IntentExpression::FactPredicate(predicate) => {
                    BooleanExpression::Predicate(predicate.clone())
                }
            },
            outcome: evaluation.outcome.clone(),
            predicate_observation: Some(evaluation.predicate.clone()),
            evidence: evidence.clone(),
        }));

        if let Some((override_id, source, reason)) = &evaluation.project_waiver {
            let override_entity = AuthoredEntityRef::IntentOverride(override_id.clone());
            let mut provenance = evidence;
            provenance.push(IntentEvidenceRef::Authored(override_entity.clone()));
            waivers.push(WaiverRecord {
                reference: WaiverRef::Project {
                    override_id: override_id.clone(),
                },
                targets: vec![reference],
                authority: match source {
                    IntentSource::User => AssertionSource::User,
                },
                source: AssertionSource::Authored(override_entity),
                rationale: reason.clone(),
                provenance,
            });
        }
    }
}

/// Lower common current outcomes that do not already own a `PlanDiagnostic` payload.
/// Standards and library diagnostics enter the plan through their native evaluators; solver
/// diagnostics are adapted from their existing payloads. Driving dimensions and geometry need
/// this explicit lowering so every actionable result reaches the one application diagnostics
/// channel.
pub(crate) fn current_intent_plan_diagnostics(
    model: &BuildingModel,
    geometry_audit: &GeometryAudit,
) -> Vec<PlanDiagnostic> {
    let mut diagnostics = Vec::new();
    for wall in &model.walls {
        for dimension in &wall.dimensions {
            if dimension.kind != DimensionKind::Driving {
                continue;
            }
            match dimension.value {
                Some(expected) if !wall.is_driving_dimension_satisfied(dimension) => {
                    let observed = wall.dimension_measurement(dimension);
                    diagnostics.push(PlanDiagnostic {
                        severity: DiagnosticSeverity::Violation,
                        code: "intent.dimension.unsatisfied".to_owned(),
                        source: Some(dimension.id.clone()),
                        message: match observed {
                            Some(observed) => format!(
                                "Driving dimension '{}' requires {expected}, but the current wall measures {observed}.",
                                dimension.name
                            ),
                            None => format!(
                                "Driving dimension '{}' could not be measured against its target {expected}.",
                                dimension.name
                            ),
                        },
                        rule: None,
                    });
                }
                None => diagnostics.push(PlanDiagnostic {
                    severity: DiagnosticSeverity::NeedsReview,
                    code: "intent.dimension.missing-value".to_owned(),
                    source: Some(dimension.id.clone()),
                    message: format!(
                        "Driving dimension '{}' has no target value.",
                        dimension.name
                    ),
                    rule: None,
                }),
                Some(_) => {}
            }
        }
    }

    diagnostics.extend(
        geometry_audit
            .violations
            .iter()
            .map(geometry_plan_diagnostic),
    );
    diagnostics
}

pub(crate) fn authored_intent_plan_diagnostics(
    evaluations: &[AuthoredIntentEvaluation],
) -> Vec<PlanDiagnostic> {
    evaluations
        .iter()
        .filter_map(authored_intent_plan_diagnostic)
        .collect()
}

fn authored_intent_plan_diagnostic(
    evaluation: &AuthoredIntentEvaluation,
) -> Option<PlanDiagnostic> {
    let assertion = &evaluation.assertion;
    let rationale = assertion
        .rationale
        .as_deref()
        .unwrap_or("Project-authored placed-object containment or clearance intent");
    let (severity, code, message) = match &evaluation.outcome {
        IntentOutcome::Violated => match assertion.mode {
            AuthoredIntentMode::Requirement => (
                DiagnosticSeverity::Violation,
                "intent.assertion.violated",
                format!(
                    "Required intent '{}' is violated: {rationale}.",
                    assertion.id.0.0
                ),
            ),
            AuthoredIntentMode::Preference { .. } => (
                DiagnosticSeverity::Warning,
                "intent.assertion.preference-unmet",
                format!(
                    "Preferred intent '{}' is not met: {rationale}.",
                    assertion.id.0.0
                ),
            ),
        },
        IntentOutcome::Unknown(unknown) => (
            if matches!(
                unknown.kind,
                IntentUnknownKind::UnsupportedCondition
                    | IntentUnknownKind::WrongSubjectKind
                    | IntentUnknownKind::EvaluationUnavailable
            ) {
                DiagnosticSeverity::Unsupported
            } else {
                DiagnosticSeverity::NeedsReview
            },
            if matches!(
                unknown.kind,
                IntentUnknownKind::UnsupportedCondition
                    | IntentUnknownKind::WrongSubjectKind
                    | IntentUnknownKind::EvaluationUnavailable
            ) {
                "intent.assertion.unsupported"
            } else {
                "intent.assertion.unknown"
            },
            format!(
                "Intent '{}' needs review: {}",
                assertion.id.0.0, unknown.detail
            ),
        ),
        IntentOutcome::Satisfied | IntentOutcome::Waived { .. } | IntentOutcome::NotApplicable => {
            return None;
        }
    };
    Some(PlanDiagnostic {
        severity,
        code: code.to_owned(),
        source: Some(evaluation.subject.element().clone()),
        message,
        rule: None,
    })
}

fn geometry_plan_diagnostic(violation: &GeometryViolation) -> PlanDiagnostic {
    let severity = match violation {
        GeometryViolation::BodyUnbuildable(_) | GeometryViolation::Overlap(_) => {
            DiagnosticSeverity::Violation
        }
        GeometryViolation::QueryUnsupported(_) => DiagnosticSeverity::Unsupported,
    };
    PlanDiagnostic {
        severity,
        code: violation.code().to_owned(),
        source: Some(violation.body_a().owner().clone()),
        message: violation.to_string(),
        rule: None,
    }
}

fn lower_standards_records(
    model: &BuildingModel,
    standards: &StandardsEvaluation,
    revision: GraphRevision,
    diagnostics_by_key: &BTreeMap<(String, Option<ElementId>, String), Vec<DiagnosticRef>>,
    records: &mut Vec<IntentRecord>,
    waivers: &mut Vec<WaiverRecord>,
) {
    let compliance_refs = compliance_entry_references(model, &standards.report, revision);
    for detail in &standards.details {
        let Some(entry) = standards.report.entries.get(detail.report_entry_index) else {
            continue;
        };
        let Some(entry_ref) = compliance_refs
            .get(detail.report_entry_index)
            .and_then(Clone::clone)
        else {
            continue;
        };
        let mut subject_participants = detail
            .subject
            .as_ref()
            .map(FactSubject::authored_participants)
            .unwrap_or_default();
        if subject_participants.is_empty()
            && let Some(subject) = entry
                .element
                .as_ref()
                .and_then(|id| authored_entity_for_element(model, id))
        {
            subject_participants.push(subject);
        }
        let subject = subject_participants.first().cloned().or_else(|| {
            entry
                .element
                .as_ref()
                .and_then(|id| authored_entity_for_element(model, id))
        });
        let rule_ref = StandardsRuleRef::resolved(entry.pack.clone(), entry.rule.clone());
        let assertion_ref = AssertionRef::Derived(DerivedAssertionId::new(
            revision,
            DerivedAssertionProvider::Standards,
            DerivedAssertionSource::StandardsRule(rule_ref.clone()),
            DerivedAssertionRole::StandardsCheck {
                subject: subject.clone(),
                ordinal: entry_ref.ordinal,
            },
        ));
        let (mode, outcome, waiver) = standards_outcome(entry, detail);
        let participants = subject_participants
            .iter()
            .cloned()
            .enumerate()
            .map(|(semantic_order, subject)| {
                AssertionParticipant::new(
                    subject,
                    AssertionParticipantRole::EvaluatedEntity,
                    semantic_order as u32,
                )
            })
            .collect::<Vec<_>>();
        let scope = subject_participants.clone();
        let mut evidence = vec![
            IntentEvidenceRef::StandardsRule(rule_ref.clone()),
            IntentEvidenceRef::ComplianceEntry(entry_ref),
            IntentEvidenceRef::Authored(AuthoredEntityRef::Site),
        ];
        evidence.extend(site_assumption_evidence(detail, revision));
        evidence.extend(
            subject_participants
                .iter()
                .cloned()
                .map(IntentEvidenceRef::Authored),
        );
        if let Some(diagnostic) = diagnostics_by_key
            .get(&(
                entry.rule.clone(),
                entry.element.clone(),
                entry.message.clone(),
            ))
            .and_then(|matches| matches.first())
        {
            evidence.push(IntentEvidenceRef::Diagnostic(diagnostic.clone()));
        }
        let rationale = detail
            .check_definition
            .as_ref()
            .map(|check| format!("{} ({})", check.title, check.citation))
            .unwrap_or_else(|| entry.message.clone());
        records.push(IntentRecord::Boolean(BooleanIntentRecord {
            assertion: CompiledAssertion {
                reference: assertion_ref.clone(),
                domain: IntentDomain::Compliance,
                scope: if scope.is_empty() {
                    AssertionScope::Project
                } else {
                    AssertionScope::Exact(scope)
                },
                participants,
                source: AssertionSource::StandardsRule(rule_ref.clone()),
                rationale,
            },
            mode,
            expression: detail
                .check_definition
                .as_ref()
                .map(|check| BooleanExpression::Predicate(check.requirement.clone()))
                .unwrap_or_else(|| BooleanExpression::Finding {
                    code: entry.rule.clone(),
                }),
            outcome,
            predicate_observation: detail.predicate.clone(),
            evidence: evidence.clone(),
        }));

        if let Some((reference, reason, overlay_pack)) = waiver {
            let mut provenance = evidence;
            provenance.push(IntentEvidenceRef::Authored(
                AuthoredEntityRef::StandardsPack(overlay_pack.clone()),
            ));
            waivers.push(WaiverRecord {
                reference,
                targets: vec![assertion_ref],
                authority: AssertionSource::Authored(AuthoredEntityRef::StandardsPack(
                    overlay_pack.clone(),
                )),
                source: AssertionSource::Authored(AuthoredEntityRef::StandardsPack(overlay_pack)),
                rationale: reason,
                provenance,
            });
        }
    }
}

fn site_assumption_evidence(
    detail: &StandardsEvaluationDetail,
    revision: GraphRevision,
) -> Vec<IntentEvidenceRef> {
    let mut keys = BTreeSet::new();
    if let Some(check) = &detail.check_definition {
        collect_applicability_assumptions(&check.applies, &mut keys);
    }
    if let Some(predicate) = &detail.predicate {
        for observed in &predicate.observed_facts {
            match observed.fact {
                Fact::OpeningHeaderMaxSpan => {
                    keys.insert(SiteAssumptionKey::GroundSnowLoad);
                }
                Fact::BracedLineRequiredLength => {
                    keys.insert(SiteAssumptionKey::SeismicDesignCategory);
                    keys.insert(SiteAssumptionKey::WindSpeed);
                }
                Fact::WallLength
                | Fact::WallHeight
                | Fact::WallIsExterior
                | Fact::WallStudSpacing
                | Fact::WallSystemRValueMilli
                | Fact::WallStudMaxHeight
                | Fact::OpeningRoughWidth
                | Fact::OpeningRoughHeight
                | Fact::OpeningHeaderDepth
                | Fact::OpeningJackStuds
                | Fact::RoomAreaSquareInches
                | Fact::RoomCeilingHeight
                | Fact::BracedLineLength
                | Fact::BracedLineProvidedLength
                | Fact::PlacedObjectContainedInRoom
                | Fact::PlacedObjectClearance { .. } => {}
            }
        }
    }
    keys.into_iter()
        .map(|key| IntentEvidenceRef::Assertion(site_assumption_assertion_ref(revision, key)))
        .collect()
}

fn referenced_site_property_keys(standards: &StandardsEvaluation) -> BTreeSet<String> {
    let mut assumptions = BTreeSet::new();
    for detail in &standards.details {
        if let Some(check) = &detail.check_definition {
            collect_applicability_assumptions(&check.applies, &mut assumptions);
        }
    }
    assumptions
        .into_iter()
        .filter_map(|key| match key {
            SiteAssumptionKey::Property(key) => Some(key),
            SiteAssumptionKey::Jurisdiction
            | SiteAssumptionKey::SeismicDesignCategory
            | SiteAssumptionKey::WindSpeed
            | SiteAssumptionKey::GroundSnowLoad
            | SiteAssumptionKey::FrostDepth => None,
        })
        .collect()
}

fn collect_applicability_assumptions(
    applicability: &Applicability,
    keys: &mut BTreeSet<SiteAssumptionKey>,
) {
    match applicability {
        Applicability::Always => {}
        Applicability::All(children) | Applicability::Any(children) => {
            for child in children {
                collect_applicability_assumptions(child, keys);
            }
        }
        Applicability::Not(child) => collect_applicability_assumptions(child, keys),
        Applicability::SeismicAtLeast(_) | Applicability::SeismicAtMost(_) => {
            keys.insert(SiteAssumptionKey::SeismicDesignCategory);
        }
        Applicability::WindSpeedAtLeast(_) => {
            keys.insert(SiteAssumptionKey::WindSpeed);
        }
        Applicability::SnowLoadAtLeast(_) => {
            keys.insert(SiteAssumptionKey::GroundSnowLoad);
        }
        Applicability::SiteFlag { key } => {
            keys.insert(SiteAssumptionKey::Property(key.clone()));
        }
    }
}

fn standards_outcome(
    entry: &ComplianceEntry,
    detail: &StandardsEvaluationDetail,
) -> (
    BooleanIntentMode,
    IntentOutcome,
    Option<(WaiverRef, String, ElementId)>,
) {
    const ADVISORY_PRIORITY: PreferencePriority = PreferencePriority(100);
    let mode = match (
        entry.outcome.clone(),
        detail.severity,
        detail.synthetic_kind,
    ) {
        (Outcome::Advisory, _, _) | (_, Some(CheckSeverity::Advisory), _) => {
            BooleanIntentMode::Preference {
                priority: ADVISORY_PRIORITY,
            }
        }
        (_, _, Some(SyntheticEntryKind::UnassociatedBracingPanel)) => {
            BooleanIntentMode::Preference {
                priority: ADVISORY_PRIORITY,
            }
        }
        _ => BooleanIntentMode::Requirement,
    };
    let mut waiver = None;
    let outcome = match &entry.outcome {
        Outcome::Pass => IntentOutcome::Satisfied,
        Outcome::Violation | Outcome::Advisory => IntentOutcome::Violated,
        Outcome::NeedsReview => IntentOutcome::Unknown(unknown_from_standards(detail)),
        Outcome::NotApplicable => IntentOutcome::NotApplicable,
        Outcome::Waived { reason } => match &detail.effective_waiver {
            Some(effective) => {
                let reference = WaiverRef::Standards {
                    overlay_pack: effective.overlay_pack.clone(),
                    rule: entry.rule.clone(),
                };
                waiver = Some((
                    reference.clone(),
                    reason.clone(),
                    effective.overlay_pack.clone(),
                ));
                IntentOutcome::Waived {
                    waiver: reference,
                    reason: reason.clone(),
                }
            }
            None => IntentOutcome::Unknown(IntentUnknown {
                kind: IntentUnknownKind::EvaluationUnavailable,
                detail: format!("Waiver provenance for '{}' is unavailable.", entry.rule),
            }),
        },
    };
    (mode, outcome, waiver)
}

fn unknown_from_standards(detail: &StandardsEvaluationDetail) -> IntentUnknown {
    let kind = if detail.is_unsupported() {
        IntentUnknownKind::UnsupportedCondition
    } else if detail.predicate.as_ref().is_some_and(|predicate| {
        predicate.observed_facts.iter().any(|observed| {
            matches!(
                &observed.observation,
                framer_standards::FactObservation::Unknown(unknown)
                    if unknown.kind == FactUnknownKind::WrongSubjectKind
            )
        })
    }) {
        IntentUnknownKind::WrongSubjectKind
    } else if detail.predicate.as_ref().is_some_and(|predicate| {
        predicate.observed_facts.iter().any(|observed| {
            matches!(
                &observed.observation,
                framer_standards::FactObservation::Unknown(unknown)
                    if unknown.kind == FactUnknownKind::UnresolvedSubject
            )
        })
    }) {
        IntentUnknownKind::UnresolvedSubject
    } else if detail.applicability == Some(Tri::Unknown)
        || detail.predicate.as_ref().is_some_and(|predicate| {
            predicate.observed_facts.iter().any(|observed| {
                matches!(
                    &observed.observation,
                    framer_standards::FactObservation::Unknown(unknown)
                        if unknown.kind == FactUnknownKind::MissingInput
                )
            })
        })
    {
        IntentUnknownKind::MissingInput
    } else {
        IntentUnknownKind::EvaluationUnavailable
    };
    IntentUnknown {
        kind,
        detail: match kind {
            IntentUnknownKind::MissingInput => {
                "One or more required standards facts are missing.".to_owned()
            }
            IntentUnknownKind::UnresolvedSubject => {
                "The standards-check subject could not be resolved.".to_owned()
            }
            IntentUnknownKind::WrongSubjectKind => {
                "A standards fact was requested for the wrong subject family.".to_owned()
            }
            IntentUnknownKind::UnsupportedCondition => {
                "The current standards evaluator does not support this condition.".to_owned()
            }
            IntentUnknownKind::UnresolvedReference | IntentUnknownKind::EvaluationUnavailable => {
                "Standards evaluation evidence is unavailable.".to_owned()
            }
        },
    }
}

fn lower_nonstandards_diagnostics(
    diagnostics: Vec<(PlanDiagnostic, DiagnosticRef)>,
    revision: GraphRevision,
    records: &mut Vec<IntentRecord>,
) {
    for (diagnostic, reference) in diagnostics {
        if matches!(
            reference.provider,
            DiagnosticProvider::Standards
                | DiagnosticProvider::Geometry
                | DiagnosticProvider::Analysis
        ) {
            continue;
        }
        let participant = reference.source.clone();
        let scope = participant
            .iter()
            .cloned()
            .collect::<Vec<AuthoredEntityRef>>();
        let participants = participant
            .iter()
            .cloned()
            .map(|subject| {
                AssertionParticipant::new(subject, AssertionParticipantRole::EvaluatedEntity, 0)
            })
            .collect::<Vec<_>>();
        let assertion = CompiledAssertion {
            reference: AssertionRef::Derived(DerivedAssertionId::new(
                revision,
                diagnostic_assertion_provider(reference.provider),
                participant
                    .clone()
                    .map(DerivedAssertionSource::Authored)
                    .unwrap_or(DerivedAssertionSource::Project),
                DerivedAssertionRole::Diagnostic {
                    provider: reference.provider,
                    code: reference.code.clone(),
                    ordinal: reference.ordinal,
                },
            )),
            domain: diagnostic_domain(reference.provider),
            scope: if scope.is_empty() {
                AssertionScope::Project
            } else {
                AssertionScope::Exact(scope)
            },
            participants,
            source: AssertionSource::Diagnostic(reference.clone()),
            rationale: diagnostic.message.clone(),
        };
        let provider = reference.provider;
        let mut evidence = vec![IntentEvidenceRef::Diagnostic(reference)];
        if let Some(participant) = participant {
            evidence.push(IntentEvidenceRef::Authored(participant));
        }
        if diagnostic.severity == DiagnosticSeverity::Info {
            records.push(IntentRecord::Assumption(AssumptionIntentRecord {
                assertion,
                premise: AssumptionPremise {
                    label: diagnostic.code,
                },
                evidence: AssumptionEvidence::Known(IntentValue::Text(diagnostic.message)),
                provenance: evidence,
            }));
            continue;
        }
        let (mode, outcome) = diagnostic_outcome(
            provider,
            &diagnostic.code,
            diagnostic.severity,
            &diagnostic.message,
        );
        records.push(IntentRecord::Boolean(BooleanIntentRecord {
            assertion,
            mode,
            expression: BooleanExpression::Finding {
                code: diagnostic.code,
            },
            outcome,
            predicate_observation: None,
            evidence,
        }));
    }
}

fn diagnostic_outcome(
    provider: DiagnosticProvider,
    code: &str,
    severity: DiagnosticSeverity,
    message: &str,
) -> (BooleanIntentMode, IntentOutcome) {
    match severity {
        DiagnosticSeverity::Violation => (BooleanIntentMode::Requirement, IntentOutcome::Violated),
        DiagnosticSeverity::Warning => warning_diagnostic_outcome(provider, code, message),
        DiagnosticSeverity::NeedsReview => (
            BooleanIntentMode::Requirement,
            IntentOutcome::Unknown(IntentUnknown {
                kind: IntentUnknownKind::MissingInput,
                detail: message.to_owned(),
            }),
        ),
        DiagnosticSeverity::Unsupported => (
            BooleanIntentMode::Requirement,
            IntentOutcome::Unknown(IntentUnknown {
                kind: IntentUnknownKind::UnsupportedCondition,
                detail: message.to_owned(),
            }),
        ),
        DiagnosticSeverity::Info => unreachable!("info diagnostics use assumption evidence"),
    }
}

fn warning_diagnostic_outcome(
    provider: DiagnosticProvider,
    code: &str,
    message: &str,
) -> (BooleanIntentMode, IntentOutcome) {
    let unknown = |kind| {
        (
            BooleanIntentMode::Requirement,
            IntentOutcome::Unknown(IntentUnknown {
                kind,
                detail: message.to_owned(),
            }),
        )
    };
    match (provider, code) {
        (DiagnosticProvider::Library, "library.lifecycle.check-failed") => {
            unknown(IntentUnknownKind::EvaluationUnavailable)
        }
        (DiagnosticProvider::Library, "library.item.source-missing") => {
            unknown(IntentUnknownKind::UnresolvedReference)
        }
        (DiagnosticProvider::Library, "library.item.diverged" | "library.item.out-of-date") => (
            BooleanIntentMode::Preference {
                priority: PreferencePriority(100),
            },
            IntentOutcome::Violated,
        ),
        (DiagnosticProvider::Library, _) => unknown(IntentUnknownKind::EvaluationUnavailable),
        (DiagnosticProvider::Solver, "room.boundary.open")
        | (DiagnosticProvider::Solver, "floor.boundary.open")
        | (DiagnosticProvider::Solver, "ceiling.boundary.open") => {
            unknown(IntentUnknownKind::MissingInput)
        }
        (DiagnosticProvider::Solver, _) => {
            (BooleanIntentMode::Requirement, IntentOutcome::Violated)
        }
        (
            DiagnosticProvider::Standards
            | DiagnosticProvider::Geometry
            | DiagnosticProvider::Analysis,
            _,
        ) => unknown(IntentUnknownKind::EvaluationUnavailable),
    }
}

fn lower_geometry_records(
    model: &BuildingModel,
    audit: &GeometryAudit,
    revision: GraphRevision,
    records: &mut Vec<IntentRecord>,
) {
    for (violation, reference) in canonical_geometry_records(model, audit, revision) {
        let mut participants = Vec::new();
        let mut scope = Vec::new();
        let mut evidence = vec![IntentEvidenceRef::Diagnostic(reference.clone())];
        for (order, body) in [Some(violation.body_a()), violation.body_b()]
            .into_iter()
            .flatten()
            .enumerate()
        {
            let body_ref = PhysicalBodyRef::new(revision, body.clone());
            evidence.push(IntentEvidenceRef::PhysicalBody(body_ref));
            if let Some(owner) = authored_entity_for_element(model, body.owner()) {
                scope.push(owner.clone());
                participants.push(AssertionParticipant::new(
                    owner.clone(),
                    AssertionParticipantRole::EvaluatedEntity,
                    u32::try_from(order).unwrap_or(u32::MAX),
                ));
                evidence.push(IntentEvidenceRef::Authored(owner));
            }
        }
        let outcome = match violation {
            GeometryViolation::QueryUnsupported(_) => IntentOutcome::Unknown(IntentUnknown {
                kind: IntentUnknownKind::UnsupportedCondition,
                detail: violation.to_string(),
            }),
            GeometryViolation::BodyUnbuildable(_) | GeometryViolation::Overlap(_) => {
                IntentOutcome::Violated
            }
        };
        records.push(IntentRecord::Boolean(BooleanIntentRecord {
            assertion: CompiledAssertion {
                reference: AssertionRef::Derived(DerivedAssertionId::new(
                    revision,
                    DerivedAssertionProvider::Geometry,
                    DerivedAssertionSource::PhysicalBody(PhysicalBodyRef::new(
                        revision,
                        violation.body_a().clone(),
                    )),
                    DerivedAssertionRole::GeometryFinding {
                        code: violation.code().to_owned(),
                        ordinal: reference.ordinal,
                    },
                )),
                domain: IntentDomain::FabricationInstallation,
                scope: if scope.is_empty() {
                    AssertionScope::Project
                } else {
                    AssertionScope::Exact(scope)
                },
                participants,
                source: AssertionSource::PhysicalBody(PhysicalBodyRef::new(
                    revision,
                    violation.body_a().clone(),
                )),
                rationale: violation.to_string(),
            },
            mode: BooleanIntentMode::Requirement,
            expression: BooleanExpression::Finding {
                code: violation.code().to_owned(),
            },
            outcome,
            predicate_observation: None,
            evidence,
        }));
    }
}

fn diagnostic_assertion_provider(provider: DiagnosticProvider) -> DerivedAssertionProvider {
    match provider {
        DiagnosticProvider::Solver => DerivedAssertionProvider::Solver,
        DiagnosticProvider::Standards => DerivedAssertionProvider::Standards,
        DiagnosticProvider::Geometry => DerivedAssertionProvider::Geometry,
        DiagnosticProvider::Library => DerivedAssertionProvider::Library,
        DiagnosticProvider::Analysis => DerivedAssertionProvider::Analysis,
    }
}

fn diagnostic_domain(provider: DiagnosticProvider) -> IntentDomain {
    match provider {
        DiagnosticProvider::Standards => IntentDomain::Compliance,
        DiagnosticProvider::Geometry => IntentDomain::FabricationInstallation,
        DiagnosticProvider::Library => IntentDomain::Resource,
        DiagnosticProvider::Solver | DiagnosticProvider::Analysis => IntentDomain::Construction,
    }
}

pub(crate) fn canonical_plan_diagnostic_records(
    model: &BuildingModel,
    plan: &ProjectFramePlan,
    revision: GraphRevision,
) -> Vec<(PlanDiagnostic, DiagnosticRef)> {
    let mut diagnostics = plan.diagnostics.clone();
    diagnostics.extend(
        plan.wall_plans
            .iter()
            .flat_map(|host| host.diagnostics.iter().cloned()),
    );
    diagnostics.extend(
        plan.floor_plans
            .iter()
            .flat_map(|host| host.diagnostics.iter().cloned()),
    );
    diagnostics.extend(
        plan.ceiling_plans
            .iter()
            .flat_map(|host| host.diagnostics.iter().cloned()),
    );
    diagnostics.extend(
        plan.roof_plans
            .iter()
            .flat_map(|host| host.diagnostics.iter().cloned()),
    );
    diagnostics.sort_by(compare_plan_diagnostic);

    let mut ordinals =
        BTreeMap::<(DiagnosticProvider, String, Option<AuthoredEntityRef>), u32>::new();
    diagnostics
        .into_iter()
        .map(|diagnostic| {
            let provider = diagnostic_provider(&diagnostic);
            let source = diagnostic
                .source
                .as_ref()
                .and_then(|id| authored_entity_for_element(model, id));
            let ordinal = ordinals
                .entry((provider, diagnostic.code.clone(), source.clone()))
                .or_default();
            let reference = DiagnosticRef {
                revision,
                provider,
                code: diagnostic.code.clone(),
                source,
                ordinal: *ordinal,
            };
            *ordinal = ordinal.saturating_add(1);
            (diagnostic, reference)
        })
        .collect()
}

pub(crate) fn canonical_geometry_records(
    model: &BuildingModel,
    audit: &GeometryAudit,
    revision: GraphRevision,
) -> Vec<(GeometryViolation, DiagnosticRef)> {
    let mut violations = audit.violations.clone();
    violations.sort_by(|left, right| {
        compare_plan_diagnostic(
            &geometry_plan_diagnostic(left),
            &geometry_plan_diagnostic(right),
        )
        .then_with(|| compare_geometry_violation(left, right))
    });
    let mut ordinals = BTreeMap::<(String, Option<AuthoredEntityRef>), u32>::new();
    violations
        .into_iter()
        .map(|violation| {
            let source = authored_entity_for_element(model, violation.body_a().owner());
            let key = (violation.code().to_owned(), source.clone());
            let ordinal = ordinals.entry(key).or_default();
            let reference = DiagnosticRef {
                revision,
                provider: DiagnosticProvider::Geometry,
                code: violation.code().to_owned(),
                source,
                ordinal: *ordinal,
            };
            *ordinal = ordinal.saturating_add(1);
            (violation, reference)
        })
        .collect()
}

fn compliance_entry_references(
    model: &BuildingModel,
    report: &ComplianceReport,
    revision: GraphRevision,
) -> Vec<Option<ComplianceEntryRef>> {
    let mut indices = (0..report.entries.len()).collect::<Vec<_>>();
    indices.sort_by(|left, right| {
        compare_compliance_entry(&report.entries[*left], &report.entries[*right])
    });
    let mut ordinals = BTreeMap::<(StandardsRuleRef, Option<AuthoredEntityRef>), u32>::new();
    let mut references = vec![None; report.entries.len()];
    for index in indices {
        let entry = &report.entries[index];
        let subject = entry
            .element
            .as_ref()
            .and_then(|id| authored_entity_for_element(model, id));
        let rule = StandardsRuleRef::resolved(entry.pack.clone(), entry.rule.clone());
        let ordinal = ordinals.entry((rule.clone(), subject.clone())).or_default();
        references[index] = Some(ComplianceEntryRef {
            revision,
            rule,
            subject,
            ordinal: *ordinal,
        });
        *ordinal = ordinal.saturating_add(1);
    }
    references
}

pub(crate) fn authored_entity_for_element(
    model: &BuildingModel,
    id: &ElementId,
) -> Option<AuthoredEntityRef> {
    if model.standards_packs.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::StandardsPack(id.clone()));
    }
    if model.materials.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::Material(id.clone()));
    }
    if model.systems.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::ConstructionSystem(id.clone()));
    }
    if model.furnishings.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::Furnishing(id.clone()));
    }
    if model.mep_objects.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::MepObject(id.clone()));
    }
    if model.levels.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::Level(id.clone()));
    }
    for wall in &model.walls {
        if wall.id == *id {
            return Some(AuthoredEntityRef::Wall(id.clone()));
        }
        if wall.openings.iter().any(|entity| entity.id == *id) {
            return Some(AuthoredEntityRef::Opening(id.clone()));
        }
        if wall.dimensions.iter().any(|entity| entity.id == *id) {
            return Some(AuthoredEntityRef::Dimension(id.clone()));
        }
        if wall.bracing.iter().any(|entity| entity.id == *id) {
            return Some(AuthoredEntityRef::BracedPanel(id.clone()));
        }
    }
    if model.wall_joins.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::WallJoin(id.clone()));
    }
    if model.rooms.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::Room(id.clone()));
    }
    if model
        .furnishing_instances
        .iter()
        .any(|entity| entity.id == *id)
    {
        return Some(AuthoredEntityRef::FurnishingInstance(id.clone()));
    }
    if model.mep_instances.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::MepInstance(id.clone()));
    }
    for roof in &model.roof_planes {
        if roof.id == *id {
            return Some(AuthoredEntityRef::RoofPlane(id.clone()));
        }
        if roof.openings.iter().any(|entity| entity.id == *id) {
            return Some(AuthoredEntityRef::RoofOpening(id.clone()));
        }
    }
    if model.ceilings.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::Ceiling(id.clone()));
    }
    if model.floor_decks.iter().any(|entity| entity.id == *id) {
        return Some(AuthoredEntityRef::FloorDeck(id.clone()));
    }
    if model
        .braced_wall_lines
        .iter()
        .any(|entity| entity.id == *id)
    {
        return Some(AuthoredEntityRef::BracedWallLine(id.clone()));
    }
    if let Some(intent_override) = model
        .intent_overrides
        .iter()
        .find(|intent_override| intent_override.id().0 == *id)
    {
        return Some(AuthoredEntityRef::IntentOverride(
            intent_override.id().clone(),
        ));
    }
    None
}

pub(crate) fn diagnostic_provider(diagnostic: &PlanDiagnostic) -> DiagnosticProvider {
    if diagnostic.rule.is_some() {
        DiagnosticProvider::Standards
    } else if diagnostic.code.starts_with("geometry.") {
        DiagnosticProvider::Geometry
    } else if diagnostic.code.starts_with("intent.") {
        DiagnosticProvider::Analysis
    } else if diagnostic.code.starts_with("library.") {
        DiagnosticProvider::Library
    } else {
        DiagnosticProvider::Solver
    }
}

fn compare_plan_diagnostic(left: &PlanDiagnostic, right: &PlanDiagnostic) -> Ordering {
    diagnostic_provider(left)
        .cmp(&diagnostic_provider(right))
        .then_with(|| left.code.cmp(&right.code))
        .then_with(|| left.source.cmp(&right.source))
        .then_with(|| severity_rank(left.severity).cmp(&severity_rank(right.severity)))
        .then_with(|| left.message.cmp(&right.message))
        .then_with(|| compare_rule_ref(left.rule.as_ref(), right.rule.as_ref()))
}

fn severity_rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Info => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Unsupported => 2,
        DiagnosticSeverity::Violation => 3,
        DiagnosticSeverity::NeedsReview => 4,
    }
}

fn compare_rule_ref(left: Option<&RuleRef>, right: Option<&RuleRef>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left
            .pack
            .cmp(&right.pack)
            .then_with(|| left.rule.cmp(&right.rule))
            .then_with(|| left.citation.cmp(&right.citation)),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

pub(crate) fn compare_geometry_violation(
    left: &GeometryViolation,
    right: &GeometryViolation,
) -> Ordering {
    left.body_a()
        .cmp(right.body_a())
        .then_with(|| left.body_b().cmp(&right.body_b()))
        .then_with(|| left.code().cmp(right.code()))
        .then_with(|| left.to_string().cmp(&right.to_string()))
}

fn outcome_rank(outcome: &Outcome) -> u8 {
    match outcome {
        Outcome::Pass => 0,
        Outcome::Violation => 1,
        Outcome::Advisory => 2,
        Outcome::NeedsReview => 3,
        Outcome::NotApplicable => 4,
        Outcome::Waived { .. } => 5,
    }
}

fn compare_compliance_entry(left: &ComplianceEntry, right: &ComplianceEntry) -> Ordering {
    left.rule
        .cmp(&right.rule)
        .then_with(|| left.element.cmp(&right.element))
        .then_with(|| left.pack.cmp(&right.pack))
        .then_with(|| outcome_rank(&left.outcome).cmp(&outcome_rank(&right.outcome)))
        .then_with(|| left.citation.cmp(&right.citation))
        .then_with(|| left.message.cmp(&right.message))
}

#[cfg(test)]
mod tests {
    use framer_core::{
        Applicability, AuthoredIntentId, AuthoredIntentMode, CheckScope, ClearanceDatum,
        ClearanceDirection, CompareOp, ComplianceCheck, DimensionAnchor, DimensionConstraint,
        DimensionDirection, DimensionKind, ExactIntentScope, Fact, FactOperand, Furnishing,
        FurnishingInstance, IntentAssertion, IntentDomain, IntentExpression, IntentOverride,
        IntentOverrideId, IntentSource, Length, Point2, Predicate, PreferencePriority,
        ProjectIntentScope, ResolutionAction, RuleOverlay,
    };
    use framer_geometry::{AssemblyKind, BodyRef, GeometryBuildDiagnostic, PhysicalScene};
    use framer_solver::{MemberKind, generate_project_plan};
    use framer_standards::EffectiveWaiver;

    use super::*;
    use crate::{ProjectNodeRef, RelationshipKind};

    fn authored_clearance_model(mode: AuthoredIntentMode, threshold: Length) -> BuildingModel {
        let mut model = BuildingModel::demo_two_bedroom();
        model.furnishings.push(Furnishing::new(
            "furnishing-test-fixture",
            "Test fixture",
            Length::from_feet(2.0),
            Length::from_feet(2.0),
            Length::from_feet(3.0),
        ));
        model.furnishing_instances.push(FurnishingInstance::new(
            "fixture-instance",
            "Fixture instance",
            "furnishing-test-fixture",
            "level-1",
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        ));
        model.intents.push(IntentAssertion {
            id: AuthoredIntentId::new("intent-front-clearance"),
            domain: IntentDomain::SpatialProgram,
            mode,
            scope: ProjectIntentScope::Exact(ExactIntentScope {
                subject: AuthoredEntityRef::FurnishingInstance(ElementId::new("fixture-instance")),
                participants: vec![AuthoredEntityRef::Room(ElementId::new("room-bed-1"))],
            }),
            expression: IntentExpression::FactPredicate(Predicate::Compare {
                fact: Fact::PlacedObjectClearance {
                    direction: ClearanceDirection::Front,
                    datum: ClearanceDatum::FootprintFace,
                },
                op: CompareOp::Ge,
                value: FactOperand::LengthLiteral(threshold),
            }),
            source: IntentSource::User,
            rationale: Some("Keep the fixture front approach clear".to_owned()),
        });
        model.sort_deterministically();
        model.validate().unwrap();
        model
    }

    #[test]
    fn authored_clearance_uses_common_outcomes_and_exact_diagnostic_severity() {
        let required =
            authored_clearance_model(AuthoredIntentMode::Requirement, Length::from_feet(5.0));
        let analysis = crate::analyze_project(&required).unwrap();
        let record = analysis
            .intent_report
            .as_ref()
            .unwrap()
            .record(&AssertionRef::Authored(AuthoredIntentId::new(
                "intent-front-clearance",
            )))
            .unwrap();
        let IntentRecord::Boolean(record) = record else {
            panic!("persisted requirement must compile as a boolean record");
        };
        assert_eq!(record.outcome, IntentOutcome::Violated);
        let observation = record
            .predicate_observation
            .as_ref()
            .expect("authored predicate observation must survive report lowering");
        assert_eq!(observation.result, Tri::False);
        assert!(matches!(
            observation.observed_facts.as_slice(),
            [framer_standards::ObservedFact {
                observation: framer_standards::FactObservation::Known(
                    framer_standards::FactValue::Length(_)
                ),
                ..
            }]
        ));
        assert_eq!(
            record.assertion.scope,
            AssertionScope::Exact(vec![
                AuthoredEntityRef::Room(ElementId::new("room-bed-1")),
                AuthoredEntityRef::FurnishingInstance(ElementId::new("fixture-instance")),
            ])
        );
        let diagnostic = analysis
            .plan
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "intent.assertion.violated")
            .unwrap();
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Violation);
        assert_eq!(diagnostic.source, Some(ElementId::new("fixture-instance")));
        let graph_record = analysis
            .graph
            .as_ref()
            .unwrap()
            .node(&ProjectNodeRef::Assertion(
                record.assertion.reference.clone(),
            ))
            .unwrap();
        assert!(
            graph_record.detail.as_deref().is_some_and(|detail| detail
                .contains("predicate observation: PredicateObservation")
                && detail.contains("Known(Length")),
            "graph assertion detail must retain the measured predicate evidence"
        );

        let preferred = authored_clearance_model(
            AuthoredIntentMode::Preference {
                priority: PreferencePriority(250),
            },
            Length::from_feet(5.0),
        );
        let preferred = crate::analyze_project(&preferred).unwrap();
        let diagnostic = preferred
            .plan
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "intent.assertion.preference-unmet")
            .unwrap();
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn authored_clearance_missing_room_and_project_waiver_fail_closed() {
        let mut open =
            authored_clearance_model(AuthoredIntentMode::Requirement, Length::from_feet(1.0));
        open.rooms
            .iter_mut()
            .find(|room| room.id == ElementId::new("room-bed-1"))
            .unwrap()
            .seed = Point2::new(Length::from_feet(-1.0), Length::from_feet(-1.0));
        open.validate().unwrap();
        let analysis = crate::analyze_project(&open).unwrap();
        let record = analysis
            .intent_report
            .as_ref()
            .unwrap()
            .record(&AssertionRef::Authored(AuthoredIntentId::new(
                "intent-front-clearance",
            )))
            .unwrap();
        assert!(matches!(
            record,
            IntentRecord::Boolean(BooleanIntentRecord {
                outcome: IntentOutcome::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::MissingInput,
                    ..
                }),
                ..
            })
        ));
        assert!(analysis.plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "intent.assertion.unknown"
                && diagnostic.severity == DiagnosticSeverity::NeedsReview
        }));

        let mut waived =
            authored_clearance_model(AuthoredIntentMode::Requirement, Length::from_feet(5.0));
        waived.intent_overrides.push(IntentOverride::Waive {
            id: IntentOverrideId::new("intent-waiver-front-clearance"),
            target: AuthoredIntentId::new("intent-front-clearance"),
            reason: "Owner-approved alternate approach".to_owned(),
            source: IntentSource::User,
        });
        waived.validate().unwrap();
        let analysis = crate::analyze_project(&waived).unwrap();
        let report = analysis.intent_report.as_ref().unwrap();
        let record = report
            .record(&AssertionRef::Authored(AuthoredIntentId::new(
                "intent-front-clearance",
            )))
            .unwrap();
        assert!(matches!(
            record,
            IntentRecord::Boolean(BooleanIntentRecord {
                outcome: IntentOutcome::Waived {
                    waiver: WaiverRef::Project { override_id },
                    ..
                },
                ..
            }) if override_id == &IntentOverrideId::new("intent-waiver-front-clearance")
        ));
        assert_eq!(report.waivers().len(), 1);
        assert_eq!(report.waivers()[0].authority, AssertionSource::User);
        assert_eq!(
            report.waivers()[0].source,
            AssertionSource::Authored(AuthoredEntityRef::IntentOverride(IntentOverrideId::new(
                "intent-waiver-front-clearance"
            )))
        );
        assert!(
            report.waivers()[0]
                .provenance
                .contains(&IntentEvidenceRef::Authored(
                    AuthoredEntityRef::IntentOverride(IntentOverrideId::new(
                        "intent-waiver-front-clearance"
                    ))
                ))
        );
        assert!(
            !analysis
                .plan
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code.starts_with("intent.assertion") })
        );

        let graph = analysis.graph.as_ref().unwrap();
        let target = ProjectNodeRef::Assertion(AssertionRef::Authored(AuthoredIntentId::new(
            "intent-front-clearance",
        )));
        let waiver = ProjectNodeRef::Authored(AuthoredEntityRef::IntentOverride(
            IntentOverrideId::new("intent-waiver-front-clearance"),
        ));
        assert!(graph.edges().iter().any(|edge| {
            edge.dependent == target
                && edge.dependency == waiver
                && edge.relationship == RelationshipKind::WaivedBy
        }));
    }

    #[test]
    fn standards_and_authored_clearance_share_the_exact_predicate_observation() {
        let mut model =
            authored_clearance_model(AuthoredIntentMode::Requirement, Length::from_feet(1.0));
        model.furnishings[0].tags.push("shared-fixture".to_owned());
        let predicate = match &model.intents[0].expression {
            IntentExpression::FactPredicate(predicate) => predicate.clone(),
        };
        let pack = model
            .standards_packs
            .iter_mut()
            .find(|pack| pack.id == model.standards[0])
            .unwrap();
        pack.checks.push(ComplianceCheck {
            rule: "zz-test.shared-front-clearance".to_owned(),
            citation: "Intent parity fixture".to_owned(),
            title: "Shared front clearance".to_owned(),
            severity: CheckSeverity::Required,
            applies: Applicability::Always,
            scope: CheckScope::PlacedObjects {
                tags: vec!["shared-fixture".to_owned()],
            },
            requirement: predicate.clone(),
        });
        pack.checks
            .sort_by(|left, right| left.rule.cmp(&right.rule));
        model.validate().unwrap();

        let plan = generate_project_plan(&model).unwrap();
        let resolved = model.resolved_standards();
        let standards = framer_standards::evaluate_detailed(&model, &resolved, &plan);
        let detail = standards
            .details
            .iter()
            .find(|detail| detail.check_id.as_deref() == Some("zz-test.shared-front-clearance"))
            .unwrap();
        let subject = FactSubject::placed_object_exact(
            PlacedObjectRef::FurnishingInstance(ElementId::new("fixture-instance")),
            ElementId::new("room-bed-1"),
        );
        let direct =
            FactSnapshot::new(&model, &resolved, &plan).evaluate_predicate(&predicate, &subject);
        assert_eq!(detail.predicate.as_ref(), Some(&direct));

        let authored = evaluate_authored_intent(&model, &resolved, &plan);
        assert_eq!(authored.len(), 1);
        assert_eq!(authored[0].predicate, direct);
        let standard_outcome = match direct.result {
            Tri::True => IntentOutcome::Satisfied,
            Tri::False => IntentOutcome::Violated,
            Tri::Unknown => IntentOutcome::Unknown(unknown_from_predicate(&direct)),
        };
        assert_eq!(authored[0].outcome, standard_outcome);

        let analysis = crate::analyze_project(&model).unwrap();
        let report_record = analysis
            .intent_report
            .as_ref()
            .unwrap()
            .record(&AssertionRef::Authored(AuthoredIntentId::new(
                "intent-front-clearance",
            )))
            .unwrap();
        let IntentRecord::Boolean(report_record) = report_record else {
            panic!("authored clearance must remain a boolean record");
        };
        assert_eq!(report_record.outcome, IntentOutcome::Satisfied);
        assert_eq!(report_record.predicate_observation.as_ref(), Some(&direct));
        let graph_record = analysis
            .graph
            .as_ref()
            .unwrap()
            .node(&ProjectNodeRef::Assertion(
                report_record.assertion.reference.clone(),
            ))
            .unwrap();
        assert!(
            graph_record
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains(&format!("{direct:?}")))
        );
    }

    fn compliance_entry(rule: &str, outcome: Outcome) -> ComplianceEntry {
        ComplianceEntry {
            rule: rule.to_owned(),
            citation: "Test citation".to_owned(),
            pack: ElementId::new("standards-test"),
            outcome,
            element: Some(ElementId::new("wall")),
            message: format!("{rule} result"),
            chain: vec![(
                ElementId::new("standards-test"),
                ResolutionAction::Introduced,
            )],
        }
    }

    fn standards_detail(severity: CheckSeverity, applicability: Tri) -> StandardsEvaluationDetail {
        StandardsEvaluationDetail {
            report_entry_index: 0,
            check_id: Some("test".to_owned()),
            definition_pack: Some(ElementId::new("standards-test")),
            check_definition: None,
            severity: Some(severity),
            subject: Some(FactSubject::Wall(ElementId::new("wall"))),
            scope_subjects: vec![FactSubject::Wall(ElementId::new("wall"))],
            applicability: Some(applicability),
            predicate: None,
            synthetic_kind: None,
            effective_waiver: None,
        }
    }

    #[test]
    fn standards_outcomes_map_to_exact_common_modes_and_results() {
        let required = standards_detail(CheckSeverity::Required, Tri::True);
        let advisory = standards_detail(CheckSeverity::Advisory, Tri::True);
        let missing = standards_detail(CheckSeverity::Required, Tri::Unknown);
        let mut unsupported = standards_detail(CheckSeverity::Required, Tri::True);
        unsupported.synthetic_kind = Some(SyntheticEntryKind::BracingOutOfDomain);
        let mut synthetic_advisory = standards_detail(CheckSeverity::Required, Tri::True);
        synthetic_advisory.synthetic_kind = Some(SyntheticEntryKind::UnassociatedBracingPanel);
        synthetic_advisory.severity = None;
        let mut waived = standards_detail(CheckSeverity::Advisory, Tri::True);
        waived.effective_waiver = Some(EffectiveWaiver {
            reason: "approved alternate".to_owned(),
            overlay_pack: ElementId::new("standards-overlay"),
            chain: vec![(
                ElementId::new("standards-overlay"),
                ResolutionAction::Waived,
            )],
        });

        let cases = [
            (
                compliance_entry("pass-required", Outcome::Pass),
                required.clone(),
                BooleanIntentMode::Requirement,
                IntentOutcome::Satisfied,
            ),
            (
                compliance_entry("pass-advisory", Outcome::Pass),
                advisory,
                BooleanIntentMode::Preference {
                    priority: PreferencePriority(100),
                },
                IntentOutcome::Satisfied,
            ),
            (
                compliance_entry("violation", Outcome::Violation),
                required.clone(),
                BooleanIntentMode::Requirement,
                IntentOutcome::Violated,
            ),
            (
                compliance_entry("advisory", Outcome::Advisory),
                synthetic_advisory,
                BooleanIntentMode::Preference {
                    priority: PreferencePriority(100),
                },
                IntentOutcome::Violated,
            ),
            (
                compliance_entry("missing", Outcome::NeedsReview),
                missing,
                BooleanIntentMode::Requirement,
                IntentOutcome::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::MissingInput,
                    detail: "One or more required standards facts are missing.".to_owned(),
                }),
            ),
            (
                compliance_entry("unsupported", Outcome::NeedsReview),
                unsupported,
                BooleanIntentMode::Requirement,
                IntentOutcome::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::UnsupportedCondition,
                    detail: "The current standards evaluator does not support this condition."
                        .to_owned(),
                }),
            ),
            (
                compliance_entry("n-a", Outcome::NotApplicable),
                required,
                BooleanIntentMode::Requirement,
                IntentOutcome::NotApplicable,
            ),
        ];
        for (entry, detail, expected_mode, expected_outcome) in cases {
            let (mode, outcome, waiver) = standards_outcome(&entry, &detail);
            assert_eq!(mode, expected_mode, "{}", entry.rule);
            assert_eq!(outcome, expected_outcome, "{}", entry.rule);
            assert!(waiver.is_none(), "{}", entry.rule);
        }

        let waived_entry = compliance_entry(
            "waived",
            Outcome::Waived {
                reason: "approved alternate".to_owned(),
            },
        );
        let (mode, outcome, waiver) = standards_outcome(&waived_entry, &waived);
        assert_eq!(
            mode,
            BooleanIntentMode::Preference {
                priority: PreferencePriority(100)
            }
        );
        assert_eq!(
            outcome,
            IntentOutcome::Waived {
                waiver: WaiverRef::Standards {
                    overlay_pack: ElementId::new("standards-overlay"),
                    rule: "waived".to_owned(),
                },
                reason: "approved alternate".to_owned(),
            }
        );
        assert!(waiver.is_some());
    }

    #[test]
    fn standards_diagnostics_use_one_exact_severity_matrix() {
        let mut entries = vec![
            compliance_entry("violation", Outcome::Violation),
            compliance_entry("advisory", Outcome::Advisory),
            compliance_entry("missing", Outcome::NeedsReview),
            compliance_entry("unsupported", Outcome::NeedsReview),
            compliance_entry("pass", Outcome::Pass),
            compliance_entry("n-a", Outcome::NotApplicable),
            compliance_entry(
                "waived",
                Outcome::Waived {
                    reason: "approved".to_owned(),
                },
            ),
        ];
        for entry in &mut entries {
            entry.element = None;
        }
        let mut details = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let mut detail = standards_detail(CheckSeverity::Required, Tri::True);
            detail.report_entry_index = index;
            detail.subject = None;
            detail.scope_subjects.clear();
            if entry.rule == "missing" {
                detail.applicability = Some(Tri::Unknown);
            }
            if entry.rule == "unsupported" {
                detail.synthetic_kind = Some(SyntheticEntryKind::BracingOutOfDomain);
            }
            details.push(detail);
        }
        let evaluation = StandardsEvaluation {
            report: ComplianceReport { entries },
            details,
        };
        let diagnostics = evaluation.diagnostics();
        assert_eq!(diagnostics.len(), 4);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code.as_str(), diagnostic.severity))
                .collect::<Vec<_>>(),
            vec![
                ("violation", DiagnosticSeverity::Violation),
                ("advisory", DiagnosticSeverity::Warning),
                ("missing", DiagnosticSeverity::NeedsReview),
                ("unsupported", DiagnosticSeverity::Unsupported),
            ]
        );
        for diagnostic in diagnostics {
            let entry = evaluation
                .report
                .entries
                .iter()
                .find(|entry| entry.rule == diagnostic.code)
                .unwrap();
            assert_eq!(diagnostic.source, entry.element);
            assert_eq!(diagnostic.message, entry.message);
            assert_eq!(diagnostic.rule.as_ref().unwrap().citation, entry.citation);
        }
    }

    #[test]
    fn existing_standard_diagnostics_are_installed_once_with_exact_payloads() {
        for model in [
            BuildingModel::demo_wall(),
            BuildingModel::demo_shell(),
            BuildingModel::demo_two_bedroom(),
        ] {
            let plan = generate_project_plan(&model).unwrap();
            let resolved = model.resolved_standards();
            let evaluation = framer_standards::evaluate_detailed(&model, &resolved, &plan);
            let expected = framer_standards::diagnostics(&evaluation.report);
            assert_eq!(evaluation.diagnostics(), expected);

            let analysis = crate::analyze_project(&model).unwrap();
            for diagnostic in expected {
                assert_eq!(
                    analysis
                        .plan
                        .diagnostics
                        .iter()
                        .filter(|candidate| **candidate == diagnostic)
                        .count(),
                    1,
                    "{}",
                    diagnostic.code
                );
            }
        }
    }

    #[test]
    fn explicit_standards_rules_precede_diagnostic_code_prefix_fallbacks() {
        let diagnostic = |code: &str, rule| PlanDiagnostic {
            severity: DiagnosticSeverity::Violation,
            code: code.to_owned(),
            source: None,
            message: "test diagnostic".to_owned(),
            rule,
        };
        let rule = |code: &str| {
            Some(RuleRef {
                pack: ElementId::new("standards-test"),
                rule: code.to_owned(),
                citation: "Test citation".to_owned(),
            })
        };

        for code in ["geometry.clearance", "intent.constraint"] {
            assert_eq!(
                diagnostic_provider(&diagnostic(code, rule(code))),
                DiagnosticProvider::Standards,
                "explicit standards provenance must own {code}",
            );
        }
        assert_eq!(
            diagnostic_provider(&diagnostic("geometry.clearance", None)),
            DiagnosticProvider::Geometry,
        );
        assert_eq!(
            diagnostic_provider(&diagnostic("intent.constraint", None)),
            DiagnosticProvider::Analysis,
        );
    }

    #[test]
    fn nonstandards_diagnostics_use_provider_and_code_semantics() {
        let model = BuildingModel::new();
        let revision = GraphRevision::for_model(&model).unwrap();
        let cases = [
            (
                DiagnosticProvider::Solver,
                "solver.violation",
                DiagnosticSeverity::Violation,
            ),
            (
                DiagnosticProvider::Solver,
                "room.boundary.open",
                DiagnosticSeverity::Warning,
            ),
            (
                DiagnosticProvider::Solver,
                "roof.outline.degenerate",
                DiagnosticSeverity::Warning,
            ),
            (
                DiagnosticProvider::Library,
                "library.item.out-of-date",
                DiagnosticSeverity::Warning,
            ),
            (
                DiagnosticProvider::Library,
                "library.lifecycle.check-failed",
                DiagnosticSeverity::Warning,
            ),
            (
                DiagnosticProvider::Library,
                "library.item.source-missing",
                DiagnosticSeverity::Warning,
            ),
            (
                DiagnosticProvider::Solver,
                "solver.needs-review",
                DiagnosticSeverity::NeedsReview,
            ),
            (
                DiagnosticProvider::Solver,
                "solver.unsupported",
                DiagnosticSeverity::Unsupported,
            ),
            (
                DiagnosticProvider::Solver,
                "solver.scope.info",
                DiagnosticSeverity::Info,
            ),
        ];
        let diagnostics = cases
            .into_iter()
            .enumerate()
            .map(|(ordinal, (provider, code, severity))| {
                (
                    PlanDiagnostic {
                        severity,
                        code: code.to_owned(),
                        source: None,
                        message: format!("message {ordinal}"),
                        rule: None,
                    },
                    DiagnosticRef {
                        revision,
                        provider,
                        code: code.to_owned(),
                        source: None,
                        ordinal: u32::try_from(ordinal).unwrap(),
                    },
                )
            })
            .collect();
        let mut records = Vec::new();
        lower_nonstandards_diagnostics(diagnostics, revision, &mut records);

        assert!(matches!(
            &records[0],
            IntentRecord::Boolean(BooleanIntentRecord {
                mode: BooleanIntentMode::Requirement,
                outcome: IntentOutcome::Violated,
                ..
            })
        ));
        assert!(matches!(
            &records[1],
            IntentRecord::Boolean(BooleanIntentRecord {
                mode: BooleanIntentMode::Requirement,
                outcome: IntentOutcome::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::MissingInput,
                    ..
                }),
                ..
            })
        ));
        assert!(matches!(
            &records[2],
            IntentRecord::Boolean(BooleanIntentRecord {
                mode: BooleanIntentMode::Requirement,
                outcome: IntentOutcome::Violated,
                ..
            })
        ));
        assert!(matches!(
            &records[3],
            IntentRecord::Boolean(BooleanIntentRecord {
                mode: BooleanIntentMode::Preference { .. },
                outcome: IntentOutcome::Violated,
                ..
            })
        ));
        assert!(matches!(
            &records[4],
            IntentRecord::Boolean(BooleanIntentRecord {
                outcome: IntentOutcome::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::EvaluationUnavailable,
                    ..
                }),
                ..
            })
        ));
        assert!(matches!(
            &records[5],
            IntentRecord::Boolean(BooleanIntentRecord {
                outcome: IntentOutcome::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::UnresolvedReference,
                    ..
                }),
                ..
            })
        ));
        assert!(matches!(
            &records[6],
            IntentRecord::Boolean(BooleanIntentRecord {
                outcome: IntentOutcome::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::MissingInput,
                    ..
                }),
                ..
            })
        ));
        assert!(matches!(
            &records[7],
            IntentRecord::Boolean(BooleanIntentRecord {
                outcome: IntentOutcome::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::UnsupportedCondition,
                    ..
                }),
                ..
            })
        ));
        assert!(matches!(&records[8], IntentRecord::Assumption(_)));
    }

    #[test]
    fn scoped_standards_waiver_shares_one_override_and_retains_native_evidence() {
        let mut model = BuildingModel::demo_shell();
        let pack = &mut model.standards_packs[0];
        let overlay_pack = pack.id.clone();
        pack.checks.push(ComplianceCheck {
            rule: "test.all-walls-waived".to_owned(),
            citation: "Test".to_owned(),
            title: "All wall heights".to_owned(),
            severity: CheckSeverity::Required,
            applies: Applicability::Always,
            scope: CheckScope::Walls {
                exterior_only: None,
                tags: Vec::new(),
            },
            requirement: Predicate::Compare {
                fact: Fact::WallHeight,
                op: CompareOp::Le,
                value: FactOperand::LengthLiteral(framer_core::Length::from_feet(20.0)),
            },
        });
        pack.overlays.push(RuleOverlay::Waive {
            target: "test.all-walls-waived".to_owned(),
            reason: "approved alternate".to_owned(),
        });
        model.validate().unwrap();

        let analysis = crate::analyze_project(&model).unwrap();
        let report = analysis.intent_report.as_ref().unwrap();
        let legacy_entries = analysis
            .standards_evaluation
            .report
            .entries
            .iter()
            .filter(|entry| entry.rule == "test.all-walls-waived")
            .collect::<Vec<_>>();
        assert_eq!(legacy_entries.len(), 1);
        assert!(legacy_entries[0].element.is_none());

        let waived_records = report
            .records()
            .iter()
            .filter(|record| {
                matches!(
                    record,
                    IntentRecord::Boolean(BooleanIntentRecord {
                        assertion: CompiledAssertion {
                            reference: AssertionRef::Derived(DerivedAssertionId {
                                provider: DerivedAssertionProvider::Standards,
                                ..
                            }),
                            ..
                        },
                        outcome: IntentOutcome::Waived { .. },
                        ..
                    })
                ) && record.assertion().rationale.contains("All wall heights")
            })
            .collect::<Vec<_>>();
        assert_eq!(waived_records.len(), model.walls.len());
        assert_eq!(report.waivers().len(), 1);
        let waiver = &report.waivers()[0];
        assert_eq!(waiver.targets.len(), model.walls.len());
        assert_eq!(
            waiver.reference,
            WaiverRef::Standards {
                overlay_pack: overlay_pack.clone(),
                rule: "test.all-walls-waived".to_owned(),
            }
        );

        let graph = analysis.graph.as_ref().unwrap();
        let overlay_node = ProjectNodeRef::Authored(AuthoredEntityRef::StandardsPack(overlay_pack));
        for record in &waived_records {
            let assertion_node = ProjectNodeRef::Assertion(record.assertion().reference.clone());
            assert!(graph.edges().iter().any(|edge| {
                edge.relationship == RelationshipKind::WaivedBy
                    && edge.dependent == assertion_node
                    && edge.dependency == overlay_node
            }));
        }

        let evidence_ref = waived_records[0]
            .evidence()
            .iter()
            .find_map(|evidence| match evidence {
                IntentEvidenceRef::ComplianceEntry(reference) => Some(reference),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            analysis.compliance_entry(evidence_ref),
            Some(legacy_entries[0])
        );
        assert_eq!(
            analysis.standards_details_for(evidence_ref).len(),
            model.walls.len()
        );
    }

    #[test]
    fn geometry_findings_keep_native_diagnostics_and_witnesses_recoverable() {
        let model = BuildingModel::demo_wall();
        let mut plan = generate_project_plan(&model).unwrap();
        let resolved = model.resolved_standards();
        let standards = framer_standards::evaluate_detailed(&model, &resolved, &plan);
        plan.diagnostics.extend(standards.diagnostics());
        let assembly_body = BodyRef::assembly(model.walls[0].id.clone(), AssemblyKind::Wall);
        let member_body = BodyRef::member(
            model.walls[0].id.clone(),
            MemberKind::CommonStud,
            "mixed-order-stud",
        );
        let audit = GeometryAudit {
            violations: vec![
                GeometryViolation::BodyUnbuildable(GeometryBuildDiagnostic::unbuildable(
                    member_body.clone(),
                    "missing solid",
                )),
                GeometryViolation::BodyUnbuildable(GeometryBuildDiagnostic::unbuildable(
                    assembly_body.clone(),
                    "missing solid",
                )),
            ],
        };
        plan.diagnostics
            .extend(current_intent_plan_diagnostics(&model, &audit));
        let revision = GraphRevision::for_model(&model).unwrap();
        let report = compile_project_intent(&model, &plan, &audit, &standards, &[], revision);
        let geometry = report
            .records()
            .iter()
            .filter(|record| {
                matches!(
                    record,
                    IntentRecord::Boolean(BooleanIntentRecord {
                        assertion: CompiledAssertion {
                            reference: AssertionRef::Derived(DerivedAssertionId {
                                provider: DerivedAssertionProvider::Geometry,
                                ..
                            }),
                            ..
                        },
                        outcome: IntentOutcome::Violated,
                        ..
                    })
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(geometry.len(), 2);
        assert!(geometry.iter().all(|record| {
            record
                .evidence()
                .iter()
                .any(|evidence| matches!(evidence, IntentEvidenceRef::PhysicalBody(_)))
        }));
        let evidence_pairs = geometry
            .iter()
            .map(|record| {
                let diagnostic = record
                    .evidence()
                    .iter()
                    .find_map(|evidence| match evidence {
                        IntentEvidenceRef::Diagnostic(reference) => Some(reference.clone()),
                        _ => None,
                    })
                    .unwrap();
                let body = record
                    .evidence()
                    .iter()
                    .find_map(|evidence| match evidence {
                        IntentEvidenceRef::PhysicalBody(reference) => Some(reference.body.clone()),
                        _ => None,
                    })
                    .unwrap();
                (body, diagnostic)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            evidence_pairs
                .iter()
                .map(|(_, reference)| reference.ordinal)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([0, 1]),
        );
        assert_eq!(
            evidence_pairs
                .iter()
                .find(|(body, _)| body == &assembly_body)
                .map(|(_, reference)| reference.ordinal),
            Some(0),
            "plan diagnostic ordering places the finished-assembly display key first",
        );
        assert_eq!(
            evidence_pairs
                .iter()
                .find(|(body, _)| body == &member_body)
                .map(|(_, reference)| reference.ordinal),
            Some(1),
            "typed BodyRef ordering differs, so geometry must share the plan ordinal stream",
        );
        assert_eq!(
            evidence_pairs
                .iter()
                .map(|(body, _)| body)
                .collect::<Vec<_>>(),
            vec![&member_body, &assembly_body],
            "intent records remain canonically ordered by their typed physical-body source",
        );

        let analysis = crate::ProjectAnalysis {
            plan,
            resolved_standards: resolved,
            physical_scene: PhysicalScene::default(),
            geometry_audit: audit.clone(),
            standards_evaluation: standards,
            library_lifecycle: crate::LibraryLifecycleStatus::default(),
            intent_report: Ok(report),
            graph: Err(crate::AnalysisError::Project(
                "not compiled in this test".to_owned(),
            )),
        };
        for (body, reference) in &evidence_pairs {
            let diagnostic = analysis.plan_diagnostic(reference).unwrap();
            let violation = analysis.geometry_violation(reference).unwrap();
            assert_eq!(violation.body_a(), body);
            assert!(diagnostic.message.contains(&body.to_string()));
            assert_eq!(diagnostic.message, violation.to_string());
            assert_eq!(diagnostic.source.as_ref(), Some(violation.body_a().owner()));
        }

        let mut changed = model;
        changed.site.jurisdiction = "different revision".to_owned();
        let mut stale = evidence_pairs[0].1.clone();
        stale.revision = GraphRevision::for_model(&changed).unwrap();
        assert!(analysis.plan_diagnostic(&stale).is_none());
        assert!(analysis.geometry_violation(&stale).is_none());
    }

    #[test]
    fn lowers_driving_but_not_reference_dimensions() {
        let mut model = BuildingModel::demo_shell();
        let wall = &mut model.walls[0];
        wall.dimensions.push(DimensionConstraint::new(
            "dimension-driving",
            "Overall wall length",
            DimensionKind::Driving,
            DimensionAnchor::WallStart,
            DimensionAnchor::WallEnd,
            DimensionDirection::Forward,
            Some(wall.length),
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "dimension-reference",
            "Measured wall length",
            DimensionKind::Reference,
            DimensionAnchor::WallStart,
            DimensionAnchor::WallEnd,
            DimensionDirection::Forward,
            None,
        ));
        let wall_id = wall.id.clone();
        let revision = GraphRevision::for_model(&model).unwrap();
        let report = compile_current_intent(&model, revision);

        let wall_records = report.assertions_for(&AuthoredEntityRef::Wall(wall_id));
        assert!(wall_records.iter().any(|record| {
            matches!(
                record,
                IntentRecord::Boolean(BooleanIntentRecord {
                    assertion: CompiledAssertion {
                        reference: AssertionRef::Derived(DerivedAssertionId {
                            role: DerivedAssertionRole::DrivingDimension,
                            ..
                        }),
                        ..
                    },
                    outcome: IntentOutcome::Satisfied,
                    ..
                })
            )
        }));
        assert!(
            report
                .assertions_for(&AuthoredEntityRef::Dimension(ElementId::new(
                    "dimension-reference"
                )))
                .is_empty()
        );
    }

    #[test]
    fn actionable_driving_dimension_reaches_the_plan_once_and_is_recoverable() {
        let mut model = BuildingModel::demo_shell();
        let mut plan = generate_project_plan(&model).unwrap();
        let revision = GraphRevision::for_model(&model).unwrap();
        let wall = &mut model.walls[0];
        wall.dimensions.push(DimensionConstraint::new(
            "dimension-unsatisfied",
            "Overall wall length",
            DimensionKind::Driving,
            DimensionAnchor::WallStart,
            DimensionAnchor::WallEnd,
            DimensionDirection::Forward,
            Some(wall.length - Length::from_whole_inches(1)),
        ));
        let audit = GeometryAudit::default();
        let diagnostics = current_intent_plan_diagnostics(&model, &audit);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Violation);
        assert_eq!(diagnostics[0].code, "intent.dimension.unsatisfied");
        plan.diagnostics.extend(diagnostics);

        let resolved = model.resolved_standards();
        let standards = framer_standards::evaluate_detailed(&model, &resolved, &plan);
        let report = compile_project_intent(&model, &plan, &audit, &standards, &[], revision);
        let record = report
            .assertions_for(&AuthoredEntityRef::Dimension(ElementId::new(
                "dimension-unsatisfied",
            )))
            .into_iter()
            .next()
            .unwrap();
        assert!(matches!(
            record,
            IntentRecord::Boolean(BooleanIntentRecord {
                outcome: IntentOutcome::Violated,
                ..
            })
        ));
        let reference = record
            .evidence()
            .iter()
            .find_map(|evidence| match evidence {
                IntentEvidenceRef::Diagnostic(reference) => Some(reference),
                _ => None,
            })
            .unwrap();
        assert_eq!(report.plan_diagnostic(reference), plan.diagnostics.last());
        assert_eq!(
            plan.diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "intent.dimension.unsatisfied")
                .count(),
            1,
        );
    }

    #[test]
    fn selections_and_missing_site_inputs_keep_their_real_modes() {
        let model = BuildingModel::new();
        let revision = GraphRevision::for_model(&model).unwrap();
        let report = compile_current_intent(&model, revision);

        assert_eq!(
            report
                .assertions_for(&AuthoredEntityRef::Site)
                .iter()
                .filter(|record| matches!(record, IntentRecord::Assumption(_)))
                .count(),
            5
        );
        assert!(
            report
                .assertions_for(&AuthoredEntityRef::Site)
                .iter()
                .all(|record| !matches!(record, IntentRecord::Boolean(_)))
        );

        let mut model = BuildingModel::demo_shell();
        model
            .site
            .properties
            .insert("coastal_exposure".to_owned(), PropertyValue::Flag(true));
        let revision = GraphRevision::for_model(&model).unwrap();
        let report = compile_current_intent(&model, revision);
        let construction_count = model.walls.len()
            + model.roof_planes.len()
            + model.ceilings.len()
            + model.floor_decks.len();
        assert_eq!(
            report
                .records()
                .iter()
                .filter(|record| matches!(
                    record,
                    IntentRecord::Boolean(BooleanIntentRecord {
                        expression: BooleanExpression::SelectedEntity { .. },
                        ..
                    })
                ))
                .count(),
            construction_count
        );
        assert_eq!(report.assertions_for(&AuthoredEntityRef::Site).len(), 6);
    }

    #[test]
    fn standards_referenced_missing_site_flags_materialize_typed_assumptions_once() {
        const KEY: &str = "missing-intent-fixture-flag";
        const RULE: &str = "test.missing-site-flag";

        let mut model = BuildingModel::demo_wall();
        model.standards_packs[0].checks.push(ComplianceCheck {
            rule: RULE.to_owned(),
            citation: "Test citation".to_owned(),
            title: "Missing site flag".to_owned(),
            severity: CheckSeverity::Required,
            applies: Applicability::All(vec![
                Applicability::SiteFlag {
                    key: KEY.to_owned(),
                },
                Applicability::Not(Box::new(Applicability::SiteFlag {
                    key: KEY.to_owned(),
                })),
            ]),
            scope: CheckScope::Walls {
                exterior_only: None,
                tags: Vec::new(),
            },
            requirement: Predicate::Compare {
                fact: Fact::WallHeight,
                op: CompareOp::Gt,
                value: FactOperand::LengthLiteral(Length::ZERO),
            },
        });

        let analysis = crate::analyze_project(&model).unwrap();
        let report = analysis.intent_report.as_ref().unwrap();
        let assumptions = report
            .records()
            .iter()
            .filter_map(|record| match record {
                IntentRecord::Assumption(record)
                    if matches!(
                        &record.assertion.reference,
                        AssertionRef::Derived(DerivedAssertionId {
                            role: DerivedAssertionRole::SiteAssumption(
                                SiteAssumptionKey::Property(key),
                            ),
                            ..
                        }) if key == KEY
                    ) =>
                {
                    Some(record)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(assumptions.len(), 1);
        assert!(matches!(
            &assumptions[0].evidence,
            AssumptionEvidence::Unavailable(IntentUnknown {
                kind: IntentUnknownKind::MissingInput,
                detail,
            }) if detail == "missing-intent-fixture-flag is not provided."
        ));
        let assumption_ref = assumptions[0].assertion.reference.clone();

        let standards_record = report
            .records()
            .iter()
            .find(|record| {
                matches!(
                    &record.assertion().source,
                    AssertionSource::StandardsRule(rule) if rule.rule == RULE
                )
            })
            .unwrap();
        assert!(
            standards_record
                .evidence()
                .contains(&IntentEvidenceRef::Assertion(assumption_ref.clone()))
        );
        assert!(report.record(&assumption_ref).is_some());
    }
}
