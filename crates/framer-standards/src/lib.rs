use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use framer_core::{
    Applicability, BoardProfile, BracedPanel, BracedWallLine, BracingMethod, BracingRow,
    BuildingModel, CheckScope, CheckSeverity, CompareOp, ComplianceCheck, ElementId, Fact,
    FactOperand, FactSubjectKind, HeaderRow, Length, Opening, Predicate, PropertyValue,
    ResolutionAction, ResolvedRule, ResolvedStandards, SiteContext, Wall, WallExposure,
};
use framer_solver::{
    DiagnosticSeverity, FrameMember, MemberKind, PlanDiagnostic, ProjectFramePlan, RuleRef,
};
use serde::{Deserialize, Serialize};

const BRACING_UNASSOCIATED_PANEL: &str = "standards.bracing.unassociated-panel";
const BRACING_OUT_OF_DOMAIN: &str = "standards.bracing.out-of-domain";
const BRACING_ASSOCIATION_TOLERANCE: Length = Length::from_whole_inches(48);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tri {
    False,
    Unknown,
    True,
}

impl Tri {
    pub const fn not(self) -> Self {
        match self {
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
            Self::True => Self::False,
        }
    }

    pub fn all(values: impl IntoIterator<Item = Self>) -> Self {
        values.into_iter().min().unwrap_or(Self::True)
    }

    pub fn any(values: impl IntoIterator<Item = Self>) -> Self {
        values.into_iter().max().unwrap_or(Self::False)
    }
}

