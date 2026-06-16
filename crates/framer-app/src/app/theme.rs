use eframe::egui::{Color32, Stroke};

pub(super) fn shell_bg() -> Color32 {
    Color32::from_rgb(18, 22, 24)
}

pub(super) fn chrome_top() -> Color32 {
    Color32::from_rgb(24, 29, 32)
}

pub(super) fn chrome_mid() -> Color32 {
    Color32::from_rgb(29, 35, 38)
}

pub(super) fn panel_bg() -> Color32 {
    Color32::from_rgb(22, 27, 30)
}

pub(super) fn panel_header() -> Color32 {
    Color32::from_rgb(29, 35, 39)
}

pub(super) fn workspace_bg() -> Color32 {
    Color32::from_rgb(15, 19, 21)
}

pub(super) fn field_bg() -> Color32 {
    Color32::from_rgb(18, 22, 24)
}

pub(super) fn control_bg() -> Color32 {
    Color32::from_rgb(35, 42, 46)
}

pub(super) fn control_bg_hover() -> Color32 {
    Color32::from_rgb(45, 54, 59)
}

pub(super) fn active_blue() -> Color32 {
    Color32::from_rgb(37, 119, 198)
}

pub(super) fn active_blue_soft() -> Color32 {
    Color32::from_rgb(30, 79, 123)
}

pub(super) fn text_primary() -> Color32 {
    Color32::from_rgb(235, 239, 240)
}

pub(super) fn text_secondary() -> Color32 {
    Color32::from_rgb(184, 193, 196)
}

pub(super) fn text_muted() -> Color32 {
    Color32::from_rgb(128, 140, 145)
}

pub(super) fn divider() -> Color32 {
    Color32::from_rgb(49, 58, 63)
}

pub(super) fn divider_soft() -> Color32 {
    Color32::from_rgb(37, 45, 49)
}

pub(super) fn success() -> Color32 {
    Color32::from_rgb(89, 190, 125)
}

pub(super) fn warning() -> Color32 {
    Color32::from_rgb(224, 174, 74)
}

pub(super) fn danger() -> Color32 {
    Color32::from_rgb(220, 92, 82)
}

pub(super) fn sheet() -> Color32 {
    Color32::from_rgb(248, 248, 245)
}

pub(super) fn sheet_grid() -> Color32 {
    Color32::from_rgb(226, 229, 226)
}

pub(super) fn sheet_grid_major() -> Color32 {
    Color32::from_rgb(205, 211, 210)
}

pub(super) fn sheet_ruler() -> Color32 {
    Color32::from_rgb(239, 241, 239)
}

pub(super) fn framing_line() -> Color32 {
    Color32::from_rgb(123, 93, 55)
}

pub(super) fn framing_line_dark() -> Color32 {
    Color32::from_rgb(79, 70, 56)
}

pub(super) fn divider_stroke() -> Stroke {
    Stroke::new(1.0, divider())
}

pub(super) fn soft_stroke() -> Stroke {
    Stroke::new(1.0, divider_soft())
}
