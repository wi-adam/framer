//! Derives room boundaries from wall topology.
//!
//! Walls form a planar graph whose nodes are wall endpoints and whose edges are
//! the walls. The bounded faces of that graph are the enclosed rooms. Given a
//! `seed` point we find the bounded face containing it and report its boundary
//! polygon, area, and perimeter. The math is general over straight segments, but
//! Slice 2 only connects walls that share endpoints (corner-formed loops);
//! mid-span (Tee) intersections arrive with interior-wall framing.

use std::collections::HashSet;

use crate::{BuildingModel, Length, Point2};

/// The derived geometry of a room: its boundary loop (counterclockwise), its
/// perimeter, and its area. None of this is persisted — it is recomputed from
/// the walls whenever they change.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomBoundary {
    pub vertices: Vec<Point2>,
    pub perimeter: Length,
    area_square_inches: f64,
}

impl RoomBoundary {
    pub fn area_square_inches(&self) -> f64 {
        self.area_square_inches
    }

    pub fn area_square_feet(&self) -> f64 {
        self.area_square_inches / 144.0
    }
}

/// The boundary of the room enclosing `seed`, or `None` if `seed` is not inside
/// any closed wall loop.
pub fn room_boundary(model: &BuildingModel, seed: Point2) -> Option<RoomBoundary> {
    bounded_faces(model)
        .into_iter()
        .find(|vertices| point_in_polygon(seed, vertices))
        .map(RoomBoundary::from_vertices)
}

/// Resolve many room seeds against the wall graph at once: the bounded faces are
/// computed a single time and each seed (in order) is matched to its enclosing
/// face. Returns `None` for any seed not inside a closed loop. Prefer this over
/// calling [`room_boundary`] in a loop.
pub fn room_boundaries(model: &BuildingModel, seeds: &[Point2]) -> Vec<Option<RoomBoundary>> {
    let faces = bounded_faces(model);
    seeds
        .iter()
        .map(|seed| {
            faces
                .iter()
                .find(|vertices| point_in_polygon(*seed, vertices))
                .cloned()
                .map(RoomBoundary::from_vertices)
        })
        .collect()
}

impl RoomBoundary {
    fn from_vertices(vertices: Vec<Point2>) -> Self {
        let perimeter = polygon_perimeter(&vertices);
        let area_square_inches = signed_area_square_inches(&vertices).abs();
        Self {
            vertices,
            perimeter,
            area_square_inches,
        }
    }
}

/// All bounded faces of the wall graph, each as a counterclockwise vertex loop.
fn bounded_faces(model: &BuildingModel) -> Vec<Vec<Point2>> {
    // Nodes: unique wall endpoints.
    let mut nodes: Vec<Point2> = Vec::new();
    let node_index = |point: Point2, nodes: &mut Vec<Point2>| -> usize {
        match nodes.iter().position(|candidate| *candidate == point) {
            Some(index) => index,
            None => {
                nodes.push(point);
                nodes.len() - 1
            }
        }
    };

    // Directed half-edges. CRITICAL INVARIANT: each wall pushes its two
    // directions consecutively, so the twin of edge `i` is always `i ^ 1`. The
    // face traversal below relies on this; do not reorder these pushes.
    let mut half_edges: Vec<(usize, usize)> = Vec::new();
    let mut seen_edges: HashSet<(usize, usize)> = HashSet::new();
    for wall in &model.walls {
        if wall.start == wall.end {
            continue;
        }
        let from = node_index(wall.start, &mut nodes);
        let to = node_index(wall.end, &mut nodes);
        // Skip a duplicate (overlapping) wall so the graph has no parallel edges.
        let undirected = (from.min(to), from.max(to));
        if !seen_edges.insert(undirected) {
            continue;
        }
        half_edges.push((from, to));
        half_edges.push((to, from));
    }

    if half_edges.is_empty() {
        return Vec::new();
    }

    // Outgoing half-edges per node, sorted counterclockwise by direction angle.
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (index, &(from, _)) in half_edges.iter().enumerate() {
        outgoing[from].push(index);
    }
    let angle = |index: usize| -> f64 {
        let (from, to) = half_edges[index];
        let delta = nodes[to];
        let origin = nodes[from];
        f64::atan2(
            (delta.y - origin.y).ticks() as f64,
            (delta.x - origin.x).ticks() as f64,
        )
    };
    for edges in &mut outgoing {
        edges.sort_by(|&a, &b| angle(a).total_cmp(&angle(b)));
    }

    // next(e): at the head of e, take the twin then its clockwise neighbour. This
    // traces each bounded face counterclockwise (positive signed area) and the
    // single outer face clockwise (negative).
    let next = |edge: usize| -> usize {
        let twin = edge ^ 1;
        let head = half_edges[twin].0;
        let ring = &outgoing[head];
        let position = ring
            .iter()
            .position(|&candidate| candidate == twin)
            .unwrap();
        ring[(position + ring.len() - 1) % ring.len()]
    };

    let mut visited = vec![false; half_edges.len()];
    let mut faces = Vec::new();
    for start in 0..half_edges.len() {
        if visited[start] {
            continue;
        }
        let mut cycle = Vec::new();
        let mut current = start;
        loop {
            visited[current] = true;
            cycle.push(half_edges[current].0);
            current = next(current);
            if current == start {
                break;
            }
        }
        let vertices: Vec<Point2> = cycle.iter().map(|&node| nodes[node]).collect();
        if signed_area_square_inches(&vertices) > 0.0 {
            faces.push(vertices);
        }
    }
    faces
}

