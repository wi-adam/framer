use std::collections::{BTreeMap, BTreeSet};

use framer_core::ElementId;
use thiserror::Error;

use crate::{AuthoredEntityRef, GeneratedMemberRef, GraphRevision, ProjectNodeRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GraphFamily {
    OwnershipReference,
    ConstraintAssertion,
    DerivationEvidence,
    ConflictAlternative,
}

/// A dependency-oriented graph relationship: `ProjectEdge::dependent` consumes or derives from
/// `ProjectEdge::dependency`. This fixed orientation makes transitive impact queries unambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelationshipKind {
    BelongsTo,
    UsesSystem,
    UsesMaterial,
    UsesFamily,
    UsesStandardsPack,
    VendoredFrom,
    References,
    HostedBy,
    GeneratedFrom,
    JustifiedBy,
    PhysicalFormOf,
    EvaluatedFrom,
    LoweredFrom,
    EvidenceFor,
    UnresolvedEvidence,
}

impl RelationshipKind {
    pub const fn family(self) -> GraphFamily {
        match self {
            Self::BelongsTo
            | Self::UsesSystem
            | Self::UsesMaterial
            | Self::UsesFamily
            | Self::UsesStandardsPack
            | Self::VendoredFrom
            | Self::References
            | Self::HostedBy => GraphFamily::OwnershipReference,
            Self::GeneratedFrom
            | Self::JustifiedBy
            | Self::PhysicalFormOf
            | Self::EvaluatedFrom
            | Self::LoweredFrom
            | Self::EvidenceFor
            | Self::UnresolvedEvidence => GraphFamily::DerivationEvidence,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::BelongsTo => "belongs to",
            Self::UsesSystem => "uses system",
            Self::UsesMaterial => "uses material",
            Self::UsesFamily => "uses family",
            Self::UsesStandardsPack => "uses standards pack",
            Self::VendoredFrom => "vendored from",
            Self::References => "references",
            Self::HostedBy => "hosted by",
            Self::GeneratedFrom => "generated from",
            Self::JustifiedBy => "justified by",
            Self::PhysicalFormOf => "physical form of",
            Self::EvaluatedFrom => "evaluated from",
            Self::LoweredFrom => "lowered from",
            Self::EvidenceFor => "evidence for",
            Self::UnresolvedEvidence => "unresolved evidence",
        }
    }

