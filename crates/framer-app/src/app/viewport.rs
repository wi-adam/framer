use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2,
};
use framer_core::{BuildingModel, Length, Point2, Wall};
use framer_solver::{FrameMember, MemberKind, MemberOrientation};

use super::labels::{join_kind_label, kind_label};
use super::{FramerApp, Selection, ViewClick, ViewportMode};

#[derive(Debug, Clone, Copy)]
pub(super) struct View3dState {
    yaw: f32,
    pitch: f32,
    zoom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewCubeAction {
    Home,
    Top,
    Front,
    Right,
}

impl Default for View3dState {
    fn default() -> Self {
        Self {
            yaw: -FRAC_PI_4,
            pitch: 0.55,
            zoom: 1.0,
        }
    }
}

impl View3dState {
    fn orbit(&mut self, delta: Vec2) {
        self.yaw += delta.x * 0.01;
        self.pitch = (self.pitch - delta.y * 0.01).clamp(0.05, FRAC_PI_2 - 0.02);
    }

    fn zoom_by(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = (self.zoom * factor).clamp(0.35, 3.0);
        }
    }

    fn snap_to(&mut self, action: ViewCubeAction) {
        match action {
            ViewCubeAction::Home => *self = Self::default(),
            ViewCubeAction::Top => {
                self.yaw = 0.0;
                self.pitch = FRAC_PI_2;
            }
            ViewCubeAction::Front => {
                self.yaw = 0.0;
                self.pitch = 0.0;
            }
            ViewCubeAction::Right => {
                self.yaw = FRAC_PI_2;
                self.pitch = 0.0;
            }
        }
    }
}

impl FramerApp {
    pub(super) fn workspace(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("CAD Workspace");
            ui.separator();
            ui.label(self.model.code.display_name.as_str());
        });
        ui.add_space(8.0);

        let Some(plan) = &self.project_plan else {
            ui.label("No valid framing plan");
            return;
        };

        let click = match self.viewport_mode {
            ViewportMode::Plan => {
                draw_project_plan(ui, &self.model, self.selected_wall, &self.selected)
            }
            ViewportMode::Elevation => {
                let Some(wall) = self.model.walls.get(self.selected_wall) else {
                    ui.label("No wall selected");
                    return;
                };
                let Some(wall_plan) = plan.wall_plan(&wall.id) else {
                    ui.label("No generated framing for selected wall");
                    return;
                };
                let selected_member = match &self.selected {
                    Selection::Member { wall_id, member_id } if wall_id == &wall.id.0 => {
                        Some(member_id.as_str())
                    }
                    _ => None,
                };
                let section_x = if self.show_section {
                    section_position(wall, &self.selected)
                } else {
                    None
                };
                draw_wall_elevation(ui, wall, &wall_plan.members, selected_member, section_x).map(
                    |member_id| ViewClick::Member {
                        wall_id: wall.id.0.clone(),
                        member_id,
                    },
                )
            }
            ViewportMode::Axonometric => draw_project_axonometric(
                ui,
                &self.model,
                self.selected_wall,
                &self.selected,
                &mut self.view_3d,
            ),
        };

        if let Some(click) = click {
            match click {
                ViewClick::Wall(index) => {
                    self.selected_wall = index;
                    self.selected = Selection::Wall;
                }
                ViewClick::Opening {
                    wall_index,
                    opening_id,
                } => {
                    self.selected_wall = wall_index;
                    self.selected = Selection::Opening(opening_id);
                }
                ViewClick::Member { wall_id, member_id } => {
                    if let Some(index) = self
                        .model
                        .walls
                        .iter()
                        .position(|wall| wall.id.0 == wall_id)
                    {
                        self.selected_wall = index;
                    }
                    self.selected = Selection::Member { wall_id, member_id };
                }
            }
        }
    }
}

fn viewport_size(ui: &Ui) -> Vec2 {
    let available = ui.available_size();
    let width = available.x.max(420.0);
    let target_height = (width * 0.72).clamp(420.0, 640.0);
    let min_height = available.y.min(360.0);
    let height = available.y.min(target_height).max(min_height);

    Vec2::new(width, height)
}

