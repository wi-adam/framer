//! Pure geometry and projection math shared across the viewport renderers.
//!
//! No egui paint side effects beyond `Pos2`/`Rect`/`Vec2` value types, and no
//! `FramerApp` coupling. Pulling these helpers into one leaf module is what lets
//! every renderer depend *downward* on `geom` instead of sideways on each other.

use std::f32::consts::FRAC_PI_2;
use std::ops::Neg;

use eframe::egui::{Pos2, Rect, Vec2};
use framer_core::{BuildingModel, Length, Point2, QuarterTurn};

use super::camera_2d::View2dState;
use super::camera_3d::View3dState;

#[derive(Clone, Copy)]
pub(super) struct ModelBounds {
    pub(super) min_x: f32,
    pub(super) min_y: f32,
    pub(super) max_x: f32,
    pub(super) max_y: f32,
}

impl ModelBounds {
    pub(super) fn from_model(model: &BuildingModel) -> Option<Self> {
        let mut bounds = None::<Self>;
        for point in model.walls.iter().flat_map(|wall| [wall.start, wall.end]) {
            bounds = Some(include_model_point(bounds, point));
        }

        for instance in &model.furnishing_instances {
            if let Some(family) = model
                .furnishings
                .iter()
                .find(|family| family.id == instance.family)
            {
                bounds = include_object_footprint(
                    bounds,
                    instance.position,
                    family.size.width,
                    family.size.depth,
                    instance.rotation,
                );
            }
        }
        for instance in &model.mep_instances {
            if let Some(family) = model
                .mep_objects
                .iter()
                .find(|family| family.id == instance.family)
            {
                bounds = include_object_footprint(
                    bounds,
                    instance.position,
                    family.size.width,
                    family.size.depth,
                    instance.rotation,
                );
            }
        }
        bounds
    }
}

fn include_model_point(bounds: Option<ModelBounds>, point: Point2) -> ModelBounds {
    let x = point.x.inches() as f32;
    let y = point.y.inches() as f32;
    match bounds {
        Some(existing) => ModelBounds {
            min_x: existing.min_x.min(x),
            min_y: existing.min_y.min(y),
            max_x: existing.max_x.max(x),
            max_y: existing.max_y.max(y),
        },
        None => ModelBounds {
            min_x: x,
            min_y: y,
            max_x: x,
            max_y: y,
        },
    }
}

fn include_object_footprint(
    bounds: Option<ModelBounds>,
    position: Point2,
    width: Length,
    depth: Length,
    rotation: QuarterTurn,
) -> Option<ModelBounds> {
    let rotated = matches!(rotation, QuarterTurn::Deg90 | QuarterTurn::Deg270);
    let footprint_width = if rotated { depth } else { width };
    let footprint_depth = if rotated { width } else { depth };
    let half_width = footprint_width / 2;
    let half_depth = footprint_depth / 2;
    let bounds = Some(include_model_point(
        bounds,
        Point2::new(position.x - half_width, position.y - half_depth),
    ));
    Some(include_model_point(
        bounds,
        Point2::new(position.x + half_width, position.y + half_depth),
    ))
}

pub(super) fn plan_point(
    point: Point2,
    bounds: ModelBounds,
    drawing: Rect,
    view: &View2dState,
) -> Pos2 {
    let width = (bounds.max_x - bounds.min_x).max(1.0);
    let depth = (bounds.max_y - bounds.min_y).max(1.0);
    let scale = (drawing.width() / width).min(drawing.height() / depth);
    let used_width = width * scale;
    let used_height = depth * scale;
    let base = Pos2::new(
        drawing.left()
            + (drawing.width() - used_width) / 2.0
            + (point.x.inches() as f32 - bounds.min_x) * scale,
        drawing.bottom()
            - (drawing.height() - used_height) / 2.0
            - (point.y.inches() as f32 - bounds.min_y) * scale,
    );
    view.apply(base, drawing)
}

/// Inverse of [`plan_point`]: map a screen position back to model coordinates.
pub(super) fn plan_inverse_point(
    screen: Pos2,
    bounds: ModelBounds,
    drawing: Rect,
    view: &View2dState,
) -> Point2 {
    let width = (bounds.max_x - bounds.min_x).max(1.0);
    let depth = (bounds.max_y - bounds.min_y).max(1.0);
    let scale = (drawing.width() / width).min(drawing.height() / depth);
    let used_width = width * scale;
    let used_height = depth * scale;
    // Undo the pan/zoom first, then invert the fit-to-bounds base mapping.
    let base = view.unapply(screen, drawing);
    let x = bounds.min_x + (base.x - drawing.left() - (drawing.width() - used_width) / 2.0) / scale;
    let y =
        bounds.min_y + (drawing.bottom() - (drawing.height() - used_height) / 2.0 - base.y) / scale;
    Point2::new(Length::from_inches(x as f64), Length::from_inches(y as f64))
}

#[derive(Clone, Copy)]
pub(super) struct Point3 {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) z: f32,
}

