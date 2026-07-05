//! Derives room boundaries from wall topology.
//!
//! Walls form a planar graph whose nodes are wall endpoints and whose edges are
//! the walls. The bounded faces of that graph are the enclosed rooms. Given a
//! `seed` point we find the bounded face containing it and report its boundary
//! polygon, area, and perimeter. Walls connect at shared endpoints (corners) and
//! at mid-span junctions (a Tee partition meeting a through wall), which are
//! split into sub-edges so interior walls carve rooms. The math is general over
//! straight segments.

use std::collections::{BTreeMap, HashSet};

use crate::{BuildingModel, ElementId, Length, Point2, Room, Wall};

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
///
/// This legacy helper considers walls across all levels. Prefer
/// [`room_boundary_on_level`] for authored room/surface lookups that belong to a
/// specific level.
pub fn room_boundary(model: &BuildingModel, seed: Point2) -> Option<RoomBoundary> {
    bounded_faces(model)
        .into_iter()
        .find(|vertices| point_in_polygon(seed, vertices))
        .map(RoomBoundary::from_vertices)
}

/// The boundary of the room enclosing `seed` on `level`, or `None` if that
/// level's walls do not enclose the point.
pub fn room_boundary_on_level(
    model: &BuildingModel,
    level: &ElementId,
    seed: Point2,
) -> Option<RoomBoundary> {
    bounded_faces_on_level(model, level)
        .into_iter()
        .find(|vertices| point_in_polygon(seed, vertices))
        .map(RoomBoundary::from_vertices)
}

/// Resolve many room seeds against the wall graph at once: the bounded faces are
/// computed a single time and each seed (in order) is matched to its enclosing
/// face. Returns `None` for any seed not inside a closed loop.
///
/// This legacy helper considers walls across all levels. Prefer
/// [`room_boundaries_on_level`] or [`room_boundaries_for_rooms`] for authored
/// room/surface lookups that belong to specific levels.
pub fn room_boundaries(model: &BuildingModel, seeds: &[Point2]) -> Vec<Option<RoomBoundary>> {
    let faces = bounded_faces(model);
    boundaries_from_faces(&faces, seeds)
}

/// Resolve many room seeds against one level's wall graph at once. Prefer this
/// over calling [`room_boundary_on_level`] in a loop.
pub fn room_boundaries_on_level(
    model: &BuildingModel,
    level: &ElementId,
    seeds: &[Point2],
) -> Vec<Option<RoomBoundary>> {
    let faces = bounded_faces_on_level(model, level);
    boundaries_from_faces(&faces, seeds)
}

/// Resolve authored rooms through their own level's wall graph in one pass per
/// distinct level. Results preserve `rooms` order; `None` marks a room whose seed
/// is not enclosed on that room's level.
pub fn room_boundaries_for_rooms(
    model: &BuildingModel,
    rooms: &[&Room],
) -> Vec<Option<RoomBoundary>> {
    let mut batches: BTreeMap<ElementId, Vec<(usize, Point2)>> = BTreeMap::new();
    for (slot, room) in rooms.iter().enumerate() {
        batches
            .entry(room.level.clone())
            .or_default()
            .push((slot, room.seed));
    }

    let mut resolved = vec![None; rooms.len()];
    for (level, entries) in batches {
        let seeds: Vec<Point2> = entries.iter().map(|(_, seed)| *seed).collect();
        for ((slot, _), boundary) in entries
            .into_iter()
            .zip(room_boundaries_on_level(model, &level, &seeds))
        {
            resolved[slot] = boundary;
        }
    }
    resolved
}

