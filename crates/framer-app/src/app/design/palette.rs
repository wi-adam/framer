//! Concrete palettes. Each function fills a [`Theme`] with one variant's colors.
//!
//! The drawing surface ("paper" + grid + framing) stays light in both variants
//! so the framing/drawing code reads identically against a bright sheet, the way
//! the mockup and most CAD apps present drawings.

use eframe::egui::Color32;

use super::tokens::Theme;

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Light "studio" palette — the look the mockup targets.
pub(crate) const fn studio_light() -> Theme {
    Theme {
        app_bg: rgb(235, 238, 241),
        title_bar: rgb(26, 30, 34),
        toolbar: rgb(250, 250, 251),
        chrome_alt: rgb(242, 244, 246),
        panel: rgb(255, 255, 255),
        panel_header: rgb(245, 247, 249),
        canvas: rgb(231, 234, 238),
        field: rgb(255, 255, 255),
        control: rgb(241, 243, 245),
        control_hover: rgb(232, 235, 239),
        overlay: rgb(255, 255, 255),

        text: rgb(26, 31, 36),
        text_secondary: rgb(74, 84, 94),
        text_muted: rgb(138, 147, 156),
        text_on_accent: rgb(255, 255, 255),

        accent: rgb(43, 124, 222),
        accent_soft: rgb(219, 233, 252),
        success: rgb(38, 158, 94),
        warning: rgb(211, 152, 41),
        danger: rgb(212, 74, 68),

        divider: rgb(224, 228, 232),
        divider_soft: rgb(236, 239, 242),
        border: rgb(206, 212, 219),

        paper: rgb(250, 250, 248),
        grid_minor: rgb(226, 229, 226),
        grid_major: rgb(203, 209, 208),
        ruler: rgb(240, 242, 240),
        framing: rgb(123, 93, 55),
        framing_dark: rgb(79, 70, 56),
        selection: rgb(43, 124, 222),
        dimension: rgb(96, 106, 118),

        dark: false,
    }
}

/// Refreshed dark palette derived from the app's prior chrome colors.
pub(crate) const fn studio_dark() -> Theme {
    Theme {
        app_bg: rgb(18, 22, 24),
        title_bar: rgb(24, 29, 32),
        toolbar: rgb(24, 29, 32),
        chrome_alt: rgb(29, 35, 38),
        panel: rgb(22, 27, 30),
        panel_header: rgb(29, 35, 39),
        canvas: rgb(15, 19, 21),
        field: rgb(18, 22, 24),
        control: rgb(35, 42, 46),
        control_hover: rgb(45, 54, 59),
        overlay: rgb(31, 38, 42),

        text: rgb(235, 239, 240),
        text_secondary: rgb(184, 193, 196),
        text_muted: rgb(128, 140, 145),
        text_on_accent: rgb(240, 245, 248),

        accent: rgb(46, 121, 198),
        accent_soft: rgb(30, 79, 123),
        success: rgb(89, 190, 125),
        warning: rgb(224, 174, 74),
        danger: rgb(220, 92, 82),

        divider: rgb(49, 58, 63),
        divider_soft: rgb(37, 45, 49),
        border: rgb(60, 70, 76),

        // Drawing surface stays light in both variants.
        paper: rgb(248, 248, 245),
        grid_minor: rgb(226, 229, 226),
        grid_major: rgb(205, 211, 210),
        ruler: rgb(239, 241, 239),
        framing: rgb(123, 93, 55),
        framing_dark: rgb(79, 70, 56),
        selection: rgb(46, 121, 198),
        dimension: rgb(96, 106, 118),

        dark: true,
    }
}