fn viewport_drawing_rect(rect: Rect, margin: f32) -> Rect {
    Rect::from_min_max(
        rect.min + Vec2::splat(margin),
        rect.max - Vec2::new(margin, margin),
    )
}

fn draw_view_title(painter: &egui::Painter, drawing: Rect, title: impl Into<String>) {
    painter.text(
        drawing.left_top() + Vec2::new(0.0, -20.0),
        Align2::LEFT_CENTER,
        title.into(),
        FontId::proportional(13.0),
        Color32::from_rgb(70, 67, 61),
    );
}

fn draw_view_empty(painter: &egui::Painter, rect: Rect, label: &str) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(14.0),
        Color32::from_rgb(70, 67, 61),
    );
}

fn draw_view_border(painter: &egui::Painter, drawing: Rect) {
    painter.rect_stroke(
        drawing,
        0.0,
        Stroke::new(1.0, Color32::from_rgb(190, 184, 172)),
        StrokeKind::Outside,
    );
}

fn draw_view_background(painter: &egui::Painter, rect: Rect, color: Color32) {
    painter.rect_filled(rect, 0.0, color);
}

fn draw_project_plan(
    ui: &mut Ui,
    model: &BuildingModel,
    selected_wall: usize,
    selection: &Selection,
) -> Option<ViewClick> {
    let desired = viewport_size(ui);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let painter = ui.painter_at(rect);

    draw_view_background(&painter, rect, Color32::from_rgb(245, 244, 239));
    let drawing = viewport_drawing_rect(rect, 52.0);
    draw_view_border(&painter, drawing);

    let Some(bounds) = ModelBounds::from_model(model) else {
        draw_view_empty(&painter, rect, "No wall segments");
        return None;
    };

    let pointer = response.interact_pointer_pos();
    let mut clicked_wall = None;
    let mut clicked_opening = None;

    for join in &model.wall_joins {
        let point = plan_point(join.point, bounds, drawing);
        painter.circle_filled(point, 4.5, Color32::from_rgb(47, 95, 127));
        painter.text(
            point + Vec2::new(6.0, -7.0),
            Align2::LEFT_CENTER,
            join_kind_label(join.kind),
            FontId::proportional(10.0),
            Color32::from_rgb(47, 95, 127),
        );
    }

    for (index, wall) in model.walls.iter().enumerate() {
        let start = plan_point(wall.start, bounds, drawing);
        let end = plan_point(wall.end, bounds, drawing);
        let hovered =
            pointer.is_some_and(|position| distance_to_segment(position, start, end) < 8.0);
        let selected = selected_wall == index && matches!(selection, Selection::Wall);
        let stroke = if selected {
            Stroke::new(5.0, Color32::from_rgb(35, 94, 150))
        } else if hovered {
            Stroke::new(4.5, Color32::from_rgb(88, 88, 78))
        } else {
            Stroke::new(3.5, Color32::from_rgb(111, 91, 63))
        };
        painter.line_segment([start, end], stroke);
        if hovered && response.clicked() {
            clicked_wall = Some(ViewClick::Wall(index));
        }

        let midpoint = Pos2::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0);
        painter.text(
            midpoint + Vec2::new(5.0, -10.0),
            Align2::LEFT_CENTER,
            &wall.name,
            FontId::proportional(12.0),
            Color32::from_rgb(60, 56, 48),
        );

        for opening in &wall.openings {
            let left = plan_point(wall.point_at_local_x(opening.left()), bounds, drawing);
            let right = plan_point(wall.point_at_local_x(opening.right()), bounds, drawing);
            let opening_hovered =
                pointer.is_some_and(|position| distance_to_segment(position, left, right) < 9.0);
            let opening_selected = matches!(selection, Selection::Opening(id) if id == &opening.id.0)
                && selected_wall == index;
            painter.line_segment(
                [left, right],
                Stroke::new(7.0, Color32::from_rgb(245, 244, 239)),
            );
            painter.line_segment(
                [left, right],
                Stroke::new(
                    if opening_selected || opening_hovered {
                        3.0
                    } else {
                        2.0
                    },
                    if opening_selected {
                        Color32::from_rgb(35, 94, 150)
                    } else {
                        Color32::from_rgb(137, 102, 52)
                    },
                ),
            );
            if opening_hovered && response.clicked() {
                clicked_opening = Some(ViewClick::Opening {
                    wall_index: index,
                    opening_id: opening.id.0.clone(),
                });
            }
        }
    }

    draw_view_title(&painter, drawing, "Whole-project plan");

    clicked_opening.or(clicked_wall)
}

