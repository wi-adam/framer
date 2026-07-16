use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BTreeSet};

use framer_core::{
    AuthoredEntityRef, AuthoredIntentId, AuthoredIntentMode, BuildingModel, CompareOp, ElementId,
    Fact, FactOperand, IntentExpression, Length, Point2, Predicate, PreferencePriority,
    ProjectIntentScope, QuarterTurn, room_boundary_on_level,
};
use framer_geometry::BodyRef;
use framer_solver::{MemberKind, generate_project_plan};
use framer_standards::{
    FactObservation, FactSnapshot, FactSubject, FactValue, PlacedObjectRef, PredicateObservation,
    Tri,
};
use thiserror::Error;

use crate::{
    AssertionRef, AssumptionEvidence, AssumptionPremise, BooleanIntentMode, DerivedAssertionId,
    DerivedAssertionProvider, DerivedAssertionRole, DerivedAssertionSource, ExactValue,
    ExactValueKind, GraphRevision, IntentEvidenceRef, IntentOutcome, IntentRecord, IntentReport,
    ObjectiveDefinition, ObjectiveDirection, ObjectiveObservation, ObjectivePriority,
    PlacementPatch, PlacementPatchError, PlacementPose, PlacementTarget, StagedPlacementPatch,
    StandardsRuleRef, analyze_project, current_placement_pose, stage_placement_patch,
};

const COARSE_LATTICE_INTERVALS: i64 = 8;
const SEARCH_BEAM_WIDTH: usize = 8;
const MAX_FACT_MEASUREMENTS: u32 = 2_048;
const MAX_FULL_CANDIDATE_ANALYSES: u32 = 128;
const MAX_OPTIONS: usize = 8;

/// Process-local authored-document generation. This value is deliberately opaque: only equality
/// and ordering are meaningful to the analysis layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentRevision(u64);

impl DocumentRevision {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Complete authority for one disposable resolution run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResolutionRevision {
    pub graph: GraphRevision,
    pub document: DocumentRevision,
}

impl ResolutionRevision {
    pub const fn new(graph: GraphRevision, document: DocumentRevision) -> Self {
        Self { graph, document }
    }
}

/// Explicit, closed request vocabulary for lazy candidate synthesis.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResolutionRequest {
    PlacementClearance { target: AuthoredIntentId },
}

impl ResolutionRequest {
    pub fn placement_clearance(target: AuthoredIntentId) -> Self {
        Self::PlacementClearance { target }
    }

    pub const fn target(&self) -> &AuthoredIntentId {
        match self {
            Self::PlacementClearance { target } => target,
        }
    }
}

/// Revision-neutral form of a generated-member source embedded in a derived assertion id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GeneratedMemberSemanticRef {
    pub host: AuthoredEntityRef,
    pub member_id: String,
    pub kind: MemberKind,
}

/// Revision-neutral semantic source for a derived assertion. All provider/source/role meaning is
/// preserved; only disposable [`GraphRevision`] fields are removed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DerivedAssertionSemanticSource {
    Project,
    Authored(AuthoredEntityRef),
    GeneratedMember(GeneratedMemberSemanticRef),
    StandardsRule(StandardsRuleRef),
    PhysicalBody(BodyRef),
}

/// Revision-neutral identity of a derived assertion used to compare candidate reports.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DerivedAssertionSemanticKey {
    pub provider: DerivedAssertionProvider,
    pub source: DerivedAssertionSemanticSource,
    pub role: DerivedAssertionRole,
}

impl DerivedAssertionSemanticKey {
    pub fn from_derived(derived: &DerivedAssertionId) -> Self {
        let source = match &derived.source {
            DerivedAssertionSource::Project => DerivedAssertionSemanticSource::Project,
            DerivedAssertionSource::Authored(entity) => {
                DerivedAssertionSemanticSource::Authored(entity.clone())
            }
            DerivedAssertionSource::GeneratedMember(member) => {
                DerivedAssertionSemanticSource::GeneratedMember(GeneratedMemberSemanticRef {
                    host: member.host.clone(),
                    member_id: member.member_id.clone(),
                    kind: member.kind,
                })
            }
            DerivedAssertionSource::StandardsRule(rule) => {
                DerivedAssertionSemanticSource::StandardsRule(rule.clone())
            }
            DerivedAssertionSource::PhysicalBody(body) => {
                DerivedAssertionSemanticSource::PhysicalBody(body.body.clone())
            }
        };
        Self {
            provider: derived.provider,
            source,
            role: derived.role.clone(),
        }
    }
}

/// Stable comparison key for authored and derived assertion records across candidate revisions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssertionSemanticKey {
    Authored(AuthoredIntentId),
    Derived(DerivedAssertionSemanticKey),
}

impl AssertionSemanticKey {
    pub fn from_assertion(reference: &AssertionRef) -> Self {
        match reference {
            AssertionRef::Authored(id) => Self::Authored(id.clone()),
            AssertionRef::Derived(derived) => {
                Self::Derived(DerivedAssertionSemanticKey::from_derived(derived))
            }
        }
    }
}

/// Stable public categories for intent outcomes across candidate revisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntentOutcomeCategory {
    Satisfied,
    Violated,
    Waived,
    Unknown,
    NotApplicable,
}

impl IntentOutcomeCategory {
    pub const fn from_outcome(outcome: &IntentOutcome) -> Self {
        match outcome {
            IntentOutcome::Satisfied => Self::Satisfied,
            IntentOutcome::Violated => Self::Violated,
            IntentOutcome::Unknown(_) => Self::Unknown,
            IntentOutcome::Waived { .. } => Self::Waived,
            IntentOutcome::NotApplicable => Self::NotApplicable,
        }
    }
}

/// Canonically sorted assertion keys grouped by their explicit outcome category.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CategorizedIntentOutcomes {
    pub satisfied: Vec<AssertionSemanticKey>,
    pub violated: Vec<AssertionSemanticKey>,
    pub waived: Vec<AssertionSemanticKey>,
    pub unknown: Vec<AssertionSemanticKey>,
    pub not_applicable: Vec<AssertionSemanticKey>,
}

impl CategorizedIntentOutcomes {
    fn push(&mut self, key: AssertionSemanticKey, category: IntentOutcomeCategory) {
        match category {
            IntentOutcomeCategory::Satisfied => self.satisfied.push(key),
            IntentOutcomeCategory::Violated => self.violated.push(key),
            IntentOutcomeCategory::Waived => self.waived.push(key),
            IntentOutcomeCategory::Unknown => self.unknown.push(key),
            IntentOutcomeCategory::NotApplicable => self.not_applicable.push(key),
        }
    }
}

/// Before/after category change for one semantic assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionTransition {
    pub assertion: AssertionSemanticKey,
    pub before: Option<IntentOutcomeCategory>,
    pub after: Option<IntentOutcomeCategory>,
}

/// Canonical per-assertion outcome and its exact revision-local evidence payload for one side of
/// a candidate comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionIntentEffect {
    pub assertion: AssertionSemanticKey,
    pub mode: BooleanIntentMode,
    pub outcome: IntentOutcomeCategory,
    pub evidence: Vec<IntentEvidenceRef>,
    pub predicate_observation: Option<PredicateObservation>,
}

/// Exact objective result and revision-local evidence for one side of a candidate comparison.
/// Objective observations remain typed scalars; they are never lowered to boolean outcomes or
/// lossy numeric scores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionObjectiveEffect {
    pub assertion: AssertionSemanticKey,
    pub definition: ObjectiveDefinition,
    pub observation: ObjectiveObservation,
    pub evidence: Vec<IntentEvidenceRef>,
}

/// Exact typed premise and revision-local provenance for one side of a candidate comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionAssumptionEffect {
    pub assertion: AssertionSemanticKey,
    pub premise: AssumptionPremise,
    pub evidence: AssumptionEvidence,
    pub provenance: Vec<IntentEvidenceRef>,
}

/// Exact categorized and mode-specific before/after evidence for one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionEffects {
    pub before: CategorizedIntentOutcomes,
    pub after: CategorizedIntentOutcomes,
    pub transitions: Vec<AssertionTransition>,
    pub before_intents: Vec<ResolutionIntentEffect>,
    pub after_intents: Vec<ResolutionIntentEffect>,
    pub before_objectives: Vec<ResolutionObjectiveEffect>,
    pub after_objectives: Vec<ResolutionObjectiveEffect>,
    pub before_assumptions: Vec<ResolutionAssumptionEffect>,
    pub after_assumptions: Vec<ResolutionAssumptionEffect>,
}

/// Cost row for one authored preference priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreferenceTierCost {
    pub priority: PreferencePriority,
    pub violated_or_unknown: u32,
    pub unknown: u32,
}

/// One canonical, named objective-vector component. Fields are private so callers can inspect but
/// cannot tamper with generated ranking state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionObjectiveComponent {
    assertion: AssertionSemanticKey,
    definition: ObjectiveDefinition,
    observation: ObjectiveObservation,
}

impl ResolutionObjectiveComponent {
    pub const fn assertion(&self) -> &AssertionSemanticKey {
        &self.assertion
    }

    pub const fn definition(&self) -> &ObjectiveDefinition {
        &self.definition
    }

    pub const fn observation(&self) -> &ObjectiveObservation {
        &self.observation
    }
}

/// Exact immutable inputs to deterministic candidate ranking. Preference rows and named objective
/// components are canonical; the private ranking key compares stronger tiers first and keeps
/// unknown diagnostic detail out of optimization while this public payload retains it exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionObjectiveVector {
    required_violated_or_unknown: u32,
    required_unknown: u32,
    preference_tiers: Vec<PreferenceTierCost>,
    objective_components: Vec<ResolutionObjectiveComponent>,
    manhattan_movement_ticks: u64,
    quarter_turn_distance: u8,
}

impl ResolutionObjectiveVector {
    pub const fn required_violated_or_unknown(&self) -> u32 {
        self.required_violated_or_unknown
    }

    pub const fn required_unknown(&self) -> u32 {
        self.required_unknown
    }

    pub fn preference_tiers(&self) -> &[PreferenceTierCost] {
        &self.preference_tiers
    }

    pub fn objective_components(&self) -> &[ResolutionObjectiveComponent] {
        &self.objective_components
    }

    pub const fn manhattan_movement_ticks(&self) -> u64 {
        self.manhattan_movement_ticks
    }

    pub const fn quarter_turn_distance(&self) -> u8 {
        self.quarter_turn_distance
    }
}

#[derive(Debug, Clone)]
struct ResolutionRankingKey {
    required_violated_or_unknown: u32,
    required_unknown: u32,
    preference_tiers: Vec<RankingPreferenceTier>,
    objective_components: Vec<RankingObjectiveComponent>,
    manhattan_movement_ticks: u64,
    quarter_turn_distance: u8,
}

impl PartialEq for ResolutionRankingKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ResolutionRankingKey {}

impl Ord for ResolutionRankingKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.required_violated_or_unknown
            .cmp(&other.required_violated_or_unknown)
            .then_with(|| self.required_unknown.cmp(&other.required_unknown))
            .then_with(|| {
                compare_ranking_preference_tiers(&self.preference_tiers, &other.preference_tiers)
            })
            .then_with(|| self.objective_components.cmp(&other.objective_components))
            .then_with(|| {
                self.manhattan_movement_ticks
                    .cmp(&other.manhattan_movement_ticks)
            })
            .then_with(|| self.quarter_turn_distance.cmp(&other.quarter_turn_distance))
    }
}

impl PartialOrd for ResolutionRankingKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RankingPreferenceTier {
    priority: Reverse<PreferencePriority>,
    violated_or_unknown: u32,
    unknown: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RankingObjectiveComponent {
    priority: Reverse<ObjectivePriority>,
    component: String,
    direction: ObjectiveDirection,
    value_kind: ExactValueKind,
    assertion: AssertionSemanticKey,
    observation: RankingObjectiveObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RankingObjectiveObservation {
    Known(RankingExactValue),
    NotApplicable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RankingExactValue {
    MinimizeLength(i64),
    MaximizeLength(Reverse<i64>),
    MinimizeInt(i64),
    MaximizeInt(Reverse<i64>),
}

/// Authored placed-object family named by a semantic patch key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlacementTargetKind {
    FurnishingInstance,
    MepInstance,
}

/// Last-resort deterministic option tie-breaker: target kind/id, rotation, X, then Y.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlacementPatchSemanticKey {
    pub target_kind: PlacementTargetKind,
    pub target_id: ElementId,
    pub rotation: u8,
    pub x_ticks: i64,
    pub y_ticks: i64,
}

impl PlacementPatchSemanticKey {
    pub fn from_patch(patch: &PlacementPatch) -> Self {
        let (target_kind, target_id) = match &patch.target {
            PlacementTarget::FurnishingInstance(id) => {
                (PlacementTargetKind::FurnishingInstance, id.clone())
            }
            PlacementTarget::MepInstance(id) => (PlacementTargetKind::MepInstance, id.clone()),
        };
        Self {
            target_kind,
            target_id,
            rotation: quarter_turn_index(patch.replacement.rotation),
            x_ticks: patch.replacement.position.x.ticks(),
            y_ticks: patch.replacement.position.y.ticks(),
        }
    }
}

/// Immutable typed authored change, exact effects, rank inputs, and target evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionOption {
    origin: ResolutionRevision,
    candidate_revision: GraphRevision,
    target: AuthoredIntentId,
    patch: PlacementPatch,
    effects: ResolutionEffects,
    objective: ResolutionObjectiveVector,
    evidence: Vec<IntentEvidenceRef>,
    target_observation: Option<PredicateObservation>,
}

impl ResolutionOption {
    pub const fn origin(&self) -> ResolutionRevision {
        self.origin
    }

    pub const fn candidate_revision(&self) -> GraphRevision {
        self.candidate_revision
    }

    pub const fn target(&self) -> &AuthoredIntentId {
        &self.target
    }

    pub const fn patch(&self) -> &PlacementPatch {
        &self.patch
    }

    pub const fn effects(&self) -> &ResolutionEffects {
        &self.effects
    }

    pub const fn objective(&self) -> &ResolutionObjectiveVector {
        &self.objective
    }

