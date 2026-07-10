//! Pick volumes and projected hit-depth resolution.

use eframe::egui::Pos2;

use super::super::geom::{OrbitProjector, Point3, point_hits_projected_quad, point_in_polygon};
use super::{CUBOID_FACE_INDICES, GABLE_QUAD_FACES, GABLE_TRIANGLE_FACES};
use crate::app::ViewClick;

pub(super) struct PickSolid {
    pub(super) click: ViewClick,
    pub(super) priority: u8,
    pub(super) shape: PickShape,
}

/// The hit-test geometry of a pickable solid. Walls/members/openings are boxes
/// (hit face-by-face); roof/ceiling/floor surfaces are thin slabs hit-tested
/// against their projected outline polygon (which need not be a quad).
pub(super) enum PickShape {
    Cuboid([Point3; 8]),
    GablePrism([Point3; 6]),
    Surface(Vec<Point3>),
    Mesh {
        points: Vec<Point3>,
        triangles: Vec<[usize; 3]>,
    },
}

impl PickSolid {
    pub(super) fn cuboid(click: ViewClick, priority: u8, corners: [Point3; 8]) -> Self {
        Self {
            click,
            priority,
            shape: PickShape::Cuboid(corners),
        }
    }

    pub(super) fn surface(click: ViewClick, priority: u8, outline: Vec<Point3>) -> Self {
        Self {
            click,
            priority,
            shape: PickShape::Surface(outline),
        }
    }

    pub(super) fn gable_prism(click: ViewClick, priority: u8, corners: [Point3; 6]) -> Self {
        Self {
            click,
            priority,
            shape: PickShape::GablePrism(corners),
        }
    }

    pub(super) fn mesh(
        click: ViewClick,
        priority: u8,
        points: Vec<Point3>,
        triangles: Vec<[usize; 3]>,
    ) -> Self {
        Self {
            click,
            priority,
            shape: PickShape::Mesh { points, triangles },
        }
    }

    pub(super) fn hit_depth(&self, pointer: Pos2, projector: &OrbitProjector) -> Option<f32> {
        match &self.shape {
            PickShape::Cuboid(corners) => {
                let mut best_depth = None::<f32>;
                for face in CUBOID_FACE_INDICES {
                    let projected = face.map(|index| projector.project_point(corners[index]));
                    let positions = projected.map(|point| point.pos);
                    if point_hits_projected_quad(pointer, &positions) {
                        let depth = projected.iter().map(|point| point.depth).sum::<f32>() / 4.0;
                        best_depth = Some(best_depth.map_or(depth, |existing| existing.max(depth)));
                    }
                }
                best_depth
            }
            PickShape::GablePrism(corners) => {
                let mut best_depth = None::<f32>;
                for face in GABLE_TRIANGLE_FACES {
                    let projected = face.map(|index| projector.project_point(corners[index]));
                    let positions = projected.map(|point| point.pos).to_vec();
                    if point_in_polygon(pointer, &positions) {
                        let depth = projected.iter().map(|point| point.depth).sum::<f32>() / 3.0;
                        best_depth = Some(best_depth.map_or(depth, |existing| existing.max(depth)));
                    }
                }
                for face in GABLE_QUAD_FACES {
                    let projected = face.map(|index| projector.project_point(corners[index]));
                    let positions = projected.map(|point| point.pos);
                    if point_hits_projected_quad(pointer, &positions) {
                        let depth = projected.iter().map(|point| point.depth).sum::<f32>() / 4.0;
                        best_depth = Some(best_depth.map_or(depth, |existing| existing.max(depth)));
                    }
                }
                best_depth
            }
            PickShape::Surface(outline) => {
                let projected: Vec<_> = outline
                    .iter()
                    .map(|point| projector.project_point(*point))
                    .collect();
                let positions: Vec<Pos2> = projected.iter().map(|point| point.pos).collect();
                if positions.len() >= 3 && point_in_polygon(pointer, &positions) {
                    Some(
                        projected.iter().map(|point| point.depth).sum::<f32>()
                            / projected.len() as f32,
                    )
                } else {
                    None
                }
            }
            PickShape::Mesh { points, triangles } => {
                let mut best_depth = None::<f32>;
                for triangle in triangles {
                    let projected = triangle.map(|index| projector.project_point(points[index]));
                    let positions = projected.map(|point| point.pos).to_vec();
                    if point_in_polygon(pointer, &positions) {
                        let depth = projected.iter().map(|point| point.depth).sum::<f32>() / 3.0;
                        best_depth = Some(best_depth.map_or(depth, |existing| existing.max(depth)));
                    }
                }
                best_depth
            }
        }
    }
}