fn draw_project_axonometric(
    ui: &mut Ui,
    model: &BuildingModel,
    selected_wall: usize,
    selection: &Selection,
    view: &mut View3dState,
) -> Option<ViewClick> {
    let desired = viewport_size(ui);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    draw_view_background(&painter, rect, Color32::from_rgb(239, 243, 241));
    let drawing = viewport_drawing_rect(rect, 42.0);
    draw_view_border(&painter, drawing);
    let cube_rect = view_cube_rect(drawing);
    let pointer = response.interact_pointer_pos();

    if response.dragged_by(egui::PointerButton::Primary)
        && !pointer.is_some_and(|position| cube_rect.contains(position))
    {
        view.orbit(response.drag_delta());
    }

    if response.hovered() {
        let zoom_factor = ui.input(|input| {
            let wheel_factor = (input.smooth_scroll_delta.y * 0.002).exp();
            wheel_factor * input.zoom_delta()
        });
        if (zoom_factor - 1.0).abs() > f32::EPSILON {
            view.zoom_by(zoom_factor);
        }
    }

    let Some(projector) = OrbitProjector::from_model(model, drawing, *view) else {
        draw_view_empty(&painter, rect, "No wall segments");
        return None;
    };

    let mut wall_draw_order = model.walls.iter().enumerate().collect::<Vec<_>>();
    wall_draw_order.sort_by(|(_, left), (_, right)| {
        let left_depth = projector.wall_depth(left);
        let right_depth = projector.wall_depth(right);
        left_depth.total_cmp(&right_depth)
    });

    let mut clicked_wall = None;
    let mut clicked_opening = None;
    for (index, wall) in wall_draw_order {
        let points = [
            projector.project(wall.start, Length::ZERO),
            projector.project(wall.end, Length::ZERO),
            projector.project(wall.end, wall.height),
            projector.project(wall.start, wall.height),
        ];
        let positions = projected_positions(points);
        let hovered = pointer.is_some_and(|position| {
            !cube_rect.contains(position) && point_hits_projected_quad(position, &positions)
        });
        let selected = selected_wall == index && matches!(selection, Selection::Wall);
        let fill = if selected {
            Color32::from_rgb(202, 220, 230)
        } else if hovered {
            Color32::from_rgb(226, 225, 214)
        } else {
            Color32::from_rgb(216, 213, 200)
        };
        draw_projected_quad(
            &painter,
            &positions,
            fill,
            Stroke::new(
                if selected || hovered { 1.75 } else { 1.0 },
                Color32::from_rgb(111, 91, 63),
            ),
        );
        if hovered && response.clicked() {
            clicked_wall = Some(ViewClick::Wall(index));
        }

        for opening in &wall.openings {
            let left = wall.point_at_local_x(opening.left());
            let right = wall.point_at_local_x(opening.right());
            let opening_points = [
                projector.project(left, opening.sill_height),
                projector.project(right, opening.sill_height),
                projector.project(right, opening.top()),
                projector.project(left, opening.top()),
            ];
            let opening_positions = projected_positions(opening_points);
            let opening_hovered = pointer.is_some_and(|position| {
                !cube_rect.contains(position)
                    && point_hits_projected_quad(position, &opening_positions)
            });
            let opening_selected = matches!(selection, Selection::Opening(id) if id == &opening.id.0)
                && selected_wall == index;
            draw_projected_quad(
                &painter,
                &opening_positions,
                Color32::from_rgba_unmultiplied(250, 250, 248, 190),
                Stroke::new(
                    if opening_selected || opening_hovered {
                        2.0
                    } else {
                        1.0
                    },
                    if opening_selected {
                        Color32::from_rgb(35, 94, 150)
                    } else {
                        Color32::from_rgb(137, 102, 52)
                    },
                ),
            );
            if opening_hovered && response.clicked() {
                clicked_opening = Some(ViewClick::Opening {
                    wall_index: index,
                    opening_id: opening.id.0.clone(),
                });
            }
        }
    }

    let cube_action = draw_view_cube(&painter, cube_rect, pointer, response.clicked());
    if let Some(action) = cube_action {
        view.snap_to(action);
        return None;
    }

    draw_view_title(&painter, drawing, "3D shell workspace");

    clicked_opening.or(clicked_wall)
}

