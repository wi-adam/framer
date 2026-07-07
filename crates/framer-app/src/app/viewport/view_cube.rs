//! Interactive view-cube overlay: clickable geometry + hit-testing, the projected
//! mesh, and edge/label drawing, ending in the `draw_view_cube` entry point.

use eframe::egui::epaint::Vertex;
use eframe::egui::{
    self, Align2, Color32, FontId, Mesh, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2,
};
use eframe::{egui_wgpu, wgpu};

use super::camera_3d::{View3dState, ViewCubeAction, ViewCubeOrientation};
use super::geom::{OrbitProjector, Point3, distance_to_segment, point_in_polygon};
use super::gpu::{Framer3dCallback, Framer3dFrameKey, GpuUniforms, GpuVertex};
use super::scene_build::{brighten, color_to_rgba};
use super::theme;
use super::view_common::draw_view_empty;
use crate::app::design::text_size;

// === extracted block appended below; visibility adjusted in place ===

pub(super) fn view_cube_rect(drawing: Rect) -> Rect {
    Rect::from_min_size(
        drawing.right_top() + Vec2::new(-152.0, 12.0),
        Vec2::splat(104.0),
    )
}

fn view_cube_body_rect(rect: Rect) -> Rect {
    Rect::from_min_max(
        rect.left_top() + Vec2::new(18.0, 8.0),
        rect.right_bottom() - Vec2::new(6.0, 6.0),
    )
}

pub(super) struct ViewCubeGeometry {
    pub(super) home_rect: Rect,
    pub(super) faces: Vec<ViewCubeFaceGeometry>,
    pub(super) edges: Vec<ViewCubeEdgeGeometry>,
    pub(super) corners: Vec<ViewCubeCornerGeometry>,
}

#[derive(Clone, Copy)]
pub(super) struct ViewCubeFaceGeometry {
    pub(super) action: ViewCubeAction,
    pub(super) points: [Pos2; 4],
}

#[derive(Clone, Copy)]
pub(super) struct ViewCubeEdgeGeometry {
    pub(super) action: ViewCubeAction,
    pub(super) points: [Pos2; 2],
}

#[derive(Clone, Copy)]
pub(super) struct ViewCubeCornerGeometry {
    pub(super) action: ViewCubeAction,
    pub(super) center: Pos2,
}

impl ViewCubeGeometry {
    pub(super) fn from_rect(rect: Rect, view: View3dState) -> Self {
        let corners = view_cube_points();
        let body_rect = view_cube_body_rect(rect);
        let projector = view_cube_projector(body_rect, view);
        let camera_direction = projector.view_direction();
        let projected = corners.map(|point| projector.project_point(point).pos);
        let face_specs = view_cube_face_specs();
        let faces = face_specs
            .iter()
            .filter_map(|spec| {
                view_cube_face_geometry(&projector, &corners, *spec, camera_direction)
            })
            .collect::<Vec<_>>();
        let mut visible_corners = [false; 8];
        for face in &faces {
            if let Some(spec) = face_specs.iter().find(|spec| spec.action == face.action) {
                for corner in spec.face {
                    visible_corners[corner] = true;
                }
            }
        }

        Self {
            home_rect: Rect::from_min_size(
                rect.left_top() + Vec2::new(6.0, 6.0),
                Vec2::splat(22.0),
            ),
            edges: view_cube_edges()
                .into_iter()
                .filter(|[start, end]| {
                    visible_corners[*start]
                        && visible_corners[*end]
                        && faces.iter().any(|face| {
                            face_specs
                                .iter()
                                .find(|spec| spec.action == face.action)
                                .is_some_and(|spec| {
                                    view_cube_face_has_edge(spec.face, *start, *end)
                                })
                        })
                })
                .map(|[start, end]| ViewCubeEdgeGeometry {
                    action: ViewCubeAction::snap(ViewCubeOrientation::from_points(
                        corners[start],
                        corners[end],
                    )),
                    points: [projected[start], projected[end]],
                })
                .collect(),
            corners: visible_corners
                .iter()
                .enumerate()
                .filter(|(_, visible)| **visible)
                .map(|(index, _)| ViewCubeCornerGeometry {
                    action: ViewCubeAction::snap(ViewCubeOrientation::from_point(corners[index])),
                    center: projected[index],
                })
                .collect(),
            faces,
        }
    }

