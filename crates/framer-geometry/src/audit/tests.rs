use framer_core::ElementId;
use framer_solver::MemberKind;

use super::*;
use crate::{
    AssemblyKind, BodyRef, ConvexPiece, GeometryBuildDiagnostic, GeometryViolation, PhysicalBody,
    PhysicalScene, PhysicalSolid, Point3, TriMesh,
};

#[test]
fn face_edge_and_point_contact_are_clean() {
    let base = assembly_body("base", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    for (name, center) in [
        ("face", Point3::new(2.0, 0.0, 0.0)),
        ("edge", Point3::new(2.0, 2.0, 0.0)),
        ("point", Point3::new(2.0, 2.0, 2.0)),
    ] {
        let other = assembly_body(name, center, [1.0; 3]);
        assert!(audit(&[base.clone(), other]).is_clean(), "{name}");
    }
}

#[test]
fn real_sub_tick_penetration_above_query_epsilon_is_rejected() {
    let depth = 1.0 / 4096.0;
    let left = assembly_body("left", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    let right = assembly_body("right", Point3::new(2.0 - depth, 0.0, 0.0), [1.0; 3]);
    let result = audit(&[left, right]);
    let overlap = only_overlap(&result);
    assert!((overlap.penetration_depth - depth).abs() < 1.0e-9);
    assert!(overlap.penetration_depth < TICK_INCHES);
}

#[test]
fn collision_domains_skip_cross_detail_but_compare_each_domain_internally() {
    let assembly = assembly_body("host", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    let hosted_member = member_body("host", "stud-a", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    assert!(audit(&[assembly, hosted_member]).is_clean());

    let member_a = member_body("wall-a", "stud-a", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    let member_b = member_body("wall-b", "stud-b", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    assert_eq!(audit(&[member_a, member_b]).violations.len(), 1);

    let assembly_a = assembly_body("wall-a", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    let assembly_b = assembly_body("wall-b", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    assert_eq!(audit(&[assembly_a, assembly_b]).violations.len(), 1);
}

#[test]
fn shuffled_input_and_duplicate_self_pairs_have_stable_results() {
    let first = member_body("wall-a", "stud-a", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    let second = member_body("wall-b", "stud-b", Point3::new(1.5, 0.0, 0.0), [1.0; 3]);
    let far = member_body("wall-c", "stud-c", Point3::new(9.0, 0.0, 0.0), [1.0; 3]);
    let forward = audit(&[first.clone(), second.clone(), far.clone()]);
    let shuffled = audit(&[far, second, first]);
    assert_eq!(forward, shuffled);

    let duplicate = member_body("wall-a", "stud-a", Point3::new(0.25, 0.0, 0.0), [1.0; 3]);
    assert!(
        audit(&[
            member_body("wall-a", "stud-a", Point3::new(0.0, 0.0, 0.0), [1.0; 3]),
            duplicate
        ])
        .is_clean()
    );
}

#[test]
fn unsupported_piece_pair_fails_closed() {
    let mesh = TriMesh {
        points: vec![
            Point3::new(-1.0, -1.0, 0.0),
            Point3::new(1.0, -1.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(-1.0, 1.0, 0.0),
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
    };
    let piece = ConvexPiece::new(mesh.clone()).unwrap();
    let unsupported = PhysicalBody::new(
        BodyRef::assembly(ElementId::new("unsupported"), AssemblyKind::Wall),
        PhysicalSolid::new(mesh, vec![piece]).unwrap(),
    );
    let valid = assembly_body("valid", Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    let result = audit(&[unsupported, valid]);
    assert!(matches!(
        result.violations.as_slice(),
        [GeometryViolation::QueryUnsupported(diagnostic)]
            if diagnostic.message.contains("cannot be lowered")
    ));
}

#[test]
fn build_failures_are_preserved_in_the_audit() {
    let body_ref = BodyRef::assembly(ElementId::new("bad-wall"), AssemblyKind::Wall);
    let mut scene = PhysicalScene::default();
    scene.diagnostics.push(GeometryBuildDiagnostic::unbuildable(
        body_ref.clone(),
        "synthetic failure",
    ));
    let audit = audit_physical_scene(&scene);
    assert!(matches!(
        audit.violations.as_slice(),
        [GeometryViolation::BodyUnbuildable(diagnostic)]
            if diagnostic.body_ref == body_ref && diagnostic.message == "synthetic failure"
    ));
}

#[test]
fn convex_piece_union_reports_its_deepest_penetration() {
    let shallow = cuboid_mesh(Point3::new(0.0, 0.0, 0.0), [1.0; 3]);
    let deep = cuboid_mesh(Point3::new(0.5, 0.0, 0.0), [1.0; 3]);
    let mut surface = shallow.clone();
    surface.append(&deep);
    let union = PhysicalBody::new(
        BodyRef::assembly(ElementId::new("union"), AssemblyKind::Wall),
        PhysicalSolid::new(
            surface,
            vec![
                ConvexPiece::new(shallow).unwrap(),
                ConvexPiece::new(deep).unwrap(),
            ],
        )
        .unwrap(),
    );
    let other = assembly_body("other", Point3::new(1.75, 0.0, 0.0), [1.0; 3]);
    let result = audit(&[union, other]);
    let overlap = only_overlap(&result);
    assert!(
        (overlap.penetration_depth - 0.75).abs() < 1.0e-9,
        "{overlap:?}"
    );
}

fn only_overlap(result: &GeometryAudit) -> &crate::GeometryOverlapViolation {
    match result.violations.as_slice() {
        [GeometryViolation::Overlap(overlap)] => overlap,
        violations => panic!("expected one overlap, got {violations:?}"),
    }
}

fn audit(bodies: &[PhysicalBody]) -> GeometryAudit {
    let mut scene = PhysicalScene::default();
    for body in bodies {
        scene.push_body(body.clone());
    }
    audit_physical_scene(&scene.finish())
}

fn assembly_body(owner: &str, center: Point3, half: [f64; 3]) -> PhysicalBody {
    body(
        BodyRef::assembly(ElementId::new(owner), AssemblyKind::Wall),
        center,
        half,
    )
}

fn member_body(owner: &str, member_id: &str, center: Point3, half: [f64; 3]) -> PhysicalBody {
    body(
        BodyRef::member(ElementId::new(owner), MemberKind::CommonStud, member_id),
        center,
        half,
    )
}

fn body(body_ref: BodyRef, center: Point3, half: [f64; 3]) -> PhysicalBody {
    let mesh = cuboid_mesh(center, half);
    let piece = ConvexPiece::new(mesh.clone()).unwrap();
    PhysicalBody::new(body_ref, PhysicalSolid::new(mesh, vec![piece]).unwrap())
}

fn cuboid_mesh(center: Point3, half: [f64; 3]) -> TriMesh {
    let [hx, hy, hz] = half;
    let points = vec![
        Point3::new(center.x - hx, center.y - hy, center.z - hz),
        Point3::new(center.x + hx, center.y - hy, center.z - hz),
        Point3::new(center.x + hx, center.y + hy, center.z - hz),
        Point3::new(center.x - hx, center.y + hy, center.z - hz),
        Point3::new(center.x - hx, center.y - hy, center.z + hz),
        Point3::new(center.x + hx, center.y - hy, center.z + hz),
        Point3::new(center.x + hx, center.y + hy, center.z + hz),
        Point3::new(center.x - hx, center.y + hy, center.z + hz),
    ];
    let triangles = vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    TriMesh { points, triangles }
}
