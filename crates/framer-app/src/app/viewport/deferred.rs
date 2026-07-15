//! Deferred native-window bridge for one logical viewport pane.
//!
//! The child callback owns only pane-local presentation state. Immutable model
//! input arrives as an owned snapshot, while every root-owned action crosses a
//! typed channel and is applied later by the root app.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui::{
    Align, ComboBox, Context, Layout, RichText, Ui, ViewportBuilder, ViewportClass, ViewportId,
    containers::menu::MenuButton,
};

use super::layout::PaneId;
use super::pane_view::{
    OwnedPaneFrame, PaneCanvasEvents, PaneInteractionPolicy, PanePresentationAction,
    draw_pane_canvas,
};
use super::{PaneRuntimeHandle, ViewportPaneRuntime};
use crate::app::ViewportMode;
use crate::app::actions::{self, ActionId};
use crate::app::context_menu::{self, ContextActionState, ContextMenuModel};

const DEFAULT_WINDOW_SIZE: [f32; 2] = [880.0, 640.0];
const MIN_WINDOW_SIZE: [f32; 2] = [320.0, 240.0];

/// Root-owned work emitted by a deferred pane callback.
pub(super) enum DeferredPaneEvent {
    Canvas(Box<PaneCanvasEvents>),
    Activate(PaneId),
    Dock(PaneId),
    SetMode { pane_id: PaneId, mode: ViewportMode },
    Action { pane_id: PaneId, action: ActionId },
}

/// Mutable state captured by egui's `Send + Sync + 'static` deferred callback.
///
/// The runtime remains behind its existing per-pane handle so docked and native
/// drawing use the exact same camera and progressive-render resources.
struct DeferredPaneState {
    mode: ViewportMode,
    snapshot: Option<Arc<OwnedPaneFrame>>,
    runtime: PaneRuntimeHandle,
    native_focused: bool,
    initial_window_size_pending: bool,
    context_menu_model: Option<ContextMenuModel>,
    context_menu_pending: bool,
}

/// Cloneable root/child bridge for one popped-out pane.
#[derive(Clone)]
pub(super) struct DeferredPaneHandle {
    pane_id: PaneId,
    state: Arc<Mutex<DeferredPaneState>>,
    events: Sender<DeferredPaneEvent>,
}

impl DeferredPaneHandle {
    pub(super) fn new(
        pane_id: PaneId,
        mode: ViewportMode,
        runtime: PaneRuntimeHandle,
        events: Sender<DeferredPaneEvent>,
    ) -> Self {
        Self {
            pane_id,
            state: Arc::new(Mutex::new(DeferredPaneState {
                mode,
                snapshot: None,
                runtime,
                native_focused: false,
                initial_window_size_pending: true,
                context_menu_model: None,
                context_menu_pending: false,
            })),
            events,
        }
    }

    /// Replace the immutable document snapshot consumed by the child callback.
    /// Passing `None` leaves the child alive with a compact loading fallback.
    pub(super) fn update_snapshot(&self, snapshot: Option<Arc<OwnedPaneFrame>>) {
        self.lock_state().snapshot = snapshot;
    }

    pub(super) fn set_mode(&self, mode: ViewportMode) {
        let mut state = self.lock_state();
        state.mode = mode;
        if mode != ViewportMode::Axonometric {
            state.context_menu_model = None;
            state.context_menu_pending = false;
        }
    }

    pub(super) fn update_context_menu(&self, model: Option<ContextMenuModel>) {
        let mut state = self.lock_state();
        state.context_menu_model = model;
        state.context_menu_pending = false;
    }

    pub(super) fn mode(&self) -> ViewportMode {
        self.lock_state().mode
    }

    /// Run a docked/root operation against the same runtime used by the child.
    ///
    /// The metadata lock is released before the runtime lock is acquired. The
    /// callback must not recursively lock the same pane runtime.
    pub(super) fn with_runtime<R>(
        &self,
        operation: impl FnOnce(&mut ViewportPaneRuntime) -> R,
    ) -> R {
        let runtime = Arc::clone(&self.lock_state().runtime);
        let mut runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        operation(&mut runtime)
    }

    pub(super) fn viewport_id(&self) -> ViewportId {
        viewport_id_for_pane(self.pane_id)
    }