    pub fn evidence(&self) -> &[IntentEvidenceRef] {
        &self.evidence
    }

    pub const fn target_observation(&self) -> Option<&PredicateObservation> {
        self.target_observation.as_ref()
    }
}

/// Whether a bounded request found at least one option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSearchOutcome {
    OptionsFound,
    BoundedNoOptions,
}

/// Makes the finite nature of synthesis explicit. An empty bounded search is not a proof of
/// mathematical infeasibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionSearchSummary {
    pub outcome: ResolutionSearchOutcome,
    pub fact_measurements: u32,
    pub measurement_cap: u32,
    pub fact_measurement_truncated: bool,
    pub feasible_candidates: u32,
    pub fully_analyzed_candidates: u32,
    pub candidate_analysis_cap: u32,
    pub candidate_analysis_truncated: bool,
}

/// Canonically ranked immutable options and the disclosed finite-search summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionOptionSet {
    origin: ResolutionRevision,
    target: AuthoredIntentId,
    options: Vec<ResolutionOption>,
    pub search: ResolutionSearchSummary,
}

impl ResolutionOptionSet {
    pub const fn origin(&self) -> ResolutionRevision {
        self.origin
    }

    pub const fn target(&self) -> &AuthoredIntentId {
        &self.target
    }

    pub fn options(&self) -> &[ResolutionOption] {
        &self.options
    }

    pub const fn search(&self) -> ResolutionSearchSummary {
        self.search
    }

    fn rebind_document(&mut self, revision: ResolutionRevision) {
        debug_assert_eq!(self.origin.graph, revision.graph);
        self.origin = revision;
        for option in &mut self.options {
            option.origin = revision;
        }
    }
}

/// Observable cache activity for focused tests and operational diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResolutionCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub rebinds: u64,
}

/// Lazy candidate cache bound to both the deterministic graph and process-local document
/// revisions. Candidate providers are called only by [`generate_resolution_options`].
#[derive(Debug, Default)]
pub struct ResolutionCache {
    bound: Option<ResolutionRevision>,
    entries: BTreeMap<ResolutionRequest, ResolutionOptionSet>,
    stats: ResolutionCacheStats,
}

impl ResolutionCache {
    pub fn clear(&mut self) {
        self.bound = None;
        self.entries.clear();
    }

    pub const fn stats(&self) -> ResolutionCacheStats {
        self.stats
    }

    fn rebind(&mut self, revision: ResolutionRevision) {
        let Some(bound) = self.bound else {
            self.bound = Some(revision);
            return;
        };
        if bound == revision {
            return;
        }

        self.stats.rebinds = self.stats.rebinds.saturating_add(1);
        self.bound = Some(revision);
        if bound.graph == revision.graph {
            // A no-op authored rebuild advances the process-local mutation guard while retaining
            // byte-identical model and external analysis inputs. Re-authorize the immutable
            // generated set for the new document generation without invoking the provider.
            for option_set in self.entries.values_mut() {
                option_set.rebind_document(revision);
            }
        } else {
            self.entries.clear();
        }
    }
}

/// Closed set of resolution capabilities callers may probe explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResolutionCapability {
    PlacementClearance,
    StructuralAlternatives,
}

/// Honest availability result for a resolution capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionCapabilityAvailability {
    Available,
    Unavailable { reason: &'static str },
}

/// Explanation returned while structural synthesis prerequisites remain unimplemented.
pub const STRUCTURAL_RESOLUTION_UNAVAILABLE_REASON: &str = "Structural alternatives require authored supports and load paths, member capacity and deflection evaluation, and engineered member families; this slice supports placement-clearance moves only.";

/// Report whether the requested candidate family is currently supported.
pub const fn resolution_capability(
    capability: ResolutionCapability,
) -> ResolutionCapabilityAvailability {
    match capability {
        ResolutionCapability::PlacementClearance => ResolutionCapabilityAvailability::Available,
        ResolutionCapability::StructuralAlternatives => {
            ResolutionCapabilityAvailability::Unavailable {
                reason: STRUCTURAL_RESOLUTION_UNAVAILABLE_REASON,
            }
        }
    }
}

/// Fail-closed request, revision, candidate-contract, and patch errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResolutionError {
    #[error("resolution graph revision is stale (expected {expected}, supplied {supplied})")]
    StaleGraphRevision {
        expected: GraphRevision,
        supplied: GraphRevision,
    },
    #[error("resolution document revision is stale (expected {expected:?}, supplied {supplied:?})")]
    StaleDocumentRevision {
        expected: DocumentRevision,
        supplied: DocumentRevision,
    },
    #[error("current intent report revision {report} does not match resolution graph {supplied}")]
    ReportRevisionMismatch {
        report: GraphRevision,
        supplied: GraphRevision,
    },
    #[error(
        "resolution option candidate revision does not match its staged authored model (expected {expected}, actual {actual})"
    )]
    CandidateRevisionMismatch {
        expected: GraphRevision,
        actual: GraphRevision,
    },
    #[error("resolution target {0:?} was not found")]
    TargetMissing(AuthoredIntentId),
    #[error("resolution target {0:?} is not a currently violated boolean assertion")]
    TargetNotViolated(AuthoredIntentId),
    #[error("resolution target {0:?} is not an authored placed-object clearance assertion")]
    TargetNotPlacementClearance(AuthoredIntentId),
    #[error("resolution target {0:?} does not have one exact placed-object and room scope")]
    InvalidTargetScope(AuthoredIntentId),
    #[error("resolution target {0:?} has no closed room boundary")]
    MissingRoomBoundary(AuthoredIntentId),
    #[error("resolution target produced duplicate or inconsistent semantic assertion key {0:?}")]
    InconsistentSemanticAssertion(Box<AssertionSemanticKey>),
    #[error("candidate intent report omitted objective component {0:?}")]
    MissingObjectiveComponent(Box<AssertionSemanticKey>),
    #[error("candidate intent report introduced objective component {0:?}")]
    UnexpectedObjectiveComponent(Box<AssertionSemanticKey>),
    #[error("objective vector contains duplicate named component {0:?}")]
    DuplicateObjectiveComponent(String),
    #[error("objective component {assertion:?} has an empty or whitespace-only name")]
    InvalidObjectiveComponentName {
        assertion: Box<AssertionSemanticKey>,
    },
    #[error(
        "objective component {assertion:?} changed definition (expected {expected:?}, actual {actual:?})"
    )]
    ObjectiveDefinitionMismatch {
        assertion: Box<AssertionSemanticKey>,
        expected: ObjectiveDefinition,
        actual: ObjectiveDefinition,
    },
    #[error(
        "objective component {assertion:?} emitted {actual:?}, but its definition declares {expected:?}"
    )]
    ObjectiveValueKindMismatch {
        assertion: Box<AssertionSemanticKey>,
        expected: ExactValueKind,
        actual: ExactValueKind,
    },
    #[error("project analysis failed while generating resolution options: {0}")]
    Analysis(String),
    #[error(transparent)]
    Patch(#[from] PlacementPatchError),
}

/// Explicitly request a lazy, revision-cached set of resolution options.
pub fn generate_resolution_options(
    model: &BuildingModel,
    revision: ResolutionRevision,
    request: &ResolutionRequest,
    cache: &mut ResolutionCache,
) -> Result<ResolutionOptionSet, ResolutionError> {
    validate_model_graph_revision(model, revision.graph)?;
    cache.rebind(revision);
    if let Some(cached) = cache.entries.get(request) {
        cache.stats.hits = cache.stats.hits.saturating_add(1);
        return Ok(cached.clone());
    }

    cache.stats.misses = cache.stats.misses.saturating_add(1);
    let generated = generate_uncached(model, revision, request)?;
    cache.entries.insert(request.clone(), generated.clone());
    Ok(generated)
}

/// Validate both current revisions and the patch's expected pose, returning a sorted, validated
/// authored-model preview. The source model remains unchanged.
pub fn stage_resolution_option(
    model: &BuildingModel,
    option: &ResolutionOption,
    current: ResolutionRevision,
) -> Result<BuildingModel, ResolutionError> {
    validate_option_revision(model, option, current)?;
    let staged = stage_placement_patch(model, &option.patch)?;
    let actual = GraphRevision::for_model(staged.model()).map_err(|error| {
        ResolutionError::Analysis(format!(
            "could not fingerprint staged authored model: {error}"
        ))
    })?;
    if actual != option.candidate_revision {
        return Err(ResolutionError::CandidateRevisionMismatch {
            expected: option.candidate_revision,
            actual,
        });
    }
    Ok(staged.into_model())
}

/// Transactionally apply a current resolution option at an ordinary authored edit boundary.
pub fn apply_resolution_option(
    model: &mut BuildingModel,
    option: &ResolutionOption,
    current: ResolutionRevision,
) -> Result<(), ResolutionError> {
    *model = stage_resolution_option(model, option, current)?;
    Ok(())
}

fn validate_model_graph_revision(
    model: &BuildingModel,
    supplied: GraphRevision,
) -> Result<(), ResolutionError> {
    let expected = GraphRevision::for_model(model).map_err(|error| {
        ResolutionError::Analysis(format!("could not fingerprint authored model: {error}"))
    })?;
    if expected != supplied {
        return Err(ResolutionError::StaleGraphRevision { expected, supplied });
    }
    Ok(())
}

fn validate_option_revision(
    model: &BuildingModel,
    option: &ResolutionOption,
    current: ResolutionRevision,
) -> Result<(), ResolutionError> {
    if option.origin.graph != current.graph {
        return Err(ResolutionError::StaleGraphRevision {
            expected: option.origin.graph,
            supplied: current.graph,
        });
    }
    if option.origin.document != current.document {
        return Err(ResolutionError::StaleDocumentRevision {
            expected: option.origin.document,
            supplied: current.document,
        });
    }
    validate_model_graph_revision(model, current.graph)
}

fn generate_uncached(
    model: &BuildingModel,
    revision: ResolutionRevision,
    request: &ResolutionRequest,
) -> Result<ResolutionOptionSet, ResolutionError> {
    let analysis =
        analyze_project(model).map_err(|error| ResolutionError::Analysis(error.to_string()))?;
    let current_report = analysis
        .intent_report
        .as_ref()
        .map_err(|error| ResolutionError::Analysis(error.to_string()))?;
    if current_report.revision() != revision.graph {
        return Err(ResolutionError::ReportRevisionMismatch {
            report: current_report.revision(),
            supplied: revision.graph,
        });
    }

    match request {
        ResolutionRequest::PlacementClearance { target } => generate_placement_clearance_options(
            model,
            current_report,
            revision,
            target,
            MAX_FULL_CANDIDATE_ANALYSES,
        ),
    }
}

#[derive(Debug, Clone)]
struct PlacementClearanceTarget {
    target: AuthoredIntentId,
    placement: PlacementTarget,
    room: ElementId,
    predicate: Predicate,
}

fn placement_clearance_target(
    model: &BuildingModel,
    report: &IntentReport,
    target: &AuthoredIntentId,
) -> Result<PlacementClearanceTarget, ResolutionError> {
    let reference = AssertionRef::Authored(target.clone());
    let Some(IntentRecord::Boolean(record)) = report.record(&reference) else {
        return if report.record(&reference).is_some() {
            Err(ResolutionError::TargetNotViolated(target.clone()))
        } else {
            Err(ResolutionError::TargetMissing(target.clone()))
        };
    };
    if !matches!(record.outcome, IntentOutcome::Violated) {
        return Err(ResolutionError::TargetNotViolated(target.clone()));
    }

    let authored = model
        .intents
        .iter()
        .find(|intent| intent.id == *target)
        .ok_or_else(|| ResolutionError::TargetMissing(target.clone()))?;
    let ProjectIntentScope::Exact(scope) = &authored.scope;
    let placement = match &scope.subject {
        AuthoredEntityRef::FurnishingInstance(id) => {
            PlacementTarget::FurnishingInstance(id.clone())
        }
        AuthoredEntityRef::MepInstance(id) => PlacementTarget::MepInstance(id.clone()),
        _ => return Err(ResolutionError::InvalidTargetScope(target.clone())),
    };
    let [AuthoredEntityRef::Room(room)] = scope.participants.as_slice() else {
        return Err(ResolutionError::InvalidTargetScope(target.clone()));
    };
    let IntentExpression::FactPredicate(predicate) = &authored.expression;
    if !predicate_contains_clearance(predicate) {
        return Err(ResolutionError::TargetNotPlacementClearance(target.clone()));
    }

    Ok(PlacementClearanceTarget {
        target: target.clone(),
        placement,
        room: room.clone(),
        predicate: predicate.clone(),
    })
}

