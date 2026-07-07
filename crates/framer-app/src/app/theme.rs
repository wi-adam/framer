//! Backwards-compatible color accessors.
//!
//! These thin wrappers forward to the active [`super::design::Theme`] so existing
//! call sites keep working while the design system drives the colors. New code
//! should prefer `design::active()` / `design::theme(ui)` and read tokens by name.

use eframe::egui::{Color32, Stroke};

use super::design;

pub(super) fn chrome_top() -> Color32 {
    design::active().toolbar
}

pub(super) fn chrome_mid() -> Color32 {
    design::active().chrome_alt
}

pub(super) fn panel_bg() -> Color32 {
    design::active().panel
}

pub(super) fn overlay() -> Color32 {
    design::active().overlay
}

pub(super) fn workspace_bg() -> Color32 {
    design::active().canvas
}

pub(super) fn active_blue() -> Color32 {
    design::active().accent
}

pub(super) fn active_blue_soft() -> Color32 {
    design::active().accent_soft
}

pub(super) fn text_primary() -> Color32 {
    design::active().text
}

pub(super) fn text_secondary() -> Color32 {
    design::active().text_secondary
}

pub(super) fn text_muted() -> Color32 {
    design::active().text_muted
}

pub(super) fn success() -> Color32 {
    design::active().success
}

pub(super) fn warning() -> Color32 {
    design::active().warning
}

pub(super) fn danger() -> Color32 {
    design::active().danger
}

pub(super) fn sheet() -> Color32 {
    design::active().paper
}

pub(super) fn sheet_grid() -> Color32 {
    design::active().grid_minor
}

pub(super) fn sheet_grid_major() -> Color32 {
    design::active().grid_major
}

pub(super) fn sheet_ruler() -> Color32 {
    design::active().ruler
}

pub(super) fn framing_line() -> Color32 {
    design::active().framing
}

pub(super) fn framing_line_dark() -> Color32 {
    design::active().framing_dark
}

pub(super) fn soft_stroke() -> Stroke {
    design::active().soft_stroke()
}
