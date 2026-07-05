use framer_core::{DimensionAxis, DimensionKind, OpeningKind, WallJoinKind};
use framer_solver::DiagnosticSeverity;

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
