use std::collections::{BTreeMap, BTreeSet};

use crate::{
    GraphFamily, GraphRevision, ProjectEdge, ProjectGraph, ProjectNodeRef, RelationshipKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GraphQueryKind {
    Dependencies,
    Dependents,
    IncomingIntent,
    DerivedFrom,
    EvidenceFor,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphStep {
    pub from: ProjectNodeRef,
    pub to: ProjectNodeRef,
    pub relationship: RelationshipKind,
    /// True when traversal followed the stored dependent → dependency orientation.
    pub toward_dependency: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphTrace {
    pub node: ProjectNodeRef,
    pub depth: u32,
    pub path: Vec<GraphStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueryCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct QueryKey {
    kind: GraphQueryKind,
    start: ProjectNodeRef,
}

/// Explicit revision-bound memoization for lazy graph closures.
///
/// Keeping the cache outside [`ProjectGraph`] lets graph compilation remain `Eq` and makes stale
/// invalidation visible and testable. A query against a different revision clears all entries
/// before computing its result.
#[derive(Debug, Clone, Default)]
pub struct GraphQueryCache {
    revision: Option<GraphRevision>,
    entries: BTreeMap<QueryKey, Vec<GraphTrace>>,
    hits: u64,
    misses: u64,
}

impl GraphQueryCache {
    pub fn query(
        &mut self,
        graph: &ProjectGraph,
        kind: GraphQueryKind,
        start: &ProjectNodeRef,
    ) -> Vec<GraphTrace> {
        self.bind_revision(graph.revision());
        let key = QueryKey {
            kind,
            start: start.clone(),
        };
        if let Some(cached) = self.entries.get(&key) {
            self.hits = self.hits.saturating_add(1);
            return cached.clone();
        }

        self.misses = self.misses.saturating_add(1);
        let result = closure(graph, kind, start);
        self.entries.insert(key, result.clone());
        result
    }

    pub fn dependencies(
        &mut self,
        graph: &ProjectGraph,
        start: &ProjectNodeRef,
    ) -> Vec<GraphTrace> {
        self.query(graph, GraphQueryKind::Dependencies, start)
    }

    /// Localized dependency closure using the product vocabulary from the intent-model spec.
    pub fn depends_on(&mut self, graph: &ProjectGraph, start: &ProjectNodeRef) -> Vec<GraphTrace> {
        self.dependencies(graph, start)
    }

    pub fn dependents(&mut self, graph: &ProjectGraph, start: &ProjectNodeRef) -> Vec<GraphTrace> {
        self.query(graph, GraphQueryKind::Dependents, start)
    }

    pub fn incoming_intent(
        &mut self,
        graph: &ProjectGraph,
        start: &ProjectNodeRef,
    ) -> Vec<GraphTrace> {
        self.query(graph, GraphQueryKind::IncomingIntent, start)
    }

    pub fn derived_from(
        &mut self,
        graph: &ProjectGraph,
        start: &ProjectNodeRef,
    ) -> Vec<GraphTrace> {
        self.query(graph, GraphQueryKind::DerivedFrom, start)
    }

    pub fn evidence_for(
        &mut self,
        graph: &ProjectGraph,
        start: &ProjectNodeRef,
    ) -> Vec<GraphTrace> {
        self.query(graph, GraphQueryKind::EvidenceFor, start)
    }

    pub fn clear(&mut self) {
        self.revision = None;
        self.entries.clear();
        self.hits = 0;
        self.misses = 0;
    }

    pub fn stats(&self) -> QueryCacheStats {
        QueryCacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.entries.len(),
        }
    }

    pub const fn revision(&self) -> Option<GraphRevision> {
        self.revision
    }

    fn bind_revision(&mut self, revision: GraphRevision) {
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.entries.clear();
            self.hits = 0;
            self.misses = 0;
        }
    }
}

fn closure(graph: &ProjectGraph, kind: GraphQueryKind, start: &ProjectNodeRef) -> Vec<GraphTrace> {
    if graph.node(start).is_none() {
        return Vec::new();
    }

    let mut visited = BTreeSet::from([start.clone()]);
    let mut frontier = BTreeMap::from([(start.clone(), Vec::<GraphStep>::new())]);
    let mut result = Vec::new();
    let mut depth = 0u32;

    while !frontier.is_empty() {
        depth = depth.saturating_add(1);
        let mut next = BTreeMap::new();
        for (current, path) in &frontier {
            // `Project` is a useful ownership endpoint, but traversing through it would turn
            // every project-wide setting into a dependency of every authored entity (and every
            // authored entity into an impact of that setting). Keep localized closures local.
            if current != start && matches!(current, ProjectNodeRef::Project) {
                continue;
            }
            for (neighbor, relationship, toward_dependency) in
                neighbors(graph.edges(), kind, current)
            {
                if visited.insert(neighbor.clone()) {
                    let mut next_path = path.clone();
                    next_path.push(GraphStep {
                        from: current.clone(),
                        to: neighbor.clone(),
                        relationship,
                        toward_dependency,
                    });
                    next.insert(neighbor, next_path);
                }
            }
        }
        result.extend(next.iter().map(|(node, path)| GraphTrace {
            node: node.clone(),
            depth,
            path: path.clone(),
        }));
        frontier = next;
    }

    result
}

fn neighbors(
    edges: &[ProjectEdge],
    kind: GraphQueryKind,
    current: &ProjectNodeRef,
) -> BTreeSet<(ProjectNodeRef, RelationshipKind, bool)> {
    let mut found = BTreeSet::new();
    for edge in edges {
        match kind {
            GraphQueryKind::Dependencies if edge.dependent == *current => {
                found.insert((edge.dependency.clone(), edge.relationship, true));
            }
            GraphQueryKind::Dependents if edge.dependency == *current => {
                found.insert((edge.dependent.clone(), edge.relationship, false));
            }
            GraphQueryKind::IncomingIntent => {
                if edge.dependency == *current
                    && matches!(edge.dependent, ProjectNodeRef::Assertion(_))
                {
                    found.insert((edge.dependent.clone(), edge.relationship, false));
                } else if edge.dependent == *current
                    && matches!(edge.dependency, ProjectNodeRef::Assertion(_))
                {
                    found.insert((edge.dependency.clone(), edge.relationship, true));
                }
            }
            GraphQueryKind::DerivedFrom
                if edge.dependent == *current
                    && edge.relationship.family() == GraphFamily::DerivationEvidence =>
            {
                found.insert((edge.dependency.clone(), edge.relationship, true));
            }
            GraphQueryKind::EvidenceFor => {
                if edge.dependent == *current && is_supporting_evidence(current, edge.relationship)
                {
                    found.insert((edge.dependency.clone(), edge.relationship, true));
                }
            }
            GraphQueryKind::Dependencies
            | GraphQueryKind::Dependents
            | GraphQueryKind::DerivedFrom => {}
        }
    }
    found
}

fn is_supporting_evidence(current: &ProjectNodeRef, relationship: RelationshipKind) -> bool {
    match relationship {
        RelationshipKind::GeneratedFrom
        | RelationshipKind::JustifiedBy
        | RelationshipKind::PhysicalFormOf
        | RelationshipKind::EvaluatedFrom
        | RelationshipKind::LoweredFrom
        | RelationshipKind::EvidenceFor
        | RelationshipKind::UnresolvedEvidence => true,
        RelationshipKind::References => matches!(
            current,
            ProjectNodeRef::SolverProvenance(_) | ProjectNodeRef::StandardsRule(_)
        ),
        RelationshipKind::BelongsTo
        | RelationshipKind::UsesSystem
        | RelationshipKind::UsesMaterial
        | RelationshipKind::UsesFamily
        | RelationshipKind::UsesStandardsPack
        | RelationshipKind::VendoredFrom
        | RelationshipKind::HostedBy => false,
    }
}

#[cfg(test)]
mod tests {
    use framer_core::{BuildingModel, ElementId};

    use super::*;
    use crate::{AuthoredEntityRef, ProjectNode, RelationshipKind};

    fn graph(model: &BuildingModel, cycle: bool) -> ProjectGraph {
        let revision = GraphRevision::for_model(model).unwrap();
        let wall = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-a")));
        let opening =
            ProjectNodeRef::Authored(AuthoredEntityRef::Opening(ElementId::new("opening-a")));
        let mut edges = vec![ProjectEdge::new(
            opening.clone(),
            wall.clone(),
            RelationshipKind::BelongsTo,
        )];
        if cycle {
            edges.push(ProjectEdge::new(
                wall.clone(),
                opening.clone(),
                RelationshipKind::References,
            ));
        }
        ProjectGraph::from_parts(
            revision,
            vec![
                ProjectNode::new(wall, "Wall", None),
                ProjectNode::new(opening, "Opening", None),
            ],
            edges,
        )
    }

    #[test]
    fn repeated_closure_hits_cache_and_cycles_terminate() {
        let model = BuildingModel::new();
        let graph = graph(&model, true);
        let wall = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-a")));
        let mut cache = GraphQueryCache::default();

        let first = cache.dependents(&graph, &wall);
        let second = cache.dependents(&graph, &wall);

        assert_eq!(first, second);
        assert_eq!(first.len(), 1);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn closure_paths_are_contiguous_shortest_paths_and_exclude_cycle_origin() {
        let model = BuildingModel::new();
        let revision = GraphRevision::for_model(&model).unwrap();
        let opening =
            ProjectNodeRef::Authored(AuthoredEntityRef::Opening(ElementId::new("opening-a")));
        let wall = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-a")));
        let project = ProjectNodeRef::Project;
        let mut nodes = vec![
            ProjectNode::new(opening.clone(), "Opening", None),
            ProjectNode::new(wall.clone(), "Wall", None),
            ProjectNode::new(project.clone(), "Project", None),
        ];
        nodes.sort();
        let graph = ProjectGraph::from_parts(
            revision,
            nodes,
            vec![
                ProjectEdge::new(opening.clone(), wall.clone(), RelationshipKind::BelongsTo),
                ProjectEdge::new(wall.clone(), project.clone(), RelationshipKind::BelongsTo),
                ProjectEdge::new(
                    project.clone(),
                    opening.clone(),
                    RelationshipKind::References,
                ),
            ],
        );
        let mut cache = GraphQueryCache::default();
        let traces = cache.dependencies(&graph, &opening);

        assert_eq!(traces.len(), 2);
        assert_eq!(traces[0].node, wall);
        assert_eq!(traces[0].depth, 1);
        assert_eq!(traces[1].node, project);
        assert_eq!(traces[1].depth, 2);
        assert!(traces.iter().all(|trace| trace.node != opening));
        for trace in traces {
            assert_eq!(trace.path.len(), trace.depth as usize);
            assert_eq!(trace.path.first().unwrap().from, opening);
            assert_eq!(trace.path.last().unwrap().to, trace.node);
            assert!(
                trace
                    .path
                    .windows(2)
                    .all(|steps| steps[0].to == steps[1].from)
            );
        }
    }

    #[test]
    fn cache_keys_query_kind_and_caches_honest_empty_results() {
        let model = BuildingModel::new();
        let graph = graph(&model, false);
        let wall = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-a")));
        let missing =
            ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-missing")));
        let mut cache = GraphQueryCache::default();

        assert!(!cache.dependents(&graph, &wall).is_empty());
        assert!(cache.dependencies(&graph, &wall).is_empty());
        assert!(cache.dependencies(&graph, &missing).is_empty());
        assert!(cache.dependencies(&graph, &missing).is_empty());

        assert_eq!(cache.stats().misses, 3);
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().entries, 3);
    }

    #[test]
    fn evidence_query_only_walks_toward_whitelisted_support() {
        let model = BuildingModel::new();
        let revision = GraphRevision::for_model(&model).unwrap();
        let source =
            ProjectNodeRef::Authored(AuthoredEntityRef::Opening(ElementId::new("opening-a")));
        let target = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-a")));
        let downstream = ProjectNodeRef::UnknownEvidence(crate::UnknownEvidenceRef::new(
            revision,
            crate::UnknownEvidenceKind::PhysicalBody,
            "downstream-body",
        ));
        let unknown = ProjectNodeRef::UnknownEvidence(crate::UnknownEvidenceRef::new(
            revision,
            crate::UnknownEvidenceKind::DiagnosticSource,
            "downstream-diagnostic",
        ));
        let mut nodes = vec![
            ProjectNode::new(source.clone(), "Source", None),
            ProjectNode::new(target.clone(), "Target", None),
            ProjectNode::new(downstream.clone(), "Downstream body", None),
            ProjectNode::new(unknown.clone(), "Downstream diagnostic", None),
        ];
        nodes.sort();
        let graph = ProjectGraph::from_parts(
            revision,
            nodes,
            vec![
                ProjectEdge::new(
                    target.clone(),
                    source.clone(),
                    RelationshipKind::GeneratedFrom,
                ),
                ProjectEdge::new(
                    downstream.clone(),
                    target.clone(),
                    RelationshipKind::PhysicalFormOf,
                ),
                ProjectEdge::new(
                    unknown.clone(),
                    downstream.clone(),
                    RelationshipKind::EvaluatedFrom,
                ),
            ],
        );
        let mut cache = GraphQueryCache::default();

        let support = cache.evidence_for(&graph, &target);
        assert_eq!(support.len(), 1);
        assert_eq!(support[0].node, source);
        assert!(support[0].path.iter().all(|step| step.toward_dependency));
        assert!(
            support
                .iter()
                .all(|trace| trace.node != downstream && trace.node != unknown)
        );

        let diagnostic_support = cache.evidence_for(&graph, &unknown);
        assert!(diagnostic_support.iter().any(|trace| trace.node == source));
        assert!(
            diagnostic_support
                .iter()
                .all(|trace| trace.path.iter().all(|step| step.toward_dependency))
        );
    }

    #[test]
    fn project_ownership_is_an_endpoint_not_a_transitive_bridge() {
        let model = BuildingModel::new();
        let revision = GraphRevision::for_model(&model).unwrap();
        let material = ProjectNodeRef::Authored(AuthoredEntityRef::Material(ElementId::new(
            "material-unused",
        )));
        let project = ProjectNodeRef::Project;
        let pack = ProjectNodeRef::Authored(AuthoredEntityRef::StandardsPack(ElementId::new(
            "standards-pack",
        )));
        let mut nodes = vec![
            ProjectNode::new(material.clone(), "Material", None),
            ProjectNode::new(project.clone(), "Project", None),
            ProjectNode::new(pack.clone(), "Standards", None),
        ];
        nodes.sort();
        let graph = ProjectGraph::from_parts(
            revision,
            nodes,
            vec![
                ProjectEdge::new(
                    material.clone(),
                    project.clone(),
                    RelationshipKind::BelongsTo,
                ),
                ProjectEdge::new(
                    project.clone(),
                    pack.clone(),
                    RelationshipKind::UsesStandardsPack,
                ),
            ],
        );
        let mut cache = GraphQueryCache::default();

        let dependencies = cache.dependencies(&graph, &material);
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].node, project);
        assert!(dependencies.iter().all(|trace| trace.node != pack));
    }

    #[test]
    fn new_revision_discards_cached_closures() {
        let first_model = BuildingModel::new();
        let first = graph(&first_model, false);
        let wall = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-a")));
        let mut cache = GraphQueryCache::default();
        cache.dependents(&first, &wall);
        cache.dependents(&first, &wall);
        assert_eq!(cache.stats().hits, 1);

        let mut second_model = first_model;
        second_model.site.jurisdiction = "Changed".to_owned();
        let second = graph(&second_model, false);
        cache.dependents(&second, &wall);

        assert_eq!(cache.revision(), Some(second.revision()));
        assert_eq!(cache.stats().hits, 0);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().entries, 1);
    }
}