#[derive(Clone, Copy)]
struct ModelBounds {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl ModelBounds {
    fn from_model(model: &BuildingModel) -> Option<Self> {
        let mut bounds = None::<Self>;
        for point in model.walls.iter().flat_map(|wall| [wall.start, wall.end]) {
            let x = point.x.inches() as f32;
            let y = point.y.inches() as f32;
            bounds = Some(match bounds {
                Some(existing) => Self {
                    min_x: existing.min_x.min(x),
                    min_y: existing.min_y.min(y),
                    max_x: existing.max_x.max(x),
                    max_y: existing.max_y.max(y),
                },
                None => Self {
                    min_x: x,
                    min_y: y,
                    max_x: x,
                    max_y: y,
                },
            });
        }
        bounds
    }
}

fn plan_point(point: Point2, bounds: ModelBounds, drawing: Rect) -> Pos2 {
    let width = (bounds.max_x - bounds.min_x).max(1.0);
    let depth = (bounds.max_y - bounds.min_y).max(1.0);
    let scale = (drawing.width() / width).min(drawing.height() / depth);
    let used_width = width * scale;
    let used_height = depth * scale;
    Pos2::new(
        drawing.left()
            + (drawing.width() - used_width) / 2.0
            + (point.x.inches() as f32 - bounds.min_x) * scale,
        drawing.bottom()
            - (drawing.height() - used_height) / 2.0
            - (point.y.inches() as f32 - bounds.min_y) * scale,
    )
}

#[derive(Clone, Copy)]
struct Point3 {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Clone, Copy)]
struct ProjectedPoint {
    pos: Pos2,
    depth: f32,
}

struct OrbitProjector {
    raw_center: Vec2,
    scale: f32,
    origin: Pos2,
    right: Vec2,
    depth_axis: Vec2,
    pitch: f32,
    center: Point3,
}

impl OrbitProjector {
    fn from_model(model: &BuildingModel, drawing: Rect, view: View3dState) -> Option<Self> {
        let points = model_3d_points(model)?;
        let yaw = view.yaw;
        let pitch = view.pitch.clamp(0.0, FRAC_PI_2);
        let right = Vec2::angled(yaw);
        let depth_axis = Vec2::new(-right.y, right.x);
        let center = model_3d_center(&points);
        let mut raw_points = Vec::with_capacity(points.len());
        for point in &points {
            raw_points.push(raw_orbit(*point, center, right, depth_axis, pitch).0);
        }

        let min_x = raw_points
            .iter()
            .map(|point| point.x)
            .fold(f32::MAX, f32::min);
        let min_y = raw_points
            .iter()
            .map(|point| point.y)
            .fold(f32::MAX, f32::min);
        let max_x = raw_points
            .iter()
            .map(|point| point.x)
            .fold(f32::MIN, f32::max);
        let max_y = raw_points
            .iter()
            .map(|point| point.y)
            .fold(f32::MIN, f32::max);
        let width = (max_x - min_x).max(1.0);
        let height = (max_y - min_y).max(1.0);
        let scale = (drawing.width() / width).min(drawing.height() / height) * 0.92 * view.zoom;
        let raw_center = Vec2::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);

        Some(Self {
            raw_center,
            scale,
            origin: drawing.center(),
            right,
            depth_axis,
            pitch,
            center,
        })
    }

    fn project(&self, point: Point2, elevation: Length) -> ProjectedPoint {
        let point = Point3::new(point.x, point.y, elevation);
        let (raw, depth) = raw_orbit(point, self.center, self.right, self.depth_axis, self.pitch);
        ProjectedPoint {
            pos: Pos2::new(
                self.origin.x + (raw.x - self.raw_center.x) * self.scale,
                self.origin.y + (raw.y - self.raw_center.y) * self.scale,
            ),
            depth,
        }
    }

    fn wall_depth(&self, wall: &Wall) -> f32 {
        let points = [
            self.project(wall.start, Length::ZERO),
            self.project(wall.end, Length::ZERO),
            self.project(wall.start, wall.height),
            self.project(wall.end, wall.height),
        ];
        points.iter().map(|point| point.depth).sum::<f32>() / points.len() as f32
    }
}