    fn lock_state(&self) -> MutexGuard<'_, DeferredPaneState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn take_initial_window_size(&self) -> bool {
        let mut state = self.lock_state();
        std::mem::take(&mut state.initial_window_size_pending)
    }

    fn begin_context_menu_request(&self) {
        let mut state = self.lock_state();
        state.context_menu_model = None;
        state.context_menu_pending = true;
    }

    fn clear_context_menu(&self) {
        let mut state = self.lock_state();
        state.context_menu_model = None;
        state.context_menu_pending = false;
    }

    fn context_menu_state(&self) -> (Option<ContextMenuModel>, bool) {
        let state = self.lock_state();
        (state.context_menu_model.clone(), state.context_menu_pending)
    }

    fn emit(&self, ctx: &Context, event: DeferredPaneEvent) {
        if self.events.send(event).is_ok() {
            ctx.request_repaint_of(ViewportId::ROOT);
        }
    }

    fn draw(&self, ui: &mut Ui, viewport_class: ViewportClass) {
        let ctx = ui.ctx().clone();

        // Embedded fallback callbacks run inside the root viewport, whose close
        // request belongs to the application rather than this pane.
        if viewport_class == ViewportClass::Deferred
            && ctx.input(|input| input.viewport().close_requested())
        {
            self.emit(&ctx, DeferredPaneEvent::Dock(self.pane_id));
        }

        let focused = viewport_class == ViewportClass::Deferred
            && ctx.input(|input| input.viewport().focused == Some(true));
        let focus_activated = {
            let mut state = self.lock_state();
            let activated = focused && !state.native_focused;
            state.native_focused = focused;
            activated
        };
        let pointer_activated = ui.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|position| ui.max_rect().contains(position))
        });
        let activated = focus_activated || pointer_activated;
        if activated {
            self.emit(&ctx, DeferredPaneEvent::Activate(self.pane_id));
        }

        let (mode, snapshot) = {
            let state = self.lock_state();
            (state.mode, state.snapshot.clone())
        };
        let presentation_actions = snapshot
            .as_deref()
            .map(OwnedPaneFrame::presentation_actions)
            .unwrap_or_default()
            .to_vec();

        self.draw_header(ui, &ctx, mode, &presentation_actions);
        ui.separator();

        let Some(snapshot) = snapshot else {
            ui.centered_and_justified(|ui| {
                ui.weak("Waiting for the project snapshot…");
            });
            return;
        };
        let mut cursor_changed = false;
        let output = self.with_runtime(|runtime| {
            let previous_cursor_model = runtime.cursor_model;
            let output = draw_pane_canvas(
                ui,
                self.pane_id.get(),
                mode,
                snapshot.as_frame(),
                PaneInteractionPolicy::DEFERRED,
                runtime,
            );
            cursor_changed = runtime.cursor_model != previous_cursor_model;
            output
        });

        let secondary_requested = output.events.secondary_click.is_some();
        if secondary_requested {
            let context_candidate = output
                .events
                .secondary_click
                .as_ref()
                .and_then(|click| click.component_key(snapshot.as_frame().model))
                .is_some_and(|target| target.is_renderable());
            if context_candidate {
                self.begin_context_menu_request();
            } else {
                self.clear_context_menu();
            }
        }

        if let Some(response) = output.axonometric_response.as_ref() {
            let (context_model, context_pending) = self.context_menu_state();
            let mut chosen = None;
            response.context_menu(|ui| {
                if let Some(model) = context_model.as_ref() {
                    chosen = context_menu::render_context_menu(ui, model, |action| {
                        let state = presentation_actions
                            .iter()
                            .find(|candidate| candidate.action == action);
                        ContextActionState {
                            enabled: state.is_some_and(|state| state.enabled),
                            disabled_reason: state.and_then(|state| state.disabled_reason),
                        }
                    });
                    if chosen.is_some() {
                        ui.close();
                    }
                } else if context_pending {
                    ui.weak("Preparing menu…");
                    ui.ctx().request_repaint();
                } else {
                    ui.close();
                }
            });
            if let Some(action) = chosen {
                self.clear_context_menu();
                self.emit(
                    &ctx,
                    DeferredPaneEvent::Action {
                        pane_id: self.pane_id,
                        action,
                    },
                );
            } else if !secondary_requested && !response.context_menu_opened() {
                self.clear_context_menu();
            }
        } else {
            self.clear_context_menu();
        }

        if activated || cursor_changed || canvas_events_need_root(&output.events) {
            self.emit(&ctx, DeferredPaneEvent::Canvas(Box::new(output.events)));
        }
    }

    fn draw_header(
        &self,
        ui: &mut Ui,
        ctx: &Context,
        current_mode: ViewportMode,
        presentation_actions: &[PanePresentationAction],
    ) {
        let mut selected_mode = current_mode;

        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Pane {}", self.pane_id.get())).strong());

            let mode_combo = ComboBox::from_id_salt(("deferred-pane-mode", self.pane_id.get()))
                .width(96.0)
                .selected_text(mode_label(current_mode))
                .show_ui(ui, |ui| {
                    for mode in VIEWPORT_MODES {
                        ui.selectable_value(&mut selected_mode, mode, mode_label(mode));
                    }
                });
            mode_combo.response.widget_info(|| {
                eframe::egui::WidgetInfo::labeled(
                    eframe::egui::WidgetType::ComboBox,
                    true,
                    format!("Pane {} mode", self.pane_id.get()),
                )
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dock = ui.small_button("Dock");
                dock.widget_info(|| {
                    eframe::egui::WidgetInfo::labeled(
                        eframe::egui::WidgetType::Button,
                        true,
                        format!("Dock pane {}", self.pane_id.get()),
                    )
                });
                if dock.clicked() {
                    self.emit(ctx, DeferredPaneEvent::Dock(self.pane_id));
                }
                let (actions_response, _) = MenuButton::new("Actions").ui(ui, |ui| {
                    for (index, action) in presentation_actions.iter().enumerate() {
                        if index == 3 {
                            ui.separator();
                        }
                        let metadata = actions::metadata(action.action);
                        let button = ui
                            .add_enabled(action.enabled, eframe::egui::Button::new(metadata.label));
                        let button = if action.enabled {
                            button.on_hover_text(metadata.tooltip)
                        } else {
                            button.on_disabled_hover_text(
                                action.disabled_reason.unwrap_or(metadata.tooltip),
                            )
                        };
                        if button.clicked() {
                            self.emit(
                                ctx,
                                DeferredPaneEvent::Action {
                                    pane_id: self.pane_id,
                                    action: action.action,
                                },
                            );
                            ui.close();
                        }
                    }
                });
                actions_response.widget_info(|| {
                    eframe::egui::WidgetInfo::labeled(
                        eframe::egui::WidgetType::Button,
                        true,
                        format!("Pane {} actions", self.pane_id.get()),
                    )
                });
                actions_response.on_hover_text("Component visibility and isolation");
            });
        });

        if selected_mode != current_mode {
            self.set_mode(selected_mode);
            self.emit(
                ctx,
                DeferredPaneEvent::SetMode {
                    pane_id: self.pane_id,
                    mode: selected_mode,
                },
            );
        }
    }
}