fn boundaries_from_faces(faces: &[Vec<Point2>], seeds: &[Point2]) -> Vec<Option<RoomBoundary>> {
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

/// The outer loop of a level whose walls form exactly one simple closed footprint:
/// every endpoint has degree two, with no loose ends or interior partitions. This
/// is intentionally narrower than [`room_boundary`]: authoring tools use it when
/// they need a perimeter footprint (for example an L-shaped roof auto-generator)
/// and should fall back when the wall graph is more complex.
pub fn level_wall_loop_outline(model: &BuildingModel, level: &ElementId) -> Option<Vec<Point2>> {
    let mut adjacency: BTreeMap<(i64, i64), Vec<(i64, i64)>> = BTreeMap::new();
    for wall in model.walls.iter().filter(|wall| &wall.level == level) {
        if wall.start == wall.end {
            continue;
        }
        let start = point_key(wall.start);
        let end = point_key(wall.end);
        if start == end {
            continue;
        }
        adjacency.entry(start).or_default().push(end);
        adjacency.entry(end).or_default().push(start);
    }
    if adjacency.len() < 3 {
        return None;
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
        neighbors.dedup();
        if neighbors.len() != 2 {
            return None;
        }
    }

    let start = *adjacency.keys().next()?;
    let mut outline = Vec::with_capacity(adjacency.len());
    let mut previous = None;
    let mut current = start;
    loop {
        outline.push(current);
        let neighbors = adjacency.get(&current)?;
        let next = match previous {
            Some(previous) if neighbors[0] == previous => neighbors[1],
            Some(_) => neighbors[0],
            None => neighbors[0],
        };
        previous = Some(current);
        current = next;
        if current == start {
            break;
        }
        if outline.len() > adjacency.len() {
            return None;
        }
    }
    if outline.len() != adjacency.len() {
        return None;
    }

    let mut points: Vec<Point2> = outline.into_iter().map(point_from_key).collect();
    if polygon_signed_area2(&points) < 0 {
        points.reverse();
    }
    Some(points)
}

/// Reflex vertices of a simple polygon, returned after normalizing the loop to
/// counterclockwise winding. Orthogonal L/T roof generation and valley detection
/// use these corners as the footprint signal for an interior valley.
pub fn concave_polygon_corners(vertices: &[Point2]) -> Vec<Point2> {
    if vertices.len() < 4 {
        return Vec::new();
    }
    let mut outline = vertices.to_vec();
    if polygon_signed_area2(&outline) < 0 {
        outline.reverse();
    }
    let mut corners = Vec::new();
    for index in 0..outline.len() {
        let previous = outline[(index + outline.len() - 1) % outline.len()];
        let current = outline[index];
        let next = outline[(index + 1) % outline.len()];
        if cross2(previous, current, next) < 0 {
            corners.push(current);
        }
    }
    corners
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
    bounded_faces_for_walls(model.walls.iter())
}

fn bounded_faces_on_level(model: &BuildingModel, level: &ElementId) -> Vec<Vec<Point2>> {
    bounded_faces_for_walls(model.walls.iter().filter(|wall| &wall.level == level))
}

fn bounded_faces_for_walls<'a>(walls: impl IntoIterator<Item = &'a Wall>) -> Vec<Vec<Point2>> {
    // Pass 1: nodes are the unique wall endpoints (these are also the mid-span
    // junction points, since a Tee partition's endpoint lands on a through wall).
    let mut nodes: Vec<Point2> = Vec::new();
    let walls: Vec<&Wall> = walls.into_iter().collect();
    for wall in &walls {
        if wall.start == wall.end {
            continue;
        }
        for endpoint in [wall.start, wall.end] {
            if !nodes.contains(&endpoint) {
                nodes.push(endpoint);
            }
        }
    }
    let find = |point: Point2| {
        nodes
            .iter()
            .position(|candidate| *candidate == point)
            .unwrap()
    };

    // Pass 2: split each wall at any node lying on its interior (a Tee/Cross
    // junction), then emit a half-edge pair per sub-edge. CRITICAL INVARIANT:
    // each sub-edge pushes its two directions consecutively, so the twin of edge
    // `i` is always `i ^ 1`. The face traversal below relies on this.
    let mut half_edges: Vec<(usize, usize)> = Vec::new();
    let mut seen_edges: HashSet<(usize, usize)> = HashSet::new();
    for wall in &walls {
        if wall.start == wall.end {
            continue;
        }
        let mut chain: Vec<Point2> = vec![wall.start, wall.end];
        for &node in &nodes {
            if wall.point_on_interior(node) {
                chain.push(node);
            }
        }
        // Order the points along the wall by distance from its start.
        chain.sort_by_key(|point| {
            let dx = (point.x - wall.start.x).ticks();
            let dy = (point.y - wall.start.y).ticks();
            dx * dx + dy * dy
        });
        chain.dedup();

        for pair in chain.windows(2) {
            let from = find(pair[0]);
            let to = find(pair[1]);
            // Skip a duplicate (overlapping) sub-edge so the graph has no parallel edges.
            let undirected = (from.min(to), from.max(to));
            if !seen_edges.insert(undirected) {
                continue;
            }
            half_edges.push((from, to));
            half_edges.push((to, from));
        }
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

fn point_key(point: Point2) -> (i64, i64) {
    (point.x.ticks(), point.y.ticks())
}

fn point_from_key((x, y): (i64, i64)) -> Point2 {
    Point2::new(Length::from_ticks(x), Length::from_ticks(y))
}

/// The number of enclosed rooms (bounded faces) in the wall graph — the count of
/// closed loops the walls form. Equivalent to the length of [`room_boundaries`]
/// for a seed inside every face, but without needing seed points.
///
/// This legacy helper considers walls across all levels. Prefer
/// [`enclosed_room_count_on_level`] for authored/editing behavior that belongs to
/// a specific level.
pub fn enclosed_room_count(model: &BuildingModel) -> usize {
    bounded_faces(model).len()
}

/// The number of enclosed rooms on one level's wall graph.
pub fn enclosed_room_count_on_level(model: &BuildingModel, level: &ElementId) -> usize {
    bounded_faces_on_level(model, level).len()
}

/// For each wall whose interior side can be determined from the enclosed rooms,
/// which side faces the room interior. The plus-side unit normal is
/// `(-along.y, along.x)` where `along = normalize(end - start)` — the SAME
/// convention the renderers use to lay layers out interior -> exterior. A `true`
/// entry means the interior/room is toward that plus-side.
///
/// Each wall's midpoint is probed a few inches to either side against the bounded
/// faces: if exactly one probe lands inside a room, that determines the side.
/// Walls where both or neither probe is inside (interior partitions dividing two
/// rooms, or open/free walls) are ambiguous and OMITTED from the map.
pub fn wall_interior_sides(model: &BuildingModel) -> BTreeMap<ElementId, bool> {
    let faces = bounded_faces(model);
    let probe = Length::from_whole_inches(6).inches();
    let mut sides = BTreeMap::new();
    for wall in &model.walls {
        if wall.start == wall.end {
            continue;
        }
        let dx = (wall.end.x - wall.start.x).inches();
        let dy = (wall.end.y - wall.start.y).inches();
        let length = (dx * dx + dy * dy).sqrt();
        if length == 0.0 {
            continue;
        }
        let (along_x, along_y) = (dx / length, dy / length);
        // Plus-side unit normal, matching the renderers' `(-along.y, along.x)`.
        let (side_x, side_y) = (-along_y, along_x);
        let mid_x = (wall.start.x + wall.end.x).inches() / 2.0;
        let mid_y = (wall.start.y + wall.end.y).inches() / 2.0;
        let plus = inches_point(mid_x + probe * side_x, mid_y + probe * side_y);
        let minus = inches_point(mid_x - probe * side_x, mid_y - probe * side_y);

        let plus_inside = faces.iter().any(|face| point_in_polygon(plus, face));
        let minus_inside = faces.iter().any(|face| point_in_polygon(minus, face));
        match (plus_inside, minus_inside) {
            (true, false) => {
                sides.insert(wall.id.clone(), true);
            }
            (false, true) => {
                sides.insert(wall.id.clone(), false);
            }
            // Ambiguous: interior partition (both inside) or free wall (neither).
            _ => {}
        }
    }
    sides
}

fn inches_point(x: f64, y: f64) -> Point2 {
    Point2::new(Length::from_inches(x), Length::from_inches(y))
}

/// The unsigned plan area of a closed polygon, in square inches (shoelace). The
/// single owner of this formula — the solver's floor/ceiling takeoff calls it
/// rather than re-deriving it, so the two cannot drift.
pub fn polygon_area_square_inches(vertices: &[Point2]) -> f64 {
    signed_area_square_inches(vertices).abs()
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

/// Triangulate a simple (non-self-intersecting) polygon by ear clipping, returning
/// index triples into `points`. Correct for **convex and concave** outlines — the
/// single source of triangulation so the renderer and the 3-D viewport do not split
/// a roof/ceiling/floor surface differently (a naive vertex-0 fan is wrong for the
/// concave loops `room_boundary` produces, e.g. an L-shaped room). Winding-agnostic:
/// callers assign their own face normal, so the emitted triangle order is not
/// orientation-constrained. Exact integer-tick math (no float) keeps it
/// deterministic. Returns empty for fewer than 3 points; a degenerate (over-collinear)
/// remainder falls back to a fan so the routine always terminates.
pub fn triangulate_simple_polygon(points: &[Point2]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }

    // Remaining original-index ring, normalized to counterclockwise so the ear test
    // (a convex corner has a positive cross product) is consistent.
    let mut ring: Vec<usize> = (0..n).collect();
    if polygon_signed_area2(points) < 0 {
        ring.reverse();
    }

    let mut triangles = Vec::with_capacity(n - 2);
    while ring.len() > 3 {
        let m = ring.len();
        let mut clipped = false;
        for i in 0..m {
            let prev = ring[(i + m - 1) % m];
            let cur = ring[i];
            let next = ring[(i + 1) % m];
            let (a, b, c) = (points[prev], points[cur], points[next]);
            // A convex corner of a CCW ring; a reflex corner cannot be an ear.
            if cross2(a, b, c) <= 0 {
                continue;
            }
            // It is an ear only if no other remaining vertex lies in triangle abc.
            let blocked = ring.iter().any(|&j| {
                j != prev && j != cur && j != next && point_in_triangle_ccw(points[j], a, b, c)
            });
            if blocked {
                continue;
            }
            triangles.push([prev, cur, next]);
            ring.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // No ear found (a degenerate, over-collinear remainder): fan what's left
            // so the routine terminates rather than looping forever.
            for k in 1..ring.len() - 1 {
                triangles.push([ring[0], ring[k], ring[k + 1]]);
            }
            return triangles;
        }
    }
    triangles.push([ring[0], ring[1], ring[2]]);
    triangles
}

/// Twice the shoelace signed area in ticks² (counterclockwise positive). `i128`
/// keeps the products exact for any reachable tick coordinate.
fn polygon_signed_area2(points: &[Point2]) -> i128 {
    let n = points.len();
    let mut sum: i128 = 0;
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        sum +=
            a.x.ticks() as i128 * b.y.ticks() as i128 - b.x.ticks() as i128 * a.y.ticks() as i128;
    }
    sum
}

/// `(b - a) × (c - a)` in ticks² (positive when `a→b→c` turns counterclockwise).
fn cross2(a: Point2, b: Point2, c: Point2) -> i128 {
    let abx = b.x.ticks() as i128 - a.x.ticks() as i128;
    let aby = b.y.ticks() as i128 - a.y.ticks() as i128;
    let acx = c.x.ticks() as i128 - a.x.ticks() as i128;
    let acy = c.y.ticks() as i128 - a.y.ticks() as i128;
    abx * acy - aby * acx
}

/// Whether `p` lies inside (or on the boundary of) the counterclockwise triangle
/// `abc`. Boundary inclusion is conservative so a vertex touching an ear's edge
/// still blocks the clip.
fn point_in_triangle_ccw(p: Point2, a: Point2, b: Point2, c: Point2) -> bool {
    cross2(a, b, p) >= 0 && cross2(b, c, p) >= 0 && cross2(c, a, p) >= 0
}

/// Even-odd ray-casting point-in-polygon test (boundary inclusion unspecified).
/// A polygon needs at least three vertices; fewer encloses no area, so the point
/// is never inside (and this guards the `len() - 1` seed against underflow).
pub fn point_in_polygon(point: Point2, vertices: &[Point2]) -> bool {
    if vertices.len() < 3 {
        return false;
    }
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
    use crate::{BuildingModel, CodeProfile, Length, Level, Point2, Room, RoomUsage, Wall};

    fn p(x_in: f64, y_in: f64) -> Point2 {
        Point2::new(Length::from_inches(x_in), Length::from_inches(y_in))
    }

    fn wall(id: &str, a: Point2, b: Point2) -> Wall {
        wall_on_level(id, "level-1", a, b)
    }

    fn wall_on_level(id: &str, level: &str, a: Point2, b: Point2) -> Wall {
        let code = CodeProfile::irc_2021_prescriptive();
        Wall::new(id, id, Length::from_feet(1.0), &code).with_placement(level, a, b)
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
    fn enclosed_room_count_tracks_closed_loops() {
        // An empty model encloses nothing; an open L of two walls still encloses
        // nothing; closing the rectangle encloses one room; a mid-span partition
        // splits it into two.
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        assert_eq!(enclosed_room_count(&model), 0);

        model.walls.push(wall("w-b", p(0.0, 0.0), p(96.0, 0.0)));
        model.walls.push(wall("w-r", p(96.0, 0.0), p(96.0, 96.0)));
        assert_eq!(enclosed_room_count(&model), 0);

        model.walls.push(wall("w-t", p(96.0, 96.0), p(0.0, 96.0)));
        model.walls.push(wall("w-l", p(0.0, 96.0), p(0.0, 0.0)));
        assert_eq!(enclosed_room_count(&model), 1);

        model
            .walls
            .push(wall("w-part", p(48.0, 0.0), p(48.0, 96.0)));
        assert_eq!(enclosed_room_count(&model), 2);
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
    fn interior_wall_at_midspan_splits_into_two_rooms() {
        // A 12ft × 8ft rectangle plus one interior wall whose endpoints land on
        // the bottom and top walls' MID-SPANS (a Tee at each end). The through
        // walls must be split at those junctions for the room to divide in two.
        let mut model = rect_model(12.0, 8.0);
        model
            .walls
            .push(wall("interior", p(72.0, 0.0), p(72.0, 96.0)));

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
    fn level_wall_loop_outline_reports_concave_l_footprint_corner() {
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let (a, b) = (72.0, 144.0);
        let pts = [p(0.0, 0.0), p(b, 0.0), p(b, a), p(a, a), p(a, b), p(0.0, b)];
        for index in 0..pts.len() {
            let next = (index + 1) % pts.len();
            model
                .walls
                .push(wall(&format!("w-{index}"), pts[index], pts[next]));
        }

        let outline = level_wall_loop_outline(&model, &ElementId::new("level-1"))
            .expect("simple L perimeter loop");

        assert_eq!(outline.len(), 6);
        assert_eq!(concave_polygon_corners(&outline), vec![p(a, a)]);
    }

    #[test]
    fn level_wall_loop_outline_rejects_partitioned_wall_graph() {
        let mut model = rect_model(12.0, 8.0);
        model
            .walls
            .push(wall("interior", p(72.0, 0.0), p(72.0, 96.0)));

        assert!(level_wall_loop_outline(&model, &ElementId::new("level-1")).is_none());
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
    fn level_scoped_room_boundary_ignores_other_levels() {
        let mut model = rect_model(12.0, 8.0);
        let level_2 = ElementId::new("level-2");
        let seed = p(72.0, 48.0);

        assert!(
            room_boundary(&model, seed).is_some(),
            "the global helper still sees the level-1 rectangle"
        );
        assert!(
            room_boundary_on_level(&model, &level_2, seed).is_none(),
            "level-2 has no walls yet"
        );
        assert_eq!(enclosed_room_count_on_level(&model, &level_2), 0);

        model
            .walls
            .push(wall_on_level("l2-b", "level-2", p(0.0, 0.0), p(240.0, 0.0)));
        model.walls.push(wall_on_level(
            "l2-r",
            "level-2",
            p(240.0, 0.0),
            p(240.0, 168.0),
        ));
        model.walls.push(wall_on_level(
            "l2-t",
            "level-2",
            p(240.0, 168.0),
            p(0.0, 168.0),
        ));
        model
            .walls
            .push(wall_on_level("l2-l", "level-2", p(0.0, 168.0), p(0.0, 0.0)));

        let boundary = room_boundary_on_level(&model, &level_2, seed)
            .expect("seed is inside the level-2 rectangle");
        assert!((boundary.area_square_feet() - 280.0).abs() < 1e-6);
        assert_eq!(enclosed_room_count_on_level(&model, &level_2), 1);
    }

    #[test]
    fn level_scoped_room_boundaries_batch_matches_seeds_in_order() {
        let mut model = rect_model(12.0, 8.0);
        model.walls.push(wall_on_level(
            "l2-b",
            "level-2",
            p(360.0, 0.0),
            p(480.0, 0.0),
        ));
        model.walls.push(wall_on_level(
            "l2-r",
            "level-2",
            p(480.0, 0.0),
            p(480.0, 120.0),
        ));
        model.walls.push(wall_on_level(
            "l2-t",
            "level-2",
            p(480.0, 120.0),
            p(360.0, 120.0),
        ));
        model.walls.push(wall_on_level(
            "l2-l",
            "level-2",
            p(360.0, 120.0),
            p(360.0, 0.0),
        ));

        let results = room_boundaries_on_level(
            &model,
            &ElementId::new("level-2"),
            &[p(420.0, 60.0), p(72.0, 48.0)],
        );

        assert_eq!(results.len(), 2);
        assert!(
            results[0]
                .as_ref()
                .is_some_and(|b| (b.area_square_feet() - 100.0).abs() < 1e-6)
        );
        assert!(
            results[1].is_none(),
            "a seed inside only the level-1 loop must not resolve on level-2"
        );
    }

    #[test]
    fn room_boundaries_for_rooms_groups_by_room_level_and_preserves_order() {
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        model
            .levels
            .push(Level::new("level-2", "Level 2", Length::from_feet(10.0)));
        model.walls.clear();
        for (index, (a, b)) in [
            (p(0.0, 0.0), p(144.0, 0.0)),
            (p(144.0, 0.0), p(144.0, 96.0)),
            (p(144.0, 96.0), p(0.0, 96.0)),
            (p(0.0, 96.0), p(0.0, 0.0)),
            (p(240.0, 0.0), p(384.0, 0.0)),
            (p(384.0, 0.0), p(384.0, 96.0)),
            (p(384.0, 96.0), p(240.0, 96.0)),
            (p(240.0, 96.0), p(240.0, 0.0)),
        ]
        .into_iter()
        .enumerate()
        {
            model.walls.push(wall(&format!("w-{index}"), a, b));
        }
        model.rooms = vec![
            Room::new(
                "room-left",
                "Left",
                RoomUsage::Living,
                "level-1",
                p(72.0, 48.0),
            ),
            Room::new(
                "room-upper",
                "Upper",
                RoomUsage::Living,
                "level-2",
                p(72.0, 48.0),
            ),
            Room::new(
                "room-right",
                "Right",
                RoomUsage::Living,
                "level-1",
                p(312.0, 48.0),
            ),
        ];
        let rooms: Vec<&Room> = model.rooms.iter().collect();

        let boundaries = room_boundaries_for_rooms(&model, &rooms);

        assert_eq!(boundaries.len(), 3);
        assert!(
            boundaries[0]
                .as_ref()
                .is_some_and(|boundary| (boundary.area_square_feet() - 96.0).abs() < 1e-6)
        );
        assert!(
            boundaries[1].is_none(),
            "the level-2 room must not borrow level-1 walls"
        );
        assert!(
            boundaries[2]
                .as_ref()
                .is_some_and(|boundary| (boundary.area_square_feet() - 96.0).abs() < 1e-6)
        );
    }

    #[test]
    fn boundary_is_deterministic() {
        let model = rect_model(10.0, 10.0);
        let first = room_boundary(&model, p(60.0, 60.0)).unwrap();
        let second = room_boundary(&model, p(60.0, 60.0)).unwrap();
        assert_eq!(first.vertices, second.vertices);
    }

    #[test]
    fn wall_interior_sides_orients_perimeter_and_omits_partitions() {
        // The two-bedroom shell: a 24ft × 16ft CCW perimeter (front/right/back/
        // left) with two interior partitions (wall-mid, wall-bed). Every
        // perimeter wall must get a determinate side that places the room on its
        // plus-side (the renderers' interior face); the partitions divide two
        // rooms and are ambiguous, so they are omitted.
        let model = BuildingModel::demo_two_bedroom();
        let sides = wall_interior_sides(&model);

        for id in ["wall-front", "wall-right", "wall-back", "wall-left"] {
            assert_eq!(
                sides.get(&ElementId::new(id)),
                Some(&true),
                "perimeter wall {id} should put the room on its plus-side",
            );
        }
        assert!(!sides.contains_key(&ElementId::new("wall-mid")));
        assert!(!sides.contains_key(&ElementId::new("wall-bed")));

        // The reported plus-side must actually face the enclosed room: probe the
        // midpoint of `wall-front` (along +x) toward its plus-side normal (+y)
        // and confirm that lands inside a bounded face.
        let faces = bounded_faces(&model);
        let front = model
            .walls
            .iter()
            .find(|wall| wall.id == ElementId::new("wall-front"))
            .unwrap();
        let mid = p(
            (front.start.x + front.end.x).inches() / 2.0,
            (front.start.y + front.end.y).inches() / 2.0,
        );
        let toward_interior = p(mid.x.inches(), mid.y.inches() + 6.0);
        assert!(
            faces
                .iter()
                .any(|face| point_in_polygon(toward_interior, face))
        );
    }

    /// Every emitted triangle must lie inside the polygon (centroid test) and the
    /// triangles must exactly tile its area — the property a naive vertex-0 fan
    /// violates for a concave outline.
    fn assert_triangulation_tiles(poly: &[Point2], tris: &[[usize; 3]]) {
        let expected = polygon_area_square_inches(poly);
        let mut total = 0.0;
        for &[a, b, c] in tris {
            let (pa, pb, pc) = (poly[a], poly[b], poly[c]);
            let area = polygon_area_square_inches(&[pa, pb, pc]);
            assert!(area > 0.5, "degenerate triangle {a},{b},{c}");
            total += area;
            let cx = (pa.x.inches() + pb.x.inches() + pc.x.inches()) / 3.0;
            let cy = (pa.y.inches() + pb.y.inches() + pc.y.inches()) / 3.0;
            assert!(
                point_in_polygon(p(cx, cy), poly),
                "triangle [{a},{b},{c}] centroid lies outside the polygon (fan spill)"
            );
        }
        assert!(
            (total - expected).abs() < 1.0,
            "triangles cover {total} sq in, polygon is {expected} sq in"
        );
    }

    #[test]
    fn triangulates_convex_rectangle() {
        let rect = vec![p(0.0, 0.0), p(40.0, 0.0), p(40.0, 20.0), p(0.0, 20.0)];
        let tris = triangulate_simple_polygon(&rect);
        assert_eq!(tris.len(), 2);
        assert_triangulation_tiles(&rect, &tris);
    }

    #[test]
    fn triangulates_concave_l_shape_in_both_windings() {
        // An L: a 40×40 square with a 20×20 notch removed at the top-right corner.
        // Vertex 3 (20,20) is reflex, so a vertex-0 fan would spill outside.
        let ccw = vec![
            p(0.0, 0.0),
            p(40.0, 0.0),
            p(40.0, 20.0),
            p(20.0, 20.0),
            p(20.0, 40.0),
            p(0.0, 40.0),
        ];
        let cw: Vec<Point2> = ccw.iter().rev().copied().collect();
        for poly in [ccw, cw] {
            let tris = triangulate_simple_polygon(&poly);
            assert_eq!(
                tris.len(),
                poly.len() - 2,
                "a simple n-gon yields n-2 triangles"
            );
            assert_triangulation_tiles(&poly, &tris);
        }
    }

    #[test]
    fn triangulate_rejects_degenerate_vertex_counts() {
        assert!(triangulate_simple_polygon(&[]).is_empty());
        assert!(triangulate_simple_polygon(&[p(0.0, 0.0), p(1.0, 0.0)]).is_empty());
        assert_eq!(
            triangulate_simple_polygon(&[p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)]),
            vec![[0, 1, 2]]
        );
    }
}
