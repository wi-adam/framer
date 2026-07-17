use std::cmp::Ordering;
use std::collections::BTreeMap;

use framer_core::{
    AuthoredEntityRef, BuildingModel, DimensionAnchor, ElementId, IntentOverride,
    LibraryVersionRef, MaterialSource, Point2, ProjectError, Provenance, ResolvedStandards,
    SurfaceRegion, Wall, room_boundaries_for_rooms,
};
use framer_geometry::{
    AssemblyKind, BodyKind, BodyRef, GeometryAudit, GeometryViolation, PhysicalScene,
    audit_physical_scene, build_physical_scene,
};
use framer_library::{LibraryIssue, LibraryIssueKind, LibraryItem};
use framer_solver::{
    DiagnosticSeverity, FrameMember, MemberKind, PlanDiagnostic, ProjectFramePlan, RoomSchedule,
    SolverError, generate_project_plan,
};
use framer_standards::{ComplianceEntry, ComplianceReport, Outcome, StandardsEvaluation};
use thiserror::Error;

use crate::graph::{GraphBuildError, GraphBuilder};
use crate::{
    ComplianceEntryRef, DiagnosticProvider, DiagnosticRef, GeneratedMemberRef, GraphRevision,
    IntentEvidenceRef, IntentOutcome, IntentRecord, IntentReport, PhysicalBodyRef, ProjectEdge,
    ProjectGraph, ProjectNode, ProjectNodeRef, RelationshipKind, RoomConsequenceKind,
    RoomConsequenceRef, SolverProvenanceRef, StandardsRuleRef, UnknownEvidenceKind,
    UnknownEvidenceRef,
};

/// A coherent generation of every UI-free derived artifact consumed by the app.
///
/// Graph compilation is deliberately fallible independently of solving. If canonical graph
/// fingerprinting ever fails, callers retain the valid plan/report/geometry generation and can
/// present analysis as unavailable without changing framing or compliance behavior.
pub struct ProjectAnalysis {
    pub plan: ProjectFramePlan,
    pub resolved_standards: ResolvedStandards,
    pub physical_scene: PhysicalScene,
    pub geometry_audit: GeometryAudit,
    pub standards_evaluation: StandardsEvaluation,
    pub library_lifecycle: LibraryLifecycleStatus,
    pub intent_report: Result<IntentReport, AnalysisError>,
    pub graph: Result<ProjectGraph, AnalysisError>,
}

impl ProjectAnalysis {
    pub fn revision(&self) -> Option<GraphRevision> {
        self.intent_report
            .as_ref()
            .ok()
            .map(IntentReport::revision)
            .or_else(|| self.graph.as_ref().ok().map(ProjectGraph::revision))
    }

    /// Recover the original standards payload referenced by common intent evidence.
    pub fn compliance_entry(&self, reference: &ComplianceEntryRef) -> Option<&ComplianceEntry> {
        if self.revision() != Some(reference.revision) {
            return None;
        }
        let mut indices = (0..self.standards_evaluation.report.entries.len()).collect::<Vec<_>>();
        indices.sort_by(|left, right| {
            compare_compliance_entry(
                &self.standards_evaluation.report.entries[*left],
                &self.standards_evaluation.report.entries[*right],
            )
        });
        let mut ordinal = 0u32;
        for index in indices {
            let entry = &self.standards_evaluation.report.entries[index];
            let same_rule = entry.rule == reference.rule.rule
                && Some(&entry.pack) == reference.rule.pack.as_ref();
            let same_subject = entry.element.as_ref()
                == reference
                    .subject
                    .as_ref()
                    .and_then(AuthoredEntityRef::element_id);
            if !same_rule || !same_subject {
                continue;
            }
            if ordinal == reference.ordinal {
                return Some(entry);
            }
            ordinal = ordinal.saturating_add(1);
        }
        None
    }

    pub fn standards_details_for(
        &self,
        reference: &ComplianceEntryRef,
    ) -> Vec<&framer_standards::StandardsEvaluationDetail> {
        let Some(entry) = self.compliance_entry(reference) else {
            return Vec::new();
        };
        let Some(index) = self
            .standards_evaluation
            .report
            .entries
            .iter()
            .position(|candidate| std::ptr::eq(candidate, entry))
        else {
            return Vec::new();
        };
        self.standards_evaluation
            .details
            .iter()
            .filter(|detail| detail.report_entry_index == index)
            .collect()
    }

    /// Recover the native geometry witness payload for a common geometry finding.
    pub fn geometry_violation(&self, reference: &DiagnosticRef) -> Option<&GeometryViolation> {
        if reference.provider != DiagnosticProvider::Geometry
            || self.revision() != Some(reference.revision)
        {
            return None;
        }
        let diagnostic = self.plan_diagnostic(reference)?;
        let mut violations = self.geometry_audit.violations.iter().collect::<Vec<_>>();
        violations.sort_by(|left, right| crate::lower::compare_geometry_violation(left, right));
        violations.into_iter().find(|violation| {
            violation.code() == reference.code
                && diagnostic.source.as_ref() == Some(violation.body_a().owner())
                && diagnostic.message == violation.to_string()
        })
    }

    /// Recover the exact native plan diagnostic carried by common intent evidence.
    pub fn plan_diagnostic(&self, reference: &DiagnosticRef) -> Option<&PlanDiagnostic> {
        self.intent_report
            .as_ref()
            .ok()
            .and_then(|report| report.plan_diagnostic(reference))
    }
}

