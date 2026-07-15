//! One logical viewport pane's canvas dispatcher.
//!
//! This module is the borrow boundary between the root app, docked tiles, and
//! deferred native viewport callbacks. Renderers receive explicit read-only
//! inputs plus one pane-owned runtime; all app/history work is returned as an
//! owned, target-tagged event bundle.

use std::sync::Arc;

use eframe::egui::{Pos2, Rect, Response, Ui};
use eframe::wgpu;
use framer_core::{BuildingModel, DimensionAnchor, DimensionAxis, Length, Point2};
use framer_geometry::{GeometryViolation, PhysicalScene};
use framer_solver::ProjectFramePlan;

use super::axonometric::{AxonometricResponse, AxonometricView, draw_project_axonometric};
use super::elevation_design::{
    DesignElevationClick, DesignElevationView, draw_wall_design_elevation,
};
use super::elevation_framing::{BuildUpContext, draw_wall_elevation, section_position};
use super::elevation_openings::OpeningDragEvent;
use super::plan::{PlanView, WallDragEvent, draw_project_plan};
use super::render::{RenderView, draw_project_render};
use super::view_common::viewport_size;
use super::{DrawWallPlanInput, ViewportPaneRuntime};
use crate::app::actions::ActionId;
use crate::app::component_visibility::{ComponentKey, ComponentVisibility, SelectionOp};
use crate::app::draw_wall::SnapResult;
use crate::app::model_edit::{OpeningDragState, WallEditHandle};
use crate::app::{RenderSettings, Selection, ViewClick, ViewLayers, ViewportMode, WorkspaceMode};

/// GPU capabilities shared by every pane drawn against one egui render state.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PaneGpuInput {
    pub(super) target_format: Option<wgpu::TextureFormat>,
    pub(super) depth_format: Option<wgpu::TextureFormat>,
    pub(super) compute_ok: bool,
    pub(super) ray_query_ok: bool,
    pub(super) ray_query_enabled: bool,
}

/// Borrowed modal-tool state for one root frame.
///
/// The owned deferred snapshot deliberately does not retain these fields. Its
/// [`OwnedPaneFrame::as_frame`] bridge supplies [`Self::disabled`] instead, so a
/// child callback can navigate and select without synchronously owning an app
/// edit transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PaneToolInput<'a> {
    pub(super) draw_wall_active: bool,
    pub(super) draw_wall_start: Option<Point2>,
    pub(super) snap_step: Option<Length>,
    pub(super) room_tool_active: bool,
    pub(super) ceiling_tool_active: bool,
    pub(super) vault_tool_active: bool,
    pub(super) floor_tool_active: bool,
    pub(super) dimension_tool_active: bool,
    pub(super) dimension_tool_axis: DimensionAxis,
    pub(super) first_dimension_anchor: Option<&'a DimensionAnchor>,
    pub(super) second_dimension_anchor: Option<&'a DimensionAnchor>,
    pub(super) active_opening_drag: Option<&'a OpeningDragState>,
    pub(super) active_wall_drag: Option<(usize, WallEditHandle)>,
}

impl PaneToolInput<'_> {
    pub(super) const fn disabled() -> Self {
        Self {
            draw_wall_active: false,
            draw_wall_start: None,
            snap_step: None,
            room_tool_active: false,
            ceiling_tool_active: false,
            vault_tool_active: false,
            floor_tool_active: false,
            dimension_tool_active: false,
            dimension_tool_axis: DimensionAxis::Horizontal,
            first_dimension_anchor: None,
            second_dimension_anchor: None,
            active_opening_drag: None,
            active_wall_drag: None,
        }
    }
}

impl Default for PaneToolInput<'_> {
    fn default() -> Self {
        Self::disabled()
    }
}

