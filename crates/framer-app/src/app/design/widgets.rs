//! Reusable component builders. Each reads [`super::active`] so it restyles for
//! free when the theme changes. These target light "panel" surfaces; the dark
//! app header builds its controls inline with explicit light-on-dark colors.
//!
//! A component library: builders are added ahead of their first call site.
#![allow(dead_code)]

use eframe::egui::{
    Align2, Button, CollapsingHeader, Color32, FontId, Pos2, Response, RichText, Sense, Stroke, Ui,
    Vec2, WidgetInfo, WidgetType,
};

use super::{Icon, active, control, icon_font, icon_text, radius, space, text_size};

/// A bare square icon button (no label) for light surfaces.
pub(crate) fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    let t = active();
    let response = ui.add_sized(
        Vec2::splat(control::ICON_BTN),
        Button::new(icon_text(icon, control::INLINE_ICON).color(t.text_secondary))
            .fill(t.control)
            .stroke(t.soft_stroke())
            .corner_radius(radius::SM),
    );
    with_tooltip(response, tooltip)
}

/// A ghost icon button: transparent until hovered. `fg` lets the caller place it
/// on a dark surface (the app header) by passing a light color.
pub(crate) fn ghost_icon_button(ui: &mut Ui, icon: Icon, fg: Color32, tooltip: &str) -> Response {
    let sense = Sense::click();
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(control::ICON_BTN), sense);
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, radius::SM, Color32::from_white_alpha(18));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon.glyph().to_string(),
        icon_font(control::INLINE_ICON),
        fg,
    );
    with_tooltip(response, tooltip)
}

/// The icon-over-label toolbar button with the mockup's blue active state.
pub(crate) fn tool_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    active_state: bool,
    enabled: bool,
) -> Response {
    let t = active();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(control::TOOL_BTN, sense);
    let hovered = enabled && response.hovered();

    let (fill, fg) = if !enabled {
        (Color32::TRANSPARENT, t.text_muted)
    } else if active_state {
        (t.accent, t.text_on_accent)
    } else if hovered {
        (t.control_hover, t.text)
    } else {
        (Color32::TRANSPARENT, t.text_secondary)
    };

    let painter = ui.painter();
    if fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, radius::SM, fill);
    }
    painter.text(
        Pos2::new(rect.center().x, rect.top() + 14.0),
        Align2::CENTER_CENTER,
        icon.glyph().to_string(),
        icon_font(control::TOOL_ICON),
        fg,
    );
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 9.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(text_size::MICRO),
        fg,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    response
}

/// A labelled group of toolbar controls (uppercase caption above a row).
pub(crate) fn tool_group(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let t = active();
    ui.vertical(|ui| {
        ui.add_space(1.0);
        ui.label(
            RichText::new(label)
                .size(text_size::MICRO)
                .strong()
                .color(t.text_muted),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            add(ui);
        });
    });
}

/// A vertical divider sized to a toolbar group's button row.
pub(crate) fn tool_divider(ui: &mut Ui) {
    let t = active();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, control::TOOL_BTN.y), Sense::hover());
    let x = rect.center().x;
    ui.painter().line_segment(
        [
            Pos2::new(x, rect.top() + 14.0),
            Pos2::new(x, rect.bottom() - 2.0),
        ],
        Stroke::new(1.0, t.divider),
    );
}

/// A sliding on/off switch with a trailing label (status-bar Grid/Ortho).
pub(crate) fn toggle_switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let t = active();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        let (rect, mut response) = ui.allocate_exact_size(Vec2::new(26.0, 15.0), Sense::click());
        if response.clicked() {
            *on = !*on;
            response.mark_changed();
        }
        let painter = ui.painter();
        let track = if *on {
            t.accent
        } else if t.dark {
            t.control_hover
        } else {
            t.border
        };
        painter.rect_filled(rect, (rect.height() / 2.0) as u8, track);
        let r = rect.height() / 2.0 - 2.0;
        let cx = if *on {
            rect.right() - r - 2.0
        } else {
            rect.left() + r + 2.0
        };
        painter.circle_filled(Pos2::new(cx, rect.center().y), r, Color32::WHITE);
        ui.label(RichText::new(label).size(text_size::LABEL).color(if *on {
            t.text
        } else {
            t.text_secondary
        }));
        response
    })
    .inner
}

/// A collapsible inspector section with a styled header.
pub(crate) fn section<R>(
    ui: &mut Ui,
    id: &str,
    title: &str,
    default_open: bool,
    body: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let t = active();
    CollapsingHeader::new(
        RichText::new(title)
            .strong()
            .size(text_size::BODY)
            .color(t.text),
    )
    .id_salt(id)
    .default_open(default_open)
    .show_unindented(ui, body)
    .body_returned
}

/// A workspace tab: text with an accent underline when selected.
pub(crate) fn tab(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let t = active();
    let color = if selected { t.accent } else { t.text_secondary };
    let response = ui.add(
        Button::new(RichText::new(label).strong().color(color))
            .frame(false)
            .corner_radius(radius::SM),
    );
    if selected {
        let rect = response.rect;
        ui.painter().line_segment(
            [
                Pos2::new(rect.left(), rect.bottom() + 3.0),
                Pos2::new(rect.right(), rect.bottom() + 3.0),
            ],
            Stroke::new(2.0, t.accent),
        );
    }
    response
}

fn with_tooltip(response: Response, tooltip: &str) -> Response {
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}
