use framer_core::{AuthoredEntityRef, BuildingModel, DimensionKind, ElementId, PropertyValue};

use crate::{
    AssertionParticipant, AssertionParticipantRole, AssertionRef, AssertionSource,
    AssumptionEvidence, AssumptionIntentRecord, AssumptionPremise, BooleanExpression,
    BooleanIntentMode, BooleanIntentRecord, CompiledAssertion, DerivedAssertionId,
    DerivedAssertionProvider, DerivedAssertionRole, DerivedAssertionSource, GraphRevision,
    IntentDomain, IntentEvidenceRef, IntentOutcome, IntentRecord, IntentReport, IntentUnknown,
    IntentUnknownKind, IntentValue, SelectionAttribute, SiteAssumptionKey,
};

pub(crate) fn compile_current_intent(
    model: &BuildingModel,
    revision: GraphRevision,
) -> IntentReport {
    let mut records = Vec::new();
    compile_driving_dimensions(model, revision, &mut records);
    compile_construction_selections(model, revision, &mut records);
    compile_site_premises(model, revision, &mut records);
    IntentReport::from_parts(revision, records, Vec::new())
}

fn compile_driving_dimensions(
    model: &BuildingModel,
    revision: GraphRevision,
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
            records.push(IntentRecord::Boolean(BooleanIntentRecord {
                assertion: CompiledAssertion {
                    reference: AssertionRef::Derived(DerivedAssertionId::new(
                        revision,
                        DerivedAssertionProvider::Core,
                        DerivedAssertionSource::Authored(dimension_ref.clone()),
                        DerivedAssertionRole::DrivingDimension,
                    )),
                    domain: IntentDomain::SpatialProgram,
                    scope: vec![wall_ref.clone(), dimension_ref.clone()],
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
                evidence: vec![
                    IntentEvidenceRef::Authored(dimension_ref),
                    IntentEvidenceRef::Authored(wall_ref),
                ],
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
            scope: vec![host.clone(), system.clone()],
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
        evidence: vec![
            IntentEvidenceRef::Authored(host),
            IntentEvidenceRef::Authored(system),
        ],
    }));
}

fn compile_site_premises(
    model: &BuildingModel,
    revision: GraphRevision,
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
    for (key, value) in &site.properties {
        site_assumption(
            revision,
            SiteAssumptionKey::Property(key.clone()),
            key,
            Some(property_value(value)),
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
            reference: AssertionRef::Derived(DerivedAssertionId::new(
                revision,
                DerivedAssertionProvider::Core,
                DerivedAssertionSource::Authored(site.clone()),
                DerivedAssertionRole::SiteAssumption(key),
            )),
            domain: IntentDomain::Compliance,
            scope: vec![site.clone()],
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

fn property_value(value: &PropertyValue) -> IntentValue {
    match value {
        PropertyValue::Int(value) => IntentValue::Int(*value),
        PropertyValue::Length(value) => IntentValue::Length(*value),
        PropertyValue::Text(value) => IntentValue::Text(value.clone()),
        PropertyValue::Flag(value) => IntentValue::Flag(*value),
    }
}

#[cfg(test)]
mod tests {
    use framer_core::{DimensionAnchor, DimensionConstraint, DimensionDirection, DimensionKind};

    use super::*;

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
}
