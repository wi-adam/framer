//! Wall envelopes, construction layers, openings, and wall-local primitives.

use std::collections::BTreeMap;

use eframe::egui::Color32;
use framer_core::{BuildingModel, ConstructionSystem, ElementId, GableWallProfile, Length, Wall};

use super::super::gpu::GpuVertex;
use super::style::{layer_band_color, material_color, neutral_band_color};
use super::{
    GABLE_RENDER_QUAD_FACES, GABLE_TRIANGLE_FACES, OutlineEdge, PickSolid, Point3, SceneBuilder,
    color_to_rgba,
};
use crate::app::{ViewClick, WallDisplay};

struct WallSegmentSpan {
    x0: f32,
    x1: f32,
    z0: Length,
    z1: Length,
}

impl WallSegmentSpan {
    fn new(x0: f32, x1: f32, z0: Length, z1: Length) -> Self {
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

impl SceneBuilder {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_wall_envelope(
        &mut self,
        model: &BuildingModel,
        wall: &Wall,
        wall_index: usize,
        total: Length,
        interior_sign: f32,
        base_elevation: f32,
        gable: Option<&GableWallProfile>,
        selected: bool,
        wall_display: WallDisplay,
    ) {
        // The full-thickness span and envelope box, shared by Outline (its edges)
        // and the pick volume below, so the box is built once per wall.
        let (env0, env1) = layer_band_span(interior_sign, total, Length::ZERO, total);
        let (visual_x0, visual_x1) = model.wall_envelope_span(wall);
        let envelope = WallCuboid::new(
            wall,
            visual_x0 as f32,
            visual_x1 as f32,
            env0,
            env1,
            base_elevation,
            base_elevation + wall.height.inches() as f32,
        );
        match wall_display {
            // Render a true layered cross-section: one cuboid per (layer x wall
            // segment), so every layer is cut by every opening. Layers run
            // interior -> exterior; `off` accumulates each layer's through-wall
            // offset from the interior face, and `interior_sign` decides which
            // physical side that interior face sits on.
            WallDisplay::Full => match model.system_for(wall) {
                Some(system) => {
                    let mut off = Length::ZERO;
                    for layer in &system.layers {
                        let (side0, side1) =
                            layer_band_span(interior_sign, total, off, layer.thickness);
                        off += layer.thickness;
                        let base = material_color(model, &layer.material);
                        let color = layer_band_color(base, selected);
                        self.push_wall_layer(
                            wall,
                            LayerBand::new(side0, side1, color),
                            visual_x0,
                            visual_x1,
                            base_elevation,
                        );
                        if let Some(gable) = gable {
                            self.push_gable_layer(wall, gable, side0, side1, color);
                        }
                    }
                }
                // Degenerate model with no resolvable system: draw a single band
                // over the full thickness so the wall is still visible.
                None => {
                    let color = layer_band_color(neutral_band_color(), selected);
                    self.push_wall_layer(
                        wall,
                        LayerBand::new(env0, env1, color),
                        visual_x0,
                        visual_x1,
                        base_elevation,
                    );
                    if let Some(gable) = gable {
                        self.push_gable_layer(wall, gable, env0, env1, color);
                    }
                }
            },
            // One monochrome full-thickness band: the wall reads as a solid volume
            // without the per-layer colors. Openings still cut it (push_wall_layer
            // does the 4-segment decomposition).
            WallDisplay::Width => {
                let color = layer_band_color(neutral_band_color(), selected);
                self.push_wall_layer(
                    wall,
                    LayerBand::new(env0, env1, color),
                    visual_x0,
                    visual_x1,
                    base_elevation,
                );
                if let Some(gable) = gable {
                    self.push_gable_layer(wall, gable, env0, env1, color);
                }
            }
            // No fill triangles: collect the envelope's 12 edges for the painter
            // overlay (and feed its corners into `points` so the projector still
            // frames the scene when nothing fills it).
            WallDisplay::Outline => {
                self.push_wall_outline(&envelope, selected);
                if let Some(gable) = gable {
                    self.push_gable_outline(wall, gable, env0, env1, selected);
                }
            }
        }

        // The pick envelope spans the full wall thickness regardless of layering,
        // mode, or which side is interior — so walls stay clickable in every mode.
        self.picks.push(PickSolid::cuboid(
            ViewClick::Wall(wall_index),
            1,
            envelope.corners,
        ));
        if let Some(gable) = gable {
            let prism = GablePrism::new(wall, gable, env0, env1);
            self.picks.push(PickSolid::gable_prism(
                ViewClick::Wall(wall_index),
                1,
                prism.corners,
            ));
        }
    }

    /// Collect the 12 edges of the wall's full-thickness envelope for the
    /// [`WallDisplay::Outline`] painter overlay. Produces no fill geometry, so the
    /// corners are also fed into `points` to keep the orbit projector framed.
    fn push_wall_outline(&mut self, envelope: &WallCuboid, selected: bool) {
        if envelope.is_degenerate() {
            return;
        }
        self.points.extend(envelope.corners);
        for [a, b] in WallCuboid::EDGES {
            self.outline_edges.push(OutlineEdge {
                a: envelope.corners[a],
                b: envelope.corners[b],
                selected,
            });
        }
    }

    /// Add the triangular gable prism to Outline mode. The rectangle below keeps
    /// its ordinary 12 edges; these nine edges close the two triangular faces and
    /// connect them through the same full wall-system thickness.
    fn push_gable_outline(
        &mut self,
        wall: &Wall,
        profile: &GableWallProfile,
        side0: f32,
        side1: f32,
        selected: bool,
    ) {
        let prism = GablePrism::new(wall, profile, side0, side1);
        self.points.extend(prism.corners);
        for [a, b] in GablePrism::EDGES {
            self.outline_edges.push(OutlineEdge {
                a: prism.corners[a],
                b: prism.corners[b],
                selected,
            });
        }
    }

    /// Extrude one wall-system layer through the derived triangular gable profile.
    /// Authored openings remain in the rectangular wall below. When one reaches
    /// the wall top, cap just that exposed part of the shared gable base.
    pub(super) fn push_gable_layer(
        &mut self,
        wall: &Wall,
        profile: &GableWallProfile,
        side0: f32,
        side1: f32,
        color: Color32,
    ) {
        let prism = GablePrism::new(wall, profile, side0, side1);
        let color = color_to_rgba(color);
        self.push_gable_prism(&prism, color);
        let basis = WallBasis::new(wall);
        let z = profile.base_elevation.inches() as f32;
        for opening in wall
            .openings
            .iter()
            .filter(|opening| opening.top() == wall.height)
        {
            let cap = [
                basis.point(opening.left().inches() as f32, side0, z),
                basis.point(opening.left().inches() as f32, side1, z),
                basis.point(opening.right().inches() as f32, side1, z),
                basis.point(opening.right().inches() as f32, side0, z),
            ];
            self.points.extend(cap);
            self.push_quad(cap, color);
        }
    }

    /// Mesh a roof member from the solver's exact spatial endpoints. The profile
    /// thickness lies horizontally across the member; its nominal depth lies in
    /// the orthogonal in-plane section axis.
    fn push_wall_layer(
        &mut self,
        wall: &Wall,
        band: LayerBand,
        visual_x0: f64,
        visual_x1: f64,
        base_elevation: f32,
    ) {
        let mut openings = wall.openings.iter().collect::<Vec<_>>();
        openings.sort_by_key(|opening| opening.left());
        let mut cursor = visual_x0 as f32;

        for opening in openings {
            self.push_wall_segment(
                wall,
                WallSegmentSpan::new(
                    cursor,
                    opening.left().inches() as f32,
                    Length::ZERO,
                    wall.height,
                ),
                band,
                base_elevation,
            );
            if opening.sill_height > Length::ZERO {
                self.push_wall_segment(
                    wall,
                    WallSegmentSpan::new(
                        opening.left().inches() as f32,
                        opening.right().inches() as f32,
                        Length::ZERO,
                        opening.sill_height,
                    ),
                    band,
                    base_elevation,
                );
            }
            if opening.top() < wall.height {
                self.push_wall_segment(
                    wall,
                    WallSegmentSpan::new(
                        opening.left().inches() as f32,
                        opening.right().inches() as f32,
                        opening.top(),
                        wall.height,
                    ),
                    band,
                    base_elevation,
                );
            }
            cursor = opening.right().inches() as f32;
        }
        self.push_wall_segment(
            wall,
            WallSegmentSpan::new(cursor, visual_x1 as f32, Length::ZERO, wall.height),
            band,
            base_elevation,
        );
    }

    fn push_wall_segment(
        &mut self,
        wall: &Wall,
        span: WallSegmentSpan,
        band: LayerBand,
        base_elevation: f32,
    ) {
        if span.x1 <= span.x0 || span.z1 <= span.z0 {
            return;
        }
        let solid = WallCuboid::new(
            wall,
            span.x0,
            span.x1,
            band.side0,
            band.side1,
            base_elevation + span.z0.inches() as f32,
            base_elevation + span.z1.inches() as f32,
        );
        self.push_cuboid(&solid, color_to_rgba(band.color));
    }

    pub(super) fn push_opening_pick(
        &mut self,
        wall: &Wall,
        wall_index: usize,
        opening_id: String,
        total: Length,
        interior_sign: f32,
        base_elevation: f32,
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
            base_elevation + opening.sill_height.inches() as f32,
            base_elevation + opening.top().inches() as f32,
        );
        self.picks.push(PickSolid::cuboid(
            ViewClick::Opening {
                wall_index,
                opening_id,
            },
            2,
            solid.corners,
        ));
    }

    pub(super) fn push_cuboid(&mut self, cuboid: &WallCuboid, color: [f32; 4]) {
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

    fn push_gable_prism(&mut self, prism: &GablePrism, color: [f32; 4]) {
        self.points.extend(prism.corners);
        for face in GABLE_TRIANGLE_FACES {
            self.push_triangle(face.map(|index| prism.corners[index]), color);
        }
        // The base closes against the rectangular wall below; omitting that
        // coincident face avoids z-fighting and double alpha blending.
        for face in GABLE_RENDER_QUAD_FACES {
            self.push_quad(face.map(|index| prism.corners[index]), color);
        }
    }
}

/// The triangular solid between one authored wall top and its matched roof rakes,
/// extruded through either one wall-system layer or the whole envelope thickness.
struct GablePrism {
    corners: [Point3; 6],
}

impl GablePrism {
    fn new(wall: &Wall, profile: &GableWallProfile, side0: f32, side1: f32) -> Self {
        let basis = WallBasis::new(wall);
        let left_z = profile.base_elevation.inches() as f32;
        let peak_z = profile.peak_elevation.inches() as f32;
        let width = profile.width.inches() as f32;
        let peak_x = profile.peak_x.inches() as f32;
        Self {
            corners: [
                basis.point(0.0, side0, left_z),
                basis.point(width, side0, left_z),
                basis.point(peak_x, side0, peak_z),
                basis.point(0.0, side1, left_z),
                basis.point(width, side1, left_z),
                basis.point(peak_x, side1, peak_z),
            ],
        }
    }

    const EDGES: [[usize; 2]; 9] = [
        [0, 1],
        [1, 2],
        [2, 0],
        [3, 4],
        [4, 5],
        [5, 3],
        [0, 3],
        [1, 4],
        [2, 5],
    ];
}

pub(super) struct WallCuboid {
    pub(super) corners: [Point3; 8],
    along: Point3,
    side: Point3,
}

impl WallCuboid {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        wall: &Wall,
        x0: f32,
        x1: f32,
        side0: f32,
        side1: f32,
        z0: f32,
        z1: f32,
    ) -> Self {
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

    /// The 12 edges of the box as `corners` index pairs: bottom quad, top quad,
    /// and the four verticals. Matches the corner winding in [`Self::new`].
    const EDGES: [[usize; 2]; 12] = [
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
    ];
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

pub(super) struct WallBasis {
    pub(super) origin_x: f32,
    pub(super) origin_y: f32,
    along_x: f32,
    along_y: f32,
    pub(super) side_x: f32,
    pub(super) side_y: f32,
}

impl WallBasis {
    pub(super) fn new(wall: &Wall) -> Self {
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

    pub(super) fn point(&self, local_x: f32, side: f32, z: f32) -> Point3 {
        Point3 {
            x: self.origin_x + self.along_x * local_x + self.side_x * side,
            y: self.origin_y + self.along_y * local_x + self.side_y * side,
            z,
        }
    }
}

/// Which way a wall's layer stack runs on the side axis (`(-along_y, along_x)`):
/// `+1` when the room interior is toward the plus-side, `-1` when toward the
/// minus-side. Walls absent from the topology map (ambiguous partitions / no
/// enclosing room) default to `-1` so their assembly stays stable.
pub(super) fn interior_sign(
    interior_sides: &BTreeMap<ElementId, bool>,
    wall_id: &ElementId,
) -> f32 {
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
pub(super) fn layer_band_span(
    interior_sign: f32,
    total: Length,
    off: Length,
    t: Length,
) -> (f32, f32) {
    let half = total.inches() as f32 / 2.0;
    let off = off.inches() as f32;
    let t = t.inches() as f32;
    let side_a = interior_sign * (half - off);
    let side_b = interior_sign * (half - (off + t));
    (side_a.min(side_b), side_a.max(side_b))
}

/// The total through-wall thickness of a wall's construction system, falling back
/// to the code stud depth for a wall with no resolvable system.
pub(super) fn wall_total_thickness(model: &BuildingModel, wall: &Wall, fallback: Length) -> Length {
    model
        .system_for(wall)
        .map(ConstructionSystem::total_thickness)
        .unwrap_or(fallback)
}