/// Current lifecycle comparison between vendored project items and the bundled library source.
/// The status is regenerated with the rest of the analysis and is never persisted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LibraryLifecycleStatus {
    pub issues: Vec<LibraryIssue>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AnalysisError {
    #[error("project graph revision could not be computed: {0}")]
    Project(String),
    #[error("project graph could not be compiled: {0}")]
    Graph(#[from] GraphBuildError),
}

impl From<ProjectError> for AnalysisError {
    fn from(error: ProjectError) -> Self {
        Self::Project(error.to_string())
    }
}

/// Recompute current library lifecycle status without requiring framing to solve successfully.
pub fn library_lifecycle_status(model: &BuildingModel) -> LibraryLifecycleStatus {
    // Preserve the existing app behavior if the bundled source cannot be loaded: local
    // divergence can still be checked, while source-version comparisons are simply unavailable.
    let loaded = framer_library::starter_library_ref().ok();
    let current_libraries = loaded
        .as_ref()
        .map(|loaded| std::slice::from_ref(&loaded.library))
        .unwrap_or_default();
    match framer_library::library_lifecycle_issues(model, current_libraries) {
        Ok(issues) => LibraryLifecycleStatus {
            issues,
            error: None,
        },
        Err(error) => LibraryLifecycleStatus {
            issues: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

/// Generate one coherent project analysis, including current library lifecycle diagnostics.
pub fn analyze_project(model: &BuildingModel) -> Result<ProjectAnalysis, SolverError> {
    let library_lifecycle = library_lifecycle_status(model);
    let mut plan = generate_project_plan(model)?;
    let physical_scene = build_physical_scene(model, &plan);
    let geometry_audit = audit_physical_scene(&physical_scene);
    let resolved_standards = model.resolved_standards();
    let standards_evaluation =
        framer_standards::evaluate_detailed(model, &resolved_standards, &plan);
    let authored_intent = crate::lower::evaluate_authored_intent(model, &resolved_standards, &plan);
    plan.diagnostics.extend(standards_evaluation.diagnostics());
    append_library_diagnostics(&mut plan, &library_lifecycle);
    plan.diagnostics
        .extend(crate::lower::current_intent_plan_diagnostics(
            model,
            &geometry_audit,
        ));
    plan.diagnostics
        .extend(crate::lower::authored_intent_plan_diagnostics(
            &authored_intent,
        ));
    let intent_report = GraphRevision::for_model(model)
        .map_err(AnalysisError::from)
        .map(|revision| {
            crate::lower::compile_project_intent(
                model,
                &plan,
                &geometry_audit,
                &standards_evaluation,
                &authored_intent,
                revision,
            )
        });
    let graph = match &intent_report {
        Ok(intent_report) => compile_project_graph_with_intent(
            model,
            &plan,
            &resolved_standards,
            &physical_scene,
            &geometry_audit,
            &standards_evaluation.report,
            intent_report,
        ),
        Err(error) => Err(error.clone()),
    };

    Ok(ProjectAnalysis {
        plan,
        resolved_standards,
        physical_scene,
        geometry_audit,
        standards_evaluation,
        library_lifecycle,
        intent_report,
        graph,
    })
}

fn append_library_diagnostics(plan: &mut ProjectFramePlan, status: &LibraryLifecycleStatus) {
    if let Some(error) = status.error.as_deref() {
        plan.diagnostics.push(PlanDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "library.lifecycle.check-failed".to_owned(),
            source: None,
            message: format!("Library lifecycle status could not be checked: {error}."),
            rule: None,
        });
        return;
    }

    for issue in &status.issues {
        let item_kind = match &issue.item {
            LibraryItem::Material(_) => "material",
            LibraryItem::System(_) => "system",
            LibraryItem::Furnishing(_) => "furnishing",
            LibraryItem::MepObject(_) => "MEP object",
            LibraryItem::StandardsPack(_) => "standards pack",
        };
        let code = match issue.kind {
            LibraryIssueKind::Diverged => "library.item.diverged",
            LibraryIssueKind::OutOfDate => "library.item.out-of-date",
            LibraryIssueKind::SourceMissing => "library.item.source-missing",
        };
        let message = match issue.kind {
            LibraryIssueKind::Diverged => format!(
                "Library {item_kind} '{}' has local edits; detach it to keep those edits or re-sync it to overwrite them from the source library.",
                issue.item_id().0
            ),
            LibraryIssueKind::OutOfDate => format!(
                "Library {item_kind} '{}' is out of date with source item '{}'.",
                issue.item_id().0,
                issue.source_id.0
            ),
            LibraryIssueKind::SourceMissing => format!(
                "Library {item_kind} '{}' references source item '{}' which is not present in the available library.",
                issue.item_id().0,
                issue.source_id.0
            ),
        };
        plan.diagnostics.push(PlanDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: code.to_owned(),
            source: Some(issue.item_id().clone()),
            message,
            rule: None,
        });
    }
}

#[cfg(test)]
fn compile_project_graph(
    model: &BuildingModel,
    plan: &ProjectFramePlan,
    resolved: &ResolvedStandards,
    physical_scene: &PhysicalScene,
    geometry_audit: &GeometryAudit,
    compliance_report: &ComplianceReport,
) -> Result<ProjectGraph, AnalysisError> {
    let revision = GraphRevision::for_model(model)?;
    let intent_report = crate::lower::compile_current_intent(model, revision);
    compile_project_graph_with_intent(
        model,
        plan,
        resolved,
        physical_scene,
        geometry_audit,
        compliance_report,
        &intent_report,
    )
}

#[allow(clippy::too_many_arguments)]
fn compile_project_graph_with_intent(
    model: &BuildingModel,
    plan: &ProjectFramePlan,
    resolved: &ResolvedStandards,
    physical_scene: &PhysicalScene,
    geometry_audit: &GeometryAudit,
    compliance_report: &ComplianceReport,
    intent_report: &IntentReport,
) -> Result<ProjectGraph, AnalysisError> {
    let revision = intent_report.revision();
    let mut compiler = Compiler {
        model,
        plan,
        resolved,
        physical_scene,
        geometry_audit,
        compliance_report,
        intent_report,
        revision,
        graph: GraphBuilder::new(revision),
        authored_by_id: BTreeMap::new(),
        library_versions: BTreeMap::new(),
        members: BTreeMap::new(),
        diagnostics_by_lowering_key: BTreeMap::new(),
    };
    compiler.compile();
    Ok(compiler.graph.finish()?)
}

struct Compiler<'a> {
    model: &'a BuildingModel,
    plan: &'a ProjectFramePlan,
    resolved: &'a ResolvedStandards,
    physical_scene: &'a PhysicalScene,
    geometry_audit: &'a GeometryAudit,
    compliance_report: &'a ComplianceReport,
    intent_report: &'a IntentReport,
    revision: GraphRevision,
    graph: GraphBuilder,
    authored_by_id: BTreeMap<ElementId, AuthoredEntityRef>,
    library_versions: BTreeMap<(String, String), LibraryVersionRef>,
    members: BTreeMap<(ElementId, String, MemberKind), GeneratedMemberRef>,
    diagnostics_by_lowering_key: BTreeMap<(String, Option<ElementId>, String), Vec<DiagnosticRef>>,
}

impl Compiler<'_> {
    fn compile(&mut self) {
        self.compile_authored_nodes();
        self.compile_authored_relationships();
        self.compile_standards_rules();
        self.compile_room_consequences();
        self.compile_members();
        self.compile_physical_bodies();
        self.compile_plan_diagnostics();
        self.compile_compliance_report();
        self.compile_geometry_audit();
        self.compile_intent_assertions();
    }

    fn compile_authored_nodes(&mut self) {
        self.graph.node(ProjectNode::new(
            ProjectNodeRef::Project,
            "Project",
            Some(format!("Analysis revision {}", self.revision)),
        ));
        self.add_authored(
            AuthoredEntityRef::Site,
            "Site context".to_owned(),
            Some(if self.model.site.jurisdiction.is_empty() {
                "No jurisdiction set".to_owned()
            } else {
                self.model.site.jurisdiction.clone()
            }),
        );

        for library in &self.model.libraries {
            let reference = LibraryVersionRef::new(&library.uid, &library.version_id);
            self.library_versions.insert(
                (library.uid.clone(), library.version_id.clone()),
                reference.clone(),
            );
            self.add_authored(
                AuthoredEntityRef::LibraryVersion(reference),
                format!("Library {} {}", library.coordinate, library.version),
                Some(library.content_hash.clone()),
            );
        }
        for pack in &self.model.standards_packs {
            self.add_authored(
                AuthoredEntityRef::StandardsPack(pack.id.clone()),
                pack.name.clone(),
                Some(format!("Standards edition {}", pack.edition)),
            );
        }
        for material in &self.model.materials {
            self.add_authored(
                AuthoredEntityRef::Material(material.id.clone()),
                material.name.clone(),
                Some("Material".to_owned()),
            );
        }
        for system in &self.model.systems {
            self.add_authored(
                AuthoredEntityRef::ConstructionSystem(system.id.clone()),
                system.name.clone(),
                Some(format!("{:?} construction system", system.kind)),
            );
        }
        for family in &self.model.furnishings {
            self.add_authored(
                AuthoredEntityRef::Furnishing(family.id.clone()),
                family.name.clone(),
                Some("Furnishing family".to_owned()),
            );
        }
        for family in &self.model.mep_objects {
            self.add_authored(
                AuthoredEntityRef::MepObject(family.id.clone()),
                family.name.clone(),
                Some(format!("{:?} MEP family", family.kind)),
            );
        }
        for level in &self.model.levels {
            self.add_authored(
                AuthoredEntityRef::Level(level.id.clone()),
                level.name.clone(),
                Some(format!("Elevation {}", level.elevation)),
            );
        }
        for wall in &self.model.walls {
            self.add_authored(
                AuthoredEntityRef::Wall(wall.id.clone()),
                wall.name.clone(),
                Some(format!("Wall {} long", wall.length)),
            );
            for opening in &wall.openings {
                self.add_authored(
                    AuthoredEntityRef::Opening(opening.id.clone()),
                    opening.name.clone(),
                    Some(format!("{:?} opening", opening.kind)),
                );
            }
            for dimension in &wall.dimensions {
                self.add_authored(
                    AuthoredEntityRef::Dimension(dimension.id.clone()),
                    dimension.name.clone(),
                    Some(format!(
                        "{:?} {:?} dimension",
                        dimension.kind, dimension.axis
                    )),
                );
            }
            for panel in &wall.bracing {
                self.add_authored(
                    AuthoredEntityRef::BracedPanel(panel.id.clone()),
                    format!("Braced panel {}", panel.id.0),
                    Some(format!("{} long, method {:?}", panel.length, panel.method)),
                );
            }
        }
        for join in &self.model.wall_joins {
            self.add_authored(
                AuthoredEntityRef::WallJoin(join.id.clone()),
                join.name.clone(),
                Some(format!("{:?} wall join", join.kind)),
            );
        }
        for room in &self.model.rooms {
            self.add_authored(
                AuthoredEntityRef::Room(room.id.clone()),
                room.name.clone(),
                Some(format!("{} room", room.usage.label())),
            );
        }
        for instance in &self.model.furnishing_instances {
            self.add_authored(
                AuthoredEntityRef::FurnishingInstance(instance.id.clone()),
                instance.name.clone(),
                Some("Placed furnishing".to_owned()),
            );
        }
        for instance in &self.model.mep_instances {
            self.add_authored(
                AuthoredEntityRef::MepInstance(instance.id.clone()),
                instance.name.clone(),
                Some("Placed MEP object".to_owned()),
            );
        }
        for roof in &self.model.roof_planes {
            self.add_authored(
                AuthoredEntityRef::RoofPlane(roof.id.clone()),
                roof.name.clone(),
                Some("Roof plane".to_owned()),
            );
            for opening in &roof.openings {
                self.add_authored(
                    AuthoredEntityRef::RoofOpening(opening.id.clone()),
                    format!("Roof opening {}", opening.id.0),
                    Some(format!("{:?}", opening.kind)),
                );
            }
        }
        for ceiling in &self.model.ceilings {
            self.add_authored(
                AuthoredEntityRef::Ceiling(ceiling.id.clone()),
                ceiling.name.clone(),
                Some("Ceiling surface".to_owned()),
            );
        }
        for floor in &self.model.floor_decks {
            self.add_authored(
                AuthoredEntityRef::FloorDeck(floor.id.clone()),
                floor.name.clone(),
                Some("Floor deck".to_owned()),
            );
        }
        for line in &self.model.braced_wall_lines {
            self.add_authored(
                AuthoredEntityRef::BracedWallLine(line.id.clone()),
                line.name.clone(),
                Some("Braced wall line".to_owned()),
            );
        }
        for intent_override in &self.model.intent_overrides {
            let IntentOverride::Waive {
                id, target, reason, ..
            } = intent_override;
            self.add_authored(
                AuthoredEntityRef::IntentOverride(id.clone()),
                format!("Intent waiver {}", id.0.0),
                Some(format!("Waives {}: {reason}", target.0.0)),
            );
        }
    }

    fn compile_authored_relationships(&mut self) {
        let project = ProjectNodeRef::Project;
        let site = ProjectNodeRef::Authored(AuthoredEntityRef::Site);
        self.graph.edge(ProjectEdge::new(
            site,
            project.clone(),
            RelationshipKind::BelongsTo,
        ));

        let authored = self.authored_by_id.values().cloned().collect::<Vec<_>>();
        for reference in authored {
            self.graph.edge(ProjectEdge::new(
                ProjectNodeRef::Authored(reference),
                project.clone(),
                RelationshipKind::BelongsTo,
            ));
        }
        for reference in self.library_versions.values() {
            self.graph.edge(ProjectEdge::new(
                ProjectNodeRef::Authored(AuthoredEntityRef::LibraryVersion(reference.clone())),
                project.clone(),
                RelationshipKind::BelongsTo,
            ));
        }

        for (order, pack) in self.model.standards.iter().enumerate() {
            self.add_id_dependency(
                project.clone(),
                pack,
                RelationshipKind::UsesStandardsPack,
                Some(order),
                UnknownEvidenceKind::AuthoredEntity,
            );
        }
        for pack in &self.model.standards_packs {
            if let Some(source) = &pack.source {
                self.add_provenance(
                    ProjectNodeRef::Authored(AuthoredEntityRef::StandardsPack(pack.id.clone())),
                    source,
                );
            }
        }
        for material in &self.model.materials {
            if let MaterialSource::Library(source) = &material.source {
                self.add_provenance(
                    ProjectNodeRef::Authored(AuthoredEntityRef::Material(material.id.clone())),
                    source,
                );
            }
        }
        for system in &self.model.systems {
            let system_ref =
                ProjectNodeRef::Authored(AuthoredEntityRef::ConstructionSystem(system.id.clone()));
            if let Some(source) = &system.source {
                self.add_provenance(system_ref.clone(), source);
            }
            for (order, layer) in system.layers.iter().enumerate() {
                self.add_id_dependency(
                    system_ref.clone(),
                    &layer.material,
                    RelationshipKind::UsesMaterial,
                    Some(order),
                    UnknownEvidenceKind::AuthoredEntity,
                );
                if let Some(cavity) = layer
                    .framing
                    .as_ref()
                    .and_then(|framing| framing.cavity_material.as_ref())
                {
                    self.add_id_dependency(
                        system_ref.clone(),
                        cavity,
                        RelationshipKind::UsesMaterial,
                        Some(order),
                        UnknownEvidenceKind::AuthoredEntity,
                    );
                }
            }
        }
        for family in &self.model.furnishings {
            if let Some(source) = &family.source {
                self.add_provenance(
                    ProjectNodeRef::Authored(AuthoredEntityRef::Furnishing(family.id.clone())),
                    source,
                );
            }
        }
        for family in &self.model.mep_objects {
            if let Some(source) = &family.source {
                self.add_provenance(
                    ProjectNodeRef::Authored(AuthoredEntityRef::MepObject(family.id.clone())),
                    source,
                );
            }
        }

        for wall in &self.model.walls {
            let wall_ref = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(wall.id.clone()));
            self.add_id_dependency(
                wall_ref.clone(),
                &wall.level,
                RelationshipKind::BelongsTo,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
            self.add_id_dependency(
                wall_ref.clone(),
                &wall.system,
                RelationshipKind::UsesSystem,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
            for opening in &wall.openings {
                self.graph.edge(ProjectEdge::new(
                    ProjectNodeRef::Authored(AuthoredEntityRef::Opening(opening.id.clone())),
                    wall_ref.clone(),
                    RelationshipKind::BelongsTo,
                ));
            }
            for dimension in &wall.dimensions {
                let dimension_ref =
                    ProjectNodeRef::Authored(AuthoredEntityRef::Dimension(dimension.id.clone()));
                self.graph.edge(ProjectEdge::new(
                    dimension_ref.clone(),
                    wall_ref.clone(),
                    RelationshipKind::BelongsTo,
                ));
                for opening in [
                    dimension_anchor_opening(&dimension.start),
                    dimension_anchor_opening(&dimension.end),
                ]
                .into_iter()
                .flatten()
                {
                    self.add_id_dependency(
                        dimension_ref.clone(),
                        opening,
                        RelationshipKind::References,
                        None,
                        UnknownEvidenceKind::AuthoredEntity,
                    );
                }
            }
            for panel in &wall.bracing {
                self.graph.edge(ProjectEdge::new(
                    ProjectNodeRef::Authored(AuthoredEntityRef::BracedPanel(panel.id.clone())),
                    wall_ref.clone(),
                    RelationshipKind::BelongsTo,
                ));
            }
        }
        for join in &self.model.wall_joins {
            let join_ref = ProjectNodeRef::Authored(AuthoredEntityRef::WallJoin(join.id.clone()));
            for wall in [&join.first_wall, &join.second_wall] {
                self.add_id_dependency(
                    join_ref.clone(),
                    wall,
                    RelationshipKind::References,
                    None,
                    UnknownEvidenceKind::AuthoredEntity,
                );
            }
        }
        for room in &self.model.rooms {
            self.add_id_dependency(
                ProjectNodeRef::Authored(AuthoredEntityRef::Room(room.id.clone())),
                &room.level,
                RelationshipKind::BelongsTo,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
        }
        for instance in &self.model.furnishing_instances {
            let instance_ref = ProjectNodeRef::Authored(AuthoredEntityRef::FurnishingInstance(
                instance.id.clone(),
            ));
            self.add_id_dependency(
                instance_ref.clone(),
                &instance.level,
                RelationshipKind::BelongsTo,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
            self.add_id_dependency(
                instance_ref,
                &instance.family,
                RelationshipKind::UsesFamily,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
        }
        for instance in &self.model.mep_instances {
            let instance_ref =
                ProjectNodeRef::Authored(AuthoredEntityRef::MepInstance(instance.id.clone()));
            self.add_id_dependency(
                instance_ref.clone(),
                &instance.level,
                RelationshipKind::BelongsTo,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
            self.add_id_dependency(
                instance_ref,
                &instance.family,
                RelationshipKind::UsesFamily,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
        }
        for roof in &self.model.roof_planes {
            let roof_ref = ProjectNodeRef::Authored(AuthoredEntityRef::RoofPlane(roof.id.clone()));
            self.add_id_dependency(
                roof_ref.clone(),
                &roof.level,
                RelationshipKind::BelongsTo,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
            self.add_id_dependency(
                roof_ref.clone(),
                &roof.system,
                RelationshipKind::UsesSystem,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
            for opening in &roof.openings {
                self.graph.edge(ProjectEdge::new(
                    ProjectNodeRef::Authored(AuthoredEntityRef::RoofOpening(opening.id.clone())),
                    roof_ref.clone(),
                    RelationshipKind::BelongsTo,
                ));
            }
        }
        for ceiling in &self.model.ceilings {
            let ceiling_ref =
                ProjectNodeRef::Authored(AuthoredEntityRef::Ceiling(ceiling.id.clone()));
            self.add_surface_dependencies(
                ceiling_ref,
                &ceiling.level,
                &ceiling.system,
                &ceiling.region,
            );
        }
        for floor in &self.model.floor_decks {
            let floor_ref =
                ProjectNodeRef::Authored(AuthoredEntityRef::FloorDeck(floor.id.clone()));
            self.add_surface_dependencies(floor_ref, &floor.level, &floor.system, &floor.region);
        }
        for line in &self.model.braced_wall_lines {
            self.add_id_dependency(
                ProjectNodeRef::Authored(AuthoredEntityRef::BracedWallLine(line.id.clone())),
                &line.level,
                RelationshipKind::BelongsTo,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
        }
    }

    fn compile_standards_rules(&mut self) {
        for rule in &self.resolved.rules {
            let reference = StandardsRuleRef::resolved(rule.pack.clone(), rule.rule.clone());
            let rule_node = self.add_rule_node(
                reference,
                &rule.citation,
                Some(format!("Resolved standards rule {}", rule.rule)),
            );
            for (order, (pack, _action)) in rule.chain.iter().enumerate() {
                self.add_id_dependency(
                    rule_node.clone(),
                    pack,
                    RelationshipKind::References,
                    Some(order),
                    UnknownEvidenceKind::AuthoredEntity,
                );
            }
        }
    }

    fn compile_members(&mut self) {
        let mut members = Vec::new();
        for host in &self.plan.wall_plans {
            members.extend(
                host.members
                    .iter()
                    .map(|member| (AuthoredEntityRef::Wall(host.wall.clone()), member)),
            );
        }
        for host in &self.plan.floor_plans {
            members.extend(
                host.members
                    .iter()
                    .map(|member| (AuthoredEntityRef::FloorDeck(host.floor.clone()), member)),
            );
        }
        for host in &self.plan.ceiling_plans {
            members.extend(
                host.members
                    .iter()
                    .map(|member| (AuthoredEntityRef::Ceiling(host.ceiling.clone()), member)),
            );
        }
        for host in &self.plan.roof_plans {
            members.extend(
                host.members
                    .iter()
                    .map(|member| (AuthoredEntityRef::RoofPlane(host.roof.clone()), member)),
            );
        }

        for (host, member) in members {
            self.add_member(host, member);
        }
    }

    fn add_member(&mut self, host: AuthoredEntityRef, member: &FrameMember) {
        let reference =
            GeneratedMemberRef::new(self.revision, host.clone(), member.id.clone(), member.kind);
        self.members.insert(
            (
                host.element_id()
                    .expect("generated member hosts have element ids")
                    .clone(),
                member.id.clone(),
                member.kind,
            ),
            reference.clone(),
        );
        let member_node = ProjectNodeRef::GeneratedMember(reference.clone());
        self.graph.node(ProjectNode::new(
            member_node.clone(),
            format!("{}: {}", member.kind.label(), member.id),
            Some(member.provenance.summary.clone()),
        ));
        let host_id = host
            .element_id()
            .expect("generated member hosts have element ids");
        if self.authored_by_id.get(host_id) == Some(&host) {
            self.graph.edge(ProjectEdge::new(
                member_node.clone(),
                ProjectNodeRef::Authored(host.clone()),
                RelationshipKind::HostedBy,
            ));
        } else {
            let found = self
                .authored_by_id
                .get(host_id)
                .map_or("missing", AuthoredEntityRef::kind_label);
            self.add_unknown_dependency(
                member_node.clone(),
                UnknownEvidenceKind::GeneratedHost,
                format!(
                    "expected {} {} but found {found}",
                    host.kind_label(),
                    host_id.0
                ),
            );
        }

        match self.authored_by_id.get(&member.source).cloned() {
            Some(source) if generated_source_matches(&host, &source) => {
                self.graph.edge(ProjectEdge::new(
                    member_node.clone(),
                    ProjectNodeRef::Authored(source),
                    RelationshipKind::GeneratedFrom,
                ));
            }
            Some(source) => self.add_unknown_dependency(
                member_node.clone(),
                UnknownEvidenceKind::GeneratedSource,
                format!(
                    "expected source for {} {} but found {} {}",
                    host.kind_label(),
                    host_id.0,
                    source.kind_label(),
                    member.source.0
                ),
            ),
            None => self.add_unknown_dependency(
                member_node.clone(),
                UnknownEvidenceKind::GeneratedSource,
                format!("missing {}", member.source.0),
            ),
        }

        let provenance = SolverProvenanceRef {
            revision: self.revision,
            member: reference,
            rule_id: member.provenance.rule_id.clone(),
        };
        let provenance_node = ProjectNodeRef::SolverProvenance(provenance);
        self.graph.node(ProjectNode::new(
            provenance_node.clone(),
            format!("Solver rule: {}", member.provenance.rule_id),
            Some(member.provenance.summary.clone()),
        ));
        self.graph.edge(ProjectEdge::new(
            provenance_node.clone(),
            ProjectNodeRef::Authored(AuthoredEntityRef::Site),
            RelationshipKind::References,
        ));
        self.graph.edge(ProjectEdge::new(
            member_node,
            provenance_node.clone(),
            RelationshipKind::JustifiedBy,
        ));
        if let Some(rule) = self
            .resolved
            .rules
            .iter()
            .find(|rule| rule.rule == member.provenance.rule_id)
        {
            let rule_node = self.add_rule_node(
                StandardsRuleRef::resolved(rule.pack.clone(), rule.rule.clone()),
                &rule.citation,
                None,
            );
            self.graph.edge(ProjectEdge::new(
                provenance_node,
                rule_node,
                RelationshipKind::References,
            ));
        }
    }

    fn compile_room_consequences(&mut self) {
        let rooms = self.model.rooms.iter().collect::<Vec<_>>();
        let boundaries = room_boundaries_for_rooms(self.model, &rooms);
        let schedules = self
            .plan
            .rooms
            .iter()
            .map(|schedule| (&schedule.room, schedule))
            .collect::<BTreeMap<_, _>>();

        for (room, boundary) in rooms.into_iter().zip(boundaries) {
            let room_node = ProjectNodeRef::Authored(AuthoredEntityRef::Room(room.id.clone()));
            let boundary_node = self.room_consequence_node(
                &room.id,
                RoomConsequenceKind::Boundary,
                "Room boundary",
                boundary.as_ref().map_or_else(
                    || "Open or unresolved room boundary".to_owned(),
                    |boundary| {
                        format!(
                            "{} vertices, {} perimeter, {} sq in",
                            boundary.vertices.len(),
                            boundary.perimeter,
                            boundary.area_square_inches().round() as i64
                        )
                    },
                ),
            );
            self.graph.edge(ProjectEdge::new(
                boundary_node.clone(),
                room_node.clone(),
                RelationshipKind::GeneratedFrom,
            ));

            if let Some(boundary) = boundary {
                for (edge_ordinal, [start, end]) in boundary_edges(&boundary.vertices).enumerate() {
                    let mut matched = false;
                    for wall in self.model.walls.iter().filter(|wall| {
                        wall.level == room.level && wall_contains_edge(wall, start, end)
                    }) {
                        matched = true;
                        self.graph.edge(ProjectEdge::new(
                            boundary_node.clone(),
                            ProjectNodeRef::Authored(AuthoredEntityRef::Wall(wall.id.clone())),
                            RelationshipKind::GeneratedFrom,
                        ));
                    }
                    if !matched {
                        self.add_unknown_dependency(
                            boundary_node.clone(),
                            UnknownEvidenceKind::RoomBoundaryWall,
                            format!("{} boundary edge {edge_ordinal}", room.id.0),
                        );
                    }
                }
            } else {
                self.add_unknown_dependency(
                    boundary_node.clone(),
                    UnknownEvidenceKind::RoomBoundary,
                    room.id.0.clone(),
                );
            }

            let schedule = schedules.get(&room.id).copied();
            let schedule_node = self.room_consequence_node(
                &room.id,
                RoomConsequenceKind::Schedule,
                "Room schedule",
                schedule.map_or_else(
                    || "Room schedule unavailable".to_owned(),
                    room_schedule_detail,
                ),
            );
            self.graph.edge(ProjectEdge::new(
                schedule_node.clone(),
                room_node,
                RelationshipKind::GeneratedFrom,
            ));
            self.graph.edge(ProjectEdge::new(
                schedule_node.clone(),
                boundary_node,
                RelationshipKind::EvaluatedFrom,
            ));
            if schedule.is_none() {
                self.add_unknown_dependency(
                    schedule_node,
                    UnknownEvidenceKind::RoomSchedule,
                    room.id.0.clone(),
                );
            }
        }

        for schedule in &self.plan.rooms {
            if self.model.rooms.iter().any(|room| room.id == schedule.room) {
                continue;
            }
            let schedule_node = self.room_consequence_node(
                &schedule.room,
                RoomConsequenceKind::Schedule,
                "Room schedule",
                room_schedule_detail(schedule),
            );
            self.add_unknown_dependency(
                schedule_node,
                UnknownEvidenceKind::AuthoredEntity,
                schedule.room.0.clone(),
            );
        }
    }

    fn room_consequence_node(
        &mut self,
        room: &ElementId,
        kind: RoomConsequenceKind,
        title: &str,
        detail: String,
    ) -> ProjectNodeRef {
        let node = ProjectNodeRef::RoomConsequence(RoomConsequenceRef::new(
            self.revision,
            room.clone(),
            kind,
        ));
        self.graph.node(ProjectNode::new(
            node.clone(),
            format!("{title}: {}", room.0),
            Some(detail),
        ));
        node
    }

    fn compile_physical_bodies(&mut self) {
        for body in self.physical_scene.bodies() {
            let body_ref = PhysicalBodyRef::new(self.revision, body.body_ref.clone());
            let body_node = ProjectNodeRef::PhysicalBody(body_ref);
            self.graph.node(ProjectNode::new(
                body_node.clone(),
                body.body_ref.to_string(),
                Some(format!("{:?}", body.body_ref.kind())),
            ));
            self.link_body_mapping(body_node, &body.body_ref);
        }
    }

    fn compile_plan_diagnostics(&mut self) {
        for (diagnostic, reference) in
            crate::lower::canonical_plan_diagnostic_records(self.model, self.plan, self.revision)
        {
            let node = ProjectNodeRef::Diagnostic(reference.clone());
            self.graph.node(ProjectNode::new(
                node.clone(),
                format!(
                    "{}: {}",
                    severity_label(diagnostic.severity),
                    diagnostic.code
                ),
                Some(diagnostic.message.clone()),
            ));
            match &diagnostic.source {
                Some(id) => {
                    if let Some(source) = self.authored_by_id.get(id) {
                        self.graph.edge(ProjectEdge::new(
                            node.clone(),
                            ProjectNodeRef::Authored(source.clone()),
                            RelationshipKind::EvaluatedFrom,
                        ));
                    } else {
                        self.add_unknown_dependency(
                            node.clone(),
                            UnknownEvidenceKind::DiagnosticSource,
                            id.0.clone(),
                        );
                    }
                }
                None => self.graph.edge(ProjectEdge::new(
                    node.clone(),
                    ProjectNodeRef::Project,
                    RelationshipKind::EvaluatedFrom,
                )),
            }
            if let Some(rule) = &diagnostic.rule {
                let rule_ref = StandardsRuleRef::resolved(rule.pack.clone(), rule.rule.clone());
                let rule_node = self.add_rule_node(rule_ref.clone(), &rule.citation, None);
                self.graph.edge(ProjectEdge::new(
                    node.clone(),
                    rule_node.clone(),
                    RelationshipKind::JustifiedBy,
                ));
                self.mark_unresolved_rule(&rule_ref, rule_node);
            }
            self.diagnostics_by_lowering_key
                .entry((diagnostic.code, diagnostic.source, diagnostic.message))
                .or_default()
                .push(reference);
        }
    }

    fn compile_compliance_report(&mut self) {
        let mut entries = self.compliance_report.entries.clone();
        entries.sort_by(compare_compliance_entry);
        let mut ordinals = BTreeMap::<(StandardsRuleRef, Option<AuthoredEntityRef>), u32>::new();
        let mut lowering_cursor = BTreeMap::<(String, Option<ElementId>, String), usize>::new();

        for entry in entries {
            let subject = entry
                .element
                .as_ref()
                .and_then(|id| self.authored_by_id.get(id))
                .cloned();
            let rule_ref = StandardsRuleRef::resolved(entry.pack.clone(), entry.rule.clone());
            let ordinal = ordinals
                .entry((rule_ref.clone(), subject.clone()))
                .or_default();
            let entry_ref = ComplianceEntryRef {
                revision: self.revision,
                rule: rule_ref.clone(),
                subject: subject.clone(),
                ordinal: *ordinal,
            };
            *ordinal = ordinal.saturating_add(1);
            let entry_node = ProjectNodeRef::ComplianceEntry(entry_ref);
            self.graph.node(ProjectNode::new(
                entry_node.clone(),
                format!(
                    "Compliance {}: {}",
                    outcome_label(&entry.outcome),
                    entry.rule
                ),
                Some(format!("{} — {}", entry.citation, entry.message)),
            ));
            let rule_node = self.add_rule_node(
                rule_ref.clone(),
                &entry.citation,
                Some(format!("Compliance rule {}", entry.rule)),
            );
            self.graph.edge(ProjectEdge::new(
                entry_node.clone(),
                rule_node.clone(),
                RelationshipKind::JustifiedBy,
            ));
            self.graph.edge(ProjectEdge::new(
                entry_node.clone(),
                ProjectNodeRef::Authored(AuthoredEntityRef::Site),
                RelationshipKind::EvaluatedFrom,
            ));
            self.mark_unresolved_rule(&rule_ref, rule_node);
            match (&entry.element, subject) {
                (Some(_), Some(subject)) => self.graph.edge(ProjectEdge::new(
                    entry_node.clone(),
                    ProjectNodeRef::Authored(subject),
                    RelationshipKind::EvaluatedFrom,
                )),
                (Some(id), None) => self.add_unknown_dependency(
                    entry_node.clone(),
                    UnknownEvidenceKind::DiagnosticSource,
                    id.0.clone(),
                ),
                (None, _) => self.graph.edge(ProjectEdge::new(
                    entry_node.clone(),
                    ProjectNodeRef::Project,
                    RelationshipKind::EvaluatedFrom,
                )),
            }

            let lowering_key = (
                entry.rule.clone(),
                entry.element.clone(),
                entry.message.clone(),
            );
            let cursor = lowering_cursor.entry(lowering_key.clone()).or_default();
            if let Some(diagnostic) = self
                .diagnostics_by_lowering_key
                .get(&lowering_key)
                .and_then(|matches| matches.get(*cursor))
            {
                self.graph.edge(ProjectEdge::new(
                    ProjectNodeRef::Diagnostic(diagnostic.clone()),
                    entry_node,
                    RelationshipKind::LoweredFrom,
                ));
                *cursor = cursor.saturating_add(1);
            }
        }
    }

    fn compile_geometry_audit(&mut self) {
        for (violation, reference) in
            crate::lower::canonical_geometry_records(self.model, self.geometry_audit, self.revision)
        {
            let diagnostic_node = ProjectNodeRef::Diagnostic(reference);
            self.graph.node(ProjectNode::new(
                diagnostic_node.clone(),
                violation.code(),
                Some(violation.to_string()),
            ));
            for body in [Some(violation.body_a()), violation.body_b()]
                .into_iter()
                .flatten()
            {
                let body_node = self.ensure_body_node(body);
                self.graph.edge(ProjectEdge::new(
                    diagnostic_node.clone(),
                    body_node,
                    RelationshipKind::EvaluatedFrom,
                ));
            }
        }
    }

    fn compile_intent_assertions(&mut self) {
        // Assertions are compiled after every current evidence family so edges never depend on
        // source-vector order and missing evidence can fail closed explicitly.
        let records = self.intent_report.records().to_vec();
        for record in &records {
            let assertion = record.assertion();
            let assertion_node = ProjectNodeRef::Assertion(assertion.reference.clone());
            self.graph.node(ProjectNode::new(
                assertion_node,
                intent_record_title(record),
                Some(intent_record_detail(record)),
            ));
        }

        for record in records {
            let assertion = record.assertion();
            let assertion_node = ProjectNodeRef::Assertion(assertion.reference.clone());
            for participant in &assertion.participants {
                let participant_node = ProjectNodeRef::Authored(participant.entity.clone());
                if self.graph.contains_node(&participant_node) {
                    self.graph.edge(
                        ProjectEdge::new(
                            assertion_node.clone(),
                            participant_node,
                            RelationshipKind::AppliesTo,
                        )
                        .ordered(participant.semantic_order as usize),
                    );
                } else {
                    self.add_unknown_dependency(
                        assertion_node.clone(),
                        UnknownEvidenceKind::AuthoredEntity,
                        format!("{:?}", participant.entity),
                    );
                }
            }

            for evidence in record.evidence() {
                let evidence_node = intent_evidence_node(evidence);
                if self.graph.contains_node(&evidence_node) {
                    let diagnostic_is_source = matches!(
                        (&assertion.source, evidence),
                        (
                            crate::AssertionSource::Diagnostic(source),
                            IntentEvidenceRef::Diagnostic(evidence)
                        ) if source == evidence
                    );
                    if matches!(evidence, IntentEvidenceRef::Diagnostic(_)) && !diagnostic_is_source
                    {
                        self.graph.edge(ProjectEdge::new(
                            evidence_node,
                            assertion_node.clone(),
                            RelationshipKind::LoweredFrom,
                        ));
                    } else {
                        self.graph.edge(ProjectEdge::new(
                            assertion_node.clone(),
                            evidence_node,
                            RelationshipKind::EvaluatedFrom,
                        ));
                    }
                } else if let Some(kind) = unknown_kind_for_intent_evidence(evidence) {
                    self.add_unknown_dependency(
                        assertion_node.clone(),
                        kind,
                        format!("{evidence:?}"),
                    );
                }
            }

            if let IntentRecord::Boolean(boolean) = &record
                && let IntentOutcome::Waived { waiver, .. } = &boolean.outcome
            {
                let waiver_node = match waiver {
                    crate::WaiverRef::Project { override_id } => ProjectNodeRef::Authored(
                        AuthoredEntityRef::IntentOverride(override_id.clone()),
                    ),
                    crate::WaiverRef::Standards { overlay_pack, .. } => ProjectNodeRef::Authored(
                        AuthoredEntityRef::StandardsPack(overlay_pack.clone()),
                    ),
                };
                if self.graph.contains_node(&waiver_node) {
                    self.graph.edge(ProjectEdge::new(
                        assertion_node,
                        waiver_node,
                        RelationshipKind::WaivedBy,
                    ));
                }
            }
        }
    }

    fn add_authored(
        &mut self,
        reference: AuthoredEntityRef,
        title: String,
        detail: Option<String>,
    ) {
        if let Some(id) = reference.element_id() {
            self.authored_by_id.insert(id.clone(), reference.clone());
        }
        self.graph.node(ProjectNode::new(
            ProjectNodeRef::Authored(reference),
            title,
            detail,
        ));
    }

    fn add_surface_dependencies(
        &mut self,
        surface: ProjectNodeRef,
        level: &ElementId,
        system: &ElementId,
        region: &SurfaceRegion,
    ) {
        self.add_id_dependency(
            surface.clone(),
            level,
            RelationshipKind::BelongsTo,
            None,
            UnknownEvidenceKind::AuthoredEntity,
        );
        self.add_id_dependency(
            surface.clone(),
            system,
            RelationshipKind::UsesSystem,
            None,
            UnknownEvidenceKind::AuthoredEntity,
        );
        if let SurfaceRegion::Room(room) = region {
            self.add_id_dependency(
                surface,
                room,
                RelationshipKind::References,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
        }
    }

    fn add_provenance(&mut self, dependent: ProjectNodeRef, provenance: &Provenance) {
        let key = (
            provenance.library_uid.clone(),
            provenance.version_id.clone(),
        );
        if let Some(reference) = self.library_versions.get(&key) {
            self.graph.edge(ProjectEdge::new(
                dependent,
                ProjectNodeRef::Authored(AuthoredEntityRef::LibraryVersion(reference.clone())),
                RelationshipKind::VendoredFrom,
            ));
        } else {
            self.add_unknown_dependency(
                dependent,
                UnknownEvidenceKind::AuthoredEntity,
                format!("library {}/{}", key.0, key.1),
            );
        }
    }

    fn add_id_dependency(
        &mut self,
        dependent: ProjectNodeRef,
        dependency: &ElementId,
        relationship: RelationshipKind,
        semantic_order: Option<usize>,
        missing_kind: UnknownEvidenceKind,
    ) {
        if let Some(reference) = self.authored_by_id.get(dependency) {
            let mut edge = ProjectEdge::new(
                dependent,
                ProjectNodeRef::Authored(reference.clone()),
                relationship,
            );
            if let Some(order) = semantic_order {
                edge = edge.ordered(order);
            }
            self.graph.edge(edge);
        } else {
            self.add_unknown_dependency(dependent, missing_kind, dependency.0.clone());
        }
    }

    fn add_unknown_dependency(
        &mut self,
        dependent: ProjectNodeRef,
        kind: UnknownEvidenceKind,
        identity: String,
    ) {
        let unknown = ProjectNodeRef::UnknownEvidence(UnknownEvidenceRef::new(
            self.revision,
            kind,
            identity.clone(),
        ));
        self.graph.node(ProjectNode::new(
            unknown.clone(),
            format!("Unresolved {kind:?}"),
            Some(identity),
        ));
        self.graph.edge(ProjectEdge::new(
            dependent,
            unknown,
            RelationshipKind::UnresolvedEvidence,
        ));
    }

    fn add_rule_node(
        &mut self,
        reference: StandardsRuleRef,
        citation: &str,
        detail: Option<String>,
    ) -> ProjectNodeRef {
        let title = reference.rule.clone();
        let node = ProjectNodeRef::StandardsRule(reference.clone());
        self.graph.node(ProjectNode::new(
            node.clone(),
            title,
            Some(match detail {
                Some(detail) if !citation.is_empty() => format!("{citation} — {detail}"),
                Some(detail) => detail,
                None => citation.to_owned(),
            }),
        ));
        if let Some(pack) = &reference.pack {
            self.add_id_dependency(
                node.clone(),
                pack,
                RelationshipKind::BelongsTo,
                None,
                UnknownEvidenceKind::AuthoredEntity,
            );
        }
        node
    }

    fn mark_unresolved_rule(&mut self, reference: &StandardsRuleRef, node: ProjectNodeRef) {
        let is_resolved = reference.pack.as_ref().is_some_and(|pack| {
            self.resolved
                .rules
                .iter()
                .any(|rule| rule.pack == *pack && rule.rule == reference.rule)
        });
        if !is_resolved {
            let identity = reference.pack.as_ref().map_or_else(
                || reference.rule.clone(),
                |pack| format!("{}/{}", pack.0, reference.rule),
            );
            self.add_unknown_dependency(node, UnknownEvidenceKind::StandardsRule, identity);
        }
    }

    fn link_body_mapping(&mut self, body_node: ProjectNodeRef, body: &BodyRef) {
        match body.kind() {
            BodyKind::Assembly(kind) => {
                if let Some(owner) = self
                    .authored_by_id
                    .get(body.owner())
                    .filter(|owner| assembly_owner_matches(owner, kind))
                    .cloned()
                {
                    self.graph.edge(ProjectEdge::new(
                        body_node,
                        ProjectNodeRef::Authored(owner),
                        RelationshipKind::PhysicalFormOf,
                    ));
                } else {
                    self.add_unknown_dependency(
                        body_node,
                        UnknownEvidenceKind::PhysicalOwner,
                        body.owner().0.clone(),
                    );
                }
            }
            BodyKind::FrameMember(kind) => {
                let member = body.member_id().and_then(|member_id| {
                    self.members
                        .get(&(body.owner().clone(), member_id.to_owned(), kind))
                });
                if let Some(member) = member {
                    self.graph.edge(ProjectEdge::new(
                        body_node,
                        ProjectNodeRef::GeneratedMember(member.clone()),
                        RelationshipKind::PhysicalFormOf,
                    ));
                } else {
                    self.add_unknown_dependency(
                        body_node,
                        UnknownEvidenceKind::GeneratedMember,
                        body.to_string(),
                    );
                }
            }
        }
    }

    fn ensure_body_node(&mut self, body: &BodyRef) -> ProjectNodeRef {
        let reference = PhysicalBodyRef::new(self.revision, body.clone());
        let node = ProjectNodeRef::PhysicalBody(reference);
        if self.graph_node_missing(&node) {
            self.graph.node(ProjectNode::new(
                node.clone(),
                body.to_string(),
                Some("Referenced by the geometry audit but absent from the physical scene".into()),
            ));
            self.link_body_mapping(node.clone(), body);
            self.add_unknown_dependency(
                node.clone(),
                UnknownEvidenceKind::PhysicalBody,
                body.to_string(),
            );
        }
        node
    }

    fn graph_node_missing(&self, _node: &ProjectNodeRef) -> bool {
        !self.graph.contains_node(_node)
    }
}

fn dimension_anchor_opening(anchor: &DimensionAnchor) -> Option<&ElementId> {
    match anchor {
        DimensionAnchor::OpeningLeft { opening }
        | DimensionAnchor::OpeningCenter { opening }
        | DimensionAnchor::OpeningRight { opening }
        | DimensionAnchor::OpeningPoint { opening, .. } => Some(opening),
        DimensionAnchor::WallStart
        | DimensionAnchor::WallEnd
        | DimensionAnchor::WallPoint { .. } => None,
    }
}

fn room_schedule_detail(schedule: &RoomSchedule) -> String {
    let enclosure = if schedule.closed { "closed" } else { "open" };
    format!(
        "{enclosure}, {} sq in, {} perimeter",
        schedule.area_square_inches, schedule.perimeter
    )
}

fn boundary_edges(vertices: &[Point2]) -> impl Iterator<Item = [Point2; 2]> + '_ {
    vertices
        .iter()
        .copied()
        .zip(vertices.iter().copied().cycle().skip(1))
        .take(vertices.len())
        .map(|(start, end)| [start, end])
}

fn wall_contains_edge(wall: &Wall, edge_start: Point2, edge_end: Point2) -> bool {
    point_on_segment(edge_start, wall.start, wall.end)
        && point_on_segment(edge_end, wall.start, wall.end)
}

fn point_on_segment(point: Point2, start: Point2, end: Point2) -> bool {
    let px = i128::from(point.x.ticks());
    let py = i128::from(point.y.ticks());
    let sx = i128::from(start.x.ticks());
    let sy = i128::from(start.y.ticks());
    let ex = i128::from(end.x.ticks());
    let ey = i128::from(end.y.ticks());
    let cross = (px - sx) * (ey - sy) - (py - sy) * (ex - sx);
    cross == 0 && px >= sx.min(ex) && px <= sx.max(ex) && py >= sy.min(ey) && py <= sy.max(ey)
}

fn assembly_owner_matches(owner: &AuthoredEntityRef, kind: AssemblyKind) -> bool {
    matches!(
        (owner, kind),
        (AuthoredEntityRef::Wall(_), AssemblyKind::Wall)
            | (AuthoredEntityRef::FloorDeck(_), AssemblyKind::FloorDeck)
            | (AuthoredEntityRef::Ceiling(_), AssemblyKind::Ceiling)
            | (AuthoredEntityRef::RoofPlane(_), AssemblyKind::RoofPlane)
    )
}

fn generated_source_matches(host: &AuthoredEntityRef, source: &AuthoredEntityRef) -> bool {
    match host {
        AuthoredEntityRef::Wall(_) => matches!(
            source,
            AuthoredEntityRef::Wall(_)
                | AuthoredEntityRef::Opening(_)
                | AuthoredEntityRef::WallJoin(_)
        ),
        AuthoredEntityRef::FloorDeck(_) => matches!(source, AuthoredEntityRef::FloorDeck(_)),
        AuthoredEntityRef::Ceiling(_) => matches!(source, AuthoredEntityRef::Ceiling(_)),
        AuthoredEntityRef::RoofPlane(_) => matches!(source, AuthoredEntityRef::RoofPlane(_)),
        AuthoredEntityRef::Site
        | AuthoredEntityRef::LibraryVersion(_)
        | AuthoredEntityRef::StandardsPack(_)
        | AuthoredEntityRef::Material(_)
        | AuthoredEntityRef::ConstructionSystem(_)
        | AuthoredEntityRef::Furnishing(_)
        | AuthoredEntityRef::MepObject(_)
        | AuthoredEntityRef::Level(_)
        | AuthoredEntityRef::Opening(_)
        | AuthoredEntityRef::Dimension(_)
        | AuthoredEntityRef::WallJoin(_)
        | AuthoredEntityRef::Room(_)
        | AuthoredEntityRef::FurnishingInstance(_)
        | AuthoredEntityRef::MepInstance(_)
        | AuthoredEntityRef::RoofOpening(_)
        | AuthoredEntityRef::BracedWallLine(_)
        | AuthoredEntityRef::BracedPanel(_)
        | AuthoredEntityRef::IntentOverride(_) => false,
    }
}

fn severity_label(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Info => "Info",
        DiagnosticSeverity::Warning => "Warning",
        DiagnosticSeverity::Unsupported => "Unsupported",
        DiagnosticSeverity::Violation => "Violation",
        DiagnosticSeverity::NeedsReview => "Needs review",
    }
}

fn intent_evidence_node(evidence: &IntentEvidenceRef) -> ProjectNodeRef {
    match evidence {
        IntentEvidenceRef::Project => ProjectNodeRef::Project,
        IntentEvidenceRef::Assertion(reference) => ProjectNodeRef::Assertion(reference.clone()),
        IntentEvidenceRef::Authored(reference) => ProjectNodeRef::Authored(reference.clone()),
        IntentEvidenceRef::GeneratedMember(reference) => {
            ProjectNodeRef::GeneratedMember(reference.clone())
        }
        IntentEvidenceRef::PhysicalBody(reference) => {
            ProjectNodeRef::PhysicalBody(reference.clone())
        }
        IntentEvidenceRef::StandardsRule(reference) => {
            ProjectNodeRef::StandardsRule(reference.clone())
        }
        IntentEvidenceRef::ComplianceEntry(reference) => {
            ProjectNodeRef::ComplianceEntry(reference.clone())
        }
        IntentEvidenceRef::Diagnostic(reference) => ProjectNodeRef::Diagnostic(reference.clone()),
    }
}

fn unknown_kind_for_intent_evidence(evidence: &IntentEvidenceRef) -> Option<UnknownEvidenceKind> {
    match evidence {
        IntentEvidenceRef::Project => None,
        IntentEvidenceRef::Assertion(_) => Some(UnknownEvidenceKind::Assertion),
        IntentEvidenceRef::Authored(_) => Some(UnknownEvidenceKind::AuthoredEntity),
        IntentEvidenceRef::GeneratedMember(_) => Some(UnknownEvidenceKind::GeneratedMember),
        IntentEvidenceRef::PhysicalBody(_) => Some(UnknownEvidenceKind::PhysicalBody),
        IntentEvidenceRef::StandardsRule(_) => Some(UnknownEvidenceKind::StandardsRule),
        IntentEvidenceRef::ComplianceEntry(_) => Some(UnknownEvidenceKind::ComplianceEntry),
        IntentEvidenceRef::Diagnostic(_) => Some(UnknownEvidenceKind::Diagnostic),
    }
}

fn intent_record_title(record: &IntentRecord) -> String {
    match record {
        IntentRecord::Boolean(record) => format!("{:?} assertion", record.mode),
        IntentRecord::Objective(record) => {
            format!("Objective: {}", record.objective.component)
        }
        IntentRecord::Assumption(record) => format!("Assumption: {}", record.premise.label),
    }
}

fn intent_record_detail(record: &IntentRecord) -> String {
    match record {
        IntentRecord::Boolean(record) => match &record.predicate_observation {
            Some(observation) => format!(
                "{:?} — {} — predicate observation: {observation:?}",
                record.outcome, record.assertion.rationale
            ),
            None => format!("{:?} — {}", record.outcome, record.assertion.rationale),
        },
        IntentRecord::Objective(record) => {
            format!("{:?} — {}", record.observation, record.assertion.rationale)
        }
        IntentRecord::Assumption(record) => {
            format!("{:?} — {}", record.evidence, record.assertion.rationale)
        }
    }
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

fn outcome_label(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Pass => "Pass",
        Outcome::Violation => "Violation",
        Outcome::Advisory => "Advisory",
        Outcome::NeedsReview => "Needs review",
        Outcome::NotApplicable => "Not applicable",
        Outcome::Waived { .. } => "Waived",
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
    use std::collections::BTreeSet;
    use std::time::Instant;

    use framer_core::{
        BracedPanel, BracedWallLine, BracingMethod, BuildingModel, Ceiling, DimensionAnchor,
        DimensionConstraint, DimensionDirection, DimensionKind, ElementId, FloorDeck, Furnishing,
        FurnishingInstance, Length, LibraryStamp, MepInstance, MepObject, MepObjectKind,
        OpeningKind, Point2, RoofOpening, RoofPlane, Room, RoomUsage, Slope, SurfaceRegion,
    };
    use framer_geometry::{GeometryBuildDiagnostic, GeometryViolation};
    use framer_solver::{DiagnosticSeverity, PlanDiagnostic, RuleRef};

    use super::*;
    use crate::{
        AssertionRef, AssertionScope, AssertionSource, BooleanExpression, BooleanIntentMode,
        BooleanIntentRecord, CompiledAssertion, DerivedAssertionId, DerivedAssertionProvider,
        DerivedAssertionRole, DerivedAssertionSource, GraphQueryCache, IntentDomain,
        ProjectNodeRef, SiteAssumptionKey,
    };

    fn analysis(model: &BuildingModel) -> ProjectAnalysis {
        analyze_project(model).expect("fixture should analyze")
    }

    fn rectangle(width: Length, depth: Length) -> Vec<Point2> {
        vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(width, Length::ZERO),
            Point2::new(width, depth),
            Point2::new(Length::ZERO, depth),
        ]
    }

    fn all_families_model() -> BuildingModel {
        let mut model = BuildingModel::demo_shell();
        let ft = Length::from_feet;

        model.libraries.push(LibraryStamp {
            uid: "library.example".to_owned(),
            version_id: "version-1".to_owned(),
            content_hash: "test-hash".to_owned(),
            coordinate: "example/library".to_owned(),
            version: "1.0.0".to_owned(),
        });
        model.furnishings.push(Furnishing::new(
            "furnishing-1",
            "Chair",
            ft(2.0),
            ft(2.0),
            ft(3.0),
        ));
        model.mep_objects.push(MepObject::new(
            "mep-object-1",
            "Outlet",
            MepObjectKind::Electrical,
            Length::from_inches(4.0),
            Length::from_inches(2.0),
            Length::from_inches(4.0),
        ));
        model.furnishing_instances.push(FurnishingInstance::new(
            "furnishing-instance-1",
            "Chair 1",
            "furnishing-1",
            "level-1",
            Point2::new(ft(4.0), ft(4.0)),
        ));
        model.mep_instances.push(MepInstance::new(
            "mep-instance-1",
            "Outlet 1",
            "mep-object-1",
            "level-1",
            Point2::new(ft(6.0), ft(4.0)),
        ));
        model.rooms.push(Room::new(
            "room-1",
            "Room",
            RoomUsage::Living,
            "level-1",
            Point2::new(ft(10.0), ft(10.0)),
        ));
        model.walls[0].dimensions.push(DimensionConstraint::new(
            "dimension-1",
            "Wall length",
            DimensionKind::Reference,
            DimensionAnchor::WallStart,
            DimensionAnchor::WallEnd,
            DimensionDirection::Forward,
            None,
        ));
        model.walls[0].bracing.push(BracedPanel {
            id: ElementId::new("braced-panel-1"),
            offset: ft(10.0),
            length: ft(4.0),
            method: BracingMethod::Wsp,
        });
        model.braced_wall_lines.push(BracedWallLine {
            id: ElementId::new("braced-wall-line-1"),
            name: "Front braced line".to_owned(),
            level: ElementId::new("level-1"),
            start: Point2::new(Length::ZERO, Length::ZERO),
            end: Point2::new(ft(28.0), Length::ZERO),
        });

        let outline = rectangle(ft(12.0), ft(10.0));
        let mut roof = RoofPlane::new(
            "roof-1",
            "Roof",
            "level-1",
            "system-roof-1",
            outline.clone(),
            Slope::new(Length::from_inches(4.0), Length::from_inches(12.0)),
            0,
            ft(8.0),
        );
        roof.openings.push(RoofOpening::new(
            "roof-opening-1",
            OpeningKind::Window,
            Point2::new(ft(4.0), ft(4.0)),
            ft(2.0),
            ft(2.0),
        ));
        model.roof_planes.push(roof);
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling-1",
            SurfaceRegion::Polygon(outline.clone()),
            ft(8.0),
        ));
        model.floor_decks.push(FloorDeck::new(
            "floor-1",
            "Floor",
            "level-1",
            "system-floor-1",
            SurfaceRegion::Polygon(outline),
        ));
        model.sort_deterministically();
        model.validate().expect("all-family fixture is valid");
        model
    }

    #[test]
    fn library_lifecycle_diagnostics_are_installed_in_the_plan_and_graph_together() {
        let mut model = BuildingModel::new();
        let loaded = framer_library::starter_library_ref().unwrap();
        let source = loaded
            .library
            .materials
            .first()
            .expect("starter library material");
        let imported = framer_library::import_material(
            &mut model,
            &loaded.library,
            &loaded.content_hash,
            &source.id,
        )
        .unwrap();
        let material_id = imported.materials[0].clone();
        model
            .materials
            .iter_mut()
            .find(|material| material.id == material_id)
            .unwrap()
            .tags
            .push("local-divergence".to_owned());

        let analysis = analysis(&model);
        assert!(analysis.library_lifecycle.error.is_none());
        assert!(analysis.library_lifecycle.issues.iter().any(|issue| {
            issue.kind == LibraryIssueKind::Diverged && issue.item_id() == &material_id
        }));
        assert!(analysis.plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "library.item.diverged"
                && diagnostic.source.as_ref() == Some(&material_id)
        }));

        let graph = analysis.graph.as_ref().unwrap();
        let material = ProjectNodeRef::Authored(AuthoredEntityRef::Material(material_id));
        let diagnostic = graph
            .nodes()
            .iter()
            .find_map(|node| match &node.reference {
                ProjectNodeRef::Diagnostic(reference)
                    if reference.provider == DiagnosticProvider::Library
                        && reference.code == "library.item.diverged" =>
                {
                    Some(node.reference.clone())
                }
                _ => None,
            })
            .expect("library diagnostic graph node");
        assert!(graph.edges().iter().any(|edge| {
            edge.dependent == diagnostic
                && edge.dependency == material
                && edge.relationship == RelationshipKind::EvaluatedFrom
        }));
    }

    #[test]
    fn compiles_equal_graphs_with_exact_opening_header_rule_chain() {
        let model = BuildingModel::demo_wall();
        let first = analysis(&model);
        let second = analysis(&model);
        let graph = first.graph.as_ref().unwrap();
        assert_eq!(graph, second.graph.as_ref().unwrap());

        let member = graph
            .generated_member("wall-1", "opening-door-1-header")
            .expect("header member node")
            .clone();
        let member_node = ProjectNodeRef::GeneratedMember(member);
        let opening =
            ProjectNodeRef::Authored(AuthoredEntityRef::Opening(ElementId::new("opening-door-1")));
        assert!(graph.edges().iter().any(|edge| {
            edge.dependent == member_node
                && edge.dependency == opening
                && edge.relationship == RelationshipKind::GeneratedFrom
        }));
        let provenance = graph.edges().iter().find_map(|edge| {
            (edge.dependent == member_node && edge.relationship == RelationshipKind::JustifiedBy)
                .then_some(&edge.dependency)
        });
        assert!(matches!(
            provenance,
            Some(ProjectNodeRef::SolverProvenance(reference))
                if reference.rule_id == "framer.starter.headers"
        ));
        assert!(graph.edges().iter().any(|edge| {
            edge.dependent == *provenance.unwrap()
                && matches!(
                    &edge.dependency,
                    ProjectNodeRef::StandardsRule(rule)
                        if rule.pack.as_ref().is_some_and(|pack| pack.0 == "std-framer-illustrative")
                            && rule.rule == "framer.starter.headers"
                )
        }));
    }

    #[test]
    fn compiles_every_current_authored_family() {
        let model = all_families_model();
        let analysis = analysis(&model);
        let graph = analysis.graph.as_ref().unwrap();
        let references = graph
            .nodes()
            .iter()
            .filter_map(|node| match &node.reference {
                ProjectNodeRef::Authored(reference) => Some(reference.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();

        let expected = [
            AuthoredEntityRef::Site,
            AuthoredEntityRef::LibraryVersion(LibraryVersionRef::new(
                "library.example",
                "version-1",
            )),
            AuthoredEntityRef::StandardsPack(model.standards_packs[0].id.clone()),
            AuthoredEntityRef::Material(model.materials[0].id.clone()),
            AuthoredEntityRef::ConstructionSystem(model.systems[0].id.clone()),
            AuthoredEntityRef::Furnishing(ElementId::new("furnishing-1")),
            AuthoredEntityRef::MepObject(ElementId::new("mep-object-1")),
            AuthoredEntityRef::Level(ElementId::new("level-1")),
            AuthoredEntityRef::Wall(ElementId::new("wall-front")),
            AuthoredEntityRef::Opening(ElementId::new("opening-front-door")),
            AuthoredEntityRef::Dimension(ElementId::new("dimension-1")),
            AuthoredEntityRef::WallJoin(ElementId::new("join-front-right")),
            AuthoredEntityRef::Room(ElementId::new("room-1")),
            AuthoredEntityRef::FurnishingInstance(ElementId::new("furnishing-instance-1")),
            AuthoredEntityRef::MepInstance(ElementId::new("mep-instance-1")),
            AuthoredEntityRef::RoofPlane(ElementId::new("roof-1")),
            AuthoredEntityRef::RoofOpening(ElementId::new("roof-opening-1")),
            AuthoredEntityRef::Ceiling(ElementId::new("ceiling-1")),
            AuthoredEntityRef::FloorDeck(ElementId::new("floor-1")),
            AuthoredEntityRef::BracedWallLine(ElementId::new("braced-wall-line-1")),
            AuthoredEntityRef::BracedPanel(ElementId::new("braced-panel-1")),
        ];
        for reference in expected {
            assert!(references.contains(&reference), "missing {reference:?}");
        }
    }

    #[test]
    fn compiles_members_and_physical_bodies_for_every_generated_host_family() {
        let model = all_families_model();
        let analysis = analysis(&model);
        let graph = analysis.graph.as_ref().unwrap();
        let host_kinds = graph
            .nodes()
            .iter()
            .filter_map(|node| match &node.reference {
                ProjectNodeRef::GeneratedMember(member) => Some(member.host.kind_label()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(
            host_kinds,
            BTreeSet::from(["wall", "floor deck", "ceiling", "roof plane"])
        );
        for node in graph.nodes() {
            let ProjectNodeRef::PhysicalBody(body) = &node.reference else {
                continue;
            };
            assert_eq!(body.revision, graph.revision());
            assert!(
                graph.edges().iter().any(|edge| {
                    edge.dependent == node.reference
                        && edge.relationship == RelationshipKind::PhysicalFormOf
                        && matches!(
                            edge.dependency,
                            ProjectNodeRef::Authored(_) | ProjectNodeRef::GeneratedMember(_)
                        )
                }),
                "unmapped physical body {:?}",
                body.body
            );
        }
    }

    #[test]
    fn missing_or_wrong_kind_member_source_becomes_explicit_unknown_evidence() {
        let model = BuildingModel::demo_wall();
        let resolved = model.resolved_standards();
        for (source, expected) in [
            (ElementId::new("missing-source"), "missing missing-source"),
            (model.levels[0].id.clone(), "found level level-1"),
        ] {
            let mut plan = generate_project_plan(&model).unwrap();
            plan.wall_plans[0].members[0].source = source;
            let scene = build_physical_scene(&model, &plan);
            let audit = audit_physical_scene(&scene);
            let report = framer_standards::evaluate(&model, &resolved, &plan);
            let graph =
                compile_project_graph(&model, &plan, &resolved, &scene, &audit, &report).unwrap();
            let member = graph
                .generated_member("wall-1", &plan.wall_plans[0].members[0].id)
                .unwrap();
            let member_node = ProjectNodeRef::GeneratedMember(member.clone());

            assert!(!graph.edges().iter().any(|edge| {
                edge.dependent == member_node
                    && edge.relationship == RelationshipKind::GeneratedFrom
            }));
            assert!(graph.edges().iter().any(|edge| {
                edge.dependent == member_node
                    && edge.relationship == RelationshipKind::UnresolvedEvidence
                    && matches!(
                        &edge.dependency,
                        ProjectNodeRef::UnknownEvidence(unknown)
                            if unknown.kind == UnknownEvidenceKind::GeneratedSource
                                && unknown.identity.contains(expected)
                    )
            }));
        }
    }

    #[test]
    fn missing_diagnostic_source_does_not_collide_with_project_diagnostic() {
        let model = BuildingModel::demo_wall();
        let mut plan = generate_project_plan(&model).unwrap();
        for source in [None, Some(ElementId::new("missing-source"))] {
            plan.diagnostics.push(PlanDiagnostic {
                severity: DiagnosticSeverity::NeedsReview,
                code: "test.same-code".to_owned(),
                source,
                message: "same message".to_owned(),
                rule: None,
            });
        }
        let host_diagnostic = PlanDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "test.same-code".to_owned(),
            source: Some(model.walls[0].id.clone()),
            message: "same host message".to_owned(),
            rule: None,
        };
        plan.diagnostics.push(host_diagnostic.clone());
        plan.wall_plans[0].diagnostics.push(host_diagnostic);
        let scene = build_physical_scene(&model, &plan);
        let audit = audit_physical_scene(&scene);
        let resolved = model.resolved_standards();
        let report = ComplianceReport::default();
        let graph =
            compile_project_graph(&model, &plan, &resolved, &scene, &audit, &report).unwrap();
        let diagnostics = graph
            .nodes()
            .iter()
            .filter_map(|node| match &node.reference {
                ProjectNodeRef::Diagnostic(reference)
                    if reference.provider == DiagnosticProvider::Solver
                        && reference.code == "test.same-code" =>
                {
                    Some(reference)
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(diagnostics.len(), 4);
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.source.is_none())
                .count(),
            2
        );
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.source.is_some())
                .count(),
            2
        );
        assert!(graph.edges().iter().any(|edge| {
            diagnostics.iter().any(|diagnostic| {
                edge.dependent == ProjectNodeRef::Diagnostic((*diagnostic).clone())
            }) && edge.relationship == RelationshipKind::UnresolvedEvidence
                && matches!(
                    &edge.dependency,
                    ProjectNodeRef::UnknownEvidence(unknown)
                        if unknown.kind == UnknownEvidenceKind::DiagnosticSource
                            && unknown.identity == "missing-source"
                )
        }));
    }

    #[test]
    fn unresolved_diagnostic_rule_is_explicit_unknown_evidence() {
        let model = BuildingModel::demo_wall();
        let mut plan = generate_project_plan(&model).unwrap();
        let pack = model.standards[0].clone();
        plan.diagnostics.push(PlanDiagnostic {
            severity: DiagnosticSeverity::NeedsReview,
            code: "missing.rule".to_owned(),
            source: Some(model.walls[0].id.clone()),
            message: "rule was not resolved".to_owned(),
            rule: Some(RuleRef {
                pack: pack.clone(),
                rule: "missing.rule".to_owned(),
                citation: "test".to_owned(),
            }),
        });
        let scene = build_physical_scene(&model, &plan);
        let audit = audit_physical_scene(&scene);
        let resolved = model.resolved_standards();
        let graph = compile_project_graph(
            &model,
            &plan,
            &resolved,
            &scene,
            &audit,
            &ComplianceReport::default(),
        )
        .unwrap();
        let rule = ProjectNodeRef::StandardsRule(StandardsRuleRef::resolved(pack, "missing.rule"));

        assert!(graph.edges().iter().any(|edge| {
            edge.dependent == rule
                && edge.relationship == RelationshipKind::UnresolvedEvidence
                && matches!(
                    &edge.dependency,
                    ProjectNodeRef::UnknownEvidence(unknown)
                        if unknown.kind == UnknownEvidenceKind::StandardsRule
                )
        }));
    }

    #[test]
    fn compliance_violation_links_to_lowered_plan_diagnostic() {
        let mut model = BuildingModel::demo_wall();
        model.walls[0].height = Length::from_feet(20.0);
        let analysis = analysis(&model);
        let graph = analysis.graph.as_ref().unwrap();
        let entry = graph.nodes().iter().find_map(|node| match &node.reference {
            ProjectNodeRef::ComplianceEntry(entry)
                if entry.rule.rule == "framer.starter.stud-height" =>
            {
                Some(node.reference.clone())
            }
            _ => None,
        });
        let entry = entry.expect("violation compliance entry");

        assert!(graph.edges().iter().any(|edge| {
            edge.dependency == entry
                && edge.relationship == RelationshipKind::LoweredFrom
                && matches!(edge.dependent, ProjectNodeRef::Diagnostic(_))
        }));
    }

    #[test]
    fn stale_generated_reference_does_not_resolve_in_new_graph() {
        let model = BuildingModel::demo_wall();
        let first = analysis(&model);
        let stale = first
            .graph
            .as_ref()
            .unwrap()
            .generated_member("wall-1", "opening-door-1-header")
            .unwrap()
            .clone();
        let mut changed = model;
        changed.site.jurisdiction = "Changed".to_owned();
        let second = analysis(&changed);
        let second_graph = second.graph.as_ref().unwrap();

        assert!(
            second_graph
                .node(&ProjectNodeRef::GeneratedMember(stale))
                .is_none()
        );
    }

    #[test]
    fn geometry_ordinals_are_independent_of_nonsemantic_audit_order() {
        let model = BuildingModel::demo_wall();
        let plan = generate_project_plan(&model).unwrap();
        let scene = build_physical_scene(&model, &plan);
        let resolved = model.resolved_standards();
        let report = ComplianceReport::default();
        let wall = model.walls[0].id.clone();
        let first_body = BodyRef::assembly(wall.clone(), AssemblyKind::Wall);
        let second_body = BodyRef::member(
            wall,
            plan.wall_plans[0].members[0].kind,
            plan.wall_plans[0].members[0].id.clone(),
        );
        let mut violations = vec![
            GeometryViolation::BodyUnbuildable(GeometryBuildDiagnostic::unbuildable(
                first_body, "first",
            )),
            GeometryViolation::BodyUnbuildable(GeometryBuildDiagnostic::unbuildable(
                second_body,
                "second",
            )),
        ];
        let first = compile_project_graph(
            &model,
            &plan,
            &resolved,
            &scene,
            &GeometryAudit {
                violations: violations.clone(),
            },
            &report,
        )
        .unwrap();
        violations.reverse();
        let reordered = compile_project_graph(
            &model,
            &plan,
            &resolved,
            &scene,
            &GeometryAudit { violations },
            &report,
        )
        .unwrap();

        assert_eq!(first, reordered);
    }

    #[test]
    fn nonsemantic_derived_vector_permutations_compile_equal_graphs() {
        let model = all_families_model();
        let plan = generate_project_plan(&model).unwrap();
        let scene = build_physical_scene(&model, &plan);
        let audit = audit_physical_scene(&scene);
        let resolved = model.resolved_standards();
        let report = framer_standards::evaluate(&model, &resolved, &plan);
        let first =
            compile_project_graph(&model, &plan, &resolved, &scene, &audit, &report).unwrap();

        let mut reordered_plan = plan.clone();
        reordered_plan.wall_plans.reverse();
        reordered_plan.floor_plans.reverse();
        reordered_plan.ceiling_plans.reverse();
        reordered_plan.roof_plans.reverse();
        reordered_plan.diagnostics.reverse();
        for members in reordered_plan
            .wall_plans
            .iter_mut()
            .map(|host| &mut host.members)
            .chain(
                reordered_plan
                    .floor_plans
                    .iter_mut()
                    .map(|host| &mut host.members),
            )
            .chain(
                reordered_plan
                    .ceiling_plans
                    .iter_mut()
                    .map(|host| &mut host.members),
            )
            .chain(
                reordered_plan
                    .roof_plans
                    .iter_mut()
                    .map(|host| &mut host.members),
            )
        {
            members.reverse();
        }
        let mut reordered_report = report.clone();
        reordered_report.entries.reverse();
        let reordered = compile_project_graph(
            &model,
            &reordered_plan,
            &resolved,
            &scene,
            &audit,
            &reordered_report,
        )
        .unwrap();

        assert_eq!(first, reordered);
    }

    #[test]
    fn audit_only_body_keeps_semantic_mapping_and_marks_scene_evidence_missing() {
        let model = BuildingModel::demo_wall();
        let plan = generate_project_plan(&model).unwrap();
        let member = &plan.wall_plans[0].members[0];
        let body = BodyRef::member(model.walls[0].id.clone(), member.kind, member.id.clone());
        let audit = GeometryAudit {
            violations: vec![GeometryViolation::BodyUnbuildable(
                GeometryBuildDiagnostic::unbuildable(body.clone(), "missing scene body"),
            )],
        };
        let resolved = model.resolved_standards();
        let graph = compile_project_graph(
            &model,
            &plan,
            &resolved,
            &PhysicalScene::default(),
            &audit,
            &ComplianceReport::default(),
        )
        .unwrap();
        let body_node =
            ProjectNodeRef::PhysicalBody(PhysicalBodyRef::new(graph.revision(), body.clone()));

        assert!(graph.edges().iter().any(|edge| {
            edge.dependent == body_node
                && edge.relationship == RelationshipKind::PhysicalFormOf
                && matches!(
                    &edge.dependency,
                    ProjectNodeRef::GeneratedMember(member_ref)
                        if member_ref.member_id == member.id
                )
        }));
        assert!(graph.edges().iter().any(|edge| {
            edge.dependent == body_node
                && edge.relationship == RelationshipKind::UnresolvedEvidence
                && matches!(
                    &edge.dependency,
                    ProjectNodeRef::UnknownEvidence(unknown)
                        if unknown.kind == UnknownEvidenceKind::PhysicalBody
                            && unknown.identity == body.to_string()
                )
        }));
    }

    #[test]
    fn stale_physical_body_reference_does_not_resolve_in_new_graph() {
        let model = BuildingModel::demo_wall();
        let first = analysis(&model);
        let stale = first
            .graph
            .as_ref()
            .unwrap()
            .nodes()
            .iter()
            .find_map(|node| match &node.reference {
                ProjectNodeRef::PhysicalBody(body) => Some(body.clone()),
                _ => None,
            })
            .expect("fixture has physical bodies");
        let mut changed = model;
        changed.site.jurisdiction = "Changed".to_owned();
        let second = analysis(&changed);

        assert!(
            second
                .graph
                .as_ref()
                .unwrap()
                .node(&ProjectNodeRef::PhysicalBody(stale))
                .is_none()
        );
    }

    #[test]
    fn first_and_cached_dependency_queries_match() {
        let model = BuildingModel::demo_wall();
        let analysis = analysis(&model);
        let graph = analysis.graph.as_ref().unwrap();
        let member = ProjectNodeRef::GeneratedMember(
            graph
                .generated_member("wall-1", "opening-door-1-header")
                .unwrap()
                .clone(),
        );
        let mut cache = GraphQueryCache::default();
        let first = cache.dependencies(graph, &member);
        let cached = cache.dependencies(graph, &member);

        assert_eq!(first, cached);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 1);
        assert!(first.iter().any(|trace| {
            trace.path.first().is_some_and(|step| {
                step.relationship == RelationshipKind::GeneratedFrom
                    || step.relationship == RelationshipKind::JustifiedBy
            })
        }));
    }

    #[test]
    fn authored_assertions_are_queryable_and_impact_is_filtered_and_cached() {
        let model = BuildingModel::demo_wall();
        let analysis = analysis(&model);
        let report = analysis.intent_report.as_ref().unwrap();
        let graph = analysis.graph.as_ref().unwrap();
        let wall_ref = AuthoredEntityRef::Wall(model.walls[0].id.clone());
        let wall_node = ProjectNodeRef::Authored(wall_ref.clone());

        let wall_assertions = report.assertions_for(&wall_ref);
        assert!(wall_assertions.iter().any(|record| {
            matches!(
                record,
                IntentRecord::Boolean(crate::BooleanIntentRecord {
                    expression: crate::BooleanExpression::SelectedEntity { .. },
                    outcome: IntentOutcome::Satisfied,
                    ..
                })
            )
        }));
        assert!(wall_assertions.iter().all(|record| {
            graph
                .node(&ProjectNodeRef::Assertion(
                    record.assertion().reference.clone(),
                ))
                .is_some()
        }));
        assert!(graph.edges().iter().any(|edge| {
            edge.dependency == wall_node
                && matches!(edge.dependent, ProjectNodeRef::Assertion(_))
                && edge.relationship == RelationshipKind::AppliesTo
        }));

        let mut cache = GraphQueryCache::default();
        let first = cache.impact_of(graph, &wall_node);
        let cached = cache.impact_of(graph, &wall_node);
        assert_eq!(first, cached);
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 1);
        assert!(!first.assertions.is_empty());
        assert!(first.derived_results.iter().all(|trace| matches!(
            trace.node,
            ProjectNodeRef::GeneratedMember(_)
                | ProjectNodeRef::RoomConsequence(_)
                | ProjectNodeRef::PhysicalBody(_)
                | ProjectNodeRef::ComplianceEntry(_)
                | ProjectNodeRef::Diagnostic(_)
        )));
    }

    #[test]
    fn missing_common_evidence_compiles_to_exact_unknown_families() {
        let model = BuildingModel::demo_wall();
        let plan = generate_project_plan(&model).unwrap();
        let resolved = model.resolved_standards();
        let scene = build_physical_scene(&model, &plan);
        let audit = audit_physical_scene(&scene);
        let revision = GraphRevision::for_model(&model).unwrap();
        let owner = AssertionRef::Derived(DerivedAssertionId::new(
            revision,
            DerivedAssertionProvider::Analysis,
            DerivedAssertionSource::Project,
            DerivedAssertionRole::Diagnostic {
                provider: DiagnosticProvider::Analysis,
                code: "test.missing-evidence".to_owned(),
                ordinal: 0,
            },
        ));
        let absent_assertion = AssertionRef::Derived(DerivedAssertionId::new(
            revision,
            DerivedAssertionProvider::Standards,
            DerivedAssertionSource::Project,
            DerivedAssertionRole::Diagnostic {
                provider: DiagnosticProvider::Standards,
                code: "test.absent-assertion".to_owned(),
                ordinal: 0,
            },
        ));
        let absent_compliance = ComplianceEntryRef {
            revision,
            rule: StandardsRuleRef::resolved(model.standards[0].clone(), "test.absent-compliance"),
            subject: Some(AuthoredEntityRef::Wall(model.walls[0].id.clone())),
            ordinal: 0,
        };
        let absent_diagnostic = DiagnosticRef {
            revision,
            provider: DiagnosticProvider::Solver,
            code: "test.absent-diagnostic".to_owned(),
            source: Some(AuthoredEntityRef::Wall(model.walls[0].id.clone())),
            ordinal: 0,
        };
        let intent = IntentReport::from_parts(
            revision,
            vec![IntentRecord::Boolean(BooleanIntentRecord {
                assertion: CompiledAssertion {
                    reference: owner.clone(),
                    domain: IntentDomain::Construction,
                    scope: AssertionScope::Project,
                    participants: Vec::new(),
                    source: AssertionSource::Project,
                    rationale: "Missing evidence test".to_owned(),
                },
                mode: BooleanIntentMode::Requirement,
                expression: BooleanExpression::Finding {
                    code: "test.missing-evidence".to_owned(),
                },
                outcome: IntentOutcome::Unknown(crate::IntentUnknown {
                    kind: crate::IntentUnknownKind::EvaluationUnavailable,
                    detail: "evidence is absent".to_owned(),
                }),
                predicate_observation: None,
                evidence: vec![
                    IntentEvidenceRef::Assertion(absent_assertion),
                    IntentEvidenceRef::ComplianceEntry(absent_compliance),
                    IntentEvidenceRef::Diagnostic(absent_diagnostic),
                ],
            })],
            Vec::new(),
        );
        let graph = compile_project_graph_with_intent(
            &model,
            &plan,
            &resolved,
            &scene,
            &audit,
            &ComplianceReport::default(),
            &intent,
        )
        .unwrap();
        let owner_node = ProjectNodeRef::Assertion(owner);
        let expected = BTreeSet::from([
            UnknownEvidenceKind::Assertion,
            UnknownEvidenceKind::ComplianceEntry,
            UnknownEvidenceKind::Diagnostic,
        ]);
        let actual = graph
            .edges()
            .iter()
            .filter_map(|edge| {
                (edge.dependent == owner_node
                    && edge.relationship == RelationshipKind::UnresolvedEvidence)
                    .then_some(&edge.dependency)
            })
            .filter_map(|dependency| match dependency {
                ProjectNodeRef::UnknownEvidence(unknown) => Some(unknown.kind),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);

        let mut cache = GraphQueryCache::default();
        let queried = cache
            .evidence_for(&graph, &owner_node)
            .into_iter()
            .filter_map(|trace| match trace.node {
                ProjectNodeRef::UnknownEvidence(unknown) => Some(unknown.kind),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(queried, expected);
    }

    #[test]
    fn site_impact_reaches_solver_and_compliance_consequences() {
        let model = BuildingModel::demo_wall();
        let analysis = analysis(&model);
        let graph = analysis.graph.as_ref().unwrap();
        let site = ProjectNodeRef::Authored(AuthoredEntityRef::Site);
        let mut cache = GraphQueryCache::default();
        let impact = cache.dependents(graph, &site);

        assert!(impact.iter().any(|trace| {
            matches!(trace.node, ProjectNodeRef::SolverProvenance(_))
                && trace.path.first().is_some_and(|step| {
                    step.relationship == RelationshipKind::References && !step.toward_dependency
                })
        }));
        assert!(
            impact
                .iter()
                .any(|trace| matches!(trace.node, ProjectNodeRef::GeneratedMember(_)))
        );
        assert!(impact.iter().any(|trace| {
            matches!(trace.node, ProjectNodeRef::ComplianceEntry(_))
                && trace.path.first().is_some_and(|step| {
                    step.relationship == RelationshipKind::EvaluatedFrom && !step.toward_dependency
                })
        }));
    }

    #[test]
    fn standards_evidence_reaches_typed_site_assumptions() {
        let model = BuildingModel::demo_wall();
        let analysis = analysis(&model);
        let report = analysis.intent_report.as_ref().unwrap();
        let graph = analysis.graph.as_ref().unwrap();
        let (standards_assertion, site_assumption) = report
            .records()
            .iter()
            .find_map(|record| {
                let AssertionRef::Derived(assertion) = &record.assertion().reference else {
                    return None;
                };
                if assertion.provider != DerivedAssertionProvider::Standards {
                    return None;
                }
                record
                    .evidence()
                    .iter()
                    .find_map(|evidence| match evidence {
                        IntentEvidenceRef::Assertion(
                            reference @ AssertionRef::Derived(assumption),
                        ) if assumption.role
                            == DerivedAssertionRole::SiteAssumption(
                                SiteAssumptionKey::GroundSnowLoad,
                            ) =>
                        {
                            Some((record.assertion().reference.clone(), reference.clone()))
                        }
                        _ => None,
                    })
            })
            .expect("header-span intent should cite the ground-snow-load premise");

        let standards_node = ProjectNodeRef::Assertion(standards_assertion);
        let assumption_node = ProjectNodeRef::Assertion(site_assumption);
        let mut cache = GraphQueryCache::default();
        let evidence = cache.evidence_for(graph, &standards_node);
        assert!(evidence.iter().any(|trace| {
            trace.node == assumption_node
                && trace.path.first().is_some_and(|step| {
                    step.relationship == RelationshipKind::EvaluatedFrom && step.toward_dependency
                })
        }));
        assert!(evidence.iter().any(|trace| {
            trace.node == ProjectNodeRef::Authored(AuthoredEntityRef::Site)
                && trace.path.iter().all(|step| step.toward_dependency)
        }));
    }

    #[test]
    fn derived_from_excludes_host_ownership_but_keeps_generation_evidence() {
        let model = BuildingModel::demo_wall();
        let analysis = analysis(&model);
        let graph = analysis.graph.as_ref().unwrap();
        let member = ProjectNodeRef::GeneratedMember(
            graph
                .generated_member("wall-1", "opening-door-1-header")
                .unwrap()
                .clone(),
        );
        let host = ProjectNodeRef::Authored(AuthoredEntityRef::Wall(ElementId::new("wall-1")));
        let source =
            ProjectNodeRef::Authored(AuthoredEntityRef::Opening(ElementId::new("opening-door-1")));
        let mut cache = GraphQueryCache::default();
        let traces = cache.derived_from(graph, &member);

        assert!(traces.iter().all(|trace| trace.node != host));
        assert!(traces.iter().any(|trace| {
            trace.node == source && trace.path[0].relationship == RelationshipKind::GeneratedFrom
        }));
        assert!(traces.iter().any(|trace| {
            matches!(trace.node, ProjectNodeRef::SolverProvenance(_))
                && trace.path[0].relationship == RelationshipKind::JustifiedBy
        }));
    }

    #[test]
    fn evidence_for_member_never_walks_into_body_or_diagnostic_consequences() {
        let model = BuildingModel::demo_wall();
        let analysis = analysis(&model);
        let graph = analysis.graph.as_ref().unwrap();
        let member = ProjectNodeRef::GeneratedMember(
            graph
                .generated_member("wall-1", "opening-door-1-header")
                .unwrap()
                .clone(),
        );
        let mut cache = GraphQueryCache::default();
        let traces = cache.evidence_for(graph, &member);

        assert!(traces.iter().any(|trace| {
            matches!(
                trace.node,
                ProjectNodeRef::Authored(AuthoredEntityRef::Opening(_))
            )
        }));
        assert!(
            traces
                .iter()
                .any(|trace| { matches!(trace.node, ProjectNodeRef::SolverProvenance(_)) })
        );
        assert!(
            traces
                .iter()
                .any(|trace| { matches!(trace.node, ProjectNodeRef::StandardsRule(_)) })
        );
        assert!(traces.iter().all(|trace| {
            !matches!(
                trace.node,
                ProjectNodeRef::PhysicalBody(_) | ProjectNodeRef::Diagnostic(_)
            )
        }));
        assert!(traces.iter().all(|trace| {
            trace.path.iter().all(|step| {
                step.toward_dependency && step.relationship != RelationshipKind::HostedBy
            })
        }));
    }

    #[test]
    fn generated_members_fail_closed_for_missing_or_wrong_kind_hosts() {
        let model = BuildingModel::demo_wall();
        let base_plan = generate_project_plan(&model).unwrap();
        let resolved = model.resolved_standards();
        for (host, expected_found) in [
            (ElementId::new("missing-wall"), "missing"),
            (model.levels[0].id.clone(), "level"),
        ] {
            let mut plan = base_plan.clone();
            plan.wall_plans[0].wall = host.clone();
            let graph = compile_project_graph(
                &model,
                &plan,
                &resolved,
                &PhysicalScene::default(),
                &GeometryAudit::default(),
                &ComplianceReport::default(),
            )
            .unwrap();
            let member = graph
                .nodes()
                .iter()
                .find_map(|node| match &node.reference {
                    ProjectNodeRef::GeneratedMember(member)
                        if member.host == AuthoredEntityRef::Wall(host.clone()) =>
                    {
                        Some(node.reference.clone())
                    }
                    _ => None,
                })
                .expect("member retains requested typed host identity");

            assert!(!graph.edges().iter().any(|edge| {
                edge.dependent == member && edge.relationship == RelationshipKind::HostedBy
            }));
            assert!(graph.edges().iter().any(|edge| {
                edge.dependent == member
                    && edge.relationship == RelationshipKind::UnresolvedEvidence
                    && matches!(
                        &edge.dependency,
                        ProjectNodeRef::UnknownEvidence(unknown)
                            if unknown.kind == UnknownEvidenceKind::GeneratedHost
                                && unknown.identity.contains(expected_found)
                    )
            }));
            assert!(graph.edges().iter().all(|edge| {
                graph.node(&edge.dependent).is_some() && graph.node(&edge.dependency).is_some()
            }));
        }
    }

    #[test]
    fn closed_room_consequences_link_schedule_boundary_and_bounding_walls() {
        let model = all_families_model();
        let analysis = analysis(&model);
        let graph = analysis.graph.as_ref().unwrap();
        let boundary = ProjectNodeRef::RoomConsequence(RoomConsequenceRef::new(
            graph.revision(),
            ElementId::new("room-1"),
            RoomConsequenceKind::Boundary,
        ));
        let schedule = ProjectNodeRef::RoomConsequence(RoomConsequenceRef::new(
            graph.revision(),
            ElementId::new("room-1"),
            RoomConsequenceKind::Schedule,
        ));
        let bounding_walls = graph
            .edges()
            .iter()
            .filter_map(|edge| {
                (edge.dependent == boundary && edge.relationship == RelationshipKind::GeneratedFrom)
                    .then(|| match &edge.dependency {
                        ProjectNodeRef::Authored(AuthoredEntityRef::Wall(wall)) => {
                            Some(wall.clone())
                        }
                        _ => None,
                    })
                    .flatten()
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(bounding_walls.len(), 4);
        assert!(graph.edges().iter().any(|edge| {
            edge.dependent == schedule
                && edge.dependency == boundary
                && edge.relationship == RelationshipKind::EvaluatedFrom
        }));
        assert!(!graph.edges().iter().any(|edge| {
            edge.dependent == boundary
                && matches!(
                    &edge.dependency,
                    ProjectNodeRef::UnknownEvidence(unknown)
                        if matches!(
                            unknown.kind,
                            UnknownEvidenceKind::RoomBoundary
                                | UnknownEvidenceKind::RoomBoundaryWall
                        )
                )
        }));
    }

    #[test]
    fn open_room_and_missing_schedule_are_explicit_unknown_consequences() {
        let mut model = BuildingModel::new();
        model.rooms.push(Room::new(
            "room-open",
            "Open room",
            RoomUsage::Living,
            "level-1",
            Point2::new(Length::from_feet(2.0), Length::from_feet(2.0)),
        ));
        let mut plan = generate_project_plan(&model).unwrap();
        plan.rooms.clear();
        let scene = build_physical_scene(&model, &plan);
        let audit = audit_physical_scene(&scene);
        let resolved = model.resolved_standards();
        let graph = compile_project_graph(
            &model,
            &plan,
            &resolved,
            &scene,
            &audit,
            &ComplianceReport::default(),
        )
        .unwrap();
        let boundary = ProjectNodeRef::RoomConsequence(RoomConsequenceRef::new(
            graph.revision(),
            ElementId::new("room-open"),
            RoomConsequenceKind::Boundary,
        ));
        let schedule = ProjectNodeRef::RoomConsequence(RoomConsequenceRef::new(
            graph.revision(),
            ElementId::new("room-open"),
            RoomConsequenceKind::Schedule,
        ));

        for (node, kind) in [
            (boundary, UnknownEvidenceKind::RoomBoundary),
            (schedule, UnknownEvidenceKind::RoomSchedule),
        ] {
            assert!(graph.edges().iter().any(|edge| {
                edge.dependent == node
                    && edge.relationship == RelationshipKind::UnresolvedEvidence
                    && matches!(
                        &edge.dependency,
                        ProjectNodeRef::UnknownEvidence(unknown) if unknown.kind == kind
                    )
            }));
        }
    }

    #[test]
    #[ignore = "manual rebuild/query performance smoke; run with --ignored --nocapture"]
    fn representative_rebuild_and_query_performance_smoke() {
        let model = all_families_model();
        let rebuild_started = Instant::now();
        let analysis = analysis(&model);
        let rebuild_elapsed = rebuild_started.elapsed();
        let graph = analysis.graph.as_ref().unwrap();
        let member = graph
            .nodes()
            .iter()
            .find_map(|node| match &node.reference {
                ProjectNodeRef::GeneratedMember(member) => {
                    Some(ProjectNodeRef::GeneratedMember(member.clone()))
                }
                _ => None,
            })
            .expect("fixture has a generated member");
        let mut cache = GraphQueryCache::default();
        let first_started = Instant::now();
        let first = cache.dependencies(graph, &member);
        let first_elapsed = first_started.elapsed();
        let cached_started = Instant::now();
        let cached = cache.dependencies(graph, &member);
        let cached_elapsed = cached_started.elapsed();

        assert_eq!(first, cached);
        assert!(!first.is_empty());
        assert_eq!(cache.stats().misses, 1);
        assert_eq!(cache.stats().hits, 1);
        eprintln!(
            "analysis performance smoke: nodes={} edges={} rebuild={rebuild_elapsed:?} first_query={first_elapsed:?} cached_query={cached_elapsed:?}",
            graph.nodes().len(),
            graph.edges().len(),
        );
    }
}