impl Point3 {
    fn new(point_x: Length, point_y: Length, elevation: Length) -> Self {
        Self {
            x: point_x.inches() as f32,
            y: point_y.inches() as f32,
            z: elevation.inches() as f32,
        }
    }
}

fn model_3d_points(model: &BuildingModel) -> Option<Vec<Point3>> {
    let mut points = Vec::new();
    for wall in &model.walls {
        points.push(Point3::new(wall.start.x, wall.start.y, Length::ZERO));
        points.push(Point3::new(wall.end.x, wall.end.y, Length::ZERO));
        points.push(Point3::new(wall.start.x, wall.start.y, wall.height));
        points.push(Point3::new(wall.end.x, wall.end.y, wall.height));
    }

    (!points.is_empty()).then_some(points)
}

fn model_3d_center(points: &[Point3]) -> Point3 {
    let mut min = Point3 {
        x: f32::MAX,
        y: f32::MAX,
        z: f32::MAX,
    };
    let mut max = Point3 {
        x: f32::MIN,
        y: f32::MIN,
        z: f32::MIN,
    };

    for point in points {
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        min.z = min.z.min(point.z);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
        max.z = max.z.max(point.z);
    }

    Point3 {
        x: (min.x + max.x) / 2.0,
        y: (min.y + max.y) / 2.0,
        z: (min.z + max.z) / 2.0,
    }
}

fn raw_orbit(
    point: Point3,
    center: Point3,
    right: Vec2,
    depth_axis: Vec2,
    pitch: f32,
) -> (Vec2, f32) {
    let x = point.x - center.x;
    let y = point.y - center.y;
    let z = point.z - center.z;
    let along_depth = x * depth_axis.x + y * depth_axis.y;
    let raw = Vec2::new(
        x * right.x + y * right.y,
        along_depth * pitch.sin() - z * pitch.cos(),
    );
    let depth = along_depth * pitch.cos() + z * pitch.sin();

    (raw, depth)
}

fn projected_positions(points: [ProjectedPoint; 4]) -> [Pos2; 4] {
    [points[0].pos, points[1].pos, points[2].pos, points[3].pos]
}

fn draw_projected_quad(painter: &egui::Painter, points: &[Pos2; 4], fill: Color32, stroke: Stroke) {
    if polygon_area(points) <= 1.0 {
        painter.line_segment([points[0], points[1]], stroke);
    } else {
        painter.add(Shape::convex_polygon(points.to_vec(), fill, stroke));
    }
}

fn point_hits_projected_quad(point: Pos2, points: &[Pos2; 4]) -> bool {
    if polygon_area(points) <= 8.0 {
        distance_to_segment(point, points[0], points[1]) < 8.0
    } else {
        point_in_polygon(point, points)
    }
}

fn polygon_area(points: &[Pos2]) -> f32 {
    if points.len() < 3 {
        return 0.0;
    }

    let mut area = 0.0;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        area += current.x * next.y - next.x * current.y;
    }
    area.abs() * 0.5
}

fn point_in_polygon(point: Pos2, points: &[Pos2]) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let a = points[current];
        let b = points[previous];
        if ((a.y > point.y) != (b.y > point.y))
            && (point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x)
        {
            inside = !inside;
        }
        previous = current;
    }

    inside
}

fn view_cube_rect(drawing: Rect) -> Rect {
    Rect::from_min_size(
        drawing.right_top() + Vec2::new(-118.0, 12.0),
        Vec2::splat(104.0),
    )
}

struct ViewCubeGeometry {
    home_rect: Rect,
    top_face: [Pos2; 4],
    right_face: [Pos2; 4],
    front_face: [Pos2; 4],
}

