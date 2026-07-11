//! Generated-member emission from shared UI-free physical solids.

use eframe::egui::Color32;
use framer_geometry::PhysicalBody;

use super::{PickSolid, Point3, SceneBuilder, color_to_rgba};
use crate::app::ViewClick;

impl SceneBuilder {
    /// Lower one geometry-owned member solid into the viewport's vertex format.
    /// Rendering and picking both consume the exact same indexed surface mesh.
    pub(super) fn push_member_body(
        &mut self,
        body: &PhysicalBody,
        click: ViewClick,
        color: Color32,
    ) {
        let points: Vec<_> = body
            .solid
            .surface
            .points
            .iter()
            .map(|point| Point3::vector(point.x as f32, point.y as f32, point.z as f32))
            .collect();
        if points.is_empty() || body.solid.surface.triangles.is_empty() {
            return;
        }
        self.points.extend_from_slice(&points);
        let color = color_to_rgba(color);
        for &triangle in &body.solid.surface.triangles {
            self.push_triangle(triangle.map(|index| points[index]), color);
        }
        self.picks.push(PickSolid::mesh(
            click,
            3,
            points,
            body.solid.surface.triangles.clone(),
        ));
    }
}
