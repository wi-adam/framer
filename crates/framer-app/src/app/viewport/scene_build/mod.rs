//! Builds the 3D scene mesh (`Scene3d`) and pickable solids from the building
//! model + frame plan, delegating each element family to a focused child module.

mod members;
mod picking;
mod style;
mod surfaces;
#[cfg(test)]
mod tests;
mod walls;

use eframe::egui::{Color32, Pos2};
use framer_core::BuildingModel;
use framer_geometry::{AssemblyKind, BodyRef, GeometryViolation, PhysicalScene};
use framer_solver::{FrameMember, ProjectFramePlan};

use super::geom::{OrbitProjector, Point3};
use super::gpu::GpuVertex;
use crate::app::{Selection, ViewClick, WallDisplay, WorkspaceMode};

use style::highlighted_member_color;
use surfaces::{level_elevation, push_ceiling_surfaces, push_floor_surfaces, push_roof_surfaces};
use walls::{interior_sign, wall_total_thickness};

use picking::PickSolid;
pub(super) use style::{brighten, color_to_rgba, member_color};

#[cfg(test)]
use framer_core::{
    ConstructionSystem, ElementId, Length, Material, RoofOpening, RoofPlane, SurfaceRegion, Wall,
};
#[cfg(test)]
use framer_geometry::{build_common_rafter_solid, matched_bearing_depth, ridge_face_setback};
#[cfg(test)]
use framer_solver::MemberKind;
#[cfg(test)]
use picking::PickShape;
#[cfg(test)]
use surfaces::region_outline_plan;
#[cfg(test)]
use walls::WallBasis;

pub(super) struct Scene3d {
    pub(super) vertices: Vec<GpuVertex>,
    pub(super) indices: Vec<u32>,
    pub(super) opaque_index_count: u32,
    pub(super) transparent_index_count: u32,
    pub(super) points: Vec<Point3>,
    picks: Vec<PickSolid>,
    /// Wall envelope edges for [`WallDisplay::Outline`], projected + drawn as a
    /// painter overlay by the axonometric view (the wgpu pipeline is triangle-only,
    /// so there is no GPU wireframe). Empty in the Width/Full modes.
    pub(super) outline_edges: Vec<OutlineEdge>,
}

/// One wall-outline edge in world space, with whether its wall is selected so the
/// overlay can highlight it. See [`Scene3d::outline_edges`].
#[derive(Clone, Copy)]
pub(super) struct OutlineEdge {
    pub(super) a: Point3,
    pub(super) b: Point3,
    pub(super) selected: bool,
    pub(super) danger: bool,
}

#[derive(Default)]
struct SceneBuilder {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    points: Vec<Point3>,
    picks: Vec<PickSolid>,
    outline_edges: Vec<OutlineEdge>,
    opaque_index_count: u32,
}

fn body_is_danger_highlighted(active: Option<&GeometryViolation>, body_ref: &BodyRef) -> bool {
    active.is_some_and(|violation| {
        violation.body_a() == body_ref || violation.body_b() == Some(body_ref)
    })
}

fn member_is_danger_highlighted(
    active: Option<&GeometryViolation>,
    owner: &framer_core::ElementId,
    member: &FrameMember,
) -> bool {
    active.is_some_and(|violation| {
        let body_ref = BodyRef::member(owner.clone(), member.kind, member.id.clone());
        violation.body_a() == &body_ref || violation.body_b() == Some(&body_ref)
    })
}

fn geometry_member_color(
    kind: framer_solver::MemberKind,
    source_selected: bool,
    member_selected: bool,
    danger: bool,
) -> Color32 {
    if danger {
        super::theme::danger()
    } else {
        highlighted_member_color(kind, source_selected, member_selected)
    }
}

