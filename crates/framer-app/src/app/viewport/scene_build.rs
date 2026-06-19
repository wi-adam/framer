//! Builds the 3D scene mesh (`Scene3d`) and pickable solids from the building
//! model + frame plan: wall envelopes, member cuboids, opening pick volumes, and
//! the color helpers shared with the view cube.

use eframe::egui::{Color32, Pos2};
use framer_core::{BuildingModel, Length, Wall};
use framer_solver::{FrameMember, MemberKind, MemberOrientation, ProjectFramePlan};

use super::geom::{OrbitProjector, Point3, point_hits_projected_quad};
use super::gpu::GpuVertex;
use crate::app::{Selection, ViewClick, WorkspaceMode};

// === extracted block appended below; visibility adjusted in place ===

pub(super) struct Scene3d {
    pub(super) vertices: Vec<GpuVertex>,
    pub(super) indices: Vec<u32>,
    pub(super) opaque_index_count: u32,
    pub(super) transparent_index_count: u32,
    pub(super) points: Vec<Point3>,
    pub(super) picks: Vec<PickSolid>,
}

#[derive(Default)]
struct SceneBuilder {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    points: Vec<Point3>,
    picks: Vec<PickSolid>,
    opaque_index_count: u32,
}

struct WallSegmentSpan {
    x0: Length,
    x1: Length,
    z0: Length,
    z1: Length,
}

impl WallSegmentSpan {
    fn new(x0: Length, x1: Length, z0: Length, z1: Length) -> Self {
        Self { x0, x1, z0, z1 }
    }
}

impl Scene3d {
    pub(super) fn from_project(
        model: &BuildingModel,
        plan: &ProjectFramePlan,
        selected_wall: usize,
        selection: &Selection,
        workspace_mode: WorkspaceMode,
    ) -> Option<Self> {
        if model.walls.is_empty() {
            return None;
        }

        let wall_depth = model.code.stud_profile.nominal_depth().inches() as f32;
        let mut builder = SceneBuilder::default();

        if workspace_mode.shows_generated_plan() {
            for (wall_index, wall) in model.walls.iter().enumerate() {
                if let Some(wall_plan) = plan.wall_plan(&wall.id) {
                    let wall_selected = selected_wall == wall_index;
                    for member in &wall_plan.members {
                        let member_selected = matches!(
                            selection,
                            Selection::Member { wall_id, member_id }
                                if wall_id == &wall.id.0 && member_id == &member.id
                        );
                        builder.push_member(
                            wall,
                            member,
                            wall_depth,
                            wall_selected,
                            member_selected,
                        );
                    }
                }
            }
        }

        builder.finish_opaque();

        for (wall_index, wall) in model.walls.iter().enumerate() {
            let wall_selected = selected_wall == wall_index && matches!(selection, Selection::Wall);
            builder.push_wall_envelope(wall, wall_index, wall_depth, wall_selected);
            for opening in &wall.openings {
                builder.push_opening_pick(wall, wall_index, opening.id.0.clone(), wall_depth);
            }
        }

        Some(builder.finish())
    }

    pub(super) fn pick(&self, pointer: Pos2, projector: &OrbitProjector) -> Option<ViewClick> {
        let mut best = None::<(u8, f32, ViewClick)>;
        for solid in &self.picks {
            let Some(depth) = solid.hit_depth(pointer, projector) else {
                continue;
            };
            let replace = best.as_ref().is_none_or(|(priority, best_depth, _)| {
                solid.priority > *priority || (solid.priority == *priority && depth > *best_depth)
            });
            if replace {
                best = Some((solid.priority, depth, solid.click.clone()));
            }
        }
        best.map(|(_, _, click)| click)
    }
}

impl SceneBuilder {
    fn push_member(
        &mut self,
        wall: &Wall,
        member: &FrameMember,
        wall_depth: f32,
        wall_selected: bool,
        selected: bool,
    ) {
        let half_member = member.cross_section_depth.inches() as f32 / 2.0;
        let (x0, x1, z0, z1) = match member.orientation {
            MemberOrientation::Horizontal => (
                member.x.inches() as f32,
                (member.x + member.cut_length).inches() as f32,
                member.elevation.inches() as f32,
                (member.elevation + member.cross_section_depth).inches() as f32,
            ),
            MemberOrientation::Vertical => (
                member.x.inches() as f32 - half_member,
                member.x.inches() as f32 + half_member,
                member.elevation.inches() as f32,
                (member.elevation + member.cut_length).inches() as f32,
            ),
        };
        let color = if selected {
            Color32::from_rgb(49, 116, 178)
        } else if wall_selected {
            brighten(member_color(member.kind), 20)
        } else {
            member_color(member.kind)
        };
        let solid = WallCuboid::new(wall, x0, x1, -wall_depth / 2.0, wall_depth / 2.0, z0, z1);
        self.push_cuboid(&solid, color_to_rgba(color));
        self.picks.push(PickSolid {
            click: ViewClick::Member {
                wall_id: wall.id.0.clone(),
                member_id: member.id.clone(),
            },
            priority: 3,
            corners: solid.corners,
        });
    }

