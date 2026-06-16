//! A bounding-volume hierarchy with a flat node array that ports directly to a
//! GPU storage buffer. Built with midpoint/median splits (deterministic), and
//! traversed iteratively with a fixed-size stack — exactly as the WGSL renderer
//! must do, since GPUs have no recursion.

use crate::aabb::Aabb;
use crate::geom::{Hit, Triangle};
use crate::math::Vec3;
use crate::ray::Ray;

/// Leaves hold at most this many triangles.
const MAX_LEAF: usize = 4;
/// Traversal stack depth. The tree is balanced (median split), so depth is
/// `ceil(log2(n / MAX_LEAF))`; 64 covers astronomically large scenes.
const STACK_SIZE: usize = 64;

/// A flattened BVH node. `count > 0` marks a leaf whose triangles are
/// `indices[left_first .. left_first + count]`; `count == 0` marks an internal
/// node whose children are nodes `left_first` and `left_first + 1`.
#[derive(Clone, Copy, Debug)]
pub struct BvhNode {
    pub aabb: Aabb,
    pub left_first: u32,
    pub count: u32,
}

/// A bounding-volume hierarchy over a triangle slice. `indices` is a permutation
/// of triangle indices; nodes reference contiguous ranges of it.
#[derive(Clone, Debug)]
pub struct Bvh {
    pub nodes: Vec<BvhNode>,
    pub indices: Vec<u32>,
}

#[inline]
fn axis_value(v: Vec3, axis: usize) -> f32 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

impl Bvh {
    /// Builds a balanced median-split BVH. Empty input yields an empty hierarchy
    /// that never reports hits.
    pub fn build(tris: &[Triangle]) -> Self {
        let mut nodes: Vec<BvhNode> = Vec::new();
        let mut indices: Vec<u32> = (0..tris.len() as u32).collect();
        if tris.is_empty() {
            return Self { nodes, indices };
        }
        let centroids: Vec<Vec3> = tris.iter().map(Triangle::centroid).collect();
        let tri_aabbs: Vec<Aabb> = tris.iter().map(Triangle::aabb).collect();

        let root = BvhNode {
            aabb: bounds_of(&indices, 0, indices.len(), &tri_aabbs),
            left_first: 0,
            count: indices.len() as u32,
        };
        nodes.push(root);
        subdivide(&mut nodes, &mut indices, 0, &centroids, &tri_aabbs);
        Self { nodes, indices }
    }

    /// Returns the nearest triangle hit along `ray`, or `None`.
    pub fn intersect(&self, tris: &[Triangle], ray: &Ray) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut stack = [0u32; STACK_SIZE];
        let mut sp = 0usize;
        stack[sp] = 0;
        sp += 1;
        let mut best: Option<Hit> = None;
        let mut closest = ray.t_max;

        while sp > 0 {
            sp -= 1;
            let node = &self.nodes[stack[sp] as usize];
            if !node.aabb.hit(ray, closest) {
                continue;
            }
            if node.count > 0 {
                for k in 0..node.count {
                    let tri = &tris[self.indices[(node.left_first + k) as usize] as usize];
                    if let Some(hit) = tri.intersect(ray) {
                        if hit.t < closest {
                            closest = hit.t;
                            best = Some(hit);
                        }
                    }
                }
            } else {
                stack[sp] = node.left_first;
                sp += 1;
                stack[sp] = node.left_first + 1;
                sp += 1;
            }
        }
        best
    }

    /// Returns true if any triangle blocks `ray` within its `[t_min, t_max]`
    /// range — the shadow-ray query, which stops at the first hit.
    pub fn occluded(&self, tris: &[Triangle], ray: &Ray) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let mut stack = [0u32; STACK_SIZE];
        let mut sp = 0usize;
        stack[sp] = 0;
        sp += 1;

        while sp > 0 {
            sp -= 1;
            let node = &self.nodes[stack[sp] as usize];
            if !node.aabb.hit(ray, ray.t_max) {
                continue;
            }
            if node.count > 0 {
                for k in 0..node.count {
                    let tri = &tris[self.indices[(node.left_first + k) as usize] as usize];
                    if tri.intersect(ray).is_some() {
                        return true;
                    }
                }
            } else {
                stack[sp] = node.left_first;
                sp += 1;
                stack[sp] = node.left_first + 1;
                sp += 1;
            }
        }
        false
    }
}

