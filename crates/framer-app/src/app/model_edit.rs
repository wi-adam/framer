use framer_core::{BuildingModel, CodeProfile, Length, Opening, Point2, Wall, WallEnd};

const OPENING_MIN_SIZE: Length = Length::from_whole_inches(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpeningEditHandle {
    Move,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl OpeningEditHandle {
    pub(super) fn resizes_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    pub(super) fn resizes_right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    pub(super) fn resizes_top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    pub(super) fn resizes_bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OpeningGeometry {
    center: Length,
    width: Length,
    height: Length,
    sill_height: Length,
}

impl OpeningGeometry {
    pub(super) fn from_opening(opening: &Opening) -> Self {
        Self {
            center: opening.center,
            width: opening.width,
            height: opening.height,
            sill_height: opening.sill_height,
        }
    }

    fn left(self) -> Length {
        self.center - self.width / 2
    }

    fn right(self) -> Length {
        self.center + self.width / 2
    }

    fn top(self) -> Length {
        self.sill_height + self.height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpeningDragState {
    pub(super) wall_index: usize,
    pub(super) opening_id: String,
    pub(super) handle: OpeningEditHandle,
    pub(super) start: OpeningGeometry,
}

impl OpeningDragState {
    pub(super) fn new(
        wall_index: usize,
        opening_id: String,
        handle: OpeningEditHandle,
        opening: &Opening,
    ) -> Self {
        Self {
            wall_index,
            opening_id,
            handle,
            start: OpeningGeometry::from_opening(opening),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OpeningDragConstraints {
    edge_clearance: Length,
    top_clearance: Length,
    snap_step: Option<Length>,
    ortho: bool,
}

impl OpeningDragConstraints {
    pub(super) fn from_code(code: &CodeProfile) -> Self {
        Self {
            edge_clearance: code.stud_profile.thickness() * 2,
            top_clearance: opening_top_clearance(code),
            snap_step: None,
            ortho: false,
        }
    }

    /// Apply the active drafting modifiers (snap increment and ortho axis-lock).
    pub(super) fn with_modifiers(mut self, snap_step: Option<Length>, ortho: bool) -> Self {
        self.snap_step = snap_step;
        self.ortho = ortho;
        self
    }
}

/// Round a length to the nearest multiple of `step` when snapping is enabled.
pub(super) fn maybe_snap(value: Length, step: Option<Length>) -> Length {
    match step {
        Some(step) if step > Length::ZERO => {
            let step_ticks = step.ticks();
            let rounded = ((value.ticks() as f64 / step_ticks as f64).round() as i64) * step_ticks;
            Length::from_ticks(rounded)
        }
        _ => value,
    }
}

/// Which draggable handle on a selected wall is being manipulated in the plan:
/// an endpoint (extend / move the corner) or the body (translate the whole wall).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WallEditHandle {
    Start,
    End,
    Body,
}

impl WallEditHandle {
    /// The wall endpoint this handle moves, or `None` for the body (whole-wall)
    /// handle.
    pub(super) fn as_wall_end(self) -> Option<WallEnd> {
        match self {
            Self::Start => Some(WallEnd::Start),
            Self::End => Some(WallEnd::End),
            Self::Body => None,
        }
    }
}

/// In-progress drag of a wall handle in the plan view. The whole gesture is one
/// coalesced undo step (mirrors [`OpeningDragState`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WallDragState {
    pub(super) wall_index: usize,
    pub(super) handle: WallEditHandle,
    /// Body translate only: the total perpendicular model delta applied so far
    /// (from drag start), so the wall tracks the cursor absolutely and recovers
    /// after a clamped frame instead of lagging behind.
    pub(super) applied: (Length, Length),
}

/// Whether moving every wall endpoint currently at `old_point` to `new_point`
/// keeps each affected wall axis-aligned. The plan editor uses this to clamp a
/// drag that would skew a perpendicular neighbour off-axis (the model forbids
/// non-orthogonal walls; such a corner is repositioned by moving a whole wall).
pub(super) fn endpoint_move_keeps_ortho(
    model: &BuildingModel,
    old_point: Point2,
    new_point: Point2,
) -> bool {
    model.walls.iter().all(|wall| {
        let other = if wall.start == old_point {
            wall.end
        } else if wall.end == old_point {
            wall.start
        } else {
            return true;
        };
        // A degenerate wall whose both ends sit on the node imposes no constraint.
        other == old_point || new_point.x == other.x || new_point.y == other.y
    })
}

/// Whether moving every endpoint at `old_point` to `new_point` keeps each
/// affected wall non-degenerate — i.e. its moved end never lands on its fixed
/// end (which would collapse it to zero length and fail `validate`).
pub(super) fn endpoint_move_keeps_positive_length(
    model: &BuildingModel,
    old_point: Point2,
    new_point: Point2,
) -> bool {
    model.walls.iter().all(|wall| {
        let other = if wall.start == old_point {
            wall.end
        } else if wall.end == old_point {
            wall.start
        } else {
            return true;
        };
        other != new_point
    })
}

/// Whether translating wall `moving` by `(dx, dy)` keeps every *other* wall
/// axis-aligned. A neighbour skews only if exactly one of its ends rides along
/// (an end coincident with one of the moving wall's ends), so its moved end no
/// longer shares an axis with its fixed end.
pub(super) fn translate_keeps_ortho(
    model: &BuildingModel,
    moving: &framer_core::ElementId,
    start: Point2,
    end: Point2,
    dx: Length,
    dy: Length,
) -> bool {
    let rides = |point: Point2| point == start || point == end;
    model.walls.iter().all(|wall| {
        if wall.id == *moving {
            return true;
        }
        match (rides(wall.start), rides(wall.end)) {
            // Untouched, or both ends ride (translates rigidly): stays aligned.
            (false, false) | (true, true) => true,
            (true, false) => {
                let moved = Point2::new(wall.start.x + dx, wall.start.y + dy);
                moved.x == wall.end.x || moved.y == wall.end.y
            }
            (false, true) => {
                let moved = Point2::new(wall.end.x + dx, wall.end.y + dy);
                wall.start.x == moved.x || wall.start.y == moved.y
            }
        }
    })
}

/// Whether translating wall `moving` by `(dx, dy)` keeps every riding neighbour
/// non-degenerate — its moved end must not land on its fixed end. The moving
/// wall itself translates rigidly, so its length is unchanged.
pub(super) fn translate_keeps_positive_length(
    model: &BuildingModel,
    moving: &framer_core::ElementId,
    start: Point2,
    end: Point2,
    dx: Length,
    dy: Length,
) -> bool {
    let rides = |point: Point2| point == start || point == end;
    model.walls.iter().all(|wall| {
        if wall.id == *moving {
            return true;
        }
        match (rides(wall.start), rides(wall.end)) {
            (true, false) => Point2::new(wall.start.x + dx, wall.start.y + dy) != wall.end,
            (false, true) => Point2::new(wall.end.x + dx, wall.end.y + dy) != wall.start,
            (false, false) | (true, true) => true,
        }
    })
}

pub(super) fn set_wall_length_keep_direction(wall: &mut Wall, length: Length) {
    wall.length = length;
    if wall.start.y == wall.end.y {
        let direction: i64 = if wall.end.x >= wall.start.x { 1 } else { -1 };
        wall.end.x = wall.start.x + length * direction;
    } else if wall.start.x == wall.end.x {
        let direction: i64 = if wall.end.y >= wall.start.y { 1 } else { -1 };
        wall.end.y = wall.start.y + length * direction;
    } else {
        wall.end = Point2::new(wall.start.x + length, wall.start.y);
    }
}

pub(super) fn apply_opening_drag(
    wall: &mut Wall,
    opening_id: &str,
    handle: OpeningEditHandle,
    start: OpeningGeometry,
    delta_x: Length,
    delta_y: Length,
    constraints: OpeningDragConstraints,
) -> bool {
    let (min_x, max_x) =
        opening_drag_horizontal_limits(wall, opening_id, start, constraints.edge_clearance);
    let usable_width = (max_x - min_x).max(Length::from_whole_inches(1));
    let min_width = OPENING_MIN_SIZE.min(usable_width);
    let max_top = opening_max_top(wall.height, constraints.top_clearance);
    let min_height = OPENING_MIN_SIZE.min(max_top.max(Length::from_whole_inches(1)));

    let Some(opening) = wall
        .openings
        .iter_mut()
        .find(|opening| opening.id.0 == opening_id)
    else {
        return false;
    };

    let mut left = start.left();
    let mut right = start.right();
    let mut bottom = start.sill_height;
    let mut top = start.top().min(max_top);

    if matches!(handle, OpeningEditHandle::Move) {
        let (delta_x, delta_y) = if constraints.ortho {
            if delta_x.abs() >= delta_y.abs() {
                (delta_x, Length::ZERO)
            } else {
                (Length::ZERO, delta_y)
            }
        } else {
            (delta_x, delta_y)
        };
        let half_width = start.width.min(usable_width) / 2;
        let center = clamp_length(
            maybe_snap(start.center + delta_x, constraints.snap_step),
            min_x + half_width,
            max_x - half_width,
        );
        let max_bottom = (max_top - start.height).max(Length::ZERO);
        bottom = clamp_length(
            maybe_snap(start.sill_height + delta_y, constraints.snap_step),
            Length::ZERO,
            max_bottom,
        );
        left = center - start.width / 2;
        right = center + start.width / 2;
        top = bottom + start.height;
    } else {
        if handle.resizes_left() {
            left = clamp_length(
                maybe_snap(start.left() + delta_x, constraints.snap_step),
                min_x,
                start.right() - min_width,
            );
        }
        if handle.resizes_right() {
            right = clamp_length(
                maybe_snap(start.right() + delta_x, constraints.snap_step),
                left + min_width,
                max_x,
            );
        }
        if handle.resizes_bottom() {
            bottom = clamp_length(
                maybe_snap(start.sill_height + delta_y, constraints.snap_step),
                Length::ZERO,
                start.top() - min_height,
            );
        }
        if handle.resizes_top() {
            top = clamp_length(
                maybe_snap(start.top() + delta_y, constraints.snap_step),
                bottom + min_height,
                max_top,
            );
        }
    }

    let center = (left + right) / 2;
    let width = right - left;
    let height = top - bottom;
    let changed = opening.center != center
        || opening.width != width
        || opening.height != height
        || opening.sill_height != bottom;

    opening.center = center;
    opening.width = width;
    opening.height = height;
    opening.sill_height = bottom;
    changed
}

pub(super) fn opening_top_clearance(code: &CodeProfile) -> Length {
    let top_plate_count = if code.double_top_plate { 2 } else { 1 };
    code.plate_profile.thickness() * top_plate_count + Length::from_ticks(1)
}

pub(super) fn opening_max_bottom(
    wall_height: Length,
    opening_height: Length,
    top_clearance: Length,
) -> Length {
    (opening_max_top(wall_height, top_clearance) - opening_height).max(Length::ZERO)
}

fn opening_max_top(wall_height: Length, top_clearance: Length) -> Length {
    (wall_height - top_clearance).max(Length::from_whole_inches(1))
}

fn opening_drag_horizontal_limits(
    wall: &Wall,
    opening_id: &str,
    start: OpeningGeometry,
    edge_clearance: Length,
) -> (Length, Length) {
    let mut min_x = edge_clearance.min(wall.length / 2);
    let mut max_x = (wall.length - min_x).max(min_x);
    let start_left = start.left();
    let start_right = start.right();

    for opening in &wall.openings {
        if opening.id.0 == opening_id {
            continue;
        }

        if opening.right() <= start_left {
            min_x = min_x.max(opening.right());
        } else if opening.left() >= start_right {
            max_x = max_x.min(opening.left());
        }
    }

    (min_x, max_x)
}

/// Generate the next free `wall-N` id, unique across every wall in the model.
/// Wall ids must be globally unique (unlike per-wall `next_opening_id`), so this
/// scopes its uniqueness check to the whole `BuildingModel`.
pub(super) fn next_wall_id(model: &BuildingModel) -> (String, usize) {
    let mut index = model.walls.len() + 1;
    loop {
        let id = format!("wall-{index}");
        if model.walls.iter().all(|wall| wall.id.0 != id) {
            return (id, index);
        }
        index += 1;
    }
}

/// Generate the next free `system-N` id, unique across every construction system
/// in the model. System ids must be globally unique (walls reference them by id).
pub(super) fn next_system_id(model: &BuildingModel) -> (String, usize) {
    let mut index = model.systems.len() + 1;
    loop {
        let id = format!("system-{index}");
        if model.systems.iter().all(|system| system.id.0 != id) {
            return (id, index);
        }
        index += 1;
    }
}

/// Generate the next free `mat-N` id, unique across every material in the model.
/// Material ids must be globally unique (layers reference them by id).
pub(super) fn next_material_id(model: &BuildingModel) -> (String, usize) {
    let mut index = model.materials.len() + 1;
    loop {
        let id = format!("mat-{index}");
        if model.materials.iter().all(|material| material.id.0 != id) {
            return (id, index);
        }
        index += 1;
    }
}

/// Generate the next free `room-N` id, unique across every room in the model.
pub(super) fn next_room_id(model: &BuildingModel) -> (String, usize) {
    let mut index = model.rooms.len() + 1;
    loop {
        let id = format!("room-{index}");
        if model.rooms.iter().all(|room| room.id.0 != id) {
            return (id, index);
        }
        index += 1;
    }
}

pub(super) fn next_opening_id(wall: &Wall, prefix: &str) -> (String, usize) {
    let mut index = wall.openings.len() + 1;
    loop {
        let id = format!("{prefix}-{index}");
        if wall.openings.iter().all(|opening| opening.id.0 != id) {
            return (id, index);
        }
        index += 1;
    }
}

pub(super) fn next_dimension_id(wall: &Wall) -> (String, usize) {
    let mut index = wall.dimensions.len() + 1;
    loop {
        let id = format!("dimension-{index}");
        if wall.dimensions.iter().all(|dimension| dimension.id.0 != id) {
            return (id, index);
        }
        index += 1;
    }
}

fn clamp_length(value: Length, min: Length, max: Length) -> Length {
    if min > max {
        (min + max) / 2
    } else {
        value.max(min).min(max)
    }
}

#[cfg(test)]
mod tests {
    use framer_core::{CodeProfile, Opening};

    use super::*;

    #[test]
    fn opening_drag_moves_center_and_bottom_in_two_axes() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_inches(72.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));
        let start = OpeningGeometry::from_opening(&wall.openings[0]);

        assert!(apply_opening_drag(
            &mut wall,
            "door",
            OpeningEditHandle::Move,
            start,
            Length::from_inches(18.0),
            Length::from_inches(8.0),
            OpeningDragConstraints::from_code(&code),
        ));

        assert_eq!(wall.openings[0].center, Length::from_inches(90.0));
        assert_eq!(wall.openings[0].sill_height, Length::from_inches(8.0));
    }

    #[test]
    fn opening_drag_resizes_from_corner_without_moving_opposite_corner() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_inches(72.0),
            Length::from_inches(36.0),
            Length::from_inches(42.0),
            Length::from_inches(36.0),
        ));
        let start = OpeningGeometry::from_opening(&wall.openings[0]);

        assert!(apply_opening_drag(
            &mut wall,
            "window",
            OpeningEditHandle::TopRight,
            start,
            Length::from_inches(12.0),
            Length::from_inches(6.0),
            OpeningDragConstraints::from_code(&code),
        ));

        let opening = &wall.openings[0];
        assert_eq!(opening.left(), start.left());
        assert_eq!(opening.sill_height, Length::from_inches(36.0));
        assert_eq!(opening.width, Length::from_inches(48.0));
        assert_eq!(opening.height, Length::from_inches(48.0));
    }

    #[test]
    fn opening_drag_clamps_to_wall_bounds() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_inches(72.0),
            Length::from_inches(36.0),
            Length::from_inches(42.0),
            Length::from_inches(36.0),
        ));
        let start = OpeningGeometry::from_opening(&wall.openings[0]);

        assert!(apply_opening_drag(
            &mut wall,
            "window",
            OpeningEditHandle::Move,
            start,
            Length::from_inches(-200.0),
            Length::from_inches(200.0),
            OpeningDragConstraints::from_code(&code),
        ));

        let opening = &wall.openings[0];
        assert_eq!(opening.left(), Length::from_inches(3.0));
        assert_eq!(
            opening.top(),
            opening_max_top(wall.height, opening_top_clearance(&code))
        );
    }

    #[test]
    fn opening_drag_stops_below_top_plates_for_header_space() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_inches(72.0),
            Length::from_inches(36.0),
            Length::from_inches(48.0),
            Length::from_inches(36.0),
        ));
        let start = OpeningGeometry::from_opening(&wall.openings[0]);
        let top_clearance = opening_top_clearance(&code);

        assert!(apply_opening_drag(
            &mut wall,
            "window",
            OpeningEditHandle::Move,
            start,
            Length::ZERO,
            Length::from_inches(200.0),
            OpeningDragConstraints::from_code(&code),
        ));

        assert_eq!(
            wall.openings[0].top(),
            opening_max_top(wall.height, top_clearance)
        );
    }

    #[test]
    fn opening_drag_stops_before_neighbor_opening() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "left-window",
            "Left window",
            Length::from_inches(48.0),
            Length::from_inches(24.0),
            Length::from_inches(42.0),
            Length::from_inches(36.0),
        ));
        wall.openings.push(Opening::window(
            "right-window",
            "Right window",
            Length::from_inches(96.0),
            Length::from_inches(24.0),
            Length::from_inches(42.0),
            Length::from_inches(36.0),
        ));
        let start = OpeningGeometry::from_opening(&wall.openings[0]);

        assert!(apply_opening_drag(
            &mut wall,
            "left-window",
            OpeningEditHandle::Move,
            start,
            Length::from_inches(100.0),
            Length::ZERO,
            OpeningDragConstraints::from_code(&code),
        ));

        assert_eq!(wall.openings[0].right(), wall.openings[1].left());
    }

    #[test]
    fn snap_rounds_move_to_step() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(20.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_inches(72.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));
        let start = OpeningGeometry::from_opening(&wall.openings[0]);
        let constraints = OpeningDragConstraints::from_code(&code)
            .with_modifiers(Some(Length::from_whole_inches(1)), false);

        assert!(apply_opening_drag(
            &mut wall,
            "door",
            OpeningEditHandle::Move,
            start,
            Length::from_inches(17.3),
            Length::ZERO,
            constraints,
        ));

        // 72 + 17.3125 = 89.3125 in, snapped to the nearest inch.
        assert_eq!(wall.openings[0].center, Length::from_whole_inches(89));
    }

    #[test]
    fn next_room_id_is_globally_unique() {
        use framer_core::{Point2, Room, RoomUsage};
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = framer_core::BuildingModel::new(code);
        model.rooms.push(Room::new(
            "room-1",
            "One",
            RoomUsage::Unspecified,
            "level-1",
            Point2::new(Length::from_feet(1.0), Length::from_feet(1.0)),
        ));

        let (id, index) = next_room_id(&model);

        assert!(model.rooms.iter().all(|room| room.id.0 != id));
        assert_eq!(id, format!("room-{index}"));
    }

    #[test]
    fn next_wall_id_is_globally_unique() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = framer_core::BuildingModel::new(code.clone());
        model
            .walls
            .push(Wall::new("wall-1", "One", Length::from_feet(8.0), &code));
        model
            .walls
            .push(Wall::new("wall-3", "Three", Length::from_feet(8.0), &code));

        let (id, index) = next_wall_id(&model);

        assert!(model.walls.iter().all(|wall| wall.id.0 != id));
        assert_eq!(id, format!("wall-{index}"));
    }

    fn ftp(x: f64, y: f64) -> Point2 {
        Point2::new(Length::from_feet(x), Length::from_feet(y))
    }

    fn placed(id: &str, start: Point2, end: Point2, code: &CodeProfile) -> Wall {
        Wall::new(id, id, Length::from_feet(1.0), code).with_placement("level-1", start, end)
    }

    #[test]
    fn endpoint_move_clamps_perpendicular_neighbour_at_a_corner() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = framer_core::BuildingModel::new(code.clone());
        // L-corner at (10,0): horizontal `a` meets vertical `b`.
        model
            .walls
            .push(placed("a", ftp(0.0, 0.0), ftp(10.0, 0.0), &code));
        model
            .walls
            .push(placed("b", ftp(10.0, 0.0), ftp(10.0, 8.0), &code));
        let node = ftp(10.0, 0.0);

        // Moving in X keeps `a` horizontal but skews `b`; moving in Y skews `a`.
        assert!(!endpoint_move_keeps_ortho(&model, node, ftp(12.0, 0.0)));
        assert!(!endpoint_move_keeps_ortho(&model, node, ftp(10.0, 3.0)));
    }

    #[test]
    fn endpoint_move_allows_collinear_and_free_ends() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = framer_core::BuildingModel::new(code.clone());
        // Collinear run a—b sharing the node at (10,0).
        model
            .walls
            .push(placed("a", ftp(0.0, 0.0), ftp(10.0, 0.0), &code));
        model
            .walls
            .push(placed("b", ftp(10.0, 0.0), ftp(20.0, 0.0), &code));
        let node = ftp(10.0, 0.0);

        // Sliding the node along the shared row keeps both horizontal.
        assert!(endpoint_move_keeps_ortho(&model, node, ftp(12.0, 0.0)));
        // Moving it off-row would skew both.
        assert!(!endpoint_move_keeps_ortho(&model, node, ftp(12.0, 4.0)));
    }

    #[test]
    fn translate_keeps_ortho_allows_perpendicular_clamps_along_axis() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = framer_core::BuildingModel::new(code.clone());
        model
            .walls
            .push(placed("a", ftp(0.0, 0.0), ftp(10.0, 0.0), &code));
        model
            .walls
            .push(placed("b", ftp(0.0, 0.0), ftp(0.0, 8.0), &code));
        let a = framer_core::ElementId::new("a");

        // Sliding horizontal `a` perpendicular (in Y) keeps vertical `b` ortho.
        assert!(translate_keeps_ortho(
            &model,
            &a,
            ftp(0.0, 0.0),
            ftp(10.0, 0.0),
            Length::ZERO,
            Length::from_feet(2.0),
        ));
        // Sliding `a` along its own axis (in X) would skew `b`.
        assert!(!translate_keeps_ortho(
            &model,
            &a,
            ftp(0.0, 0.0),
            ftp(10.0, 0.0),
            Length::from_feet(2.0),
            Length::ZERO,
        ));
    }

    #[test]
    fn endpoint_move_rejects_collapsing_a_wall_onto_its_fixed_end() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = framer_core::BuildingModel::new(code.clone());
        model
            .walls
            .push(placed("a", ftp(0.0, 0.0), ftp(10.0, 0.0), &code));

        // Dragging the end (10,0) back onto the fixed start (0,0) would collapse it.
        assert!(!endpoint_move_keeps_positive_length(
            &model,
            ftp(10.0, 0.0),
            ftp(0.0, 0.0)
        ));
        // A normal extend keeps positive length.
        assert!(endpoint_move_keeps_positive_length(
            &model,
            ftp(10.0, 0.0),
            ftp(8.0, 0.0)
        ));
    }

    #[test]
    fn translate_rejects_collapsing_a_neighbour() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = framer_core::BuildingModel::new(code.clone());
        // `a` horizontal at y=0; `b` vertical from the shared corner up to (0,8).
        model
            .walls
            .push(placed("a", ftp(0.0, 0.0), ftp(10.0, 0.0), &code));
        model
            .walls
            .push(placed("b", ftp(0.0, 0.0), ftp(0.0, 8.0), &code));
        let a = framer_core::ElementId::new("a");

        // Sliding `a` up by 8ft drives `b`'s bottom onto its top (0,8) → collapse.
        assert!(!translate_keeps_positive_length(
            &model,
            &a,
            ftp(0.0, 0.0),
            ftp(10.0, 0.0),
            Length::ZERO,
            Length::from_feet(8.0),
        ));
        // A smaller slide keeps `b` non-degenerate.
        assert!(translate_keeps_positive_length(
            &model,
            &a,
            ftp(0.0, 0.0),
            ftp(10.0, 0.0),
            Length::ZERO,
            Length::from_feet(2.0),
        ));
    }

    #[test]
    fn ortho_locks_move_to_dominant_axis() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(20.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_inches(72.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));
        let start = OpeningGeometry::from_opening(&wall.openings[0]);
        let constraints = OpeningDragConstraints::from_code(&code).with_modifiers(None, true);

        assert!(apply_opening_drag(
            &mut wall,
            "door",
            OpeningEditHandle::Move,
            start,
            Length::from_inches(18.0),
            Length::from_inches(8.0),
            constraints,
        ));

        // X dominates, so the vertical delta is dropped.
        assert_eq!(wall.openings[0].center, Length::from_inches(90.0));
        assert_eq!(wall.openings[0].sill_height, start.sill_height);
    }
}
