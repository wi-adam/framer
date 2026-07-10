//! Wall and roof frame-member emission plus board and cut-rafter primitives.

use eframe::egui::Color32;
use framer_core::{
    BuildingModel, ConstructionSystem, ElementId, Length, MemberFamily, Point2, RoofPlane, Wall,
};
use framer_solver::{FrameMember, MemberKind, MemberOrientation};

use super::walls::{WallBasis, WallCuboid, layer_band_span};
use super::{
    CUBOID_FACE_INDICES, PickSolid, Point3, SceneBuilder, color_to_rgba, cross, normalized, offset,
    vector_between,
};
use crate::app::ViewClick;

impl SceneBuilder {
    pub(super) fn push_roof_member(
        &mut self,
        model: &BuildingModel,
        plane: Option<&RoofPlane>,
        host_id: &ElementId,
        member: &FrameMember,
        color: Color32,
    ) {
        let is_common_stick_rafter = member.kind == MemberKind::Rafter
            && plane.is_some_and(|plane| {
                roof_member_family(model, plane) == Some(MemberFamily::Rafter)
            });
        if is_common_stick_rafter
            && let Some(plane) = plane
            && self.push_common_rafter(
                host_id,
                plane,
                member,
                matched_bearing_depth(model, plane),
                color,
            )
        {
            return;
        }
        self.push_spatial_member(host_id, member, color);
    }