/// All read-only inputs used to paint one pane in one app frame.
///
/// Keeping this free of `&mut FramerApp` lets the root render several panes from
/// one coherent frame and lets a deferred callback borrow an owned snapshot via
/// [`OwnedPaneFrame::as_frame`].
#[derive(Clone, Copy)]
pub(super) struct PaneFrame<'a> {
    pub(super) model: &'a BuildingModel,
    pub(super) plan: Option<&'a ProjectFramePlan>,
    pub(super) physical_scene: Option<&'a PhysicalScene>,
    pub(super) active_geometry_violation: Option<&'a GeometryViolation>,
    pub(super) selected_wall: usize,
    pub(super) selection: &'a Selection,
    pub(super) selected_components: &'a [ComponentKey],
    pub(super) component_visibility: &'a ComponentVisibility,
    pub(super) workspace_mode: WorkspaceMode,
    pub(super) layers: ViewLayers,
    pub(super) show_section: bool,
    pub(super) render_settings: RenderSettings,
    pub(super) tools: PaneToolInput<'a>,
    pub(super) gpu: PaneGpuInput,
}

/// Clone-owned document/presentation snapshot for a deferred viewport.
///
/// Cameras and progressive render state remain in [`ViewportPaneRuntime`]; this
/// value contains only the immutable state needed to draw against one coherent
/// document revision. Modal tool state is intentionally omitted.
#[derive(Clone)]
pub(super) struct OwnedPaneFrame {
    payload: Arc<OwnedPaneFramePayload>,
    presentation_actions: Vec<PanePresentationAction>,
}

/// Heavy document state shared across deferred panes and root repaints until
/// the next authored-document rebuild.
pub(in crate::app) struct OwnedPaneDocument {
    model: BuildingModel,
    plan: Option<ProjectFramePlan>,
    physical_scene: Option<PhysicalScene>,
}

struct OwnedPaneFramePayload {
    document: Arc<OwnedPaneDocument>,
    active_geometry_violation: Option<GeometryViolation>,
    selected_wall: usize,
    selection: Selection,
    selected_components: Vec<ComponentKey>,
    component_visibility: ComponentVisibility,
    workspace_mode: WorkspaceMode,
    layers: ViewLayers,
    show_section: bool,
    render_settings: RenderSettings,
    gpu: PaneGpuInput,
}

#[derive(Clone)]
pub(super) struct PanePresentationAction {
    pub(super) action: ActionId,
    pub(super) enabled: bool,
    pub(super) disabled_reason: Option<&'static str>,
}

pub(super) const PANE_PRESENTATION_ACTIONS: [ActionId; 5] = [
    ActionId::IsolateDim,
    ActionId::IsolateHide,
    ActionId::ExitIsolation,
    ActionId::HideSelection,
    ActionId::ShowAllComponents,
];

impl OwnedPaneFrame {
    /// Clone the volatile presentation portion of a root-frame input while
    /// sharing its rebuild-scoped document snapshot.
    pub(super) fn from_frame(document: Arc<OwnedPaneDocument>, frame: &PaneFrame<'_>) -> Self {
        Self {
            payload: Arc::new(OwnedPaneFramePayload {
                document,
                active_geometry_violation: frame.active_geometry_violation.cloned(),
                selected_wall: frame.selected_wall,
                selection: frame.selection.clone(),
                selected_components: frame.selected_components.to_vec(),
                component_visibility: frame.component_visibility.clone(),
                workspace_mode: frame.workspace_mode,
                layers: frame.layers,
                show_section: frame.show_section,
                render_settings: frame.render_settings,
                gpu: frame.gpu,
            }),
            presentation_actions: Vec::new(),
        }
    }

    pub(super) fn with_presentation_actions(&self, actions: Vec<PanePresentationAction>) -> Self {
        Self {
            payload: Arc::clone(&self.payload),
            presentation_actions: actions,
        }
    }

    pub(super) fn presentation_actions(&self) -> &[PanePresentationAction] {
        &self.presentation_actions
    }