fn bounds_of(indices: &[u32], start: usize, end: usize, tri_aabbs: &[Aabb]) -> Aabb {
    let mut bb = Aabb::EMPTY;
    for &i in &indices[start..end] {
        bb = bb.union(&tri_aabbs[i as usize]);
    }
    bb
}

/// Recursively splits the node covering `nodes[node_index]`'s index range.
fn subdivide(
    nodes: &mut Vec<BvhNode>,
    indices: &mut [u32],
    node_index: usize,
    centroids: &[Vec3],
    tri_aabbs: &[Aabb],
) {
    let start = nodes[node_index].left_first as usize;
    let count = nodes[node_index].count as usize;
    let end = start + count;
    if count <= MAX_LEAF {
        return;
    }

    // Split along the axis with the largest centroid spread.
    let mut centroid_bounds = Aabb::EMPTY;
    for &i in &indices[start..end] {
        centroid_bounds.grow(centroids[i as usize]);
    }
    let extent = centroid_bounds.extent();
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    if axis_value(extent, axis) <= 0.0 {
        return; // all centroids coincide — keep as a (larger) leaf
    }

    // Balanced median split: sort the range by centroid on `axis` (index
    // tie-break keeps it deterministic across architectures), split in half.
    indices[start..end].sort_by(|&a, &b| {
        let ca = axis_value(centroids[a as usize], axis);
        let cb = axis_value(centroids[b as usize], axis);
        ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b))
    });
    let mid = start + count / 2;

    let left_child = nodes.len();
    nodes.push(BvhNode {
        aabb: bounds_of(indices, start, mid, tri_aabbs),
        left_first: start as u32,
        count: (mid - start) as u32,
    });
    nodes.push(BvhNode {
        aabb: bounds_of(indices, mid, end, tri_aabbs),
        left_first: mid as u32,
        count: (end - mid) as u32,
    });
    nodes[node_index].count = 0;
    nodes[node_index].left_first = left_child as u32;

    subdivide(nodes, indices, left_child, centroids, tri_aabbs);
    subdivide(nodes, indices, left_child + 1, centroids, tri_aabbs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Triangle;
    use crate::math::Vec3;
    use crate::ray::Ray;
    use crate::rng::Pcg32;

    fn random_triangles(n: usize, rng: &mut Pcg32) -> Vec<Triangle> {
        let mut tris = Vec::with_capacity(n);
        for i in 0..n {
            let base = Vec3::new(
                rng.next_f32() * 10.0 - 5.0,
                rng.next_f32() * 10.0 - 5.0,
                rng.next_f32() * 10.0 - 5.0,
            );
            let j1 = Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32());
            let j2 = Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32());
            tris.push(Triangle::new(base, base + j1, base + j2, i as u32));
        }
        tris
    }

    /// Reference nearest-hit by testing every triangle.
    fn brute_force(tris: &[Triangle], ray: &Ray) -> Option<(f32, u32)> {
        let mut best: Option<(f32, u32)> = None;
        for tri in tris {
            if let Some(hit) = tri.intersect(ray) {
                if best.is_none_or(|(t, _)| hit.t < t) {
                    best = Some((hit.t, hit.material));
                }
            }
        }
        best
    }

    #[test]
    fn traversal_matches_brute_force() {
        let mut rng = Pcg32::seed(2024, 1);
        let tris = random_triangles(400, &mut rng);
        let bvh = Bvh::build(&tris);

        let mut ray_rng = Pcg32::seed(99, 7);
        let mut tested = 0;
        let mut hits = 0;
        for _ in 0..3000 {
            let origin = Vec3::new(
                ray_rng.next_f32() * 16.0 - 8.0,
                ray_rng.next_f32() * 16.0 - 8.0,
                ray_rng.next_f32() * 16.0 - 8.0,
            );
            let dir = Vec3::new(
                ray_rng.next_f32() * 2.0 - 1.0,
                ray_rng.next_f32() * 2.0 - 1.0,
                ray_rng.next_f32() * 2.0 - 1.0,
            )
            .normalize();
            let ray = Ray::new(origin, dir);

            let expected = brute_force(&tris, &ray);
            let actual = bvh.intersect(&tris, &ray).map(|h| (h.t, h.material));
            match (expected, actual) {
                (None, None) => {}
                (Some((te, me)), Some((ta, ma))) => {
                    assert!((te - ta).abs() < 1e-3, "t mismatch {te} vs {ta}");
                    assert_eq!(me, ma, "material mismatch");
                    hits += 1;
                }
                (e, a) => panic!("hit disagreement: brute={e:?} bvh={a:?}"),
            }
            tested += 1;
        }
        assert_eq!(tested, 3000);
        assert!(hits > 50, "test scene too sparse to be meaningful ({hits} hits)");
    }

    #[test]
    fn every_triangle_is_reachable() {
        // Fire a ray straight at each triangle's centroid; the BVH must find a hit.
        let mut rng = Pcg32::seed(5, 5);
        let tris = random_triangles(120, &mut rng);
        let bvh = Bvh::build(&tris);
        for tri in &tris {
            let c = tri.centroid();
            let origin = c + tri.geom_normal * 3.0;
            let ray = Ray::new(origin, -tri.geom_normal);
            assert!(
                bvh.intersect(&tris, &ray).is_some(),
                "missed a triangle the brute-force tracer would hit"
            );
        }
    }

    #[test]
    fn occluded_matches_intersect() {
        let mut rng = Pcg32::seed(11, 13);
        let tris = random_triangles(200, &mut rng);
        let bvh = Bvh::build(&tris);
        let mut ray_rng = Pcg32::seed(3, 1);
        for _ in 0..2000 {
            let origin = Vec3::new(
                ray_rng.next_f32() * 14.0 - 7.0,
                ray_rng.next_f32() * 14.0 - 7.0,
                ray_rng.next_f32() * 14.0 - 7.0,
            );
            let dir = Vec3::new(
                ray_rng.next_f32() * 2.0 - 1.0,
                ray_rng.next_f32() * 2.0 - 1.0,
                ray_rng.next_f32() * 2.0 - 1.0,
            )
            .normalize();
            let ray = Ray::new(origin, dir);
            assert_eq!(bvh.occluded(&tris, &ray), bvh.intersect(&tris, &ray).is_some());
        }
    }

    #[test]
    fn single_triangle() {
        let tris = vec![Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            0,
        )];
        let bvh = Bvh::build(&tris);
        let ray = Ray::new(Vec3::new(0.2, 0.2, 5.0), Vec3::new(0.0, 0.0, -1.0));
        assert!(bvh.intersect(&tris, &ray).is_some());
        let miss = Ray::new(Vec3::new(5.0, 5.0, 5.0), Vec3::new(0.0, 0.0, -1.0));
        assert!(bvh.intersect(&tris, &miss).is_none());
    }

    #[test]
    fn empty_scene_never_hits() {
        let tris: Vec<Triangle> = Vec::new();
        let bvh = Bvh::build(&tris);
        let ray = Ray::new(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0));
        assert!(bvh.intersect(&tris, &ray).is_none());
        assert!(!bvh.occluded(&tris, &ray));
    }

    #[test]
    fn node_aabbs_enclose_children() {
        let mut rng = Pcg32::seed(8, 8);
        let tris = random_triangles(150, &mut rng);
        let bvh = Bvh::build(&tris);
        for node in &bvh.nodes {
            if node.count == 0 {
                let l = &bvh.nodes[node.left_first as usize];
                let r = &bvh.nodes[node.left_first as usize + 1];
                let union = l.aabb.union(&r.aabb);
                // Parent must contain both children (allow tiny float slack).
                assert!(node.aabb.min.x <= union.min.x + 1e-3);
                assert!(node.aabb.max.x >= union.max.x - 1e-3);
            }
        }
    }
}
