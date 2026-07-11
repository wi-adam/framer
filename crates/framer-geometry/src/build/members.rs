use std::collections::BTreeMap;

use framer_core::{
    BuildingModel, ConstructionSystem, ElementId, FloorDeck, Length, MemberFamily, Point2,
    RoofPlane, SpanDirection, Wall,
};
use framer_solver::{FrameMember, MemberKind, MemberOrientation, ProjectFramePlan};

use super::{
    cross, cuboid_solid, level_elevation, level_top, normalized, offset, push_body_result,
    region_outline, vector_between,
};
use crate::{BodyRef, PhysicalScene, PhysicalSolid, Point3, TriMesh};

pub(super) fn build_members(
    model: &BuildingModel,
    plan: &ProjectFramePlan,
    scene: &mut PhysicalScene,
) {
    let fallback_depth = model.framing_defaults().stud_profile.nominal_depth();
    let interior_sides = framer_core::wall_interior_sides(model);
    for wall_plan in &plan.wall_plans {
        let Some(wall) = model.walls.iter().find(|wall| wall.id == wall_plan.wall) else {
            for member in &wall_plan.members {
                let body_ref = member_ref(&wall_plan.wall, member);
                push_body_result(scene, body_ref, Err("wall member host is missing".into()));
            }
            continue;
        };
        let total = model
            .system_for(wall)
            .map(ConstructionSystem::total_thickness)
            .unwrap_or(fallback_depth);
        let sign = interior_sign(&interior_sides, &wall.id);
        let base = level_elevation(model, &wall.level);
        for member in &wall_plan.members {
            let body_ref = member_ref(&wall.id, member);
            push_body_result(
                scene,
                body_ref,
                wall_member_solid(wall, member, total, sign, base),
            );
        }
    }

    for floor_plan in &plan.floor_plans {
        let Some(deck) = model
            .floor_decks
            .iter()
            .find(|deck| deck.id == floor_plan.floor)
        else {
            push_missing_host_members(scene, &floor_plan.floor, &floor_plan.members, "floor deck");
            continue;
        };
        let outline = region_outline(model, &deck.region);
        let surface_z = level_elevation(model, &deck.level);
        for member in &floor_plan.members {
            let body_ref = member_ref(&deck.id, member);
            push_body_result(
                scene,
                body_ref,
                flat_floor_member_solid(deck, member, outline.as_deref(), surface_z),
            );
        }
    }

    for ceiling_plan in &plan.ceiling_plans {
        let Some(ceiling) = model
            .ceilings
            .iter()
            .find(|ceiling| ceiling.id == ceiling_plan.ceiling)
        else {
            push_missing_host_members(
                scene,
                &ceiling_plan.ceiling,
                &ceiling_plan.members,
                "ceiling",
            );
            continue;
        };
        let outline = region_outline(model, &ceiling.region);
        let reference = (level_top(model, &ceiling.level) - ceiling.height).inches();
        for member in &ceiling_plan.members {
            let body_ref = member_ref(&ceiling.id, member);
            push_body_result(
                scene,
                body_ref,
                ceiling_member_solid(ceiling, member, outline.as_deref(), reference),
            );
        }
    }

    let ridge_boards: Vec<_> = plan
        .roof_plans
        .iter()
        .flat_map(|roof_plan| &roof_plan.members)
        .filter(|member| member.kind == MemberKind::RidgeBoard)
        .collect();
    let roof_members: Vec<_> = plan
        .roof_plans
        .iter()
        .flat_map(|roof_plan| &roof_plan.members)
        .collect();
    for roof_plan in &plan.roof_plans {
        let plane = model
            .roof_planes
            .iter()
            .find(|plane| plane.id == roof_plan.roof);
        for member in &roof_plan.members {
            let body_ref = member_ref(&roof_plan.roof, member);
            push_body_result(
                scene,
                body_ref,
                roof_member_solid(model, plane, member, &ridge_boards, &roof_members),
            );
        }
    }
}

fn push_missing_host_members(
    scene: &mut PhysicalScene,
    owner: &ElementId,
    members: &[FrameMember],
    host: &str,
) {
    for member in members {
        push_body_result(
            scene,
            member_ref(owner, member),
            Err(format!("{host} member host is missing")),
        );
    }
}

fn member_ref(owner: &ElementId, member: &FrameMember) -> BodyRef {
    BodyRef::member(owner.clone(), member.kind, member.id.clone())
}

