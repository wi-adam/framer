//! Geometry helpers for the interactive draw-wall tool: ortho locking, grid
//! snapping, and snapping to existing wall endpoints. These are pure functions
//! over the authored model so they can be unit-tested without the egui layer.

use framer_core::{BuildingModel, ElementId, Length, Point2, Wall, WallJoin, WallJoinKind};

use super::model_edit::maybe_snap;

/// What kind of geometry a resolved snap landed on. Drives the on-screen
/// indicator and whether committing the point forms a wall join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SnapKind {
    Endpoint,
    Midpoint,
    /// Crossing of two alignment guides — the cursor is level with one source and
    /// in line with another.
    Intersection,
    OnWall,
    /// Cursor shares an axis with an existing endpoint/midpoint (extension guide).
    Alignment,
    Grid,
    Free,
}

/// Orientation of an inference guide. A `Vertical` guide holds a constant x
/// (`at`); a `Horizontal` guide holds a constant y.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuideAxis {
    Vertical,
    Horizontal,
}

/// A dashed inference line shown while snapping, anchored at a `source` point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Guide {
    pub(super) axis: GuideAxis,
    /// The constant coordinate of the guide (x for `Vertical`, y for `Horizontal`).
    pub(super) at: Length,
    /// The existing point the guide extends from (its tick is drawn there).
    pub(super) source: Point2,
}

/// At most one vertical (x) and one horizontal (y) guide are ever live at once.
pub(super) const NO_GUIDES: [Option<Guide>; 2] = [None, None];

/// The outcome of resolving a raw cursor point: the committed point, what it
/// snapped to, and any inference guides to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SnapResult {
    pub(super) point: Point2,
    pub(super) kind: SnapKind,
    pub(super) guides: [Option<Guide>; 2],
}

/// Everything [`resolve_snap`] needs to turn a raw cursor point into a snapped
/// one. Shared by the draw-wall tool and the wall-endpoint editor.
pub(super) struct SnapContext<'a> {
    pub(super) model: &'a BuildingModel,
    /// Raw cursor position in model coordinates.
    pub(super) raw: Point2,
    /// Ortho reference: the fixed point the segment is drawn/edited relative to.
    pub(super) anchor: Option<Point2>,
    /// Walls that must not be snap targets (e.g. the wall being edited).
    pub(super) exclude: &'a [ElementId],
    /// Radius (model units) within which a fresh snap is *acquired*.
    pub(super) tolerance: Length,
    /// Larger radius (model units) a held snap must leave before it *releases*.
    pub(super) release_tolerance: Length,
    pub(super) grid_step: Option<Length>,
    /// Alt held: skip all snapping and place freely (ortho + grid only).
    pub(super) suspend: bool,
    /// The snap held from the previous frame, for sticky hysteresis.
    pub(super) previous: Option<SnapResult>,
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

/// The nearest existing wall endpoint within `tolerance` of `point` that keeps
/// the wall axis-aligned relative to `start`. Candidates that would make a
/// diagonal wall are skipped so a farther ortho-compatible endpoint can win.
pub(super) fn snap_to_endpoint(
    model: &BuildingModel,
    start: Option<Point2>,
    point: Point2,
    tolerance: Length,
    exclude: &[ElementId],
) -> Option<Point2> {
    let tolerance_sq = squared(tolerance.ticks());
    let mut best: Option<(i64, Point2)> = None;
    for wall in &model.walls {
        if exclude.contains(&wall.id) {
            continue;
        }
        for endpoint in [wall.start, wall.end] {
            if !keeps_ortho(start, endpoint) {
                continue;
            }
            let distance_sq = point_distance_sq(point, endpoint);
            if distance_sq <= tolerance_sq && best.is_none_or(|(best_sq, _)| distance_sq < best_sq)
            {
                best = Some((distance_sq, endpoint));
            }
        }
    }
    best.map(|(_, endpoint)| endpoint)
}

