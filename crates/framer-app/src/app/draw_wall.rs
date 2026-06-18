//! Geometry helpers for the interactive draw-wall tool: ortho locking, grid
//! snapping, and snapping to existing wall endpoints. These are pure functions
//! over the authored model so they can be unit-tested without the egui layer.

use framer_core::{BuildingModel, ElementId, Length, Point2, Wall, WallJoin};

use super::model_edit::maybe_snap;

/// A draw point resolved from a raw cursor position, ready to commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResolvedPoint {
    pub(super) point: Point2,
    /// True when the point snapped onto an existing wall endpoint, meaning a
    /// join should be recorded at this location.
    pub(super) on_existing: bool,
}

/// Lock `raw_end` to an axis-aligned segment from `start` by keeping the
/// dominant-axis delta and collapsing the other onto `start`. The result always
/// shares either the x or y coordinate of `start` (a horizontal or vertical
/// wall). Ties favour horizontal.
pub(super) fn ortho_lock(start: Point2, raw_end: Point2) -> Point2 {
    let dx = (raw_end.x - start.x).abs();
    let dy = (raw_end.y - start.y).abs();
    if dx >= dy {
        Point2::new(raw_end.x, start.y)
    } else {
        Point2::new(start.x, raw_end.y)
    }
}

/// Round both axes of `point` to the nearest multiple of `step` (a no-op when
/// snapping is disabled).
pub(super) fn snap_to_grid(point: Point2, step: Option<Length>) -> Point2 {
    Point2::new(maybe_snap(point.x, step), maybe_snap(point.y, step))
}

/// The nearest existing wall endpoint within `tolerance` of `point`, if any.
pub(super) fn snap_to_endpoint(
    model: &BuildingModel,
    point: Point2,
    tolerance: Length,
) -> Option<Point2> {
    let tolerance_sq = squared(tolerance.ticks());
    let mut best: Option<(i64, Point2)> = None;
    for wall in &model.walls {
        for endpoint in [wall.start, wall.end] {
            let distance_sq = point_distance_sq(point, endpoint);
            if distance_sq <= tolerance_sq && best.is_none_or(|(best_sq, _)| distance_sq < best_sq)
            {
                best = Some((distance_sq, endpoint));
            }
        }
    }
    best.map(|(_, endpoint)| endpoint)
}

/// Resolve a raw cursor point into the point we would commit. Snapping to an
/// existing wall endpoint wins, because that is what forms joins — but only when
/// the result keeps the wall axis-aligned relative to `start`. Otherwise the
/// point is ortho-locked to `start` (for the second point onward) and
/// grid-snapped.
pub(super) fn resolve_draw_point(
    model: &BuildingModel,
    start: Option<Point2>,
    raw: Point2,
    step: Option<Length>,
    tolerance: Length,
) -> ResolvedPoint {
    if let Some(endpoint) = snap_to_endpoint(model, raw, tolerance) {
        let keeps_ortho = match start {
            Some(start) => endpoint.x == start.x || endpoint.y == start.y,
            None => true,
        };
        if keeps_ortho {
            return ResolvedPoint {
                point: endpoint,
                on_existing: true,
            };
        }
    }

    let locked = match start {
        Some(start) => ortho_lock(start, raw),
        None => raw,
    };
    ResolvedPoint {
        point: snap_to_grid(locked, step),
        on_existing: false,
    }
}

/// Ids of every wall that has `point` as one of its endpoints, in model order.
/// Used to record a corner join between a newly drawn wall and each wall it
/// meets at a shared endpoint.
pub(super) fn walls_sharing_endpoint(model: &BuildingModel, point: Point2) -> Vec<ElementId> {
    model
        .walls
        .iter()
        .filter(|wall| wall.start == point || wall.end == point)
        .map(|wall| wall.id.clone())
        .collect()
}