fn predicate_contains_clearance(predicate: &Predicate) -> bool {
    match predicate {
        Predicate::All(children) | Predicate::Any(children) => {
            children.iter().any(predicate_contains_clearance)
        }
        Predicate::Not(child) => predicate_contains_clearance(child),
        Predicate::Compare { fact, value, .. } => {
            matches!(fact, Fact::PlacedObjectClearance { .. })
                || matches!(value, FactOperand::Fact(Fact::PlacedObjectClearance { .. }))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PoseKey {
    rotation: u8,
    x_ticks: i64,
    y_ticks: i64,
}

impl PoseKey {
    fn new(pose: PlacementPose) -> Self {
        Self {
            rotation: quarter_turn_index(pose.rotation),
            x_ticks: pose.position.x.ticks(),
            y_ticks: pose.position.y.ticks(),
        }
    }

    fn pose(self) -> PlacementPose {
        PlacementPose::new(
            Point2::new(
                Length::from_ticks(self.x_ticks),
                Length::from_ticks(self.y_ticks),
            ),
            quarter_turn_from_index(self.rotation),
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct MeasuredCandidate {
    pose: PlacementPose,
    feasible: bool,
    shortfall_ticks: u64,
}

#[derive(Debug, Default)]
struct FactMeasurementBudget {
    used: u32,
    truncated: bool,
}

fn generate_placement_clearance_options(
    model: &BuildingModel,
    current_report: &IntentReport,
    revision: ResolutionRevision,
    target_id: &AuthoredIntentId,
    candidate_analysis_cap: u32,
) -> Result<ResolutionOptionSet, ResolutionError> {
    let target = placement_clearance_target(model, current_report, target_id)?;
    let current_pose = current_placement_pose(model, &target.placement)?;
    let room = model
        .rooms
        .iter()
        .find(|room| room.id == target.room)
        .ok_or_else(|| ResolutionError::InvalidTargetScope(target.target.clone()))?;
    let boundary = room_boundary_on_level(model, &room.level, room.seed)
        .ok_or_else(|| ResolutionError::MissingRoomBoundary(target.target.clone()))?;
    let Some(min_x) = boundary.vertices.iter().map(|point| point.x.ticks()).min() else {
        return Err(ResolutionError::MissingRoomBoundary(target.target.clone()));
    };
    let max_x = boundary
        .vertices
        .iter()
        .map(|point| point.x.ticks())
        .max()
        .expect("non-empty boundary has an X maximum");
    let min_y = boundary
        .vertices
        .iter()
        .map(|point| point.y.ticks())
        .min()
        .expect("non-empty boundary has a Y minimum");
    let max_y = boundary
        .vertices
        .iter()
        .map(|point| point.y.ticks())
        .max()
        .expect("non-empty boundary has a Y maximum");

    let facts_per_candidate = predicate_facts(&target.predicate).len().saturating_add(1) as u32;
    let mut measurement_budget = FactMeasurementBudget::default();
    let mut measured = BTreeMap::<PoseKey, MeasuredCandidate>::new();
    let mut coarse = BTreeSet::new();
    for rotation in QuarterTurn::ALL {
        coarse.insert(PoseKey::new(PlacementPose::new(
            current_pose.position,
            rotation,
        )));
        for x_index in 0..=COARSE_LATTICE_INTERVALS {
            let x = interpolate_ticks(min_x, max_x, x_index, COARSE_LATTICE_INTERVALS);
            for y_index in 0..=COARSE_LATTICE_INTERVALS {
                let y = interpolate_ticks(min_y, max_y, y_index, COARSE_LATTICE_INTERVALS);
                coarse.insert(PoseKey {
                    rotation: quarter_turn_index(rotation),
                    x_ticks: x,
                    y_ticks: y,
                });
            }
        }
    }
    measure_pose_keys(
        model,
        &target,
        current_pose,
        coarse,
        facts_per_candidate,
        &mut measurement_budget,
        &mut measured,
    )?;

    let mut step_x = ((i128::from(max_x) - i128::from(min_x)).unsigned_abs() / 8)
        .max(1)
        .min(i64::MAX as u128) as i64;
    let mut step_y = ((i128::from(max_y) - i128::from(min_y)).unsigned_abs() / 8)
        .max(1)
        .min(i64::MAX as u128) as i64;
    loop {
        step_x = (step_x / 2).max(1);
        step_y = (step_y / 2).max(1);
        let beam = search_beam(&measured, current_pose);
        let mut refinements = BTreeSet::new();
        for candidate in beam {
            let key = PoseKey::new(candidate.pose);
            for x_delta in [-step_x, 0, step_x] {
                for y_delta in [-step_y, 0, step_y] {
                    refinements.insert(PoseKey {
                        rotation: key.rotation,
                        x_ticks: offset_clamped(key.x_ticks, x_delta, min_x, max_x),
                        y_ticks: offset_clamped(key.y_ticks, y_delta, min_y, max_y),
                    });
                }
            }
        }
        let before = measured.len();
        measure_pose_keys(
            model,
            &target,
            current_pose,
            refinements,
            facts_per_candidate,
            &mut measurement_budget,
            &mut measured,
        )?;
        if (step_x == 1 && step_y == 1)
            || measured.len() == before
            || measurement_budget.used.saturating_add(facts_per_candidate) > MAX_FACT_MEASUREMENTS
        {
            break;
        }
    }

    let mut feasible = measured
        .values()
        .copied()
        .filter(|candidate| candidate.feasible && candidate.pose != current_pose)
        .collect::<Vec<_>>();
    feasible.sort_by(|left, right| {
        preliminary_candidate_key(left.pose, current_pose)
            .cmp(&preliminary_candidate_key(right.pose, current_pose))
    });
    let feasible_candidates = feasible.len().min(u32::MAX as usize) as u32;

    let before_modes = report_mode_effects(current_report)?;
    let target_key = AssertionSemanticKey::Authored(target.target.clone());
    let mut options = Vec::new();
    let mut fully_analyzed_candidates = 0u32;
    let mut candidate_analysis_truncated = false;
    let mut feasible = feasible.into_iter().peekable();
    while let Some(candidate) = feasible.next() {
        if fully_analyzed_candidates >= candidate_analysis_cap {
            candidate_analysis_truncated = true;
            break;
        }
        let patch = PlacementPatch::new(target.placement.clone(), current_pose, candidate.pose);
        let staged = match stage_placement_patch(model, &patch) {
            Ok(staged) => staged,
            Err(_) => continue,
        };
        fully_analyzed_candidates = fully_analyzed_candidates.saturating_add(1);
        let candidate_analysis = match analyze_project(staged.model()) {
            Ok(analysis) => analysis,
            Err(_) => continue,
        };
        let candidate_report = match candidate_analysis.intent_report.as_ref() {
            Ok(report) => report,
            Err(_) => continue,
        };
        let after_modes = report_mode_effects(candidate_report)?;
        validate_objective_contract(&before_modes.objectives, &after_modes.objectives)?;
        if after_modes
            .boolean_states
            .get(&target_key)
            .map(|state| state.category)
            != Some(IntentOutcomeCategory::Satisfied)
            || !required_non_regression(&before_modes.boolean_states, &after_modes.boolean_states)
        {
            continue;
        }
        let Some(IntentRecord::Boolean(target_record)) =
            candidate_report.record(&AssertionRef::Authored(target.target.clone()))
        else {
            continue;
        };
        let objective = objective_vector(
            &after_modes.boolean_states,
            &after_modes.objectives,
            current_pose,
            candidate.pose,
        )?;
        let effects = resolution_effects(&before_modes, &after_modes);
        options.push(ResolutionOption {
            origin: revision,
            candidate_revision: candidate_report.revision(),
            target: target.target.clone(),
            patch,
            effects,
            objective,
            evidence: target_record.evidence.clone(),
            target_observation: target_record.predicate_observation.clone(),
        });
        canonicalize_options(&mut options);
        if feasible
            .peek()
            .is_some_and(|next| analyzed_option_prefix_is_final(&options, next.pose, current_pose))
        {
            break;
        }
    }

    canonicalize_options(&mut options);
    let outcome = if options.is_empty() {
        ResolutionSearchOutcome::BoundedNoOptions
    } else {
        ResolutionSearchOutcome::OptionsFound
    };
    Ok(ResolutionOptionSet {
        origin: revision,
        target: target.target,
        options,
        search: ResolutionSearchSummary {
            outcome,
            fact_measurements: measurement_budget.used,
            measurement_cap: MAX_FACT_MEASUREMENTS,
            fact_measurement_truncated: measurement_budget.truncated,
            feasible_candidates,
            fully_analyzed_candidates,
            candidate_analysis_cap,
            candidate_analysis_truncated,
        },
    })
}

fn canonicalize_options(options: &mut Vec<ResolutionOption>) {
    options.sort_by(|left, right| {
        compare_objective_rank(&left.objective, &right.objective).then_with(|| {
            PlacementPatchSemanticKey::from_patch(&left.patch)
                .cmp(&PlacementPatchSemanticKey::from_patch(&right.patch))
        })
    });
    options.dedup_by(|left, right| left.patch == right.patch);
    options.truncate(MAX_OPTIONS);
}

/// Once eight analyzed options have zero hard/preference cost, the remaining preliminary order
/// (movement, rotation, semantic pose) is an exact lower bound for the only objective components
/// still able to differ. This permits an early stop without allowing the old movement-first
/// pre-analysis truncation to discard a better preference outcome.
fn analyzed_option_prefix_is_final(
    options: &[ResolutionOption],
    next_pose: PlacementPose,
    current_pose: PlacementPose,
) -> bool {
    if options.len() < MAX_OPTIONS
        || options.iter().any(|option| {
            option.objective.required_violated_or_unknown != 0
                || option.objective.required_unknown != 0
                || option
                    .objective
                    .preference_tiers
                    .iter()
                    .any(|tier| tier.violated_or_unknown != 0 || tier.unknown != 0)
                || !option.objective.objective_components.is_empty()
        })
    {
        return false;
    }
    let worst = options
        .last()
        .expect("a complete option prefix has a final option");
    preliminary_candidate_key(worst.patch.replacement, current_pose)
        <= preliminary_candidate_key(next_pose, current_pose)
}

fn measure_pose_keys(
    model: &BuildingModel,
    target: &PlacementClearanceTarget,
    current_pose: PlacementPose,
    poses: impl IntoIterator<Item = PoseKey>,
    facts_per_candidate: u32,
    measurement_budget: &mut FactMeasurementBudget,
    measured: &mut BTreeMap<PoseKey, MeasuredCandidate>,
) -> Result<(), ResolutionError> {
    for key in poses {
        if measured.contains_key(&key) {
            continue;
        }
        if measurement_budget.used.saturating_add(facts_per_candidate) > MAX_FACT_MEASUREMENTS {
            measurement_budget.truncated = true;
            continue;
        }
        let pose = key.pose();
        let patch = PlacementPatch::new(target.placement.clone(), current_pose, pose);
        let staged = if pose == current_pose {
            None
        } else {
            stage_placement_patch(model, &patch).ok()
        };
        let candidate = staged
            .as_ref()
            .map(StagedPlacementPatch::model)
            .unwrap_or(model);
        let plan = match generate_project_plan(candidate) {
            Ok(plan) => plan,
            Err(_) => {
                measurement_budget.used =
                    measurement_budget.used.saturating_add(facts_per_candidate);
                measured.insert(
                    key,
                    MeasuredCandidate {
                        pose,
                        feasible: false,
                        shortfall_ticks: u64::MAX,
                    },
                );
                continue;
            }
        };
        let resolved = candidate.resolved_standards();
        let snapshot = FactSnapshot::new(candidate, &resolved, &plan);
        let subject = FactSubject::placed_object_exact(
            placed_object_ref(&target.placement),
            target.room.clone(),
        );
        let containment = snapshot.observe(Fact::PlacedObjectContainedInRoom, &subject);
        let predicate = snapshot.evaluate_predicate(&target.predicate, &subject);
        measurement_budget.used = measurement_budget.used.saturating_add(facts_per_candidate);
        let contained = matches!(containment, FactObservation::Known(FactValue::Flag(true)));
        let feasible = contained && predicate.result == Tri::True;
        measured.insert(
            key,
            MeasuredCandidate {
                pose,
                feasible,
                shortfall_ticks: if feasible {
                    0
                } else {
                    clearance_shortfall(&target.predicate, &predicate).unwrap_or(u64::MAX - 1)
                },
            },
        );
    }
    Ok(())
}

fn placed_object_ref(target: &PlacementTarget) -> PlacedObjectRef {
    match target {
        PlacementTarget::FurnishingInstance(id) => PlacedObjectRef::FurnishingInstance(id.clone()),
        PlacementTarget::MepInstance(id) => PlacedObjectRef::MepInstance(id.clone()),
    }
}

fn predicate_facts(predicate: &Predicate) -> BTreeSet<Fact> {
    fn collect(predicate: &Predicate, facts: &mut BTreeSet<Fact>) {
        match predicate {
            Predicate::All(children) | Predicate::Any(children) => {
                for child in children {
                    collect(child, facts);
                }
            }
            Predicate::Not(child) => collect(child, facts),
            Predicate::Compare { fact, value, .. } => {
                facts.insert(*fact);
                if let FactOperand::Fact(other) = value {
                    facts.insert(*other);
                }
            }
        }
    }
    let mut facts = BTreeSet::new();
    collect(predicate, &mut facts);
    facts
}

fn clearance_shortfall(predicate: &Predicate, observation: &PredicateObservation) -> Option<u64> {
    let observed = observation
        .observed_facts
        .iter()
        .map(|entry| (entry.fact, &entry.observation))
        .collect::<BTreeMap<_, _>>();

    fn operand_ticks(
        operand: &FactOperand,
        observed: &BTreeMap<Fact, &FactObservation>,
    ) -> Option<i64> {
        match operand {
            FactOperand::LengthLiteral(length) => Some(length.ticks()),
            FactOperand::Fact(fact) => match observed.get(fact)? {
                FactObservation::Known(FactValue::Length(length)) => Some(length.ticks()),
                _ => None,
            },
            FactOperand::IntLiteral(_) | FactOperand::FlagLiteral(_) => None,
        }
    }

    fn recurse(predicate: &Predicate, observed: &BTreeMap<Fact, &FactObservation>) -> Option<u64> {
        match predicate {
            Predicate::All(children) => children.iter().try_fold(0u64, |total, child| {
                recurse(child, observed).map(|value| total.saturating_add(value))
            }),
            Predicate::Any(children) => children
                .iter()
                .filter_map(|child| recurse(child, observed))
                .min(),
            Predicate::Not(_) => None,
            Predicate::Compare { fact, op, value }
                if matches!(fact, Fact::PlacedObjectClearance { .. }) =>
            {
                let left = match observed.get(fact)? {
                    FactObservation::Known(FactValue::Length(length)) => length.ticks(),
                    _ => return None,
                };
                let right = operand_ticks(value, observed)?;
                let shortfall = match op {
                    CompareOp::Lt if left < right => 0,
                    CompareOp::Lt => i128::from(left) - i128::from(right) + 1,
                    CompareOp::Le if left <= right => 0,
                    CompareOp::Le => i128::from(left) - i128::from(right),
                    CompareOp::Eq => (i128::from(left) - i128::from(right)).abs(),
                    CompareOp::Ge if left >= right => 0,
                    CompareOp::Ge => i128::from(right) - i128::from(left),
                    CompareOp::Gt if left > right => 0,
                    CompareOp::Gt => i128::from(right) - i128::from(left) + 1,
                    CompareOp::Ne if left != right => 0,
                    CompareOp::Ne => 1,
                };
                Some(shortfall.max(0).min(i128::from(u64::MAX)) as u64)
            }
            Predicate::Compare { .. } => Some(0),
        }
    }

    recurse(predicate, &observed)
}

fn search_beam(
    measured: &BTreeMap<PoseKey, MeasuredCandidate>,
    current: PlacementPose,
) -> Vec<MeasuredCandidate> {
    let mut candidates = measured.values().copied().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.shortfall_ticks
            .cmp(&right.shortfall_ticks)
            .then_with(|| {
                preliminary_candidate_key(left.pose, current)
                    .cmp(&preliminary_candidate_key(right.pose, current))
            })
    });
    candidates.truncate(SEARCH_BEAM_WIDTH);
    candidates
}

fn preliminary_candidate_key(pose: PlacementPose, current: PlacementPose) -> (u64, u8, PoseKey) {
    (
        manhattan_movement(current, pose),
        quarter_turn_distance(current.rotation, pose.rotation),
        PoseKey::new(pose),
    )
}

fn interpolate_ticks(min: i64, max: i64, index: i64, intervals: i64) -> i64 {
    let value = i128::from(min)
        + (i128::from(max) - i128::from(min)) * i128::from(index) / i128::from(intervals);
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

fn offset_clamped(value: i64, delta: i64, min: i64, max: i64) -> i64 {
    (i128::from(value) + i128::from(delta)).clamp(i128::from(min), i128::from(max)) as i64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BooleanState {
    mode: BooleanIntentMode,
    category: IntentOutcomeCategory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportModeEffects {
    boolean_states: BTreeMap<AssertionSemanticKey, BooleanState>,
    booleans: Vec<ResolutionIntentEffect>,
    objectives: Vec<ResolutionObjectiveEffect>,
    assumptions: Vec<ResolutionAssumptionEffect>,
}

/// Adapt every closed intent-record mode in one pass. This prevents an objective or assumption
/// from disappearing merely because a boolean-only caller forgot to inspect another variant.
fn report_mode_effects(report: &IntentReport) -> Result<ReportModeEffects, ResolutionError> {
    let mut boolean_states = BTreeMap::new();
    let mut booleans = Vec::new();
    let mut objectives = Vec::new();
    let mut assumptions = Vec::new();
    let mut assertions = BTreeSet::new();
    for record in report.records() {
        let key = AssertionSemanticKey::from_assertion(&record.assertion().reference);
        if !assertions.insert(key.clone()) {
            return Err(ResolutionError::InconsistentSemanticAssertion(Box::new(
                key,
            )));
        }
        match record {
            IntentRecord::Boolean(record) => {
                let category = IntentOutcomeCategory::from_outcome(&record.outcome);
                boolean_states.insert(
                    key.clone(),
                    BooleanState {
                        mode: record.mode,
                        category,
                    },
                );
                booleans.push(ResolutionIntentEffect {
                    assertion: key,
                    mode: record.mode,
                    outcome: category,
                    evidence: record.evidence.clone(),
                    predicate_observation: record.predicate_observation.clone(),
                });
            }
            IntentRecord::Objective(record) => {
                validate_objective_definition(&key, &record.objective)?;
                validate_objective_value_kind(&key, &record.objective, &record.observation)?;
                objectives.push(ResolutionObjectiveEffect {
                    assertion: key,
                    definition: record.objective.clone(),
                    observation: record.observation.clone(),
                    evidence: record.evidence.clone(),
                });
            }
            IntentRecord::Assumption(record) => {
                assumptions.push(ResolutionAssumptionEffect {
                    assertion: key,
                    premise: record.premise.clone(),
                    evidence: record.evidence.clone(),
                    provenance: record.provenance.clone(),
                });
            }
        }
    }
    booleans.sort_by(|left, right| left.assertion.cmp(&right.assertion));
    objectives.sort_by(compare_objective_effect_keys);
    assumptions.sort_by(|left, right| left.assertion.cmp(&right.assertion));
    Ok(ReportModeEffects {
        boolean_states,
        booleans,
        objectives,
        assumptions,
    })
}

fn validate_objective_definition(
    assertion: &AssertionSemanticKey,
    definition: &ObjectiveDefinition,
) -> Result<(), ResolutionError> {
    if definition.component.trim().is_empty() {
        return Err(ResolutionError::InvalidObjectiveComponentName {
            assertion: Box::new(assertion.clone()),
        });
    }
    Ok(())
}

fn validate_objective_value_kind(
    assertion: &AssertionSemanticKey,
    definition: &ObjectiveDefinition,
    observation: &ObjectiveObservation,
) -> Result<(), ResolutionError> {
    let ObjectiveObservation::Known(value) = observation else {
        return Ok(());
    };
    let actual = value.kind();
    if actual != definition.value_kind {
        return Err(ResolutionError::ObjectiveValueKindMismatch {
            assertion: Box::new(assertion.clone()),
            expected: definition.value_kind,
            actual,
        });
    }
    Ok(())
}

fn validate_objective_contract(
    before: &[ResolutionObjectiveEffect],
    after: &[ResolutionObjectiveEffect],
) -> Result<(), ResolutionError> {
    let before = before
        .iter()
        .map(|effect| (effect.assertion.clone(), effect))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .iter()
        .map(|effect| (effect.assertion.clone(), effect))
        .collect::<BTreeMap<_, _>>();
    for (assertion, expected) in &before {
        let Some(actual) = after.get(assertion) else {
            return Err(ResolutionError::MissingObjectiveComponent(Box::new(
                assertion.clone(),
            )));
        };
        if expected.definition != actual.definition {
            return Err(ResolutionError::ObjectiveDefinitionMismatch {
                assertion: Box::new(assertion.clone()),
                expected: expected.definition.clone(),
                actual: actual.definition.clone(),
            });
        }
    }
    if let Some(assertion) = after
        .keys()
        .find(|assertion| !before.contains_key(*assertion))
    {
        return Err(ResolutionError::UnexpectedObjectiveComponent(Box::new(
            assertion.clone(),
        )));
    }
    Ok(())
}

fn resolution_effects(before: &ReportModeEffects, after: &ReportModeEffects) -> ResolutionEffects {
    ResolutionEffects {
        before: categorize_states(&before.boolean_states),
        after: categorize_states(&after.boolean_states),
        transitions: assertion_transitions(&before.boolean_states, &after.boolean_states),
        before_intents: before.booleans.clone(),
        after_intents: after.booleans.clone(),
        before_objectives: before.objectives.clone(),
        after_objectives: after.objectives.clone(),
        before_assumptions: before.assumptions.clone(),
        after_assumptions: after.assumptions.clone(),
    }
}

fn categorize_states(
    states: &BTreeMap<AssertionSemanticKey, BooleanState>,
) -> CategorizedIntentOutcomes {
    let mut categorized = CategorizedIntentOutcomes::default();
    for (key, state) in states {
        categorized.push(key.clone(), state.category);
    }
    categorized
}

fn assertion_transitions(
    before: &BTreeMap<AssertionSemanticKey, BooleanState>,
    after: &BTreeMap<AssertionSemanticKey, BooleanState>,
) -> Vec<AssertionTransition> {
    before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|assertion| {
            let before = before.get(&assertion).map(|state| state.category);
            let after = after.get(&assertion).map(|state| state.category);
            (before != after).then_some(AssertionTransition {
                assertion,
                before,
                after,
            })
        })
        .collect()
}

fn required_non_regression(
    before: &BTreeMap<AssertionSemanticKey, BooleanState>,
    after: &BTreeMap<AssertionSemanticKey, BooleanState>,
) -> bool {
    for (key, state) in before {
        if !matches!(state.mode, AuthoredIntentMode::Requirement) {
            continue;
        }
        // A derived finding may disappear because the candidate resolved it. An authored
        // requirement, however, must remain present so a provider bug cannot hide it from the
        // hard gate. Newly introduced failing requirements are checked below.
        let Some(candidate) = after.get(key) else {
            if matches!(key, AssertionSemanticKey::Authored(_)) {
                return false;
            }
            continue;
        };
        if required_category_regresses(state.category, candidate.category) {
            return false;
        }
    }
    after.iter().all(|(key, state)| {
        !matches!(state.mode, AuthoredIntentMode::Requirement)
            || before.contains_key(key)
            || !matches!(
                state.category,
                IntentOutcomeCategory::Violated | IntentOutcomeCategory::Unknown
            )
    })
}

fn required_category_regresses(
    before: IntentOutcomeCategory,
    after: IntentOutcomeCategory,
) -> bool {
    match before {
        IntentOutcomeCategory::Violated => matches!(after, IntentOutcomeCategory::Unknown),
        IntentOutcomeCategory::Unknown => matches!(after, IntentOutcomeCategory::Violated),
        IntentOutcomeCategory::Satisfied
        | IntentOutcomeCategory::Waived
        | IntentOutcomeCategory::NotApplicable => matches!(
            after,
            IntentOutcomeCategory::Violated | IntentOutcomeCategory::Unknown
        ),
    }
}

fn objective_vector(
    states: &BTreeMap<AssertionSemanticKey, BooleanState>,
    objectives: &[ResolutionObjectiveEffect],
    current: PlacementPose,
    candidate: PlacementPose,
) -> Result<ResolutionObjectiveVector, ResolutionError> {
    let mut required_violated_or_unknown = 0u32;
    let mut required_unknown = 0u32;
    let mut preference = BTreeMap::<PreferencePriority, (u32, u32)>::new();
    for state in states.values() {
        let failing = matches!(
            state.category,
            IntentOutcomeCategory::Violated | IntentOutcomeCategory::Unknown
        );
        let unknown = state.category == IntentOutcomeCategory::Unknown;
        match state.mode {
            AuthoredIntentMode::Requirement => {
                required_violated_or_unknown =
                    required_violated_or_unknown.saturating_add(u32::from(failing));
                required_unknown = required_unknown.saturating_add(u32::from(unknown));
            }
            AuthoredIntentMode::Preference { priority } => {
                let counts = preference.entry(priority).or_default();
                counts.0 = counts.0.saturating_add(u32::from(failing));
                counts.1 = counts.1.saturating_add(u32::from(unknown));
            }
        }
    }
    let preference_tiers = preference
        .into_iter()
        .rev()
        .map(
            |(priority, (violated_or_unknown, unknown))| PreferenceTierCost {
                priority,
                violated_or_unknown,
                unknown,
            },
        )
        .collect();
    let mut names = BTreeSet::new();
    let mut objective_components = Vec::with_capacity(objectives.len());
    for effect in objectives {
        validate_objective_definition(&effect.assertion, &effect.definition)?;
        validate_objective_value_kind(&effect.assertion, &effect.definition, &effect.observation)?;
        if !names.insert(effect.definition.component.clone()) {
            return Err(ResolutionError::DuplicateObjectiveComponent(
                effect.definition.component.clone(),
            ));
        }
        objective_components.push(ResolutionObjectiveComponent {
            assertion: effect.assertion.clone(),
            definition: effect.definition.clone(),
            observation: effect.observation.clone(),
        });
    }
    objective_components.sort_by_key(objective_component_key);
    Ok(ResolutionObjectiveVector {
        required_violated_or_unknown,
        required_unknown,
        preference_tiers,
        objective_components,
        manhattan_movement_ticks: manhattan_movement(current, candidate),
        quarter_turn_distance: quarter_turn_distance(current.rotation, candidate.rotation),
    })
}

fn compare_ranking_preference_tiers(
    left: &[RankingPreferenceTier],
    right: &[RankingPreferenceTier],
) -> Ordering {
    let canonical = |tiers: &[RankingPreferenceTier]| {
        tiers
            .iter()
            .map(|tier| (tier.priority.0, (tier.violated_or_unknown, tier.unknown)))
            .collect::<BTreeMap<_, _>>()
    };
    let left = canonical(left);
    let right = canonical(right);
    for priority in left
        .keys()
        .chain(right.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .rev()
    {
        let ordering = left
            .get(&priority)
            .copied()
            .unwrap_or_default()
            .cmp(&right.get(&priority).copied().unwrap_or_default());
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn compare_objective_rank(
    left: &ResolutionObjectiveVector,
    right: &ResolutionObjectiveVector,
) -> Ordering {
    resolution_ranking_key(left).cmp(&resolution_ranking_key(right))
}

fn canonical_preference_costs(
    tiers: &[PreferenceTierCost],
) -> BTreeMap<PreferencePriority, (u32, u32)> {
    let mut canonical = BTreeMap::<PreferencePriority, (u32, u32)>::new();
    for tier in tiers {
        let counts = canonical.entry(tier.priority).or_default();
        counts.0 = counts.0.saturating_add(tier.violated_or_unknown);
        counts.1 = counts.1.saturating_add(tier.unknown);
    }
    canonical
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ObjectiveComponentKey {
    priority: Reverse<ObjectivePriority>,
    component: String,
    direction: ObjectiveDirection,
    value_kind: ExactValueKind,
    assertion: AssertionSemanticKey,
}

fn objective_component_key(component: &ResolutionObjectiveComponent) -> ObjectiveComponentKey {
    ObjectiveComponentKey {
        priority: Reverse(component.definition.priority),
        component: component.definition.component.clone(),
        direction: component.definition.direction,
        value_kind: component.definition.value_kind,
        assertion: component.assertion.clone(),
    }
}

fn objective_effect_key(effect: &ResolutionObjectiveEffect) -> ObjectiveComponentKey {
    ObjectiveComponentKey {
        priority: Reverse(effect.definition.priority),
        component: effect.definition.component.clone(),
        direction: effect.definition.direction,
        value_kind: effect.definition.value_kind,
        assertion: effect.assertion.clone(),
    }
}

fn compare_objective_effect_keys(
    left: &ResolutionObjectiveEffect,
    right: &ResolutionObjectiveEffect,
) -> Ordering {
    objective_effect_key(left).cmp(&objective_effect_key(right))
}

fn resolution_ranking_key(vector: &ResolutionObjectiveVector) -> ResolutionRankingKey {
    let preference_tiers = canonical_preference_costs(&vector.preference_tiers)
        .into_iter()
        .rev()
        .map(
            |(priority, (violated_or_unknown, unknown))| RankingPreferenceTier {
                priority: Reverse(priority),
                violated_or_unknown,
                unknown,
            },
        )
        .collect();
    let mut objective_components = vector
        .objective_components
        .iter()
        .map(ranking_objective_component)
        .collect::<Vec<_>>();
    objective_components.sort();
    ResolutionRankingKey {
        required_violated_or_unknown: vector.required_violated_or_unknown,
        required_unknown: vector.required_unknown,
        preference_tiers,
        objective_components,
        manhattan_movement_ticks: vector.manhattan_movement_ticks,
        quarter_turn_distance: vector.quarter_turn_distance,
    }
}

fn ranking_objective_component(
    component: &ResolutionObjectiveComponent,
) -> RankingObjectiveComponent {
    RankingObjectiveComponent {
        priority: Reverse(component.definition.priority),
        component: component.definition.component.clone(),
        direction: component.definition.direction,
        value_kind: component.definition.value_kind,
        assertion: component.assertion.clone(),
        observation: match &component.observation {
            ObjectiveObservation::Known(value) => RankingObjectiveObservation::Known(
                ranking_exact_value(component.definition.direction, value),
            ),
            ObjectiveObservation::NotApplicable => RankingObjectiveObservation::NotApplicable,
            ObjectiveObservation::Unknown(_) => RankingObjectiveObservation::Unknown,
        },
    }
}

fn ranking_exact_value(direction: ObjectiveDirection, value: &ExactValue) -> RankingExactValue {
    match (direction, value) {
        (ObjectiveDirection::Minimize, ExactValue::Length(value)) => {
            RankingExactValue::MinimizeLength(value.ticks())
        }
        (ObjectiveDirection::Maximize, ExactValue::Length(value)) => {
            RankingExactValue::MaximizeLength(Reverse(value.ticks()))
        }
        (ObjectiveDirection::Minimize, ExactValue::Int(value)) => {
            RankingExactValue::MinimizeInt(*value)
        }
        (ObjectiveDirection::Maximize, ExactValue::Int(value)) => {
            RankingExactValue::MaximizeInt(Reverse(*value))
        }
    }
}

fn manhattan_movement(current: PlacementPose, candidate: PlacementPose) -> u64 {
    current
        .position
        .x
        .ticks()
        .abs_diff(candidate.position.x.ticks())
        .saturating_add(
            current
                .position
                .y
                .ticks()
                .abs_diff(candidate.position.y.ticks()),
        )
}

fn quarter_turn_distance(current: QuarterTurn, candidate: QuarterTurn) -> u8 {
    let difference = quarter_turn_index(current).abs_diff(quarter_turn_index(candidate));
    difference.min(4 - difference)
}

const fn quarter_turn_index(rotation: QuarterTurn) -> u8 {
    match rotation {
        QuarterTurn::Deg0 => 0,
        QuarterTurn::Deg90 => 1,
        QuarterTurn::Deg180 => 2,
        QuarterTurn::Deg270 => 3,
    }
}

const fn quarter_turn_from_index(index: u8) -> QuarterTurn {
    match index % 4 {
        0 => QuarterTurn::Deg0,
        1 => QuarterTurn::Deg90,
        2 => QuarterTurn::Deg180,
        _ => QuarterTurn::Deg270,
    }
}

#[cfg(test)]
mod tests {
    use framer_core::{
        AuthoredEntityRef, ClearanceDatum, ClearanceDirection, ExactIntentScope, FramingDefaults,
        Furnishing, FurnishingInstance, IntentAssertion, IntentDomain, IntentSource, MepInstance,
        MepObject, MepObjectKind, Room, RoomUsage, Wall, load_project, save_project,
    };

    use super::*;
    use crate::{
        AssertionScope, AssertionSource, AssumptionIntentRecord, CompiledAssertion,
        GRAPH_CONTRACT_VERSION, GeneratedMemberRef, IntentUnknown, IntentUnknownKind, IntentValue,
        ObjectiveIntentRecord,
    };

    const TARGET_INTENT: &str = "intent-resolution-clearance";
    const GUARD_INTENT: &str = "intent-preserve-clearance";

    fn inches_point(x: i64, y: i64) -> Point2 {
        Point2::new(Length::from_whole_inches(x), Length::from_whole_inches(y))
    }

    fn request() -> ResolutionRequest {
        ResolutionRequest::placement_clearance(AuthoredIntentId::new(TARGET_INTENT))
    }

    fn resolution_revision(model: &BuildingModel, document: u64) -> ResolutionRevision {
        ResolutionRevision::new(
            GraphRevision::for_model(model).unwrap(),
            DocumentRevision::new(document),
        )
    }

    fn objective_priority(value: u16) -> ObjectivePriority {
        ObjectivePriority::new(value).expect("test objective priority must be nonzero")
    }

    fn test_assertion(id: &str) -> CompiledAssertion {
        CompiledAssertion {
            reference: AssertionRef::Authored(AuthoredIntentId::new(id)),
            domain: IntentDomain::Resource,
            scope: AssertionScope::Project,
            participants: Vec::new(),
            source: AssertionSource::User,
            rationale: format!("Synthetic assertion {id}"),
        }
    }

    fn objective_definition(
        component: &str,
        direction: ObjectiveDirection,
        priority: u16,
        value_kind: ExactValueKind,
    ) -> ObjectiveDefinition {
        ObjectiveDefinition {
            component: component.to_owned(),
            direction,
            priority: objective_priority(priority),
            value_kind,
        }
    }

    fn objective_effect(
        id: &str,
        definition: ObjectiveDefinition,
        observation: ObjectiveObservation,
    ) -> ResolutionObjectiveEffect {
        ResolutionObjectiveEffect {
            assertion: AssertionSemanticKey::Authored(AuthoredIntentId::new(id)),
            definition,
            observation,
            evidence: vec![IntentEvidenceRef::Project],
        }
    }

    fn objective_test_vector(effects: &[ResolutionObjectiveEffect]) -> ResolutionObjectiveVector {
        let pose = PlacementPose::new(Point2::default(), QuarterTurn::Deg0);
        objective_vector(&BTreeMap::new(), effects, pose, pose).unwrap()
    }

    fn clearance_model(
        mode: AuthoredIntentMode,
        threshold: Length,
        position: Point2,
    ) -> BuildingModel {
        let mut model = BuildingModel::demo_two_bedroom();
        model.furnishings.push(Furnishing::new(
            "resolution-fixture",
            "Resolution fixture",
            Length::from_feet(2.0),
            Length::from_feet(2.0),
            Length::from_feet(3.0),
        ));
        model.furnishing_instances.push(FurnishingInstance::new(
            "resolution-fixture-1",
            "Resolution fixture 1",
            "resolution-fixture",
            "level-1",
            position,
        ));
        model.intents.push(IntentAssertion {
            id: AuthoredIntentId::new(TARGET_INTENT),
            domain: IntentDomain::SpatialProgram,
            mode,
            scope: ProjectIntentScope::Exact(ExactIntentScope {
                subject: AuthoredEntityRef::FurnishingInstance(ElementId::new(
                    "resolution-fixture-1",
                )),
                participants: vec![AuthoredEntityRef::Room(ElementId::new("room-bed-1"))],
            }),
            expression: IntentExpression::FactPredicate(Predicate::Compare {
                fact: Fact::PlacedObjectClearance {
                    direction: framer_core::ClearanceDirection::Front,
                    datum: framer_core::ClearanceDatum::FootprintFace,
                },
                op: CompareOp::Ge,
                value: FactOperand::LengthLiteral(threshold),
            }),
            source: IntentSource::User,
            rationale: Some("Test explicit resolution".to_owned()),
        });
        model.sort_deterministically();
        model.validate().unwrap();
        model
    }

    fn mep_clearance_model() -> BuildingModel {
        let mut model = BuildingModel::demo_two_bedroom();
        model.mep_objects.push(MepObject::new(
            "resolution-mep",
            "Resolution MEP fixture",
            MepObjectKind::Plumbing,
            Length::from_feet(2.0),
            Length::from_feet(2.0),
            Length::from_feet(3.0),
        ));
        model.mep_instances.push(MepInstance::new(
            "resolution-mep-1",
            "Resolution MEP fixture 1",
            "resolution-mep",
            "level-1",
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        ));
        model.intents.push(IntentAssertion {
            id: AuthoredIntentId::new(TARGET_INTENT),
            domain: IntentDomain::Mep,
            mode: AuthoredIntentMode::Requirement,
            scope: ProjectIntentScope::Exact(ExactIntentScope {
                subject: AuthoredEntityRef::MepInstance(ElementId::new("resolution-mep-1")),
                participants: vec![AuthoredEntityRef::Room(ElementId::new("room-bed-1"))],
            }),
            expression: IntentExpression::FactPredicate(Predicate::Compare {
                fact: Fact::PlacedObjectClearance {
                    direction: ClearanceDirection::Front,
                    datum: ClearanceDatum::FootprintFace,
                },
                op: CompareOp::Ge,
                value: FactOperand::LengthLiteral(Length::from_feet(5.0)),
            }),
            source: IntentSource::User,
            rationale: Some("Keep service access clear".to_owned()),
        });
        model.sort_deterministically();
        model.validate().unwrap();
        model
    }

    fn containment_violation_model() -> BuildingModel {
        let mut model = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::ZERO,
            Point2::new(Length::from_feet(-10.0), Length::from_feet(-10.0)),
        );
        model.intents[0].expression = IntentExpression::FactPredicate(Predicate::Compare {
            fact: Fact::PlacedObjectContainedInRoom,
            op: CompareOp::Eq,
            value: FactOperand::FlagLiteral(true),
        });
        model.sort_deterministically();
        model.validate().unwrap();
        model
    }

    fn observed_clearance(model: &BuildingModel, direction: ClearanceDirection) -> Length {
        let plan = generate_project_plan(model).unwrap();
        let resolved = model.resolved_standards();
        let subject = FactSubject::placed_object_exact(
            PlacedObjectRef::FurnishingInstance(ElementId::new("target")),
            ElementId::new("room"),
        );
        let observation = FactSnapshot::new(model, &resolved, &plan).observe(
            Fact::PlacedObjectClearance {
                direction,
                datum: ClearanceDatum::FootprintFace,
            },
            &subject,
        );
        let FactObservation::Known(FactValue::Length(clearance)) = observation else {
            panic!("expected known clearance, got {observation:?}");
        };
        clearance
    }

    /// Rectangular room + asymmetric obstacle. The four desired-pose directional observations
    /// form an exact integer-tick box: only the center pose at Deg0 satisfies all four.
    fn unique_option_model() -> BuildingModel {
        let defaults = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls = vec![
            Wall::new(
                "wall-south",
                "South wall",
                Length::from_whole_inches(80),
                &defaults,
            )
            .with_placement("level-1", inches_point(0, 0), inches_point(80, 0)),
            Wall::new(
                "wall-east",
                "East wall",
                Length::from_whole_inches(100),
                &defaults,
            )
            .with_placement("level-1", inches_point(80, 0), inches_point(80, 100)),
            Wall::new(
                "wall-north",
                "North wall",
                Length::from_whole_inches(80),
                &defaults,
            )
            .with_placement("level-1", inches_point(80, 100), inches_point(0, 100)),
            Wall::new(
                "wall-west",
                "West wall",
                Length::from_whole_inches(100),
                &defaults,
            )
            .with_placement("level-1", inches_point(0, 100), inches_point(0, 0)),
        ];
        model.rooms.push(Room::new(
            "room",
            "Room",
            RoomUsage::Other,
            "level-1",
            inches_point(40, 50),
        ));
        model.furnishings.extend([
            Furnishing::new(
                "target-family",
                "Target family",
                Length::from_whole_inches(40),
                Length::from_whole_inches(20),
                Length::from_whole_inches(30),
            ),
            Furnishing::new(
                "obstacle-family",
                "Obstacle family",
                Length::from_whole_inches(40),
                Length::from_whole_inches(10),
                Length::from_whole_inches(30),
            ),
        ]);
        let mut target = FurnishingInstance::new(
            "target",
            "Target",
            "target-family",
            "level-1",
            inches_point(40, 50),
        );
        target.rotation = QuarterTurn::Deg90;
        model.furnishing_instances.extend([
            target,
            FurnishingInstance::new(
                "obstacle",
                "Obstacle",
                "obstacle-family",
                "level-1",
                inches_point(40, 25),
            ),
        ]);
        model.sort_deterministically();
        model.validate().unwrap();

        let mut desired = model.clone();
        desired
            .furnishing_instances
            .iter_mut()
            .find(|instance| instance.id == ElementId::new("target"))
            .unwrap()
            .rotation = QuarterTurn::Deg0;
        let predicate = Predicate::All(
            [
                ClearanceDirection::Front,
                ClearanceDirection::Back,
                ClearanceDirection::Left,
                ClearanceDirection::Right,
            ]
            .into_iter()
            .map(|direction| Predicate::Compare {
                fact: Fact::PlacedObjectClearance {
                    direction,
                    datum: ClearanceDatum::FootprintFace,
                },
                op: CompareOp::Ge,
                value: FactOperand::LengthLiteral(observed_clearance(&desired, direction)),
            })
            .collect(),
        );
        model.intents.push(IntentAssertion {
            id: AuthoredIntentId::new(TARGET_INTENT),
            domain: IntentDomain::SpatialProgram,
            mode: AuthoredIntentMode::Requirement,
            scope: ProjectIntentScope::Exact(ExactIntentScope {
                subject: AuthoredEntityRef::FurnishingInstance(ElementId::new("target")),
                participants: vec![AuthoredEntityRef::Room(ElementId::new("room"))],
            }),
            expression: IntentExpression::FactPredicate(predicate),
            source: IntentSource::User,
            rationale: Some("One exact service pose".to_owned()),
        });
        model.sort_deterministically();
        model.validate().unwrap();
        model
    }

    fn required_guard_model() -> BuildingModel {
        let mut model = unique_option_model();
        model
            .intents
            .iter_mut()
            .find(|intent| intent.id == AuthoredIntentId::new(TARGET_INTENT))
            .unwrap()
            .mode = AuthoredIntentMode::Preference {
            priority: PreferencePriority(900),
        };
        let preserved = observed_clearance(&model, ClearanceDirection::Front);
        model.intents.push(IntentAssertion {
            id: AuthoredIntentId::new(GUARD_INTENT),
            domain: IntentDomain::SpatialProgram,
            mode: AuthoredIntentMode::Requirement,
            scope: ProjectIntentScope::Exact(ExactIntentScope {
                subject: AuthoredEntityRef::FurnishingInstance(ElementId::new("target")),
                participants: vec![AuthoredEntityRef::Room(ElementId::new("room"))],
            }),
            expression: IntentExpression::FactPredicate(Predicate::Compare {
                fact: Fact::PlacedObjectClearance {
                    direction: ClearanceDirection::Front,
                    datum: ClearanceDatum::FootprintFace,
                },
                op: CompareOp::Le,
                value: FactOperand::LengthLiteral(preserved),
            }),
            source: IntentSource::User,
            rationale: Some("Preserve the required service-datum contact".to_owned()),
        });
        model.sort_deterministically();
        model.validate().unwrap();
        model
    }

    fn authored_outcome(report: &IntentReport, id: &str) -> IntentOutcomeCategory {
        let Some(IntentRecord::Boolean(record)) =
            report.record(&AssertionRef::Authored(AuthoredIntentId::new(id)))
        else {
            panic!("missing authored boolean outcome {id}");
        };
        IntentOutcomeCategory::from_outcome(&record.outcome)
    }

    #[test]
    fn public_provider_rejects_missing_satisfied_and_violated_containment_targets() {
        let violated = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::from_feet(5.0),
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        );
        let missing = ResolutionRequest::placement_clearance(AuthoredIntentId::new("missing"));
        assert!(matches!(
            generate_resolution_options(
                &violated,
                resolution_revision(&violated, 1),
                &missing,
                &mut ResolutionCache::default(),
            ),
            Err(ResolutionError::TargetMissing(id)) if id == AuthoredIntentId::new("missing")
        ));

        let satisfied = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::ZERO,
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        );
        assert!(matches!(
            generate_resolution_options(
                &satisfied,
                resolution_revision(&satisfied, 2),
                &request(),
                &mut ResolutionCache::default(),
            ),
            Err(ResolutionError::TargetNotViolated(id))
                if id == AuthoredIntentId::new(TARGET_INTENT)
        ));

        let containment = containment_violation_model();
        let report = analyze_project(&containment)
            .unwrap()
            .intent_report
            .unwrap();
        assert_eq!(
            authored_outcome(&report, TARGET_INTENT),
            IntentOutcomeCategory::Violated
        );
        assert!(matches!(
            generate_resolution_options(
                &containment,
                resolution_revision(&containment, 3),
                &request(),
                &mut ResolutionCache::default(),
            ),
            Err(ResolutionError::TargetNotPlacementClearance(id))
                if id == AuthoredIntentId::new(TARGET_INTENT)
        ));
    }

    #[test]
    fn uncached_provider_rejects_an_intent_report_from_another_graph_revision() {
        let model = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::from_feet(5.0),
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        );
        let report_revision = GraphRevision::for_model(&model).unwrap();
        let supplied = GraphRevision::for_model_with_contract(
            &model,
            GRAPH_CONTRACT_VERSION.saturating_add(1),
        )
        .unwrap();
        assert_ne!(report_revision, supplied);

        assert!(matches!(
            generate_uncached(
                &model,
                ResolutionRevision::new(supplied, DocumentRevision::new(1)),
                &request(),
            ),
            Err(ResolutionError::ReportRevisionMismatch {
                report,
                supplied: actual_supplied,
            }) if report == report_revision && actual_supplied == supplied
        ));
    }

    #[test]
    fn placement_target_rejects_a_non_placeable_authored_subject() {
        let mut model = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::from_feet(5.0),
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        );
        let report = analyze_project(&model).unwrap().intent_report.unwrap();
        let authored = model
            .intents
            .iter_mut()
            .find(|intent| intent.id == AuthoredIntentId::new(TARGET_INTENT))
            .unwrap();
        let ProjectIntentScope::Exact(scope) = &mut authored.scope;
        scope.subject = AuthoredEntityRef::Room(ElementId::new("room-bed-1"));

        assert!(matches!(
            placement_clearance_target(&model, &report, &AuthoredIntentId::new(TARGET_INTENT)),
            Err(ResolutionError::InvalidTargetScope(id))
                if id == AuthoredIntentId::new(TARGET_INTENT)
        ));
    }

    #[test]
    fn placement_provider_rejects_a_target_room_without_a_closed_boundary() {
        let mut model = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::from_feet(5.0),
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        );
        let report = analyze_project(&model).unwrap().intent_report.unwrap();
        let target_room = model
            .rooms
            .iter_mut()
            .find(|room| room.id == ElementId::new("room-bed-1"))
            .unwrap();
        target_room.level = ElementId::new("level-without-walls");

        assert!(matches!(
            generate_placement_clearance_options(
                &model,
                &report,
                ResolutionRevision::new(report.revision(), DocumentRevision::new(1)),
                &AuthoredIntentId::new(TARGET_INTENT),
                MAX_FULL_CANDIDATE_ANALYSES,
            ),
            Err(ResolutionError::MissingRoomBoundary(id))
                if id == AuthoredIntentId::new(TARGET_INTENT)
        ));
    }

    #[test]
    fn mode_effect_adapter_rejects_duplicate_semantic_assertions() {
        let model = BuildingModel::new();
        let report_revision = GraphRevision::for_model(&model).unwrap();
        let alternate_revision =
            GraphRevision::for_model_with_contract(&model, GRAPH_CONTRACT_VERSION + 1).unwrap();
        let assertion = |revision| CompiledAssertion {
            reference: AssertionRef::Derived(DerivedAssertionId::new(
                revision,
                DerivedAssertionProvider::Core,
                DerivedAssertionSource::Project,
                DerivedAssertionRole::DrivingDimension,
            )),
            domain: IntentDomain::Resource,
            scope: AssertionScope::Project,
            participants: Vec::new(),
            source: AssertionSource::Project,
            rationale: "Synthetic duplicate semantic assertion".to_owned(),
        };
        let record = |revision| {
            IntentRecord::Assumption(AssumptionIntentRecord {
                assertion: assertion(revision),
                premise: AssumptionPremise {
                    label: "duplicate premise".to_owned(),
                },
                evidence: AssumptionEvidence::Known(IntentValue::Flag(true)),
                provenance: vec![IntentEvidenceRef::Project],
            })
        };
        let report = IntentReport::from_parts(
            report_revision,
            vec![record(report_revision), record(alternate_revision)],
            Vec::new(),
        );
        assert_eq!(report.records().len(), 2);

        assert!(matches!(
            report_mode_effects(&report),
            Err(ResolutionError::InconsistentSemanticAssertion(_))
        ));
    }

    #[test]
    fn mep_clearance_provider_and_structural_capability_are_explicit() {
        let model = mep_clearance_model();
        let revision = resolution_revision(&model, 4);
        let options = generate_resolution_options(
            &model,
            revision,
            &request(),
            &mut ResolutionCache::default(),
        )
        .unwrap();
        let option = options.options().first().expect("MEP options");
        assert!(matches!(
            option.patch().target,
            PlacementTarget::MepInstance(ref id) if id == &ElementId::new("resolution-mep-1")
        ));
        stage_resolution_option(&model, option, revision).unwrap();

        assert_eq!(
            resolution_capability(ResolutionCapability::PlacementClearance),
            ResolutionCapabilityAvailability::Available
        );
        let ResolutionCapabilityAvailability::Unavailable { reason } =
            resolution_capability(ResolutionCapability::StructuralAlternatives)
        else {
            panic!("structural alternatives must remain explicitly unavailable");
        };
        assert_eq!(reason, STRUCTURAL_RESOLUTION_UNAVAILABLE_REASON);
        assert!(!reason.trim().is_empty());
    }

    #[test]
    fn provider_filters_target_preference_improvement_that_regresses_a_requirement() {
        let unguarded = unique_option_model();
        let unguarded_revision = resolution_revision(&unguarded, 10);
        let unguarded_options = generate_resolution_options(
            &unguarded,
            unguarded_revision,
            &request(),
            &mut ResolutionCache::default(),
        )
        .unwrap();
        let replacement = unguarded_options.options()[0].patch().replacement;

        let guarded = required_guard_model();
        let guarded_report = analyze_project(&guarded).unwrap().intent_report.unwrap();
        assert_eq!(
            authored_outcome(&guarded_report, TARGET_INTENT),
            IntentOutcomeCategory::Violated
        );
        assert_eq!(
            authored_outcome(&guarded_report, GUARD_INTENT),
            IntentOutcomeCategory::Satisfied
        );

        let placement = PlacementTarget::FurnishingInstance(ElementId::new("target"));
        let expected = current_placement_pose(&guarded, &placement).unwrap();
        let candidate = stage_placement_patch(
            &guarded,
            &PlacementPatch::new(placement, expected, replacement),
        )
        .unwrap();
        let candidate_report = analyze_project(candidate.model())
            .unwrap()
            .intent_report
            .unwrap();
        assert_eq!(
            authored_outcome(&candidate_report, TARGET_INTENT),
            IntentOutcomeCategory::Satisfied
        );
        assert_eq!(
            authored_outcome(&candidate_report, GUARD_INTENT),
            IntentOutcomeCategory::Violated
        );

        let filtered = generate_resolution_options(
            &guarded,
            resolution_revision(&guarded, 11),
            &request(),
            &mut ResolutionCache::default(),
        )
        .unwrap();
        assert!(filtered.options().is_empty());
        assert!(filtered.search.feasible_candidates > 0);
        assert!(filtered.search.fully_analyzed_candidates > 0);
    }

    #[test]
    fn several_options_are_unique_stably_ordered_and_fully_evidenced() {
        let model = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::from_feet(5.0),
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        );
        let revision = resolution_revision(&model, 7);
        let first = generate_resolution_options(
            &model,
            revision,
            &request(),
            &mut ResolutionCache::default(),
        )
        .unwrap();
        let second = generate_resolution_options(
            &model,
            revision,
            &request(),
            &mut ResolutionCache::default(),
        )
        .unwrap();

        assert!(first.options().len() > 1, "{:#?}", first.search());
        assert!(first.options().len() <= MAX_OPTIONS);
        assert_eq!(first, second);
        assert!(first.search.fact_measurements <= MAX_FACT_MEASUREMENTS);
        assert!(first.search.fully_analyzed_candidates <= first.search.candidate_analysis_cap);
        assert!(first.search.fully_analyzed_candidates <= first.search.feasible_candidates);
        assert!(!first.search.candidate_analysis_truncated);
        let ordering = first
            .options()
            .iter()
            .map(|option| {
                (
                    resolution_ranking_key(&option.objective),
                    PlacementPatchSemanticKey::from_patch(&option.patch),
                )
            })
            .collect::<Vec<_>>();
        assert!(ordering.windows(2).all(|pair| pair[0] < pair[1]));

        let current_report = analyze_project(&model).unwrap().intent_report.unwrap();
        for option in first.options() {
            let staged = stage_resolution_option(&model, option, revision).unwrap();
            let candidate_report = analyze_project(&staged).unwrap().intent_report.unwrap();
            let current_effects = report_mode_effects(&current_report).unwrap();
            let candidate_effects = report_mode_effects(&candidate_report).unwrap();
            assert_eq!(option.effects.before_intents, current_effects.booleans);
            assert_eq!(option.effects.after_intents, candidate_effects.booleans);
            assert_eq!(option.effects.before_objectives, current_effects.objectives);
            assert_eq!(
                option.effects.after_objectives,
                candidate_effects.objectives
            );
            assert_eq!(
                option.effects.before_assumptions,
                current_effects.assumptions
            );
            assert_eq!(
                option.effects.after_assumptions,
                candidate_effects.assumptions
            );
            assert_eq!(
                option.candidate_revision,
                GraphRevision::for_model(&staged).unwrap()
            );
            assert!(option.effects.after_intents.iter().any(|effect| {
                effect.assertion
                    == AssertionSemanticKey::Authored(AuthoredIntentId::new(TARGET_INTENT))
                    && effect.outcome == IntentOutcomeCategory::Satisfied
                    && !effect.evidence.is_empty()
            }));
        }
    }

    #[test]
    fn candidate_analysis_summary_reports_cap_truncation() {
        let model = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::from_feet(5.0),
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        );
        let revision = resolution_revision(&model, 8);
        let report = analyze_project(&model).unwrap().intent_report.unwrap();
        let options = generate_placement_clearance_options(
            &model,
            &report,
            revision,
            &AuthoredIntentId::new(TARGET_INTENT),
            1,
        )
        .unwrap();

        assert!(
            options.search.feasible_candidates > 1,
            "{:#?}",
            options.search
        );
        assert_eq!(options.search.fully_analyzed_candidates, 1);
        assert_eq!(options.search.candidate_analysis_cap, 1);
        assert!(options.search.candidate_analysis_truncated);
    }

    #[test]
    fn unique_option_stages_current_and_rejects_both_stale_revisions_and_pose() {
        let model = unique_option_model();
        let revision = resolution_revision(&model, 41);
        let mut cache = ResolutionCache::default();
        let first = generate_resolution_options(&model, revision, &request(), &mut cache).unwrap();
        assert_eq!(first.options().len(), 1, "{first:#?}");
        let option = first.options()[0].clone();
        assert_eq!(cache.stats().misses, 1);

        let hit = generate_resolution_options(&model, revision, &request(), &mut cache).unwrap();
        assert_eq!(hit, first);
        assert_eq!(cache.stats().hits, 1);

        let newer_document = ResolutionRevision::new(revision.graph, DocumentRevision::new(42));
        let rebound =
            generate_resolution_options(&model, newer_document, &request(), &mut cache).unwrap();
        assert_eq!(rebound.options().len(), 1);
        assert_eq!(rebound.origin(), newer_document);
        assert_eq!(rebound.options()[0].origin(), newer_document);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 2);
        assert_eq!(cache.stats().rebinds, 1);

        let source = model.clone();
        let staged = stage_resolution_option(&model, &option, revision).unwrap();
        assert_eq!(model, source);
        staged.validate().unwrap();
        let encoded = save_project(&staged).unwrap();
        assert_eq!(load_project(&encoded).unwrap(), staged);
        let mut applied = model.clone();
        apply_resolution_option(&mut applied, &option, revision).unwrap();
        assert_eq!(applied, staged);

        assert!(matches!(
            stage_resolution_option(
                &model,
                &option,
                ResolutionRevision::new(option.candidate_revision(), revision.document),
            ),
            Err(ResolutionError::StaleGraphRevision { .. })
        ));
        assert!(matches!(
            stage_resolution_option(&model, &option, newer_document),
            Err(ResolutionError::StaleDocumentRevision { .. })
        ));

        let mut changed = model.clone();
        changed
            .furnishing_instances
            .iter_mut()
            .find(|instance| instance.id == ElementId::new("target"))
            .unwrap()
            .position
            .x += Length::from_ticks(1);
        changed.sort_deterministically();
        changed.validate().unwrap();
        let changed_revision = resolution_revision(&changed, revision.document.value());
        let mut forged = option.clone();
        forged.origin = changed_revision;
        assert!(matches!(
            stage_resolution_option(&changed, &forged, changed_revision),
            Err(ResolutionError::Patch(
                PlacementPatchError::StaleExpectedPose { .. }
            ))
        ));

        let mut forged_patch = option.clone();
        forged_patch.patch.replacement.position.x += Length::from_ticks(1);
        assert!(matches!(
            stage_resolution_option(&model, &forged_patch, revision),
            Err(ResolutionError::CandidateRevisionMismatch { .. })
        ));
    }

    #[test]
    fn graph_change_invalidates_cached_options_instead_of_rebinding_them() {
        let model = unique_option_model();
        let revision = resolution_revision(&model, 51);
        let mut cache = ResolutionCache::default();
        let first = generate_resolution_options(&model, revision, &request(), &mut cache).unwrap();
        let old_option = first.options()[0].clone();

        let mut changed = model.clone();
        changed
            .furnishing_instances
            .iter_mut()
            .find(|instance| instance.id == ElementId::new("target"))
            .unwrap()
            .position
            .x += Length::from_ticks(1);
        changed.sort_deterministically();
        changed.validate().unwrap();
        let changed_revision = resolution_revision(&changed, 52);
        let regenerated =
            generate_resolution_options(&changed, changed_revision, &request(), &mut cache)
                .unwrap();

        assert_eq!(cache.stats().misses, 2);
        assert_eq!(cache.stats().hits, 0);
        assert_eq!(cache.stats().rebinds, 1);
        assert_eq!(regenerated.origin(), changed_revision);
        assert!(
            !regenerated.options().is_empty(),
            "{:#?}",
            regenerated.search()
        );
        assert!(
            regenerated
                .options()
                .iter()
                .all(|option| option.origin() == changed_revision)
        );
        assert_ne!(
            regenerated.options()[0].patch().before(),
            old_option.patch().before()
        );
        assert!(matches!(
            stage_resolution_option(&changed, &old_option, changed_revision),
            Err(ResolutionError::StaleGraphRevision { .. })
        ));
    }

    #[test]
    fn bounded_search_can_return_no_options_without_claiming_infeasibility() {
        let model = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::from_feet(1_000.0),
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        );
        let options = generate_resolution_options(
            &model,
            resolution_revision(&model, 9),
            &request(),
            &mut ResolutionCache::default(),
        )
        .unwrap();
        assert!(options.options().is_empty());
        assert_eq!(
            options.search.outcome,
            ResolutionSearchOutcome::BoundedNoOptions
        );
        assert!(options.search.fact_measurements <= MAX_FACT_MEASUREMENTS);
        assert_eq!(options.search.fully_analyzed_candidates, 0);
    }

    #[test]
    fn fact_measurement_summary_reports_atomic_budget_truncation() {
        let mut model = clearance_model(
            AuthoredIntentMode::Requirement,
            Length::from_feet(1_000.0),
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        );
        let mut predicates = Vec::new();
        for direction in [
            ClearanceDirection::Left,
            ClearanceDirection::Right,
            ClearanceDirection::Front,
            ClearanceDirection::Back,
            ClearanceDirection::Around,
        ] {
            for datum in [ClearanceDatum::Centerline, ClearanceDatum::FootprintFace] {
                predicates.push(Predicate::Compare {
                    fact: Fact::PlacedObjectClearance { direction, datum },
                    op: CompareOp::Ge,
                    value: FactOperand::LengthLiteral(Length::from_feet(1_000.0)),
                });
            }
        }
        let authored = model
            .intents
            .iter_mut()
            .find(|intent| intent.id == AuthoredIntentId::new(TARGET_INTENT))
            .unwrap();
        authored.expression = IntentExpression::FactPredicate(Predicate::All(predicates));
        model.sort_deterministically();
        model.validate().unwrap();

        let options = generate_resolution_options(
            &model,
            resolution_revision(&model, 91),
            &request(),
            &mut ResolutionCache::default(),
        )
        .unwrap();
        let IntentExpression::FactPredicate(predicate) = &model.intents[0].expression;
        let facts_per_candidate = predicate_facts(predicate).len().saturating_add(1) as u32;

        assert!(options.search.fact_measurement_truncated);
        assert!(options.search.fact_measurements <= options.search.measurement_cap);
        assert!(
            options
                .search
                .fact_measurements
                .saturating_add(facts_per_candidate)
                > options.search.measurement_cap,
            "the last all-or-nothing fact batch must be rejected before exceeding the cap",
        );
    }

    #[test]
    fn authored_requirement_disappearance_fails_closed_but_derived_resolution_may_disappear() {
        let state = BooleanState {
            mode: AuthoredIntentMode::Requirement,
            category: IntentOutcomeCategory::Satisfied,
        };
        let authored = AssertionSemanticKey::Authored(AuthoredIntentId::new("authored-guard"));
        assert!(!required_non_regression(
            &BTreeMap::from([(authored, state)]),
            &BTreeMap::new(),
        ));

        let derived = AssertionSemanticKey::Derived(DerivedAssertionSemanticKey {
            provider: DerivedAssertionProvider::Core,
            source: DerivedAssertionSemanticSource::Project,
            role: DerivedAssertionRole::DrivingDimension,
        });
        assert!(required_non_regression(
            &BTreeMap::from([(derived, state)]),
            &BTreeMap::new(),
        ));
    }

    #[test]
    fn required_non_regression_precedes_preference_ranking() {
        let target = AssertionSemanticKey::Authored(AuthoredIntentId::new("target"));
        let guard = AssertionSemanticKey::Authored(AuthoredIntentId::new("guard"));
        let existing = AssertionSemanticKey::Authored(AuthoredIntentId::new("existing"));
        let preference = AssertionSemanticKey::Authored(AuthoredIntentId::new("preference"));
        let required = AuthoredIntentMode::Requirement;
        let preferred = AuthoredIntentMode::Preference {
            priority: PreferencePriority(500),
        };
        let before = BTreeMap::from([
            (
                target.clone(),
                BooleanState {
                    mode: required,
                    category: IntentOutcomeCategory::Violated,
                },
            ),
            (
                guard.clone(),
                BooleanState {
                    mode: required,
                    category: IntentOutcomeCategory::Satisfied,
                },
            ),
            (
                existing.clone(),
                BooleanState {
                    mode: required,
                    category: IntentOutcomeCategory::Violated,
                },
            ),
            (
                preference.clone(),
                BooleanState {
                    mode: preferred,
                    category: IntentOutcomeCategory::Violated,
                },
            ),
        ]);
        let traded = BTreeMap::from([
            (
                target.clone(),
                BooleanState {
                    mode: required,
                    category: IntentOutcomeCategory::Satisfied,
                },
            ),
            (
                guard.clone(),
                BooleanState {
                    mode: required,
                    category: IntentOutcomeCategory::Violated,
                },
            ),
            (
                existing.clone(),
                BooleanState {
                    mode: required,
                    category: IntentOutcomeCategory::Satisfied,
                },
            ),
            (
                preference.clone(),
                BooleanState {
                    mode: preferred,
                    category: IntentOutcomeCategory::Satisfied,
                },
            ),
        ]);
        let preserved = BTreeMap::from([
            (
                target.clone(),
                BooleanState {
                    mode: required,
                    category: IntentOutcomeCategory::Satisfied,
                },
            ),
            (
                guard.clone(),
                BooleanState {
                    mode: required,
                    category: IntentOutcomeCategory::Satisfied,
                },
            ),
            (
                existing.clone(),
                BooleanState {
                    mode: required,
                    category: IntentOutcomeCategory::Violated,
                },
            ),
            (
                preference.clone(),
                BooleanState {
                    mode: preferred,
                    category: IntentOutcomeCategory::Violated,
                },
            ),
        ]);
        let pose = PlacementPose::new(Point2::default(), QuarterTurn::Deg0);

        assert!(!required_non_regression(&before, &traded));
        assert!(required_non_regression(&before, &preserved));
        let mut missing_authored_requirement = preserved.clone();
        missing_authored_requirement.remove(&guard);
        assert!(
            !required_non_regression(&before, &missing_authored_requirement),
            "a missing authored requirement must fail closed rather than disappear from ranking",
        );
        assert!(
            compare_objective_rank(
                &objective_vector(&traded, &[], pose, pose).unwrap(),
                &objective_vector(&preserved, &[], pose, pose).unwrap(),
            ) == Ordering::Less,
            "the hard gate, not soft ranking, must reject the required trade"
        );
    }

    #[test]
    fn derived_assertion_semantic_keys_strip_only_revisions() {
        let model = BuildingModel::new();
        let first_revision =
            GraphRevision::for_model_with_contract(&model, GRAPH_CONTRACT_VERSION).unwrap();
        let second_revision =
            GraphRevision::for_model_with_contract(&model, GRAPH_CONTRACT_VERSION + 1).unwrap();
        let make = |revision| {
            AssertionRef::Derived(DerivedAssertionId::new(
                revision,
                DerivedAssertionProvider::Solver,
                DerivedAssertionSource::GeneratedMember(GeneratedMemberRef::new(
                    revision,
                    AuthoredEntityRef::Wall(ElementId::new("wall")),
                    "stud-1",
                    MemberKind::CommonStud,
                )),
                DerivedAssertionRole::Diagnostic {
                    provider: crate::DiagnosticProvider::Solver,
                    code: "test.finding".to_owned(),
                    ordinal: 3,
                },
            ))
        };
        let first = AssertionSemanticKey::from_assertion(&make(first_revision));
        let second = AssertionSemanticKey::from_assertion(&make(second_revision));

        assert_eq!(first, second);
        let AssertionSemanticKey::Derived(DerivedAssertionSemanticKey {
            provider: DerivedAssertionProvider::Solver,
            source:
                DerivedAssertionSemanticSource::GeneratedMember(GeneratedMemberSemanticRef {
                    host,
                    member_id,
                    kind: MemberKind::CommonStud,
                }),
            role:
                DerivedAssertionRole::Diagnostic {
                    code, ordinal: 3, ..
                },
        }) = first
        else {
            panic!("semantic key did not preserve provider/source/role");
        };
        assert_eq!(host, AuthoredEntityRef::Wall(ElementId::new("wall")));
        assert_eq!(member_id, "stud-1");
        assert_eq!(code, "test.finding");
    }

    #[test]
    fn mode_effects_capture_exact_objectives_and_assumptions_before_and_after() {
        let revision = GraphRevision::for_model(&BuildingModel::new()).unwrap();
        let objective_assertion = test_assertion("objective-cost");
        let assumption_assertion = test_assertion("assumption-budget");
        let definition = objective_definition(
            "material-cost",
            ObjectiveDirection::Minimize,
            500,
            ExactValueKind::Int,
        );
        let before_report = IntentReport::from_parts(
            revision,
            vec![
                IntentRecord::Objective(ObjectiveIntentRecord {
                    assertion: objective_assertion.clone(),
                    objective: definition.clone(),
                    observation: ObjectiveObservation::Known(ExactValue::Int(12)),
                    evidence: vec![IntentEvidenceRef::Project],
                }),
                IntentRecord::Assumption(AssumptionIntentRecord {
                    assertion: assumption_assertion.clone(),
                    premise: AssumptionPremise {
                        label: "available budget".to_owned(),
                    },
                    evidence: AssumptionEvidence::Known(IntentValue::Int(20)),
                    provenance: vec![IntentEvidenceRef::Project],
                }),
            ],
            Vec::new(),
        );
        let unavailable = IntentUnknown {
            kind: IntentUnknownKind::MissingInput,
            detail: "budget was removed".to_owned(),
        };
        let after_report = IntentReport::from_parts(
            revision,
            vec![
                IntentRecord::Objective(ObjectiveIntentRecord {
                    assertion: objective_assertion,
                    objective: definition.clone(),
                    observation: ObjectiveObservation::Known(ExactValue::Int(8)),
                    evidence: vec![IntentEvidenceRef::Project],
                }),
                IntentRecord::Assumption(AssumptionIntentRecord {
                    assertion: assumption_assertion,
                    premise: AssumptionPremise {
                        label: "available budget".to_owned(),
                    },
                    evidence: AssumptionEvidence::Unavailable(unavailable.clone()),
                    provenance: vec![IntentEvidenceRef::Project],
                }),
            ],
            Vec::new(),
        );

        let before = report_mode_effects(&before_report).unwrap();
        let after = report_mode_effects(&after_report).unwrap();
        validate_objective_contract(&before.objectives, &after.objectives).unwrap();
        let effects = resolution_effects(&before, &after);

        assert!(effects.before_intents.is_empty());
        assert!(effects.after_intents.is_empty());
        assert_eq!(effects.before_objectives.len(), 1);
        assert_eq!(effects.before_objectives[0].definition, definition);
        assert_eq!(
            effects.before_objectives[0].observation,
            ObjectiveObservation::Known(ExactValue::Int(12))
        );
        assert_eq!(
            effects.after_objectives[0].observation,
            ObjectiveObservation::Known(ExactValue::Int(8))
        );
        assert_eq!(
            effects.before_objectives[0].evidence,
            vec![IntentEvidenceRef::Project]
        );
        assert_eq!(
            effects.before_assumptions[0].evidence,
            AssumptionEvidence::Known(IntentValue::Int(20))
        );
        assert_eq!(
            effects.after_assumptions[0].evidence,
            AssumptionEvidence::Unavailable(unavailable)
        );
        assert_eq!(
            effects.after_assumptions[0].provenance,
            vec![IntentEvidenceRef::Project]
        );
    }

    #[test]
    fn exact_objectives_rank_in_declared_direction_for_length_and_int() {
        for (kind, smaller, larger) in [
            (
                ExactValueKind::Length,
                ExactValue::Length(Length::from_whole_inches(4)),
                ExactValue::Length(Length::from_whole_inches(9)),
            ),
            (ExactValueKind::Int, ExactValue::Int(4), ExactValue::Int(9)),
        ] {
            let minimize =
                objective_definition("quantity", ObjectiveDirection::Minimize, 100, kind);
            let maximize =
                objective_definition("quantity", ObjectiveDirection::Maximize, 100, kind);
            let smaller_min = objective_test_vector(&[objective_effect(
                "objective-quantity",
                minimize.clone(),
                ObjectiveObservation::Known(smaller.clone()),
            )]);
            let larger_min = objective_test_vector(&[objective_effect(
                "objective-quantity",
                minimize,
                ObjectiveObservation::Known(larger.clone()),
            )]);
            assert_eq!(
                compare_objective_rank(&smaller_min, &larger_min),
                Ordering::Less
            );

            let smaller_max = objective_test_vector(&[objective_effect(
                "objective-quantity",
                maximize.clone(),
                ObjectiveObservation::Known(smaller),
            )]);
            let larger_max = objective_test_vector(&[objective_effect(
                "objective-quantity",
                maximize,
                ObjectiveObservation::Known(larger),
            )]);
            assert_eq!(
                compare_objective_rank(&larger_max, &smaller_max),
                Ordering::Less
            );
        }
    }

    #[test]
    fn objective_status_priority_and_unknown_exposure_are_canonical() {
        let high = objective_definition(
            "high-tier",
            ObjectiveDirection::Minimize,
            500,
            ExactValueKind::Int,
        );
        let low = objective_definition(
            "low-tier",
            ObjectiveDirection::Minimize,
            100,
            ExactValueKind::Int,
        );
        let high_bad_low_good = objective_test_vector(&[
            objective_effect(
                "objective-low",
                low.clone(),
                ObjectiveObservation::Known(ExactValue::Int(0)),
            ),
            objective_effect(
                "objective-high",
                high.clone(),
                ObjectiveObservation::Known(ExactValue::Int(10)),
            ),
        ]);
        let mut high_good_low_bad = objective_test_vector(&[
            objective_effect(
                "objective-high",
                high.clone(),
                ObjectiveObservation::Known(ExactValue::Int(5)),
            ),
            objective_effect(
                "objective-low",
                low,
                ObjectiveObservation::Known(ExactValue::Int(100)),
            ),
        ]);
        high_good_low_bad.manhattan_movement_ticks = u64::MAX;
        assert_eq!(
            compare_objective_rank(&high_good_low_bad, &high_bad_low_good),
            Ordering::Less,
            "the stronger objective tier must dominate lower tiers and movement"
        );
        assert_eq!(
            high_bad_low_good.objective_components()[0]
                .definition()
                .component,
            "high-tier"
        );

        let status_definition = objective_definition(
            "availability",
            ObjectiveDirection::Minimize,
            50,
            ExactValueKind::Int,
        );
        let known = objective_test_vector(&[objective_effect(
            "objective-status",
            status_definition.clone(),
            ObjectiveObservation::Known(ExactValue::Int(1)),
        )]);
        let not_applicable = objective_test_vector(&[objective_effect(
            "objective-status",
            status_definition.clone(),
            ObjectiveObservation::NotApplicable,
        )]);
        let first_unknown = ObjectiveObservation::Unknown(IntentUnknown {
            kind: IntentUnknownKind::MissingInput,
            detail: "first diagnostic detail".to_owned(),
        });
        let second_unknown = ObjectiveObservation::Unknown(IntentUnknown {
            kind: IntentUnknownKind::EvaluationUnavailable,
            detail: "different diagnostic detail".to_owned(),
        });
        let unknown_a = objective_test_vector(&[objective_effect(
            "objective-status",
            status_definition.clone(),
            first_unknown.clone(),
        )]);
        let unknown_b = objective_test_vector(&[objective_effect(
            "objective-status",
            status_definition,
            second_unknown,
        )]);
        assert_eq!(
            compare_objective_rank(&known, &not_applicable),
            Ordering::Less
        );
        assert_eq!(
            compare_objective_rank(&not_applicable, &unknown_a),
            Ordering::Less
        );
        assert_ne!(
            unknown_a, unknown_b,
            "exact public observations remain visible"
        );
        assert_eq!(
            resolution_ranking_key(&unknown_a),
            resolution_ranking_key(&unknown_b),
            "unknown diagnostic prose is evidence, not a score"
        );
        assert_eq!(
            unknown_a.objective_components()[0].observation(),
            &first_unknown
        );
    }

    #[test]
    fn malformed_objective_contracts_reject_before_ranking() {
        let definition = objective_definition(
            "cost",
            ObjectiveDirection::Minimize,
            100,
            ExactValueKind::Int,
        );
        let baseline = objective_effect(
            "objective-cost",
            definition.clone(),
            ObjectiveObservation::Known(ExactValue::Int(3)),
        );
        assert!(matches!(
            validate_objective_contract(std::slice::from_ref(&baseline), &[]),
            Err(ResolutionError::MissingObjectiveComponent(_))
        ));
        assert!(matches!(
            validate_objective_contract(&[], std::slice::from_ref(&baseline)),
            Err(ResolutionError::UnexpectedObjectiveComponent(_))
        ));

        let changed = objective_effect(
            "objective-cost",
            objective_definition(
                "cost",
                ObjectiveDirection::Maximize,
                100,
                ExactValueKind::Int,
            ),
            ObjectiveObservation::Known(ExactValue::Int(3)),
        );
        assert!(matches!(
            validate_objective_contract(std::slice::from_ref(&baseline), &[changed]),
            Err(ResolutionError::ObjectiveDefinitionMismatch { .. })
        ));

        let pose = PlacementPose::new(Point2::default(), QuarterTurn::Deg0);
        let wrong_kind = objective_effect(
            "objective-cost",
            definition.clone(),
            ObjectiveObservation::Known(ExactValue::Length(Length::from_whole_inches(3))),
        );
        assert!(matches!(
            objective_vector(&BTreeMap::new(), &[wrong_kind], pose, pose),
            Err(ResolutionError::ObjectiveValueKindMismatch { .. })
        ));

        let unnamed = objective_effect(
            "objective-unnamed",
            objective_definition(
                " \t ",
                ObjectiveDirection::Minimize,
                100,
                ExactValueKind::Int,
            ),
            ObjectiveObservation::Known(ExactValue::Int(3)),
        );
        assert!(matches!(
            objective_vector(&BTreeMap::new(), &[unnamed], pose, pose),
            Err(ResolutionError::InvalidObjectiveComponentName { .. })
        ));

        let duplicate_name = vec![
            baseline,
            objective_effect(
                "objective-other",
                definition,
                ObjectiveObservation::Known(ExactValue::Int(4)),
            ),
        ];
        assert!(matches!(
            objective_vector(&BTreeMap::new(), &duplicate_name, pose, pose),
            Err(ResolutionError::DuplicateObjectiveComponent(component))
                if component == "cost"
        ));
    }

    #[test]
    fn objective_canonicalization_and_private_ranking_key_obey_total_order_laws() {
        let first = objective_effect(
            "objective-a",
            objective_definition("a", ObjectiveDirection::Minimize, 100, ExactValueKind::Int),
            ObjectiveObservation::Known(ExactValue::Int(1)),
        );
        let second = objective_effect(
            "objective-z",
            objective_definition(
                "z",
                ObjectiveDirection::Maximize,
                500,
                ExactValueKind::Length,
            ),
            ObjectiveObservation::Known(ExactValue::Length(Length::from_whole_inches(8))),
        );
        let canonical = objective_test_vector(&[first.clone(), second.clone()]);
        let shuffled = objective_test_vector(&[second, first]);
        assert_eq!(canonical, shuffled);

        let definition = objective_definition(
            "law",
            ObjectiveDirection::Minimize,
            100,
            ExactValueKind::Int,
        );
        let vectors = [
            objective_test_vector(&[objective_effect(
                "objective-law",
                definition.clone(),
                ObjectiveObservation::Known(ExactValue::Int(1)),
            )]),
            objective_test_vector(&[objective_effect(
                "objective-law",
                definition.clone(),
                ObjectiveObservation::Known(ExactValue::Int(2)),
            )]),
            objective_test_vector(&[objective_effect(
                "objective-law",
                definition.clone(),
                ObjectiveObservation::NotApplicable,
            )]),
            objective_test_vector(&[objective_effect(
                "objective-law",
                definition,
                ObjectiveObservation::Unknown(IntentUnknown {
                    kind: IntentUnknownKind::MissingInput,
                    detail: "not ranked".to_owned(),
                }),
            )]),
        ];
        let keys = vectors
            .iter()
            .map(resolution_ranking_key)
            .collect::<Vec<_>>();
        for left in &keys {
            for right in &keys {
                assert_eq!(left.cmp(right), right.cmp(left).reverse());
                assert_eq!(left.cmp(right) == Ordering::Equal, left == right);
            }
        }
        for left in &keys {
            for middle in &keys {
                for right in &keys {
                    if left <= middle && middle <= right {
                        assert!(left <= right);
                    }
                }
            }
        }
    }

    #[test]
    fn objective_tiers_and_patch_ties_follow_the_locked_lexicographic_order() {
        let stronger_failure = ResolutionObjectiveVector {
            required_violated_or_unknown: 0,
            required_unknown: 0,
            preference_tiers: vec![
                PreferenceTierCost {
                    priority: PreferencePriority(500),
                    violated_or_unknown: 1,
                    unknown: 0,
                },
                PreferenceTierCost {
                    priority: PreferencePriority(100),
                    violated_or_unknown: 0,
                    unknown: 0,
                },
            ],
            objective_components: Vec::new(),
            manhattan_movement_ticks: 0,
            quarter_turn_distance: 0,
        };
        let weaker_failures = ResolutionObjectiveVector {
            required_violated_or_unknown: 0,
            required_unknown: 0,
            preference_tiers: vec![
                PreferenceTierCost {
                    priority: PreferencePriority(500),
                    violated_or_unknown: 0,
                    unknown: 0,
                },
                PreferenceTierCost {
                    priority: PreferencePriority(100),
                    violated_or_unknown: 10,
                    unknown: 10,
                },
            ],
            objective_components: Vec::new(),
            manhattan_movement_ticks: 10_000,
            quarter_turn_distance: 2,
        };
        assert_eq!(
            compare_objective_rank(&weaker_failures, &stronger_failure),
            Ordering::Less
        );

        let canonical = ResolutionObjectiveVector {
            required_violated_or_unknown: 0,
            required_unknown: 0,
            preference_tiers: vec![
                PreferenceTierCost {
                    priority: PreferencePriority(500),
                    violated_or_unknown: 1,
                    unknown: 0,
                },
                PreferenceTierCost {
                    priority: PreferencePriority(100),
                    violated_or_unknown: 2,
                    unknown: 1,
                },
            ],
            objective_components: Vec::new(),
            manhattan_movement_ticks: 12,
            quarter_turn_distance: 1,
        };
        let mut reversed = canonical.clone();
        reversed.preference_tiers.reverse();
        assert_ne!(canonical, reversed, "the public payload remains structural");
        assert_eq!(
            resolution_ranking_key(&canonical),
            resolution_ranking_key(&reversed)
        );

        let duplicate_tiers = ResolutionObjectiveVector {
            required_violated_or_unknown: 0,
            required_unknown: 0,
            preference_tiers: vec![
                PreferenceTierCost {
                    priority: PreferencePriority(500),
                    violated_or_unknown: 1,
                    unknown: 0,
                },
                PreferenceTierCost {
                    priority: PreferencePriority(500),
                    violated_or_unknown: 2,
                    unknown: 1,
                },
            ],
            objective_components: Vec::new(),
            manhattan_movement_ticks: 12,
            quarter_turn_distance: 1,
        };
        let merged_tier = ResolutionObjectiveVector {
            required_violated_or_unknown: 0,
            required_unknown: 0,
            preference_tiers: vec![PreferenceTierCost {
                priority: PreferencePriority(500),
                violated_or_unknown: 3,
                unknown: 1,
            }],
            objective_components: Vec::new(),
            manhattan_movement_ticks: 12,
            quarter_turn_distance: 1,
        };
        assert_ne!(duplicate_tiers, merged_tier);
        assert_eq!(
            resolution_ranking_key(&duplicate_tiers),
            resolution_ranking_key(&merged_tier)
        );
        let mut ordered = BTreeSet::new();
        assert!(ordered.insert(resolution_ranking_key(&canonical)));
        assert!(!ordered.insert(resolution_ranking_key(&reversed)));
        assert!(ordered.insert(resolution_ranking_key(&duplicate_tiers)));
        assert!(!ordered.insert(resolution_ranking_key(&merged_tier)));

        let pose = PlacementPose::new(inches_point(1, 1), QuarterTurn::Deg0);
        let target = PlacementTarget::FurnishingInstance(ElementId::new("target"));
        let x_first = PlacementPatchSemanticKey::from_patch(&PlacementPatch::new(
            target.clone(),
            pose,
            PlacementPose::new(inches_point(1, 2), QuarterTurn::Deg90),
        ));
        let x_second = PlacementPatchSemanticKey::from_patch(&PlacementPatch::new(
            target,
            pose,
            PlacementPose::new(inches_point(2, 1), QuarterTurn::Deg90),
        ));
        assert!(x_first < x_second, "rotation ties compare X before Y");
        assert_eq!(
            quarter_turn_distance(QuarterTurn::Deg0, QuarterTurn::Deg270),
            1
        );
    }
}
