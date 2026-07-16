use framer_core::ElementId;
pub use framer_core::{AuthoredEntityRef, AuthoredIntentId, LibraryVersionRef};
use framer_geometry::BodyRef;
use framer_solver::MemberKind;

use crate::GraphRevision;

/// Revision-scoped identity for one generated framing member.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GeneratedMemberRef {
    pub revision: GraphRevision,
    pub host: AuthoredEntityRef,
    pub member_id: String,
    pub kind: MemberKind,
}

impl GeneratedMemberRef {
    pub fn new(
        revision: GraphRevision,
        host: AuthoredEntityRef,
        member_id: impl Into<String>,
        kind: MemberKind,
    ) -> Self {
        Self {
            revision,
            host,
            member_id: member_id.into(),
            kind,
        }
    }
}

/// Solver-owned generation evidence for one member. Solver-local rule ids remain valid evidence
/// even when they do not name a standards-pack rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SolverProvenanceRef {
    pub revision: GraphRevision,
    pub member: GeneratedMemberRef,
    pub rule_id: String,
}

/// A physical `BodyRef` scoped to the exact graph revision that generated it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalBodyRef {
    pub revision: GraphRevision,
    pub body: BodyRef,
}

/// Current regenerated room/topology fact. Both variants are disposable consequences of the
/// authored room and wall graph; neither is persisted or valid across graph revisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoomConsequenceKind {
    Schedule,
    Boundary,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoomConsequenceRef {
    pub revision: GraphRevision,
    pub room: ElementId,
    pub kind: RoomConsequenceKind,
}

impl RoomConsequenceRef {
    pub fn new(revision: GraphRevision, room: ElementId, kind: RoomConsequenceKind) -> Self {
        Self {
            revision,
            room,
            kind,
        }
    }
}

impl PhysicalBodyRef {
    pub fn new(revision: GraphRevision, body: BodyRef) -> Self {
        Self { revision, body }
    }
}

/// Identity for a resolved or explicitly unresolved standards rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StandardsRuleRef {
    pub pack: Option<ElementId>,
    pub rule: String,
}

impl StandardsRuleRef {
    pub fn resolved(pack: ElementId, rule: impl Into<String>) -> Self {
        Self {
            pack: Some(pack),
            rule: rule.into(),
        }
    }

    pub fn unresolved(rule: impl Into<String>) -> Self {
        Self {
            pack: None,
            rule: rule.into(),
        }
    }
}

/// Deterministic identity for one entry in the regenerated compliance report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComplianceEntryRef {
    pub revision: GraphRevision,
    pub rule: StandardsRuleRef,
    pub subject: Option<AuthoredEntityRef>,
    pub ordinal: u32,
}

/// Provider responsible for a derived diagnostic node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticProvider {
    Solver,
    Standards,
    Geometry,
    Library,
    Analysis,
}

/// Revision-scoped diagnostic identity. `ordinal` disambiguates equal code/source pairs in a
/// canonical provider stream; it is not persisted and carries no cross-revision authority.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagnosticRef {
    pub revision: GraphRevision,
    pub provider: DiagnosticProvider,
    pub code: String,
    pub source: Option<AuthoredEntityRef>,
    pub ordinal: u32,
}

/// The expected semantic family for evidence that could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnknownEvidenceKind {
    AuthoredEntity,
    StandardsRule,
    GeneratedMember,
    GeneratedHost,
    GeneratedSource,
    PhysicalBody,
    PhysicalOwner,
    RoomSchedule,
    RoomBoundary,
    RoomBoundaryWall,
    DiagnosticSource,
}

/// Explicit fail-closed graph node for optional or inconsistent derived evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnknownEvidenceRef {
    pub revision: GraphRevision,
    pub kind: UnknownEvidenceKind,
    pub identity: String,
}

impl UnknownEvidenceRef {
    pub fn new(
        revision: GraphRevision,
        kind: UnknownEvidenceKind,
        identity: impl Into<String>,
    ) -> Self {
        Self {
            revision,
            kind,
            identity: identity.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DerivedAssertionProvider {
    Core,
    Solver,
    Standards,
    Geometry,
    Analysis,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DerivedAssertionSource {
    Project,
    Authored(AuthoredEntityRef),
    GeneratedMember(GeneratedMemberRef),
    StandardsRule(StandardsRuleRef),
    PhysicalBody(PhysicalBodyRef),
}

/// Disposable derived assertion id, constructed from provider, semantic source, and role.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DerivedAssertionId {
    pub revision: GraphRevision,
    pub provider: DerivedAssertionProvider,
    pub source: DerivedAssertionSource,
    pub role: String,
}

impl DerivedAssertionId {
    pub fn new(
        revision: GraphRevision,
        provider: DerivedAssertionProvider,
        source: DerivedAssertionSource,
        role: impl Into<String>,
    ) -> Self {
        Self {
            revision,
            provider,
            source,
            role: role.into(),
        }
    }
}

/// Authored and derived assertion identities are type-disjoint even when their display text is
/// identical. Derived ids are authoritative only inside one graph revision.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssertionRef {
    Authored(AuthoredIntentId),
    Derived(DerivedAssertionId),
}

/// Project-wide semantic node identity. App-only selection and render cache ids never enter this
/// enum; generated and evidence variants are disposable for the current [`GraphRevision`].
///
/// [`GraphRevision`]: crate::GraphRevision
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectNodeRef {
    Project,
    Authored(AuthoredEntityRef),
    GeneratedMember(GeneratedMemberRef),
    SolverProvenance(SolverProvenanceRef),
    PhysicalBody(PhysicalBodyRef),
    RoomConsequence(RoomConsequenceRef),
    StandardsRule(StandardsRuleRef),
    ComplianceEntry(ComplianceEntryRef),
    Diagnostic(DiagnosticRef),
    Assertion(AssertionRef),
    UnknownEvidence(UnknownEvidenceRef),
}

impl ProjectNodeRef {
    pub const fn is_revision_scoped(&self) -> bool {
        match self {
            Self::Project | Self::Authored(_) | Self::StandardsRule(_) => false,
            Self::Assertion(AssertionRef::Authored(_)) => false,
            Self::GeneratedMember(_)
            | Self::SolverProvenance(_)
            | Self::PhysicalBody(_)
            | Self::RoomConsequence(_)
            | Self::ComplianceEntry(_)
            | Self::Diagnostic(_)
            | Self::Assertion(AssertionRef::Derived(_))
            | Self::UnknownEvidence(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_and_derived_assertion_namespaces_cannot_collide() {
        let authored = AssertionRef::Authored(AuthoredIntentId::new("same-text"));
        let revision = GraphRevision::for_model(&framer_core::BuildingModel::new()).unwrap();
        let derived = AssertionRef::Derived(DerivedAssertionId::new(
            revision,
            DerivedAssertionProvider::Analysis,
            DerivedAssertionSource::Project,
            "same-text",
        ));

        assert_ne!(authored, derived);
    }
}