impl ViewCubeGeometry {
    fn from_rect(rect: Rect) -> Self {
        let center_x = rect.center().x;
        let top_y = rect.top() + 8.0;
        let top = pos2(center_x, top_y);
        let right = pos2(center_x + 38.0, top_y + 22.0);
        let middle = pos2(center_x, top_y + 44.0);
        let left = pos2(center_x - 38.0, top_y + 22.0);
        let right_bottom = pos2(center_x + 38.0, top_y + 64.0);
        let bottom = pos2(center_x, top_y + 86.0);
        let left_bottom = pos2(center_x - 38.0, top_y + 64.0);

        Self {
            home_rect: Rect::from_min_size(
                rect.left_top() + Vec2::new(6.0, 6.0),
                Vec2::splat(22.0),
            ),
            top_face: [top, right, middle, left],
            right_face: [right, right_bottom, bottom, middle],
            front_face: [left, middle, bottom, left_bottom],
        }
    }

    fn hit(&self, position: Pos2) -> Option<ViewCubeAction> {
        if self.home_rect.contains(position) {
            Some(ViewCubeAction::Home)
        } else if point_in_polygon(position, &self.top_face) {
            Some(ViewCubeAction::Top)
        } else if point_in_polygon(position, &self.right_face) {
            Some(ViewCubeAction::Right)
        } else if point_in_polygon(position, &self.front_face) {
            Some(ViewCubeAction::Front)
        } else {
            None
        }
    }
}

fn draw_view_cube(
    painter: &egui::Painter,
    rect: Rect,
    pointer: Option<Pos2>,
    clicked: bool,
) -> Option<ViewCubeAction> {
    let geometry = ViewCubeGeometry::from_rect(rect);
    let hovered_action = pointer.and_then(|position| geometry.hit(position));
    let hovered_home = hovered_action == Some(ViewCubeAction::Home);
    let hovered_top = hovered_action == Some(ViewCubeAction::Top);
    let hovered_right = hovered_action == Some(ViewCubeAction::Right);
    let hovered_front = hovered_action == Some(ViewCubeAction::Front);

    painter.rect_filled(
        rect,
        4.0,
        Color32::from_rgba_unmultiplied(250, 250, 248, 215),
    );
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, Color32::from_rgb(174, 176, 170)),
        StrokeKind::Outside,
    );
    painter.rect_filled(
        geometry.home_rect,
        3.0,
        if hovered_home {
            Color32::from_rgb(214, 225, 232)
        } else {
            Color32::from_rgb(232, 234, 229)
        },
    );
    painter.rect_stroke(
        geometry.home_rect,
        3.0,
        Stroke::new(1.0, Color32::from_rgb(129, 132, 127)),
        StrokeKind::Outside,
    );
    painter.text(
        geometry.home_rect.center(),
        Align2::CENTER_CENTER,
        "H",
        FontId::proportional(11.0),
        Color32::from_rgb(61, 67, 71),
    );

    draw_view_cube_face(
        painter,
        &geometry.top_face,
        hovered_top,
        Color32::from_rgb(232, 237, 234),
        "Top",
    );
    draw_view_cube_face(
        painter,
        &geometry.right_face,
        hovered_right,
        Color32::from_rgb(213, 224, 230),
        "Right",
    );
    draw_view_cube_face(
        painter,
        &geometry.front_face,
        hovered_front,
        Color32::from_rgb(222, 218, 207),
        "Front",
    );

    if clicked { hovered_action } else { None }
}

fn draw_view_cube_face(
    painter: &egui::Painter,
    points: &[Pos2; 4],
    hovered: bool,
    fill: Color32,
    label: &str,
) {
    let fill = if hovered {
        Color32::from_rgb(194, 218, 232)
    } else {
        fill
    };
    painter.add(Shape::convex_polygon(
        points.to_vec(),
        fill,
        Stroke::new(1.0, Color32::from_rgb(105, 108, 103)),
    ));
    let center = points
        .iter()
        .fold(Vec2::ZERO, |sum, point| sum + point.to_vec2())
        / points.len() as f32;
    painter.text(
        Pos2::new(center.x, center.y),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.0),
        Color32::from_rgb(57, 61, 64),
    );
}

fn distance_to_segment(point: Pos2, start: Pos2, end: Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + segment * t)
}