    /// Borrow this owned snapshot as the same input consumed by a docked pane.
    /// Deferred snapshots never expose an in-progress modal authoring gesture.
    pub(super) fn as_frame(&self) -> PaneFrame<'_> {
        let payload = self.payload.as_ref();
        let document = payload.document.as_ref();
        PaneFrame {
            model: &document.model,
            plan: document.plan.as_ref(),
            physical_scene: document.physical_scene.as_ref(),
            active_geometry_violation: payload.active_geometry_violation.as_ref(),
            selected_wall: payload.selected_wall,
            selection: &payload.selection,
            selected_components: &payload.selected_components,
            component_visibility: &payload.component_visibility,
            workspace_mode: payload.workspace_mode,
            layers: payload.layers,
            show_section: payload.show_section,
            render_settings: payload.render_settings,
            tools: PaneToolInput::disabled(),
            gpu: payload.gpu,
        }
    }
}

impl OwnedPaneDocument {
    /// Deep-clone the document generation consumed by deferred children.
    pub(super) fn from_frame(frame: &PaneFrame<'_>) -> Self {
        Self {
            model: frame.model.clone(),
            plan: frame.plan.cloned(),
            physical_scene: frame.physical_scene.cloned(),
        }
    }
}

/// Which interactions a canvas may return to its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PaneInteractionPolicy {
    pub(super) authoring_gestures: bool,
}

impl PaneInteractionPolicy {
    pub(super) const DOCKED: Self = Self {
        authoring_gestures: true,
    };
    pub(super) const DEFERRED: Self = Self {
        authoring_gestures: false,
    };
}

/// An elevation drag event with enough context for the root app to apply it.
pub(super) struct PaneOpeningDragEvent {
    pub(super) wall_index: usize,
    pub(super) event: OpeningDragEvent,
}

/// Owned, channel-safe events emitted while painting one logical pane.
///
/// `target_id` tags the whole bundle. The egui [`Response`] stays outside this
/// type because it is useful only in the callback that painted the pane.
pub(super) struct PaneCanvasEvents {
    pub(super) target_id: u64,
    pub(super) primary_click: Option<ViewClick>,
    pub(super) secondary_click: Option<ViewClick>,
    /// Captured in the pane callback so deferred selection keeps Command-toggle
    /// semantics when the root drains this event later.
    pub(super) selection_op: SelectionOp,
    pub(super) opening_drag: Option<PaneOpeningDragEvent>,
    pub(super) wall_drag: Option<WallDragEvent>,
    pub(super) cursor_model: Option<Point2>,
    pub(super) toolbar_anchor: Option<Pos2>,
    pub(super) snap: Option<SnapResult>,
}

impl PaneCanvasEvents {
    pub(super) fn new(target_id: u64) -> Self {
        Self {
            target_id,
            primary_click: None,
            secondary_click: None,
            selection_op: SelectionOp::Replace,
            opening_drag: None,
            wall_drag: None,
            cursor_model: None,
            toolbar_anchor: None,
            snap: None,
        }
    }
}

/// Canvas output. The event bundle may cross a channel; the response may not.
pub(super) struct PaneCanvasOutput {
    pub(super) events: PaneCanvasEvents,
    pub(super) axonometric_response: Option<Response>,
}

impl PaneCanvasOutput {
    fn new(target_id: u64) -> Self {
        Self {
            events: PaneCanvasEvents::new(target_id),
            axonometric_response: None,
        }
    }
}