/// Resolve a raw cursor point against the model, applying snapping in priority
/// order (endpoint → midpoint → mid-span → ortho/grid fallback), with sticky
/// hysteresis and an Alt-suspend escape hatch. Shared by drawing and editing.
pub(super) fn resolve_snap(ctx: &SnapContext) -> SnapResult {
    // Alt held: place freely, ignoring all geometry.
    if ctx.suspend {
        return free_result(ctx);
    }

    // Sticky: keep a genuine prior snap until the cursor leaves the release
    // radius, so the snap doesn't flicker between nearby candidates.
    if let Some(previous) = &ctx.previous
        && let Some(held) = hold_previous(ctx, previous)
    {
        return held;
    }

    if let Some(point) =
        snap_to_endpoint(ctx.model, ctx.anchor, ctx.raw, ctx.tolerance, ctx.exclude)
    {
        return SnapResult {
            point,
            kind: SnapKind::Endpoint,
            guides: NO_GUIDES,
        };
    }
    if let Some(point) =
        snap_to_midpoint(ctx.model, ctx.anchor, ctx.raw, ctx.tolerance, ctx.exclude)
    {
        return SnapResult {
            point,
            kind: SnapKind::Midpoint,
            guides: NO_GUIDES,
        };
    }
    if let Some(point) =
        snap_to_wall_line(ctx.model, ctx.anchor, ctx.raw, ctx.tolerance, ctx.exclude)
    {
        return SnapResult {
            point,
            kind: SnapKind::OnWall,
            guides: NO_GUIDES,
        };
    }
    if let Some(result) = snap_to_alignment(ctx) {
        return result;
    }

    free_result(ctx)
}

/// Apply sticky hysteresis to the previous frame's snap. A point lock (endpoint,
/// midpoint, on-wall, intersection) is re-emitted whole while the cursor stays
/// within the release radius. An `Alignment` only locks one axis, so it is
/// *re-projected*: the guide's axis stays pinned (released by that axis's
/// distance), while the free axis tracks the cursor. Returns `None` to release.
fn hold_previous(ctx: &SnapContext, previous: &SnapResult) -> Option<SnapResult> {
    let release = ctx.release_tolerance.ticks();
    match previous.kind {
        SnapKind::Endpoint | SnapKind::Midpoint | SnapKind::OnWall | SnapKind::Intersection => {
            (point_distance_sq(ctx.raw, previous.point) <= squared(release)).then_some(*previous)
        }
        SnapKind::Alignment => {
            let guide = previous.guides.iter().flatten().next()?;
            let base = match ctx.anchor {
                Some(anchor) => ortho_lock(anchor, ctx.raw),
                None => ctx.raw,
            };
            match guide.axis {
                GuideAxis::Vertical => {
                    ((base.x - guide.at).ticks().abs() <= release).then(|| SnapResult {
                        point: Point2::new(guide.at, base.y),
                        kind: SnapKind::Alignment,
                        guides: [Some(*guide), None],
                    })
                }
                GuideAxis::Horizontal => {
                    ((base.y - guide.at).ticks().abs() <= release).then(|| SnapResult {
                        point: Point2::new(base.x, guide.at),
                        kind: SnapKind::Alignment,
                        guides: [None, Some(*guide)],
                    })
                }
            }
        }
        SnapKind::Grid | SnapKind::Free => None,
    }
}

/// The ortho-locked, grid-snapped fallback when nothing snappable is in range.
fn free_result(ctx: &SnapContext) -> SnapResult {
    let locked = match ctx.anchor {
        Some(anchor) => ortho_lock(anchor, ctx.raw),
        None => ctx.raw,
    };
    let point = snap_to_grid(locked, ctx.grid_step);
    let kind = if ctx.grid_step.is_some() {
        SnapKind::Grid
    } else {
        SnapKind::Free
    };
    SnapResult {
        point,
        kind,
        guides: NO_GUIDES,
    }
}

