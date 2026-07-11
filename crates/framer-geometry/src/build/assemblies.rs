use std::collections::BTreeMap;

use framer_core::{
    BuildingModel, Ceiling, ConstructionSystem, ElementId, FloorDeck, GableWallProfile, Length,
    Point2, RoofPlane, Wall,
};

use super::members::WallBasis;
use super::{
    cuboid_solid, level_elevation, level_top, merged_solid, push_body_result, region_outline,
};
use crate::{AssemblyKind, BodyRef, ConvexPiece, PhysicalScene, PhysicalSolid, Point3, TriMesh};

pub(super) fn build_assemblies(model: &BuildingModel, scene: &mut PhysicalScene) {
    let fallback_depth = model.framing_defaults().stud_profile.nominal_depth();
    let interior_sides = framer_core::wall_interior_sides(model);
    let gables = model.gable_wall_profiles();
    for wall in &model.walls {
        let body_ref = BodyRef::assembly(wall.id.clone(), AssemblyKind::Wall);
        let total = model
            .system_for(wall)
            .map(ConstructionSystem::total_thickness)
            .unwrap_or(fallback_depth);
        push_body_result(
            scene,
            body_ref,
            wall_assembly_solid(
                model,
                wall,
                total,
                interior_sign(&interior_sides, &wall.id),
                level_elevation(model, &wall.level),
                gables.get(&wall.id),
            ),
        );
    }

    for deck in &model.floor_decks {
        let body_ref = BodyRef::assembly(deck.id.clone(), AssemblyKind::FloorDeck);
        push_body_result(scene, body_ref, floor_assembly_solid(model, deck));
    }
    for ceiling in &model.ceilings {
        let body_ref = BodyRef::assembly(ceiling.id.clone(), AssemblyKind::Ceiling);
        push_body_result(scene, body_ref, ceiling_assembly_solid(model, ceiling));
    }
    for plane in &model.roof_planes {
        let body_ref = BodyRef::assembly(plane.id.clone(), AssemblyKind::RoofPlane);
        push_body_result(scene, body_ref, roof_assembly_solid(model, plane));
    }
}

fn wall_assembly_solid(
    model: &BuildingModel,
    wall: &Wall,
    total: Length,
    interior_sign: f64,
    base_elevation: f64,
    gable: Option<&GableWallProfile>,
) -> Result<PhysicalSolid, String> {
    let half = total.inches() / 2.0;
    let side_a = interior_sign * half;
    let side_b = -interior_sign * half;
    let (side0, side1) = (side_a.min(side_b), side_a.max(side_b));
    let (visual_x0, visual_x1) = model.wall_envelope_span(wall);
    let basis = WallBasis::new(wall);
    let mut solids = Vec::new();
    let mut openings = wall.openings.iter().collect::<Vec<_>>();
    openings.sort_by_key(|opening| opening.left());
    let mut cursor = visual_x0;
    for opening in openings {
        push_wall_segment(
            &mut solids,
            &basis,
            cursor,
            opening.left().inches(),
            side0,
            side1,
            base_elevation,
            base_elevation + wall.height.inches(),
        )?;
        if opening.sill_height > Length::ZERO {
            push_wall_segment(
                &mut solids,
                &basis,
                opening.left().inches(),
                opening.right().inches(),
                side0,
                side1,
                base_elevation,
                base_elevation + opening.sill_height.inches(),
            )?;
        }
        if opening.top() < wall.height {
            push_wall_segment(
                &mut solids,
                &basis,
                opening.left().inches(),
                opening.right().inches(),
                side0,
                side1,
                base_elevation + opening.top().inches(),
                base_elevation + wall.height.inches(),
            )?;
        }
        cursor = opening.right().inches();
    }
    push_wall_segment(
        &mut solids,
        &basis,
        cursor,
        visual_x1,
        side0,
        side1,
        base_elevation,
        base_elevation + wall.height.inches(),
    )?;
    if let Some(profile) = gable {
        let left_z = profile.base_elevation.inches();
        let points = [
            basis.point(0.0, side0, left_z),
            basis.point(profile.width.inches(), side0, left_z),
            basis.point(
                profile.peak_x.inches(),
                side0,
                profile.peak_elevation.inches(),
            ),
            basis.point(0.0, side1, left_z),
            basis.point(profile.width.inches(), side1, left_z),
            basis.point(
                profile.peak_x.inches(),
                side1,
                profile.peak_elevation.inches(),
            ),
        ];
        solids.push(triangular_prism(points)?);
    }
    merged_solid(solids)
}