fn wall_member_solid(
    wall: &Wall,
    member: &FrameMember,
    total: Length,
    interior_sign: f64,
    base_elevation: f64,
) -> Result<PhysicalSolid, String> {
    if member.kind == MemberKind::RakePlate && member.sloped.is_some() {
        return rake_plate_solid(wall, member, total, interior_sign);
    }
    if member.sloped.is_some() {
        return spatial_board_solid(member);
    }
    let half_member = member.cross_section_depth.inches() / 2.0;
    let (x0, x1, z0, z1) = match member.orientation {
        MemberOrientation::Horizontal => (
            member.x.inches(),
            (member.x + member.cut_length).inches(),
            base_elevation + member.elevation.inches(),
            base_elevation + (member.elevation + member.cross_section_depth).inches(),
        ),
        MemberOrientation::Vertical => (
            member.x.inches() - half_member,
            member.x.inches() + half_member,
            base_elevation + member.elevation.inches(),
            base_elevation + (member.elevation + member.cut_length).inches(),
        ),
    };
    let (side0, side1) =
        layer_band_span(interior_sign, total, member.side_offset, member.side_depth);
    cuboid_solid(WallBasis::new(wall).cuboid(x0, x1, side0, side1, z0, z1))
}

fn roof_member_solid(
    model: &BuildingModel,
    plane: Option<&RoofPlane>,
    member: &FrameMember,
    ridge_boards: &[&FrameMember],
    roof_members: &[&FrameMember],
) -> Result<PhysicalSolid, String> {
    let is_common_stick_rafter = member.kind == MemberKind::Rafter
        && plane
            .is_some_and(|plane| roof_member_family(model, plane) == Some(MemberFamily::Rafter));
    if is_common_stick_rafter {
        let plane = plane.ok_or_else(|| "common rafter roof plane is missing".to_string())?;
        let ridge_setback = ridge_face_setback(member, ridge_boards).unwrap_or(0.0);
        let valley_setback = roof_members
            .iter()
            .filter(|target| target.kind == MemberKind::ValleyRafter)
            .filter_map(|target| endpoint_face_setback(member, target, true))
            .fold(0.0, f64::max);
        return build_common_rafter_solid(
            member,
            plane,
            matched_bearing_depth(model, plane).map(Length::inches),
            Some(ridge_setback.max(valley_setback)).filter(|setback| *setback > 0.0),
        )
        .map(|built| built.solid);
    }
    roof_spatial_board_solid(member, roof_members)
}

fn roof_spatial_board_solid(
    member: &FrameMember,
    roof_members: &[&FrameMember],
) -> Result<PhysicalSolid, String> {
    let sloped = member
        .sloped
        .ok_or_else(|| "spatial member lacks endpoint placement".to_string())?;
    let original_start = Point3::new(
        sloped.start.x.inches(),
        sloped.start.y.inches(),
        sloped.low_elevation.inches(),
    );
    let original_end = Point3::new(
        sloped.end.x.inches(),
        sloped.end.y.inches(),
        sloped.high_elevation.inches(),
    );
    let target_kinds: &[MemberKind] = match member.kind {
        MemberKind::JackRafter | MemberKind::Blocking => {
            &[MemberKind::HipRafter, MemberKind::ValleyRafter]
        }
        MemberKind::HipRafter => &[MemberKind::Rafter, MemberKind::RidgeBoard],
        MemberKind::ValleyRafter => &[MemberKind::RidgeBoard],
        _ => return spatial_board_solid_between(member, original_start, original_end),
    };
    let start_setback = if member.kind == MemberKind::Blocking {
        roof_members
            .iter()
            .filter(|target| target_kinds.contains(&target.kind))
            .filter_map(|target| endpoint_face_setback(member, target, false))
            .fold(0.0, f64::max)
    } else {
        0.0
    };
    let end_setback = roof_members
        .iter()
        .filter(|target| target_kinds.contains(&target.kind))
        .filter_map(|target| endpoint_face_setback(member, target, true))
        .fold(0.0, f64::max);
    let plan_dx = original_end.x - original_start.x;
    let plan_dy = original_end.y - original_start.y;
    let plan_length = (plan_dx * plan_dx + plan_dy * plan_dy).sqrt();
    if plan_length <= f64::EPSILON || start_setback + end_setback >= plan_length {
        return Err("roof member face setbacks consume its plan length".into());
    }
    let delta = vector_between(original_start, original_end);
    let start = offset(original_start, delta, start_setback / plan_length);
    let end = offset(original_end, delta, -end_setback / plan_length);
    spatial_board_solid_between(member, start, end)
}