/// Paint one logical pane by dispatching to the existing mode renderer.
///
/// This function owns no app state and performs no model/history mutation. It
/// updates only the supplied pane's cameras/progressive runtime and returns all
/// root-owned work as [`PaneCanvasEvents`].
pub(super) fn draw_pane_canvas(
    ui: &mut Ui,
    target_id: u64,
    mode: ViewportMode,
    frame: PaneFrame<'_>,
    policy: PaneInteractionPolicy,
    runtime: &mut ViewportPaneRuntime,
) -> PaneCanvasOutput {
    let canvas = Rect::from_min_size(ui.next_widget_position(), viewport_size(ui));
    let mut output = PaneCanvasOutput::new(target_id);
    output.events.selection_op = ui.input(|input| {
        if input.modifiers.command {
            SelectionOp::Toggle
        } else {
            SelectionOp::Replace
        }
    });
    let previous_snap = runtime.previous_snap;
    runtime.cursor_model = None;

    let no_selection = Selection::None;
    let multiple_components_selected = frame.selected_components.len() > 1;
    let selection = if multiple_components_selected {
        &no_selection
    } else {
        frame.selection
    };
    let tools = if policy.authoring_gestures {
        frame.tools
    } else {
        PaneToolInput::disabled()
    };

    match mode {
        ViewportMode::Plan | ViewportMode::RoofPlan => {
            let draw_tool = DrawWallPlanInput {
                active: tools.draw_wall_active,
                start: tools.draw_wall_start,
                snap_step: tools.snap_step,
                previous_snap: policy.authoring_gestures.then_some(previous_snap).flatten(),
            };
            let active_wall_drag = (!multiple_components_selected && policy.authoring_gestures)
                .then_some(tools.active_wall_drag)
                .flatten();
            let mut cursor = None;
            let mut toolbar_anchor = None;
            let mut snap = None;
            let mut wall_drag = None;
            let click = draw_project_plan(
                ui,
                PlanView {
                    model: frame.model,
                    selected_wall: frame.selected_wall,
                    selection,
                    layers: frame.layers,
                    draw_tool: &draw_tool,
                    room_tool_active: tools.room_tool_active,
                    ceiling_tool_active: tools.ceiling_tool_active,
                    vault_tool_active: tools.vault_tool_active,
                    floor_tool_active: tools.floor_tool_active,
                    roof_plan_mode: mode == ViewportMode::RoofPlan,
                    active_wall_drag,
                },
                &mut runtime.plan_view,
                &mut cursor,
                &mut toolbar_anchor,
                &mut snap,
                &mut wall_drag,
            );

            runtime.cursor_model = cursor;
            runtime.previous_snap = snap;
            output.events.cursor_model = cursor;
            output.events.toolbar_anchor = toolbar_anchor;
            output.events.snap = snap;
            output.events.wall_drag = policy.authoring_gestures.then_some(wall_drag).flatten();
            output.events.primary_click = filter_click(click, policy);
        }
        ViewportMode::Elevation => {
            runtime.previous_snap = None;
            let Some(wall) = frame.model.walls.get(frame.selected_wall) else {
                ui.label("No wall selected");
                return output;
            };
            let camera = runtime
                .elevation_views
                .entry(wall.id.0.clone())
                .or_default();

            if !frame.workspace_mode.shows_generated_plan() {
                let selected_opening = match selection {
                    Selection::Opening(id) => Some(id.as_str()),
                    _ => None,
                };
                let selected_dimension = match selection {
                    Selection::Dimension(id) => Some(id.as_str()),
                    _ => None,
                };
                let active_opening_drag = (!multiple_components_selected
                    && policy.authoring_gestures)
                    .then_some(tools.active_opening_drag)
                    .flatten()
                    .filter(|drag| drag.wall_index == frame.selected_wall);
                let response = draw_wall_design_elevation(
                    ui,
                    wall,
                    DesignElevationView {
                        selected_opening,
                        selected_dimension,
                        edit_handles_enabled: !multiple_components_selected
                            && policy.authoring_gestures,
                        dimension_tool_active: tools.dimension_tool_active,
                        dimension_tool_axis: tools.dimension_tool_axis,
                        first_dimension_anchor: tools.first_dimension_anchor,
                        second_dimension_anchor: tools.second_dimension_anchor,
                        active_opening_drag,
                    },
                    camera,
                );
                output.events.opening_drag = if policy.authoring_gestures {
                    response.opening_drag.map(|event| PaneOpeningDragEvent {
                        wall_index: frame.selected_wall,
                        event,
                    })
                } else {
                    None
                };
                let click = response.click.map(|click| match click {
                    DesignElevationClick::Opening(opening_id) => ViewClick::Opening {
                        wall_index: frame.selected_wall,
                        opening_id,
                    },
                    DesignElevationClick::Dimension(dimension_id) => ViewClick::Dimension {
                        wall_index: frame.selected_wall,
                        dimension_id,
                    },
                    DesignElevationClick::DimensionAnchor(anchor) => ViewClick::DimensionAnchor {
                        wall_index: frame.selected_wall,
                        anchor,
                    },
                    DesignElevationClick::DimensionPlacement { axis, line_offset } => {
                        ViewClick::DimensionPlacement {
                            wall_index: frame.selected_wall,
                            axis,
                            line_offset,
                        }
                    }
                });
                output.events.primary_click = filter_click(click, policy);
            } else {
                let Some(plan) = frame.plan else {
                    ui.label("No valid framing plan");
                    return output;
                };
                let Some(wall_plan) = plan.wall_plan(&wall.id) else {
                    ui.label("No generated framing for selected wall");
                    return output;
                };
                let selected_member = match selection {
                    Selection::Member {
                        source_id,
                        member_id,
                    } if source_id == &wall.id.0 => Some(member_id.as_str()),
                    _ => None,
                };
                let section_x = frame
                    .show_section
                    .then(|| section_position(wall, selection))
                    .flatten();
                output.events.primary_click = draw_wall_elevation(
                    ui,
                    wall,
                    &wall_plan.members,
                    selected_member,
                    section_x,
                    BuildUpContext {
                        system: frame.model.system_for(wall),
                        materials: &frame.model.materials,
                    },
                    camera,
                )
                .map(|member_id| ViewClick::Member {
                    source_id: wall.id.0.clone(),
                    member_id,
                });
            }
        }
        ViewportMode::Axonometric => {
            runtime.previous_snap = None;
            let (Some(plan), Some(physical_scene)) = (frame.plan, frame.physical_scene) else {
                ui.label("No valid framing plan");
                return output;
            };
            if !frame.selected_components.is_empty()
                || frame.component_visibility.isolation_mode().is_some()
                || frame.component_visibility.has_hidden()
            {
                output.events.toolbar_anchor =
                    Some(Pos2::new(canvas.center().x, canvas.bottom() - 8.0));
            }
            let AxonometricResponse {
                response,
                primary_click,
                secondary_click,
            } = draw_project_axonometric(
                ui,
                AxonometricView {
                    target_id,
                    model: frame.model,
                    plan,
                    physical_scene,
                    active_geometry_violation: frame.active_geometry_violation,
                    selected_components: frame.selected_components,
                    component_visibility: frame.component_visibility,
                    workspace_mode: frame.workspace_mode,
                    wall_display: frame.layers.wall_display,
                    gpu_target_format: frame.gpu.target_format,
                    gpu_depth_format: frame.gpu.depth_format,
                },
                &mut runtime.view_3d,
            );
            output.events.primary_click = primary_click;
            output.events.secondary_click = secondary_click;
            output.axonometric_response = Some(response);
        }
        ViewportMode::Render => {
            runtime.previous_snap = None;
            draw_project_render(
                ui,
                RenderView {
                    target_id,
                    model: frame.model,
                    settings: frame.render_settings,
                    gpu_compute_ok: frame.gpu.compute_ok,
                    gpu_target_format: frame.gpu.target_format,
                    gpu_ray_query_ok: frame.gpu.ray_query_ok,
                    ray_query_enabled: frame.gpu.ray_query_enabled,
                },
                &mut runtime.view_3d,
                &mut runtime.render,
            );
        }
    }

    output
}

