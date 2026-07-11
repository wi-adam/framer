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
        Ok(solid) => scene.push_body(PhysicalBody::new(body_ref, solid)),
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
    use framer_core::{
        CeilingSlope, FramingDefaults, LayerFunction, MemberFamily, Point2, RoofPlane, Slope, Wall,
    };

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
                .bodies()
                .iter()
                .filter(|body| body.body_ref.member_id().is_some())
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

    #[test]
    fn opposing_sloped_ceilings_butt_at_rims_without_member_overlaps() {
        let mut model = example_shell();
        model.roof_planes.clear();
        let pitch = Slope::new(Length::from_whole_inches(3), Length::from_whole_inches(12));
        for ceiling in &mut model.ceilings {
            ceiling.slope = Some(CeilingSlope::new(pitch, 0));
        }
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let audit = crate::audit_project(&model, &plan);
        assert!(audit.is_clean(), "{:#?}", audit.violations);
    }

    #[test]
    fn gable_and_truss_shed_roof_fields_audit_clean() {
        for (width, depth) in [(24.0, 16.0), (16.0, 24.0)] {
            let mut model = roof_test_model();
            let p = |x, y| Point2::new(Length::from_feet(x), Length::from_feet(y));
            let ridge_y = depth / 2.0;
            let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
            model.roof_planes = vec![
                RoofPlane::new(
                    "roof-south",
                    "South gable field",
                    "level-1",
                    "system-roof-1",
                    vec![
                        p(0.0, 0.0),
                        p(width, 0.0),
                        p(width, ridge_y),
                        p(0.0, ridge_y),
                    ],
                    slope,
                    0,
                    Length::from_feet(9.0),
                ),
                RoofPlane::new(
                    "roof-north",
                    "North gable field",
                    "level-1",
                    "system-roof-1",
                    vec![
                        p(width, depth),
                        p(0.0, depth),
                        p(0.0, ridge_y),
                        p(width, ridge_y),
                    ],
                    slope,
                    0,
                    Length::from_feet(9.0),
                ),
            ];
            let plan = framer_solver::generate_project_plan(&model).unwrap();
            let audit = crate::audit_project(&model, &plan);
            assert!(audit.is_clean(), "{width}x{depth}: {:#?}", audit.violations);
        }

        let mut shed = roof_test_model();
        let roof_system = shed
            .systems
            .iter_mut()
            .find(|system| system.id.0 == "system-roof-1")
            .unwrap();
        roof_system
            .layers
            .iter_mut()
            .find(|layer| layer.function == LayerFunction::Framing)
            .unwrap()
            .framing
            .as_mut()
            .unwrap()
            .member_family = MemberFamily::Truss;
        shed.roof_planes.push(RoofPlane::new(
            "roof-shed",
            "Truss shed",
            "level-1",
            "system-roof-1",
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::from_feet(20.0), Length::ZERO),
                Point2::new(Length::from_feet(20.0), Length::from_feet(12.0)),
                Point2::new(Length::ZERO, Length::from_feet(12.0)),
            ],
            Slope::new(Length::from_whole_inches(3), Length::from_whole_inches(12)),
            0,
            Length::from_feet(9.0),
        ));
        let plan = framer_solver::generate_project_plan(&shed).unwrap();
        let audit = crate::audit_project(&shed, &plan);
        assert!(audit.is_clean(), "{:#?}", audit.violations);
    }

    #[test]
    fn equal_pitch_valley_and_mirrored_l_audit_clean() {
        let model = valley_test_model();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let audit = crate::audit_project(&model, &plan);
        assert!(audit.is_clean(), "{:#?}", audit.violations);

        let mut mirrored = model;
        for wall in &mut mirrored.walls {
            wall.start.x = Length::ZERO - wall.start.x;
            wall.end.x = Length::ZERO - wall.end.x;
        }
        for join in &mut mirrored.wall_joins {
            join.point.x = Length::ZERO - join.point.x;
        }
        for plane in &mut mirrored.roof_planes {
            for point in &mut plane.outline {
                point.x = Length::ZERO - point.x;
            }
        }
        let plan = framer_solver::generate_project_plan(&mirrored).unwrap();
        let audit = crate::audit_project(&mirrored, &plan);
        assert!(audit.is_clean(), "mirrored: {:#?}", audit.violations);
    }

    fn roof_test_model() -> BuildingModel {
        let mut model = example_shell();
        model.walls.clear();
        model.wall_joins.clear();
        model.rooms.clear();
        model.ceilings.clear();
        model.floor_decks.clear();
        model.roof_planes.clear();
        model
    }

    fn valley_test_model() -> BuildingModel {
        let mut model = roof_test_model();
        let defaults = FramingDefaults::irc_2021_starter();
        let p = |x, y| Point2::new(Length::from_feet(x), Length::from_feet(y));
        let footprint = [
            p(0.0, 0.0),
            p(24.0, 0.0),
            p(24.0, 12.0),
            p(12.0, 12.0),
            p(12.0, 24.0),
            p(0.0, 24.0),
            p(0.0, 0.0),
        ];
        for (index, pair) in footprint.windows(2).enumerate() {
            model.walls.push(
                Wall::new(
                    format!("wall-l-{index}"),
                    format!("L footprint wall {index}"),
                    Length::from_feet(1.0),
                    &defaults,
                )
                .with_placement("level-1", pair[0], pair[1]),
            );
        }
        model.reconcile_joins();
        let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
        model.roof_planes = vec![
            RoofPlane::new(
                "roof-a",
                "Lower L-wing valley slope",
                "level-1",
                "system-roof-1",
                vec![p(0.0, 0.0), p(24.0, 0.0), p(24.0, 12.0), p(12.0, 12.0)],
                slope,
                0,
                Length::from_feet(9.0),
            ),
            RoofPlane::new(
                "roof-b",
                "Upper L-wing valley slope",
                "level-1",
                "system-roof-1",
                vec![p(0.0, 0.0), p(0.0, 24.0), p(12.0, 24.0), p(12.0, 12.0)],
                slope,
                0,
                Length::from_feet(9.0),
            ),
        ];
        model
    }
}
