use std::cmp::Ordering;
use std::fmt;

use parry3d_f64::math::{Pose, Vector};
use parry3d_f64::query::{
    ContactManifold, ContactManifoldsWorkspace, DefaultQueryDispatcher, PersistentQueryDispatcher,
    contact, intersection_test,
};
use parry3d_f64::shape::{ConvexPolyhedron, Shape};

use crate::{ConvexPiece, PhysicalSolid, Point3};

struct QueryPiece {
    pose: Pose,
    shape: ConvexPolyhedron,
    signature: Vec<[u64; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SolidContact {
    pub penetration_depth: f64,
    pub witness: Point3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueryError {
    message: String,
}

impl QueryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Query two semantic solids as unions of maintained-library convex hulls.
/// Every piece pair is evaluated; unsupported lowering/query paths fail closed.
pub(crate) fn solid_contact(
    left: &PhysicalSolid,
    right: &PhysicalSolid,
) -> Result<Option<SolidContact>, QueryError> {
    let left_shapes = lower_pieces("left", &left.convex_pieces)?;
    let right_shapes = lower_pieces("right", &right.convex_pieces)?;
    let mut deepest: Option<SolidContact> = None;

    for (left_index, left_piece) in left_shapes.iter().enumerate() {
        for (right_index, right_piece) in right_shapes.iter().enumerate() {
            let primary = contact(
                &left_piece.pose,
                &left_piece.shape,
                &right_piece.pose,
                &right_piece.shape,
                0.0,
            )
            .map_err(|_| {
                QueryError::new(format!(
                    "unsupported convex query for pieces {left_index} and {right_index}"
                ))
            })?;
            if let Some(primary) = primary
                && (primary.dist != 0.0 || left_piece.signature != right_piece.signature)
            {
                retain_contact(
                    &mut deepest,
                    primary.dist,
                    primary.point1,
                    primary.point2,
                    left_index,
                    right_index,
                )?;
                continue;
            }

            if primary.is_none() {
                let intersects = intersection_test(
                    &left_piece.pose,
                    &left_piece.shape,
                    &right_piece.pose,
                    &right_piece.shape,
                )
                .map_err(|_| {
                    QueryError::new(format!(
                        "unsupported intersection query for pieces {left_index} and {right_index}"
                    ))
                })?;
                if !intersects {
                    continue;
                }
            }

            // A single GJK contact can be indeterminate for coincident hulls.
            // The maintained manifold query supplies their penetration depth.
            let mut manifolds: Vec<ContactManifold<(), ()>> = Vec::new();
            let mut workspace: Option<ContactManifoldsWorkspace> = None;
            let relative_pose = left_piece.pose.inv_mul(&right_piece.pose);
            DefaultQueryDispatcher
                .contact_manifolds(
                    &relative_pose,
                    &left_piece.shape,
                    &right_piece.shape,
                    0.0,
                    &mut manifolds,
                    &mut workspace,
                )
                .map_err(|_| {
                    QueryError::new(format!(
                        "unsupported convex query for pieces {left_index} and {right_index}"
                    ))
                })?;
            let mut found_contact = false;
            for contact in manifolds.iter().flat_map(|manifold| &manifold.points) {
                found_contact = true;
                retain_contact(
                    &mut deepest,
                    contact.dist,
                    left_piece.pose * contact.local_p1,
                    right_piece.pose * contact.local_p2,
                    left_index,
                    right_index,
                )?;
            }
            if !found_contact {
                let intersects = intersection_test(
                    &left_piece.pose,
                    &left_piece.shape,
                    &right_piece.pose,
                    &right_piece.shape,
                )
                .map_err(|_| {
                    QueryError::new(format!(
                        "unsupported intersection query for pieces {left_index} and {right_index}"
                    ))
                })?;
                if intersects {
                    return Err(QueryError::new(format!(
                        "intersecting convex pieces {left_index} and {right_index} produced no contact"
                    )));
                }
            }
        }
    }
    Ok(deepest)
}

fn retain_contact(
    deepest: &mut Option<SolidContact>,
    distance: f64,
    point1: Vector,
    point2: Vector,
    left_index: usize,
    right_index: usize,
) -> Result<(), QueryError> {
    if !distance.is_finite() || !point1.is_finite() || !point2.is_finite() {
        return Err(QueryError::new(format!(
            "non-finite convex contact for pieces {left_index} and {right_index}"
        )));
    }
    let midpoint = (point1 + point2) * 0.5;
    let candidate = SolidContact {
        penetration_depth: (-distance).max(0.0),
        witness: Point3::new(midpoint.x, midpoint.y, midpoint.z),
    };
    if deepest.as_ref().is_none_or(|current| {
        candidate.penetration_depth > current.penetration_depth
            || (candidate.penetration_depth == current.penetration_depth
                && compare_point(candidate.witness, current.witness) == Ordering::Less)
    }) {
        *deepest = Some(candidate);
    }
    Ok(())
}

fn lower_pieces(side: &str, pieces: &[ConvexPiece]) -> Result<Vec<QueryPiece>, QueryError> {
    pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| {
            let source = &piece.mesh().points;
            if source
                .iter()
                .any(|point| !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite())
            {
                return Err(QueryError::new(format!(
                    "{side} convex piece {index} contains a non-finite point"
                )));
            }
            let min = source.iter().fold(
                Vector::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
                |min, point| min.min(Vector::new(point.x, point.y, point.z)),
            );
            let max = source.iter().fold(
                Vector::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
                |max, point| max.max(Vector::new(point.x, point.y, point.z)),
            );
            let center = (min + max) * 0.5;
            let points: Vec<_> = source
                .iter()
                .map(|point| Vector::new(point.x, point.y, point.z) - center)
                .collect();
            let shape = ConvexPolyhedron::from_convex_hull(&points).ok_or_else(|| {
                QueryError::new(format!(
                    "{side} convex piece {index} cannot be lowered to a 3-D convex hull"
                ))
            })?;
            let mass = shape.mass_properties(1.0).mass();
            if !mass.is_finite() || mass <= 0.0 {
                return Err(QueryError::new(format!(
                    "{side} convex piece {index} cannot be lowered to a positive-volume convex hull"
                )));
            }
            Ok(QueryPiece {
                pose: Pose::translation(center.x, center.y, center.z),
                shape,
                signature: point_signature(source),
            })
        })
        .collect()
}

fn point_signature(points: &[Point3]) -> Vec<[u64; 3]> {
    let mut signature: Vec<_> = points
        .iter()
        .map(|point| {
            [
                normalized_bits(point.x),
                normalized_bits(point.y),
                normalized_bits(point.z),
            ]
        })
        .collect();
    signature.sort_unstable();
    signature
}

fn normalized_bits(value: f64) -> u64 {
    if value == 0.0 { 0 } else { value.to_bits() }
}

fn compare_point(left: Point3, right: Point3) -> Ordering {
    left.x
        .total_cmp(&right.x)
        .then_with(|| left.y.total_cmp(&right.y))
        .then_with(|| left.z.total_cmp(&right.z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConvexPiece, TriMesh};

    #[test]
    fn cuboid_queries_distinguish_separation_contact_and_penetration() {
        let base = cuboid(Point3::new(0.0, 0.0, 0.0), [1.0; 3], [0.0; 3]);
        let separated = cuboid(Point3::new(2.1, 0.0, 0.0), [1.0; 3], [0.0; 3]);
        let touching = cuboid(Point3::new(2.0, 0.0, 0.0), [1.0; 3], [0.0; 3]);
        let penetrating = cuboid(Point3::new(1.75, 0.0, 0.0), [1.0; 3], [0.0; 3]);

        assert_eq!(solid_contact(&base, &separated).unwrap(), None);
        let contact = solid_contact(&base, &touching).unwrap().unwrap();
        assert!(contact.penetration_depth <= 1.0e-9, "{contact:?}");
        let overlap = solid_contact(&base, &penetrating).unwrap().unwrap();
        assert!(
            (overlap.penetration_depth - 0.25).abs() < 1.0e-9,
            "{overlap:?}"
        );
    }

    #[test]
    fn containment_and_rotated_sloped_hulls_are_supported() {
        let outer = cuboid(Point3::new(0.0, 0.0, 0.0), [2.0; 3], [0.0; 3]);
        let inner = cuboid(Point3::new(0.0, 0.0, 0.0), [0.5; 3], [0.0; 3]);
        assert!(
            solid_contact(&outer, &inner)
                .unwrap()
                .unwrap()
                .penetration_depth
                > 0.0
        );

        let rotated = cuboid(
            Point3::new(0.75, 0.0, 0.25),
            [1.0, 0.4, 0.5],
            [0.35, 0.5, 0.2],
        );
        assert!(
            solid_contact(&outer, &rotated)
                .unwrap()
                .unwrap()
                .penetration_depth
                > 0.0
        );
    }

    #[test]
    fn reversing_query_order_preserves_depth_and_witness() {
        let left = cuboid(Point3::new(0.0, 0.0, 0.0), [1.0; 3], [0.0; 3]);
        let right = cuboid(Point3::new(1.75, 0.25, 0.0), [1.0; 3], [0.0; 3]);
        let forward = solid_contact(&left, &right).unwrap().unwrap();
        let reverse = solid_contact(&right, &left).unwrap().unwrap();
        assert!((forward.penetration_depth - reverse.penetration_depth).abs() < 1.0e-12);
        assert!(forward.witness.distance_squared(reverse.witness) < 1.0e-18);
    }

    #[test]
    fn degenerate_convex_piece_is_an_explicit_unsupported_path() {
        let mesh = TriMesh {
            points: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        };
        let unsupported = PhysicalSolid::new(
            mesh.clone(),
            vec![ConvexPiece::new(mesh).expect("piece wrapper")],
        )
        .unwrap();
        let valid = cuboid(Point3::new(0.0, 0.0, 0.0), [1.0; 3], [0.0; 3]);
        assert!(
            solid_contact(&unsupported, &valid)
                .unwrap_err()
                .to_string()
                .contains("cannot be lowered")
        );
    }

    fn cuboid(center: Point3, half: [f64; 3], rotation: [f64; 3]) -> PhysicalSolid {
        let mut points = Vec::with_capacity(8);
        for [x, y, z] in [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ] {
            let rotated = rotate(Point3::new(x * half[0], y * half[1], z * half[2]), rotation);
            points.push(Point3::new(
                center.x + rotated.x,
                center.y + rotated.y,
                center.z + rotated.z,
            ));
        }
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
        let mesh = TriMesh { points, triangles };
        let piece = ConvexPiece::new(mesh.clone()).unwrap();
        PhysicalSolid::new(mesh, vec![piece]).unwrap()
    }

    fn rotate(point: Point3, [rx, ry, rz]: [f64; 3]) -> Point3 {
        let (sin_x, cos_x) = rx.sin_cos();
        let (sin_y, cos_y) = ry.sin_cos();
        let (sin_z, cos_z) = rz.sin_cos();
        let after_x = Point3::new(
            point.x,
            point.y * cos_x - point.z * sin_x,
            point.y * sin_x + point.z * cos_x,
        );
        let after_y = Point3::new(
            after_x.x * cos_y + after_x.z * sin_y,
            after_x.y,
            -after_x.x * sin_y + after_x.z * cos_y,
        );
        Point3::new(
            after_y.x * cos_z - after_y.y * sin_z,
            after_y.x * sin_z + after_y.y * cos_z,
            after_y.z,
        )
    }
}
