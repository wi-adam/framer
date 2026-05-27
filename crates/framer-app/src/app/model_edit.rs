use framer_core::{CodeProfile, Length, Opening, Point2, Wall};

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
}

impl OpeningDragConstraints {
    pub(super) fn from_code(code: &CodeProfile) -> Self {
        Self {
            edge_clearance: code.stud_profile.thickness() * 2,
            top_clearance: opening_top_clearance(code),
        }
    }
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
        let half_width = start.width.min(usable_width) / 2;
        let center = clamp_length(
            start.center + delta_x,
            min_x + half_width,
            max_x - half_width,
        );
        let max_bottom = (max_top - start.height).max(Length::ZERO);
        bottom = clamp_length(start.sill_height + delta_y, Length::ZERO, max_bottom);
        left = center - start.width / 2;
        right = center + start.width / 2;
        top = bottom + start.height;
    } else {
        if handle.resizes_left() {
            left = clamp_length(start.left() + delta_x, min_x, start.right() - min_width);
        }
        if handle.resizes_right() {
            right = clamp_length(start.right() + delta_x, left + min_width, max_x);
        }
        if handle.resizes_bottom() {
            bottom = clamp_length(
                start.sill_height + delta_y,
                Length::ZERO,
                start.top() - min_height,
            );
        }
        if handle.resizes_top() {
            top = clamp_length(start.top() + delta_y, bottom + min_height, max_top);
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
}
