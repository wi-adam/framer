use std::collections::BTreeMap;
use std::fmt::Write;

use framer_core::{
    BoardProfile, BuildingModel, CodeProfile, ElementId, Length, ModelError, Opening, Point2, Wall,
    WallJoinKind,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WallFramePlan {
    pub wall: ElementId,
    pub members: Vec<FrameMember>,
    pub diagnostics: Vec<PlanDiagnostic>,
}

impl WallFramePlan {
    pub fn bom(&self) -> Vec<BomItem> {
        bom_from_members(self.members.iter())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFramePlan {
    pub wall_plans: Vec<WallFramePlan>,
    pub diagnostics: Vec<PlanDiagnostic>,
}

impl ProjectFramePlan {
    pub fn bom(&self) -> Vec<BomItem> {
        bom_from_members(
            self.wall_plans
                .iter()
                .flat_map(|wall_plan| wall_plan.members.iter()),
        )
    }

    pub fn wall_plan(&self, wall: &ElementId) -> Option<&WallFramePlan> {
        self.wall_plans
            .iter()
            .find(|wall_plan| wall_plan.wall == *wall)
    }

    pub fn wall_plan_mut(&mut self, wall: &ElementId) -> Option<&mut WallFramePlan> {
        self.wall_plans
            .iter_mut()
            .find(|wall_plan| wall_plan.wall == *wall)
    }
}

fn bom_from_members<'a>(members: impl IntoIterator<Item = &'a FrameMember>) -> Vec<BomItem> {
    let mut grouped: BTreeMap<(BoardProfile, MemberKind, Length), u32> = BTreeMap::new();
    for member in members {
        *grouped
            .entry((member.profile, member.kind, member.cut_length))
            .or_default() += 1;
    }

    grouped
        .into_iter()
        .map(|((profile, kind, cut_length), quantity)| BomItem {
            profile,
            kind,
            cut_length,
            quantity,
            total_length: cut_length * quantity as i64,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameMember {
    pub id: String,
    pub source: ElementId,
    pub kind: MemberKind,
    pub profile: BoardProfile,
    pub orientation: MemberOrientation,
    pub x: Length,
    pub elevation: Length,
    pub cut_length: Length,
    pub cross_section_depth: Length,
    pub provenance: RuleProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleProvenance {
    pub rule_id: String,
    pub summary: String,
}

impl RuleProvenance {
    fn new(rule_id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.into(),
            summary: summary.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MemberKind {
    BottomPlate,
    TopPlate,
    CornerPost,
    CommonStud,
    KingStud,
    JackStud,
    Header,
    RoughSill,
    CrippleStud,
}

impl MemberKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::BottomPlate => "bottom plate",
            Self::TopPlate => "top plate",
            Self::CornerPost => "corner post",
            Self::CommonStud => "common stud",
            Self::KingStud => "king stud",
            Self::JackStud => "jack stud",
            Self::Header => "header",
            Self::RoughSill => "rough sill",
            Self::CrippleStud => "cripple stud",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemberOrientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BomItem {
    pub profile: BoardProfile,
    pub kind: MemberKind,
    pub cut_length: Length,
    pub quantity: u32,
    pub total_length: Length,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub source: Option<ElementId>,
    pub message: String,
}

impl PlanDiagnostic {
    fn new(
        severity: DiagnosticSeverity,
        code: impl Into<String>,
        source: Option<ElementId>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            source,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Unsupported,
}

pub fn generate_wall_plan(wall: &Wall, code: &CodeProfile) -> Result<WallFramePlan, SolverError> {
    wall.validate()?;

    let mut members = Vec::new();
    let mut diagnostics = starter_profile_diagnostics(wall, code);
    let plate_thickness = code.plate_profile.thickness();
    let stud_thickness = code.stud_profile.thickness();
    let top_plate_count = if code.double_top_plate { 2 } else { 1 };
    let stud_top = wall.height - plate_thickness * top_plate_count as i64;
    let stud_length = stud_top - plate_thickness;

    if stud_length <= Length::ZERO {
        return Err(SolverError::WallTooShortForPlateStack {
            wall: wall.id.clone(),
        });
    }

    for opening in &wall.openings {
        if opening.left() < stud_thickness * 2 || opening.right() + stud_thickness * 2 > wall.length
        {
            return Err(SolverError::OpeningTooCloseToWallEnd {
                wall: wall.id.clone(),
                opening: opening.id.clone(),
            });
        }

        if opening.top() >= stud_top {
            return Err(SolverError::OpeningLeavesNoHeaderSpace {
                wall: wall.id.clone(),
                opening: opening.id.clone(),
            });
        }
    }

    members.push(frame_member(
        "bottom-plate-1",
        &wall.id,
        MemberKind::BottomPlate,
        code.plate_profile,
        FrameMemberPlacement::new(
            MemberOrientation::Horizontal,
            Length::ZERO,
            Length::ZERO,
            wall.length,
            code.plate_profile.thickness(),
        ),
        RuleProvenance::new(
            "wall.plate.continuous",
            "Bottom plate runs the authored wall length using the configured plate profile.",
        ),
    ));

    for index in 0..top_plate_count {
        members.push(frame_member(
            format!("top-plate-{}", index + 1),
            &wall.id,
            MemberKind::TopPlate,
            code.plate_profile,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                Length::ZERO,
                wall.height - plate_thickness * (index as i64 + 1),
                wall.length,
                code.plate_profile.thickness(),
            ),
            RuleProvenance::new(
                "wall.plate.double-top",
                format!(
                    "Top plate {} of {} uses the wall length and configured double-top-plate setting.",
                    index + 1,
                    top_plate_count
                ),
            ),
        ));
    }

    let stud_base = plate_thickness;
    for x in stud_positions(wall.length, wall.stud_spacing, stud_thickness) {
        if !is_inside_opening_framing_assembly(x, &wall.openings, stud_thickness) {
            members.push(frame_member(
                format!("stud-{}", x.ticks()),
                &wall.id,
                MemberKind::CommonStud,
                code.stud_profile,
                FrameMemberPlacement::new(
                    MemberOrientation::Vertical,
                    x,
                    stud_base,
                    stud_length,
                    code.stud_profile.thickness(),
                ),
                RuleProvenance::new(
                    "wall.studs.on-center",
                    format!(
                        "End studs align with wall faces, interior common studs are placed at {} layout marks, and authored opening framing assemblies are kept clear.",
                        wall.stud_spacing
                    ),
                ),
            ));
        }
    }

    let mut openings = wall.openings.clone();
    openings.sort_by_key(Opening::left);
    for opening in openings {
        add_opening_members(
            &mut members,
            &mut diagnostics,
            wall,
            code,
            &opening,
            top_plate_count,
        );
    }

    Ok(WallFramePlan {
        wall: wall.id.clone(),
        members,
        diagnostics,
    })
}

pub fn generate_project_plan(model: &BuildingModel) -> Result<ProjectFramePlan, SolverError> {
    model.validate()?;

    let mut plan = ProjectFramePlan {
        wall_plans: Vec::with_capacity(model.walls.len()),
        diagnostics: project_diagnostics(model),
    };

    for wall in &model.walls {
        plan.wall_plans.push(generate_wall_plan(wall, &model.code)?);
    }

    add_join_members(&mut plan, model)?;

    Ok(plan)
}

fn project_diagnostics(model: &BuildingModel) -> Vec<PlanDiagnostic> {
    let mut diagnostics = Vec::new();
    if model.walls.len() > 1 {
        diagnostics.push(PlanDiagnostic::new(
            DiagnosticSeverity::Info,
            "solver.scope.multi-wall-shell-alpha",
            None,
            format!(
                "Project framing is generated across {} connected wall segments and {} authored wall joins; floor, roof, shear, and engineered load-path design remain future work.",
                model.walls.len(),
                model.wall_joins.len()
            ),
        ));
    }

    diagnostics
}

fn add_join_members(plan: &mut ProjectFramePlan, model: &BuildingModel) -> Result<(), SolverError> {
    let plate_thickness = model.code.plate_profile.thickness();
    let stud_base = plate_thickness;
    let top_plate_count = if model.code.double_top_plate { 2 } else { 1 };

    for join in &model.wall_joins {
        if !matches!(join.kind, WallJoinKind::Corner | WallJoinKind::EndToEnd) {
            plan.diagnostics.push(PlanDiagnostic::new(
                DiagnosticSeverity::Unsupported,
                "wall.join.unsupported-kind",
                Some(join.id.clone()),
                format!(
                    "{} is stored as an authored {:?} join, but only corner/end-to-end join framing is generated in this alpha.",
                    join.name, join.kind
                ),
            ));
            continue;
        }

        for wall_id in [&join.first_wall, &join.second_wall] {
            let wall = model
                .walls
                .iter()
                .find(|candidate| candidate.id == *wall_id)
                .ok_or_else(|| SolverError::MissingWallForJoin {
                    join: join.id.clone(),
                    wall: wall_id.clone(),
                })?;
            let join_x = wall.local_x_for_point(join.point).ok_or_else(|| {
                SolverError::JoinPointOutsideWall {
                    join: join.id.clone(),
                    wall: wall.id.clone(),
                }
            })?;
            let post_x =
                face_aligned_center(join_x, wall.length, model.code.stud_profile.thickness());
            let stud_top = wall.height - plate_thickness * top_plate_count as i64;
            let stud_length = stud_top - stud_base;

            if stud_length <= Length::ZERO {
                return Err(SolverError::WallTooShortForPlateStack {
                    wall: wall.id.clone(),
                });
            }

            let wall_plan =
                plan.wall_plan_mut(&wall.id)
                    .ok_or_else(|| SolverError::MissingWallPlan {
                        wall: wall.id.clone(),
                    })?;
            wall_plan.members.push(frame_member(
                format!("{}-{}-corner-post", join.id.0, wall.id.0),
                &join.id,
                MemberKind::CornerPost,
                model.code.stud_profile,
                FrameMemberPlacement::new(
                    MemberOrientation::Vertical,
                    post_x,
                    stud_base,
                    stud_length,
                    model.code.stud_profile.thickness(),
                ),
                RuleProvenance::new(
                    "wall.join.corner-posts",
                    format!(
                        "A corner post is generated on {} with its faces inside the wall edge at {} to make the authored {} wall join visible in the project framing plan.",
                        wall.name,
                        post_x,
                        join.name
                    ),
                ),
            ));
        }
    }

    Ok(())
}

fn face_aligned_center(x: Length, length: Length, member_depth: Length) -> Length {
    if length <= member_depth {
        return length / 2;
    }

    let half_depth = member_depth / 2;
    x.max(half_depth).min(length - half_depth)
}

fn add_opening_members(
    members: &mut Vec<FrameMember>,
    diagnostics: &mut Vec<PlanDiagnostic>,
    wall: &Wall,
    code: &CodeProfile,
    opening: &Opening,
    top_plate_count: usize,
) {
    let plate_thickness = code.plate_profile.thickness();
    let stud_base = plate_thickness;
    let stud_top = wall.height - plate_thickness * top_plate_count as i64;
    let header_bottom = opening.top();
    let header_depth = code.default_header_depth.min(stud_top - header_bottom);
    let header_top = header_bottom + header_depth;
    let left = opening.left();
    let right = opening.right();
    let stud_thickness = code.stud_profile.thickness();
    let side_positions = OpeningSidePositions::new(left, right, stud_thickness);

    if header_depth < code.default_header_depth {
        diagnostics.push(PlanDiagnostic::new(
            DiagnosticSeverity::Warning,
            "opening.header.depth-clipped",
            Some(opening.id.clone()),
            format!(
                "Header depth for {} is clipped to {} because the opening top is close to the top plates.",
                opening.name, header_depth
            ),
        ));
    }

    for (side, king_x, jack_x) in [
        ("left", side_positions.left_king, side_positions.left_jack),
        (
            "right",
            side_positions.right_king,
            side_positions.right_jack,
        ),
    ] {
        members.push(frame_member(
            format!("{}-king-{}", opening.id.0, side),
            &opening.id,
            MemberKind::KingStud,
            code.stud_profile,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                king_x,
                stud_base,
                stud_top - stud_base,
                code.stud_profile.thickness(),
            ),
            RuleProvenance::new(
                "opening.king-studs.each-side",
                format!(
                    "A king stud is generated at the {side} rough opening edge for {}.",
                    opening.name
                ),
            ),
        ));

        members.push(frame_member(
            format!("{}-jack-{}", opening.id.0, side),
            &opening.id,
            MemberKind::JackStud,
            code.stud_profile,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                jack_x,
                stud_base,
                header_bottom - stud_base,
                code.stud_profile.thickness(),
            ),
            RuleProvenance::new(
                "opening.jack-studs.header-bearing",
                format!(
                    "A jack stud is generated at the {side} rough opening edge to support the starter-profile header.",
                ),
            ),
        ));
    }

    members.push(frame_member(
        format!("{}-header", opening.id.0),
        &opening.id,
        MemberKind::Header,
        code.header_profile,
        FrameMemberPlacement::new(
            MemberOrientation::Horizontal,
            side_positions.left_jack_left_face,
            header_bottom,
            opening.width + stud_thickness * 2,
            header_depth,
        ),
        RuleProvenance::new(
            "opening.header.default-profile",
            format!(
                "Header uses the configured starter profile {} with default depth {}; no span/load lookup is performed yet.",
                code.header_profile.label(),
                header_depth
            ),
        ),
    ));

    if opening.has_sill() {
        members.push(frame_member(
            format!("{}-sill", opening.id.0),
            &opening.id,
            MemberKind::RoughSill,
            code.stud_profile,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                left,
                opening.sill_height,
                opening.width,
                code.stud_profile.thickness(),
            ),
            RuleProvenance::new(
                "opening.window.rough-sill",
                format!(
                    "A rough sill is generated for {} because this opening type has a sill height.",
                    opening.name
                ),
            ),
        ));

        add_cripples(
            members,
            code,
            opening,
            "lower",
            plate_thickness,
            opening.sill_height,
        );
    }

    add_cripples(members, code, opening, "upper", header_top, stud_top);
}

struct OpeningSidePositions {
    left_king: Length,
    left_jack: Length,
    left_jack_left_face: Length,
    right_jack: Length,
    right_king: Length,
}

impl OpeningSidePositions {
    fn new(opening_left: Length, opening_right: Length, stud_thickness: Length) -> Self {
        let half_stud = stud_thickness / 2;
        Self {
            left_king: opening_left - stud_thickness - half_stud,
            left_jack: opening_left - half_stud,
            left_jack_left_face: opening_left - stud_thickness,
            right_jack: opening_right + half_stud,
            right_king: opening_right + stud_thickness + half_stud,
        }
    }
}

fn add_cripples(
    members: &mut Vec<FrameMember>,
    code: &CodeProfile,
    opening: &Opening,
    label: &str,
    bottom: Length,
    top: Length,
) {
    let cut_length = top - bottom;
    if cut_length <= Length::ZERO {
        return;
    }

    for x in cripple_positions(opening.left(), opening.right(), code.default_stud_spacing) {
        members.push(frame_member(
            format!("{}-cripple-{}-{}", opening.id.0, label, x.ticks()),
            &opening.id,
            MemberKind::CrippleStud,
            code.stud_profile,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                x,
                bottom,
                cut_length,
                code.stud_profile.thickness(),
            ),
            RuleProvenance::new(
                "opening.cripples.on-center",
                format!(
                    "{} cripple studs are generated across {} at the default {} spacing where clear span allows.",
                    title_case(label),
                    opening.name,
                    code.default_stud_spacing
                ),
            ),
        ));
    }
}

struct FrameMemberPlacement {
    orientation: MemberOrientation,
    x: Length,
    elevation: Length,
    cut_length: Length,
    cross_section_depth: Length,
}

impl FrameMemberPlacement {
    fn new(
        orientation: MemberOrientation,
        x: Length,
        elevation: Length,
        cut_length: Length,
        cross_section_depth: Length,
    ) -> Self {
        Self {
            orientation,
            x,
            elevation,
            cut_length,
            cross_section_depth,
        }
    }
}

fn frame_member(
    id: impl Into<String>,
    source: &ElementId,
    kind: MemberKind,
    profile: BoardProfile,
    placement: FrameMemberPlacement,
    provenance: RuleProvenance,
) -> FrameMember {
    FrameMember {
        id: id.into(),
        source: source.clone(),
        kind,
        profile,
        orientation: placement.orientation,
        x: placement.x,
        elevation: placement.elevation,
        cut_length: placement.cut_length,
        cross_section_depth: placement.cross_section_depth,
        provenance,
    }
}

fn starter_profile_diagnostics(wall: &Wall, code: &CodeProfile) -> Vec<PlanDiagnostic> {
    let mut diagnostics = vec![
        PlanDiagnostic::new(
            DiagnosticSeverity::Warning,
            "code-profile.starter-only",
            Some(wall.id.clone()),
            format!(
                "{} is starter rule data for deterministic framing defaults, not complete IRC compliance.",
                code.display_name
            ),
        ),
        PlanDiagnostic::new(
            DiagnosticSeverity::Info,
            "solver.scope.wall-segment-alpha",
            Some(wall.id.clone()),
            "This wall segment is framed with deterministic starter rules. Project generation aggregates connected wall segments and authored joins; engineered load paths, hold-downs, floors, and roofs remain future work.",
        ),
    ];

    for opening in &wall.openings {
        if matches!(opening.kind, framer_core::OpeningKind::GarageDoor) {
            diagnostics.push(PlanDiagnostic::new(
                DiagnosticSeverity::Unsupported,
                "opening.garage-door.starter-header",
                Some(opening.id.clone()),
                format!(
                    "{} is modeled as a wide rough opening with starter-profile king, jack, and header members; garage-door-specific structural design is unsupported.",
                    opening.name
                ),
            ));
        }
    }

    diagnostics
}

fn stud_positions(length: Length, spacing: Length, stud_thickness: Length) -> Vec<Length> {
    if length <= stud_thickness {
        return vec![length / 2];
    }

    let mut positions = Vec::new();
    let first_center = stud_thickness / 2;
    let last_center = length - first_center;

    positions.push(first_center);

    let mut x = spacing;
    while x < last_center {
        positions.push(x);
        x += spacing;
    }

    if positions.last().copied() != Some(last_center) {
        positions.push(last_center);
    }
    positions
}

fn cripple_positions(left: Length, right: Length, spacing: Length) -> Vec<Length> {
    let mut positions = Vec::new();
    let mut x = left + spacing;
    while x < right {
        positions.push(x);
        x += spacing;
    }
    positions
}

fn is_inside_opening_framing_assembly(
    x: Length,
    openings: &[Opening],
    stud_thickness: Length,
) -> bool {
    openings.iter().any(|opening| {
        x >= opening.left() - stud_thickness * 2 && x <= opening.right() + stud_thickness * 2
    })
}

pub fn export_bom_csv(bom: &[BomItem]) -> String {
    let mut items = bom.to_vec();
    items.sort_by_key(|item| (item.profile, item.kind, item.cut_length));

    let mut csv =
        "quantity,profile,kind,cut_length_inches,cut_length_display,total_length_inches,total_length_display\n"
            .to_owned();
    for item in items {
        let fields = [
            item.quantity.to_string(),
            item.profile.label().to_owned(),
            item.kind.label().to_owned(),
            decimal_inches(item.cut_length),
            item.cut_length.to_string(),
            decimal_inches(item.total_length),
            item.total_length.to_string(),
        ];

        csv.push_str(
            &fields
                .iter()
                .map(|field| csv_field(field))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push('\n');
    }
    csv
}

pub fn export_wall_elevation_svg(wall: &Wall, plan: &WallFramePlan) -> String {
    let width = wall.length.inches().max(1.0);
    let height = wall.height.inches().max(1.0);
    let margin = 12.0;
    let mut svg = String::new();

    writeln!(svg, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{:.4} {:.4} {:.4} {:.4}" width="{:.4}in" height="{:.4}in">"#,
        -margin,
        -margin,
        width + margin * 2.0,
        height + margin * 2.0,
        (width + margin * 2.0) / 12.0,
        (height + margin * 2.0) / 12.0
    )
    .unwrap();
    writeln!(
        svg,
        "  <title>{}</title>",
        escape_xml(&format!("{} framing elevation", wall.name))
    )
    .unwrap();
    writeln!(
        svg,
        "  <desc>Starter framing elevation generated from authored Framer intent. Not a permit drawing.</desc>"
    )
    .unwrap();
    writeln!(
        svg,
        r##"  <rect x="0" y="0" width="{:.4}" height="{:.4}" fill="#faf8f2" stroke="#bdb7aa" stroke-width="0.25"/>"##,
        width, height
    )
    .unwrap();

    for opening in &wall.openings {
        writeln!(
            svg,
            r##"  <rect data-opening="{}" x="{}" y="{}" width="{}" height="{}" fill="#ffffff" fill-opacity="0.28" stroke="#896634" stroke-width="0.25" stroke-dasharray="2 1">"##,
            escape_xml(&opening.id.0),
            svg_number(opening.left().inches()),
            svg_number(height - opening.top().inches()),
            svg_number(opening.width.inches()),
            svg_number(opening.height.inches())
        )
        .unwrap();
        writeln!(
            svg,
            "    <title>{} {}</title>",
            escape_xml(&format!("{:?}", opening.kind)),
            escape_xml(&opening.name)
        )
        .unwrap();
        writeln!(svg, "  </rect>").unwrap();
    }

    for member in &plan.members {
        let (x, y, member_width, member_height) = match member.orientation {
            MemberOrientation::Horizontal => (
                member.x.inches(),
                height - member.elevation.inches() - member.cross_section_depth.inches(),
                member.cut_length.inches(),
                member.cross_section_depth.inches(),
            ),
            MemberOrientation::Vertical => (
                member.x.inches() - member.cross_section_depth.inches() / 2.0,
                height - member.elevation.inches() - member.cut_length.inches(),
                member.cross_section_depth.inches(),
                member.cut_length.inches(),
            ),
        };

        writeln!(
            svg,
            r##"  <rect id="{}" x="{}" y="{}" width="{}" height="{}" fill="{}" stroke="#574634" stroke-width="0.18">"##,
            escape_xml(&member.id),
            svg_number(x),
            svg_number(y),
            svg_number(member_width),
            svg_number(member_height),
            member_svg_color(member.kind)
        )
        .unwrap();
        writeln!(
            svg,
            "    <title>{}: {} {}</title>",
            escape_xml(&member.id),
            escape_xml(member.profile.label()),
            escape_xml(member.kind.label())
        )
        .unwrap();
        writeln!(
            svg,
            "    <desc>{}</desc>",
            escape_xml(&member.provenance.summary)
        )
        .unwrap();
        writeln!(svg, "  </rect>").unwrap();
    }

    writeln!(
        svg,
        r##"  <text x="0" y="{:.4}" font-family="Arial, sans-serif" font-size="4" fill="#46433d">{} x {}</text>"##,
        height + 7.0,
        escape_xml(&wall.length.to_string()),
        escape_xml(&wall.height.to_string())
    )
    .unwrap();
    writeln!(svg, "</svg>").unwrap();
    svg
}

pub fn export_project_svg(model: &BuildingModel, plan: &ProjectFramePlan) -> String {
    let bounds = project_bounds(model);
    let width = (bounds.max.x - bounds.min.x).inches().max(1.0);
    let depth = (bounds.max.y - bounds.min.y).inches().max(1.0);
    let max_wall_height = model
        .walls
        .iter()
        .map(|wall| wall.height.inches())
        .fold(96.0, f64::max);
    let margin = 18.0;
    let plan_height = depth + margin * 2.0;
    let elevation_height = (max_wall_height + 26.0) * model.walls.len().max(1) as f64;
    let canvas_width = width.max(220.0) + margin * 2.0;
    let canvas_height = plan_height + elevation_height + margin;
    let mut svg = String::new();

    writeln!(svg, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" width="{}in" height="{}in">"#,
        svg_number(canvas_width),
        svg_number(canvas_height),
        svg_number(canvas_width / 12.0),
        svg_number(canvas_height / 12.0)
    )
    .unwrap();
    writeln!(svg, "  <title>Framer project shell plan</title>").unwrap();
    writeln!(
        svg,
        "  <desc>Whole-project alpha export generated from authored Framer intent. Not a permit drawing.</desc>"
    )
    .unwrap();
    writeln!(
        svg,
        r##"  <rect x="0" y="0" width="{}" height="{}" fill="#f7f5ee"/>"##,
        svg_number(canvas_width),
        svg_number(canvas_height)
    )
    .unwrap();

    writeln!(
        svg,
        r##"  <text x="{}" y="12" font-family="Arial, sans-serif" font-size="5" fill="#46433d">Plan</text>"##,
        svg_number(margin)
    )
    .unwrap();

    for join in &model.wall_joins {
        let point = project_svg_point(join.point, bounds.min, margin, plan_height - margin);
        writeln!(
            svg,
            r##"  <circle data-join="{}" cx="{}" cy="{}" r="2.5" fill="#2f5f7f">"##,
            escape_xml(&join.id.0),
            svg_number(point.0),
            svg_number(point.1)
        )
        .unwrap();
        writeln!(svg, "    <title>{}</title>", escape_xml(&join.name)).unwrap();
        writeln!(svg, "  </circle>").unwrap();
    }

    for wall in &model.walls {
        let start = project_svg_point(wall.start, bounds.min, margin, plan_height - margin);
        let end = project_svg_point(wall.end, bounds.min, margin, plan_height - margin);
        writeln!(
            svg,
            r##"  <line data-wall="{}" x1="{}" y1="{}" x2="{}" y2="{}" stroke="#6f5b3f" stroke-width="3" stroke-linecap="round">"##,
            escape_xml(&wall.id.0),
            svg_number(start.0),
            svg_number(start.1),
            svg_number(end.0),
            svg_number(end.1)
        )
        .unwrap();
        writeln!(svg, "    <title>{}</title>", escape_xml(&wall.name)).unwrap();
        writeln!(svg, "  </line>").unwrap();

        for opening in &wall.openings {
            let left = wall.point_at_local_x(opening.left());
            let right = wall.point_at_local_x(opening.right());
            let left = project_svg_point(left, bounds.min, margin, plan_height - margin);
            let right = project_svg_point(right, bounds.min, margin, plan_height - margin);
            writeln!(
                svg,
                r##"  <line data-opening="{}" x1="{}" y1="{}" x2="{}" y2="{}" stroke="#f7f5ee" stroke-width="5" stroke-linecap="butt"/>"##,
                escape_xml(&opening.id.0),
                svg_number(left.0),
                svg_number(left.1),
                svg_number(right.0),
                svg_number(right.1)
            )
            .unwrap();
            writeln!(
                svg,
                r##"  <line data-opening-edge="{}" x1="{}" y1="{}" x2="{}" y2="{}" stroke="#896634" stroke-width="1.25" stroke-linecap="butt"/>"##,
                escape_xml(&opening.id.0),
                svg_number(left.0),
                svg_number(left.1),
                svg_number(right.0),
                svg_number(right.1)
            )
            .unwrap();
        }
    }

    let mut elevation_y = plan_height + margin;
    for wall in &model.walls {
        if let Some(wall_plan) = plan.wall_plan(&wall.id) {
            writeln!(
                svg,
                r##"  <g data-wall-elevation="{}" transform="translate({}, {})">"##,
                escape_xml(&wall.id.0),
                svg_number(margin),
                svg_number(elevation_y)
            )
            .unwrap();
            writeln!(
                svg,
                r##"    <text x="0" y="-5" font-family="Arial, sans-serif" font-size="4.5" fill="#46433d">{}</text>"##,
                escape_xml(&wall.name)
            )
            .unwrap();
            write_wall_elevation_contents(&mut svg, wall, wall_plan, 0.45, "    ");
            writeln!(svg, "  </g>").unwrap();
            elevation_y += max_wall_height + 26.0;
        }
    }

    writeln!(svg, "</svg>").unwrap();
    svg
}

fn write_wall_elevation_contents(
    svg: &mut String,
    wall: &Wall,
    plan: &WallFramePlan,
    scale: f64,
    indent: &str,
) {
    let width = wall.length.inches().max(1.0) * scale;
    let height = wall.height.inches().max(1.0) * scale;
    writeln!(
        svg,
        r##"{indent}<rect x="0" y="0" width="{}" height="{}" fill="#faf8f2" stroke="#bdb7aa" stroke-width="0.25"/>"##,
        svg_number(width),
        svg_number(height)
    )
    .unwrap();

    for member in &plan.members {
        let (x, y, member_width, member_height) = match member.orientation {
            MemberOrientation::Horizontal => (
                member.x.inches() * scale,
                height
                    - member.elevation.inches() * scale
                    - member.cross_section_depth.inches() * scale,
                member.cut_length.inches() * scale,
                member.cross_section_depth.inches() * scale,
            ),
            MemberOrientation::Vertical => (
                member.x.inches() * scale - member.cross_section_depth.inches() * scale / 2.0,
                height - member.elevation.inches() * scale - member.cut_length.inches() * scale,
                member.cross_section_depth.inches() * scale,
                member.cut_length.inches() * scale,
            ),
        };
        writeln!(
            svg,
            r##"{indent}<rect data-member="{}" x="{}" y="{}" width="{}" height="{}" fill="{}" stroke="#574634" stroke-width="0.16"/>"##,
            escape_xml(&member.id),
            svg_number(x),
            svg_number(y),
            svg_number(member_width.max(0.75)),
            svg_number(member_height.max(0.75)),
            member_svg_color(member.kind)
        )
        .unwrap();
    }
}

fn member_svg_color(kind: MemberKind) -> &'static str {
    match kind {
        MemberKind::BottomPlate | MemberKind::TopPlate => "#635543",
        MemberKind::CornerPost => "#345f7f",
        MemberKind::CommonStud => "#ba915e",
        MemberKind::KingStud => "#97643d",
        MemberKind::JackStud => "#d3a85f",
        MemberKind::Header => "#738263",
        MemberKind::RoughSill => "#5c7990",
        MemberKind::CrippleStud => "#dabe8b",
    }
}

struct ProjectBounds {
    min: Point2,
    max: Point2,
}

fn project_bounds(model: &BuildingModel) -> ProjectBounds {
    let mut min_x = Length::ZERO;
    let mut min_y = Length::ZERO;
    let mut max_x = Length::ZERO;
    let mut max_y = Length::ZERO;
    let mut initialized = false;

    for point in model
        .walls
        .iter()
        .flat_map(|wall| [wall.start, wall.end])
        .chain(model.wall_joins.iter().map(|join| join.point))
    {
        if !initialized {
            min_x = point.x;
            min_y = point.y;
            max_x = point.x;
            max_y = point.y;
            initialized = true;
        } else {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
    }

    ProjectBounds {
        min: Point2::new(min_x, min_y),
        max: Point2::new(max_x, max_y),
    }
}

fn project_svg_point(point: Point2, min: Point2, margin: f64, baseline: f64) -> (f64, f64) {
    (
        margin + (point.x - min.x).inches(),
        baseline - (point.y - min.y).inches(),
    )
}

fn decimal_inches(length: Length) -> String {
    format!("{:.4}", length.inches())
}

fn svg_number(value: f64) -> String {
    let rounded = format!("{value:.4}");
    rounded
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_ascii_uppercase().to_string() + chars.as_str()
}

#[derive(Debug, Error)]
pub enum SolverError {
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error("wall {wall:?} was not found while generating join {join:?}")]
    MissingWallForJoin { join: ElementId, wall: ElementId },
    #[error("wall {wall:?} has no generated plan")]
    MissingWallPlan { wall: ElementId },
    #[error("join {join:?} point is outside wall {wall:?}")]
    JoinPointOutsideWall { join: ElementId, wall: ElementId },
    #[error("wall {wall:?} is too short for its configured plate stack")]
    WallTooShortForPlateStack { wall: ElementId },
    #[error("opening {opening:?} in wall {wall:?} leaves no header space below the top plates")]
    OpeningLeavesNoHeaderSpace { wall: ElementId, opening: ElementId },
    #[error(
        "opening {opening:?} in wall {wall:?} is too close to a wall end for starter king/jack framing"
    )]
    OpeningTooCloseToWallEnd { wall: ElementId, opening: ElementId },
}

#[cfg(test)]
mod tests {
    use framer_core::{
        BuildingModel, CodeProfile, ElementId, Opening, Wall, load_project, save_project,
    };

    use super::*;

    #[test]
    fn wall_with_door_generates_kings_jacks_and_header() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let plan = generate_wall_plan(&wall, &code).unwrap();

        assert_eq!(
            plan.members
                .iter()
                .filter(|member| member.kind == MemberKind::KingStud)
                .count(),
            2
        );
        assert_eq!(
            plan.members
                .iter()
                .filter(|member| member.kind == MemberKind::JackStud)
                .count(),
            2
        );
        assert!(plan.members.iter().any(|member| {
            member.kind == MemberKind::Header && member.cut_length == Length::from_inches(39.0)
        }));
    }

    #[test]
    fn king_and_jack_studs_are_adjacent_not_overlapping() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let plan = generate_wall_plan(&wall, &code).unwrap();
        let left_king = find_member(&plan, "door-king-left");
        let left_jack = find_member(&plan, "door-jack-left");
        let right_jack = find_member(&plan, "door-jack-right");
        let right_king = find_member(&plan, "door-king-right");

        assert_eq!(left_jack.x - left_king.x, code.stud_profile.thickness());
        assert_eq!(right_king.x - right_jack.x, code.stud_profile.thickness());
        assert_eq!(
            left_jack.x + code.stud_profile.thickness() / 2,
            wall.openings[0].left()
        );
        assert_eq!(
            right_jack.x - code.stud_profile.thickness() / 2,
            wall.openings[0].right()
        );
        assert!(!plan.members.iter().any(|member| {
            member.kind == MemberKind::CommonStud
                && member.x > left_king.x - code.stud_profile.thickness() / 2
                && member.x < right_king.x + code.stud_profile.thickness() / 2
        }));
    }

    #[test]
    fn bom_groups_cut_lengths() {
        let code = CodeProfile::irc_2021_prescriptive();
        let wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);

        let plan = generate_wall_plan(&wall, &code).unwrap();
        let bom = plan.bom();

        assert!(bom.iter().any(|item| {
            item.kind == MemberKind::TopPlate
                && item.cut_length == Length::from_feet(8.0)
                && item.quantity == 2
        }));
    }

    #[test]
    fn end_studs_align_faces_with_wall_edges() {
        let code = CodeProfile::irc_2021_prescriptive();
        let wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);

        let plan = generate_wall_plan(&wall, &code).unwrap();
        let mut common_studs = plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::CommonStud)
            .collect::<Vec<_>>();
        common_studs.sort_by_key(|member| member.x);

        let half_stud = code.stud_profile.thickness() / 2;
        assert_eq!(common_studs.first().unwrap().x, half_stud);
        assert_eq!(common_studs.last().unwrap().x, wall.length - half_stud);
        assert!(
            !common_studs
                .iter()
                .any(|member| member.x == Length::ZERO || member.x == wall.length)
        );
    }

    #[test]
    fn eight_foot_wall_uses_actual_plate_thickness_for_stud_length() {
        let code = CodeProfile::irc_2021_prescriptive();
        let wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);

        let plan = generate_wall_plan(&wall, &code).unwrap();

        assert!(plan.members.iter().any(|member| {
            member.kind == MemberKind::CommonStud && member.cut_length == Length::from_inches(91.5)
        }));
    }

    #[test]
    fn project_round_trip_regenerates_same_wall_plan() {
        let model = BuildingModel::demo_wall();
        let original = generate_wall_plan(&model.walls[0], &model.code).unwrap();

        let serialized = save_project(&model).unwrap();
        let loaded = load_project(&serialized).unwrap();
        let regenerated = generate_wall_plan(&loaded.walls[0], &loaded.code).unwrap();

        assert_eq!(loaded, model);
        assert_eq!(regenerated, original);
    }

    #[test]
    fn project_plan_frames_connected_shell_and_groups_bom() {
        let model = BuildingModel::demo_shell();
        let plan = generate_project_plan(&model).unwrap();

        assert_eq!(plan.wall_plans.len(), 4);
        assert!(plan.wall_plan(&ElementId::new("wall-front")).is_some());
        assert!(
            plan.wall_plans
                .iter()
                .flat_map(|wall_plan| wall_plan.members.iter())
                .any(|member| member.kind == MemberKind::CornerPost
                    && member.source.0 == "join-front-right")
        );
        assert!(plan.bom().iter().any(|item| {
            item.kind == MemberKind::CornerPost && item.quantity >= model.wall_joins.len() as u32
        }));
    }

    #[test]
    fn corner_posts_align_faces_with_joined_wall_edges() {
        let model = BuildingModel::demo_shell();
        let plan = generate_project_plan(&model).unwrap();
        let half_stud = model.code.stud_profile.thickness() / 2;

        for join in &model.wall_joins {
            for wall_id in [&join.first_wall, &join.second_wall] {
                let wall = model
                    .walls
                    .iter()
                    .find(|candidate| candidate.id == *wall_id)
                    .unwrap();
                let join_x = wall.local_x_for_point(join.point).unwrap();
                let expected_x =
                    face_aligned_center(join_x, wall.length, model.code.stud_profile.thickness());
                let wall_plan = plan.wall_plan(&wall.id).unwrap();
                let member_id = format!("{}-{}-corner-post", join.id.0, wall.id.0);
                let member = find_member(wall_plan, &member_id);

                assert_eq!(member.x, expected_x, "{member_id}");
                assert!(member.x >= half_stud, "{member_id}");
                assert!(member.x <= wall.length - half_stud, "{member_id}");
            }
        }
    }

    #[test]
    fn project_exports_include_whole_shell_plan_and_elevations() {
        let model = BuildingModel::demo_shell();
        let plan = generate_project_plan(&model).unwrap();
        let svg = export_project_svg(&model, &plan);
        let csv = export_bom_csv(&plan.bom());

        assert!(svg.contains("data-wall=\"wall-front\""));
        assert!(svg.contains("data-join=\"join-front-right\""));
        assert!(svg.contains("data-wall-elevation=\"wall-right\""));
        assert!(csv.contains("corner post"));
    }

    #[test]
    fn window_generates_sill_and_cripples() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(4.0),
            Length::from_inches(48.0),
            Length::from_inches(36.0),
            Length::from_inches(36.0),
        ));

        let plan = generate_wall_plan(&wall, &code).unwrap();

        assert!(
            plan.members
                .iter()
                .any(|member| member.kind == MemberKind::RoughSill)
        );
        assert!(
            plan.members
                .iter()
                .any(|member| member.kind == MemberKind::CrippleStud)
        );
    }

    #[test]
    fn generated_members_include_source_and_rule_provenance() {
        let model = BuildingModel::demo_wall();
        let plan = generate_wall_plan(&model.walls[0], &model.code).unwrap();

        let header = plan
            .members
            .iter()
            .find(|member| member.id == "opening-door-1-header")
            .unwrap();

        assert_eq!(header.source.0, "opening-door-1");
        assert_eq!(header.provenance.rule_id, "opening.header.default-profile");
        assert!(header.provenance.summary.contains("no span/load lookup"));
        assert_eq!(header.cross_section_depth, model.code.default_header_depth);
    }

    #[test]
    fn garage_door_reports_unsupported_starter_assumption() {
        let model = BuildingModel::demo_wall();
        let plan = generate_wall_plan(&model.walls[0], &model.code).unwrap();

        assert!(plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Unsupported
                && diagnostic.code == "opening.garage-door.starter-header"
                && diagnostic
                    .source
                    .as_ref()
                    .is_some_and(|source| source.0 == "opening-garage-1")
        }));
    }

    #[test]
    fn exports_are_deterministic_and_useful() {
        let model = BuildingModel::demo_wall();
        let wall = &model.walls[0];
        let plan = generate_wall_plan(wall, &model.code).unwrap();

        let first_svg = export_wall_elevation_svg(wall, &plan);
        let second_svg = export_wall_elevation_svg(wall, &plan);
        let csv = export_bom_csv(&plan.bom());

        assert_eq!(first_svg, second_svg);
        assert!(first_svg.contains("<svg"));
        assert!(first_svg.contains("opening-garage-1-header"));
        assert!(first_svg.contains(r#"data-opening="opening-garage-1""#));
        assert!(
            first_svg
                .lines()
                .any(|line| line.contains(r#"id="opening-door-1-header""#)
                    && line.contains(r#"height="9""#))
        );
        assert!(csv.starts_with("quantity,profile,kind"));
        assert!(csv.contains("total_length_inches"));
    }

    fn find_member<'a>(plan: &'a WallFramePlan, id: &str) -> &'a FrameMember {
        plan.members
            .iter()
            .find(|member| member.id == id)
            .unwrap_or_else(|| panic!("expected member {id}"))
    }
}