    pub(super) fn hit(&self, position: Pos2) -> Option<ViewCubeAction> {
        if self.home_rect.contains(position) {
            Some(ViewCubeAction::Home)
        } else if let Some(corner) = self
            .corners
            .iter()
            .filter(|corner| corner.center.distance(position) <= 8.0)
            .min_by(|left, right| {
                left.center
                    .distance(position)
                    .total_cmp(&right.center.distance(position))
            })
        {
            Some(corner.action)
        } else if let Some(edge) = self
            .edges
            .iter()
            .filter(|edge| distance_to_segment(position, edge.points[0], edge.points[1]) <= 7.0)
            .min_by(|left, right| {
                distance_to_segment(position, left.points[0], left.points[1]).total_cmp(
                    &distance_to_segment(position, right.points[0], right.points[1]),
                )
            })
        {
            Some(edge.action)
        } else {
            self.faces
                .iter()
                .find(|face| point_in_polygon(position, &face.points))
                .map(|face| face.action)
        }
    }
}

fn view_cube_projector(rect: Rect, view: View3dState) -> OrbitProjector {
    let mut cube_view = view;
    cube_view.zoom = 1.0;
    OrbitProjector::from_points(&view_cube_points(), rect, cube_view)
        .expect("view cube has fixed geometry")
}

fn view_cube_points() -> [Point3; 8] {
    [
        Point3::vector(-1.0, -1.0, -1.0),
        Point3::vector(1.0, -1.0, -1.0),
        Point3::vector(1.0, 1.0, -1.0),
        Point3::vector(-1.0, 1.0, -1.0),
        Point3::vector(-1.0, -1.0, 1.0),
        Point3::vector(1.0, -1.0, 1.0),
        Point3::vector(1.0, 1.0, 1.0),
        Point3::vector(-1.0, 1.0, 1.0),
    ]
}

#[derive(Clone, Copy)]
struct ViewCubeFaceSpec {
    action: ViewCubeAction,
    face: [usize; 4],
    normal: Point3,
    color: Color32,
}

fn view_cube_face_specs() -> [ViewCubeFaceSpec; 6] {
    [
        ViewCubeFaceSpec {
            action: ViewCubeAction::BOTTOM,
            face: [0, 3, 2, 1],
            normal: -Point3::Z,
            color: theme::sheet_grid_major(),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::TOP,
            face: [4, 5, 6, 7],
            normal: Point3::Z,
            color: theme::sheet(),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::BACK,
            face: [0, 1, 5, 4],
            normal: -Point3::Y,
            color: theme::sheet_grid_major(),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::RIGHT,
            face: [1, 2, 6, 5],
            normal: Point3::X,
            color: theme::sheet_ruler(),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::FRONT,
            face: [2, 3, 7, 6],
            normal: Point3::Y,
            color: theme::sheet(),
        },
        ViewCubeFaceSpec {
            action: ViewCubeAction::LEFT,
            face: [3, 0, 4, 7],
            normal: -Point3::X,
            color: theme::sheet_grid_major(),
        },
    ]
}

fn view_cube_edges() -> [[usize; 2]; 12] {
    [
        [0, 1],
        [1, 2],
        [2, 3],
        [3, 0],
        [4, 5],
        [5, 6],
        [6, 7],
        [7, 4],
        [0, 4],
        [1, 5],
        [2, 6],
        [3, 7],
    ]
}

fn view_cube_face_has_edge(face: [usize; 4], start: usize, end: usize) -> bool {
    face.iter().enumerate().any(|(index, corner)| {
        let next = face[(index + 1) % face.len()];
        (*corner == start && next == end) || (*corner == end && next == start)
    })
}

fn view_cube_face_geometry(
    projector: &OrbitProjector,
    corners: &[Point3; 8],
    spec: ViewCubeFaceSpec,
    camera_direction: Point3,
) -> Option<ViewCubeFaceGeometry> {
    if spec.normal.dot(camera_direction) <= 0.0 {
        return None;
    }

    let points = spec
        .face
        .map(|index| projector.project_point(corners[index]).pos);
    Some(ViewCubeFaceGeometry {
        action: spec.action,
        points,
    })
}