/// Build the corner joins for a freshly drawn `new_wall` (not yet pushed into
/// `model`): one `Corner` join to every existing wall that shares either of the
/// new wall's endpoints. Ids are unique against both the model and the joins
/// already produced in this call. Slice 1 records corners only; mid-span (Tee)
/// joins arrive with interior-wall framing.
pub(super) fn corner_joins_for_new_wall(model: &BuildingModel, new_wall: &Wall) -> Vec<WallJoin> {
    let mut joins: Vec<WallJoin> = Vec::new();
    // At most one join per other wall: a wall that shares both endpoints with the
    // new wall (a coincident segment) still meets it once.
    let mut joined: Vec<ElementId> = Vec::new();
    for endpoint in [new_wall.start, new_wall.end] {
        for other in walls_sharing_endpoint(model, endpoint) {
            if joined.contains(&other) {
                continue;
            }
            let id = next_join_id(model, &joins);
            let name = format!("{} \u{2013} {} corner", new_wall.id.0, other.0);
            joins.push(WallJoin::corner(
                id,
                name,
                new_wall.id.0.clone(),
                other.0.clone(),
                endpoint,
            ));
            joined.push(other);
        }
    }
    joins
}

/// The next free `join-N` id, unique against the model's existing joins and any
/// `extra` joins already staged in the current operation.
fn next_join_id(model: &BuildingModel, extra: &[WallJoin]) -> String {
    let mut index = model.wall_joins.len() + extra.len() + 1;
    loop {
        let id = format!("join-{index}");
        let collides = model
            .wall_joins
            .iter()
            .chain(extra)
            .any(|join| join.id.0 == id);
        if !collides {
            return id;
        }
        index += 1;
    }
}

fn point_distance_sq(a: Point2, b: Point2) -> i64 {
    squared((a.x - b.x).ticks()) + squared((a.y - b.y).ticks())
}

fn squared(value: i64) -> i64 {
    value * value
}

#[cfg(test)]
mod tests {
    use super::*;
    use framer_core::{BuildingModel, CodeProfile, ElementId, Length, Point2, Wall, WallJoinKind};

    fn p(x_in: f64, y_in: f64) -> Point2 {
        Point2::new(Length::from_inches(x_in), Length::from_inches(y_in))
    }

    fn wall_from(id: &str, start: Point2, end: Point2) -> Wall {
        let code = CodeProfile::irc_2021_prescriptive();
        Wall::new(id, id, Length::from_feet(10.0), &code).with_placement("level-1", start, end)
    }

    fn empty_model() -> BuildingModel {
        BuildingModel::new(CodeProfile::irc_2021_prescriptive())
    }

    #[test]
    fn ortho_lock_keeps_horizontal_when_x_dominates() {
        assert_eq!(ortho_lock(p(0.0, 0.0), p(100.0, 12.0)), p(100.0, 0.0));
    }

    #[test]
    fn ortho_lock_keeps_vertical_when_y_dominates() {
        assert_eq!(ortho_lock(p(0.0, 0.0), p(12.0, 100.0)), p(0.0, 100.0));
    }

    #[test]
    fn snap_to_grid_rounds_each_axis() {
        let snapped = snap_to_grid(p(14.0, 21.0), Some(Length::from_whole_inches(6)));
        assert_eq!(snapped, p(12.0, 24.0));
    }

    #[test]
    fn snap_to_endpoint_returns_nearest_within_tolerance() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        let got = snap_to_endpoint(&model, p(118.0, 0.0), Length::from_inches(6.0));

