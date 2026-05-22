use framer_core::{OpeningKind, WallJoinKind};
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

pub(super) fn diagnostic_code_prefix(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Info => "Info",
        DiagnosticSeverity::Warning => "Warning",
        DiagnosticSeverity::Unsupported => "Unsupported",
    }
}
