mod assemblies;
mod members;

pub use members::{
    RafterPrism, build_common_rafter_solid, matched_bearing_depth, ridge_face_setback,
};

use framer_core::{BuildingModel, ElementId, Length, SurfaceRegion};
use framer_solver::ProjectFramePlan;

use crate::{
    BodyRef, GeometryBuildDiagnostic, PhysicalBody, PhysicalScene, PhysicalSolid, Point3, TriMesh,
};

pub fn build_physical_scene(model: &BuildingModel, plan: &ProjectFramePlan) -> PhysicalScene {
    let mut scene = PhysicalScene::default();
    members::build_members(model, plan, &mut scene);
    assemblies::build_assemblies(model, &mut scene);
    scene.finish()
}

fn push_body_result(
    scene: &mut PhysicalScene,
    body_ref: BodyRef,
    result: Result<PhysicalSolid, String>,
) {
    match result {
        Ok(solid) => scene.bodies.push(PhysicalBody::new(body_ref, solid)),
        Err(message) => scene
            .diagnostics
            .push(GeometryBuildDiagnostic::unbuildable(body_ref, message)),
    }
}

fn level_elevation(model: &BuildingModel, level_id: &ElementId) -> f64 {
    model
        .levels
        .iter()
        .find(|level| level.id == *level_id)
        .map(|level| level.elevation.inches())
        .unwrap_or(0.0)
}

fn level_top(model: &BuildingModel, level_id: &ElementId) -> Length {
    model
        .levels
        .iter()
        .find(|level| level.id == *level_id)
        .map(|level| level.elevation + level.height)
        .unwrap_or(Length::ZERO)
}

fn region_outline(
    model: &BuildingModel,
    region: &SurfaceRegion,
) -> Option<Vec<framer_core::Point2>> {
    let outline = match region {
        SurfaceRegion::Polygon(points) => points.clone(),
        SurfaceRegion::Room(room_id) => {
            let room = model.rooms.iter().find(|room| room.id == *room_id)?;
            framer_core::room_boundary_on_level(model, &room.level, room.seed)?.vertices
        }
    };
    (outline.len() >= 3).then_some(outline)
}

const CUBOID_FACES: [[usize; 4]; 6] = [
    [0, 3, 2, 1],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [3, 0, 4, 7],
];

fn cuboid_solid(corners: [Point3; 8]) -> Result<PhysicalSolid, String> {
    if corners[0].distance_squared(corners[1]) <= f64::EPSILON
        || corners[1].distance_squared(corners[2]) <= f64::EPSILON
        || corners[0].distance_squared(corners[4]) <= f64::EPSILON
    {
        return Err("cuboid has a zero-length axis".into());
    }
    let points = corners.to_vec();
    let mut triangles = Vec::with_capacity(12);
    for [a, b, c, d] in CUBOID_FACES {
        triangles.push([a, b, c]);
        triangles.push([a, c, d]);
    }
    let mesh = TriMesh { points, triangles };
    let piece = crate::ConvexPiece::new(mesh.clone())
        .ok_or_else(|| "cuboid did not produce a convex query piece".to_string())?;
    PhysicalSolid::new(mesh, vec![piece])
        .ok_or_else(|| "cuboid did not produce a physical solid".into())
}

fn merged_solid(solids: Vec<PhysicalSolid>) -> Result<PhysicalSolid, String> {
    if solids.is_empty() {
        return Err("body has no occupied convex pieces".into());
    }
    let mut surface = TriMesh::default();
    let mut pieces = Vec::new();
    for solid in solids {
        surface.append(&solid.surface);
        pieces.extend(solid.convex_pieces);
    }
    PhysicalSolid::new(surface, pieces).ok_or_else(|| "body has no physical solid".into())
}

fn vector_between(start: Point3, end: Point3) -> Point3 {
    Point3::new(end.x - start.x, end.y - start.y, end.z - start.z)
}

fn offset(point: Point3, axis: Point3, amount: f64) -> Point3 {
    Point3::new(
        point.x + axis.x * amount,
        point.y + axis.y * amount,
        point.z + axis.z * amount,
    )
}

fn cross(a: Point3, b: Point3) -> Point3 {
    Point3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn dot(a: Point3, b: Point3) -> f64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn normalized(vector: Point3) -> Option<Point3> {
    let length = dot(vector, vector).sqrt();
    (length > f64::EPSILON)
        .then(|| Point3::new(vector.x / length, vector.y / length, vector.z / length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssemblyKind, BodyRef};

    fn example_shell() -> BuildingModel {
        framer_core::load_project(include_str!(
            "../../../../examples/projects/demo-shell.framer"
        ))
        .unwrap()
    }

    #[test]
    fn every_generated_member_and_authored_assembly_has_one_body() {
        let model = example_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let scene = build_physical_scene(&model, &plan);
        assert!(scene.diagnostics.is_empty(), "{:?}", scene.diagnostics);

        let members: Vec<_> = plan
            .wall_plans
            .iter()
            .map(|plan| (&plan.wall, &plan.members))
            .chain(
                plan.floor_plans
                    .iter()
                    .map(|plan| (&plan.floor, &plan.members)),
            )
            .chain(
                plan.ceiling_plans
                    .iter()
                    .map(|plan| (&plan.ceiling, &plan.members)),
            )
            .chain(
                plan.roof_plans
                    .iter()
                    .map(|plan| (&plan.roof, &plan.members)),
            )
            .flat_map(|(owner, members)| {
                members.iter().map(move |member| {
                    BodyRef::member(owner.clone(), member.kind, member.id.clone())
                })
            })
            .collect();
        assert!(!members.is_empty());
        for body_ref in &members {
            assert!(scene.body(body_ref).is_some(), "missing {body_ref:?}");
        }
        assert_eq!(
            scene
                .bodies
                .iter()
                .filter(|body| body.body_ref.member_id.is_some())
                .count(),
            members.len()
        );

        for wall in &model.walls {
            assert!(
                scene
                    .body(&BodyRef::assembly(wall.id.clone(), AssemblyKind::Wall))
                    .is_some()
            );
        }
        for floor in &model.floor_decks {
            assert!(
                scene
                    .body(&BodyRef::assembly(
                        floor.id.clone(),
                        AssemblyKind::FloorDeck,
                    ))
                    .is_some()
            );
        }
        for ceiling in &model.ceilings {
            assert!(
                scene
                    .body(&BodyRef::assembly(
                        ceiling.id.clone(),
                        AssemblyKind::Ceiling,
                    ))
                    .is_some()
            );
        }
        for roof in &model.roof_planes {
            assert!(
                scene
                    .body(&BodyRef::assembly(roof.id.clone(), AssemblyKind::RoofPlane,))
                    .is_some()
            );
        }
    }

    #[test]
    fn invalid_member_geometry_fails_closed_with_its_body_ref() {
        let model = example_shell();
        let mut plan = framer_solver::generate_project_plan(&model).unwrap();
        let wall_plan = plan.wall_plans.first_mut().unwrap();
        let member = wall_plan.members.first_mut().unwrap();
        member.side_depth = Length::ZERO;
        let expected = BodyRef::member(wall_plan.wall.clone(), member.kind, member.id.clone());

        let scene = build_physical_scene(&model, &plan);
        assert!(scene.body(&expected).is_none());
        assert!(scene.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == GeometryBuildDiagnostic::CODE && diagnostic.body_ref == expected
        }));
    }
}