fn endpoint_face_setback(source: &FrameMember, target: &FrameMember, at_end: bool) -> Option<f64> {
    let source_placement = source.sloped?;
    let target_placement = target.sloped?;
    let endpoint = if at_end {
        source_placement.end
    } else {
        source_placement.start
    };
    let endpoint_z = if at_end {
        source_placement.high_elevation.inches()
    } else {
        source_placement.low_elevation.inches()
    };
    let tolerance = Length::from_ticks(1);
    let target_dx = (target_placement.end.x - target_placement.start.x).inches();
    let target_dy = (target_placement.end.y - target_placement.start.y).inches();
    let target_length_squared = target_dx * target_dx + target_dy * target_dy;
    if target_length_squared <= f64::EPSILON {
        return None;
    }
    let target_t = ((endpoint.x - target_placement.start.x).inches() * target_dx
        + (endpoint.y - target_placement.start.y).inches() * target_dy)
        / target_length_squared;
    let target_tolerance = tolerance.inches() / target_length_squared.sqrt();
    if target_t < -target_tolerance || target_t > 1.0 + target_tolerance {
        return None;
    }

    let source_start = Point3::new(
        source_placement.start.x.inches(),
        source_placement.start.y.inches(),
        source_placement.low_elevation.inches(),
    );
    let source_end = Point3::new(
        source_placement.end.x.inches(),
        source_placement.end.y.inches(),
        source_placement.high_elevation.inches(),
    );
    let along = normalized(vector_between(source_start, source_end))?;
    let plan_length = (along.x * along.x + along.y * along.y).sqrt();
    if plan_length <= f64::EPSILON {
        return None;
    }
    let plan_direction = Point3::new(along.x / plan_length, along.y / plan_length, 0.0);
    let source_across = Point3::new(-plan_direction.y, plan_direction.x, 0.0);
    let mut source_section = normalized(cross(along, source_across))?;
    if source_section.z < 0.0 {
        source_section = Point3::new(-source_section.x, -source_section.y, -source_section.z);
    }
    let target_length = target_length_squared.sqrt();
    let target_across = Point3::new(-target_dy / target_length, target_dx / target_length, 0.0);
    let target_start = Point3::new(
        target_placement.start.x.inches(),
        target_placement.start.y.inches(),
        target_placement.low_elevation.inches(),
    );
    let target_end = Point3::new(
        target_placement.end.x.inches(),
        target_placement.end.y.inches(),
        target_placement.high_elevation.inches(),
    );
    let target_along = normalized(vector_between(target_start, target_end))?;
    let mut target_section = normalized(cross(target_along, target_across))?;
    if target_section.z < 0.0 {
        target_section = Point3::new(-target_section.x, -target_section.y, -target_section.z);
    }
    let source_z = [source.side_offset, source.side_offset + source.side_depth]
        .map(|offset| endpoint_z + source_section.z * offset.inches());
    let target_anchor_z = target_placement.low_elevation.inches()
        + (target_placement.high_elevation - target_placement.low_elevation).inches() * target_t;
    let target_z = [target.side_offset, target.side_offset + target.side_depth]
        .map(|offset| target_anchor_z + target_section.z * offset.inches());
    if source_z[1].min(target_z[1]) < source_z[0].max(target_z[0]) - tolerance.inches() {
        return None;
    }
    let direction_dot = plan_direction.x * target_across.x + plan_direction.y * target_across.y;
    let approach = direction_dot.abs();
    if approach <= f64::EPSILON {
        return None;
    }
    let side_sign = if at_end {
        (-direction_dot).signum()
    } else {
        direction_dot.signum()
    };
    let half = source.cross_section_depth.inches() * 0.5;
    let section0 = source.side_offset.inches();
    let section1 = (source.side_offset + source.side_depth).inches();
    let min_offset = [-half, half]
        .into_iter()
        .flat_map(|across_offset| {
            [section0, section1].into_iter().map(move |section_offset| {
                let projection = source_across.x * across_offset * target_across.x
                    + source_across.y * across_offset * target_across.y
                    + source_section.x * section_offset * target_across.x
                    + source_section.y * section_offset * target_across.y;
                side_sign * projection
            })
        })
        .fold(f64::INFINITY, f64::min);
    let endpoint_dx = (endpoint.x - target_placement.start.x).inches();
    let endpoint_dy = (endpoint.y - target_placement.start.y).inches();
    let current_side_distance =
        side_sign * (endpoint_dx * target_across.x + endpoint_dy * target_across.y);
    let required_side_distance = target.cross_section_depth.inches() * 0.5 - min_offset;
    if current_side_distance.abs() > required_side_distance + tolerance.inches() {
        return None;
    }
    Some((required_side_distance - current_side_distance).max(0.0) / approach)
}