fn draw_wall_elevation(
    ui: &mut Ui,
    wall: &Wall,
    members: &[FrameMember],
    selected_member: Option<&str>,
    section_x: Option<Length>,
) -> Option<String> {
    let available = ui.available_size();
    let desired = Vec2::new(available.x.max(420.0), (available.y - 16.0).max(420.0));
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let painter = ui.painter_at(rect);

    let margin = 52.0;
    let drawing = Rect::from_min_max(
        rect.min + Vec2::splat(margin),
        rect.max - Vec2::new(margin, margin),
    );

    painter.rect_filled(rect, 0.0, Color32::from_rgb(246, 244, 239));
    painter.rect_stroke(
        drawing,
        0.0,
        Stroke::new(1.0, Color32::from_rgb(190, 184, 172)),
        StrokeKind::Outside,
    );

    let sx = drawing.width() / wall.length.inches().max(1.0) as f32;
    let sy = drawing.height() / wall.height.inches().max(1.0) as f32;
    let pointer = response.interact_pointer_pos();
    let mut clicked = None;

    draw_opening_guides(&painter, drawing, sx, sy, wall);

    for member in members {
        let member_rect = member_rect(drawing, sx, sy, member);
        let hovered = pointer.is_some_and(|position| member_rect.contains(position));
        let selected = selected_member == Some(member.id.as_str());
        draw_member_rect(&painter, member_rect, member.kind, selected, hovered);
        if hovered && response.clicked() {
            clicked = Some(member.id.clone());
        }
    }

    if let Some(section_x) = section_x {
        draw_section_line(&painter, drawing, sx, section_x);
    }

    painter.text(
        Pos2::new(drawing.left(), drawing.bottom() + 20.0),
        Align2::LEFT_CENTER,
        format!("{} x {}", wall.length, wall.height),
        FontId::proportional(13.0),
        Color32::from_rgb(70, 67, 61),
    );

    clicked
}

fn member_rect(drawing: Rect, sx: f32, sy: f32, member: &FrameMember) -> Rect {
    let start_x = drawing.left() + member.x.inches() as f32 * sx;
    let start_y = drawing.bottom() - member.elevation.inches() as f32 * sy;

    match member.orientation {
        MemberOrientation::Horizontal => {
            let width = (member.cut_length.inches() as f32 * sx).max(2.0);
            let height = (member.cross_section_depth.inches() as f32 * sy).max(3.0);
            Rect::from_min_size(
                Pos2::new(start_x, start_y - height),
                Vec2::new(width, height),
            )
        }
        MemberOrientation::Vertical => {
            let width = (member.cross_section_depth.inches() as f32 * sx).max(3.0);
            let height = (member.cut_length.inches() as f32 * sy).max(2.0);
            Rect::from_min_size(
                Pos2::new(start_x - width / 2.0, start_y - height),
                Vec2::new(width, height),
            )
        }
    }
}

fn draw_opening_guides(painter: &egui::Painter, drawing: Rect, sx: f32, sy: f32, wall: &Wall) {
    for opening in &wall.openings {
        let x = drawing.left() + opening.left().inches() as f32 * sx;
        let y = drawing.bottom() - opening.top().inches() as f32 * sy;
        let width = (opening.width.inches() as f32 * sx).max(4.0);
        let height = (opening.height.inches() as f32 * sy).max(4.0);
        let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, height));
        painter.rect_filled(
            rect,
            0.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 76),
        );
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, Color32::from_rgb(137, 102, 52)),
            StrokeKind::Outside,
        );
        painter.text(
            rect.left_top() + Vec2::new(4.0, 12.0),
            Align2::LEFT_CENTER,
            kind_label(opening.kind),
            FontId::proportional(11.0),
            Color32::from_rgb(99, 74, 39),
        );
    }
}

fn draw_member_rect(
    painter: &egui::Painter,
    rect: Rect,
    kind: MemberKind,
    selected: bool,
    hovered: bool,
) {
    painter.rect_filled(rect, 1.0, member_color(kind));
    let stroke = if selected {
        Stroke::new(2.0, Color32::from_rgb(34, 95, 155))
    } else if hovered {
        Stroke::new(1.5, Color32::from_rgb(40, 40, 40))
    } else {
        Stroke::new(0.75, Color32::from_rgb(87, 70, 52))
    };
    painter.rect_stroke(rect, 1.0, stroke, StrokeKind::Outside);
}

