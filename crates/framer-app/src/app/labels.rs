use framer_core::{DimensionAxis, DimensionKind, OpeningKind, WallJoinKind};
use framer_geometry::{AssemblyKind, BodyKind, BodyRef};
use framer_solver::{DiagnosticSeverity, MemberKind};

pub(super) fn kind_label(kind: OpeningKind) -> &'static str {
    match kind {
        OpeningKind::Door => "Door",
        OpeningKind::Window => "Window",
        OpeningKind::GarageDoor => "Garage door",
        OpeningKind::Skylight => "Skylight",
        OpeningKind::Stair => "Stair",
    }
}

pub(super) fn join_kind_label(kind: WallJoinKind) -> &'static str {
    match kind {
        WallJoinKind::Corner => "Corner",
        WallJoinKind::EndToEnd => "End-to-end",
        WallJoinKind::Tee => "Tee",
        WallJoinKind::Cross => "Cross",
    }
}

pub(super) fn dimension_kind_label(kind: DimensionKind) -> &'static str {
    match kind {
        DimensionKind::Driving => "Driving",
        DimensionKind::Reference => "Reference",
    }
}

pub(super) fn dimension_axis_label(axis: DimensionAxis) -> &'static str {
    match axis {
        DimensionAxis::Horizontal => "Horizontal",
        DimensionAxis::Vertical => "Vertical",
    }
}

pub(super) fn diagnostic_code_prefix(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Info => "Info",
        DiagnosticSeverity::Warning => "Warning",
        DiagnosticSeverity::Unsupported => "Unsupported",
        DiagnosticSeverity::Violation => "Violation",
        DiagnosticSeverity::NeedsReview => "Needs review",
    }
}

pub(super) fn geometry_body_label(body: &BodyRef) -> String {
    let owner = &body.owner().0;
    match body.kind() {
        BodyKind::Assembly(kind) => {
            format!("{} {owner}", assembly_kind_label(kind))
        }
        BodyKind::FrameMember(kind) => format!(
            "{} {owner}/{}",
            member_kind_label(kind),
            body.member_id().unwrap_or("missing-member-id")
        ),
    }
}

fn assembly_kind_label(kind: AssemblyKind) -> &'static str {
    match kind {
        AssemblyKind::Wall => "Wall assembly",
        AssemblyKind::FloorDeck => "Floor deck assembly",
        AssemblyKind::Ceiling => "Ceiling assembly",
        AssemblyKind::RoofPlane => "Roof plane assembly",
    }
}

fn member_kind_label(kind: MemberKind) -> &'static str {
    match kind {
        MemberKind::BottomPlate => "Bottom plate",
        MemberKind::TopPlate => "Top plate",
        MemberKind::CornerPost => "Corner post",
        MemberKind::PartitionStud => "Partition stud",
        MemberKind::BackingStud => "Backing stud",
        MemberKind::CommonStud => "Common stud",
        MemberKind::KingStud => "King stud",
        MemberKind::JackStud => "Jack stud",
        MemberKind::Header => "Header",
        MemberKind::RoughSill => "Rough sill",
        MemberKind::CrippleStud => "Cripple stud",
        MemberKind::GableStud => "Gable stud",
        MemberKind::RakePlate => "Rake plate",
        MemberKind::FloorJoist => "Floor joist",
        MemberKind::CeilingJoist => "Ceiling joist",
        MemberKind::RimJoist => "Rim joist",
        MemberKind::Blocking => "Blocking",
        MemberKind::Rafter => "Rafter",
        MemberKind::RidgeBoard => "Ridge board",
        MemberKind::HipRafter => "Hip rafter",
        MemberKind::ValleyRafter => "Valley rafter",
        MemberKind::JackRafter => "Jack rafter",
    }
}