fn flat_floor_member_solid(
    deck: &FloorDeck,
    member: &FrameMember,
    outline: Option<&[Point2]>,
    surface_z: f64,
) -> Result<PhysicalSolid, String> {
    let outline = outline.ok_or_else(|| "floor member region cannot be resolved".to_string())?;
    horizontal_surface_member_solid(outline, deck.span, member, surface_z, -1.0)
}

fn ceiling_member_solid(
    ceiling: &framer_core::Ceiling,
    member: &FrameMember,
    outline: Option<&[Point2]>,
    reference: f64,
) -> Result<PhysicalSolid, String> {
    if member.sloped.is_some() {
        return sloped_ceiling_member_solid(ceiling, member, reference);
    }
    let outline = outline.ok_or_else(|| "ceiling member region cannot be resolved".to_string())?;
    horizontal_surface_member_solid(outline, SpanDirection::Shorter, member, reference, 1.0)
}

fn sloped_ceiling_member_solid(
    ceiling: &framer_core::Ceiling,
    member: &FrameMember,
    reference: f64,
) -> Result<PhysicalSolid, String> {
    let frame = ceiling
        .frame(Length::from_inches(reference))
        .ok_or_else(|| "sloped ceiling member has no valid surface frame".to_string())?;
    let sloped = member
        .sloped
        .ok_or_else(|| "sloped ceiling member lacks endpoint placement".to_string())?;
    let (up_x, up_y) = frame.up_slope();
    let thickness = member.cross_section_depth.inches();
    let mut start = Point3::new(
        sloped.start.x.inches(),
        sloped.start.y.inches(),
        sloped.low_elevation.inches(),
    );
    let mut end = Point3::new(
        sloped.end.x.inches(),
        sloped.end.y.inches(),
        sloped.high_elevation.inches(),
    );
    match member.kind {
        MemberKind::CeilingJoist => {
            let section_up = -frame.rise_over_run()
                / (1.0 + frame.rise_over_run() * frame.rise_over_run()).sqrt();
            let section0 = member.side_offset.inches();
            let section1 = (member.side_offset + member.side_depth).inches();
            let shift0 = section_up * section0;
            let shift1 = section_up * section1;
            let start_trim = thickness - shift0.min(shift1);
            let end_trim = thickness + shift0.max(shift1);
            start.x += up_x * start_trim;
            start.y += up_y * start_trim;
            start.z = frame.elevation_at(start.x, start.y);
            end.x -= up_x * end_trim;
            end.y -= up_y * end_trim;
            end.z = frame.elevation_at(end.x, end.y);
        }
        MemberKind::RimJoist => {
            let midpoint_x = (start.x + end.x) * 0.5;
            let midpoint_y = (start.y + end.y) * 0.5;
            let direction = if frame.up_slope_distance(midpoint_x, midpoint_y) <= thickness * 0.5 {
                1.0
            } else {
                -1.0
            };
            start.x += up_x * thickness * 0.5 * direction;
            start.y += up_y * thickness * 0.5 * direction;
            end.x += up_x * thickness * 0.5 * direction;
            end.y += up_y * thickness * 0.5 * direction;
        }
        _ => {}
    }
    spatial_board_solid_between(member, start, end)
}

fn horizontal_surface_member_solid(
    outline: &[Point2],
    span: SpanDirection,
    member: &FrameMember,
    surface_z: f64,
    section_sign: f64,
) -> Result<PhysicalSolid, String> {
    let layout = SurfaceLayout::new(outline, span)
        .ok_or_else(|| "surface member has a degenerate host outline".to_string())?;
    let half = member.cross_section_depth.inches() / 2.0;
    let (u0, u1, v0, v1) = match member.orientation {
        MemberOrientation::Vertical => (
            member.elevation.inches() + half,
            (member.elevation + member.cut_length).inches()
                - member.cross_section_depth.inches()
                - half,
            member.x.inches() - half,
            member.x.inches() + half,
        ),
        MemberOrientation::Horizontal => (
            member.elevation.inches() - half,
            member.elevation.inches() + half,
            member.x.inches(),
            (member.x + member.cut_length).inches(),
        ),
    };
    let z0 = surface_z + section_sign * member.side_offset.inches();
    let z1 = surface_z + section_sign * (member.side_offset + member.side_depth).inches();
    let (z0, z1) = (z0.min(z1), z0.max(z1));
    let point = |u: f64, v: f64, z: f64| layout.point(u, v, z);
    cuboid_solid([
        point(u0, v0, z0),
        point(u1, v0, z0),
        point(u1, v1, z0),
        point(u0, v1, z0),
        point(u0, v0, z1),
        point(u1, v0, z1),
        point(u1, v1, z1),
        point(u0, v1, z1),
    ])
}

