//! Builds the 3D scene mesh (`Scene3d`) and pickable solids from the building
//! model + frame plan: wall envelopes, member cuboids, opening pick volumes, and
//! the color helpers shared with the view cube.

use std::collections::BTreeMap;

use eframe::egui::{Color32, Pos2};
use framer_core::{
    AssemblyFace, BuildingModel, ConstructionSystem, ElementId, Length, Material, Point2,
    RoofPlane, SurfaceRegion, Wall,
};
use framer_solver::{FrameMember, MemberKind, MemberOrientation, ProjectFramePlan};

use super::geom::{OrbitProjector, Point3, point_hits_projected_quad, point_in_polygon};
use super::gpu::GpuVertex;
use crate::app::{Selection, ViewClick, WallDisplay, WorkspaceMode};

// === extracted block appended below; visibility adjusted in place ===

pub(super) struct Scene3d {
    pub(super) vertices: Vec<GpuVertex>,
    pub(super) indices: Vec<u32>,
    pub(super) opaque_index_count: u32,
    pub(super) transparent_index_count: u32,
    pub(super) points: Vec<Point3>,
    pub(super) picks: Vec<PickSolid>,
    /// Wall envelope edges for [`WallDisplay::Outline`], projected + drawn as a
    /// painter overlay by the axonometric view (the wgpu pipeline is triangle-only,
    /// so there is no GPU wireframe). Empty in the Width/Full modes.
    pub(super) outline_edges: Vec<OutlineEdge>,
}

/// One wall-outline edge in world space, with whether its wall is selected so the
/// overlay can highlight it. See [`Scene3d::outline_edges`].
#[derive(Clone, Copy)]
pub(super) struct OutlineEdge {
    pub(super) a: Point3,
    pub(super) b: Point3,
    pub(super) selected: bool,
}