/// Register/update a pane's deferred viewport from the root app frame.
///
/// On integrations without native multi-viewport support, egui invokes the
/// same callback immediately inside its embedded-window fallback.
pub(super) fn show_deferred_pane(ctx: &Context, handle: &DeferredPaneHandle) {
    let builder = deferred_viewport_builder(handle);
    let callback_handle = handle.clone();

    ctx.show_viewport_deferred(handle.viewport_id(), builder, move |ui, viewport_class| {
        callback_handle.draw(ui, viewport_class)
    });
}

fn deferred_viewport_builder(handle: &DeferredPaneHandle) -> ViewportBuilder {
    let mode = handle.mode();
    let mut builder = ViewportBuilder::default()
        .with_title(format!(
            "Framer — {} · Pane {}",
            mode_label(mode),
            handle.pane_id.get()
        ))
        .with_min_inner_size(MIN_WINDOW_SIZE)
        .with_resizable(true);
    // `inner_size` is a viewport command after creation, so repeating it would
    // overwrite a user's native-window resize on every parent pass.
    if handle.take_initial_window_size() {
        builder = builder.with_inner_size(DEFAULT_WINDOW_SIZE);
    }
    builder
}

const VIEWPORT_MODES: [ViewportMode; 5] = [
    ViewportMode::Plan,
    ViewportMode::RoofPlan,
    ViewportMode::Elevation,
    ViewportMode::Axonometric,
    ViewportMode::Render,
];

fn mode_label(mode: ViewportMode) -> &'static str {
    match mode {
        ViewportMode::Plan => "Plan",
        ViewportMode::RoofPlan => "Roof",
        ViewportMode::Elevation => "Elevation",
        ViewportMode::Axonometric => "3D",
        ViewportMode::Render => "Render",
    }
}

fn viewport_id_for_pane(pane_id: PaneId) -> ViewportId {
    ViewportId::from_hash_of(("framer-deferred-pane", pane_id.get()))
}