fn spatial_board_solid(member: &FrameMember) -> Result<PhysicalSolid, String> {
    let sloped = member
        .sloped
        .ok_or_else(|| "spatial member lacks endpoint placement".to_string())?;
    spatial_board_solid_between(
        member,
        Point3::new(
            sloped.start.x.inches(),
            sloped.start.y.inches(),
            sloped.low_elevation.inches(),
        ),
        Point3::new(
            sloped.end.x.inches(),
            sloped.end.y.inches(),
            sloped.high_elevation.inches(),
        ),
    )
}

fn spatial_board_solid_between(
    member: &FrameMember,
    start: Point3,
    end: Point3,
) -> Result<PhysicalSolid, String> {
    board_prism(
        start,
        end,
        None,
        -member.cross_section_depth.inches() / 2.0,
        member.cross_section_depth.inches() / 2.0,
        member.side_offset.inches(),
        (member.side_offset + member.side_depth).inches(),
    )
}

fn rake_plate_solid(
    wall: &Wall,
    member: &FrameMember,
    total: Length,
    interior_sign: f64,
) -> Result<PhysicalSolid, String> {
    let sloped = member
        .sloped
        .ok_or_else(|| "rake plate lacks endpoint placement".to_string())?;
    let basis = WallBasis::new(wall);
    let (side0, side1) =
        layer_band_span(interior_sign, total, member.side_offset, member.side_depth);
    board_prism(
        Point3::new(
            sloped.start.x.inches(),
            sloped.start.y.inches(),
            sloped.low_elevation.inches(),
        ),
        Point3::new(
            sloped.end.x.inches(),
            sloped.end.y.inches(),
            sloped.high_elevation.inches(),
        ),
        Some(Point3::new(basis.side_x, basis.side_y, 0.0)),
        side0,
        side1,
        -member.cross_section_depth.inches(),
        0.0,
    )
}

fn board_prism(
    start: Point3,
    end: Point3,
    across: Option<Point3>,
    across0: f64,
    across1: f64,
    section0: f64,
    section1: f64,
) -> Result<PhysicalSolid, String> {
    if across1 - across0 <= f64::EPSILON || section1 - section0 <= f64::EPSILON {
        return Err("board prism has a nonpositive cross section".into());
    }
    let along = normalized(vector_between(start, end))
        .ok_or_else(|| "board prism has coincident endpoints".to_string())?;
    let across = match across {
        Some(axis) => normalized(axis),
        None => {
            let plan_length = (along.x * along.x + along.y * along.y).sqrt();
            if plan_length > f64::EPSILON {
                Some(Point3::new(
                    -along.y / plan_length,
                    along.x / plan_length,
                    0.0,
                ))
            } else {
                Some(Point3::X)
            }
        }
    }
    .ok_or_else(|| "board prism has no across axis".to_string())?;
    let mut section = normalized(cross(along, across))
        .ok_or_else(|| "board prism axes are parallel".to_string())?;
    if section.z < 0.0 {
        section = Point3::new(-section.x, -section.y, -section.z);
    }
    let point = |origin: Point3, across_offset: f64, section_offset: f64| {
        offset(
            offset(origin, across, across_offset),
            section,
            section_offset,
        )
    };
    cuboid_solid([
        point(start, across0, section0),
        point(end, across0, section0),
        point(end, across1, section0),
        point(start, across1, section0),
        point(start, across0, section1),
        point(end, across0, section1),
        point(end, across1, section1),
        point(start, across1, section1),
    ])
}

pub struct RafterPrism {
    pub solid: PhysicalSolid,
    pub profile: Vec<[f64; 2]>,
}

