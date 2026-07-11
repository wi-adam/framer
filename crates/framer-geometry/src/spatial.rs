use std::collections::BTreeSet;

use rstar::{AABB as RstarAabb, RTree, RTreeObject};

use crate::{Aabb, PhysicalBody};

/// Canonical indices into a [`PhysicalScene`](crate::PhysicalScene) body slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CandidatePair {
    pub left: usize,
    pub right: usize,
}

impl CandidatePair {
    fn new(left: usize, right: usize) -> Option<Self> {
        (left != right).then(|| {
            let (left, right) = if left < right {
                (left, right)
            } else {
                (right, left)
            };
            Self { left, right }
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct IndexedBody {
    index: usize,
    envelope: RstarAabb<[f64; 3]>,
}

impl RTreeObject for IndexedBody {
    type Envelope = RstarAabb<[f64; 3]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

/// Generate every body pair whose axis-aligned bounds intersect. R-tree
/// traversal order is intentionally discarded in favor of canonical indices.
pub(crate) fn candidate_pairs(bodies: &[PhysicalBody]) -> Vec<CandidatePair> {
    let indexed: Vec<_> = bodies
        .iter()
        .enumerate()
        .map(|(index, body)| IndexedBody {
            index,
            envelope: envelope(body.aabb),
        })
        .collect();
    let tree = RTree::bulk_load(indexed);
    let mut pairs = BTreeSet::new();
    for body in tree.iter() {
        for other in tree.locate_in_envelope_intersecting(body.envelope) {
            if let Some(pair) = CandidatePair::new(body.index, other.index) {
                pairs.insert(pair);
            }
        }
    }
    pairs.into_iter().collect()
}

fn envelope(aabb: Aabb) -> RstarAabb<[f64; 3]> {
    RstarAabb::from_corners(
        [aabb.min.x, aabb.min.y, aabb.min.z],
        [aabb.max.x, aabb.max.y, aabb.max.z],
    )
}

#[cfg(test)]
mod tests {
    use framer_core::ElementId;

    use super::*;
    use crate::{AssemblyKind, BodyRef, ConvexPiece, PhysicalSolid, Point3, TriMesh};

    #[test]
    fn rtree_candidates_match_the_brute_force_oracle() {
        let specs = [
            (0.0, 0.0, 0.0, 1.0),
            (1.0, 0.0, 0.0, 1.0), // face contact with 0
            (2.25, 0.0, 0.0, 1.0),
            (0.25, 0.25, 0.25, 0.2), // contained by 0
            (0.0, 1.0, 1.0, 1.0),    // edge contact with 0
            (-3.0, -2.0, 4.0, 0.5),
            (-3.4, -2.0, 4.0, 0.5),
        ];
        let bodies: Vec<_> = specs
            .into_iter()
            .enumerate()
            .map(|(index, (x, y, z, size))| test_body(index, x, y, z, size))
            .collect();

        let indexed = candidate_pairs(&bodies);
        let brute_force: Vec<_> = (0..bodies.len())
            .flat_map(|left| ((left + 1)..bodies.len()).map(move |right| (left, right)))
            .filter(|&(left, right)| intersects(bodies[left].aabb, bodies[right].aabb))
            .map(|(left, right)| CandidatePair { left, right })
            .collect();
        assert_eq!(indexed, brute_force);
    }

    fn intersects(left: Aabb, right: Aabb) -> bool {
        left.min.x <= right.max.x
            && left.max.x >= right.min.x
            && left.min.y <= right.max.y
            && left.max.y >= right.min.y
            && left.min.z <= right.max.z
            && left.max.z >= right.min.z
    }

    fn test_body(index: usize, x: f64, y: f64, z: f64, size: f64) -> PhysicalBody {
        let points = vec![
            Point3::new(x, y, z),
            Point3::new(x + size, y, z),
            Point3::new(x + size, y + size, z),
            Point3::new(x, y + size, z),
            Point3::new(x, y, z + size),
            Point3::new(x + size, y, z + size),
            Point3::new(x + size, y + size, z + size),
            Point3::new(x, y + size, z + size),
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
        let mesh = TriMesh { points, triangles };
        let piece = ConvexPiece::new(mesh.clone()).unwrap();
        let solid = PhysicalSolid::new(mesh, vec![piece]).unwrap();
        PhysicalBody::new(
            BodyRef::assembly(ElementId::new(format!("body-{index}")), AssemblyKind::Wall),
            solid,
        )
    }
}