impl Scene3d {
    #[cfg(test)]
    pub(super) fn from_project(
        model: &BuildingModel,
        plan: &ProjectFramePlan,
        selected_wall: usize,
        selection: &Selection,
        workspace_mode: WorkspaceMode,
        wall_display: WallDisplay,
    ) -> Option<Self> {
        let physical_scene = framer_geometry::build_physical_scene(model, plan);
        Self::from_project_with_geometry(
            model,
            plan,
            &physical_scene,
            None,
            selected_wall,
            selection,
            workspace_mode,
            wall_display,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_project_with_geometry(
        model: &BuildingModel,
        plan: &ProjectFramePlan,
        physical_scene: &PhysicalScene,
        active_geometry_violation: Option<&GeometryViolation>,
        selected_wall: usize,
        selection: &Selection,
        workspace_mode: WorkspaceMode,
        wall_display: WallDisplay,
    ) -> Option<Self> {
        if model.walls.is_empty()
            && model.roof_planes.is_empty()
            && model.ceilings.is_empty()
            && model.floor_decks.is_empty()
        {
            return None;
        }

        // Fall back to the code stud depth only when a wall has no system (a
        // degenerate model); resolved per-wall below.
        let fallback_depth = model.framing_defaults().stud_profile.nominal_depth();
        // Which side of each wall faces the room interior, derived from topology
        // once per frame. Layers (and members) lay out interior -> exterior from
        // this, so reversing a wall no longer mirrors the assembly.
        let interior_sides = framer_core::wall_interior_sides(model);
        // Derive gable profiles once. The per-wall helper rebuilds level topology,
        // which is needlessly expensive in this hot per-frame path.
        let gable_profiles = model.gable_wall_profiles();
        let mut builder = SceneBuilder::default();
        let shows_generated_plan = workspace_mode.shows_generated_plan();
        if shows_generated_plan {
            for (wall_index, wall) in model.walls.iter().enumerate() {
                if let Some(wall_plan) = plan.wall_plan(&wall.id) {
                    // `selected_wall` remains the active editing context when a
                    // roof/ceiling/floor member is selected. Do not let that stale
                    // index highlight an unrelated wall's entire generated plan.
                    let wall_selected =
                        selected_wall == wall_index && matches!(selection, Selection::Wall);
                    for member in &wall_plan.members {
                        let member_selected = matches!(
                            selection,
                            Selection::Member { source_id, member_id }
                                if source_id == &wall.id.0 && member_id == &member.id
                        );
                        let color = geometry_member_color(
                            member.kind,
                            wall_selected,
                            member_selected,
                            member_is_danger_highlighted(
                                active_geometry_violation,
                                &wall.id,
                                member,
                            ),
                        );
                        builder.push_shared_member(physical_scene, &wall.id, member, color);
                    }
                }
            }

            for floor_plan in &plan.floor_plans {
                for member in &floor_plan.members {
                    let source_selected = matches!(
                        selection,
                        Selection::FloorDeck(id) if id == &floor_plan.floor.0
                    );
                    let member_selected = matches!(
                        selection,
                        Selection::Member { source_id, member_id }
                            if source_id == &floor_plan.floor.0 && member_id == &member.id
                    );
                    builder.push_shared_member(
                        physical_scene,
                        &floor_plan.floor,
                        member,
                        geometry_member_color(
                            member.kind,
                            source_selected,
                            member_selected,
                            member_is_danger_highlighted(
                                active_geometry_violation,
                                &floor_plan.floor,
                                member,
                            ),
                        ),
                    );
                }
            }
            for ceiling_plan in &plan.ceiling_plans {
                for member in &ceiling_plan.members {
                    let source_selected = matches!(
                        selection,
                        Selection::Ceiling(id) if id == &ceiling_plan.ceiling.0
                    );
                    let member_selected = matches!(
                        selection,
                        Selection::Member { source_id, member_id }
                            if source_id == &ceiling_plan.ceiling.0 && member_id == &member.id
                    );
                    builder.push_shared_member(
                        physical_scene,
                        &ceiling_plan.ceiling,
                        member,
                        geometry_member_color(
                            member.kind,
                            source_selected,
                            member_selected,
                            member_is_danger_highlighted(
                                active_geometry_violation,
                                &ceiling_plan.ceiling,
                                member,
                            ),
                        ),
                    );
                }
            }
            for roof_plan in &plan.roof_plans {
                for member in &roof_plan.members {
                    let source_selected =
                        matches!(selection, Selection::RoofPlane(id) if id == &roof_plan.roof.0);
                    let member_selected = matches!(
                        selection,
                        Selection::Member { source_id, member_id }
                            if source_id == &roof_plan.roof.0 && member_id == &member.id
                    );
                    let color = geometry_member_color(
                        member.kind,
                        source_selected,
                        member_selected,
                        member_is_danger_highlighted(
                            active_geometry_violation,
                            &roof_plan.roof,
                            member,
                        ),
                    );
                    builder.push_shared_member(physical_scene, &roof_plan.roof, member, color);
                }
            }
        }

        // In Design, authored roof assemblies are ordinary opaque model surfaces.
        // In Plan, defer them until after the opaque member pass and draw them with
        // alpha so the spatial roof framing remains legible through the skin.
        if !shows_generated_plan {
            push_roof_surfaces(
                &mut builder,
                model,
                selection,
                active_geometry_violation,
                false,
            );
        }

        // Ceilings and floor decks remain opaque authored surfaces in every mode.
        push_ceiling_surfaces(&mut builder, model, selection, active_geometry_violation);
        push_floor_surfaces(&mut builder, model, selection, active_geometry_violation);

        builder.finish_opaque();

        if shows_generated_plan {
            push_roof_surfaces(
                &mut builder,
                model,
                selection,
                active_geometry_violation,
                true,
            );
        }

        for (wall_index, wall) in model.walls.iter().enumerate() {
            let total = wall_total_thickness(model, wall, fallback_depth);
            let sign = interior_sign(&interior_sides, &wall.id);
            let base_elevation = level_elevation(model, &wall.level);
            let wall_selected = selected_wall == wall_index && matches!(selection, Selection::Wall);
            let body_ref = BodyRef::assembly(wall.id.clone(), AssemblyKind::Wall);
            builder.push_wall_envelope(
                model,
                wall,
                wall_index,
                total,
                sign,
                base_elevation,
                gable_profiles.get(&wall.id),
                wall_selected,
                body_is_danger_highlighted(active_geometry_violation, &body_ref),
                wall_display,
            );
            for opening in &wall.openings {
                builder.push_opening_pick(
                    wall,
                    wall_index,
                    opening.id.0.clone(),
                    total,
                    sign,
                    base_elevation,
                );
            }
        }

        Some(builder.finish())
    }

    pub(super) fn pick(&self, pointer: Pos2, projector: &OrbitProjector) -> Option<ViewClick> {
        let mut best = None::<(u8, f32, ViewClick)>;
        for solid in &self.picks {
            let Some(depth) = solid.hit_depth(pointer, projector) else {
                continue;
            };
            let replace = best.as_ref().is_none_or(|(priority, best_depth, _)| {
                solid.priority > *priority || (solid.priority == *priority && depth > *best_depth)
            });
            if replace {
                best = Some((solid.priority, depth, solid.click.clone()));
            }
        }
        best.map(|(_, _, click)| click)
    }
}

impl SceneBuilder {
    fn push_shared_member(
        &mut self,
        physical_scene: &PhysicalScene,
        owner: &framer_core::ElementId,
        member: &FrameMember,
        color: Color32,
    ) {
        let body_ref = BodyRef::member(owner.clone(), member.kind, member.id.clone());
        let Some(body) = physical_scene.body(&body_ref) else {
            return;
        };
        self.push_member_body(
            body,
            ViewClick::Member {
                source_id: owner.0.clone(),
                member_id: member.id.clone(),
            },
            color,
        );
    }

    fn push_triangle(&mut self, points: [Point3; 3], color: [f32; 4]) {
        let normal = face_normal(points[0], points[1], points[2]);
        let base = self.vertices.len() as u32;
        for point in points {
            self.vertices.push(GpuVertex {
                position: [point.x, point.y, point.z],
                color,
                normal: [normal.x, normal.y, normal.z],
            });
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    fn push_quad(&mut self, points: [Point3; 4], color: [f32; 4]) {
        let normal = face_normal(points[0], points[1], points[2]);
        let base = self.vertices.len() as u32;
        for point in points {
            self.vertices.push(GpuVertex {
                position: [point.x, point.y, point.z],
                color,
                normal: [normal.x, normal.y, normal.z],
            });
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    /// Emit one face of a surface from precomputed triangle index triples (an
    /// ear-clip from `triangulate_simple_polygon`, correct for concave outlines),
    /// all sharing a single flat `normal`.
    fn push_face(
        &mut self,
        verts: &[Point3],
        triangles: &[[usize; 3]],
        normal: Point3,
        color: [f32; 4],
    ) {
        for &[ia, ib, ic] in triangles {
            let base = self.vertices.len() as u32;
            for &index in &[ia, ib, ic] {
                let point = verts[index];
                self.vertices.push(GpuVertex {
                    position: [point.x, point.y, point.z],
                    color,
                    normal: [normal.x, normal.y, normal.z],
                });
            }
            self.indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
    }

    fn finish_opaque(&mut self) {
        self.opaque_index_count = self.indices.len() as u32;
    }

    fn finish(self) -> Scene3d {
        let total_index_count = self.indices.len() as u32;
        Scene3d {
            vertices: self.vertices,
            indices: self.indices,
            opaque_index_count: self.opaque_index_count,
            transparent_index_count: total_index_count - self.opaque_index_count,
            points: self.points,
            picks: self.picks,
            outline_edges: self.outline_edges,
        }
    }
}

const GABLE_TRIANGLE_FACES: [[usize; 3]; 2] = [[0, 2, 1], [3, 4, 5]];
const GABLE_RENDER_QUAD_FACES: [[usize; 4]; 2] = [[0, 3, 5, 2], [1, 2, 5, 4]];
const GABLE_QUAD_FACES: [[usize; 4]; 3] = [[0, 1, 4, 3], [0, 3, 5, 2], [1, 2, 5, 4]];

fn vector_between(start: Point3, end: Point3) -> Point3 {
    Point3::vector(end.x - start.x, end.y - start.y, end.z - start.z)
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3::vector(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn normalized(vector: Point3) -> Option<Point3> {
    let length = (vector.x * vector.x + vector.y * vector.y + vector.z * vector.z).sqrt();
    (length > f32::EPSILON)
        .then(|| Point3::vector(vector.x / length, vector.y / length, vector.z / length))
}

fn face_normal(a: Point3, b: Point3, c: Point3) -> Point3 {
    normalized(cross(vector_between(a, b), vector_between(a, c))).unwrap_or(Point3::Z)
}
const CUBOID_FACE_INDICES: [[usize; 4]; 6] = [
    [0, 3, 2, 1],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [3, 0, 4, 7],
];