    fn push_wall_envelope(
        &mut self,
        wall: &Wall,
        wall_index: usize,
        wall_depth: f32,
        selected: bool,
    ) {
        let color = if selected {
            Color32::from_rgba_unmultiplied(92, 145, 190, 82)
        } else {
            Color32::from_rgba_unmultiplied(188, 179, 158, 54)
        };
        let mut openings = wall.openings.iter().collect::<Vec<_>>();
        openings.sort_by_key(|opening| opening.left());
        let mut cursor = Length::ZERO;

        for opening in openings {
            self.push_wall_segment(
                wall,
                WallSegmentSpan::new(cursor, opening.left(), Length::ZERO, wall.height),
                wall_depth,
                color,
            );
            if opening.sill_height > Length::ZERO {
                self.push_wall_segment(
                    wall,
                    WallSegmentSpan::new(
                        opening.left(),
                        opening.right(),
                        Length::ZERO,
                        opening.sill_height,
                    ),
                    wall_depth,
                    color,
                );
            }
            if opening.top() < wall.height {
                self.push_wall_segment(
                    wall,
                    WallSegmentSpan::new(
                        opening.left(),
                        opening.right(),
                        opening.top(),
                        wall.height,
                    ),
                    wall_depth,
                    color,
                );
            }
            cursor = opening.right();
        }
        self.push_wall_segment(
            wall,
            WallSegmentSpan::new(cursor, wall.length, Length::ZERO, wall.height),
            wall_depth,
            color,
        );

        let solid = WallCuboid::new(
            wall,
            0.0,
            wall.length.inches() as f32,
            -wall_depth / 2.0,
            wall_depth / 2.0,
            0.0,
            wall.height.inches() as f32,
        );
        self.picks.push(PickSolid {
            click: ViewClick::Wall(wall_index),
            priority: 1,
            corners: solid.corners,
        });
    }

    fn push_wall_segment(
        &mut self,
        wall: &Wall,
        span: WallSegmentSpan,
        wall_depth: f32,
        color: Color32,
    ) {
        if span.x1 <= span.x0 || span.z1 <= span.z0 {
            return;
        }
        let solid = WallCuboid::new(
            wall,
            span.x0.inches() as f32,
            span.x1.inches() as f32,
            -wall_depth / 2.0,
            wall_depth / 2.0,
            span.z0.inches() as f32,
            span.z1.inches() as f32,
        );
        self.push_cuboid(&solid, color_to_rgba(color));
    }

    fn push_opening_pick(
        &mut self,
        wall: &Wall,
        wall_index: usize,
        opening_id: String,
        wall_depth: f32,
    ) {
        let Some(opening) = wall
            .openings
            .iter()
            .find(|candidate| candidate.id.0 == opening_id)
        else {
            return;
        };
        let solid = WallCuboid::new(
            wall,
            opening.left().inches() as f32,
            opening.right().inches() as f32,
            -wall_depth / 2.0,
            wall_depth / 2.0,
            opening.sill_height.inches() as f32,
            opening.top().inches() as f32,
        );
        self.picks.push(PickSolid {
            click: ViewClick::Opening {
                wall_index,
                opening_id,
            },
            priority: 2,
            corners: solid.corners,
        });
    }