/// Snap a *free* axis (one not pinned by ortho-lock) onto an existing
/// endpoint/midpoint's coordinate, surfacing an extension guide. When both axes
/// are free and each finds a source, the result is their `Intersection`.
fn snap_to_alignment(ctx: &SnapContext) -> Option<SnapResult> {
    let base = match ctx.anchor {
        Some(anchor) => ortho_lock(anchor, ctx.raw),
        None => ctx.raw,
    };
    // With an anchor the wall is ortho-locked: the axis equal to the anchor is
    // pinned, so only the *other* axis may snap to an alignment source.
    let (free_x, free_y) = match ctx.anchor {
        Some(anchor) => (base.y == anchor.y, base.x == anchor.x),
        None => (true, true),
    };

    let sources = alignment_sources(ctx);
    let vguide = free_x
        .then(|| nearest_source_on_axis(&sources, GuideAxis::Vertical, base.x, ctx.tolerance))
        .flatten();
    let hguide = free_y
        .then(|| nearest_source_on_axis(&sources, GuideAxis::Horizontal, base.y, ctx.tolerance))
        .flatten();

    match (vguide, hguide) {
        (Some(v), Some(h)) => Some(SnapResult {
            point: Point2::new(v.at, h.at),
            kind: SnapKind::Intersection,
            guides: [Some(v), Some(h)],
        }),
        (Some(v), None) => Some(SnapResult {
            point: Point2::new(v.at, base.y),
            kind: SnapKind::Alignment,
            guides: [Some(v), None],
        }),
        (None, Some(h)) => Some(SnapResult {
            point: Point2::new(base.x, h.at),
            kind: SnapKind::Alignment,
            guides: [None, Some(h)],
        }),
        (None, None) => None,
    }
}

/// Endpoints and midpoints of every non-excluded wall — the points the cursor can
/// align to. The anchor itself is skipped (aligning to it is redundant with
/// ortho-lock).
fn alignment_sources(ctx: &SnapContext) -> Vec<Point2> {
    let mut sources = Vec::new();
    for wall in &ctx.model.walls {
        if wall.start == wall.end || ctx.exclude.contains(&wall.id) {
            continue;
        }
        for point in [wall.start, wall.end, midpoint(wall.start, wall.end)] {
            if Some(point) == ctx.anchor {
                continue;
            }
            sources.push(point);
        }
    }
    sources
}

/// The nearest source whose coordinate on `axis` is within `tolerance` of
/// `target`, returned as a guide. `Vertical` compares x; `Horizontal` compares y.
fn nearest_source_on_axis(
    sources: &[Point2],
    axis: GuideAxis,
    target: Length,
    tolerance: Length,
) -> Option<Guide> {
    let tol = tolerance.ticks();
    let mut best: Option<(i64, Point2)> = None;
    for &source in sources {
        let coord = match axis {
            GuideAxis::Vertical => source.x,
            GuideAxis::Horizontal => source.y,
        };
        let distance = (coord - target).ticks().abs();
        if distance <= tol && best.is_none_or(|(best_d, _)| distance < best_d) {
            best = Some((distance, source));
        }
    }
    best.map(|(_, source)| {
        let at = match axis {
            GuideAxis::Vertical => source.x,
            GuideAxis::Horizontal => source.y,
        };
        Guide { axis, at, source }
    })
}

/// Whether `candidate` keeps the wall axis-aligned relative to `start` (shares an
/// axis), or is unconstrained when there is no start point yet.
fn keeps_ortho(start: Option<Point2>, candidate: Point2) -> bool {
    match start {
        Some(start) => candidate.x == start.x || candidate.y == start.y,
        None => true,
    }
}

/// The projection of `point` onto the nearest wall segment within `tolerance`,
/// but only when it lands on that wall's interior (endpoints are handled by
/// [`snap_to_endpoint`]). This is what lets a wall snap mid-span to form a Tee.
pub(super) fn snap_to_wall_line(
    model: &BuildingModel,
    start: Option<Point2>,
    point: Point2,
    tolerance: Length,
    exclude: &[ElementId],
) -> Option<Point2> {
    let tolerance_sq = squared(tolerance.ticks());
    let mut best: Option<(i64, Point2)> = None;
    for wall in &model.walls {
        if wall.start == wall.end || exclude.contains(&wall.id) {
            continue;
        }
        let projected = project_onto_segment(point, wall.start, wall.end);
        if !wall.point_on_interior(projected) || !keeps_ortho(start, projected) {
            continue;
        }
        let distance_sq = point_distance_sq(point, projected);
        if distance_sq <= tolerance_sq && best.is_none_or(|(best_sq, _)| distance_sq < best_sq) {
            best = Some((distance_sq, projected));
        }
    }
    best.map(|(_, projected)| projected)
}