/// Shoelace signed area in square inches (counterclockwise positive).
fn signed_area_square_inches(vertices: &[Point2]) -> f64 {
    if vertices.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for index in 0..vertices.len() {
        let current = vertices[index];
        let next = vertices[(index + 1) % vertices.len()];
        sum += current.x.inches() * next.y.inches() - next.x.inches() * current.y.inches();
    }
    sum / 2.0
}

fn polygon_perimeter(vertices: &[Point2]) -> Length {
    let mut total = Length::ZERO;
    for index in 0..vertices.len() {
        let current = vertices[index];
        let next = vertices[(index + 1) % vertices.len()];
        let dx = (next.x - current.x).inches();
        let dy = (next.y - current.y).inches();
        total += Length::from_inches(dx.hypot(dy));
    }
    total
}

/// Even-odd ray-casting point-in-polygon test (boundary inclusion unspecified).
fn point_in_polygon(point: Point2, vertices: &[Point2]) -> bool {
    let (px, py) = (point.x.inches(), point.y.inches());
    let mut inside = false;
    let mut j = vertices.len() - 1;
    for i in 0..vertices.len() {
        let (xi, yi) = (vertices[i].x.inches(), vertices[i].y.inches());
        let (xj, yj) = (vertices[j].x.inches(), vertices[j].y.inches());
        let intersects = (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi;
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildingModel, CodeProfile, Length, Point2, Wall};

    fn p(x_in: f64, y_in: f64) -> Point2 {
        Point2::new(Length::from_inches(x_in), Length::from_inches(y_in))
    }

    fn wall(id: &str, a: Point2, b: Point2) -> Wall {
        let code = CodeProfile::irc_2021_prescriptive();
        Wall::new(id, id, Length::from_feet(1.0), &code).with_placement("level-1", a, b)
    }

    /// A closed `w_ft` × `h_ft` rectangle of four walls at the origin.
    fn rect_model(w_ft: f64, h_ft: f64) -> BuildingModel {
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let (w, h) = (w_ft * 12.0, h_ft * 12.0);
        model.walls.push(wall("w-b", p(0.0, 0.0), p(w, 0.0)));
        model.walls.push(wall("w-r", p(w, 0.0), p(w, h)));
        model.walls.push(wall("w-t", p(w, h), p(0.0, h)));
        model.walls.push(wall("w-l", p(0.0, h), p(0.0, 0.0)));
        model
    }

    #[test]
    fn rectangle_yields_room_boundary_with_correct_area() {
        let model = rect_model(12.0, 8.0);
        let boundary = room_boundary(&model, p(72.0, 48.0)).expect("seed is inside");

        assert_eq!(boundary.vertices.len(), 4);
        assert!((boundary.area_square_feet() - 96.0).abs() < 1e-6);
        assert_eq!(boundary.perimeter, Length::from_feet(40.0));
    }

    #[test]
    fn seed_outside_walls_has_no_boundary() {
        let model = rect_model(12.0, 8.0);
        assert!(room_boundary(&model, p(10_000.0, 10_000.0)).is_none());
    }

    #[test]
    fn open_chain_has_no_boundary() {
        // A U shape (three walls) never closes, so no bounded face exists.
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        model.walls.push(wall("w-l", p(0.0, 96.0), p(0.0, 0.0)));
        model.walls.push(wall("w-b", p(0.0, 0.0), p(120.0, 0.0)));
        model.walls.push(wall("w-r", p(120.0, 0.0), p(120.0, 96.0)));

        assert!(room_boundary(&model, p(60.0, 48.0)).is_none());
    }

    #[test]
    fn two_rooms_sharing_a_wall_resolve_separately() {
        // Two 6ft × 8ft rooms side by side sharing the divider at x = 6ft. The
        // bottom and top runs are split at the divider so every meeting is a node
        // (corner-formed, no mid-span crossing).
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let (six, twelve, eight) = (72.0, 144.0, 96.0);
        // Bottom: two segments meeting at (6ft, 0).
        model.walls.push(wall("b1", p(0.0, 0.0), p(six, 0.0)));
        model.walls.push(wall("b2", p(six, 0.0), p(twelve, 0.0)));
        // Top: two segments meeting at (6ft, 8ft).
        model
            .walls
            .push(wall("t1", p(twelve, eight), p(six, eight)));
        model.walls.push(wall("t2", p(six, eight), p(0.0, eight)));
        // Outer sides + shared divider.
        model.walls.push(wall("l", p(0.0, eight), p(0.0, 0.0)));
        model
            .walls
            .push(wall("r", p(twelve, 0.0), p(twelve, eight)));
        model.walls.push(wall("d", p(six, 0.0), p(six, eight)));

        let left = room_boundary(&model, p(36.0, 48.0)).expect("left room");
        let right = room_boundary(&model, p(108.0, 48.0)).expect("right room");

        assert!((left.area_square_feet() - 48.0).abs() < 1e-6);
        assert!((right.area_square_feet() - 48.0).abs() < 1e-6);
        assert_eq!(left.vertices.len(), 4);
        assert_eq!(right.vertices.len(), 4);
    }

    #[test]
    fn l_shaped_room_resolves_with_correct_area() {
        // A concave L-shaped loop (a 12×12 square with a 6×6 bite out of the
        // top-right): 12*12 - 6*6 = 108 sq ft. Six walls, all corner-joined.
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let (a, b) = (72.0, 144.0); // 6ft, 12ft in inches
        let pts = [p(0.0, 0.0), p(b, 0.0), p(b, a), p(a, a), p(a, b), p(0.0, b)];
        for index in 0..pts.len() {
            let next = (index + 1) % pts.len();
            model
                .walls
                .push(wall(&format!("w-{index}"), pts[index], pts[next]));
        }

        let boundary = room_boundary(&model, p(36.0, 36.0)).expect("seed inside the L");

        assert_eq!(boundary.vertices.len(), 6);
        assert!((boundary.area_square_feet() - 108.0).abs() < 1e-6);
    }

    #[test]
    fn room_boundaries_batch_matches_seeds_in_order() {
        let model = rect_model(12.0, 8.0);
        let results = room_boundaries(&model, &[p(72.0, 48.0), p(10_000.0, 10_000.0)]);

        assert_eq!(results.len(), 2);
        assert!(
            results[0]
                .as_ref()
                .is_some_and(|b| (b.area_square_feet() - 96.0).abs() < 1e-6)
        );
        assert!(results[1].is_none());
    }

    #[test]
    fn boundary_is_deterministic() {
        let model = rect_model(10.0, 10.0);
        let first = room_boundary(&model, p(60.0, 60.0)).unwrap();
        let second = room_boundary(&model, p(60.0, 60.0)).unwrap();
        assert_eq!(first.vertices, second.vertices);
    }
}