    pub(super) fn push_member(
        &mut self,
        wall: &Wall,
        member: &FrameMember,
        total: Length,
        interior_sign: f32,
        base_elevation: f32,
        color: Color32,
    ) {
        // Rake plates are wall-owned but spatially sloped. Their explicit endpoint
        // elevations are already absolute, unlike ordinary wall-local stud/plate z.
        if member.kind == MemberKind::RakePlate && member.sloped.is_some() {
            self.push_rake_plate(wall, member, total, interior_sign, color);
            return;
        }
        if member.sloped.is_some() {
            self.push_spatial_member(&wall.id, member, color);
            return;
        }
        let half_member = member.cross_section_depth.inches() as f32 / 2.0;
        let (x0, x1, z0, z1) = match member.orientation {
            MemberOrientation::Horizontal => (
                member.x.inches() as f32,
                (member.x + member.cut_length).inches() as f32,
                base_elevation + member.elevation.inches() as f32,
                base_elevation + (member.elevation + member.cross_section_depth).inches() as f32,
            ),
            MemberOrientation::Vertical => (
                member.x.inches() as f32 - half_member,
                member.x.inches() as f32 + half_member,
                base_elevation + member.elevation.inches() as f32,
                base_elevation + (member.elevation + member.cut_length).inches() as f32,
            ),
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
                source_id: wall.id.0.clone(),
                member_id: member.id.clone(),
            },
            3,
            solid.corners,
        ));
    }

    fn push_spatial_member(&mut self, host_id: &ElementId, member: &FrameMember, color: Color32) {
        let Some(sloped) = member.sloped else {
            return;
        };
        let start = Point3::vector(
            sloped.start.x.inches() as f32,
            sloped.start.y.inches() as f32,
            sloped.low_elevation.inches() as f32,
        );
        let end = Point3::vector(
            sloped.end.x.inches() as f32,
            sloped.end.y.inches() as f32,
            sloped.high_elevation.inches() as f32,
        );
        let Some(prism) = BoardPrism::new(
            start,
            end,
            member.cross_section_depth.inches() as f32 * -0.5,
            member.cross_section_depth.inches() as f32 * 0.5,
            member.side_offset.inches() as f32,
            (member.side_offset + member.side_depth).inches() as f32,
        ) else {
            return;
        };
        self.push_board_prism(&prism, color_to_rgba(color));
        self.picks.push(PickSolid::cuboid(
            ViewClick::Member {
                source_id: host_id.0.clone(),
                member_id: member.id.clone(),
            },
            3,
            prism.corners,
        ));
    }

    /// Mesh one generated common stick rafter with vertical tail/ridge faces and,
    /// when its authored eave exactly matches a wall, a horizontal birdsmouth
    /// seat. The solver's endpoints and BOM length remain unchanged; this is the
    /// Plan-mode construction-detail presentation of that derived member.
    fn push_common_rafter(
        &mut self,
        host_id: &ElementId,
        plane: &RoofPlane,
        member: &FrameMember,
        bearing_depth: Option<Length>,
        color: Color32,
    ) -> bool {
        let Some(sloped) = member.sloped else {
            return false;
        };
        let start = Point3::vector(
            sloped.start.x.inches() as f32,
            sloped.start.y.inches() as f32,
            sloped.low_elevation.inches() as f32,
        );
        let end = Point3::vector(
            sloped.end.x.inches() as f32,
            sloped.end.y.inches() as f32,
            sloped.high_elevation.inches() as f32,
        );
        let Some(prism) = RafterPrism::new(
            start,
            end,
            member.cross_section_depth.inches() as f32,
            member.side_offset.inches() as f32,
            (member.side_offset + member.side_depth).inches() as f32,
            plane,
            bearing_depth.map(|depth| depth.inches() as f32),
        ) else {
            return false;
        };
        self.push_profile_prism(&prism, color_to_rgba(color));
        self.picks.push(PickSolid::mesh(
            ViewClick::Member {
                source_id: host_id.0.clone(),
                member_id: member.id.clone(),
            },
            3,
            prism.points.clone(),
            prism.triangles.clone(),
        ));
        true
    }

    /// A wall-owned rake plate follows explicit spatial endpoints but keeps wall
    /// construction axes: the framing-band depth spans through the wall and the
    /// board thickness drops inside the gable plane below the roof rake.
    fn push_rake_plate(
        &mut self,
        wall: &Wall,
        member: &FrameMember,
        total: Length,
        interior_sign: f32,
        color: Color32,
    ) {
        let Some(sloped) = member.sloped else {
            return;
        };
        let start = Point3::vector(
            sloped.start.x.inches() as f32,
            sloped.start.y.inches() as f32,
            sloped.low_elevation.inches() as f32,
        );
        let end = Point3::vector(
            sloped.end.x.inches() as f32,
            sloped.end.y.inches() as f32,
            sloped.high_elevation.inches() as f32,
        );
        let (side0, side1) =
            layer_band_span(interior_sign, total, member.side_offset, member.side_depth);
        let basis = WallBasis::new(wall);
        let across = Point3::vector(basis.side_x, basis.side_y, 0.0);
        let Some(prism) = BoardPrism::new_with_across(
            start,
            end,
            across,
            side0,
            side1,
            -member.cross_section_depth.inches() as f32,
            0.0,
        ) else {
            return;
        };
        self.push_board_prism(&prism, color_to_rgba(color));
        self.picks.push(PickSolid::cuboid(
            ViewClick::Member {
                source_id: wall.id.0.clone(),
                member_id: member.id.clone(),
            },
            3,
            prism.corners,
        ));
    }

    fn push_board_prism(&mut self, prism: &BoardPrism, color: [f32; 4]) {
        self.points.extend(prism.corners);
        for face in CUBOID_FACE_INDICES {
            self.push_quad(face.map(|index| prism.corners[index]), color);
        }
    }

    fn push_profile_prism(&mut self, prism: &RafterPrism, color: [f32; 4]) {
        self.points.extend_from_slice(&prism.points);
        for &triangle in &prism.triangles {
            self.push_triangle(triangle.map(|index| prism.points[index]), color);
        }
    }
}

/// A board whose centerline may run in any 3-D direction. Corner order matches
/// [`CUBOID_FACE_INDICES`], so the existing pick and face topology works for
/// jack, ridge, hip, valley, blocking, and rake-plate members alike. Common stick
/// rafters use [`RafterPrism`] so their longitudinal cuts are explicit.
struct BoardPrism {
    corners: [Point3; 8],
}

