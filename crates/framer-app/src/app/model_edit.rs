use framer_core::{Length, Point2, Wall};

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