impl From<bool> for Tri {
    fn from(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactValue {
    Length(Length),
    Int(i64),
    Flag(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FactSubject {
    Wall(ElementId),
    Opening(ElementId),
    Room(ElementId),
    BracedWallLine(ElementId),
}

impl FactSubject {
    pub const fn subject_kind(&self) -> FactSubjectKind {
        match self {
            Self::Wall(_) => FactSubjectKind::Wall,
            Self::Opening(_) => FactSubjectKind::Opening,
            Self::Room(_) => FactSubjectKind::Room,
            Self::BracedWallLine(_) => FactSubjectKind::BracedWallLine,
        }
    }

    pub fn element(&self) -> &ElementId {
        match self {
            Self::Wall(id) | Self::Opening(id) | Self::Room(id) | Self::BracedWallLine(id) => id,
        }
    }
}

/// Backwards-compatible name for the original standards evaluator subject.
pub type EntityRef = FactSubject;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactUnknownKind {
    MissingInput,
    UnresolvedSubject,
    WrongSubjectKind,
    UnsupportedCondition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactUnknown {
    pub fact: Fact,
    pub subject: FactSubject,
    pub kind: FactUnknownKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactObservation {
    Known(FactValue),
    Unknown(FactUnknown),
}

impl FactObservation {
    pub const fn known_value(&self) -> Option<FactValue> {
        match self {
            Self::Known(value) => Some(*value),
            Self::Unknown(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedFact {
    pub fact: Fact,
    pub observation: FactObservation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredicateObservation {
    pub result: Tri,
    pub observed_facts: Vec<ObservedFact>,
}

/// One immutable, revision-local view of all quantitative standards inputs.
///
/// Both standards checks and intent analysis use this concrete provider so a
/// given fact/subject pair has one measurement implementation.
#[derive(Debug, Clone, Copy)]
pub struct FactSnapshot<'a> {
    model: &'a BuildingModel,
    resolved: &'a ResolvedStandards,
    plan: &'a ProjectFramePlan,
}

impl<'a> FactSnapshot<'a> {
    pub const fn new(
        model: &'a BuildingModel,
        resolved: &'a ResolvedStandards,
        plan: &'a ProjectFramePlan,
    ) -> Self {
        Self {
            model,
            resolved,
            plan,
        }
    }

    pub fn subjects_for(&self, scope: &CheckScope) -> Vec<FactSubject> {
        let mut subjects = scoped_entities(self.model, scope.clone());
        subjects.sort();
        subjects.dedup();
        subjects
    }

    pub fn observe(&self, fact: Fact, subject: &FactSubject) -> FactObservation {
        if fact.subject_kind() != subject.subject_kind() {
            return self.unknown(fact, subject, FactUnknownKind::WrongSubjectKind);
        }
        if !self.subject_exists(subject) {
            return self.unknown(fact, subject, FactUnknownKind::UnresolvedSubject);
        }

        match raw_fact_value(fact, subject, self.model, self.resolved, self.plan) {
            Ok(value) => FactObservation::Known(value),
            Err(kind) => self.unknown(fact, subject, kind),
        }
    }

    pub fn evaluate_predicate(
        &self,
        predicate: &Predicate,
        subject: &FactSubject,
    ) -> PredicateObservation {
        let mut observed = BTreeMap::new();
        let result = self.predicate_result(predicate, subject, &mut observed);
        PredicateObservation {
            result,
            observed_facts: observed
                .into_iter()
                .map(|(fact, observation)| ObservedFact { fact, observation })
                .collect(),
        }
    }

    fn predicate_result(
        &self,
        predicate: &Predicate,
        subject: &FactSubject,
        observed: &mut BTreeMap<Fact, FactObservation>,
    ) -> Tri {
        match predicate {
            Predicate::All(children) => Tri::all(
                children
                    .iter()
                    .map(|child| self.predicate_result(child, subject, observed)),
            ),
            Predicate::Any(children) => Tri::any(
                children
                    .iter()
                    .map(|child| self.predicate_result(child, subject, observed)),
            ),
            Predicate::Not(child) => self.predicate_result(child, subject, observed).not(),
            Predicate::Compare { fact, op, value } => {
                let Some(left) = self.observed_value(*fact, subject, observed) else {
                    return Tri::Unknown;
                };
                let Some(right) = self.operand_value(value, subject, observed) else {
                    return Tri::Unknown;
                };
                compare_fact_values(left, *op, right)
            }
        }
    }

    fn operand_value(
        &self,
        operand: &FactOperand,
        subject: &FactSubject,
        observed: &mut BTreeMap<Fact, FactObservation>,
    ) -> Option<FactValue> {
        match operand {
            FactOperand::LengthLiteral(length) => Some(FactValue::Length(*length)),
            FactOperand::IntLiteral(value) => Some(FactValue::Int(*value)),
            FactOperand::FlagLiteral(value) => Some(FactValue::Flag(*value)),
            FactOperand::Fact(fact) => self.observed_value(*fact, subject, observed),
        }
    }

    fn observed_value(
        &self,
        fact: Fact,
        subject: &FactSubject,
        observed: &mut BTreeMap<Fact, FactObservation>,
    ) -> Option<FactValue> {
        observed
            .entry(fact)
            .or_insert_with(|| self.observe(fact, subject))
            .known_value()
    }

    fn subject_exists(&self, subject: &FactSubject) -> bool {
        match subject {
            FactSubject::Wall(id) => find_wall(self.model, id).is_some(),
            FactSubject::Opening(id) => find_opening(self.model, id).is_some(),
            FactSubject::Room(id) => self.model.rooms.iter().any(|room| room.id == *id),
            FactSubject::BracedWallLine(id) => find_braced_line(self.model, id).is_some(),
        }
    }

    fn unknown(&self, fact: Fact, subject: &FactSubject, kind: FactUnknownKind) -> FactObservation {
        FactObservation::Unknown(FactUnknown {
            fact,
            subject: subject.clone(),
            kind,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Pass,
    Violation,
    Advisory,
    NeedsReview,
    NotApplicable,
    Waived { reason: String },
}

impl Outcome {
    fn label(&self) -> &'static str {
        match self {
            Self::Pass => "Pass",
            Self::Violation => "Violation",
            Self::Advisory => "Advisory",
            Self::NeedsReview => "NeedsReview",
            Self::NotApplicable => "NotApplicable",
            Self::Waived { .. } => "Waived",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplianceEntry {
    pub rule: String,
    pub citation: String,
    pub pack: ElementId,
    pub outcome: Outcome,
    pub element: Option<ElementId>,
    pub message: String,
    pub chain: Vec<(ElementId, ResolutionAction)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub entries: Vec<ComplianceEntry>,
}

impl ComplianceReport {
    pub fn to_csv(&self) -> String {
        let mut csv = "rule,citation,pack,outcome,element,message,chain\n".to_owned();
        for entry in &self.entries {
            let fields = [
                entry.rule.clone(),
                entry.citation.clone(),
                entry.pack.0.clone(),
                entry.outcome.label().to_owned(),
                entry
                    .element
                    .as_ref()
                    .map(|id| id.0.clone())
                    .unwrap_or_default(),
                entry.message.clone(),
                entry
                    .chain
                    .iter()
                    .map(|(pack, action)| format!("{}:{action:?}", pack.0))
                    .collect::<Vec<_>>()
                    .join(";"),
            ];
            csv.push_str(
                &fields
                    .iter()
                    .map(|field| csv_field(field))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            csv.push('\n');
        }
        csv
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SyntheticEntryKind {
    UnassociatedBracingPanel,
    BracingOutOfDomain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveWaiver {
    pub reason: String,
    pub overlay_pack: ElementId,
    pub chain: Vec<(ElementId, ResolutionAction)>,
}

/// Evidence for one logical subject behind a compliance report entry.
///
/// `report_entry_index` indexes `StandardsEvaluation::report.entries`. Several
/// details may intentionally point at the same legacy subjectless entry (most
/// notably a scoped waived rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardsEvaluationDetail {
    pub report_entry_index: usize,
    pub check_id: Option<String>,
    pub definition_pack: Option<ElementId>,
    pub check_definition: Option<ComplianceCheck>,
    pub severity: Option<CheckSeverity>,
    pub subject: Option<FactSubject>,
    pub scope_subjects: Vec<FactSubject>,
    pub applicability: Option<Tri>,
    pub predicate: Option<PredicateObservation>,
    pub synthetic_kind: Option<SyntheticEntryKind>,
    pub effective_waiver: Option<EffectiveWaiver>,
}

impl StandardsEvaluationDetail {
    pub fn is_unsupported(&self) -> bool {
        self.synthetic_kind == Some(SyntheticEntryKind::BracingOutOfDomain)
            || self.predicate.as_ref().is_some_and(|predicate| {
                predicate.observed_facts.iter().any(|observed| {
                    matches!(
                        &observed.observation,
                        FactObservation::Unknown(unknown)
                            if unknown.kind == FactUnknownKind::UnsupportedCondition
                    )
                })
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StandardsEvaluation {
    pub report: ComplianceReport,
    pub details: Vec<StandardsEvaluationDetail>,
}

impl StandardsEvaluation {
    /// Lower the detailed evaluation through the canonical application diagnostics matrix.
    /// Detail is required to distinguish a missing fact from an unsupported fact without
    /// changing the frozen `ComplianceReport` payload.
    pub fn diagnostics(&self) -> Vec<PlanDiagnostic> {
        self.report
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let severity = match entry.outcome {
                    Outcome::Violation => DiagnosticSeverity::Violation,
                    Outcome::Advisory => DiagnosticSeverity::Warning,
                    Outcome::NeedsReview => {
                        if self
                            .details
                            .iter()
                            .filter(|detail| detail.report_entry_index == index)
                            .any(StandardsEvaluationDetail::is_unsupported)
                        {
                            DiagnosticSeverity::Unsupported
                        } else {
                            DiagnosticSeverity::NeedsReview
                        }
                    }
                    Outcome::Pass | Outcome::NotApplicable | Outcome::Waived { .. } => {
                        return None;
                    }
                };
                Some(PlanDiagnostic {
                    severity,
                    code: entry.rule.clone(),
                    source: entry.element.clone(),
                    message: entry.message.clone(),
                    rule: Some(RuleRef {
                        pack: entry.pack.clone(),
                        rule: entry.rule.clone(),
                        citation: entry.citation.clone(),
                    }),
                })
            })
            .collect()
    }
}

#[derive(Debug)]
struct PendingEvaluationEntry {
    report: ComplianceEntry,
    details: Vec<StandardsEvaluationDetail>,
}

pub fn evaluate(
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> ComplianceReport {
    evaluate_detailed(model, resolved, plan).report
}

pub fn evaluate_detailed(
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> StandardsEvaluation {
    let snapshot = FactSnapshot::new(model, resolved, plan);
    let active_checks = resolved
        .checks
        .iter()
        .map(|(pack, check)| (check.rule.as_str(), (pack, check)))
        .collect::<BTreeMap<_, _>>();
    let mut check_definitions = active_checks.clone();
    for (pack, check) in &resolved.check_definitions {
        check_definitions.insert(check.rule.as_str(), (pack, check));
    }
    let mut pending = bracing_diagnostic_entries(model, resolved)
        .into_iter()
        .map(|report| {
            let (synthetic_kind, subject) = match report.rule.as_str() {
                BRACING_UNASSOCIATED_PANEL => {
                    (Some(SyntheticEntryKind::UnassociatedBracingPanel), None)
                }
                BRACING_OUT_OF_DOMAIN => (
                    Some(SyntheticEntryKind::BracingOutOfDomain),
                    report.element.clone().map(FactSubject::BracedWallLine),
                ),
                _ => (None, None),
            };
            let scope_subjects = subject.iter().cloned().collect();
            PendingEvaluationEntry {
                report,
                details: vec![StandardsEvaluationDetail {
                    report_entry_index: 0,
                    check_id: None,
                    definition_pack: None,
                    check_definition: None,
                    severity: None,
                    subject,
                    scope_subjects,
                    applicability: None,
                    predicate: None,
                    synthetic_kind,
                    effective_waiver: None,
                }],
            }
        })
        .collect::<Vec<_>>();

    for rule in resolved.rules.iter().filter(|rule| rule.severity.is_some()) {
        if let Some(reason) = &rule.waived {
            let definition = check_definitions.get(rule.rule.as_str()).copied();
            let scope_subjects = definition
                .map(|(_, check)| snapshot.subjects_for(&check.scope))
                .unwrap_or_default();
            let applicability_observation =
                definition.map(|(_, check)| applicability(check.applies.clone(), &model.site));
            let effective_waiver = effective_waiver(rule);
            let detail_subjects = optional_subjects(&scope_subjects);
            let details = detail_subjects
                .into_iter()
                .map(|subject| StandardsEvaluationDetail {
                    report_entry_index: 0,
                    check_id: definition.map(|(_, check)| check.rule.clone()),
                    definition_pack: definition.map(|(pack, _)| pack.clone()),
                    check_definition: definition.map(|(_, check)| check.clone()),
                    severity: rule.severity,
                    predicate: match (definition, applicability_observation, &subject) {
                        (Some((_, check)), Some(Tri::True), Some(subject)) => {
                            Some(snapshot.evaluate_predicate(&check.requirement, subject))
                        }
                        _ => None,
                    },
                    subject,
                    scope_subjects: scope_subjects.clone(),
                    applicability: applicability_observation,
                    synthetic_kind: None,
                    effective_waiver: effective_waiver.clone(),
                })
                .collect();
            pending.push(PendingEvaluationEntry {
                report: entry(
                    rule,
                    Outcome::Waived {
                        reason: reason.clone(),
                    },
                    None,
                    format!("Waived: {reason}"),
                ),
                details,
            });
            continue;
        }

        let Some((_, check)) = active_checks.get(rule.rule.as_str()) else {
            continue;
        };
        let definition = check_definitions
            .get(rule.rule.as_str())
            .copied()
            .unwrap_or_else(|| active_checks[rule.rule.as_str()]);
        let scope_subjects = snapshot.subjects_for(&check.scope);
        let applicability_observation = applicability(check.applies.clone(), &model.site);

        match applicability_observation {
            Tri::False => {
                pending.push(PendingEvaluationEntry {
                    report: entry(
                        rule,
                        Outcome::NotApplicable,
                        None,
                        format!("{} is not applicable for this site context.", check.title),
                    ),
                    details: optional_subjects(&scope_subjects)
                        .into_iter()
                        .map(|subject| {
                            check_detail(
                                definition,
                                rule,
                                subject,
                                scope_subjects.clone(),
                                applicability_observation,
                                None,
                            )
                        })
                        .collect(),
                });
            }
            Tri::Unknown => {
                pending.push(PendingEvaluationEntry {
                    report: entry(
                        rule,
                        Outcome::NeedsReview,
                        None,
                        format!("{} applicability needs review.", check.title),
                    ),
                    details: optional_subjects(&scope_subjects)
                        .into_iter()
                        .map(|subject| {
                            check_detail(
                                definition,
                                rule,
                                subject,
                                scope_subjects.clone(),
                                applicability_observation,
                                None,
                            )
                        })
                        .collect(),
                });
            }
            Tri::True => {
                for subject in &scope_subjects {
                    let predicate = snapshot.evaluate_predicate(&check.requirement, subject);
                    let outcome = match predicate.result {
                        Tri::True => Outcome::Pass,
                        Tri::False => match check.severity {
                            CheckSeverity::Required => Outcome::Violation,
                            CheckSeverity::Advisory => Outcome::Advisory,
                        },
                        Tri::Unknown => Outcome::NeedsReview,
                    };
                    pending.push(PendingEvaluationEntry {
                        report: entry(
                            rule,
                            outcome.clone(),
                            Some(subject.element().clone()),
                            outcome_message(&check.title, &outcome),
                        ),
                        details: vec![check_detail(
                            definition,
                            rule,
                            Some(subject.clone()),
                            scope_subjects.clone(),
                            applicability_observation,
                            Some(predicate),
                        )],
                    });
                }
            }
        }
    }

    pending.sort_by(|left, right| {
        left.report
            .rule
            .cmp(&right.report.rule)
            .then_with(|| left.report.element.cmp(&right.report.element))
    });
    let mut report = ComplianceReport::default();
    let mut details = Vec::new();
    for (report_entry_index, mut pending_entry) in pending.into_iter().enumerate() {
        pending_entry
            .details
            .sort_by(|left, right| left.subject.cmp(&right.subject));
        for detail in &mut pending_entry.details {
            detail.report_entry_index = report_entry_index;
        }
        report.entries.push(pending_entry.report);
        details.extend(pending_entry.details);
    }
    StandardsEvaluation { report, details }
}

fn optional_subjects(subjects: &[FactSubject]) -> Vec<Option<FactSubject>> {
    if subjects.is_empty() {
        vec![None]
    } else {
        subjects.iter().cloned().map(Some).collect()
    }
}

fn check_detail(
    definition: (&ElementId, &ComplianceCheck),
    rule: &ResolvedRule,
    subject: Option<FactSubject>,
    scope_subjects: Vec<FactSubject>,
    applicability: Tri,
    predicate: Option<PredicateObservation>,
) -> StandardsEvaluationDetail {
    StandardsEvaluationDetail {
        report_entry_index: 0,
        check_id: Some(definition.1.rule.clone()),
        definition_pack: Some(definition.0.clone()),
        check_definition: Some(definition.1.clone()),
        severity: rule.severity,
        subject,
        scope_subjects,
        applicability: Some(applicability),
        predicate,
        synthetic_kind: None,
        effective_waiver: None,
    }
}

fn effective_waiver(rule: &ResolvedRule) -> Option<EffectiveWaiver> {
    let reason = rule.waived.clone()?;
    let overlay_pack =
        rule.chain.iter().rev().find_map(|(pack, action)| {
            (*action == ResolutionAction::Waived).then(|| pack.clone())
        })?;
    Some(EffectiveWaiver {
        reason,
        overlay_pack,
        chain: rule.chain.clone(),
    })
}

/// Compatibility lowering for callers that retain only the frozen v1 report.
///
/// Prefer [`StandardsEvaluation::diagnostics`] when detailed evaluation is available; the report
/// alone cannot distinguish missing facts from unsupported facts except for legacy synthetic
/// bracing entries.
pub fn diagnostics(report: &ComplianceReport) -> Vec<PlanDiagnostic> {
    report
        .entries
        .iter()
        .filter_map(|entry| {
            let severity = match entry.outcome {
                Outcome::Violation => DiagnosticSeverity::Violation,
                Outcome::Advisory => DiagnosticSeverity::Warning,
                Outcome::NeedsReview if entry.rule == BRACING_OUT_OF_DOMAIN => {
                    DiagnosticSeverity::Unsupported
                }
                Outcome::NeedsReview => DiagnosticSeverity::NeedsReview,
                Outcome::Pass | Outcome::NotApplicable | Outcome::Waived { .. } => return None,
            };
            Some(PlanDiagnostic {
                severity,
                code: entry.rule.clone(),
                source: entry.element.clone(),
                message: entry.message.clone(),
                rule: Some(RuleRef {
                    pack: entry.pack.clone(),
                    rule: entry.rule.clone(),
                    citation: entry.citation.clone(),
                }),
            })
        })
        .collect()
}

pub fn fact_value(
    fact: Fact,
    entity: &EntityRef,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> Option<FactValue> {
    FactSnapshot::new(model, resolved, plan)
        .observe(fact, entity)
        .known_value()
}

fn raw_fact_value(
    fact: Fact,
    entity: &FactSubject,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> Result<FactValue, FactUnknownKind> {
    use FactUnknownKind::{MissingInput, UnsupportedCondition, WrongSubjectKind};

    match (fact, entity) {
        (Fact::WallLength, EntityRef::Wall(wall)) => Ok(FactValue::Length(
            find_wall(model, wall).ok_or(MissingInput)?.length,
        )),
        (Fact::WallHeight, EntityRef::Wall(wall)) => Ok(FactValue::Length(
            find_wall(model, wall).ok_or(MissingInput)?.height,
        )),
        (Fact::WallIsExterior, EntityRef::Wall(wall)) => {
            let wall = find_wall(model, wall).ok_or(MissingInput)?;
            let system = model.system_for(wall).ok_or(MissingInput)?;
            Ok(FactValue::Flag(system.exposure() == WallExposure::Exterior))
        }
        (Fact::WallStudSpacing, EntityRef::Wall(wall)) => {
            let wall = find_wall(model, wall).ok_or(MissingInput)?;
            let system = model.system_for(wall).ok_or(MissingInput)?;
            Ok(FactValue::Length(
                system
                    .framing_layer()
                    .and_then(|layer| layer.framing.as_ref())
                    .ok_or(MissingInput)?
                    .spacing,
            ))
        }
        (Fact::WallSystemRValueMilli, EntityRef::Wall(wall)) => {
            let wall = find_wall(model, wall).ok_or(MissingInput)?;
            let system = model.system_for(wall).ok_or(MissingInput)?;
            Ok(FactValue::Int(system.r_value_milli(&model.materials)))
        }
        (Fact::WallStudMaxHeight, EntityRef::Wall(wall)) => {
            let wall = find_wall(model, wall).ok_or(MissingInput)?;
            Ok(FactValue::Length(
                wall_stud_max_height(wall.id.clone(), model, resolved)
                    .ok_or(UnsupportedCondition)?,
            ))
        }
        (Fact::OpeningRoughWidth, EntityRef::Opening(opening)) => Ok(FactValue::Length(
            find_opening(model, opening).ok_or(MissingInput)?.1.width,
        )),
        (Fact::OpeningRoughHeight, EntityRef::Opening(opening)) => Ok(FactValue::Length(
            find_opening(model, opening).ok_or(MissingInput)?.1.height,
        )),
        (Fact::OpeningHeaderDepth, EntityRef::Opening(opening)) => {
            let header = opening_headers(plan, opening)
                .into_iter()
                .next()
                .ok_or(MissingInput)?;
            Ok(FactValue::Length(header.cross_section_depth))
        }
        (Fact::OpeningJackStuds, EntityRef::Opening(opening)) => {
            let members = opening_members(plan, opening);
            let count = members
                .into_iter()
                .filter(|member| member.kind == MemberKind::JackStud)
                .count()
                / 2;
            i64::try_from(count)
                .map(FactValue::Int)
                .map_err(|_| UnsupportedCondition)
        }
        (Fact::OpeningHeaderMaxSpan, EntityRef::Opening(opening)) => {
            let (_, opening_model) = find_opening(model, opening).ok_or(MissingInput)?;
            if opening_headers(plan, opening).is_empty() {
                return Err(MissingInput);
            }
            Ok(FactValue::Length(
                opening_header_max_span(opening_model, opening, model, resolved, plan)
                    .ok_or(UnsupportedCondition)?,
            ))
        }
        (Fact::RoomAreaSquareInches, EntityRef::Room(room)) => plan
            .rooms
            .iter()
            .find(|schedule| schedule.room == *room)
            .map(|schedule| FactValue::Int(schedule.area_square_inches))
            .ok_or(MissingInput),
        (Fact::RoomCeilingHeight, EntityRef::Room(room)) => {
            let room = model
                .rooms
                .iter()
                .find(|candidate| candidate.id == *room)
                .ok_or(MissingInput)?;
            let level = model
                .levels
                .iter()
                .find(|candidate| candidate.id == room.level)
                .ok_or(MissingInput)?;
            (level.height > Length::ZERO)
                .then_some(FactValue::Length(level.height))
                .ok_or(MissingInput)
        }
        (Fact::BracedLineLength, EntityRef::BracedWallLine(line)) => Ok(FactValue::Length(
            braced_line_length(find_braced_line(model, line).ok_or(MissingInput)?)
                .ok_or(UnsupportedCondition)?,
        )),
        (Fact::BracedLineProvidedLength, EntityRef::BracedWallLine(line)) => {
            let line = find_braced_line(model, line).ok_or(MissingInput)?;
            Ok(FactValue::Length(braced_line_provided_length(model, line)))
        }
        (Fact::BracedLineRequiredLength, EntityRef::BracedWallLine(line)) => {
            let line = find_braced_line(model, line).ok_or(MissingInput)?;
            match braced_line_required_length(line, model, resolved) {
                BracingRequirement::Known(length) => Ok(FactValue::Length(length)),
                BracingRequirement::Unknown => Err(MissingInput),
                BracingRequirement::OutOfDomain => Err(UnsupportedCondition),
            }
        }
        _ => Err(WrongSubjectKind),
    }
}

fn bracing_diagnostic_entries(
    model: &BuildingModel,
    resolved: &ResolvedStandards,
) -> Vec<ComplianceEntry> {
    let mut entries = Vec::new();
    let (pack, citation) = bracing_context(model, resolved);

    for (wall, panel) in bracing_panel_refs(model) {
        if associated_line_for_panel(model, wall, panel).is_none() {
            entries.push(ComplianceEntry {
                rule: BRACING_UNASSOCIATED_PANEL.to_owned(),
                citation: citation.clone(),
                pack: pack.clone(),
                outcome: Outcome::Advisory,
                element: Some(panel.id.clone()),
                message: format!(
                    "Braced panel {} is not associated with a parallel braced wall line within 4 ft.",
                    panel.id.0
                ),
                chain: Vec::new(),
            });
        }
    }

    for line in &model.braced_wall_lines {
        if braced_line_required_length(line, model, resolved) == BracingRequirement::OutOfDomain {
            entries.push(ComplianceEntry {
                rule: BRACING_OUT_OF_DOMAIN.to_owned(),
                citation: citation.clone(),
                pack: pack.clone(),
                outcome: Outcome::NeedsReview,
                element: Some(line.id.clone()),
                message: format!(
                    "Braced wall line {} is outside the resolved bracing table domain.",
                    line.id.0
                ),
                chain: Vec::new(),
            });
        }
    }

    entries
}

fn bracing_context(model: &BuildingModel, resolved: &ResolvedStandards) -> (ElementId, String) {
    resolved
        .bracing
        .first()
        .map(|(pack, table)| (pack.clone(), table.citation.clone()))
        .or_else(|| {
            model
                .standards
                .first()
                .map(|pack| (pack.clone(), String::new()))
        })
        .unwrap_or_else(|| (ElementId::new("standards"), String::new()))
}

fn entry(
    rule: &ResolvedRule,
    outcome: Outcome,
    element: Option<ElementId>,
    message: String,
) -> ComplianceEntry {
    ComplianceEntry {
        rule: rule.rule.clone(),
        citation: rule.citation.clone(),
        pack: rule.pack.clone(),
        outcome,
        element,
        message,
        chain: rule.chain.clone(),
    }
}

fn outcome_message(title: &str, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Pass => format!("{title} passed."),
        Outcome::Violation => format!("{title} failed."),
        Outcome::Advisory => format!("{title} advisory failed."),
        Outcome::NeedsReview => format!("{title} needs review; one or more facts are unavailable."),
        Outcome::NotApplicable => format!("{title} is not applicable."),
        Outcome::Waived { reason } => format!("Waived: {reason}"),
    }
}

fn scoped_entities(model: &BuildingModel, scope: CheckScope) -> Vec<EntityRef> {
    match scope {
        CheckScope::Walls {
            exterior_only,
            tags,
        } => model
            .walls
            .iter()
            .filter(|wall| tags.iter().all(|tag| wall.tags.contains(tag)))
            .filter(|wall| {
                exterior_only.is_none_or(|expected| {
                    model
                        .system_for(wall)
                        .map(|system| (system.exposure() == WallExposure::Exterior) == expected)
                        .unwrap_or(false)
                })
            })
            .map(|wall| EntityRef::Wall(wall.id.clone()))
            .collect(),
        CheckScope::Openings { tags } => {
            if !tags.is_empty() {
                return Vec::new();
            }
            model
                .walls
                .iter()
                .flat_map(|wall| wall.openings.iter())
                .map(|opening| EntityRef::Opening(opening.id.clone()))
                .collect()
        }
        CheckScope::Rooms { tags } => model
            .rooms
            .iter()
            .filter(|room| tags.iter().all(|tag| room.tags.contains(tag)))
            .map(|room| EntityRef::Room(room.id.clone()))
            .collect(),
        CheckScope::BracedWallLines => model
            .braced_wall_lines
            .iter()
            .map(|line| EntityRef::BracedWallLine(line.id.clone()))
            .collect(),
    }
}

fn applicability(applies: Applicability, site: &SiteContext) -> Tri {
    match applies {
        Applicability::Always => Tri::True,
        Applicability::All(children) => {
            Tri::all(children.into_iter().map(|child| applicability(child, site)))
        }
        Applicability::Any(children) => {
            Tri::any(children.into_iter().map(|child| applicability(child, site)))
        }
        Applicability::Not(child) => applicability(*child, site).not(),
        Applicability::SeismicAtLeast(category) => site
            .seismic
            .map(|site_category| site_category >= category)
            .map(Tri::from)
            .unwrap_or(Tri::Unknown),
        Applicability::SeismicAtMost(category) => site
            .seismic
            .map(|site_category| site_category <= category)
            .map(Tri::from)
            .unwrap_or(Tri::Unknown),
        Applicability::WindSpeedAtLeast(speed) => site
            .wind_speed_mph
            .map(|site_speed| site_speed >= speed)
            .map(Tri::from)
            .unwrap_or(Tri::Unknown),
        Applicability::SnowLoadAtLeast(load) => site
            .ground_snow_load_psf
            .map(|site_load| site_load >= load)
            .map(Tri::from)
            .unwrap_or(Tri::Unknown),
        Applicability::SiteFlag { key } => match site.properties.get(&key) {
            Some(PropertyValue::Flag(value)) => Tri::from(*value),
            Some(_) | None => Tri::Unknown,
        },
    }
}

fn compare_fact_values(left: FactValue, op: CompareOp, right: FactValue) -> Tri {
    match (left, right) {
        (FactValue::Length(left), FactValue::Length(right)) => compare_ord(left, op, right),
        (FactValue::Int(left), FactValue::Int(right)) => compare_ord(left, op, right),
        (FactValue::Flag(left), FactValue::Flag(right)) => match op {
            CompareOp::Eq => Tri::from(left == right),
            CompareOp::Ne => Tri::from(left != right),
            CompareOp::Lt | CompareOp::Le | CompareOp::Ge | CompareOp::Gt => Tri::Unknown,
        },
        _ => Tri::Unknown,
    }
}

fn compare_ord<T: Ord>(left: T, op: CompareOp, right: T) -> Tri {
    Tri::from(match op {
        CompareOp::Lt => left < right,
        CompareOp::Le => left <= right,
        CompareOp::Eq => left == right,
        CompareOp::Ge => left >= right,
        CompareOp::Gt => left > right,
        CompareOp::Ne => left != right,
    })
}

fn find_wall<'a>(model: &'a BuildingModel, wall: &ElementId) -> Option<&'a framer_core::Wall> {
    model.walls.iter().find(|candidate| candidate.id == *wall)
}

fn find_opening<'a>(
    model: &'a BuildingModel,
    opening: &ElementId,
) -> Option<(&'a framer_core::Wall, &'a Opening)> {
    model.walls.iter().find_map(|wall| {
        wall.openings
            .iter()
            .find(|candidate| candidate.id == *opening)
            .map(|opening| (wall, opening))
    })
}

fn wall_stud_max_height(
    wall_id: ElementId,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
) -> Option<Length> {
    let wall = find_wall(model, &wall_id)?;
    let system = model.system_for(wall)?;
    let framing = system.framing_layer()?.framing.as_ref()?;
    let exterior = system.exposure() == WallExposure::Exterior;
    resolved
        .studs
        .iter()
        .flat_map(|(_, table)| table.rows.iter())
        .find(|row| row.profile == framing.member && row.spacing == framing.spacing)
        .map(|row| {
            if exterior {
                row.max_height_bearing
            } else {
                row.max_height_nonbearing
            }
        })
}

fn opening_members<'a>(plan: &'a ProjectFramePlan, opening: &ElementId) -> Vec<&'a FrameMember> {
    plan.wall_plans
        .iter()
        .flat_map(|wall| wall.members.iter())
        .filter(|member| member.source == *opening)
        .collect()
}

fn opening_headers<'a>(plan: &'a ProjectFramePlan, opening: &ElementId) -> Vec<&'a FrameMember> {
    opening_members(plan, opening)
        .into_iter()
        .filter(|member| member.kind == MemberKind::Header)
        .collect()
}

fn opening_header_max_span(
    opening_model: &Opening,
    opening: &ElementId,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> Option<Length> {
    let headers = opening_headers(plan, opening);
    let first = headers.first()?;
    let profile = first.profile;
    let plies = u8::try_from(headers.len()).ok()?;
    select_header_row(resolved, &model.site, opening_model.width, profile, plies)
        .map(|row| row.max_span)
}

fn select_header_row(
    resolved: &ResolvedStandards,
    site: &SiteContext,
    span: Length,
    profile: BoardProfile,
    plies: u8,
) -> Option<HeaderRow> {
    let rows = resolved
        .headers
        .iter()
        .flat_map(|(_, table)| table.rows.iter())
        .filter(|row| row.profile == profile && row.plies == plies)
        .collect::<Vec<_>>();
    let widest_width = rows.iter().map(|row| row.max_building_width).max()?;
    let highest_snow = rows.iter().map(|row| row.max_ground_snow_psf).max()?;

    rows.into_iter()
        .filter(|row| row.max_building_width == widest_width)
        .filter(|row| row.max_span >= span)
        .filter(|row| match site.ground_snow_load_psf {
            Some(load) => row.max_ground_snow_psf >= load,
            None => row.max_ground_snow_psf == highest_snow,
        })
        .min_by_key(|row| (row.max_span, row.jack_studs))
        .cloned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BracingRequirement {
    Known(Length),
    Unknown,
    OutOfDomain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DistanceKey {
    cross_sq: i128,
    line_len_sq: i128,
}

impl Ord for DistanceKey {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.cross_sq * other.line_len_sq).cmp(&(other.cross_sq * self.line_len_sq))
    }
}

impl PartialOrd for DistanceKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn bracing_panel_refs(model: &BuildingModel) -> Vec<(&Wall, &BracedPanel)> {
    model
        .walls
        .iter()
        .flat_map(|wall| wall.bracing.iter().map(move |panel| (wall, panel)))
        .collect()
}

fn find_braced_line<'a>(model: &'a BuildingModel, line: &ElementId) -> Option<&'a BracedWallLine> {
    model
        .braced_wall_lines
        .iter()
        .find(|candidate| candidate.id == *line)
}

fn braced_line_length(line: &BracedWallLine) -> Option<Length> {
    if line.start.y == line.end.y && line.start.x != line.end.x {
        Some((line.end.x - line.start.x).abs())
    } else if line.start.x == line.end.x && line.start.y != line.end.y {
        Some((line.end.y - line.start.y).abs())
    } else {
        None
    }
}

fn braced_line_provided_length(model: &BuildingModel, line: &BracedWallLine) -> Length {
    associated_panels_for_line(model, line)
        .into_iter()
        .fold(Length::ZERO, |sum, (_, panel)| sum + panel.length)
}

fn braced_line_required_length(
    line: &BracedWallLine,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
) -> BracingRequirement {
    let Some(line_length) = braced_line_length(line) else {
        return BracingRequirement::Unknown;
    };
    let methods = associated_panels_for_line(model, line)
        .into_iter()
        .map(|(_, panel)| panel.method)
        .collect::<BTreeSet<_>>();
    if methods.is_empty() {
        return BracingRequirement::Unknown;
    }

    let mut required = Length::ZERO;
    let mut unknown = false;
    let mut out_of_domain = false;
    for method in methods {
        match bracing_required_for_method(method, line_length, &model.site, resolved) {
            BracingRequirement::Known(length) => required = required.max(length),
            BracingRequirement::Unknown => unknown = true,
            BracingRequirement::OutOfDomain => out_of_domain = true,
        }
    }

    if out_of_domain {
        BracingRequirement::OutOfDomain
    } else if unknown {
        BracingRequirement::Unknown
    } else {
        BracingRequirement::Known(required)
    }
}

fn bracing_required_for_method(
    method: BracingMethod,
    line_length: Length,
    site: &SiteContext,
    resolved: &ResolvedStandards,
) -> BracingRequirement {
    let rows = resolved
        .bracing
        .iter()
        .flat_map(|(_, table)| table.rows.iter())
        .filter(|row| row.method == method)
        .filter(|row| row.line_length >= line_length)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return BracingRequirement::OutOfDomain;
    }
    if site.seismic.is_none() && rows.iter().any(|row| row.max_seismic.is_some()) {
        return BracingRequirement::Unknown;
    }
    if site.wind_speed_mph.is_none() && rows.iter().any(|row| row.max_wind_speed_mph.is_some()) {
        return BracingRequirement::Unknown;
    }

    rows.into_iter()
        .filter(|row| bracing_row_matches_site(row, site))
        .min_by_key(|row| {
            (
                row.line_length,
                row.required_length,
                row.max_seismic,
                row.max_wind_speed_mph,
            )
        })
        .map(|row| BracingRequirement::Known(row.required_length))
        .unwrap_or(BracingRequirement::OutOfDomain)
}

fn bracing_row_matches_site(row: &BracingRow, site: &SiteContext) -> bool {
    let seismic_matches = row
        .max_seismic
        .is_none_or(|max| site.seismic.is_some_and(|site| max >= site));
    let wind_matches = row
        .max_wind_speed_mph
        .is_none_or(|max| site.wind_speed_mph.is_some_and(|site| max >= site));
    seismic_matches && wind_matches
}

fn associated_panels_for_line<'a>(
    model: &'a BuildingModel,
    line: &BracedWallLine,
) -> Vec<(&'a Wall, &'a BracedPanel)> {
    bracing_panel_refs(model)
        .into_iter()
        .filter(|(wall, panel)| {
            associated_line_for_panel(model, wall, panel)
                .is_some_and(|candidate| candidate.id == line.id)
        })
        .collect()
}

fn associated_line_for_panel<'a>(
    model: &'a BuildingModel,
    wall: &Wall,
    panel: &BracedPanel,
) -> Option<&'a BracedWallLine> {
    let wall_direction = direction(wall.start, wall.end);
    if wall_direction == (0, 0) {
        return None;
    }
    let midpoint = wall.point_at_local_x(panel.offset + panel.length / 2);

    model
        .braced_wall_lines
        .iter()
        .filter(|line| line.level == wall.level)
        .filter(|line| {
            let line_direction = direction(line.start, line.end);
            line_direction != (0, 0) && cross(wall_direction, line_direction) == 0
        })
        .filter_map(|line| {
            let distance = distance_to_line(midpoint, line)?;
            (distance.within(BRACING_ASSOCIATION_TOLERANCE)).then_some((distance, &line.id, line))
        })
        .min_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)))
        .map(|(_, _, line)| line)
}

impl DistanceKey {
    fn within(self, tolerance: Length) -> bool {
        let tolerance = i128::from(tolerance.ticks());
        self.cross_sq <= tolerance * tolerance * self.line_len_sq
    }
}

fn distance_to_line(point: framer_core::Point2, line: &BracedWallLine) -> Option<DistanceKey> {
    let line_direction = direction(line.start, line.end);
    let line_len_sq = dot(line_direction, line_direction);
    if line_len_sq == 0 {
        return None;
    }
    let offset = direction(line.start, point);
    let cross = cross(line_direction, offset);
    Some(DistanceKey {
        cross_sq: cross * cross,
        line_len_sq,
    })
}

fn direction(start: framer_core::Point2, end: framer_core::Point2) -> (i128, i128) {
    (
        i128::from((end.x - start.x).ticks()),
        i128::from((end.y - start.y).ticks()),
    )
}

fn cross(left: (i128, i128), right: (i128, i128)) -> i128 {
    left.0 * right.1 - left.1 * right.0
}

fn dot(left: (i128, i128), right: (i128, i128)) -> i128 {
    left.0 * right.0 + left.1 * right.1
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use framer_core::{
        BracedPanel, BracedWallLine, BracingMethod, ComplianceCheck, FramingDefaults, Point2, Room,
        RoomUsage, SeismicDesignCategory, StandardsPack, Wall,
    };
    use framer_solver::generate_project_plan;

    use super::*;

    #[test]
    fn tri_uses_kleene_truth_tables() {
        assert_eq!(Tri::False.not(), Tri::True);
        assert_eq!(Tri::Unknown.not(), Tri::Unknown);
        assert_eq!(Tri::True.not(), Tri::False);
        assert_eq!(Tri::all([]), Tri::True);
        assert_eq!(Tri::all([Tri::True, Tri::Unknown]), Tri::Unknown);
        assert_eq!(Tri::all([Tri::True, Tri::False]), Tri::False);
        assert_eq!(Tri::any([]), Tri::False);
        assert_eq!(Tri::any([Tri::False, Tri::Unknown]), Tri::Unknown);
        assert_eq!(Tri::any([Tri::False, Tri::True]), Tri::True);
    }

    #[test]
    fn applicability_unknown_site_values_need_review() {
        let site = SiteContext::default();

        assert_eq!(
            applicability(
                Applicability::SeismicAtLeast(SeismicDesignCategory::D0),
                &site
            ),
            Tri::Unknown
        );
        assert_eq!(
            applicability(
                Applicability::SiteFlag {
                    key: "sprinklers".to_owned()
                },
                &site
            ),
            Tri::Unknown
        );
    }

    #[test]
    fn fact_snapshot_observes_every_current_fact_and_matches_fact_value() {
        let wall_model = BuildingModel::demo_wall();
        let wall_resolved = wall_model.resolved_standards();
        let wall_plan = generate_project_plan(&wall_model).unwrap();
        let wall_snapshot = FactSnapshot::new(&wall_model, &wall_resolved, &wall_plan);
        let wall = FactSubject::Wall(wall_model.walls[0].id.clone());
        for fact in [
            Fact::WallLength,
            Fact::WallHeight,
            Fact::WallIsExterior,
            Fact::WallStudSpacing,
            Fact::WallSystemRValueMilli,
            Fact::WallStudMaxHeight,
        ] {
            assert_known_with_wrapper(
                fact,
                &wall,
                &wall_snapshot,
                &wall_model,
                &wall_resolved,
                &wall_plan,
            );
        }

        let opening = FactSubject::Opening(wall_model.walls[0].openings[0].id.clone());
        for fact in [
            Fact::OpeningRoughWidth,
            Fact::OpeningRoughHeight,
            Fact::OpeningHeaderDepth,
            Fact::OpeningJackStuds,
            Fact::OpeningHeaderMaxSpan,
        ] {
            assert_known_with_wrapper(
                fact,
                &opening,
                &wall_snapshot,
                &wall_model,
                &wall_resolved,
                &wall_plan,
            );
        }

        let mut room_model = BuildingModel::demo_two_bedroom();
        room_model.levels[0].height = Length::from_feet(8.0);
        let room_resolved = room_model.resolved_standards();
        let room_plan = generate_project_plan(&room_model).unwrap();
        let room_snapshot = FactSnapshot::new(&room_model, &room_resolved, &room_plan);
        let room = FactSubject::Room(room_model.rooms[0].id.clone());
        for fact in [Fact::RoomAreaSquareInches, Fact::RoomCeilingHeight] {
            assert_known_with_wrapper(
                fact,
                &room,
                &room_snapshot,
                &room_model,
                &room_resolved,
                &room_plan,
            );
        }

        let mut bracing_model = braced_line_model(Length::from_feet(20.0));
        bracing_model.site.seismic = Some(SeismicDesignCategory::C);
        bracing_model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let bracing_resolved = bracing_model.resolved_standards();
        let bracing_plan = generate_project_plan(&bracing_model).unwrap();
        let bracing_snapshot = FactSnapshot::new(&bracing_model, &bracing_resolved, &bracing_plan);
        let line = FactSubject::BracedWallLine(ElementId::new("bwl"));
        for fact in [
            Fact::BracedLineLength,
            Fact::BracedLineRequiredLength,
            Fact::BracedLineProvidedLength,
        ] {
            assert_known_with_wrapper(
                fact,
                &line,
                &bracing_snapshot,
                &bracing_model,
                &bracing_resolved,
                &bracing_plan,
            );
        }
    }

    #[test]
    fn fact_snapshot_fails_closed_with_structured_unknowns() {
        let model = BuildingModel::demo_wall();
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let snapshot = FactSnapshot::new(&model, &resolved, &plan);
        let opening = FactSubject::Opening(model.walls[0].openings[0].id.clone());

        assert_unknown_kind(
            snapshot.observe(Fact::WallLength, &opening),
            FactUnknownKind::WrongSubjectKind,
        );
        let missing_opening = FactSubject::Opening(ElementId::new("missing-opening"));
        assert_unknown_kind(
            snapshot.observe(Fact::OpeningJackStuds, &missing_opening),
            FactUnknownKind::UnresolvedSubject,
        );
        assert_eq!(
            fact_value(
                Fact::OpeningJackStuds,
                &missing_opening,
                &model,
                &resolved,
                &plan,
            ),
            None
        );

        let mut empty_member_plan = plan.clone();
        for wall_plan in &mut empty_member_plan.wall_plans {
            wall_plan.members.clear();
        }
        let empty_member_snapshot = FactSnapshot::new(&model, &resolved, &empty_member_plan);
        assert_eq!(
            empty_member_snapshot.observe(Fact::OpeningJackStuds, &opening),
            FactObservation::Known(FactValue::Int(0)),
            "the shared snapshot preserves the frozen v1 empty-member count",
        );
        assert_eq!(
            fact_value(
                Fact::OpeningJackStuds,
                &opening,
                &model,
                &resolved,
                &empty_member_plan,
            ),
            Some(FactValue::Int(0)),
        );

        let mut unsupported_model = braced_line_model(Length::from_feet(50.0));
        unsupported_model.site.seismic = Some(SeismicDesignCategory::C);
        unsupported_model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let unsupported_resolved = unsupported_model.resolved_standards();
        let unsupported_plan = generate_project_plan(&unsupported_model).unwrap();
        let unsupported_snapshot =
            FactSnapshot::new(&unsupported_model, &unsupported_resolved, &unsupported_plan);
        assert_unknown_kind(
            unsupported_snapshot.observe(
                Fact::BracedLineRequiredLength,
                &FactSubject::BracedWallLine(ElementId::new("bwl")),
            ),
            FactUnknownKind::UnsupportedCondition,
        );
    }

    #[test]
    fn subjects_and_predicate_facts_are_canonical() {
        let defaults = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls = vec![
            Wall::new("wall-z", "Wall Z", Length::from_feet(8.0), &defaults),
            Wall::new("wall-a", "Wall A", Length::from_feet(8.0), &defaults),
        ];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        model.walls.push(model.walls[1].clone());
        let snapshot = FactSnapshot::new(&model, &resolved, &plan);

        let subjects = snapshot.subjects_for(&CheckScope::Walls {
            exterior_only: None,
            tags: Vec::new(),
        });
        assert_eq!(
            subjects,
            vec![
                FactSubject::Wall(ElementId::new("wall-a")),
                FactSubject::Wall(ElementId::new("wall-z")),
            ]
        );

        let observation = snapshot.evaluate_predicate(
            &Predicate::All(vec![
                Predicate::Compare {
                    fact: Fact::WallHeight,
                    op: CompareOp::Le,
                    value: FactOperand::Fact(Fact::WallStudMaxHeight),
                },
                Predicate::Compare {
                    fact: Fact::WallLength,
                    op: CompareOp::Gt,
                    value: FactOperand::LengthLiteral(Length::ZERO),
                },
                Predicate::Compare {
                    fact: Fact::WallHeight,
                    op: CompareOp::Eq,
                    value: FactOperand::Fact(Fact::WallHeight),
                },
            ]),
            &FactSubject::Wall(ElementId::new("wall-a")),
        );
        assert_eq!(observation.result, Tri::True);
        assert_eq!(
            observation
                .observed_facts
                .iter()
                .map(|observed| observed.fact)
                .collect::<Vec<_>>(),
            vec![Fact::WallLength, Fact::WallHeight, Fact::WallStudMaxHeight,]
        );
    }

    #[test]
    fn wall_facts_report_known_values_and_unknown_table_misses() {
        let model = one_wall_model(Length::from_feet(8.0));
        let plan = generate_project_plan(&model).unwrap();
        let mut resolved = model.resolved_standards();
        let wall = EntityRef::Wall(ElementId::new("wall"));

        assert_eq!(
            fact_value(Fact::WallLength, &wall, &model, &resolved, &plan),
            Some(FactValue::Length(Length::from_feet(8.0)))
        );
        assert_eq!(
            fact_value(Fact::WallIsExterior, &wall, &model, &resolved, &plan),
            Some(FactValue::Flag(true))
        );

        resolved.studs.clear();
        assert_eq!(
            fact_value(Fact::WallStudMaxHeight, &wall, &model, &resolved, &plan),
            None
        );
    }

    #[test]
    fn detailed_diagnostics_distinguish_unsupported_facts_from_missing_facts() {
        let mut model = one_wall_model(Length::from_feet(8.0));
        let mut pack = StandardsPack::irc_2021_starter();
        pack.checks = vec![ComplianceCheck {
            rule: "test.wall.unsupported-stud-height".to_owned(),
            citation: "Test".to_owned(),
            title: "Unsupported stud height".to_owned(),
            severity: CheckSeverity::Required,
            applies: Applicability::Always,
            scope: CheckScope::Walls {
                exterior_only: None,
                tags: Vec::new(),
            },
            requirement: Predicate::Compare {
                fact: Fact::WallStudMaxHeight,
                op: CompareOp::Ge,
                value: FactOperand::LengthLiteral(Length::from_feet(8.0)),
            },
        }];
        model.standards = vec![pack.id.clone()];
        model.standards_packs = vec![pack];

        let plan = generate_project_plan(&model).unwrap();
        let mut resolved = model.resolved_standards();
        resolved.studs.clear();
        let evaluation = evaluate_detailed(&model, &resolved, &plan);

        assert!(has_outcome(
            &evaluation.report,
            "test.wall.unsupported-stud-height",
            &Outcome::NeedsReview,
        ));
        assert!(detail_for(&evaluation, "test.wall.unsupported-stud-height").is_unsupported());
        assert_eq!(
            evaluation.diagnostics()[0].severity,
            DiagnosticSeverity::Unsupported,
        );
        assert_eq!(
            diagnostics(&evaluation.report)[0].severity,
            DiagnosticSeverity::NeedsReview,
            "the frozen report cannot encode the detailed unsupported reason",
        );
    }

    #[test]
    fn bracing_association_uses_parallel_tolerance_and_tie_break() {
        let mut model = braced_line_model(Length::from_feet(20.0));
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        model.braced_wall_lines = vec![
            braced_line("bwl-b", Length::from_feet(20.0), Length::from_feet(2.0)),
            braced_line(
                "bwl-far",
                Length::from_feet(20.0),
                Length::from_whole_inches(49),
            ),
            BracedWallLine {
                id: ElementId::new("bwl-cross"),
                name: "Cross line".to_owned(),
                level: ElementId::new("level-1"),
                start: Point2::new(Length::from_feet(4.0), Length::ZERO),
                end: Point2::new(Length::from_feet(4.0), Length::from_feet(20.0)),
            },
            braced_line("bwl-a", Length::from_feet(20.0), Length::from_feet(-2.0)),
        ];

        let associated =
            associated_line_for_panel(&model, &model.walls[0], &model.walls[0].bracing[0])
                .expect("associated braced wall line");
        assert_eq!(associated.id, ElementId::new("bwl-a"));

        model.braced_wall_lines = vec![braced_line(
            "bwl-too-far",
            Length::from_feet(20.0),
            Length::from_whole_inches(49),
        )];
        assert!(
            associated_line_for_panel(&model, &model.walls[0], &model.walls[0].bracing[0])
                .is_none()
        );
    }

    #[test]
    fn braced_line_facts_use_associated_panels_and_sdc_bands() {
        let mut model = braced_line_model(Length::from_feet(20.0));
        model.site.seismic = Some(SeismicDesignCategory::C);
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let plan = generate_project_plan(&model).unwrap();
        let resolved = model.resolved_standards();
        let line = EntityRef::BracedWallLine(ElementId::new("bwl"));

        assert_eq!(
            fact_value(Fact::BracedLineLength, &line, &model, &resolved, &plan),
            Some(FactValue::Length(Length::from_feet(20.0)))
        );
        assert_eq!(
            fact_value(
                Fact::BracedLineProvidedLength,
                &line,
                &model,
                &resolved,
                &plan
            ),
            Some(FactValue::Length(Length::from_feet(4.0)))
        );
        assert_eq!(
            fact_value(
                Fact::BracedLineRequiredLength,
                &line,
                &model,
                &resolved,
                &plan
            ),
            Some(FactValue::Length(Length::from_feet(4.0)))
        );

        model.site.seismic = Some(SeismicDesignCategory::D2);
        let resolved = model.resolved_standards();
        assert_eq!(
            fact_value(
                Fact::BracedLineRequiredLength,
                &line,
                &model,
                &resolved,
                &plan
            ),
            Some(FactValue::Length(Length::from_feet(6.0)))
        );
    }

    #[test]
    fn braced_line_required_length_uses_multi_method_max() {
        let mut model = braced_line_model(Length::from_feet(20.0));
        model.site.seismic = Some(SeismicDesignCategory::D2);
        model.walls[0].bracing = vec![
            braced_panel(
                "panel-wsp",
                Length::from_feet(2.0),
                Length::from_feet(4.0),
                BracingMethod::Wsp,
            ),
            braced_panel(
                "panel-gb",
                Length::from_feet(8.0),
                Length::from_feet(4.0),
                BracingMethod::Gb,
            ),
        ];
        let plan = generate_project_plan(&model).unwrap();
        let resolved = model.resolved_standards();
        let line = EntityRef::BracedWallLine(ElementId::new("bwl"));

        assert_eq!(
            fact_value(
                Fact::BracedLineRequiredLength,
                &line,
                &model,
                &resolved,
                &plan
            ),
            Some(FactValue::Length(Length::from_feet(8.0)))
        );
    }

    #[test]
    fn unknown_sdc_turns_bracing_checks_into_needs_review() {
        let mut model = braced_line_model(Length::from_feet(20.0));
        model.site.seismic = None;
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let report = evaluate(&model, &resolved, &plan);

        assert!(has_outcome(
            &report,
            "irc2021.r602.10.braced-length",
            &Outcome::NeedsReview
        ));
        assert!(diagnostics(&report).iter().any(|diagnostic| {
            diagnostic.code == "irc2021.r602.10.braced-length"
                && diagnostic.severity == DiagnosticSeverity::NeedsReview
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("bwl")
        }));
    }

    #[test]
    fn bracing_out_of_domain_lowers_to_unsupported_diagnostic() {
        let mut model = braced_line_model(Length::from_feet(50.0));
        model.site.seismic = Some(SeismicDesignCategory::C);
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let evaluation = evaluate_detailed(&model, &resolved, &plan);
        let report = &evaluation.report;

        assert!(diagnostics(report).iter().any(|diagnostic| {
            diagnostic.code == BRACING_OUT_OF_DOMAIN
                && diagnostic.severity == DiagnosticSeverity::Unsupported
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("bwl")
        }));
        assert!(evaluation.details.iter().any(|detail| {
            detail.synthetic_kind == Some(SyntheticEntryKind::BracingOutOfDomain)
                && detail.subject == Some(FactSubject::BracedWallLine(ElementId::new("bwl")))
        }));
    }

    #[test]
    fn unassociated_bracing_panels_emit_advisory_diagnostics() {
        let mut model = one_wall_model(Length::from_feet(20.0));
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let evaluation = evaluate_detailed(&model, &resolved, &plan);
        let report = &evaluation.report;

        assert_eq!(&evaluate(&model, &resolved, &plan), report);
        assert!(diagnostics(report).iter().any(|diagnostic| {
            diagnostic.code == BRACING_UNASSOCIATED_PANEL
                && diagnostic.severity == DiagnosticSeverity::Warning
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("panel")
        }));
        assert!(evaluation.details.iter().any(|detail| {
            detail.synthetic_kind == Some(SyntheticEntryKind::UnassociatedBracingPanel)
        }));
    }

    #[test]
    fn evaluate_maps_required_advisory_unknown_and_waived_outcomes() {
        let mut model = one_wall_model(Length::from_feet(8.0));
        model.rooms.push(Room::new(
            "room",
            "Room",
            RoomUsage::Living,
            "level-1",
            Point2::new(Length::from_feet(1.0), Length::from_feet(1.0)),
        ));
        model.rooms[0].tags.push("habitable".to_owned());
        let mut pack = StandardsPack::irc_2021_starter();
        pack.checks = vec![
            wall_check(
                "test.wall.pass",
                CheckSeverity::Required,
                CompareOp::Le,
                FactOperand::LengthLiteral(Length::from_feet(12.0)),
            ),
            wall_check(
                "test.wall.violation",
                CheckSeverity::Required,
                CompareOp::Gt,
                FactOperand::LengthLiteral(Length::from_feet(20.0)),
            ),
            wall_check(
                "test.wall.advisory",
                CheckSeverity::Advisory,
                CompareOp::Gt,
                FactOperand::LengthLiteral(Length::from_feet(20.0)),
            ),
            ComplianceCheck {
                rule: "test.room.unknown".to_owned(),
                citation: "Test".to_owned(),
                title: "Room unknown".to_owned(),
                severity: CheckSeverity::Required,
                applies: Applicability::Always,
                scope: CheckScope::Rooms {
                    tags: vec!["habitable".to_owned()],
                },
                requirement: Predicate::Compare {
                    fact: Fact::RoomCeilingHeight,
                    op: CompareOp::Ge,
                    value: FactOperand::LengthLiteral(Length::from_feet(7.0)),
                },
            },
            wall_check(
                "test.wall.waived",
                CheckSeverity::Required,
                CompareOp::Gt,
                FactOperand::LengthLiteral(Length::from_feet(20.0)),
            ),
        ];
        pack.overlays.push(framer_core::RuleOverlay::Waive {
            target: "test.wall.waived".to_owned(),
            reason: "accepted by AHJ".to_owned(),
        });
        model.standards = vec![pack.id.clone()];
        model.standards_packs = vec![pack];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let report = evaluate(&model, &resolved, &plan);

        assert!(has_outcome(&report, "test.wall.pass", &Outcome::Pass));
        assert!(has_outcome(
            &report,
            "test.wall.violation",
            &Outcome::Violation
        ));
        assert!(has_outcome(
            &report,
            "test.wall.advisory",
            &Outcome::Advisory
        ));
        assert!(has_outcome(
            &report,
            "test.room.unknown",
            &Outcome::NeedsReview
        ));
        assert!(report.entries.iter().any(|entry| matches!(
            &entry.outcome,
            Outcome::Waived { reason } if reason == "accepted by AHJ"
        )));

        let diagnostics = diagnostics(&report);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Violation
                && diagnostic.rule.as_ref().map(|rule| rule.rule.as_str())
                    == Some("test.wall.violation")
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::NeedsReview
                && diagnostic.code == "test.room.unknown"
        }));
    }

    #[test]
    fn detailed_evaluation_preserves_legacy_outputs_and_scoped_waiver_evidence() {
        let defaults = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.site.wind_speed_mph = Some(100);
        model.walls = vec![
            Wall::new("wall-b", "Wall B", Length::from_feet(8.0), &defaults),
            Wall::new("wall-a", "Wall A", Length::from_feet(8.0), &defaults),
        ];
        let mut room = Room::new(
            "room-unknown",
            "Room unknown",
            RoomUsage::Living,
            "level-1",
            Point2::new(Length::from_feet(1.0), Length::from_feet(1.0)),
        );
        room.tags.push("unknown".to_owned());
        model.rooms.push(room);

        let mut not_applicable = wall_check(
            "test.wall.not-applicable",
            CheckSeverity::Required,
            CompareOp::Gt,
            FactOperand::LengthLiteral(Length::ZERO),
        );
        not_applicable.applies = Applicability::WindSpeedAtLeast(200);
        let mut applicability_unknown = wall_check(
            "test.wall.applicability-unknown",
            CheckSeverity::Required,
            CompareOp::Gt,
            FactOperand::LengthLiteral(Length::ZERO),
        );
        applicability_unknown.applies = Applicability::SiteFlag {
            key: "missing-flag".to_owned(),
        };
        let room_unknown = ComplianceCheck {
            rule: "test.room.unknown-detail".to_owned(),
            citation: "Test".to_owned(),
            title: "Room unknown detail".to_owned(),
            severity: CheckSeverity::Required,
            applies: Applicability::Always,
            scope: CheckScope::Rooms {
                tags: vec!["unknown".to_owned()],
            },
            requirement: Predicate::Compare {
                fact: Fact::RoomCeilingHeight,
                op: CompareOp::Ge,
                value: FactOperand::LengthLiteral(Length::from_feet(7.0)),
            },
        };

        let mut base = StandardsPack::irc_2021_starter();
        base.checks = vec![
            wall_check(
                "test.wall.pass-detail",
                CheckSeverity::Required,
                CompareOp::Le,
                FactOperand::LengthLiteral(Length::from_feet(12.0)),
            ),
            wall_check(
                "test.wall.violation-detail",
                CheckSeverity::Required,
                CompareOp::Gt,
                FactOperand::LengthLiteral(Length::from_feet(20.0)),
            ),
            wall_check(
                "test.wall.advisory-detail",
                CheckSeverity::Advisory,
                CompareOp::Gt,
                FactOperand::LengthLiteral(Length::from_feet(20.0)),
            ),
            room_unknown,
            not_applicable,
            applicability_unknown,
            wall_check(
                "test.wall.waived-detail",
                CheckSeverity::Required,
                CompareOp::Gt,
                FactOperand::LengthLiteral(Length::from_feet(20.0)),
            ),
        ];
        let base_id = base.id.clone();

        let mut overlay = StandardsPack::irc_2021_starter();
        overlay.id = ElementId::new("std-overlay");
        overlay.name = "Test overlay".to_owned();
        overlay.tables.studs.clear();
        overlay.tables.headers.clear();
        overlay.tables.fastening.clear();
        overlay.tables.bracing.clear();
        overlay.checks.clear();
        overlay.overlays = vec![
            framer_core::RuleOverlay::Severity {
                target: "test.wall.waived-detail".to_owned(),
                severity: CheckSeverity::Advisory,
            },
            framer_core::RuleOverlay::Waive {
                target: "test.wall.waived-detail".to_owned(),
                reason: "approved by test AHJ".to_owned(),
            },
        ];
        let overlay_id = overlay.id.clone();
        model.standards = vec![base_id.clone(), overlay_id.clone()];
        model.standards_packs = vec![base, overlay];

        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let evaluation = evaluate_detailed(&model, &resolved, &plan);
        assert_eq!(
            evaluation,
            evaluate_detailed(&model, &resolved, &plan),
            "detailed evaluation must be canonical"
        );
        const FROZEN_REPORT_CSV: &str = concat!(
            "rule,citation,pack,outcome,element,message,chain\n",
            "test.room.unknown-detail,Test,std-irc-2021,NeedsReview,room-unknown,Room unknown detail needs review; one or more facts are unavailable.,std-irc-2021:Introduced\n",
            "test.wall.advisory-detail,Test,std-irc-2021,Advisory,wall-a,test.wall.advisory-detail advisory failed.,std-irc-2021:Introduced\n",
            "test.wall.advisory-detail,Test,std-irc-2021,Advisory,wall-b,test.wall.advisory-detail advisory failed.,std-irc-2021:Introduced\n",
            "test.wall.applicability-unknown,Test,std-irc-2021,NeedsReview,,test.wall.applicability-unknown applicability needs review.,std-irc-2021:Introduced\n",
            "test.wall.not-applicable,Test,std-irc-2021,NotApplicable,,test.wall.not-applicable is not applicable for this site context.,std-irc-2021:Introduced\n",
            "test.wall.pass-detail,Test,std-irc-2021,Pass,wall-a,test.wall.pass-detail passed.,std-irc-2021:Introduced\n",
            "test.wall.pass-detail,Test,std-irc-2021,Pass,wall-b,test.wall.pass-detail passed.,std-irc-2021:Introduced\n",
            "test.wall.violation-detail,Test,std-irc-2021,Violation,wall-a,test.wall.violation-detail failed.,std-irc-2021:Introduced\n",
            "test.wall.violation-detail,Test,std-irc-2021,Violation,wall-b,test.wall.violation-detail failed.,std-irc-2021:Introduced\n",
            "test.wall.waived-detail,Test,std-irc-2021,Waived,,Waived: approved by test AHJ,std-irc-2021:Introduced;std-overlay:Reseverity;std-overlay:Waived\n",
        );
        assert_eq!(evaluation.report.to_csv(), FROZEN_REPORT_CSV);

        let wrapper_report = evaluate(&model, &resolved, &plan);
        assert_eq!(wrapper_report, evaluation.report);
        assert_eq!(wrapper_report.to_csv(), evaluation.report.to_csv());
        assert_eq!(
            diagnostics(&wrapper_report),
            diagnostics(&evaluation.report)
        );
        assert_eq!(
            evaluation
                .diagnostics()
                .iter()
                .map(|diagnostic| {
                    (
                        diagnostic.code.as_str(),
                        diagnostic.source.as_ref().map(|source| source.0.as_str()),
                        diagnostic.severity,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    "test.room.unknown-detail",
                    Some("room-unknown"),
                    DiagnosticSeverity::NeedsReview,
                ),
                (
                    "test.wall.advisory-detail",
                    Some("wall-a"),
                    DiagnosticSeverity::Warning,
                ),
                (
                    "test.wall.advisory-detail",
                    Some("wall-b"),
                    DiagnosticSeverity::Warning,
                ),
                (
                    "test.wall.applicability-unknown",
                    None,
                    DiagnosticSeverity::NeedsReview,
                ),
                (
                    "test.wall.violation-detail",
                    Some("wall-a"),
                    DiagnosticSeverity::Violation,
                ),
                (
                    "test.wall.violation-detail",
                    Some("wall-b"),
                    DiagnosticSeverity::Violation,
                ),
            ]
        );

        assert!(has_outcome(
            &evaluation.report,
            "test.wall.pass-detail",
            &Outcome::Pass
        ));
        assert!(has_outcome(
            &evaluation.report,
            "test.wall.violation-detail",
            &Outcome::Violation
        ));
        assert!(has_outcome(
            &evaluation.report,
            "test.wall.advisory-detail",
            &Outcome::Advisory
        ));
        assert!(has_outcome(
            &evaluation.report,
            "test.room.unknown-detail",
            &Outcome::NeedsReview
        ));
        assert!(has_outcome(
            &evaluation.report,
            "test.wall.not-applicable",
            &Outcome::NotApplicable
        ));

        let required_detail = detail_for(&evaluation, "test.wall.violation-detail");
        assert_eq!(required_detail.severity, Some(CheckSeverity::Required));
        assert_eq!(
            required_detail.predicate.as_ref().map(|value| value.result),
            Some(Tri::False)
        );
        let advisory_detail = detail_for(&evaluation, "test.wall.advisory-detail");
        assert_eq!(advisory_detail.severity, Some(CheckSeverity::Advisory));
        assert_eq!(
            advisory_detail.predicate.as_ref().map(|value| value.result),
            Some(Tri::False)
        );
        let unknown_detail = detail_for(&evaluation, "test.room.unknown-detail");
        assert_eq!(unknown_detail.applicability, Some(Tri::True));
        let unknown_predicate = unknown_detail.predicate.as_ref().unwrap();
        assert_eq!(unknown_predicate.result, Tri::Unknown);
        assert!(matches!(
            &unknown_predicate.observed_facts[0].observation,
            FactObservation::Unknown(FactUnknown {
                kind: FactUnknownKind::MissingInput,
                ..
            })
        ));
        let not_applicable_detail = detail_for(&evaluation, "test.wall.not-applicable");
        assert_eq!(not_applicable_detail.applicability, Some(Tri::False));
        assert!(not_applicable_detail.predicate.is_none());
        let applicability_unknown_detail =
            detail_for(&evaluation, "test.wall.applicability-unknown");
        assert_eq!(
            applicability_unknown_detail.applicability,
            Some(Tri::Unknown)
        );
        assert!(applicability_unknown_detail.predicate.is_none());

        let waived_report_entries = evaluation
            .report
            .entries
            .iter()
            .filter(|entry| entry.rule == "test.wall.waived-detail")
            .collect::<Vec<_>>();
        assert_eq!(waived_report_entries.len(), 1);
        assert_eq!(waived_report_entries[0].element, None);
        assert!(matches!(
            &waived_report_entries[0].outcome,
            Outcome::Waived { reason } if reason == "approved by test AHJ"
        ));

        let waived_details = evaluation
            .details
            .iter()
            .filter(|detail| detail.check_id.as_deref() == Some("test.wall.waived-detail"))
            .collect::<Vec<_>>();
        assert_eq!(waived_details.len(), 2);
        assert_eq!(
            waived_details
                .iter()
                .map(|detail| detail.subject.clone().unwrap())
                .collect::<Vec<_>>(),
            vec![
                FactSubject::Wall(ElementId::new("wall-a")),
                FactSubject::Wall(ElementId::new("wall-b")),
            ]
        );
        assert!(
            waived_details
                .iter()
                .all(|detail| detail.report_entry_index == waived_details[0].report_entry_index)
        );
        for detail in waived_details {
            assert_eq!(detail.definition_pack.as_ref(), Some(&base_id));
            assert_eq!(detail.severity, Some(CheckSeverity::Advisory));
            assert_eq!(
                detail.scope_subjects,
                vec![
                    FactSubject::Wall(ElementId::new("wall-a")),
                    FactSubject::Wall(ElementId::new("wall-b")),
                ]
            );
            let waiver = detail.effective_waiver.as_ref().unwrap();
            assert_eq!(waiver.reason, "approved by test AHJ");
            assert_eq!(waiver.overlay_pack, overlay_id);
            assert_eq!(
                waiver.chain.last(),
                Some(&(overlay_id.clone(), ResolutionAction::Waived))
            );
        }
    }

    #[test]
    fn report_csv_is_deterministic_and_escaped() {
        let model = one_wall_model(Length::from_feet(8.0));
        let mut pack = StandardsPack::irc_2021_starter();
        pack.checks = vec![wall_check(
            "test.wall.pass",
            CheckSeverity::Required,
            CompareOp::Le,
            FactOperand::LengthLiteral(Length::from_feet(12.0)),
        )];
        let mut model = model;
        model.standards = vec![pack.id.clone()];
        model.standards_packs = vec![pack];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();

        let first = evaluate(&model, &resolved, &plan).to_csv();
        let second = evaluate(&model, &resolved, &plan).to_csv();

        assert_eq!(first, second);
        assert!(first.starts_with("rule,citation,pack,outcome,element,message,chain\n"));
        assert!(first.contains("test.wall.pass,Test,std-irc-2021,Pass,wall"));
    }

    fn assert_known_with_wrapper(
        fact: Fact,
        subject: &FactSubject,
        snapshot: &FactSnapshot<'_>,
        model: &BuildingModel,
        resolved: &ResolvedStandards,
        plan: &ProjectFramePlan,
    ) {
        let observation = snapshot.observe(fact, subject);
        let FactObservation::Known(value) = observation else {
            panic!("expected {fact:?} for {subject:?} to be known, got {observation:?}");
        };
        assert_eq!(
            fact_value(fact, subject, model, resolved, plan),
            Some(value),
            "compatibility wrapper diverged for {fact:?}"
        );
    }

    fn assert_unknown_kind(observation: FactObservation, expected: FactUnknownKind) {
        let FactObservation::Unknown(unknown) = observation else {
            panic!("expected {expected:?}, got {observation:?}");
        };
        assert_eq!(unknown.kind, expected);
    }

    fn detail_for<'a>(
        evaluation: &'a StandardsEvaluation,
        check_id: &str,
    ) -> &'a StandardsEvaluationDetail {
        evaluation
            .details
            .iter()
            .find(|detail| detail.check_id.as_deref() == Some(check_id))
            .unwrap_or_else(|| panic!("missing detail for {check_id}"))
    }

    fn one_wall_model(length: Length) -> BuildingModel {
        let defaults = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls = vec![Wall::new("wall", "Wall", length, &defaults)];
        model
    }

    fn braced_line_model(length: Length) -> BuildingModel {
        let mut model = one_wall_model(length);
        model.braced_wall_lines = vec![braced_line("bwl", length, Length::ZERO)];
        model
    }

    fn braced_line(id: &str, length: Length, y: Length) -> BracedWallLine {
        BracedWallLine {
            id: ElementId::new(id),
            name: id.to_owned(),
            level: ElementId::new("level-1"),
            start: Point2::new(Length::ZERO, y),
            end: Point2::new(length, y),
        }
    }

    fn braced_panel(
        id: &str,
        offset: Length,
        length: Length,
        method: BracingMethod,
    ) -> BracedPanel {
        BracedPanel {
            id: ElementId::new(id),
            offset,
            length,
            method,
        }
    }

    fn wall_check(
        rule: &str,
        severity: CheckSeverity,
        op: CompareOp,
        value: FactOperand,
    ) -> ComplianceCheck {
        ComplianceCheck {
            rule: rule.to_owned(),
            citation: "Test".to_owned(),
            title: rule.to_owned(),
            severity,
            applies: Applicability::Always,
            scope: CheckScope::Walls {
                exterior_only: None,
                tags: Vec::new(),
            },
            requirement: Predicate::Compare {
                fact: Fact::WallLength,
                op,
                value,
            },
        }
    }

    fn has_outcome(report: &ComplianceReport, rule: &str, outcome: &Outcome) -> bool {
        report
            .entries
            .iter()
            .any(|entry| entry.rule == rule && &entry.outcome == outcome)
    }
}