impl BoardPrism {
    fn new(
        start: Point3,
        end: Point3,
        across0: f32,
        across1: f32,
        section0: f32,
        section1: f32,
    ) -> Option<Self> {
        let along = normalized(vector_between(start, end))?;
        let plan_length = (along.x * along.x + along.y * along.y).sqrt();
        let across = if plan_length > f32::EPSILON {
            Point3::vector(-along.y / plan_length, along.x / plan_length, 0.0)
        } else {
            Point3::X
        };
        Self::new_with_across(start, end, across, across0, across1, section0, section1)
    }

    fn new_with_across(
        start: Point3,
        end: Point3,
        across: Point3,
        across0: f32,
        across1: f32,
        section0: f32,
        section1: f32,
    ) -> Option<Self> {
        if across1 - across0 <= f32::EPSILON || section1 - section0 <= f32::EPSILON {
            return None;
        }
        let along = normalized(vector_between(start, end))?;
        let across = normalized(across)?;
        let mut section = normalized(cross(along, across))?;
        // Both roof-band offsets and the rake-plate "below the rake" convention
        // depend on a consistently upward-facing section axis.
        if section.z < 0.0 {
            section = -section;
        }
        let point = |origin: Point3, across_offset: f32, section_offset: f32| {
            offset(
                offset(origin, across, across_offset),
                section,
                section_offset,
            )
        };
        Some(Self {
            corners: [
                point(start, across0, section0),
                point(end, across0, section0),
                point(end, across1, section0),
                point(start, across1, section0),
                point(start, across0, section1),
                point(end, across0, section1),
                point(end, across1, section1),
                point(start, across1, section1),
            ],
        })
    }
}

/// A common-rafter solid extruded through its board thickness from a concave
/// longitudinal profile. `profile` is `(plan run, building elevation)` and is
/// retained in tests so they can name the plumb and birdsmouth edges directly.
pub(super) struct RafterPrism {
    #[cfg(test)]
    pub(super) profile: Vec<[f32; 2]>,
    pub(super) points: Vec<Point3>,
    pub(super) triangles: Vec<[usize; 3]>,
}