pub(super) fn draw_view_cube(
    painter: &egui::Painter,
    rect: Rect,
    pointer: Option<Pos2>,
    clicked: bool,
    view: View3dState,
    gpu_target_format: Option<wgpu::TextureFormat>,
) -> Option<ViewCubeAction> {
    let geometry = ViewCubeGeometry::from_rect(rect, view);
    let hovered_action = pointer.and_then(|position| geometry.hit(position));
    let hovered_home = hovered_action == Some(ViewCubeAction::Home);

    painter.rect_filled(rect, 4.0, theme::with_alpha(theme::sheet(), 215));
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, theme::sheet_grid_major()),
        StrokeKind::Outside,
    );
    painter.rect_filled(
        geometry.home_rect,
        3.0,
        if hovered_home {
            theme::active_blue_soft()
        } else {
            theme::sheet_ruler()
        },
    );
    painter.rect_stroke(
        geometry.home_rect,
        3.0,
        Stroke::new(1.0, theme::dimension_line()),
        StrokeKind::Outside,
    );
    painter.text(
        geometry.home_rect.center(),
        Align2::CENTER_CENTER,
        "H",
        FontId::proportional(text_size::LABEL),
        theme::dimension_line(),
    );

    let body_rect = view_cube_body_rect(rect);
    let projector = view_cube_projector(body_rect, view);
    if let Some(target_format) = gpu_target_format {
        let (vertices, indices) = view_cube_mesh(hovered_action);
        painter.add(egui_wgpu::Callback::new_paint_callback(
            body_rect,
            Framer3dCallback {
                frame_key: Framer3dFrameKey::VIEW_CUBE,
                opaque_index_count: indices.len() as u32,
                transparent_index_count: 0,
                uniforms: GpuUniforms::from_projector_with_depth_base(&projector, body_rect, 0.14),
                vertices,
                indices,
                target_format,
            },
        ));
        draw_view_cube_edges(painter, &geometry, hovered_action);
        draw_view_cube_labels(painter, &projector, &geometry);
    } else {
        draw_view_empty(painter, body_rect, "3D");
    }

    if clicked { hovered_action } else { None }
}

pub(super) fn view_cube_mesh(hovered_action: Option<ViewCubeAction>) -> (Vec<GpuVertex>, Vec<u32>) {
    let corners = view_cube_points();
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    let hovered_orientation = hovered_action.and_then(ViewCubeAction::orientation);
    for spec in view_cube_face_specs() {
        let face_orientation = spec.action.orientation().expect("cube faces snap");
        let color = if hovered_orientation
            .is_some_and(|orientation| orientation.includes_face(face_orientation))
        {
            brighten(spec.color, 24)
        } else {
            spec.color
        };
        push_view_cube_face(
            &mut vertices,
            &mut indices,
            &corners,
            spec.face,
            spec.normal,
            color,
        );
    }

    (vertices, indices)
}

fn push_view_cube_face(
    vertices: &mut Vec<GpuVertex>,
    indices: &mut Vec<u32>,
    corners: &[Point3; 8],
    face: [usize; 4],
    normal: Point3,
    color: Color32,
) {
    push_view_cube_quad(
        vertices,
        indices,
        face.map(|index| corners[index]),
        normal,
        color_to_rgba(color),
    );
}

