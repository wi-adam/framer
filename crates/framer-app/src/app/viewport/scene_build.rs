//! Builds the 3D scene mesh (`Scene3d`) and pickable solids from the building
//! model + frame plan: wall envelopes, member cuboids, opening pick volumes, and
//! the color helpers shared with the view cube.

use std::collections::BTreeMap;

use eframe::egui::{Color32, Pos2};
use framer_core::{BuildingModel, ConstructionSystem, ElementId, Length, Material, Wall};
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

/// One construction-layer band across the wall section: its two face positions on
/// the side axis (inches, span `[side0, side1]` with `side0 <= side1`) and fill
/// color. The interior face may be either end depending on the wall's orientation.
#[derive(Clone, Copy)]
struct LayerBand {
    side0: f32,
    side1: f32,
    color: Color32,
}

impl LayerBand {
    fn new(side0: f32, side1: f32, color: Color32) -> Self {
        Self {
            side0,
            side1,
            color,
        }
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

        // Fall back to the code stud depth only when a wall has no system (a
        // degenerate model); resolved per-wall below.
        let fallback_depth = model.code.stud_profile.nominal_depth();
        // Which side of each wall faces the room interior, derived from topology
        // once per frame. Layers (and members) lay out interior -> exterior from
        // this, so reversing a wall no longer mirrors the assembly.
        let interior_sides = framer_core::wall_interior_sides(model);
        let mut builder = SceneBuilder::default();

        if workspace_mode.shows_generated_plan() {
            for (wall_index, wall) in model.walls.iter().enumerate() {
                let total = wall_total_thickness(model, wall, fallback_depth);
                let sign = interior_sign(&interior_sides, &wall.id);
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
                            total,
                            sign,
                            wall_selected,
                            member_selected,
                        );
                    }
                }
            }
        }

        builder.finish_opaque();

        for (wall_index, wall) in model.walls.iter().enumerate() {
            let total = wall_total_thickness(model, wall, fallback_depth);
            let sign = interior_sign(&interior_sides, &wall.id);
            let wall_selected = selected_wall == wall_index && matches!(selection, Selection::Wall);
            builder.push_wall_envelope(model, wall, wall_index, total, sign, wall_selected);
            for opening in &wall.openings {
                builder.push_opening_pick(wall, wall_index, opening.id.0.clone(), total, sign);
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
        total: Length,
        interior_sign: f32,
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
        // Studs/plates sit inside the framing-layer band: the solver gives the
        // band as [side_offset, side_offset + side_depth] running interior ->
        // exterior, so `interior_sign` decides which way that runs in side space.
        let (side0, side1) =
            layer_band_span(interior_sign, total, member.side_offset, member.side_depth);
        let solid = WallCuboid::new(wall, x0, x1, side0, side1, z0, z1);
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
        model: &BuildingModel,
        wall: &Wall,
        wall_index: usize,
        total: Length,
        interior_sign: f32,
        selected: bool,
    ) {
        match model.system_for(wall) {
            // Render a true layered cross-section: one cuboid per (layer x wall
            // segment), so every layer is cut by every opening. Layers run
            // interior -> exterior; `off` accumulates each layer's through-wall
            // offset from the interior face, and `interior_sign` decides which
            // physical side that interior face sits on.
            Some(system) => {
                let mut off = Length::ZERO;
                for layer in &system.layers {
                    let (side0, side1) =
                        layer_band_span(interior_sign, total, off, layer.thickness);
                    off += layer.thickness;
                    let base = material_color(model, &layer.material);
                    let color = layer_band_color(base, selected);
                    self.push_wall_layer(wall, LayerBand::new(side0, side1, color));
                }
            }
            // Degenerate model with no resolvable system: draw a single band over
            // the full thickness so the wall is still visible.
            None => {
                let base = neutral_band_color();
                let color = layer_band_color(base, selected);
                let (side0, side1) = layer_band_span(interior_sign, total, Length::ZERO, total);
                self.push_wall_layer(wall, LayerBand::new(side0, side1, color));
            }
        }

        // The pick envelope spans the full wall thickness regardless of layering
        // or which side is interior.
        let (env0, env1) = layer_band_span(interior_sign, total, Length::ZERO, total);
        let solid = WallCuboid::new(
            wall,
            0.0,
            wall.length.inches() as f32,
            env0,
            env1,
            0.0,
            wall.height.inches() as f32,
        );
        self.picks.push(PickSolid {
            click: ViewClick::Wall(wall_index),
            priority: 1,
            corners: solid.corners,
        });
    }

    /// Extrude one layer band across the wall face, applying the 4-segment
    /// opening decomposition (clear span left/right + sill apron + header apron)
    /// so the layer is cut by every opening.
    fn push_wall_layer(&mut self, wall: &Wall, band: LayerBand) {
        let mut openings = wall.openings.iter().collect::<Vec<_>>();
        openings.sort_by_key(|opening| opening.left());
        let mut cursor = Length::ZERO;

        for opening in openings {
            self.push_wall_segment(
                wall,
                WallSegmentSpan::new(cursor, opening.left(), Length::ZERO, wall.height),
                band,
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
                    band,
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
                    band,
                );
            }
            cursor = opening.right();
        }
        self.push_wall_segment(
            wall,
            WallSegmentSpan::new(cursor, wall.length, Length::ZERO, wall.height),
            band,
        );
    }

    fn push_wall_segment(&mut self, wall: &Wall, span: WallSegmentSpan, band: LayerBand) {
        if span.x1 <= span.x0 || span.z1 <= span.z0 {
            return;
        }
        let solid = WallCuboid::new(
            wall,
            span.x0.inches() as f32,
            span.x1.inches() as f32,
            band.side0,
            band.side1,
            span.z0.inches() as f32,
            span.z1.inches() as f32,
        );
        self.push_cuboid(&solid, color_to_rgba(band.color));
    }

    fn push_opening_pick(
        &mut self,
        wall: &Wall,
        wall_index: usize,
        opening_id: String,
        total: Length,
        interior_sign: f32,
    ) {
        let Some(opening) = wall
            .openings
            .iter()
            .find(|candidate| candidate.id.0 == opening_id)
        else {
            return;
        };
        // Openings span the full thickness regardless of which side is interior.
        let (side0, side1) = layer_band_span(interior_sign, total, Length::ZERO, total);
        let solid = WallCuboid::new(
            wall,
            opening.left().inches() as f32,
            opening.right().inches() as f32,
            side0,
            side1,
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

/// Layer bands render in the transparent (non-depth-writing) pass so framing
/// members stay visible inside the wall; this alpha keeps the layered
/// cross-section legible while letting studs show through.
const LAYER_BAND_ALPHA: u8 = 168;

/// A material's representative appearance color as a translucent `Color32` for a
/// layer band, so the colored cross-section reads while framing members inside
/// the wall remain visible through it.
pub(super) fn material_color_to_rgba(material: &Material) -> Color32 {
    let [r, g, b] = material.color();
    Color32::from_rgba_unmultiplied(r, g, b, LAYER_BAND_ALPHA)
}

/// Which way a wall's layer stack runs on the side axis (`(-along_y, along_x)`):
/// `+1` when the room interior is toward the plus-side, `-1` when toward the
/// minus-side. Walls absent from the topology map (ambiguous partitions / no
/// enclosing room) DEFAULT to `-1` so their assembly stays stable.
fn interior_sign(interior_sides: &BTreeMap<ElementId, bool>, wall_id: &ElementId) -> f32 {
    match interior_sides.get(wall_id) {
        Some(true) => 1.0,
        _ => -1.0,
    }
}

/// The side-axis span `[min, max]` of one layer band, laid out interior ->
/// exterior. With cumulative interior offset `off` and thickness `t` the band's
/// interior face is at `interior_sign * (total/2 - off)` and its exterior face at
/// `interior_sign * (total/2 - (off + t))`. Flipping `interior_sign` mirrors the
/// whole stack across the centerline, so reversing a wall keeps each layer on the
/// room side it belongs to. The cross-section keeps spanning the full `total`.
fn layer_band_span(interior_sign: f32, total: Length, off: Length, t: Length) -> (f32, f32) {
    let half = total.inches() as f32 / 2.0;
    let off = off.inches() as f32;
    let t = t.inches() as f32;
    let side_a = interior_sign * (half - off);
    let side_b = interior_sign * (half - (off + t));
    (side_a.min(side_b), side_a.max(side_b))
}

/// The total through-wall thickness of a wall's construction system, falling back
/// to the code stud depth for a wall with no resolvable system.
fn wall_total_thickness(model: &BuildingModel, wall: &Wall, fallback: Length) -> Length {
    model
        .system_for(wall)
        .map(ConstructionSystem::total_thickness)
        .unwrap_or(fallback)
}

/// The fill color for a layer band: the resolved material color, brightened a
/// touch when the wall is selected.
fn layer_band_color(base: Color32, selected: bool) -> Color32 {
    if selected { brighten(base, 24) } else { base }
}

/// Resolve a layer material's color, falling back to a neutral tone when the
/// material id is dangling.
fn material_color(model: &BuildingModel, id: &framer_core::ElementId) -> Color32 {
    model
        .material(id)
        .map(material_color_to_rgba)
        .unwrap_or_else(neutral_band_color)
}

/// The neutral fallback band color (translucent) used when a layer or wall has no
/// resolvable material/system.
fn neutral_band_color() -> Color32 {
    Color32::from_rgba_unmultiplied(188, 179, 158, LAYER_BAND_ALPHA)
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