impl RafterPrism {
    /// A geometry-safety cap, not a code-compliance judgment. It prevents a steep
    /// pitch plus a deep bearing wall from consuming the rafter section.
    const MAX_NOTCH_DEPTH_FRACTION: f32 = 1.0 / 3.0;

    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        start: Point3,
        end: Point3,
        board_thickness: f32,
        section0: f32,
        section1: f32,
        plane: &RoofPlane,
        bearing_depth: Option<f32>,
    ) -> Option<Self> {
        if board_thickness <= f32::EPSILON || section1 - section0 <= f32::EPSILON {
            return None;
        }
        let plan_dx = end.x - start.x;
        let plan_dy = end.y - start.y;
        let plan_run = (plan_dx * plan_dx + plan_dy * plan_dy).sqrt();
        if plan_run <= f32::EPSILON {
            return None;
        }
        let run = Point3::vector(plan_dx / plan_run, plan_dy / plan_run, 0.0);
        let across = Point3::vector(-run.y, run.x, 0.0);
        let rise_over_run = (end.z - start.z) / plan_run;
        let slope_cosine = 1.0 / (1.0 + rise_over_run * rise_over_run).sqrt();
        let lower_z = |u: f32| start.z + rise_over_run * u + section0 / slope_cosine;
        let upper_z = |u: f32| start.z + rise_over_run * u + section1 / slope_cosine;

        let mut profile = vec![[0.0, lower_z(0.0)]];
        if let (Some(frame), Some(bearing_depth)) = (plane.frame(), bearing_depth)
            && rise_over_run > f32::EPSILON
            && bearing_depth > f32::EPSILON
        {
            let bearing_run = -frame.up_slope_distance(start.x as f64, start.y as f64) as f32;
            let max_notch_depth =
                (section1 - section0) / slope_cosine * Self::MAX_NOTCH_DEPTH_FRACTION;
            let seat_run = bearing_depth.min(max_notch_depth / rise_over_run);
            let heel_run = (bearing_run - seat_run).max(0.0);
            let toe_run = bearing_run.min(plan_run);
            let seat_z = lower_z(toe_run);
            if toe_run - heel_run > 1.0e-3 && seat_z - lower_z(heel_run) > 1.0e-3 {
                profile.extend([
                    [heel_run, lower_z(heel_run)],
                    [heel_run, seat_z],
                    [toe_run, seat_z],
                ]);
            }
        }
        profile.extend([
            [plan_run, lower_z(plan_run)],
            [plan_run, upper_z(plan_run)],
            [0.0, upper_z(0.0)],
        ]);

        // Reuse the core's deterministic concave-polygon ear clipper rather than
        // maintaining a second triangulation kernel in the app.
        let local_outline: Vec<Point2> = profile
            .iter()
            .map(|[u, z]| {
                Point2::new(
                    Length::from_inches(*u as f64),
                    Length::from_inches(*z as f64),
                )
            })
            .collect();
        let end_triangles = framer_core::triangulate_simple_polygon(&local_outline);
        if end_triangles.len() + 2 != profile.len() {
            return None;
        }

        let half_thickness = board_thickness / 2.0;
        let world_point = |[u, z]: [f32; 2], across_offset: f32| {
            Point3::vector(
                start.x + run.x * u + across.x * across_offset,
                start.y + run.y * u + across.y * across_offset,
                z,
            )
        };
        let mut points = Vec::with_capacity(profile.len() * 2);
        points.extend(
            profile
                .iter()
                .copied()
                .map(|point| world_point(point, -half_thickness)),
        );
        points.extend(
            profile
                .iter()
                .copied()
                .map(|point| world_point(point, half_thickness)),
        );

        let count = profile.len();
        let mut triangles = Vec::with_capacity(end_triangles.len() * 2 + count * 2);
        triangles.extend(end_triangles.iter().copied());
        triangles.extend(
            end_triangles
                .iter()
                .map(|[a, b, c]| [count + c, count + b, count + a]),
        );
        for index in 0..count {
            let next = (index + 1) % count;
            triangles.push([index, next, count + next]);
            triangles.push([index, count + next, count + index]);
        }

        Some(Self {
            #[cfg(test)]
            profile,
            points,
            triangles,
        })
    }
}

/// The triangular solid between one authored wall top and its matched roof rakes,
/// extruded through either one wall-system layer or the whole envelope thickness.
fn roof_member_family(model: &BuildingModel, plane: &RoofPlane) -> Option<MemberFamily> {
    model
        .systems
        .iter()
        .find(|system| system.id == plane.system)
        .and_then(ConstructionSystem::framing_layer)
        .and_then(|layer| layer.framing.as_ref())
        .map(|framing| framing.member_family)
}

/// Nominal bearing width for a generated roof plane whose authored eave is the
/// exact centerline segment of one same-level wall. Exact matching deliberately
/// fails closed for manually floating/partial roof planes: the view must not
/// invent a birdsmouth without a real authored bearing relationship.
pub(super) fn matched_bearing_depth(model: &BuildingModel, plane: &RoofPlane) -> Option<Length> {
    let count = plane.outline.len();
    if count < 2 || plane.eave_edge as usize >= count {
        return None;
    }
    let index = plane.eave_edge as usize;
    let eave = (plane.outline[index], plane.outline[(index + 1) % count]);
    model
        .walls
        .iter()
        .find(|wall| {
            wall.level == plane.level
                && ((wall.start == eave.0 && wall.end == eave.1)
                    || (wall.start == eave.1 && wall.end == eave.0))
        })
        .and_then(|wall| model.system_for(wall))
        .and_then(ConstructionSystem::framing_layer)
        .and_then(|layer| layer.framing.as_ref())
        .map(|framing| framing.member.nominal_depth())
}