/// Modal tool clicks are never allowed to escape a deferred callback. Ordinary
/// selection and empty-canvas clicks remain available for root-side handling.
fn filter_click(click: Option<ViewClick>, policy: PaneInteractionPolicy) -> Option<ViewClick> {
    click.filter(|click| {
        policy.authoring_gestures
            || !matches!(
                click,
                ViewClick::DimensionAnchor { .. }
                    | ViewClick::DimensionPlacement { .. }
                    | ViewClick::DrawWallPoint { .. }
                    | ViewClick::DrawWallCancel
                    | ViewClick::PlaceRoom { .. }
                    | ViewClick::PlaceCeiling { .. }
                    | ViewClick::PlaceFloor { .. }
                    | ViewClick::PlaceVault { .. }
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_snapshot_derivatives_share_payload_and_keep_actions_independent() {
        let model = BuildingModel::demo_wall();
        let selection = Selection::Wall;
        let selected_components = Vec::new();
        let visibility = ComponentVisibility::default();
        let frame = PaneFrame {
            model: &model,
            plan: None,
            physical_scene: None,
            active_geometry_violation: None,
            selected_wall: 0,
            selection: &selection,
            selected_components: &selected_components,
            component_visibility: &visibility,
            workspace_mode: WorkspaceMode::Design,
            layers: ViewLayers::default(),
            show_section: true,
            render_settings: RenderSettings::default(),
            tools: PaneToolInput {
                draw_wall_active: true,
                room_tool_active: true,
                ..PaneToolInput::disabled()
            },
            gpu: PaneGpuInput::default(),
        };

        let document = Arc::new(OwnedPaneDocument::from_frame(&frame));
        let owned = OwnedPaneFrame::from_frame(document, &frame);
        let enabled = owned.with_presentation_actions(vec![PanePresentationAction {
            action: ActionId::HideSelection,
            enabled: true,
            disabled_reason: None,
        }]);
        let disabled = owned.with_presentation_actions(vec![PanePresentationAction {
            action: ActionId::HideSelection,
            enabled: false,
            disabled_reason: Some("Unavailable in this viewport"),
        }]);
        let borrowed = enabled.as_frame();

        assert!(Arc::ptr_eq(&enabled.payload, &disabled.payload));
        assert!(std::ptr::eq(
            enabled.as_frame().model,
            disabled.as_frame().model
        ));
        assert_eq!(borrowed.model.walls, model.walls);
        assert_eq!(borrowed.selection, &Selection::Wall);
        assert_eq!(borrowed.tools, PaneToolInput::disabled());
        assert_eq!(enabled.presentation_actions().len(), 1);
        assert_eq!(
            enabled.presentation_actions()[0].action,
            ActionId::HideSelection
        );
        assert!(enabled.presentation_actions()[0].enabled);
        assert!(enabled.presentation_actions()[0].disabled_reason.is_none());
        assert_eq!(disabled.presentation_actions().len(), 1);
        assert_eq!(
            disabled.presentation_actions()[0].action,
            ActionId::HideSelection
        );
        assert!(!disabled.presentation_actions()[0].enabled);
        assert_eq!(
            disabled.presentation_actions()[0].disabled_reason,
            Some("Unavailable in this viewport")
        );
        assert_eq!(disabled.as_frame().tools, PaneToolInput::disabled());
    }

    #[test]
    fn deferred_policy_filters_authoring_clicks_but_preserves_selection() {
        let selection = filter_click(Some(ViewClick::Wall(3)), PaneInteractionPolicy::DEFERRED);
        assert!(matches!(selection, Some(ViewClick::Wall(3))));

        let empty = filter_click(
            Some(ViewClick::EmptyCanvas),
            PaneInteractionPolicy::DEFERRED,
        );
        assert!(matches!(empty, Some(ViewClick::EmptyCanvas)));

        let authoring = filter_click(
            Some(ViewClick::DrawWallCancel),
            PaneInteractionPolicy::DEFERRED,
        );
        assert!(authoring.is_none());
    }

    #[test]
    fn docked_policy_preserves_authoring_clicks() {
        let click = filter_click(
            Some(ViewClick::PlaceRoom {
                point: Point2::new(Length::ZERO, Length::ZERO),
            }),
            PaneInteractionPolicy::DOCKED,
        );
        assert!(matches!(click, Some(ViewClick::PlaceRoom { .. })));
    }

    #[test]
    fn pane_events_are_target_tagged_and_channel_safe() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send::<PaneCanvasEvents>();
        assert_send_sync::<OwnedPaneFrame>();

        let events = PaneCanvasEvents::new(73);
        assert_eq!(events.target_id, 73);
    }
}
