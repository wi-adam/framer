use std::collections::{BTreeMap, BTreeSet};

use framer_core::{AuthoredEntityRef, ElementId, Length, Predicate};

use crate::{
    AssertionRef, ComplianceEntryRef, DiagnosticRef, GeneratedMemberRef, GraphRevision,
    PhysicalBodyRef, StandardsRuleRef,
};

/// Product-wide classification for compiled intent. The vocabulary is deliberately broader than
/// today's assertion sources so later slices do not need to overload "compliance" for every kind
/// of design decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntentDomain {
    SpatialProgram,
    Construction,
    StructuralPerformance,
    EnvelopeBuildingScience,
    Mep,
    Compliance,
    Resource,
    FabricationInstallation,
    OperationalMaintenance,
    Aesthetic,
}

/// The semantic role an authored entity plays in one assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssertionParticipantRole {
    Subject,
    Host,
    Constraint,
    SelectedSystem,
    SitePremise,
    EvaluatedEntity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionParticipant {
    pub entity: AuthoredEntityRef,
    pub role: AssertionParticipantRole,
    /// Stable semantic position within the role-qualified expression. This is not a vector index
    /// from the persisted model.
    pub semantic_order: u32,
}

impl AssertionParticipant {
    pub const fn new(
        entity: AuthoredEntityRef,
        role: AssertionParticipantRole,
        semantic_order: u32,
    ) -> Self {
        Self {
            entity,
            role,
            semantic_order,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssertionSource {
    Project,
    Authored(AuthoredEntityRef),
    StandardsRule(StandardsRuleRef),
    Diagnostic(DiagnosticRef),
    PhysicalBody(PhysicalBodyRef),
}

/// Common assertion metadata, shared by boolean, objective, and assumption records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAssertion {
    pub reference: AssertionRef,
    pub domain: IntentDomain,
    /// Canonical, role-free projection used by selection lookup and deterministic presentation.
    pub scope: Vec<AuthoredEntityRef>,
    /// Role-qualified expression participants. Semantic order is preserved independently of the
    /// canonical scope projection.
    pub participants: Vec<AssertionParticipant>,
    pub source: AssertionSource,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreferencePriority(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BooleanIntentMode {
    Requirement,
    Preference { priority: PreferencePriority },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SelectionAttribute {
    ConstructionSystem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BooleanExpression {
    ExactLength {
        label: String,
        expected: Length,
        observed: Option<Length>,
    },
    SelectedEntity {
        attribute: SelectionAttribute,
        selected: ElementId,
    },
    Predicate(Predicate),
    Finding {
        code: String,
    },
}

impl BooleanExpression {
    /// Normalize a prohibition into the common positive requirement envelope. Evaluators only
    /// need to understand predicates; prohibition is represented exactly as predicate negation.
    pub fn prohibition(prohibited: Predicate) -> Self {
        Self::Predicate(Predicate::Not(Box::new(prohibited)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntentUnknownKind {
    MissingInput,
    UnresolvedSubject,
    UnresolvedReference,
    WrongSubjectKind,
    UnsupportedCondition,
    EvaluationUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentUnknown {
    pub kind: IntentUnknownKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WaiverRef {
    Project {
        override_id: ElementId,
    },
    Standards {
        overlay_pack: ElementId,
        rule: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaiverRecord {
    pub reference: WaiverRef,
    pub target: AssertionRef,
    pub source: AssertionSource,
    pub rationale: String,
    pub provenance: Vec<IntentEvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentOutcome {
    Satisfied,
    Violated,
    Unknown(IntentUnknown),
    Waived { waiver: WaiverRef, reason: String },
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectiveDirection {
    Minimize,
    Maximize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExactValue {
    Length(Length),
    Int(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectiveDefinition {
    pub component: String,
    pub direction: ObjectiveDirection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectiveObservation {
    Known(ExactValue),
    Unknown(IntentUnknown),
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntentValue {
    Length(Length),
    Int(i64),
    Text(String),
    Flag(bool),
    Entity(ElementId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssumptionPremise {
    pub label: String,
}

/// Assumptions state the available premise; they are never coerced into a boolean outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssumptionEvidence {
    Known(IntentValue),
    Unavailable(IntentUnknown),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntentEvidenceRef {
    Project,
    Authored(AuthoredEntityRef),
    GeneratedMember(GeneratedMemberRef),
    PhysicalBody(PhysicalBodyRef),
    StandardsRule(StandardsRuleRef),
    ComplianceEntry(ComplianceEntryRef),
    Diagnostic(DiagnosticRef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BooleanIntentRecord {
    pub assertion: CompiledAssertion,
    pub mode: BooleanIntentMode,
    pub expression: BooleanExpression,
    pub outcome: IntentOutcome,
    pub evidence: Vec<IntentEvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectiveIntentRecord {
    pub assertion: CompiledAssertion,
    pub objective: ObjectiveDefinition,
    pub observation: ObjectiveObservation,
    pub evidence: Vec<IntentEvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssumptionIntentRecord {
    pub assertion: CompiledAssertion,
    pub premise: AssumptionPremise,
    pub evidence: AssumptionEvidence,
    pub provenance: Vec<IntentEvidenceRef>,
}

/// The shape of this enum makes incompatible mode/result combinations unrepresentable: an
/// objective or assumption cannot accidentally be rendered as satisfied or violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentRecord {
    Boolean(BooleanIntentRecord),
    Objective(ObjectiveIntentRecord),
    Assumption(AssumptionIntentRecord),
}

impl IntentRecord {
    pub const fn assertion(&self) -> &CompiledAssertion {
        match self {
            Self::Boolean(record) => &record.assertion,
            Self::Objective(record) => &record.assertion,
            Self::Assumption(record) => &record.assertion,
        }
    }

    pub fn evidence(&self) -> &[IntentEvidenceRef] {
        match self {
            Self::Boolean(record) => &record.evidence,
            Self::Objective(record) => &record.evidence,
            Self::Assumption(record) => &record.provenance,
        }
    }
}

/// Canonical, revision-bound compilation of current project intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentReport {
    revision: GraphRevision,
    records: Vec<IntentRecord>,
    waivers: Vec<WaiverRecord>,
    participant_index: BTreeMap<AuthoredEntityRef, Vec<usize>>,
}

impl IntentReport {
    pub(crate) fn from_parts(
        revision: GraphRevision,
        records: Vec<IntentRecord>,
        waivers: Vec<WaiverRecord>,
    ) -> Self {
        let mut unique = BTreeMap::new();
        for mut record in records {
            canonicalize_record(&mut record);
            unique.insert(record.assertion().reference.clone(), record);
        }
        let records = unique.into_values().collect::<Vec<_>>();

        let mut participant_index = BTreeMap::<AuthoredEntityRef, Vec<usize>>::new();
        for (index, record) in records.iter().enumerate() {
            let participants = record
                .assertion()
                .participants
                .iter()
                .map(|participant| participant.entity.clone())
                .collect::<BTreeSet<_>>();
            for participant in participants {
                participant_index
                    .entry(participant)
                    .or_default()
                    .push(index);
            }
        }

        let mut waiver_map = BTreeMap::new();
        for mut waiver in waivers {
            waiver.provenance.sort();
            waiver.provenance.dedup();
            waiver_map.insert(waiver.reference.clone(), waiver);
        }

        Self {
            revision,
            records,
            waivers: waiver_map.into_values().collect(),
            participant_index,
        }
    }

    pub const fn revision(&self) -> GraphRevision {
        self.revision
    }

    pub fn records(&self) -> &[IntentRecord] {
        &self.records
    }

    pub fn waivers(&self) -> &[WaiverRecord] {
        &self.waivers
    }

    /// Query role-qualified participants, returning each assertion at most once even when the
    /// same entity occupies more than one role.
    pub fn assertions_for(&self, participant: &AuthoredEntityRef) -> Vec<&IntentRecord> {
        self.participant_index
            .get(participant)
            .into_iter()
            .flatten()
            .filter_map(|index| self.records.get(*index))
            .collect()
    }

    pub fn record(&self, reference: &AssertionRef) -> Option<&IntentRecord> {
        self.records
            .binary_search_by(|record| record.assertion().reference.cmp(reference))
            .ok()
            .and_then(|index| self.records.get(index))
    }
}

fn canonicalize_record(record: &mut IntentRecord) {
    let assertion = match record {
        IntentRecord::Boolean(record) => {
            record.evidence.sort();
            record.evidence.dedup();
            &mut record.assertion
        }
        IntentRecord::Objective(record) => {
            record.evidence.sort();
            record.evidence.dedup();
            &mut record.assertion
        }
        IntentRecord::Assumption(record) => {
            record.provenance.sort();
            record.provenance.dedup();
            &mut record.assertion
        }
    };
    assertion.scope.sort();
    assertion.scope.dedup();
    assertion.participants.sort_by(|left, right| {
        left.semantic_order
            .cmp(&right.semantic_order)
            .then_with(|| left.role.cmp(&right.role))
            .then_with(|| left.entity.cmp(&right.entity))
    });
    assertion.participants.dedup_by(|left, right| {
        left.entity == right.entity
            && left.role == right.role
            && left.semantic_order == right.semantic_order
    });
}

#[cfg(test)]
mod tests {
    use framer_core::{BuildingModel, CompareOp, Fact, FactOperand};

    use super::*;
    use crate::{
        DerivedAssertionId, DerivedAssertionProvider, DerivedAssertionRole, DerivedAssertionSource,
    };

    #[test]
    fn participant_lookup_deduplicates_role_overlap() {
        let revision = GraphRevision::for_model(&BuildingModel::new()).unwrap();
        let wall = AuthoredEntityRef::Wall(ElementId::new("wall-a"));
        let reference = AssertionRef::Derived(DerivedAssertionId::new(
            revision,
            DerivedAssertionProvider::Core,
            DerivedAssertionSource::Authored(wall.clone()),
            DerivedAssertionRole::DrivingDimension,
        ));
        let report = IntentReport::from_parts(
            revision,
            vec![IntentRecord::Boolean(BooleanIntentRecord {
                assertion: CompiledAssertion {
                    reference: reference.clone(),
                    domain: IntentDomain::SpatialProgram,
                    scope: vec![wall.clone(), wall.clone()],
                    participants: vec![
                        AssertionParticipant::new(
                            wall.clone(),
                            AssertionParticipantRole::Subject,
                            0,
                        ),
                        AssertionParticipant::new(wall.clone(), AssertionParticipantRole::Host, 1),
                    ],
                    source: AssertionSource::Authored(wall.clone()),
                    rationale: "test".to_owned(),
                },
                mode: BooleanIntentMode::Requirement,
                expression: BooleanExpression::Finding {
                    code: "test".to_owned(),
                },
                outcome: IntentOutcome::Satisfied,
                evidence: vec![IntentEvidenceRef::Authored(wall.clone())],
            })],
            Vec::new(),
        );

        assert_eq!(report.assertions_for(&wall).len(), 1);
        assert_eq!(report.records()[0].assertion().scope, vec![wall]);
        assert!(report.record(&reference).is_some());
    }

    #[test]
    fn prohibition_is_normalized_as_predicate_negation() {
        let prohibited = Predicate::Compare {
            fact: Fact::WallHeight,
            op: CompareOp::Gt,
            value: FactOperand::LengthLiteral(Length::from_feet(12.0)),
        };
        assert_eq!(
            BooleanExpression::prohibition(prohibited.clone()),
            BooleanExpression::Predicate(Predicate::Not(Box::new(prohibited)))
        );
    }
}
