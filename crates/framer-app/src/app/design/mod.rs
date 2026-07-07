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

use eframe::{
    Storage,
    egui::{self, Context, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Vec2},
};

pub(crate) use icons::{Icon, icon_font, icon_text};
pub(crate) use palette::{studio_dark, studio_light};
pub(crate) use tokens::{Theme, control, radius, space, text_size};

thread_local! {
    static ACTIVE: Cell<Theme> = const { Cell::new(palette::studio_light()) };
}

const THEME_STORAGE_KEY: &str = "framer.theme";
const LIGHT_THEME_STORAGE_VALUE: &str = "light";
const DARK_THEME_STORAGE_VALUE: &str = "dark";

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

/// Restore the persisted theme, defaulting to light when storage is absent or stale.
pub(crate) fn theme_from_storage(storage: Option<&dyn Storage>) -> Theme {
    storage
        .and_then(|storage| storage.get_string(THEME_STORAGE_KEY))
        .as_deref()
        .and_then(theme_from_storage_value)
        .unwrap_or_else(studio_light)
}

/// Persist the active theme for the next eframe launch.
pub(crate) fn save_theme(storage: &mut dyn Storage) {
    storage.set_string(THEME_STORAGE_KEY, theme_storage_value(active()).to_owned());
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
    let egui_theme = if theme.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(egui_theme);

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
    v.window_fill = theme.overlay;
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

    ctx.set_style_of(egui_theme, style.clone());
    ctx.set_global_style(style);
    ctx.request_repaint();
}

fn theme_from_storage_value(value: &str) -> Option<Theme> {
    match value {
        DARK_THEME_STORAGE_VALUE => Some(studio_dark()),
        LIGHT_THEME_STORAGE_VALUE => Some(studio_light()),
        _ => None,
    }
}

fn theme_storage_value(theme: Theme) -> &'static str {
    if theme.dark {
        DARK_THEME_STORAGE_VALUE
    } else {
        LIGHT_THEME_STORAGE_VALUE
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct MemoryStorage {
        values: HashMap<String, String>,
    }

    impl Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.values.insert(key.to_owned(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.values.remove(key);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn restores_theme_from_storage() {
        let mut storage = MemoryStorage::default();
        storage.set_string(THEME_STORAGE_KEY, DARK_THEME_STORAGE_VALUE.to_owned());

        assert_eq!(theme_from_storage(Some(&storage)), studio_dark());

        storage.set_string(THEME_STORAGE_KEY, LIGHT_THEME_STORAGE_VALUE.to_owned());
        assert_eq!(theme_from_storage(Some(&storage)), studio_light());
    }

    #[test]
    fn invalid_or_missing_theme_storage_defaults_to_light() {
        let mut storage = MemoryStorage::default();
        storage.set_string(THEME_STORAGE_KEY, "system".to_owned());

        assert_eq!(theme_from_storage(Some(&storage)), studio_light());
        assert_eq!(theme_from_storage(None), studio_light());
    }

    #[test]
    fn saves_active_theme_to_storage() {
        let ctx = Context::default();
        install(&ctx, studio_dark());
        let mut storage = MemoryStorage::default();

        save_theme(&mut storage);

        assert_eq!(
            storage.get_string(THEME_STORAGE_KEY).as_deref(),
            Some(DARK_THEME_STORAGE_VALUE)
        );
    }

    #[test]
    fn install_pins_egui_theme_and_per_theme_style_slot() {
        let ctx = Context::default();
        install(&ctx, studio_light());

        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert_eq!(
            ctx.style_of(egui::Theme::Light).visuals.window_fill,
            studio_light().overlay,
        );

        set_theme(&ctx, studio_dark());

        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.window_fill,
            studio_dark().overlay,
        );
    }
}
