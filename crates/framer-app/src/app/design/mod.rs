//! Framer's design system: semantic tokens, palettes, icons, and the egui style
//! derived from them.
//!
//! The active [`Theme`] lives in a thread-local cell so the many existing
//! `theme::*` helpers and new widgets can read it with no argument threading
//! (`design::active()`). egui is single-threaded for UI, so a per-thread cell is
//! sufficient and lock-free.

mod icons;
mod palette;
mod tokens;
pub(crate) mod widgets;

use std::cell::Cell;

use eframe::egui::{self, Context, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Vec2};

pub(crate) use icons::{Icon, icon_font, icon_text};
pub(crate) use palette::{studio_dark, studio_light};
pub(crate) use tokens::{Theme, control, radius, space, text_size};

thread_local! {
    static ACTIVE: Cell<Theme> = const { Cell::new(palette::studio_light()) };
}

/// The currently active theme.
pub(crate) fn active() -> Theme {
    ACTIVE.with(|cell| cell.get())
}

/// Convenience accessor; `ui` is accepted for call-site ergonomics and future
/// per-surface theming. Used by widgets introduced in later reskin phases.
#[allow(dead_code)]
pub(crate) fn theme(_ui: &egui::Ui) -> Theme {
    active()
}

fn set_active(theme: Theme) {
    ACTIVE.with(|cell| cell.set(theme));
}

/// Install fonts and apply `theme`. Call once at startup.
pub(crate) fn install(ctx: &Context, theme: Theme) {
    icons::install_fonts(ctx);
    set_theme(ctx, theme);
}

/// Switch the active theme and rebuild the egui style from it.
pub(crate) fn set_theme(ctx: &Context, theme: Theme) {
    set_active(theme);
    configure_style(ctx, theme);
}

/// Toggle between light and dark, returning the variant now active.
pub(crate) fn toggle_theme(ctx: &Context) -> Theme {
    let next = if active().dark {
        studio_light()
    } else {
        studio_dark()
    };
    set_theme(ctx, next);
    next
}

/// Build egui's [`egui::Style`] from theme tokens. Starts from egui's light/dark
/// base so unspecified surfaces (popups, scrollbars, shadows) are variant-correct,
/// then overrides with our tokens.
fn configure_style(ctx: &Context, theme: Theme) {
    let mut style = (*ctx.global_style()).clone();

    style.visuals = if theme.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(text_size::HEADING, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Body,
        FontId::new(text_size::BODY, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(text_size::BODY, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(text_size::LABEL, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(text_size::MONO, FontFamily::Monospace),
    );

    style.spacing.item_spacing = Vec2::new(space::MD, space::SM + 1.0);
    style.spacing.button_padding = Vec2::new(space::MD, space::SM);
    style.spacing.interact_size = Vec2::new(40.0, 22.0);
    style.spacing.window_margin = egui::Margin::symmetric(space::MD as i8, space::SM as i8 + 2);
    style.spacing.menu_margin = egui::Margin::symmetric(space::MD as i8, space::SM as i8);

    let v = &mut style.visuals;
    v.panel_fill = theme.app_bg;
    v.window_fill = theme.panel;
    v.window_stroke = theme.divider_stroke();
    v.extreme_bg_color = theme.canvas;
    v.faint_bg_color = theme.chrome_alt;
    v.text_edit_bg_color = Some(theme.field);
    v.selection.bg_fill = theme.accent;
    v.selection.stroke = Stroke::new(1.0, theme.text_on_accent);
    v.hyperlink_color = theme.accent;
    v.warn_fg_color = theme.warning;
    v.error_fg_color = theme.danger;
    v.button_frame = true;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = true;
    v.popup_shadow.color = if theme.dark {
        egui::Color32::from_black_alpha(96)
    } else {
        egui::Color32::from_black_alpha(40)
    };

    let r = CornerRadius::same(radius::SM);
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = r;
    }

    v.widgets.noninteractive.bg_fill = theme.chrome_alt;
    v.widgets.noninteractive.weak_bg_fill = theme.panel_header;
    v.widgets.noninteractive.bg_stroke = theme.soft_stroke();
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, theme.text_secondary);

    v.widgets.inactive.weak_bg_fill = theme.control;
    v.widgets.inactive.bg_fill = theme.control;
    v.widgets.inactive.bg_stroke = theme.divider_stroke();
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, theme.text_secondary);

    v.widgets.hovered.weak_bg_fill = theme.control_hover;
    v.widgets.hovered.bg_fill = theme.control_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, theme.accent_soft);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, theme.text);

    v.widgets.active.weak_bg_fill = theme.accent_soft;
    v.widgets.active.bg_fill = theme.accent;
    v.widgets.active.bg_stroke = theme.accent_stroke();
    v.widgets.active.fg_stroke = Stroke::new(1.0, theme.text_on_accent);

    v.widgets.open = v.widgets.hovered;

    ctx.set_global_style(style);
}