    fn push_cuboid(&mut self, cuboid: &WallCuboid, color: [f32; 4]) {
        if cuboid.is_degenerate() {
            return;
        }

        self.points.extend(cuboid.corners);
        for face in cuboid.faces() {
            let base = self.vertices.len() as u32;
            for corner in face.corners {
                let point = cuboid.corners[corner];
                self.vertices.push(GpuVertex {
                    position: [point.x, point.y, point.z],
                    color,
                    normal: [face.normal.x, face.normal.y, face.normal.z],
                });
            }
            self.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    fn finish_opaque(&mut self) {
        self.opaque_index_count = self.indices.len() as u32;
    }

    fn finish(self) -> Scene3d {
        let total_index_count = self.indices.len() as u32;
        Scene3d {
            vertices: self.vertices,
            indices: self.indices,
            opaque_index_count: self.opaque_index_count,
            transparent_index_count: total_index_count - self.opaque_index_count,
            points: self.points,
            picks: self.picks,
        }
    }
}

#[derive(Clone, Copy)]
struct WallCuboid {
    corners: [Point3; 8],
    along: Point3,
    side: Point3,
}

impl WallCuboid {
    #[allow(clippy::too_many_arguments)]
    fn new(wall: &Wall, x0: f32, x1: f32, side0: f32, side1: f32, z0: f32, z1: f32) -> Self {
        let basis = WallBasis::new(wall);
        let corners = [
            basis.point(x0, side0, z0),
            basis.point(x1, side0, z0),
            basis.point(x1, side1, z0),
            basis.point(x0, side1, z0),
            basis.point(x0, side0, z1),
            basis.point(x1, side0, z1),
            basis.point(x1, side1, z1),
            basis.point(x0, side1, z1),
        ];
        Self {
            corners,
            along: Point3::vector(basis.along_x, basis.along_y, 0.0),
            side: Point3::vector(basis.side_x, basis.side_y, 0.0),
        }
    }

    fn is_degenerate(&self) -> bool {
        self.corners[0].distance_squared(self.corners[1]) < f32::EPSILON
            || self.corners[1].distance_squared(self.corners[2]) < f32::EPSILON
            || self.corners[0].distance_squared(self.corners[4]) < f32::EPSILON
    }

    fn faces(&self) -> [CuboidFace; 6] {
        [
            CuboidFace::new([0, 3, 2, 1], -Point3::Z),
            CuboidFace::new([4, 5, 6, 7], Point3::Z),
            CuboidFace::new([0, 1, 5, 4], -self.side),
            CuboidFace::new([1, 2, 6, 5], self.along),
            CuboidFace::new([2, 3, 7, 6], self.side),
            CuboidFace::new([3, 0, 4, 7], -self.along),
        ]
    }
}

#[derive(Clone, Copy)]
struct CuboidFace {
    corners: [usize; 4],
    normal: Point3,
}

impl CuboidFace {
    fn new(corners: [usize; 4], normal: Point3) -> Self {
        Self { corners, normal }
    }
}

pub(super) struct PickSolid {
    pub(super) click: ViewClick,
    priority: u8,
    corners: [Point3; 8],
}

impl PickSolid {
    fn hit_depth(&self, pointer: Pos2, projector: &OrbitProjector) -> Option<f32> {
        let mut best_depth = None::<f32>;
        for face in CUBOID_FACE_INDICES {
            let projected = face.map(|index| projector.project_point(self.corners[index]));
            let positions = projected.map(|point| point.pos);
            if point_hits_projected_quad(pointer, &positions) {
                let depth = projected.iter().map(|point| point.depth).sum::<f32>() / 4.0;
                best_depth = Some(best_depth.map_or(depth, |existing| existing.max(depth)));
            }
        }
        best_depth
    }
}

const CUBOID_FACE_INDICES: [[usize; 4]; 6] = [
    [0, 3, 2, 1],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [3, 0, 4, 7],
];

struct WallBasis {
    origin_x: f32,
    origin_y: f32,
    along_x: f32,
    along_y: f32,
    side_x: f32,
    side_y: f32,
}

impl WallBasis {
    fn new(wall: &Wall) -> Self {
        let dx = (wall.end.x - wall.start.x).inches() as f32;
        let dy = (wall.end.y - wall.start.y).inches() as f32;
        let length = (dx * dx + dy * dy).sqrt().max(1.0);
        let along_x = dx / length;
        let along_y = dy / length;
        Self {
            origin_x: wall.start.x.inches() as f32,
            origin_y: wall.start.y.inches() as f32,
            along_x,
            along_y,
            side_x: -along_y,
            side_y: along_x,
        }
    }

    fn point(&self, local_x: f32, side: f32, z: f32) -> Point3 {
        Point3 {
            x: self.origin_x + self.along_x * local_x + self.side_x * side,
            y: self.origin_y + self.along_y * local_x + self.side_y * side,
            z,
        }
    }
}

pub(super) fn color_to_rgba(color: Color32) -> [f32; 4] {
    [
        color.r() as f32 / 255.0,
        color.g() as f32 / 255.0,
        color.b() as f32 / 255.0,
        color.a() as f32 / 255.0,
    ]
}

pub(super) fn brighten(color: Color32, amount: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(
        color.r().saturating_add(amount),
        color.g().saturating_add(amount),
        color.b().saturating_add(amount),
        color.a(),
    )
}

pub(super) fn member_color(kind: MemberKind) -> Color32 {
    match kind {
        MemberKind::BottomPlate | MemberKind::TopPlate => Color32::from_rgb(99, 85, 67),
        MemberKind::CornerPost => Color32::from_rgb(52, 95, 127),
        MemberKind::PartitionStud => Color32::from_rgb(79, 127, 95),
        MemberKind::BackingStud => Color32::from_rgb(127, 111, 79),
        MemberKind::CommonStud => Color32::from_rgb(186, 145, 94),
        MemberKind::KingStud => Color32::from_rgb(151, 100, 61),
        MemberKind::JackStud => Color32::from_rgb(211, 168, 95),
        MemberKind::Header => Color32::from_rgb(115, 130, 99),
        MemberKind::RoughSill => Color32::from_rgb(92, 121, 144),
        MemberKind::CrippleStud => Color32::from_rgb(218, 190, 139),
    }
}
