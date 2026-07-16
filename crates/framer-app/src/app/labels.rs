use framer_analysis::{
    BooleanIntentMode, IntentDomain, IntentOutcome, IntentUnknownKind, ObjectiveDirection,
    SelectionAttribute,
};
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

pub(super) fn intent_domain_label(domain: IntentDomain) -> &'static str {
    match domain {
        IntentDomain::SpatialProgram => "Spatial program",
        IntentDomain::Construction => "Construction",
        IntentDomain::StructuralPerformance => "Structural / performance",
        IntentDomain::EnvelopeBuildingScience => "Envelope / building science",
        IntentDomain::Mep => "MEP",
        IntentDomain::Compliance => "Compliance",
        IntentDomain::Resource => "Resource",
        IntentDomain::FabricationInstallation => "Fabrication / installation",
        IntentDomain::OperationalMaintenance => "Operational / maintenance",
        IntentDomain::Aesthetic => "Aesthetic",
    }
}

pub(super) fn intent_outcome_label(outcome: &IntentOutcome) -> &'static str {
    match outcome {
        IntentOutcome::Violated => "Violated",
        IntentOutcome::Unknown(_) => "Unknown",
        IntentOutcome::Waived { .. } => "Waived",
        IntentOutcome::Satisfied => "Satisfied",
        IntentOutcome::NotApplicable => "Not applicable",
    }
}

pub(super) fn intent_unknown_kind_label(kind: IntentUnknownKind) -> &'static str {
    match kind {
        IntentUnknownKind::MissingInput => "Missing input",
        IntentUnknownKind::UnresolvedSubject => "Unresolved subject",
        IntentUnknownKind::UnresolvedReference => "Unresolved reference",
        IntentUnknownKind::WrongSubjectKind => "Wrong subject kind",
        IntentUnknownKind::UnsupportedCondition => "Unsupported condition",
        IntentUnknownKind::EvaluationUnavailable => "Evaluation unavailable",
    }
}

pub(super) fn boolean_intent_mode_label(mode: BooleanIntentMode) -> String {
    match mode {
        BooleanIntentMode::Requirement => "Requirement".to_owned(),
        BooleanIntentMode::Preference { priority } => {
            format!("Preference {}", priority.0)
        }
    }
}

pub(super) fn selection_attribute_label(attribute: SelectionAttribute) -> &'static str {
    match attribute {
        SelectionAttribute::ConstructionSystem => "Construction system",
    }
}

pub(super) fn objective_direction_label(direction: ObjectiveDirection) -> &'static str {
    match direction {
        ObjectiveDirection::Minimize => "Minimize",
        ObjectiveDirection::Maximize => "Maximize",
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