#[derive(Default)]
struct SceneBuilder {
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
    points: Vec<Point3>,
    picks: Vec<PickSolid>,
    outline_edges: Vec<OutlineEdge>,
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
        wall_display: WallDisplay,
    ) -> Option<Self> {
        if model.walls.is_empty()
            && model.roof_planes.is_empty()
            && model.ceilings.is_empty()
            && model.floor_decks.is_empty()
        {
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

        // Authored roof planes, flat ceilings, and floor decks: opaque surface
        // slabs that frame and pick like wall envelopes, shown in every workspace
        // mode (the model carries them whether or not the generated plan is shown).
        // Each outline is ear-clipped once (concave room loops included) and the
        // resulting triangulation drives the lifted/sloped vertices.
        // Cathedral classification for every plane in one wall-graph pass, rather
        // than re-resolving each Room ceiling per plane on every repaint.
        let cathedral = model.roof_cathedral_flags();
        for (index, plane) in model.roof_planes.iter().enumerate() {
            let Some(verts) = roof_plane_outline_world(plane) else {
                continue;
            };
            let triangles = framer_core::triangulate_simple_polygon(&plane.outline);
            let color = surface_color(model, &plane.system, SurfaceFace::Roof);
            // A cathedral plane (no ceiling below) shows the assembly's interior
            // finish on its underside, dropped one assembly-thickness clear.
            let underside = cathedral.get(index).copied().unwrap_or(false).then(|| {
                let under = surface_color(model, &plane.system, SurfaceFace::RoofUnderside);
                (under, roof_assembly_drop(model, &plane.system))
            });
            let selected = matches!(selection, Selection::RoofPlane(id) if id == &plane.id.0);
            builder.push_surface(
                &verts,
                &triangles,
                color,
                underside,
                ViewClick::RoofPlane {
                    id: plane.id.0.clone(),
                },
                selected,
            );
        }
        for ceiling in &model.ceilings {
            let Some(plan) = region_outline_plan(model, &ceiling.region) else {
                continue;
            };
            // The ceiling's low-edge building elevation: it hangs `height` below the
            // level top. A sloped (scissor/vault) ceiling lifts each plan vertex onto
            // its sloped plane via the shared frame, exactly like a roof plane; a flat
            // ceiling stays at a constant elevation.
            let reference_elevation = model
                .levels
                .iter()
                .find(|level| level.id == ceiling.level)
                .map(|level| level.elevation + level.height - ceiling.height)
                .unwrap_or(Length::ZERO);
            let verts = match ceiling.frame(reference_elevation) {
                Some(frame) => plan
                    .iter()
                    .map(|p| {
                        let (x, y) = (p.x.inches(), p.y.inches());
                        Point3::vector(x as f32, y as f32, frame.elevation_at(x, y) as f32)
                    })
                    .collect(),
                None => lift_outline(&plan, reference_elevation.inches() as f32),
            };
            let triangles = framer_core::triangulate_simple_polygon(&plan);
            let color = surface_color(model, &ceiling.system, SurfaceFace::Ceiling);
            let selected = matches!(selection, Selection::Ceiling(id) if id == &ceiling.id.0);
            builder.push_surface(
                &verts,
                &triangles,
                color,
                None,
                ViewClick::Ceiling {
                    id: ceiling.id.0.clone(),
                },
                selected,
            );
        }
        for deck in &model.floor_decks {
            let z = level_elevation(model, &deck.level);
            let Some(plan) = region_outline_plan(model, &deck.region) else {
                continue;
            };
            let verts = lift_outline(&plan, z);
            let triangles = framer_core::triangulate_simple_polygon(&plan);
            let color = surface_color(model, &deck.system, SurfaceFace::Floor);
            let selected = matches!(selection, Selection::FloorDeck(id) if id == &deck.id.0);
            builder.push_surface(
                &verts,
                &triangles,
                color,
                None,
                ViewClick::FloorDeck {
                    id: deck.id.0.clone(),
                },
                selected,
            );
        }

        builder.finish_opaque();

        for (wall_index, wall) in model.walls.iter().enumerate() {
            let total = wall_total_thickness(model, wall, fallback_depth);
            let sign = interior_sign(&interior_sides, &wall.id);
            let wall_selected = selected_wall == wall_index && matches!(selection, Selection::Wall);
            builder.push_wall_envelope(
                model,
                wall,
                wall_index,
                total,
                sign,
                wall_selected,
                wall_display,
            );
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
        self.picks.push(PickSolid::cuboid(
            ViewClick::Member {
                wall_id: wall.id.0.clone(),
                member_id: member.id.clone(),
            },
            3,
            solid.corners,
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn push_wall_envelope(
        &mut self,
        model: &BuildingModel,
        wall: &Wall,
        wall_index: usize,
        total: Length,
        interior_sign: f32,
        selected: bool,
        wall_display: WallDisplay,
    ) {
        // The full-thickness span and envelope box, shared by Outline (its edges)
        // and the pick volume below, so the box is built once per wall.
        let (env0, env1) = layer_band_span(interior_sign, total, Length::ZERO, total);
        let envelope = WallCuboid::new(
            wall,
            0.0,
            wall.length.inches() as f32,
            env0,
            env1,
            0.0,
            wall.height.inches() as f32,
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
                        self.push_wall_layer(wall, LayerBand::new(side0, side1, color));
                    }
                }
                // Degenerate model with no resolvable system: draw a single band
                // over the full thickness so the wall is still visible.
                None => {
                    let color = layer_band_color(neutral_band_color(), selected);
                    self.push_wall_layer(wall, LayerBand::new(env0, env1, color));
                }
            },
            // One monochrome full-thickness band: the wall reads as a solid volume
            // without the per-layer colors. Openings still cut it (push_wall_layer
            // does the 4-segment decomposition).
            WallDisplay::Width => {
                let color = layer_band_color(neutral_band_color(), selected);
                self.push_wall_layer(wall, LayerBand::new(env0, env1, color));
            }
            // No fill triangles: collect the envelope's 12 edges for the painter
            // overlay (and feed its corners into `points` so the projector still
            // frames the scene when nothing fills it).
            WallDisplay::Outline => self.push_wall_outline(&envelope, selected),
        }

        // The pick envelope spans the full wall thickness regardless of layering,
        // mode, or which side is interior — so walls stay clickable in every mode.
        self.picks.push(PickSolid::cuboid(
            ViewClick::Wall(wall_index),
            1,
            envelope.corners,
        ));
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
        self.picks.push(PickSolid::cuboid(
            ViewClick::Opening {
                wall_index,
                opening_id,
            },
            2,
            solid.corners,
        ));
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

    /// Push one flat surface (a roof plane / flat ceiling / floor deck) from its
    /// world-space outline polygon: an opaque double-faced sheet (top + bottom,
    /// opposite normals — the axonometric pipeline does not cull, so both sides
    /// light correctly), feeding the orbit framing and a polygon pick volume. The
    /// outline lies on the same plane the path-traced render uses, so the two views
    /// agree. Brightened when selected, like a wall envelope.
    fn push_surface(
        &mut self,
        outline: &[Point3],
        triangles: &[[usize; 3]],
        color: Color32,
        underside: Option<(Color32, f32)>,
        click: ViewClick,
        selected: bool,
    ) {
        if outline.len() < 3 {
            return;
        }
        let shade = |c: Color32| color_to_rgba(if selected { brighten(c, 30) } else { c });
        // Both faces share the same triangulation; the normal (uniform per face)
        // decides which way each lights, so winding need not be reversed.
        let up = polygon_normal(outline);
        self.push_face(outline, triangles, up, shade(color));
        match underside {
            // A cathedral roof underside: a distinct interior finish, dropped one
            // assembly-thickness below the weather face (backface culling is off, so
            // coincident faces of different colors would z-fight).
            Some((under_color, drop)) => {
                let lowered: Vec<Point3> = outline
                    .iter()
                    .map(|p| Point3::vector(p.x, p.y, p.z - drop))
                    .collect();
                self.push_face(&lowered, triangles, -up, shade(under_color));
                self.points.extend_from_slice(&lowered);
            }
            // A flat surface (or a roof with a ceiling below): both faces share one
            // color, so the coincident back face has no z-fight to resolve.
            None => self.push_face(outline, triangles, -up, shade(color)),
        }
        self.points.extend_from_slice(outline);
        self.picks.push(PickSolid::surface(
            click,
            SURFACE_PICK_PRIORITY,
            outline.to_vec(),
        ));
    }

    /// Emit one face of a surface from precomputed triangle index triples (an
    /// ear-clip from `triangulate_simple_polygon`, correct for concave outlines),
    /// all sharing a single flat `normal`.
    fn push_face(
        &mut self,
        verts: &[Point3],
        triangles: &[[usize; 3]],
        normal: Point3,
        color: [f32; 4],
    ) {
        for &[ia, ib, ic] in triangles {
            let base = self.vertices.len() as u32;
            for &index in &[ia, ib, ic] {
                let point = verts[index];
                self.vertices.push(GpuVertex {
                    position: [point.x, point.y, point.z],
                    color,
                    normal: [normal.x, normal.y, normal.z],
                });
            }
            self.indices.extend_from_slice(&[base, base + 1, base + 2]);
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
            outline_edges: self.outline_edges,
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

pub(super) struct PickSolid {
    pub(super) click: ViewClick,
    priority: u8,
    shape: PickShape,
}

/// The hit-test geometry of a pickable solid. Walls/members/openings are boxes
/// (hit face-by-face); roof/ceiling/floor surfaces are thin slabs hit-tested
/// against their projected outline polygon (which need not be a quad).
enum PickShape {
    Cuboid([Point3; 8]),
    Surface(Vec<Point3>),
}

impl PickSolid {
    fn cuboid(click: ViewClick, priority: u8, corners: [Point3; 8]) -> Self {
        Self {
            click,
            priority,
            shape: PickShape::Cuboid(corners),
        }
    }

    fn surface(click: ViewClick, priority: u8, outline: Vec<Point3>) -> Self {
        Self {
            click,
            priority,
            shape: PickShape::Surface(outline),
        }
    }

    fn hit_depth(&self, pointer: Pos2, projector: &OrbitProjector) -> Option<f32> {
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
        }
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
        MemberKind::FloorJoist => Color32::from_rgb(156, 123, 79),
        MemberKind::CeilingJoist => Color32::from_rgb(127, 156, 143),
        MemberKind::RimJoist => Color32::from_rgb(111, 85, 53),
        MemberKind::Blocking => Color32::from_rgb(181, 154, 106),
        MemberKind::Rafter => Color32::from_rgb(138, 111, 74),
        MemberKind::RidgeBoard => Color32::from_rgb(93, 74, 50),
        MemberKind::HipRafter => Color32::from_rgb(127, 104, 72),
        MemberKind::ValleyRafter => Color32::from_rgb(114, 95, 127),
        MemberKind::JackRafter => Color32::from_rgb(168, 132, 79),
    }
}

// === roof / ceiling / floor surfaces ===

/// Surfaces share the wall envelope's pick priority: a roof rarely overlaps a wall
/// in screen space, and ties fall back to depth (the nearer solid wins).
const SURFACE_PICK_PRIORITY: u8 = 1;

/// Which finished face of a surface assembly is shown, so it picks the layer the
/// viewer sees and a sensible fallback color.
#[derive(Clone, Copy)]
enum SurfaceFace {
    Roof,
    /// A cathedral roof plane's underside — the assembly's conditioned-side finish.
    RoofUnderside,
    Ceiling,
    Floor,
}

/// The fill color of a roof/ceiling/floor surface: the resolved color of its
/// system's representative finish face (the layer selection lives in `framer-core`
/// so this 3-D view and the path-traced render pick the same face), falling back to
/// a neutral tone so it stays visible when the system or material is missing.
fn surface_color(model: &BuildingModel, system_id: &ElementId, face: SurfaceFace) -> Color32 {
    let (fallback, assembly_face) = match face {
        SurfaceFace::Roof => (Color32::from_rgb(96, 99, 107), AssemblyFace::Finished),
        SurfaceFace::RoofUnderside => (Color32::from_rgb(226, 226, 222), AssemblyFace::Underside),
        SurfaceFace::Ceiling => (Color32::from_rgb(226, 226, 222), AssemblyFace::Finished),
        SurfaceFace::Floor => (Color32::from_rgb(150, 120, 86), AssemblyFace::Finished),
    };
    model
        .systems
        .iter()
        .find(|system| system.id == *system_id)
        .and_then(|system| system.surface_finish_material(assembly_face))
        .and_then(|material| model.material(material))
        .map(|material| {
            let [r, g, b] = material.color();
            Color32::from_rgb(r, g, b)
        })
        .unwrap_or(fallback)
}

/// Vertical drop from a roof plane's structural face to its conditioned-side
/// underside: the assembly's through-thickness (a default when the system is
/// missing). Separates the two coplanar faces so, with backface culling off, the
/// distinctly colored underside reads from inside while the weather face reads
/// from outside.
fn roof_assembly_drop(model: &BuildingModel, system_id: &ElementId) -> f32 {
    /// Nominal drop when no system resolves a real thickness (≈ a 2×6 roof).
    const DEFAULT_DROP_IN: f32 = 6.0;
    model
        .systems
        .iter()
        .find(|system| system.id == *system_id)
        .map(|system| system.total_thickness().inches() as f32)
        .filter(|drop| *drop > 0.0)
        .unwrap_or(DEFAULT_DROP_IN)
}

/// A level's floor elevation (inches), or 0 when the level is missing.
fn level_elevation(model: &BuildingModel, level_id: &ElementId) -> f32 {
    model
        .levels
        .iter()
        .find(|level| level.id == *level_id)
        .map(|level| level.elevation.inches() as f32)
        .unwrap_or(0.0)
}

/// A ceiling/floor-deck region's closed plan outline. `Room` regions resolve
/// through the wall graph (mirroring the solver), so the drawn surface tracks the
/// same enclosed face the joists frame; an unknown room or an open (mid-edit) loop
/// yields `None` and the surface is simply skipped.
fn region_outline_plan(model: &BuildingModel, region: &SurfaceRegion) -> Option<Vec<Point2>> {
    let outline = match region {
        SurfaceRegion::Polygon(points) => points.clone(),
        SurfaceRegion::Room(room_id) => {
            let room = model.rooms.iter().find(|room| room.id == *room_id)?;
            framer_core::room_boundary_on_level(model, &room.level, room.seed)?.vertices
        }
    };
    (outline.len() >= 3).then_some(outline)
}

/// Lift a plan outline to constant elevation `z` (a flat ceiling/floor surface).
fn lift_outline(outline: &[Point2], z: f32) -> Vec<Point3> {
    outline
        .iter()
        .map(|point| Point3::vector(point.x.inches() as f32, point.y.inches() as f32, z))
        .collect()
}

/// A roof plane's plan outline lifted onto its sloped plane via the shared
/// [`framer_core::RoofPlaneFrame`] — the same affine elevation field the solver's
/// framing and the path-traced render use, so the slab lies on exactly the plane
/// the rafters frame. `None` for a degenerate outline (no eave length).
fn roof_plane_outline_world(plane: &RoofPlane) -> Option<Vec<Point3>> {
    let frame = plane.frame()?;
    Some(
        plane
            .outline
            .iter()
            .map(|p| {
                let (x, y) = (p.x.inches(), p.y.inches());
                Point3::vector(x as f32, y as f32, frame.elevation_at(x, y) as f32)
            })
            .collect(),
    )
}

/// The unit normal of a planar polygon (Newell's method), oriented upward (+z) so
/// a surface's top face faces the sky. Falls back to +Z for a degenerate polygon.
fn polygon_normal(verts: &[Point3]) -> Point3 {
    let n = verts.len();
    let (mut nx, mut ny, mut nz) = (0.0_f32, 0.0_f32, 0.0_f32);
    for i in 0..n {
        let a = verts[i];
        let b = verts[(i + 1) % n];
        nx += (a.y - b.y) * (a.z + b.z);
        ny += (a.z - b.z) * (a.x + b.x);
        nz += (a.x - b.x) * (a.y + b.y);
    }
    let length = (nx * nx + ny * ny + nz * nz).sqrt();
    if length <= f32::EPSILON {
        return Point3::Z;
    }
    let sign = if nz < 0.0 { -1.0 } else { 1.0 };
    Point3::vector(sign * nx / length, sign * ny / length, sign * nz / length)
}

#[cfg(test)]
mod surface_tests {
    use super::*;
    use eframe::egui::Rect;
    use framer_core::{
        BoardProfile, Ceiling, CodeProfile, ConstructionLayer, FloorDeck, FramingPattern,
        FramingSpec, LayerFunction, Level, MemberFamily, Point2, Room, RoomUsage, Slope,
        SystemKind,
    };
    use framer_solver::ProjectFramePlan;

    use crate::app::viewport::camera_3d::View3dState;

    fn empty_plan() -> ProjectFramePlan {
        ProjectFramePlan {
            wall_plans: Vec::new(),
            floor_plans: Vec::new(),
            ceiling_plans: Vec::new(),
            roof_plans: Vec::new(),
            diagnostics: Vec::new(),
            rooms: Vec::new(),
            layers: Vec::new(),
        }
    }

    fn rect() -> Vec<Point2> {
        vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(12.0), Length::ZERO),
            Point2::new(Length::from_feet(12.0), Length::from_feet(8.0)),
            Point2::new(Length::ZERO, Length::from_feet(8.0)),
        ]
    }

    fn finish_system(
        id: &str,
        kind: SystemKind,
        finish: LayerFunction,
        finish_material: &str,
        finish_first: bool,
    ) -> ConstructionSystem {
        let framing = ConstructionLayer::new(
            LayerFunction::Framing,
            "mat-spf",
            BoardProfile::TwoBySix.nominal_depth(),
        )
        .with_framing(FramingSpec {
            member: BoardProfile::TwoBySix,
            spacing: Length::from_whole_inches(16),
            pattern: FramingPattern::Single,
            member_family: MemberFamily::Rafter,
            cavity_material: None,
        });
        let finish = ConstructionLayer::new(finish, finish_material, Length::from_whole_inches(1));
        ConstructionSystem {
            id: ElementId::new(id),
            name: id.to_owned(),
            kind,
            source: None,
            layers: if finish_first {
                vec![finish, framing]
            } else {
                vec![framing, finish]
            },
        }
    }

    /// A model with one sloped roof plane, one flat ceiling, and one floor deck
    /// over a 12×8 footprint (Polygon regions, so no walls are needed).
    fn surface_model() -> BuildingModel {
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        for level in &mut model.levels {
            if level.id.0 == "level-1" {
                level.height = Length::from_whole_inches(108);
            }
        }
        model
            .materials
            .push(Material::solid_color("mat-roof", "Shingle", [44, 46, 52]));
        model.materials.push(Material::solid_color(
            "mat-ceil",
            "Ceiling",
            [232, 232, 228],
        ));
        model.materials.push(Material::solid_color(
            "mat-floor",
            "Subfloor",
            [150, 116, 78],
        ));
        model.systems.push(finish_system(
            "system-roof",
            SystemKind::Roof,
            LayerFunction::Roofing,
            "mat-roof",
            false,
        ));
        model.systems.push(finish_system(
            "system-ceiling",
            SystemKind::Ceiling,
            LayerFunction::CeilingFinish,
            "mat-ceil",
            true,
        ));
        model.systems.push(finish_system(
            "system-floor",
            SystemKind::Floor,
            LayerFunction::InteriorFinish,
            "mat-floor",
            true,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-1",
            "Roof",
            "level-1",
            "system-roof",
            rect(),
            Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
            0,
            Length::from_feet(8.0),
        ));
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(rect()),
            Length::from_whole_inches(12),
        ));
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(rect()),
        ));
        model
    }

    /// The same 12×8 surface model, but capped by a rectangular hip roof: two
    /// trapezoid fields plus two triangular hip ends sharing a shortened ridge.
    fn hip_surface_model() -> BuildingModel {
        let mut model = surface_model();
        model.roof_planes.clear();

        let ft = Length::from_feet;
        let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
        let springing = ft(8.0);
        let ridge_west = Point2::new(ft(4.0), ft(4.0));
        let ridge_east = Point2::new(ft(8.0), ft(4.0));

        model.roof_planes.push(RoofPlane::new(
            "roof-east",
            "East hip end",
            "level-1",
            "system-roof",
            vec![
                Point2::new(ft(12.0), Length::ZERO),
                Point2::new(ft(12.0), ft(8.0)),
                ridge_east,
            ],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-north",
            "North hip field",
            "level-1",
            "system-roof",
            vec![
                Point2::new(ft(12.0), ft(8.0)),
                Point2::new(Length::ZERO, ft(8.0)),
                ridge_west,
                ridge_east,
            ],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-south",
            "South hip field",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(ft(12.0), Length::ZERO),
                ridge_east,
                ridge_west,
            ],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-west",
            "West hip end",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, ft(8.0)),
                Point2::new(Length::ZERO, Length::ZERO),
                ridge_west,
            ],
            slope,
            0,
            springing,
        ));
        model
    }

    fn build(model: &BuildingModel, selection: &Selection) -> Scene3d {
        Scene3d::from_project(
            model,
            &empty_plan(),
            0,
            selection,
            WorkspaceMode::Design,
            WallDisplay::Outline,
        )
        .expect("a model with surfaces builds a scene")
    }

    fn pick_clicks(scene: &Scene3d) -> Vec<ViewClick> {
        scene.picks.iter().map(|pick| pick.click.clone()).collect()
    }

    #[test]
    fn surfaces_emit_geometry_and_pick_volumes() {
        let scene = build(&surface_model(), &Selection::Wall);
        // Each surface is a double-faced sheet (two triangles per side for a quad),
        // so geometry is present and the framing points include the surface corners.
        assert!(!scene.vertices.is_empty(), "no surface geometry emitted");
        assert!(
            scene.points.len() >= 12,
            "surfaces did not feed the framing"
        );

        let clicks = pick_clicks(&scene);
        assert!(
            clicks
                .iter()
                .any(|c| matches!(c, ViewClick::RoofPlane { id } if id == "roof-1")),
            "no roof-plane pick volume emitted"
        );
        assert!(
            clicks
                .iter()
                .any(|c| matches!(c, ViewClick::Ceiling { id } if id == "ceiling-1")),
        );
        assert!(
            clicks
                .iter()
                .any(|c| matches!(c, ViewClick::FloorDeck { id } if id == "deck-1")),
        );
    }

    #[test]
    fn roof_surface_is_sloped_and_decks_sit_at_their_elevations() {
        let scene = build(&surface_model(), &Selection::Wall);
        let zs: Vec<f32> = scene.vertices.iter().map(|v| v.position[2]).collect();
        let lo = zs.iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        // Floor deck at level elevation 0; roof ridge at 96 + 96*(6/12) = 144".
        assert!((lo - 0.0).abs() < 0.5, "lowest surface z {lo}, want ~0");
        assert!(
            (hi - 144.0).abs() < 0.5,
            "highest surface z {hi}, want ~144"
        );
        // Pin the flat ceiling at level top (108") − height (12") = 96". It shares
        // the roof's eave elevation, so check a fully-horizontal triangle (which the
        // sloped roof never produces) rather than the raw z range — otherwise a
        // regression in the ceiling formula anywhere in (0, 144) would slip through.
        let flat_zs: Vec<f32> = horizontal_triangle_elevations(&scene);
        assert!(
            flat_zs.iter().any(|z| (z - 96.0).abs() < 0.5),
            "no horizontal ceiling surface at ~96in: {flat_zs:?}"
        );
        assert!(
            flat_zs.iter().any(|z| z.abs() < 0.5),
            "no horizontal floor surface at ~0in: {flat_zs:?}"
        );
        // The geometry is finite (no NaN normals from a degenerate fan).
        for v in &scene.vertices {
            assert!(v.position.iter().all(|c| c.is_finite()));
            assert!(v.normal.iter().all(|c| c.is_finite()));
        }
    }

    #[test]
    fn hip_roof_surfaces_emit_four_lifted_pickable_planes() {
        let scene = build(&hip_surface_model(), &Selection::Wall);
        let clicks = pick_clicks(&scene);
        for id in ["roof-east", "roof-north", "roof-south", "roof-west"] {
            assert!(
                clicks
                    .iter()
                    .any(|c| matches!(c, ViewClick::RoofPlane { id: found } if found == id)),
                "no pick volume emitted for {id}"
            );
        }

        let mut tilted_zs: Vec<f32> = Vec::new();
        let mut up_facing_tilted = 0;
        for tri in scene.indices.chunks_exact(3) {
            let v = |i: u32| scene.vertices[i as usize];
            let (a, b, c) = (v(tri[0]), v(tri[1]), v(tri[2]));
            let tilted = a.normal[2] > 0.1 && a.normal[2] < 0.99;
            if tilted {
                up_facing_tilted += 1;
                tilted_zs.extend([a.position[2], b.position[2], c.position[2]]);
            }
        }
        assert_eq!(
            up_facing_tilted, 6,
            "two trapezoids plus two triangles should emit six up-facing roof triangles"
        );
        let lo = tilted_zs.iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = tilted_zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!((lo - 96.0).abs() < 0.5, "hip eaves at {lo}, want ~96in");
        assert!((hi - 120.0).abs() < 0.5, "hip ridge at {hi}, want ~120in");
    }

    /// The elevations of every fully-horizontal triangle (all three vertices at one
    /// z) in the mesh — the flat ceiling/floor surfaces, never the sloped roof.
    fn horizontal_triangle_elevations(scene: &Scene3d) -> Vec<f32> {
        scene
            .indices
            .chunks_exact(3)
            .filter_map(|tri| {
                let z = |i: u32| scene.vertices[i as usize].position[2];
                let (a, b, c) = (z(tri[0]), z(tri[1]), z(tri[2]));
                ((a - b).abs() < 1.0e-3 && (a - c).abs() < 1.0e-3).then_some(a)
            })
            .collect()
    }

    /// A model with one gable roof plane over a 12×8 footprint and **no ceiling** —
    /// a cathedral. The roof system stacks a conditioned-side finish (soffit),
    /// framing, and roofing, so the weather face and the underside read as distinct
    /// colors.
    fn cathedral_model() -> BuildingModel {
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        for level in &mut model.levels {
            if level.id.0 == "level-1" {
                level.height = Length::from_whole_inches(108);
            }
        }
        model
            .materials
            .push(Material::solid_color("mat-roof", "Shingle", [44, 46, 52]));
        model.materials.push(Material::solid_color(
            "mat-soffit",
            "Soffit",
            [205, 180, 140],
        ));
        let framing = ConstructionLayer::new(
            LayerFunction::Framing,
            "mat-spf",
            BoardProfile::TwoBySix.nominal_depth(),
        )
        .with_framing(FramingSpec {
            member: BoardProfile::TwoBySix,
            spacing: Length::from_whole_inches(16),
            pattern: FramingPattern::Single,
            member_family: MemberFamily::Rafter,
            cavity_material: None,
        });
        model.systems.push(ConstructionSystem {
            id: ElementId::new("system-roof"),
            name: "Roof".to_owned(),
            kind: SystemKind::Roof,
            source: None,
            layers: vec![
                ConstructionLayer::new(
                    LayerFunction::CeilingFinish,
                    "mat-soffit",
                    Length::from_whole_inches(1),
                ),
                framing,
                ConstructionLayer::new(
                    LayerFunction::Roofing,
                    "mat-roof",
                    Length::from_whole_inches(1),
                ),
            ],
        });
        model.roof_planes.push(RoofPlane::new(
            "roof-1",
            "Roof",
            "level-1",
            "system-roof",
            rect(),
            Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
            0,
            Length::from_feet(8.0),
        ));
        model
    }

    /// The (color, min-z) of each sloped triangle facing up vs. down — used to tell
    /// the weather face (up) from the cathedral underside (down).
    fn sloped_faces(scene: &Scene3d, facing_up: bool) -> Vec<([f32; 4], f32)> {
        scene
            .indices
            .chunks_exact(3)
            .filter_map(|tri| {
                let v = |i: u32| scene.vertices[i as usize];
                let (a, b, c) = (v(tri[0]), v(tri[1]), v(tri[2]));
                let zs = [a.position[2], b.position[2], c.position[2]];
                let sloped = zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
                    - zs.iter().cloned().fold(f32::INFINITY, f32::min)
                    > 1.0;
                let up = a.normal[2] > 0.5;
                let down = a.normal[2] < -0.5;
                (sloped && (if facing_up { up } else { down }))
                    .then_some((a.color, zs.iter().cloned().fold(f32::INFINITY, f32::min)))
            })
            .collect()
    }

    #[test]
    fn cathedral_roof_underside_is_a_distinct_lowered_face() {
        let scene = build(&cathedral_model(), &Selection::Wall);
        let weather = sloped_faces(&scene, true);
        let underside = sloped_faces(&scene, false);
        assert!(!weather.is_empty(), "no up-facing weather roof triangles");
        assert!(
            !underside.is_empty(),
            "cathedral roof emitted no down-facing underside"
        );
        // The underside is a distinct (interior-finish) color, not the weather face.
        let weather_color = weather[0].0;
        let underside_color = underside[0].0;
        assert_ne!(
            weather_color, underside_color,
            "cathedral underside should differ from the weather face"
        );
        // ...and it is dropped one assembly-thickness (1 + 6 + 1 = 8in) below it.
        let weather_lo = weather.iter().map(|f| f.1).fold(f32::INFINITY, f32::min);
        let underside_lo = underside.iter().map(|f| f.1).fold(f32::INFINITY, f32::min);
        let drop = weather_lo - underside_lo;
        assert!(
            (drop - 8.0).abs() < 0.5,
            "underside dropped {drop}in below the weather face, want ~8"
        );
    }

    #[test]
    fn roof_with_a_ceiling_below_has_no_distinct_underside() {
        // Cover the footprint with a flat ceiling: the plane is no longer a
        // cathedral, so both roof faces share the weather color.
        let mut model = cathedral_model();
        model.systems.push(finish_system(
            "system-ceiling",
            SystemKind::Ceiling,
            LayerFunction::CeilingFinish,
            "mat-soffit",
            true,
        ));
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(rect()),
            Length::from_whole_inches(12),
        ));
        let scene = build(&model, &Selection::Wall);
        let weather = sloped_faces(&scene, true);
        let underside = sloped_faces(&scene, false);
        assert!(!weather.is_empty() && !underside.is_empty());
        // Every sloped down-face matches the weather color (no cathedral underside).
        assert!(
            underside.iter().all(|f| f.0 == weather[0].0),
            "a roof with a ceiling below should not recolor its underside"
        );
    }

    #[test]
    fn sloped_ceiling_is_lifted_via_the_frame_in_the_mesher() {
        // Slice A4: the app mesher lifts a sloped ceiling onto its plane via the
        // shared frame, instead of drawing it at a constant elevation. Isolate the
        // ceiling as the only tilted geometry (the surface_model roof is removed; the
        // floor stays horizontal), then a 6:12 slope over the 8ft run rises the
        // ceiling from its 96" springing to 144".
        let mut model = surface_model();
        model.roof_planes.clear();
        model.ceilings[0].slope = Some(framer_core::CeilingSlope::new(
            Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
            0,
        ));
        let scene = build(&model, &Selection::Wall);

        // Tilted triangles (normal has both a vertical and a horizontal component) are
        // the lifted ceiling; a flat ceiling would have purely vertical normals. Span
        // all three vertices of each tilted triangle so the elevation range does not
        // depend on the triangulator's vertex order.
        let mut tilted_zs: Vec<f32> = Vec::new();
        for tri in scene.indices.chunks_exact(3) {
            let nz = scene.vertices[tri[0] as usize].normal[2].abs();
            if nz > 0.1 && nz < 0.99 {
                tilted_zs.extend(tri.iter().map(|&i| scene.vertices[i as usize].position[2]));
            }
        }
        assert!(
            !tilted_zs.is_empty(),
            "the sloped ceiling is lifted via the frame, not drawn flat"
        );
        let lo = tilted_zs.iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = tilted_zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!((lo - 96.0).abs() < 0.5, "ceiling springs at 96in, got {lo}");
        assert!((hi - 144.0).abs() < 0.5, "ceiling rises to 144in, got {hi}");
    }

    #[test]
    fn surface_is_two_faced_with_opposite_normals() {
        // push_surface deliberately emits the outline twice with opposite normals so
        // the un-culled axonometric pipeline lights it from both sides. Pin that: the
        // flat floor deck's horizontal triangles must include both an up- and a
        // down-facing normal at its elevation.
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        model.systems.push(finish_system(
            "system-floor",
            SystemKind::Floor,
            LayerFunction::InteriorFinish,
            "mat-floor",
            true,
        ));
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(rect()),
        ));
        let scene = build(&model, &Selection::Wall);
        let normals_z: Vec<f32> = scene
            .indices
            .chunks_exact(3)
            .filter_map(|tri| {
                let v = |i: u32| scene.vertices[i as usize];
                let (a, b, c) = (v(tri[0]), v(tri[1]), v(tri[2]));
                let flat = (a.position[2] - b.position[2]).abs() < 1.0e-3
                    && (a.position[2] - c.position[2]).abs() < 1.0e-3;
                flat.then_some(a.normal[2])
            })
            .collect();
        assert!(
            normals_z.iter().any(|n| *n > 0.5),
            "no up-facing floor triangle"
        );
        assert!(
            normals_z.iter().any(|n| *n < -0.5),
            "no down-facing floor triangle (surface is not two-faced)"
        );
    }

    #[test]
    fn clicking_a_roof_surface_picks_it() {
        let scene = build(&surface_model(), &Selection::Wall);
        let drawing = Rect::from_min_size((0.0, 0.0).into(), (600.0, 400.0).into());
        let projector = OrbitProjector::from_points(&scene.points, drawing, View3dState::default())
            .expect("a projector for the surface points");
        // Aim at the roof's plan centroid (6ft, 4ft); the projected point must land
        // on the roof surface and pick it.
        let centroid = Point3::vector(
            Length::from_feet(6.0).inches() as f32,
            Length::from_feet(4.0).inches() as f32,
            // The plane elevation at the centroid: springing 96" + 48" up-slope ×
            // 6/12 = 120".
            120.0,
        );
        let screen = projector.project_point(centroid).pos;
        match scene.pick(screen, &projector) {
            Some(ViewClick::RoofPlane { id }) => assert_eq!(id, "roof-1"),
            _ => panic!("expected to pick the roof plane at its centroid"),
        }
    }

    /// Six corner-joined walls enclosing a concave L-shaped room with a floor deck
    /// attached via `SurfaceRegion::Room`.
    fn l_shaped_room_model() -> BuildingModel {
        let ft = Length::from_feet;
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let pts = [
            Point2::new(ft(0.0), ft(0.0)),
            Point2::new(ft(12.0), ft(0.0)),
            Point2::new(ft(12.0), ft(6.0)),
            Point2::new(ft(6.0), ft(6.0)),
            Point2::new(ft(6.0), ft(12.0)),
            Point2::new(ft(0.0), ft(12.0)),
        ];
        for i in 0..pts.len() {
            let next = (i + 1) % pts.len();
            model.walls.push(
                Wall::new(format!("w-{i}"), "Wall", ft(1.0), &model.code)
                    .with_placement("level-1", pts[i], pts[next]),
            );
        }
        model.rooms.push(Room::new(
            "room-1",
            "L room",
            RoomUsage::default(),
            "level-1",
            Point2::new(ft(3.0), ft(3.0)),
        ));
        model.systems.push(finish_system(
            "system-floor",
            SystemKind::Floor,
            LayerFunction::InteriorFinish,
            "mat-floor",
            true,
        ));
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Room(ElementId::new("room-1")),
        ));
        model
    }

    #[test]
    fn room_region_surface_resolves_concave_loop_and_tiles_it() {
        // The 3D mesher's Room arm: SurfaceRegion::Room ->
        // room_boundary_on_level (a concave L) -> ear-clip -> surface. The deck's
        // pick volume must carry the full 6-vertex L outline (not a convex hull),
        // and its triangulation must tile the L's plan area (no fan spill into the
        // notch).
        let scene = build(&l_shaped_room_model(), &Selection::Wall);
        let deck = scene
            .picks
            .iter()
            .find(|p| matches!(&p.click, ViewClick::FloorDeck { id } if id == "deck-1"))
            .expect("a floor-deck pick volume");
        let PickShape::Surface(outline) = &deck.shape else {
            panic!("a floor deck must pick as a surface polygon, not a cuboid");
        };
        assert_eq!(outline.len(), 6, "the concave L room loop has six vertices");

        // Tile the resolved outline through the same ear-clip the mesher used.
        let plan: Vec<Point2> = outline
            .iter()
            .map(|p| {
                Point2::new(
                    Length::from_inches(p.x as f64),
                    Length::from_inches(p.y as f64),
                )
            })
            .collect();
        let tris = framer_core::triangulate_simple_polygon(&plan);
        assert_eq!(tris.len(), 4, "an L (6-gon) ear-clips to four triangles");
        let area: f64 = tris
            .iter()
            .map(|&[a, b, c]| framer_core::polygon_area_square_inches(&[plan[a], plan[b], plan[c]]))
            .sum();
        // 12×12 − 6×6 = 108 sq ft = 15552 sq in.
        assert!(
            (area - 15552.0).abs() < 5.0,
            "L deck triangles cover {area} sq in, expected 15552"
        );
    }

    fn stacked_unenclosed_room_deck_model() -> BuildingModel {
        let ft = Length::from_feet;
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        model
            .levels
            .push(Level::new("level-2", "Level 2", ft(10.0)));
        let outline = rect();
        for i in 0..outline.len() {
            let next = (i + 1) % outline.len();
            model.walls.push(
                Wall::new(format!("w-{i}"), "Wall", ft(1.0), &model.code).with_placement(
                    "level-1",
                    outline[i],
                    outline[next],
                ),
            );
        }
        model.rooms.push(Room::new(
            "room-2",
            "Upper room",
            RoomUsage::Living,
            "level-2",
            Point2::new(ft(6.0), ft(4.0)),
        ));
        model.systems.push(finish_system(
            "system-floor",
            SystemKind::Floor,
            LayerFunction::InteriorFinish,
            "mat-floor",
            true,
        ));
        model.floor_decks.push(FloorDeck::new(
            "deck-2",
            "Upper deck",
            "level-2",
            "system-floor",
            SurfaceRegion::Room(ElementId::new("room-2")),
        ));
        model
    }

    #[test]
    fn room_region_mesh_resolves_against_the_room_level() {
        let model = stacked_unenclosed_room_deck_model();

        assert!(
            region_outline_plan(&model, &model.floor_decks[0].region).is_none(),
            "a level-2 room region must not borrow the level-1 enclosure"
        );

        let scene = build(&model, &Selection::Wall);
        assert!(
            !pick_clicks(&scene)
                .iter()
                .any(|click| matches!(click, ViewClick::FloorDeck { id } if id == "deck-2")),
            "no floor-deck pick volume should be emitted for the unresolved level-2 room"
        );
    }
}