fn push_view_cube_quad(
    vertices: &mut Vec<GpuVertex>,
    indices: &mut Vec<u32>,
    points: [Point3; 4],
    normal: Point3,
    color: [f32; 4],
) {
    let base = vertices.len() as u32;
    for point in points {
        vertices.push(GpuVertex {
            position: [point.x, point.y, point.z],
            color,
            normal: [normal.x, normal.y, normal.z],
        });
    }
    indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

#[derive(Clone, Copy)]
pub(super) struct ViewCubeLabelSpec {
    action: ViewCubeAction,
    pub(super) text: &'static str,
    pub(super) center: Point3,
    pub(super) u_axis: Point3,
    v_axis: Point3,
    width: f32,
}

pub(super) fn view_cube_label_specs() -> [ViewCubeLabelSpec; 3] {
    [
        ViewCubeLabelSpec {
            action: ViewCubeAction::TOP,
            text: "TOP",
            center: Point3::vector(0.0, 0.0, 1.0),
            u_axis: Point3::Y,
            v_axis: Point3::X,
            width: 0.90,
        },
        ViewCubeLabelSpec {
            action: ViewCubeAction::RIGHT,
            text: "RIGHT",
            center: Point3::vector(1.0, 0.0, 0.0),
            u_axis: -Point3::Y,
            v_axis: Point3::Z,
            width: 1.28,
        },
        ViewCubeLabelSpec {
            action: ViewCubeAction::FRONT,
            text: "FRONT",
            center: Point3::vector(0.0, 1.0, 0.0),
            u_axis: Point3::X,
            v_axis: Point3::Z,
            width: 1.28,
        },
    ]
}

fn draw_view_cube_edges(
    painter: &egui::Painter,
    geometry: &ViewCubeGeometry,
    hovered_action: Option<ViewCubeAction>,
) {
    let stroke = Stroke::new(1.0, theme::with_alpha(theme::dimension_line(), 128));
    for face in &geometry.faces {
        for index in 0..face.points.len() {
            painter.line_segment(
                [
                    face.points[index],
                    face.points[(index + 1) % face.points.len()],
                ],
                stroke,
            );
        }
    }

    let Some(orientation) = hovered_action.and_then(ViewCubeAction::orientation) else {
        return;
    };

    let highlight = Stroke::new(2.25, theme::active_blue());
    match orientation.component_count() {
        1 => {
            if let Some(face) = geometry
                .faces
                .iter()
                .find(|face| face.action.orientation() == Some(orientation))
            {
                for index in 0..face.points.len() {
                    painter.line_segment(
                        [
                            face.points[index],
                            face.points[(index + 1) % face.points.len()],
                        ],
                        highlight,
                    );
                }
            }
        }
        2 => {
            if let Some(edge) = geometry
                .edges
                .iter()
                .find(|edge| edge.action.orientation() == Some(orientation))
            {
                painter.line_segment(edge.points, highlight);
            }
        }
        3 => {
            if let Some(corner) = geometry
                .corners
                .iter()
                .find(|corner| corner.action.orientation() == Some(orientation))
            {
                painter.circle_filled(
                    corner.center,
                    4.0,
                    theme::with_alpha(theme::active_blue(), 130),
                );
                painter.circle_stroke(corner.center, 4.0, highlight);
            }
        }
        _ => {}
    }
}

fn draw_view_cube_labels(
    painter: &egui::Painter,
    projector: &OrbitProjector,
    geometry: &ViewCubeGeometry,
) {
    for spec in view_cube_label_specs() {
        if geometry.faces.iter().any(|face| face.action == spec.action) {
            draw_view_cube_projected_label(painter, projector, spec);
        }
    }
}

fn draw_view_cube_projected_label(
    painter: &egui::Painter,
    projector: &OrbitProjector,
    spec: ViewCubeLabelSpec,
) {
    let color = theme::with_alpha(theme::framing_line_dark(), 215);
    let galley = painter.layout_no_wrap(
        spec.text.to_owned(),
        FontId::proportional(text_size::BODY),
        color,
    );
    let size = galley.rect.size();
    if size.x <= f32::EPSILON || size.y <= f32::EPSILON {
        return;
    }

    let center = projector.project_point(spec.center).pos;
    let u = projector
        .project_point(spec.center.offset(spec.u_axis, 1.0))
        .pos
        - center;
    let v = projector
        .project_point(spec.center.offset(spec.v_axis, 1.0))
        .pos
        - center;
    let point_scale = spec.width / size.x;
    let glyph_center = galley.rect.center();
    let font_image_size = painter.fonts_mut(|fonts| fonts.font_image_size());
    let uv_scale = Vec2::new(
        1.0 / font_image_size[0] as f32,
        1.0 / font_image_size[1] as f32,
    );
    let mut mesh = Mesh::default();

    for row in &galley.rows {
        if row.visuals.mesh.is_empty() {
            continue;
        }
        let index_offset = mesh.vertices.len() as u32;
        mesh.indices.extend(
            row.visuals
                .mesh
                .indices
                .iter()
                .map(|index| index + index_offset),
        );
        mesh.vertices
            .extend(row.visuals.mesh.vertices.iter().map(|vertex| {
                let local = row.pos + vertex.pos.to_vec2();
                let centered = local - glyph_center;
                let pos = center + u * (centered.x * point_scale) - v * (centered.y * point_scale);
                Vertex {
                    pos,
                    uv: (vertex.uv.to_vec2() * uv_scale).to_pos2(),
                    color,
                }
            }));
    }

    if !mesh.is_empty() {
        painter.add(Shape::mesh(mesh));
    }
}
