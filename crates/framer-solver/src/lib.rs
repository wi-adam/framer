use std::collections::BTreeMap;
use std::fmt::Write;

use framer_core::{BoardProfile, CodeProfile, ElementId, Length, ModelError, Opening, Wall};
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
        let mut grouped: BTreeMap<(BoardProfile, MemberKind, Length), u32> = BTreeMap::new();
        for member in &self.members {
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
    let top_plate_count = if code.double_top_plate { 2 } else { 1 };
    let stud_top = wall.height - plate_thickness * top_plate_count as i64;
    let stud_length = stud_top - plate_thickness;

    if stud_length <= Length::ZERO {
        return Err(SolverError::WallTooShortForPlateStack {
            wall: wall.id.clone(),
        });
    }

    for opening in &wall.openings {
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
        MemberOrientation::Horizontal,
        Length::ZERO,
        Length::ZERO,
        wall.length,
        code.plate_profile.thickness(),
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
            MemberOrientation::Horizontal,
            Length::ZERO,
            wall.height - plate_thickness * (index as i64 + 1),
            wall.length,
            code.plate_profile.thickness(),
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
    for x in stud_positions(wall.length, wall.stud_spacing) {
        if !is_inside_opening(x, &wall.openings) {
            members.push(frame_member(
                format!("stud-{}", x.ticks()),
                &wall.id,
                MemberKind::CommonStud,
                code.stud_profile,
                MemberOrientation::Vertical,
                x,
                stud_base,
                stud_length,
                code.stud_profile.thickness(),
                RuleProvenance::new(
                    "wall.studs.on-center",
                    format!(
                        "Common studs are placed from wall ends at {} on center, skipping the clear width of authored openings.",
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

    for (side, x) in [("left", left), ("right", right)] {
        members.push(frame_member(
            format!("{}-king-{}", opening.id.0, side),
            &opening.id,
            MemberKind::KingStud,
            code.stud_profile,
            MemberOrientation::Vertical,
            x,
            stud_base,
            stud_top - stud_base,
            code.stud_profile.thickness(),
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
            MemberOrientation::Vertical,
            x,
            stud_base,
            header_bottom - stud_base,
            code.stud_profile.thickness(),
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
        MemberOrientation::Horizontal,
        left,
        header_bottom,
        opening.width,
        header_depth,
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
            MemberOrientation::Horizontal,
            left,
            opening.sill_height,
            opening.width,
            code.stud_profile.thickness(),
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
            MemberOrientation::Vertical,
            x,
            bottom,
            cut_length,
            code.stud_profile.thickness(),
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

fn frame_member(
    id: impl Into<String>,
    source: &ElementId,
    kind: MemberKind,
    profile: BoardProfile,
    orientation: MemberOrientation,
    x: Length,
    elevation: Length,
    cut_length: Length,
    cross_section_depth: Length,
    provenance: RuleProvenance,
) -> FrameMember {
    FrameMember {
        id: id.into(),
        source: source.clone(),
        kind,
        profile,
        orientation,
        x,
        elevation,
        cut_length,
        cross_section_depth,
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
            DiagnosticSeverity::Unsupported,
            "solver.scope.straight-wall-only",
            Some(wall.id.clone()),
            "Phase 1 frames this authored straight wall only; corners, intersections, hold-downs, and multi-wall load paths are not solved yet.",
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

fn stud_positions(length: Length, spacing: Length) -> Vec<Length> {
    let mut positions = Vec::new();
    let mut x = Length::ZERO;
    while x < length {
        positions.push(x);
        x += spacing;
    }

    if positions.last().copied() != Some(length) {
        positions.push(length);
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

fn is_inside_opening(x: Length, openings: &[Opening]) -> bool {
    openings
        .iter()
        .any(|opening| x > opening.left() && x < opening.right())
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

fn member_svg_color(kind: MemberKind) -> &'static str {
    match kind {
        MemberKind::BottomPlate | MemberKind::TopPlate => "#635543",
        MemberKind::CommonStud => "#ba915e",
        MemberKind::KingStud => "#97643d",
        MemberKind::JackStud => "#d3a85f",
        MemberKind::Header => "#738263",
        MemberKind::RoughSill => "#5c7990",
        MemberKind::CrippleStud => "#dabe8b",
    }
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
    #[error("wall {wall:?} is too short for its configured plate stack")]
    WallTooShortForPlateStack { wall: ElementId },
    #[error("opening {opening:?} in wall {wall:?} leaves no header space below the top plates")]
    OpeningLeavesNoHeaderSpace { wall: ElementId, opening: ElementId },
}

#[cfg(test)]
mod tests {
    use framer_core::{BuildingModel, CodeProfile, Opening, Wall, load_project, save_project};

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
            member.kind == MemberKind::Header && member.cut_length == Length::from_inches(36.0)
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
}