/// The nearest existing wall midpoint within `tolerance` that keeps the wall
/// axis-aligned relative to `start`. A midpoint lies on the wall interior, so a
/// snap here forms a Tee just like a mid-span snap.
pub(super) fn snap_to_midpoint(
    model: &BuildingModel,
    start: Option<Point2>,
    point: Point2,
    tolerance: Length,
    exclude: &[ElementId],
) -> Option<Point2> {
    let tolerance_sq = squared(tolerance.ticks());
    let mut best: Option<(i64, Point2)> = None;
    for wall in &model.walls {
        if wall.start == wall.end || exclude.contains(&wall.id) {
            continue;
        }
        let mid = midpoint(wall.start, wall.end);
        if !keeps_ortho(start, mid) {
            continue;
        }
        let distance_sq = point_distance_sq(point, mid);
        if distance_sq <= tolerance_sq && best.is_none_or(|(best_sq, _)| distance_sq < best_sq) {
            best = Some((distance_sq, mid));
        }
    }
    best.map(|(_, mid)| mid)
}

/// Midpoint of `a`–`b`, rounded to whole ticks.
fn midpoint(a: Point2, b: Point2) -> Point2 {
    Point2::new(
        Length::from_ticks((a.x.ticks() + b.x.ticks()) / 2),
        Length::from_ticks((a.y.ticks() + b.y.ticks()) / 2),
    )
}

/// Closest point to `point` on the segment `a`–`b`, rounded to whole ticks.
fn project_onto_segment(point: Point2, a: Point2, b: Point2) -> Point2 {
    let edge_x = (b.x - a.x).ticks() as f64;
    let edge_y = (b.y - a.y).ticks() as f64;
    let len_sq = edge_x * edge_x + edge_y * edge_y;
    if len_sq == 0.0 {
        return a;
    }
    let offset_x = (point.x - a.x).ticks() as f64;
    let offset_y = (point.y - a.y).ticks() as f64;
    let t = ((offset_x * edge_x + offset_y * edge_y) / len_sq).clamp(0.0, 1.0);
    Point2::new(
        Length::from_ticks((a.x.ticks() as f64 + t * edge_x).round() as i64),
        Length::from_ticks((a.y.ticks() as f64 + t * edge_y).round() as i64),
    )
}

/// Ids of every wall that has `point` as one of its endpoints, in model order.
/// Used to record a corner join between a newly drawn wall and each wall it
/// meets at a shared endpoint.
pub(super) fn walls_sharing_endpoint(
    model: &BuildingModel,
    level: &ElementId,
    point: Point2,
) -> Vec<ElementId> {
    model
        .walls
        .iter()
        .filter(|wall| &wall.level == level && (wall.start == point || wall.end == point))
        .map(|wall| wall.id.clone())
        .collect()
}

/// Build the joins for a freshly drawn `new_wall` (not yet pushed into `model`):
/// a `Corner` join to every existing wall that shares one of the new wall's
/// endpoints, and a `Tee` join to every existing wall whose interior (mid-span)
/// the new wall's endpoint lands on. Ids are unique against the model and the
/// joins already produced in this call; at most one join per other wall.
pub(super) fn joins_for_new_wall(model: &BuildingModel, new_wall: &Wall) -> Vec<WallJoin> {
    let mut joins: Vec<WallJoin> = Vec::new();
    let mut joined: Vec<ElementId> = Vec::new();
    for endpoint in [new_wall.start, new_wall.end] {
        for other in walls_sharing_endpoint(model, &new_wall.level, endpoint) {
            if joined.contains(&other) {
                continue;
            }
            let id = next_join_id(model, &joins);
            joins.push(WallJoin::new(
                id,
                format!("{} \u{2013} {} corner", new_wall.id.0, other.0),
                WallJoinKind::Corner,
                new_wall.id.0.clone(),
                other.0.clone(),
                endpoint,
            ));
            joined.push(other);
        }
        for through in walls_with_interior_point(model, &new_wall.level, endpoint) {
            if joined.contains(&through) {
                continue;
            }
            let id = next_join_id(model, &joins);
            // first = through wall (owns the point mid-span), second = new partition.
            joins.push(WallJoin::new(
                id,
                format!("{} \u{2013} {} tee", new_wall.id.0, through.0),
                WallJoinKind::Tee,
                through.0.clone(),
                new_wall.id.0.clone(),
                endpoint,
            ));
            joined.push(through);
        }
    }
    joins
}