#[allow(clippy::too_many_arguments)]
fn push_wall_segment(
    solids: &mut Vec<PhysicalSolid>,
    basis: &WallBasis,
    x0: f64,
    x1: f64,
    side0: f64,
    side1: f64,
    z0: f64,
    z1: f64,
) -> Result<(), String> {
    if x1 <= x0 || z1 <= z0 {
        return Ok(());
    }
    solids.push(cuboid_solid(basis.cuboid(x0, x1, side0, side1, z0, z1))?);
    Ok(())
}

fn floor_assembly_solid(model: &BuildingModel, deck: &FloorDeck) -> Result<PhysicalSolid, String> {
    let outline = region_outline(model, &deck.region)
        .ok_or_else(|| "floor deck region cannot be resolved".to_string())?;
    let thickness = system_thickness(model, &deck.system)?;
    let top = level_elevation(model, &deck.level);
    extrude_plan_polygon(&outline, top - thickness, top)
}

fn ceiling_assembly_solid(
    model: &BuildingModel,
    ceiling: &Ceiling,
) -> Result<PhysicalSolid, String> {
    let outline = region_outline(model, &ceiling.region)
        .ok_or_else(|| "ceiling region cannot be resolved".to_string())?;
    let thickness = system_thickness(model, &ceiling.system)?;
    let reference = (level_top(model, &ceiling.level) - ceiling.height).inches();
    match ceiling.frame(Length::from_inches(reference)) {
        Some(frame) => extrude_lifted_polygon(&outline, |point| {
            let x = point.x.inches();
            let y = point.y.inches();
            (Point3::new(x, y, frame.elevation_at(x, y)), thickness)
        }),
        None => extrude_plan_polygon(&outline, reference, reference + thickness),
    }
}

fn roof_assembly_solid(model: &BuildingModel, plane: &RoofPlane) -> Result<PhysicalSolid, String> {
    let frame = plane
        .frame()
        .ok_or_else(|| "roof plane has no valid affine frame".to_string())?;
    let triangulation = model
        .roof_surface_triangulation(plane)
        .ok_or_else(|| "roof surface and cavities cannot be triangulated".to_string())?;
    let thickness = system_thickness(model, &plane.system)?;
    extrude_triangulated_polygon(&triangulation.points, &triangulation.triangles, |point| {
        let x = point.x.inches();
        let y = point.y.inches();
        (Point3::new(x, y, frame.elevation_at(x, y)), thickness)
    })
}

fn system_thickness(model: &BuildingModel, system_id: &ElementId) -> Result<f64, String> {
    model
        .systems
        .iter()
        .find(|system| system.id == *system_id)
        .map(|system| system.total_thickness().inches())
        .filter(|thickness| *thickness > 0.0)
        .ok_or_else(|| format!("construction system {system_id:?} has no positive thickness"))
}

fn extrude_plan_polygon(outline: &[Point2], z0: f64, z1: f64) -> Result<PhysicalSolid, String> {
    extrude_lifted_polygon(outline, |point| {
        (Point3::new(point.x.inches(), point.y.inches(), z0), z1 - z0)
    })
}

fn extrude_lifted_polygon(
    outline: &[Point2],
    lift: impl Fn(Point2) -> (Point3, f64),
) -> Result<PhysicalSolid, String> {
    let triangles = framer_core::triangulate_simple_polygon(outline);
    extrude_triangulated_polygon(outline, &triangles, lift)
}

fn extrude_triangulated_polygon(
    points: &[Point2],
    triangles: &[[usize; 3]],
    lift: impl Fn(Point2) -> (Point3, f64),
) -> Result<PhysicalSolid, String> {
    if points.len() < 3 || triangles.is_empty() {
        return Err("assembly outline produced no occupied triangles".into());
    }
    let mut solids = Vec::with_capacity(triangles.len());
    for &[a, b, c] in triangles {
        let (a0, lift_a) = lift(points[a]);
        let (b0, lift_b) = lift(points[b]);
        let (c0, lift_c) = lift(points[c]);
        if lift_a <= 0.0 || lift_b <= 0.0 || lift_c <= 0.0 {
            return Err("assembly extrusion has nonpositive thickness".into());
        }
        solids.push(triangular_prism([
            a0,
            b0,
            c0,
            Point3::new(a0.x, a0.y, a0.z + lift_a),
            Point3::new(b0.x, b0.y, b0.z + lift_b),
            Point3::new(c0.x, c0.y, c0.z + lift_c),
        ])?);
    }
    merged_solid(solids)
}

