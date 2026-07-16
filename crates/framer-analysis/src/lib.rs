//! UI-free cross-domain analysis over Framer's authored and regenerated project state.
//!
//! [`BuildingModel`](framer_core::BuildingModel) remains the only persisted source of truth.
//! This crate compiles a deterministic, disposable graph and serves revision-bound explanation
//! and impact queries without introducing a dependency from any lower domain crate.

#![forbid(unsafe_code)]

mod compile;
mod graph;
mod identity;
mod intent;
mod lower;
mod query;
mod revision;

pub use compile::{
    AnalysisError, LibraryLifecycleStatus, ProjectAnalysis, analyze_project,
    library_lifecycle_status,
};
pub use graph::{
    GraphBuildError, GraphFamily, ProjectEdge, ProjectGraph, ProjectNode, RelationshipKind,
};
pub use identity::{
    AssertionRef, AuthoredEntityRef, AuthoredIntentId, ComplianceEntryRef, DerivedAssertionId,
    DerivedAssertionProvider, DerivedAssertionRole, DerivedAssertionSource, DiagnosticProvider,
    DiagnosticRef, GeneratedMemberRef, LibraryVersionRef, PhysicalBodyRef, ProjectNodeRef,
    RoomConsequenceKind, RoomConsequenceRef, SiteAssumptionKey, SolverProvenanceRef,
    StandardsRuleRef, UnknownEvidenceKind, UnknownEvidenceRef,
};
pub use intent::{
    AssertionParticipant, AssertionParticipantRole, AssertionScope, AssertionSource,
    AssumptionEvidence, AssumptionIntentRecord, AssumptionPremise, BooleanExpression,
    BooleanIntentMode, BooleanIntentRecord, CompiledAssertion, ExactValue, IntentDomain,
    IntentEvidenceRef, IntentOutcome, IntentRecord, IntentReport, IntentUnknown, IntentUnknownKind,
    IntentValue, ObjectiveDefinition, ObjectiveDirection, ObjectiveIntentRecord,
    ObjectiveObservation, PreferencePriority, SelectionAttribute, WaiverRecord, WaiverRef,
};
pub use query::{
    DependencyImpact, GraphQueryCache, GraphQueryKind, GraphStep, GraphTrace, QueryCacheStats,
};
pub use revision::{GRAPH_CONTRACT_VERSION, GraphRevision};

/// Identify which typed domain owns the canonical row for an existing plan diagnostic.
/// Presentation layers use this to avoid rendering geometry audit findings twice after they are
/// also lowered through `ProjectFramePlan::diagnostics`.
pub fn plan_diagnostic_provider(diagnostic: &framer_solver::PlanDiagnostic) -> DiagnosticProvider {
    lower::diagnostic_provider(diagnostic)
}