/// Ids of every wall whose interior (not an endpoint) contains `point` — the
/// through walls for a new wall meeting them mid-span.
fn walls_with_interior_point(
    model: &BuildingModel,
    level: &ElementId,
    point: Point2,
) -> Vec<ElementId> {
    model
        .walls
        .iter()
        .filter(|wall| &wall.level == level && wall.point_on_interior(point))
        .map(|wall| wall.id.clone())
        .collect()
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

        let got = snap_to_endpoint(&model, None, p(118.0, 0.0), Length::from_inches(6.0), &[]);

        assert_eq!(got, Some(p(120.0, 0.0)));
    }

    #[test]
    fn snap_to_endpoint_ignores_far_points() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        assert_eq!(
            snap_to_endpoint(&model, None, p(60.0, 60.0), Length::from_inches(6.0), &[]),
            None
        );
    }

    #[test]
    fn resolve_first_point_grid_snaps() {
        let model = empty_model();
        let mut context = ctx(&model, p(14.0, 21.0));
        context.grid_step = Some(Length::from_whole_inches(6));

        let result = resolve_snap(&context);

        assert_eq!(result.point, p(12.0, 24.0));
        assert!(!forms_join(result));
    }

    #[test]
    fn resolve_second_point_ortho_locks_and_grid_snaps() {
        let model = empty_model();
        let mut context = ctx(&model, p(98.0, 7.0));
        context.anchor = Some(p(0.0, 0.0));
        context.grid_step = Some(Length::from_whole_inches(6));
        context.tolerance = Length::from_inches(2.0);

        let result = resolve_snap(&context);

        // X dominates -> horizontal; 98in grid-snaps to 96; y locks to anchor's 0.
        assert_eq!(result.point, p(96.0, 0.0));
        assert!(!forms_join(result));
    }

    #[test]
    fn resolve_second_point_snaps_to_aligned_existing_endpoint() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        // Drawing from (120,0); the far endpoint (0,0) is aligned (shares y) and
        // close to the cursor, so it snaps and flags a join site.
        let mut context = ctx(&model, p(2.0, 1.0));
        context.anchor = Some(p(120.0, 0.0));
        context.grid_step = Some(Length::from_whole_inches(6));

        let result = resolve_snap(&context);

        assert_eq!(result.point, p(0.0, 0.0));
        assert!(forms_join(result));
    }

    #[test]
    fn resolve_rejects_diagonal_existing_endpoint() {
        let mut model = empty_model();
        // An existing endpoint at (120,120) that is diagonal to the anchor point.
        model
            .walls
            .push(wall_from("wall-1", p(120.0, 120.0), p(120.0, 200.0)));

        let mut context = ctx(&model, p(118.0, 119.0));
        context.anchor = Some(p(0.0, 0.0));
        context.grid_step = Some(Length::from_whole_inches(6));

        let result = resolve_snap(&context);

        // The diagonal endpoint would make a non-ortho wall, so it is rejected
        // and we fall back to ortho + grid (Y dominates -> vertical at x=0).
        assert!(!forms_join(result));
        assert_eq!(result.point, p(0.0, 120.0));
    }

    #[test]
    fn corner_joins_link_new_wall_to_each_shared_endpoint() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        // New wall shares the (120,0) endpoint with wall-1 and is otherwise free.
        let new_wall = wall_from("wall-2", p(120.0, 0.0), p(120.0, 96.0));
        let joins = joins_for_new_wall(&model, &new_wall);

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
        let joins = joins_for_new_wall(&model, &new_wall);

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

        assert!(joins_for_new_wall(&model, &new_wall).is_empty());
    }

    #[test]
    fn snap_to_wall_line_projects_onto_interior() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(0.0, 120.0)));

        // A cursor 3in off the vertical wall at y=60 snaps onto the wall line.
        let got = snap_to_wall_line(&model, None, p(3.0, 60.0), Length::from_inches(6.0), &[]);
        assert_eq!(got, Some(p(0.0, 60.0)));
    }

    #[test]
    fn snap_to_wall_line_ignores_endpoint_projection_and_far_points() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(0.0, 120.0)));

        // Projects to the wall's endpoint (0,120) -> excluded (interior only).
        assert_eq!(
            snap_to_wall_line(&model, None, p(3.0, 130.0), Length::from_inches(6.0), &[]),
            None
        );
        // Too far from the wall line.
        assert_eq!(
            snap_to_wall_line(&model, None, p(60.0, 60.0), Length::from_inches(6.0), &[]),
            None
        );
    }

    #[test]
    fn joins_for_new_wall_creates_tee_at_midspan() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("through", p(0.0, 0.0), p(240.0, 0.0)));

        // A partition whose start lands on the through wall's interior -> Tee.
        let new_wall = wall_from("partition", p(120.0, 0.0), p(120.0, 96.0));
        let joins = joins_for_new_wall(&model, &new_wall);

        assert_eq!(joins.len(), 1);
        assert_eq!(joins[0].kind, WallJoinKind::Tee);
        assert_eq!(joins[0].point, p(120.0, 0.0));
        // first = through wall, second = new partition (the solver relies on this).
        assert_eq!(joins[0].first_wall, ElementId::new("through"));
        assert_eq!(joins[0].second_wall, ElementId::new("partition"));
    }

    #[test]
    fn joins_for_new_wall_ignores_cross_level_midspan_tee() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("through", p(0.0, 0.0), p(240.0, 0.0)));

        let new_wall = wall_from("partition", p(120.0, 0.0), p(120.0, 96.0)).with_placement(
            "level-2",
            p(120.0, 0.0),
            p(120.0, 96.0),
        );

        assert!(joins_for_new_wall(&model, &new_wall).is_empty());
    }

    #[test]
    fn resolve_second_point_snaps_onto_wall_interior_as_tee() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("through", p(0.0, 0.0), p(240.0, 0.0)));

        // Drawing up from (120,96) toward the through wall; the cursor just shy of
        // it snaps onto the wall's interior at (120,0), keeping the wall vertical.
        // (120,0) is also the through wall's midpoint, so it resolves as a midpoint
        // snap — which is still on the interior and forms a Tee.
        let mut context = ctx(&model, p(120.0, 3.0));
        context.anchor = Some(p(120.0, 96.0));
        context.grid_step = Some(Length::from_whole_inches(6));

        let result = resolve_snap(&context);

        assert_eq!(result.point, p(120.0, 0.0));
        assert!(forms_join(result), "a mid-span snap flags a join site");
    }

    #[test]
    fn resolve_prefers_farther_ortho_wall_over_nearer_diagonal_one() {
        // A parallel wall whose nearest interior projection would be diagonal to
        // the anchor must not block snapping to a perpendicular wall that is ortho.
        let mut model = empty_model();
        // Perpendicular through wall the partition should Tee into.
        model
            .walls
            .push(wall_from("perp", p(120.0, 0.0), p(120.0, 240.0)));

        // Anchored at (0,60) drawing right; cursor near the perpendicular wall.
        let mut context = ctx(&model, p(118.0, 60.0));
        context.anchor = Some(p(0.0, 60.0));
        context.grid_step = Some(Length::from_whole_inches(6));

        let result = resolve_snap(&context);

        assert_eq!(result.point, p(120.0, 60.0));
        assert!(forms_join(result));
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
        model.walls.push(
            wall_from("wall-3", p(120.0, 0.0), p(240.0, 0.0)).with_placement(
                "level-2",
                p(120.0, 0.0),
                p(240.0, 0.0),
            ),
        );

        let touching = walls_sharing_endpoint(&model, &ElementId::new("level-1"), p(120.0, 0.0));

        assert_eq!(
            touching,
            vec![ElementId::new("wall-1"), ElementId::new("wall-2")]
        );
    }

    /// A snap that lands on existing wall geometry (endpoint, midpoint, or
    /// interior) — the kinds that would record a join when committed.
    fn forms_join(result: SnapResult) -> bool {
        matches!(
            result.kind,
            SnapKind::Endpoint | SnapKind::Midpoint | SnapKind::OnWall
        )
    }

    fn ctx<'a>(model: &'a BuildingModel, raw: Point2) -> SnapContext<'a> {
        SnapContext {
            model,
            raw,
            anchor: None,
            exclude: &[],
            tolerance: Length::from_inches(6.0),
            release_tolerance: Length::from_inches(10.0),
            grid_step: None,
            suspend: false,
            previous: None,
        }
    }

    #[test]
    fn resolve_snap_snaps_to_endpoint() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        let result = resolve_snap(&ctx(&model, p(118.0, 1.0)));

        assert_eq!(result.kind, SnapKind::Endpoint);
        assert_eq!(result.point, p(120.0, 0.0));
        assert!(forms_join(result));
    }

    #[test]
    fn resolve_snap_projects_onto_wall_interior() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(0.0, 120.0)));

        // (3,30) projects to (0,30): on the interior, not endpoint or midpoint.
        let result = resolve_snap(&ctx(&model, p(3.0, 30.0)));

        assert_eq!(result.kind, SnapKind::OnWall);
        assert_eq!(result.point, p(0.0, 30.0));
        assert!(forms_join(result));
    }

    #[test]
    fn resolve_snap_snaps_to_midpoint() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        // Near the midpoint (60,0), far from both endpoints.
        let result = resolve_snap(&ctx(&model, p(61.0, 2.0)));

        assert_eq!(result.kind, SnapKind::Midpoint);
        assert_eq!(result.point, p(60.0, 0.0));
    }

    #[test]
    fn resolve_snap_endpoint_beats_midpoint() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("short", p(0.0, 0.0), p(12.0, 0.0)));

        // (10,0) is 2in from endpoint (12,0) and 4in from midpoint (6,0).
        let result = resolve_snap(&ctx(&model, p(10.0, 0.0)));

        assert_eq!(result.kind, SnapKind::Endpoint);
        assert_eq!(result.point, p(12.0, 0.0));
    }

    #[test]
    fn resolve_snap_falls_back_to_ortho_grid_when_nothing_near() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        let mut context = ctx(&model, p(98.0, 7.0));
        context.anchor = Some(p(0.0, 0.0));
        context.grid_step = Some(Length::from_whole_inches(6));

        let result = resolve_snap(&context);

        // x dominates → horizontal (y locks to anchor's 0); 98in grid-snaps to 96.
        assert_eq!(result.point, p(96.0, 0.0));
        assert_eq!(result.kind, SnapKind::Grid);
    }

    #[test]
    fn resolve_snap_suspend_skips_all_snapping() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        let mut context = ctx(&model, p(118.0, 1.0));
        context.suspend = true;

        let result = resolve_snap(&context);

        // Would snap to endpoint (120,0); suspended → raw passes through untouched.
        assert_eq!(result.kind, SnapKind::Free);
        assert_eq!(result.point, p(118.0, 1.0));
        assert!(!forms_join(result));
    }

    #[test]
    fn resolve_snap_excludes_named_walls() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        let exclude = [ElementId::new("wall-1")];
        let mut context = ctx(&model, p(118.0, 1.0));
        context.exclude = &exclude;

        let result = resolve_snap(&context);

        // The only wall is excluded → nothing to snap to.
        assert_eq!(result.kind, SnapKind::Free);
        assert_eq!(result.point, p(118.0, 1.0));
    }

    #[test]
    fn resolve_snap_holds_previous_within_release_radius() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        // Last frame snapped to endpoint (120,0). Cursor at (112,3) is 8.5in away:
        // outside the 6in acquire radius but inside the 10in release radius.
        let mut context = ctx(&model, p(112.0, 3.0));
        context.previous = Some(SnapResult {
            point: p(120.0, 0.0),
            kind: SnapKind::Endpoint,
            guides: NO_GUIDES,
        });

        let result = resolve_snap(&context);

        // The held endpoint wins over the fresh on-wall snap a recompute would find.
        assert_eq!(result.kind, SnapKind::Endpoint);
        assert_eq!(result.point, p(120.0, 0.0));
    }

    #[test]
    fn resolve_snap_releases_previous_beyond_release_radius() {
        let mut model = empty_model();
        model
            .walls
            .push(wall_from("wall-1", p(0.0, 0.0), p(120.0, 0.0)));

        // Cursor at (108,9) is 15in from the held endpoint: beyond the 10in release.
        let mut context = ctx(&model, p(108.0, 9.0));
        context.previous = Some(SnapResult {
            point: p(120.0, 0.0),
            kind: SnapKind::Endpoint,
            guides: NO_GUIDES,
        });

        let result = resolve_snap(&context);

        // Released, and nothing fresh is within acquire range → free passthrough.
        assert_eq!(result.kind, SnapKind::Free);
        assert_eq!(result.point, p(108.0, 9.0));
    }

    #[test]
    fn resolve_snap_alignment_slides_along_held_guide() {
        let mut model = empty_model();
        // A wall whose endpoints sit at x=100 — the source for a vertical guide.
        model
            .walls
            .push(wall_from("v", p(100.0, 0.0), p(100.0, 40.0)));

        // Last frame aligned x→100 (vertical guide), at (100,30). The cursor slid
        // down to y=38 — still within the 2D release radius of the old point, so
        // the buggy "re-emit previous" path would freeze y at 30.
        let mut context = ctx(&model, p(101.0, 38.0));
        context.previous = Some(SnapResult {
            point: p(100.0, 30.0),
            kind: SnapKind::Alignment,
            guides: [
                Some(Guide {
                    axis: GuideAxis::Vertical,
                    at: Length::from_inches(100.0),
                    source: p(100.0, 0.0),
                }),
                None,
            ],
        });

        let result = resolve_snap(&context);

        // The guide stays locked on x; the free y axis tracks the cursor (no freeze).
        assert_eq!(result.kind, SnapKind::Alignment);
        assert_eq!(result.point, p(100.0, 38.0));
    }

    #[test]
    fn resolve_snap_aligns_free_axis_to_source_with_guide() {
        let mut model = empty_model();
        // A wall whose endpoints sit at y=50, far from where we're drawing.
        model
            .walls
            .push(wall_from("wall-1", p(100.0, 50.0), p(200.0, 50.0)));

        // Drawing a vertical wall up from (0,0); the cursor near (2,48) ortho-locks
        // to x=0, and its free y aligns to the existing endpoints' y=50.
        let mut context = ctx(&model, p(2.0, 48.0));
        context.anchor = Some(p(0.0, 0.0));

        let result = resolve_snap(&context);

        assert_eq!(result.kind, SnapKind::Alignment);
        assert_eq!(result.point, p(0.0, 50.0));
        assert!(result.guides.iter().flatten().any(|guide| {
            guide.axis == GuideAxis::Horizontal && guide.at == Length::from_inches(50.0)
        }));
    }

    #[test]
    fn resolve_snap_intersects_two_guides() {
        let mut model = empty_model();
        // Source A contributes a vertical guide at x=100; source B a horizontal
        // guide at y=80.
        model
            .walls
            .push(wall_from("a", p(100.0, 0.0), p(100.0, 40.0)));
        model
            .walls
            .push(wall_from("b", p(0.0, 80.0), p(40.0, 80.0)));

        // Unanchored cursor near (98,82) aligns x→100 and y→80 simultaneously.
        let result = resolve_snap(&ctx(&model, p(98.0, 82.0)));

        assert_eq!(result.kind, SnapKind::Intersection);
        assert_eq!(result.point, p(100.0, 80.0));
        let live: Vec<_> = result.guides.iter().flatten().collect();
        assert_eq!(live.len(), 2);
    }

    #[test]
    fn resolve_snap_endpoint_beats_alignment() {
        let mut model = empty_model();
        // An endpoint right under the cursor, plus another wall sharing its y.
        model
            .walls
            .push(wall_from("near", p(10.0, 10.0), p(10.0, 60.0)));
        model
            .walls
            .push(wall_from("aligned", p(80.0, 10.0), p(140.0, 10.0)));

        // (12,11) is within acquire of endpoint (10,10); alignment must not win.
        let result = resolve_snap(&ctx(&model, p(12.0, 11.0)));

        assert_eq!(result.kind, SnapKind::Endpoint);
        assert_eq!(result.point, p(10.0, 10.0));
    }
}