        assert_eq!(got, Some(p(120.0, 0.0)));
    }

    #[test]
    fn snap_to_endpoint_ignores_far_points() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        assert_eq!(
            snap_to_endpoint(&model, p(60.0, 60.0), Length::from_inches(6.0)),
            None
        );
    }

    #[test]
    fn resolve_first_point_grid_snaps() {
        let model = empty_model();
        let resolved = resolve_draw_point(
            &model,
            None,
            p(14.0, 21.0),
            Some(Length::from_whole_inches(6)),
            Length::from_inches(6.0),
        );

        assert_eq!(resolved.point, p(12.0, 24.0));
        assert!(!resolved.on_existing);
    }

    #[test]
    fn resolve_second_point_ortho_locks_and_grid_snaps() {
        let model = empty_model();
        let resolved = resolve_draw_point(
            &model,
            Some(p(0.0, 0.0)),
            p(98.0, 7.0),
            Some(Length::from_whole_inches(6)),
            Length::from_inches(2.0),
        );

        // X dominates -> horizontal; 98 in snaps to 96; y locks to the start's 0.
        assert_eq!(resolved.point, p(96.0, 0.0));
        assert!(!resolved.on_existing);
    }

    #[test]
    fn resolve_second_point_snaps_to_aligned_existing_endpoint() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        // Drawing from (120,0); the far endpoint (0,0) is aligned (shares y) and
        // close to the cursor, so it snaps and flags a join site.
        let resolved = resolve_draw_point(
            &model,
            Some(p(120.0, 0.0)),
            p(2.0, 1.0),
            Some(Length::from_whole_inches(6)),
            Length::from_inches(6.0),
        );

        assert_eq!(resolved.point, p(0.0, 0.0));
        assert!(resolved.on_existing);
    }

    #[test]
    fn resolve_rejects_diagonal_existing_endpoint() {
        let mut model = empty_model();
        // An existing endpoint at (120,120) that is diagonal to the start point.
        model
            .walls
            .push(wall_from("wall-1", p(120.0, 120.0), p(120.0, 200.0)));

        let resolved = resolve_draw_point(
            &model,
            Some(p(0.0, 0.0)),
            p(118.0, 119.0),
            Some(Length::from_whole_inches(6)),
            Length::from_inches(6.0),
        );

        // The diagonal endpoint would make a non-ortho wall, so it is rejected
        // and we fall back to ortho + grid (Y dominates -> vertical at x=0).
        assert!(!resolved.on_existing);
        assert_eq!(resolved.point, p(0.0, 120.0));
    }

    #[test]
    fn corner_joins_link_new_wall_to_each_shared_endpoint() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        // New wall shares the (120,0) endpoint with wall-1 and is otherwise free.
        let new_wall = wall_from("wall-2", p(120.0, 0.0), p(120.0, 96.0));
        let joins = corner_joins_for_new_wall(&model, &new_wall);

        assert_eq!(joins.len(), 1);
        let join = &joins[0];
        assert_eq!(join.kind, WallJoinKind::Corner);
        assert_eq!(join.point, p(120.0, 0.0));
        assert_eq!(join.first_wall, ElementId::new("wall-2"));
        assert_eq!(join.second_wall, ElementId::new("wall-1"));
        // The generated id does not collide with existing joins.
        assert!(
            model
                .wall_joins
                .iter()
                .all(|existing| existing.id != join.id)
        );
    }

    #[test]
    fn corner_joins_dedup_when_same_wall_at_both_endpoints() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        // A new wall coincident with wall-1 shares BOTH endpoints with it; that
        // is still a single meeting, so only one corner join should be recorded.
        let new_wall = wall_from("wall-2", p(0.0, 0.0), p(120.0, 0.0));
        let joins = corner_joins_for_new_wall(&model, &new_wall);

        assert_eq!(joins.len(), 1);
        assert_eq!(joins[0].second_wall, ElementId::new("wall-1"));
    }

    #[test]
    fn corner_joins_for_free_floating_wall_are_empty() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        let new_wall = wall_from("wall-2", p(0.0, 240.0), p(120.0, 240.0));

        assert!(corner_joins_for_new_wall(&model, &new_wall).is_empty());
    }

    #[test]
    fn walls_sharing_endpoint_lists_all_touching_walls() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));
        model
            .walls
            .push(wall_from("wall-2", p(120.0, 0.0), p(120.0, 96.0)));

        let touching = walls_sharing_endpoint(&model, p(120.0, 0.0));

        assert_eq!(
            touching,
            vec![ElementId::new("wall-1"), ElementId::new("wall-2")]
        );
    }
}