pub fn build_common_rafter_solid(
    member: &FrameMember,
    plane: &RoofPlane,
    bearing_depth: Option<f64>,
    ridge_face_setback: Option<f64>,
) -> Result<RafterPrism, String> {
    const MAX_NOTCH_DEPTH_FRACTION: f64 = 1.0 / 3.0;
    let sloped = member
        .sloped
        .ok_or_else(|| "common rafter lacks endpoint placement".to_string())?;
    let start = Point3::new(
        sloped.start.x.inches(),
        sloped.start.y.inches(),
        sloped.low_elevation.inches(),
    );
    let end = Point3::new(
        sloped.end.x.inches(),
        sloped.end.y.inches(),
        sloped.high_elevation.inches(),
    );
    let board_thickness = member.cross_section_depth.inches();
    let section0 = member.side_offset.inches();
    let section1 = (member.side_offset + member.side_depth).inches();
    if board_thickness <= f64::EPSILON || section1 - section0 <= f64::EPSILON {
        return Err("common rafter has a nonpositive cross section".into());
    }
    let plan_dx = end.x - start.x;
    let plan_dy = end.y - start.y;
    let plan_run = (plan_dx * plan_dx + plan_dy * plan_dy).sqrt();
    if plan_run <= f64::EPSILON {
        return Err("common rafter has no plan run".into());
    }
    let profile_run = plan_run - ridge_face_setback.unwrap_or(0.0);
    if profile_run <= f64::EPSILON {
        return Err("ridge-face setback consumes the common rafter".into());
    }
    let run = Point3::new(plan_dx / plan_run, plan_dy / plan_run, 0.0);
    let across = Point3::new(-run.y, run.x, 0.0);
    let rise_over_run = (end.z - start.z) / plan_run;
    let slope_cosine = 1.0 / (1.0 + rise_over_run * rise_over_run).sqrt();
    let lower_z = |u: f64| start.z + rise_over_run * u + section0 / slope_cosine;
    let upper_z = |u: f64| start.z + rise_over_run * u + section1 / slope_cosine;

    let mut profile = vec![[0.0, lower_z(0.0)]];
    if let (Some(frame), Some(bearing_depth)) = (plane.frame(), bearing_depth)
        && rise_over_run > f64::EPSILON
        && bearing_depth > f64::EPSILON
    {
        let bearing_run = -frame.up_slope_distance(start.x, start.y);
        let max_notch_depth = (section1 - section0) / slope_cosine * MAX_NOTCH_DEPTH_FRACTION;
        let seat_run = bearing_depth.min(max_notch_depth / rise_over_run);
        let heel_run = (bearing_run - seat_run).max(0.0);
        let toe_run = bearing_run.min(profile_run);
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
        [profile_run, lower_z(profile_run)],
        [profile_run, upper_z(profile_run)],
        [0.0, upper_z(0.0)],
    ]);

    let local_outline: Vec<Point2> = profile
        .iter()
        .map(|[u, z]| Point2::new(Length::from_inches(*u), Length::from_inches(*z)))
        .collect();
    let end_triangles = framer_core::triangulate_simple_polygon(&local_outline);
    if end_triangles.len() + 2 != profile.len() {
        return Err("common-rafter profile could not be triangulated".into());
    }

    let half_thickness = board_thickness / 2.0;
    let world_point = |[u, z]: [f64; 2], across_offset: f64| {
        Point3::new(
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
    let surface = TriMesh { points, triangles };

    let mut pieces = Vec::with_capacity(end_triangles.len());
    for [a, b, c] in end_triangles {
        let prism = triangular_prism(
            [profile[a], profile[b], profile[c]],
            &world_point,
            half_thickness,
        )?;
        pieces.extend(prism.convex_pieces);
    }
    let solid = PhysicalSolid::new(surface, pieces)
        .ok_or_else(|| "common rafter has no physical solid".to_string())?;
    Ok(RafterPrism { solid, profile })
}

fn triangular_prism(
    triangle: [[f64; 2]; 3],
    world_point: &impl Fn([f64; 2], f64) -> Point3,
    half_thickness: f64,
) -> Result<PhysicalSolid, String> {
    let [a, b, c] = triangle;
    let points = vec![
        world_point(a, -half_thickness),
        world_point(b, -half_thickness),
        world_point(c, -half_thickness),
        world_point(a, half_thickness),
        world_point(b, half_thickness),
        world_point(c, half_thickness),
    ];
    let triangles = vec![
        [0, 2, 1],
        [3, 4, 5],
        [0, 1, 4],
        [0, 4, 3],
        [1, 2, 5],
        [1, 5, 4],
        [2, 0, 3],
        [2, 3, 5],
    ];
    let mesh = TriMesh { points, triangles };
    let piece = crate::ConvexPiece::new(mesh.clone())
        .ok_or_else(|| "rafter profile triangle is degenerate".to_string())?;
    PhysicalSolid::new(mesh, vec![piece])
        .ok_or_else(|| "rafter profile triangle has no solid".into())
}

pub fn ridge_face_setback(member: &FrameMember, ridge_boards: &[&FrameMember]) -> Option<f64> {
    let rafter = member.sloped?;
    let rafter_dx = (rafter.end.x - rafter.start.x).inches();
    let rafter_dy = (rafter.end.y - rafter.start.y).inches();
    let rafter_run = (rafter_dx * rafter_dx + rafter_dy * rafter_dy).sqrt();
    if rafter_run <= f64::EPSILON {
        return None;
    }
    let rafter_unit = (rafter_dx / rafter_run, rafter_dy / rafter_run);
    let one_tick = Length::from_ticks(1);
    ridge_boards.iter().find_map(|ridge| {
        let ridge = *ridge;
        let placement = ridge.sloped?;
        if (placement.low_elevation - rafter.high_elevation).abs() > one_tick
            || !point_on_plan_segment(rafter.end, placement.start, placement.end, one_tick)
        {
            return None;
        }
        let ridge_dx = (placement.end.x - placement.start.x).inches();
        let ridge_dy = (placement.end.y - placement.start.y).inches();
        let ridge_length = (ridge_dx * ridge_dx + ridge_dy * ridge_dy).sqrt();
        if ridge_length <= f64::EPSILON {
            return None;
        }
        let ridge_across = (-ridge_dy / ridge_length, ridge_dx / ridge_length);
        let approach = (rafter_unit.0 * ridge_across.0 + rafter_unit.1 * ridge_across.1).abs();
        if approach <= f64::EPSILON {
            return None;
        }
        let setback = ridge.cross_section_depth.inches() * 0.5 / approach;
        (setback < rafter_run).then_some(setback)
    })
}

fn point_on_plan_segment(point: Point2, start: Point2, end: Point2, tolerance: Length) -> bool {
    let px = point.x.inches();
    let py = point.y.inches();
    let ax = start.x.inches();
    let ay = start.y.inches();
    let dx = (end.x - start.x).inches();
    let dy = (end.y - start.y).inches();
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f64::EPSILON {
        return false;
    }
    let distance = ((px - ax) * dy - (py - ay) * dx).abs() / length_squared.sqrt();
    let projection = (px - ax) * dx + (py - ay) * dy;
    let along_tolerance = tolerance.inches() * length_squared.sqrt();
    distance <= tolerance.inches()
        && projection >= -along_tolerance
        && projection <= length_squared + along_tolerance
}

fn roof_member_family(model: &BuildingModel, plane: &RoofPlane) -> Option<MemberFamily> {
    model
        .systems
        .iter()
        .find(|system| system.id == plane.system)
        .and_then(ConstructionSystem::framing_layer)
        .and_then(|layer| layer.framing.as_ref())
        .map(|framing| framing.member_family)
}

pub fn matched_bearing_depth(model: &BuildingModel, plane: &RoofPlane) -> Option<Length> {
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

pub(super) struct WallBasis {
    origin_x: f64,
    origin_y: f64,
    along_x: f64,
    along_y: f64,
    side_x: f64,
    side_y: f64,
}

impl WallBasis {
    pub(super) fn new(wall: &Wall) -> Self {
        let dx = (wall.end.x - wall.start.x).inches();
        let dy = (wall.end.y - wall.start.y).inches();
        let length = (dx * dx + dy * dy).sqrt().max(1.0);
        let along_x = dx / length;
        let along_y = dy / length;
        Self {
            origin_x: wall.start.x.inches(),
            origin_y: wall.start.y.inches(),
            along_x,
            along_y,
            side_x: -along_y,
            side_y: along_x,
        }
    }

    pub(super) fn point(&self, local_x: f64, side: f64, z: f64) -> Point3 {
        Point3::new(
            self.origin_x + self.along_x * local_x + self.side_x * side,
            self.origin_y + self.along_y * local_x + self.side_y * side,
            z,
        )
    }

    pub(super) fn cuboid(
        &self,
        x0: f64,
        x1: f64,
        side0: f64,
        side1: f64,
        z0: f64,
        z1: f64,
    ) -> [Point3; 8] {
        [
            self.point(x0, side0, z0),
            self.point(x1, side0, z0),
            self.point(x1, side1, z0),
            self.point(x0, side1, z0),
            self.point(x0, side0, z1),
            self.point(x1, side0, z1),
            self.point(x1, side1, z1),
            self.point(x0, side1, z1),
        ]
    }
}

fn interior_sign(interior_sides: &BTreeMap<ElementId, bool>, wall_id: &ElementId) -> f64 {
    match interior_sides.get(wall_id) {
        Some(true) => 1.0,
        _ => -1.0,
    }
}

fn layer_band_span(
    interior_sign: f64,
    total: Length,
    off: Length,
    thickness: Length,
) -> (f64, f64) {
    let half = total.inches() / 2.0;
    let side_a = interior_sign * (half - off.inches());
    let side_b = interior_sign * (half - (off + thickness).inches());
    (side_a.min(side_b), side_a.max(side_b))
}

struct SurfaceLayout {
    min_x: f64,
    min_y: f64,
    span_along_x: bool,
}

impl SurfaceLayout {
    fn new(outline: &[Point2], span: SpanDirection) -> Option<Self> {
        let first = outline.first()?;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (first.x, first.y, first.x, first.y);
        for point in &outline[1..] {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
        let width = max_x - min_x;
        let depth = max_y - min_y;
        if width <= Length::ZERO || depth <= Length::ZERO {
            return None;
        }
        let span_along_x = match span {
            SpanDirection::Shorter | SpanDirection::Across => width <= depth,
            SpanDirection::Along => width >= depth,
            SpanDirection::Explicit(direction) => direction.x.abs() >= direction.y.abs(),
        };
        Some(Self {
            min_x: min_x.inches(),
            min_y: min_y.inches(),
            span_along_x,
        })
    }

    fn point(&self, span: f64, layout: f64, z: f64) -> Point3 {
        if self.span_along_x {
            Point3::new(self.min_x + span, self.min_y + layout, z)
        } else {
            Point3::new(self.min_x + layout, self.min_y + span, z)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_shell() -> BuildingModel {
        framer_core::load_project(include_str!(
            "../../../../examples/projects/demo-shell.framer"
        ))
        .unwrap()
    }

    #[test]
    fn common_rafter_keeps_birdsmouth_and_ridge_face_cut_in_both_fields() {
        let mut model = example_shell();
        for plane in &mut model.roof_planes {
            plane.eave_overhang = Length::from_whole_inches(12);
        }
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let ridges: Vec<_> = plan
            .roof_plans
            .iter()
            .flat_map(|plan| &plan.members)
            .filter(|member| member.kind == MemberKind::RidgeBoard)
            .collect();
        let mut profiles = Vec::new();
        for roof_plan in &plan.roof_plans {
            let plane = model
                .roof_planes
                .iter()
                .find(|plane| plane.id == roof_plan.roof)
                .unwrap();
            let Some(member) = roof_plan
                .members
                .iter()
                .find(|member| member.kind == MemberKind::Rafter)
            else {
                continue;
            };
            let Some(setback) = ridge_face_setback(member, &ridges) else {
                continue;
            };
            let Some(bearing) = matched_bearing_depth(&model, plane) else {
                continue;
            };
            assert!((setback - 0.75).abs() < 1.0e-6);
            let built =
                build_common_rafter_solid(member, plane, Some(bearing.inches()), Some(setback))
                    .unwrap();
            assert_eq!(built.profile.len(), 7);
            assert!((built.profile[1][0] - built.profile[2][0]).abs() < 1.0e-6);
            assert!((built.profile[2][1] - built.profile[3][1]).abs() < 1.0e-6);
            assert_eq!(built.solid.convex_pieces.len(), built.profile.len() - 2);
            profiles.push(built.profile);
        }
        assert!(profiles.len() >= 2, "both gable fields produce cut rafters");
    }

    #[test]
    fn unmatched_bearing_falls_back_to_an_uncut_board_profile() {
        let model = example_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let roof_plan = plan.roof_plans.first().unwrap();
        let plane = model
            .roof_planes
            .iter()
            .find(|plane| plane.id == roof_plan.roof)
            .unwrap();
        let member = roof_plan
            .members
            .iter()
            .find(|member| member.kind == MemberKind::Rafter)
            .unwrap();
        let built = build_common_rafter_solid(member, plane, None, None).unwrap();
        assert_eq!(built.profile.len(), 4);
    }

    #[test]
    fn roof_face_setbacks_ignore_plan_coincident_members_on_other_elevations() {
        let model = example_shell();
        let plan = framer_solver::generate_project_plan(&model).unwrap();
        let members: Vec<_> = plan
            .roof_plans
            .iter()
            .flat_map(|plan| &plan.members)
            .collect();
        let (jack, hip) = members
            .iter()
            .filter(|member| member.kind == MemberKind::JackRafter)
            .find_map(|jack| {
                members
                    .iter()
                    .filter(|member| member.kind == MemberKind::HipRafter)
                    .find(|hip| endpoint_face_setback(jack, hip, true).is_some())
                    .map(|hip| (*jack, *hip))
            })
            .expect("the hip example has a jack terminating at a hip");
        let mut raised = hip.clone();
        let placement = raised.sloped.as_mut().unwrap();
        placement.low_elevation += Length::from_feet(10.0);
        placement.high_elevation += Length::from_feet(10.0);
        assert!(endpoint_face_setback(jack, &raised, true).is_none());
    }
}