fn draw_section_line(painter: &egui::Painter, drawing: Rect, sx: f32, x: Length) {
    let px = drawing.left() + x.inches() as f32 * sx;
    painter.line_segment(
        [
            Pos2::new(px, drawing.top()),
            Pos2::new(px, drawing.bottom()),
        ],
        Stroke::new(1.5, Color32::from_rgb(45, 91, 138)),
    );
    painter.text(
        Pos2::new(px + 5.0, drawing.top() + 14.0),
        Align2::LEFT_CENTER,
        "A-A",
        FontId::proportional(12.0),
        Color32::from_rgb(45, 91, 138),
    );
}

fn member_color(kind: MemberKind) -> Color32 {
    match kind {
        MemberKind::BottomPlate | MemberKind::TopPlate => Color32::from_rgb(99, 85, 67),
        MemberKind::CornerPost => Color32::from_rgb(52, 95, 127),
        MemberKind::CommonStud => Color32::from_rgb(186, 145, 94),
        MemberKind::KingStud => Color32::from_rgb(151, 100, 61),
        MemberKind::JackStud => Color32::from_rgb(211, 168, 95),
        MemberKind::Header => Color32::from_rgb(115, 130, 99),
        MemberKind::RoughSill => Color32::from_rgb(92, 121, 144),
        MemberKind::CrippleStud => Color32::from_rgb(218, 190, 139),
    }
}

fn section_position(wall: &Wall, selection: &Selection) -> Option<Length> {
    match selection {
        Selection::Opening(id) => wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == *id)
            .map(|opening| opening.center),
        Selection::Member { .. } | Selection::Join(_) | Selection::Level(_) | Selection::Wall => {
            Some(wall.length / 2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_3d_state_orbits_zooms_and_snaps() {
        let mut view = View3dState::default();
        let initial_yaw = view.yaw;
        let initial_pitch = view.pitch;

        view.orbit(Vec2::new(20.0, -10.0));
        assert!(view.yaw > initial_yaw);
        assert!(view.pitch > initial_pitch);

        view.zoom_by(10.0);
        assert_eq!(view.zoom, 3.0);

        view.snap_to(ViewCubeAction::Top);
        assert_close(view.yaw, 0.0);
        assert_close(view.pitch, FRAC_PI_2);

        view.snap_to(ViewCubeAction::Home);
        assert_close(view.yaw, -FRAC_PI_4);
        assert_close(view.zoom, 1.0);
    }

    #[test]
    fn orbit_projector_changes_projection_when_view_rotates() {
        let model = BuildingModel::demo_shell();
        let drawing = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let front_end = model.walls[0].end;

        let home = OrbitProjector::from_model(&model, drawing, View3dState::default())
            .unwrap()
            .project(front_end, Length::ZERO)
            .pos;
        let mut right_view = View3dState::default();
        right_view.snap_to(ViewCubeAction::Right);
        let right = OrbitProjector::from_model(&model, drawing, right_view)
            .unwrap()
            .project(front_end, Length::ZERO)
            .pos;

        assert!(home.distance(right) > 8.0);
    }

    #[test]
    fn view_cube_geometry_hits_clickable_faces() {
        let rect = Rect::from_min_size(pos2(100.0, 80.0), Vec2::splat(104.0));
        let geometry = ViewCubeGeometry::from_rect(rect);

        assert_eq!(
            geometry.hit(geometry.home_rect.center()),
            Some(ViewCubeAction::Home)
        );
        assert_eq!(
            geometry.hit(face_center(&geometry.top_face)),
            Some(ViewCubeAction::Top)
        );
        assert_eq!(
            geometry.hit(face_center(&geometry.right_face)),
            Some(ViewCubeAction::Right)
        );
        assert_eq!(
            geometry.hit(face_center(&geometry.front_face)),
            Some(ViewCubeAction::Front)
        );
        assert_eq!(
            geometry.hit(rect.left_bottom() + Vec2::new(4.0, -4.0)),
            None
        );
    }

    fn face_center(points: &[Pos2; 4]) -> Pos2 {
        let center = points
            .iter()
            .fold(Vec2::ZERO, |sum, point| sum + point.to_vec2())
            / points.len() as f32;
        Pos2::new(center.x, center.y)
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {actual} to be close to {expected}"
        );
    }
}
