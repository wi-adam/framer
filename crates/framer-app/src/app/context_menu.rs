//! Shared app-layer context-menu context, composition, and rendering.
//!
//! Commands remain registered in [`super::actions`] and dispatched by
//! [`super::FramerApp`]. This module only describes which actions a contextual
//! surface presents and renders that description consistently. Surface builders
//! are intentionally explicit while Framer has a small, closed set of menus; a
//! future contribution registry can replace their internals without changing the
//! context/model/renderer contract.

use eframe::egui::{self, Ui};

use super::actions::{self, ActionId};
use super::{ComponentKey, ViewportMode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ContextMenuSurface {
    Viewport(ViewportMode),
    /// Reserved for the Model Browser's independently composed row menu. It is
    /// deliberately not routed through the viewport builder.
    #[allow(dead_code)]
    ModelBrowser,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ContextMenuTarget {
    Component(ComponentKey),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ContextMenuContext {
    pub(super) surface: ContextMenuSurface,
    pub(super) target: ContextMenuTarget,
}

impl ContextMenuContext {
    pub(super) fn viewport(viewport: ViewportMode, target: ComponentKey) -> Self {
        Self {
            surface: ContextMenuSurface::Viewport(viewport),
            target: ContextMenuTarget::Component(target),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ContextMenuModel {
    sections: Vec<ContextMenuSection>,
}

impl ContextMenuModel {
    pub(super) fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContextMenuSection {
    items: Vec<ContextMenuItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ContextMenuItem {
    Action {
        id: ActionId,
        label: Option<&'static str>,
    },
    Submenu {
        label: &'static str,
        items: Vec<ContextMenuItem>,
    },
}

impl ContextMenuItem {
    fn action(id: ActionId) -> Self {
        Self::Action { id, label: None }
    }

    fn labeled_action(id: ActionId, label: &'static str) -> Self {
        Self::Action {
            id,
            label: Some(label),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ContextActionState {
    pub(super) enabled: bool,
    pub(super) disabled_reason: Option<&'static str>,
}

pub(super) fn build_context_menu(context: &ContextMenuContext) -> ContextMenuModel {
    match (&context.surface, &context.target) {
        (
            ContextMenuSurface::Viewport(ViewportMode::Axonometric),
            ContextMenuTarget::Component(_),
        ) => build_3d_selection_menu(),
        (
            ContextMenuSurface::Viewport(
                ViewportMode::Plan
                | ViewportMode::RoofPlan
                | ViewportMode::Elevation
                | ViewportMode::Render,
            )
            | ContextMenuSurface::ModelBrowser,
            ContextMenuTarget::Component(_),
        ) => ContextMenuModel::default(),
    }
}

fn build_3d_selection_menu() -> ContextMenuModel {
    ContextMenuModel {
        sections: vec![
            ContextMenuSection {
                items: vec![ContextMenuItem::Submenu {
                    label: "Isolate",
                    items: vec![
                        ContextMenuItem::labeled_action(ActionId::IsolateDim, "Dim Others"),
                        ContextMenuItem::labeled_action(ActionId::IsolateHide, "Hide Others"),
                    ],
                }],
            },
            ContextMenuSection {
                items: vec![
                    ContextMenuItem::action(ActionId::ExitIsolation),
                    ContextMenuItem::action(ActionId::HideSelection),
                    ContextMenuItem::action(ActionId::ShowAllComponents),
                ],
            },
        ],
    }
}

/// Render `model` and return the action chosen this frame, if any.
///
/// The caller remains responsible for executing the returned `ActionId`; this
/// keeps context menus on the same enablement and dispatch paths as command
/// search, the context toolbar, and the Model Browser.
pub(super) fn render_context_menu(
    ui: &mut Ui,
    model: &ContextMenuModel,
    mut action_state: impl FnMut(ActionId) -> ContextActionState,
) -> Option<ActionId> {
    ui.set_min_width(184.0);
    let mut chosen = None;
    for (index, section) in model.sections.iter().enumerate() {
        if index > 0 {
            ui.separator();
        }
        for item in &section.items {
            if chosen.is_none() {
                chosen = render_item(ui, item, &mut action_state);
            }
        }
    }
    chosen
}

fn render_item(
    ui: &mut Ui,
    item: &ContextMenuItem,
    action_state: &mut impl FnMut(ActionId) -> ContextActionState,
) -> Option<ActionId> {
    match item {
        ContextMenuItem::Action { id, label } => {
            let metadata = actions::metadata(*id);
            let state = action_state(*id);
            let response = ui.add_enabled(
                state.enabled,
                egui::Button::new(label.unwrap_or(metadata.label)),
            );
            let response = if state.enabled {
                response.on_hover_text(metadata.tooltip)
            } else {
                response.on_disabled_hover_text(state.disabled_reason.unwrap_or(metadata.tooltip))
            };
            response.clicked().then_some(*id)
        }
        ContextMenuItem::Submenu { label, items } => ui
            .menu_button(*label, |ui| {
                for item in items {
                    if let Some(action) = render_item(ui, item, action_state) {
                        ui.close();
                        return Some(action);
                    }
                }
                None
            })
            .inner
            .flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::component_visibility::AuthoredComponentKind;

    fn wall_context(viewport: ViewportMode) -> ContextMenuContext {
        ContextMenuContext::viewport(
            viewport,
            ComponentKey::authored(AuthoredComponentKind::Wall, "wall-1"),
        )
    }

    fn action_ids(model: &ContextMenuModel) -> Vec<ActionId> {
        fn collect(item: &ContextMenuItem, ids: &mut Vec<ActionId>) {
            match item {
                ContextMenuItem::Action { id, .. } => ids.push(*id),
                ContextMenuItem::Submenu { items, .. } => {
                    for item in items {
                        collect(item, ids);
                    }
                }
            }
        }

        let mut ids = Vec::new();
        for section in &model.sections {
            for item in &section.items {
                collect(item, &mut ids);
            }
        }
        ids
    }

    #[test]
    fn axonometric_component_menu_has_stable_presentation_action_order() {
        let model = build_context_menu(&wall_context(ViewportMode::Axonometric));

        assert_eq!(model.sections.len(), 2);
        assert_eq!(
            action_ids(&model),
            vec![
                ActionId::IsolateDim,
                ActionId::IsolateHide,
                ActionId::ExitIsolation,
                ActionId::HideSelection,
                ActionId::ShowAllComponents,
            ]
        );
    }

    #[test]
    fn non_3d_viewports_and_model_browser_do_not_inherit_the_3d_menu() {
        for viewport in [
            ViewportMode::Plan,
            ViewportMode::RoofPlan,
            ViewportMode::Elevation,
            ViewportMode::Render,
        ] {
            assert!(build_context_menu(&wall_context(viewport)).is_empty());
        }

        let browser = ContextMenuContext {
            surface: ContextMenuSurface::ModelBrowser,
            target: ContextMenuTarget::Component(ComponentKey::authored(
                AuthoredComponentKind::Wall,
                "wall-1",
            )),
        };
        assert!(build_context_menu(&browser).is_empty());
    }
}