fn canvas_events_need_root(events: &PaneCanvasEvents) -> bool {
    events.primary_click.is_some()
        || events.secondary_click.is_some()
        || events.opening_drag.is_some()
        || events.wall_drag.is_some()
        || events.cursor_model.is_some()
        || events.toolbar_anchor.is_some()
        || events.snap.is_some()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::app::component_visibility::{AuthoredComponentKind, ComponentKey};
    use crate::app::context_menu::{ContextMenuContext, build_context_menu};
    use crate::app::viewport::PaneIdGenerator;

    fn pane_ids() -> (PaneId, PaneId) {
        let mut ids = PaneIdGenerator::default();
        (
            ids.allocate().expect("first pane id"),
            ids.allocate().expect("second pane id"),
        )
    }

    #[test]
    fn deferred_viewport_ids_are_stable_and_pane_scoped() {
        let (first, second) = pane_ids();

        assert_eq!(viewport_id_for_pane(first), viewport_id_for_pane(first));
        assert_ne!(viewport_id_for_pane(first), viewport_id_for_pane(second));
        assert_ne!(viewport_id_for_pane(first), ViewportId::ROOT);
    }

    #[test]
    fn handle_uses_the_existing_shared_runtime() {
        let (pane_id, _) = pane_ids();
        let runtime = Arc::new(Mutex::new(ViewportPaneRuntime::default()));
        let initial_yaw = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .view_3d
            .yaw
            .to_bits();
        let (sender, _receiver) = mpsc::channel();
        let handle = DeferredPaneHandle::new(
            pane_id,
            ViewportMode::Axonometric,
            Arc::clone(&runtime),
            sender,
        );

        handle.with_runtime(|runtime| runtime.view_3d.orbit([12.0, -8.0].into()));

        let runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_ne!(runtime.view_3d.yaw.to_bits(), initial_yaw);
    }

    #[test]
    fn deferred_bridge_and_callback_are_thread_safe() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync>() {}
        fn assert_callback<F>(_: F)
        where
            F: Fn(&mut Ui, ViewportClass) + Send + Sync + 'static,
        {
        }

        assert_send::<DeferredPaneEvent>();
        assert_send_sync::<DeferredPaneHandle>();

        let (pane_id, _) = pane_ids();
        let runtime = Arc::new(Mutex::new(ViewportPaneRuntime::default()));
        let (sender, _receiver) = mpsc::channel();
        let handle =
            DeferredPaneHandle::new(pane_id, ViewportMode::Plan, Arc::clone(&runtime), sender);
        assert_callback(move |ui, viewport_class| handle.draw(ui, viewport_class));
    }

    #[test]
    fn deferred_viewport_default_size_is_only_requested_once() {
        let (pane_id, _) = pane_ids();
        let runtime = Arc::new(Mutex::new(ViewportPaneRuntime::default()));
        let (sender, _receiver) = mpsc::channel();
        let handle =
            DeferredPaneHandle::new(pane_id, ViewportMode::Plan, Arc::clone(&runtime), sender);

        let initial = deferred_viewport_builder(&handle);
        let update = deferred_viewport_builder(&handle);

        assert_eq!(initial.inner_size, Some(DEFAULT_WINDOW_SIZE.into()));
        assert_eq!(update.inner_size, None);
        assert_eq!(
            update.min_inner_size,
            Some(MIN_WINDOW_SIZE.into()),
            "minimum size remains a persistent window constraint"
        );
        assert_eq!(update.resizable, Some(true));
    }

    #[test]
    fn deferred_context_menu_waits_for_and_installs_root_composition() {
        let (pane_id, _) = pane_ids();
        let runtime = Arc::new(Mutex::new(ViewportPaneRuntime::default()));
        let (sender, _receiver) = mpsc::channel();
        let handle = DeferredPaneHandle::new(pane_id, ViewportMode::Axonometric, runtime, sender);

        handle.begin_context_menu_request();
        let (model, pending) = handle.context_menu_state();
        assert!(model.is_none());
        assert!(pending);

        let context = ContextMenuContext::viewport(
            ViewportMode::Axonometric,
            ComponentKey::authored(AuthoredComponentKind::Wall, "wall-1"),
        );
        handle.update_context_menu(Some(build_context_menu(&context)));
        let (model, pending) = handle.context_menu_state();
        assert!(model.is_some_and(|model| !model.is_empty()));
        assert!(!pending);

        handle.clear_context_menu();
        let (model, pending) = handle.context_menu_state();
        assert!(model.is_none());
        assert!(!pending);
    }
}