fn triangular_prism(points: [Point3; 6]) -> Result<PhysicalSolid, String> {
    let triangles = vec![
        [0, 2, 1],
        [3, 4, 5],
        [0, 1, 4],
        [0, 4, 3],
        [1, 2, 5],
        [1, 5, 4],
        [2, 0, 3],
        [2, 3, 5],
    ];
    let mesh = TriMesh {
        points: points.to_vec(),
        triangles,
    };
    let piece = ConvexPiece::new(mesh.clone())
        .ok_or_else(|| "triangular prism is degenerate".to_string())?;
    PhysicalSolid::new(mesh, vec![piece])
        .ok_or_else(|| "triangular prism has no physical solid".into())
}

fn interior_sign(interior_sides: &BTreeMap<ElementId, bool>, wall_id: &ElementId) -> f64 {
    match interior_sides.get(wall_id) {
        Some(true) => 1.0,
        _ => -1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use framer_core::{OpeningKind, RoofOpening};

    fn example_shell() -> BuildingModel {
        framer_core::load_project(include_str!(
            "../../../../examples/projects/demo-shell.framer"
        ))
        .unwrap()
    }

    #[test]
    fn wall_envelope_uses_lapped_span_and_preserves_opening_cavity() {
        let model = example_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let scene = crate::build_physical_scene(&model, &plan);
        let body_ref = BodyRef::assembly(model.walls[0].id.clone(), AssemblyKind::Wall);
        let body = scene.body(&body_ref).unwrap();
        let (x0, x1) = model.wall_envelope_span(&model.walls[0]);
        assert!((body.aabb.max.x - body.aabb.min.x >= (x1 - x0).abs() - 1.0e-6));
        assert!(body.solid.convex_pieces.len() >= 4);
    }

    #[test]
    fn roof_opening_is_a_real_hole_in_the_convex_piece_union() {
        let mut model = example_shell();
        let plane_id = model.roof_planes[0].id.clone();
        model.roof_planes[0].openings.push(RoofOpening::new(
            "skylight-test",
            OpeningKind::Skylight,
            Point2::new(Length::from_feet(8.0), Length::from_feet(4.0)),
            Length::from_feet(2.0),
            Length::from_feet(3.0),
        ));
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let scene = crate::build_physical_scene(&model, &plan);
        assert!(scene.diagnostics.is_empty(), "{:?}", scene.diagnostics);
        let body_ref = BodyRef::assembly(plane_id, AssemblyKind::RoofPlane);
        let body = scene.body(&body_ref).unwrap();
        assert!(body.solid.convex_pieces.len() > 2);
    }

    #[test]
    fn roof_opening_outside_its_host_fails_closed() {
        let mut model = example_shell();
        let plane_id = model.roof_planes[0].id.clone();
        model.roof_planes[0].openings.push(RoofOpening::new(
            "outside-skylight",
            OpeningKind::Skylight,
            Point2::new(Length::from_feet(200.0), Length::from_feet(200.0)),
            Length::from_feet(2.0),
            Length::from_feet(3.0),
        ));
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let scene = crate::build_physical_scene(&model, &plan);
        let expected = BodyRef::assembly(plane_id, AssemblyKind::RoofPlane);
        assert!(scene.body(&expected).is_none());
        assert!(scene.diagnostics.iter().any(|diagnostic| {
            diagnostic.body_ref == expected
                && diagnostic.code == crate::GeometryBuildDiagnostic::CODE
        }));
    }

    #[test]
    fn roof_cavity_geometry_is_independent_of_opening_vector_order() {
        let mut model = example_shell();
        let plane_id = model.roof_planes[0].id.clone();
        model.roof_planes[0].openings = vec![
            RoofOpening::new(
                "skylight-b",
                OpeningKind::Skylight,
                Point2::new(Length::from_feet(10.0), Length::from_feet(3.0)),
                Length::from_feet(2.0),
                Length::from_feet(2.0),
            ),
            RoofOpening::new(
                "skylight-a",
                OpeningKind::Skylight,
                Point2::new(Length::from_feet(4.0), Length::from_feet(3.0)),
                Length::from_feet(2.0),
                Length::from_feet(2.0),
            ),
        ];
        let mut reversed = model.clone();
        reversed.roof_planes[0].openings.reverse();
        let body_ref = BodyRef::assembly(plane_id, AssemblyKind::RoofPlane);
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let reversed_plan = framer_solver::generate_project_plan(&reversed).unwrap();
        let scene = crate::build_physical_scene(&model, &plan);
        let reversed_scene = crate::build_physical_scene(&reversed, &reversed_plan);
        assert_eq!(
            scene.body(&body_ref).unwrap().solid,
            reversed_scene.body(&body_ref).unwrap().solid
        );
    }
}