#[derive(Clone, Copy)]
pub(super) struct ProjectedPoint {
    pub(super) pos: Pos2,
    pub(super) depth: f32,
}

pub(super) struct OrbitProjector {
    pub(super) raw_center: Vec2,
    pub(super) scale: f32,
    pub(super) origin: Pos2,
    pub(super) right: Vec2,
    pub(super) depth_axis: Vec2,
    pub(super) pitch: f32,
    pub(super) center: Point3,
    pub(super) depth_center: f32,
    pub(super) depth_scale: f32,
}

impl OrbitProjector {
    #[cfg(test)]
    pub(super) fn from_model(
        model: &BuildingModel,
        drawing: Rect,
        view: View3dState,
    ) -> Option<Self> {
        let points = model_3d_points(model)?;
        Self::from_points(&points, drawing, view)
    }

    pub(super) fn from_points(points: &[Point3], drawing: Rect, view: View3dState) -> Option<Self> {
        if points.is_empty() {
            return None;
        }
        let yaw = view.yaw;
        let pitch = view.pitch.clamp(-FRAC_PI_2, FRAC_PI_2);
        let right = Vec2::angled(yaw);
        let depth_axis = Vec2::new(-right.y, right.x);
        let auto_center = model_3d_center(points);
        let radius = model_3d_radius(points, auto_center).max(1.0);
        // Pan slides the pivot by `pan * radius` (radius-relative world units),
        // matching the Render camera so the shared vantage stays in sync. Dolly is
        // perspective-only and has no meaning for this orthographic projection.
        let center = Point3::vector(
            auto_center.x + view.pan.x * radius,
            auto_center.y + view.pan.y * radius,
            auto_center.z + view.pan.z * radius,
        );
        let diameter = radius * 2.0;
        let scale = drawing.width().min(drawing.height()) / diameter * 0.92 * view.zoom;

        Some(Self {
            raw_center: Vec2::ZERO,
            scale,
            origin: drawing.center(),
            right,
            depth_axis,
            pitch,
            center,
            depth_center: 0.0,
            depth_scale: 0.45 / diameter,
        })
    }

    #[cfg(test)]
    pub(super) fn project(&self, point: Point2, elevation: Length) -> ProjectedPoint {
        let point = Point3::new(point.x, point.y, elevation);
        self.project_point(point)
    }

    pub(super) fn project_point(&self, point: Point3) -> ProjectedPoint {
        let (raw, depth) = raw_orbit(point, self.center, self.right, self.depth_axis, self.pitch);
        ProjectedPoint {
            pos: Pos2::new(
                self.origin.x + (raw.x - self.raw_center.x) * self.scale,
                self.origin.y + (raw.y - self.raw_center.y) * self.scale,
            ),
            depth,
        }
    }

    pub(super) fn view_direction(&self) -> Point3 {
        Point3::vector(
            self.depth_axis.x * self.pitch.cos(),
            self.depth_axis.y * self.pitch.cos(),
            self.pitch.sin(),
        )
    }
}

impl Point3 {
    pub(super) const X: Self = Self {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    pub(super) const Y: Self = Self {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    pub(super) const Z: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    #[cfg(test)]
    pub(super) fn new(point_x: Length, point_y: Length, elevation: Length) -> Self {
        Self {
            x: point_x.inches() as f32,
            y: point_y.inches() as f32,
            z: elevation.inches() as f32,
        }
    }

    pub(super) fn vector(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub(super) fn distance_squared(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }

    pub(super) fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub(super) fn offset(self, axis: Self, amount: f32) -> Self {
        Self {
            x: self.x + axis.x * amount,
            y: self.y + axis.y * amount,
            z: self.z + axis.z * amount,
        }
    }
}

impl Neg for Point3 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
}

#[cfg(test)]
pub(super) fn model_3d_points(model: &BuildingModel) -> Option<Vec<Point3>> {
    let mut points = Vec::new();
    for wall in &model.walls {
        points.push(Point3::new(wall.start.x, wall.start.y, Length::ZERO));
        points.push(Point3::new(wall.end.x, wall.end.y, Length::ZERO));
        points.push(Point3::new(wall.start.x, wall.start.y, wall.height));
        points.push(Point3::new(wall.end.x, wall.end.y, wall.height));
    }

    (!points.is_empty()).then_some(points)
}

pub(super) fn model_3d_center(points: &[Point3]) -> Point3 {
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

pub(super) fn model_3d_radius(points: &[Point3], center: Point3) -> f32 {
    points
        .iter()
        .map(|point| point.distance_squared(center))
        .fold(0.0, f32::max)
        .sqrt()
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

pub(super) fn point_hits_projected_quad(point: Pos2, points: &[Pos2; 4]) -> bool {
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

pub(super) fn point_in_polygon(point: Pos2, points: &[Pos2]) -> bool {
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

pub(super) fn distance_to_segment(point: Pos2, start: Pos2, end: Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + segment * t)
}
