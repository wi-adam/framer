//! Semantic design tokens.
//!
//! A [`Theme`] is a cheap `Copy` bundle of every color the UI is allowed to
//! reference, grouped by role so call sites read intent (`t.accent`) instead of
//! raw hex. Palettes (`studio_light`, `studio_dark`) are just data that fills
//! this struct; see [`super::palette`].
//!
//! This is a design vocabulary (palette + metric scales) consumed across reskin
//! phases, so not every token is referenced at every commit.
#![allow(dead_code)]

use eframe::egui::{Color32, Stroke};

/// The full semantic palette for one theme variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Theme {
    // Surfaces, lightest-behind to most-foreground.
    pub app_bg: Color32,
    pub title_bar: Color32,
    pub toolbar: Color32,
    pub chrome_alt: Color32,
    pub panel: Color32,
    pub panel_header: Color32,
    pub canvas: Color32,
    pub field: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub overlay: Color32,

    // Text.
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub text_on_accent: Color32,

    // Accent + semantic.
    pub accent: Color32,
    pub accent_soft: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,

    // Lines.
    pub divider: Color32,
    pub divider_soft: Color32,
    pub border: Color32,

    // Drawing surface (the "paper" and what is drawn on it).
    pub paper: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,
    pub ruler: Color32,
    pub framing: Color32,
    pub framing_dark: Color32,
    pub selection: Color32,
    pub dimension: Color32,

    /// True for dark variants; lets a few widgets pick a base.
    pub dark: bool,
}

impl Theme {
    pub(crate) fn divider_stroke(&self) -> Stroke {
        Stroke::new(1.0, self.divider)
    }

    pub(crate) fn soft_stroke(&self) -> Stroke {
        Stroke::new(1.0, self.divider_soft)
    }

    pub(crate) fn border_stroke(&self) -> Stroke {
        Stroke::new(1.0, self.border)
    }

    pub(crate) fn accent_stroke(&self) -> Stroke {
        Stroke::new(1.0, self.accent)
    }
}

/// Spacing scale (logical points). Use these instead of literal gaps.
pub(crate) mod space {
    pub(crate) const XS: f32 = 2.0;
    pub(crate) const SM: f32 = 4.0;
    pub(crate) const MD: f32 = 8.0;
    pub(crate) const LG: f32 = 12.0;
    pub(crate) const XL: f32 = 16.0;
}

/// Corner radii (egui takes `u8`).
pub(crate) mod radius {
    pub(crate) const SM: u8 = 3;
    pub(crate) const MD: u8 = 5;
}

/// Type scale (font sizes in points).
pub(crate) mod text_size {
    pub(crate) const TITLE: f32 = 18.5;
    pub(crate) const HEADING: f32 = 15.0;
    pub(crate) const BODY: f32 = 12.5;
    pub(crate) const LABEL: f32 = 10.5;
    pub(crate) const MICRO: f32 = 9.5;
    pub(crate) const MONO: f32 = 12.0;
}

/// Control sizing.
pub(crate) mod control {
    use eframe::egui::Vec2;

    /// Icon-over-label toolbar button.
    pub(crate) const TOOL_BTN: Vec2 = Vec2::new(46.0, 44.0);
    /// Bare square icon button.
    pub(crate) const ICON_BTN: f32 = 26.0;
    /// Property/list row height.
    pub(crate) const ROW_H: f32 = 24.0;
    /// Toolbar icon glyph size.
    pub(crate) const TOOL_ICON: f32 = 18.0;
    /// Inline / status icon glyph size.
    pub(crate) const INLINE_ICON: f32 = 14.0;
}

#[cfg(test)]
mod tests {
    use eframe::egui::Color32;

    use super::super::palette::{studio_dark, studio_light};
    use super::*;

    const MIN_UI_TEXT_CONTRAST: f32 = 3.0;

    #[test]
    fn text_and_surface_token_pairings_keep_minimum_contrast() {
        for (theme_name, theme) in [("light", studio_light()), ("dark", studio_dark())] {
            for (pair_name, foreground, background) in token_pairings(theme) {
                let ratio = contrast_ratio(foreground, background);
                assert!(
                    ratio >= MIN_UI_TEXT_CONTRAST,
                    "{theme_name} {pair_name} contrast {ratio:.2} should be at least {MIN_UI_TEXT_CONTRAST:.1}"
                );
            }
        }
    }

    fn token_pairings(theme: Theme) -> Vec<(&'static str, Color32, Color32)> {
        vec![
            ("text on panel", theme.text, theme.panel),
            ("secondary text on panel", theme.text_secondary, theme.panel),
            ("muted text on panel", theme.text_muted, theme.panel),
            ("text on toolbar", theme.text, theme.toolbar),
            (
                "secondary text on toolbar",
                theme.text_secondary,
                theme.toolbar,
            ),
            ("text on control", theme.text, theme.control),
            (
                "secondary text on control",
                theme.text_secondary,
                theme.control,
            ),
            ("text on field", theme.text, theme.field),
            ("secondary text on field", theme.text_secondary, theme.field),
            ("text on overlay", theme.text, theme.overlay),
            (
                "secondary text on overlay",
                theme.text_secondary,
                theme.overlay,
            ),
            ("accent text on accent", theme.text_on_accent, theme.accent),
            ("danger text on panel", theme.danger, theme.panel),
            ("warning text on panel", theme.warning, theme.panel),
        ]
    }

    fn contrast_ratio(a: Color32, b: Color32) -> f32 {
        let lighter = relative_luminance(a).max(relative_luminance(b));
        let darker = relative_luminance(a).min(relative_luminance(b));
        (lighter + 0.05) / (darker + 0.05)
    }

    fn relative_luminance(color: Color32) -> f32 {
        0.2126 * linear_channel(color.r())
            + 0.7152 * linear_channel(color.g())
            + 0.0722 * linear_channel(color.b())
    }

    fn linear_channel(value: u8) -> f32 {
        let channel = f32::from(value) / 255.0;
        if channel <= 0.03928 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }
}