    pub const fn inverse_label(self) -> &'static str {
        match self {
            Self::BelongsTo => "contains",
            Self::UsesSystem | Self::UsesMaterial | Self::UsesFamily | Self::UsesStandardsPack => {
                "used by"
            }
            Self::VendoredFrom => "source for",
            Self::References => "referenced by",
            Self::HostedBy => "hosts",
            Self::GeneratedFrom => "generates",
            Self::JustifiedBy => "justifies",
            Self::PhysicalFormOf => "has physical form",
            Self::EvaluatedFrom => "evaluated by",
            Self::LoweredFrom => "lowers to",
            Self::EvidenceFor => "has evidence",
            Self::UnresolvedEvidence => "missing for",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectNode {
    pub reference: ProjectNodeRef,
    pub title: String,
    pub detail: Option<String>,
}

impl ProjectNode {
    pub fn new(
        reference: ProjectNodeRef,
        title: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            reference,
            title: title.into(),
            detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProjectEdge {
    pub dependent: ProjectNodeRef,
    pub dependency: ProjectNodeRef,
    pub relationship: RelationshipKind,
    /// Stable semantic position where order matters (for example standards stack or layer order).
    pub semantic_order: Option<u32>,
}

impl ProjectEdge {
    pub fn new(
        dependent: ProjectNodeRef,
        dependency: ProjectNodeRef,
        relationship: RelationshipKind,
    ) -> Self {
        Self {
            dependent,
            dependency,
            relationship,
            semantic_order: None,
        }
    }

    pub fn ordered(mut self, semantic_order: usize) -> Self {
        self.semantic_order = u32::try_from(semantic_order).ok();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGraph {
    revision: GraphRevision,
    nodes: Vec<ProjectNode>,
    edges: Vec<ProjectEdge>,
}

/// Internal graph-compilation invariant failure.
///
/// Compilers should normally lower unavailable domain evidence to an explicit
/// [`ProjectNodeRef::UnknownEvidence`] node. This error keeps a missed lowering or future
/// compiler defect independently fallible instead of allowing it to panic the app rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GraphBuildError {
    #[error("graph edge has missing dependent endpoint: {reference:?}")]
    MissingDependent { reference: Box<ProjectNodeRef> },
    #[error("graph edge has missing dependency endpoint: {reference:?}")]
    MissingDependency { reference: Box<ProjectNodeRef> },
}

impl ProjectGraph {
    pub(crate) fn from_parts(
        revision: GraphRevision,
        nodes: Vec<ProjectNode>,
        edges: Vec<ProjectEdge>,
    ) -> Self {
        Self {
            revision,
            nodes,
            edges,
        }
    }

    pub const fn revision(&self) -> GraphRevision {
        self.revision
    }

    pub fn nodes(&self) -> &[ProjectNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[ProjectEdge] {
        &self.edges
    }

    pub fn node(&self, reference: &ProjectNodeRef) -> Option<&ProjectNode> {
        self.nodes
            .binary_search_by(|node| node.reference.cmp(reference))
            .ok()
            .map(|index| &self.nodes[index])
    }

    pub fn authored_entity(&self, id: &ElementId) -> Option<&AuthoredEntityRef> {
        self.nodes.iter().find_map(|node| match &node.reference {
            ProjectNodeRef::Authored(reference) if reference.element_id() == Some(id) => {
                Some(reference)
            }
            _ => None,
        })
    }

    pub fn generated_member(&self, host_id: &str, member_id: &str) -> Option<&GeneratedMemberRef> {
        self.nodes.iter().find_map(|node| match &node.reference {
            ProjectNodeRef::GeneratedMember(reference)
                if reference.member_id == member_id
                    && reference
                        .host
                        .element_id()
                        .is_some_and(|id| id.0 == host_id) =>
            {
                Some(reference)
            }
            _ => None,
        })
    }
}

pub(crate) struct GraphBuilder {
    revision: GraphRevision,
    nodes: BTreeMap<ProjectNodeRef, ProjectNode>,
    edges: BTreeSet<ProjectEdge>,
}

impl GraphBuilder {
    pub(crate) fn new(revision: GraphRevision) -> Self {
        Self {
            revision,
            nodes: BTreeMap::new(),
            edges: BTreeSet::new(),
        }
    }

    pub(crate) fn node(&mut self, node: ProjectNode) {
        self.nodes.entry(node.reference.clone()).or_insert(node);
    }

    pub(crate) fn edge(&mut self, edge: ProjectEdge) {
        self.edges.insert(edge);
    }

    pub(crate) fn contains_node(&self, reference: &ProjectNodeRef) -> bool {
        self.nodes.contains_key(reference)
    }

    pub(crate) fn finish(self) -> Result<ProjectGraph, GraphBuildError> {
        for edge in &self.edges {
            if !self.nodes.contains_key(&edge.dependent) {
                return Err(GraphBuildError::MissingDependent {
                    reference: Box::new(edge.dependent.clone()),
                });
            }
            if !self.nodes.contains_key(&edge.dependency) {
                return Err(GraphBuildError::MissingDependency {
                    reference: Box::new(edge.dependency.clone()),
                });
            }
        }
        Ok(ProjectGraph::from_parts(
            self.revision,
            self.nodes.into_values().collect(),
            self.edges.into_iter().collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_relationship_is_ownership_not_derivation() {
        assert_eq!(
            RelationshipKind::HostedBy.family(),
            GraphFamily::OwnershipReference
        );
        assert_eq!(
            RelationshipKind::GeneratedFrom.family(),
            GraphFamily::DerivationEvidence
        );
    }

    #[test]
    fn missing_edge_endpoint_is_a_typed_error_not_a_panic() {
        let revision = GraphRevision::for_model(&framer_core::BuildingModel::new()).unwrap();
        let wall = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-a")));
        let opening =
            ProjectNodeRef::Authored(AuthoredEntityRef::Opening(ElementId::new("opening-a")));
        let mut builder = GraphBuilder::new(revision);
        builder.node(ProjectNode::new(wall.clone(), "Wall", None));
        builder.edge(ProjectEdge::new(
            opening.clone(),
            wall,
            RelationshipKind::BelongsTo,
        ));

        assert_eq!(
            builder.finish(),
            Err(GraphBuildError::MissingDependent {
                reference: Box::new(opening)
            })
        );
    }

    #[test]
    fn missing_edge_dependency_is_a_typed_error_not_a_panic() {
        let revision = GraphRevision::for_model(&framer_core::BuildingModel::new()).unwrap();
        let wall = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-a")));
        let opening =
            ProjectNodeRef::Authored(AuthoredEntityRef::Opening(ElementId::new("opening-a")));
        let mut builder = GraphBuilder::new(revision);
        builder.node(ProjectNode::new(opening.clone(), "Opening", None));
        builder.edge(ProjectEdge::new(
            opening,
            wall.clone(),
            RelationshipKind::BelongsTo,
        ));

        assert_eq!(
            builder.finish(),
            Err(GraphBuildError::MissingDependency {
                reference: Box::new(wall)
            })
        );
    }
}
