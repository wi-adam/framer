//! The 3D workspace (axonometric) renderer: orbit/pan/zoom input, scene paint via
//! the GPU callback, view-cube interaction, and picking. Uses the
//! `AxonometricView<'_>` bundle to keep the call site legible.

use eframe::egui::{self, Sense, Ui};
use eframe::{egui_wgpu, wgpu};
use framer_core::BuildingModel;
use framer_solver::ProjectFramePlan;

use super::camera_3d::View3dState;
use super::geom::OrbitProjector;
use super::gpu::{Framer3dCallback, Framer3dFrameKey, GpuUniforms};
use super::scene_build::Scene3d;
use super::theme;
use super::view_common::{
    draw_view_background, draw_view_border, draw_view_empty, draw_view_title,
    pointer_started_in_rect, viewport_drawing_rect, viewport_size,
};
use super::view_cube::{draw_view_cube, view_cube_rect};
use crate::app::{Selection, ViewClick, WorkspaceMode};

// === extracted block appended below; visibility adjusted in place ===

pub(super) struct AxonometricView<'a> {
    pub(super) model: &'a BuildingModel,
    pub(super) plan: &'a ProjectFramePlan,
    pub(super) selected_wall: usize,
    pub(super) selection: &'a Selection,
    pub(super) workspace_mode: WorkspaceMode,
    pub(super) gpu_target_format: Option<wgpu::TextureFormat>,
}

pub(super) fn draw_project_axonometric(
    ui: &mut Ui,
    axonometric: AxonometricView<'_>,
    view: &mut View3dState,
) -> Option<ViewClick> {
    let AxonometricView {
        model,
        plan,
        selected_wall,
        selection,
        workspace_mode,
        gpu_target_format,
    } = axonometric;

    let desired = viewport_size(ui);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    draw_view_background(&painter, rect, theme::sheet());
    let drawing = viewport_drawing_rect(rect, 42.0);
    draw_view_border(&painter, drawing);
    let cube_rect = view_cube_rect(drawing);
    let pointer = response.interact_pointer_pos();
    let cube_hover_pointer = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|position| cube_rect.contains(*position));
    let press_origin = ui.input(|input| input.pointer.press_origin());
    let shift = ui.input(|input| input.modifiers.shift);
    let dragging_primary = response.dragged_by(egui::PointerButton::Primary);
    let dragging_middle = response.dragged_by(egui::PointerButton::Middle);
    // Middle-drag or Shift+left-drag pans; plain left-drag orbits. The shared pan
    // state keeps the Render view and this view on the same vantage.
    let panning = dragging_middle || (dragging_primary && shift);
    let orbiting = dragging_primary && !shift;
    let dragging_from_cube = orbiting && pointer_started_in_rect(press_origin, cube_rect);

    if panning {
        view.pan(response.drag_delta(), drawing.width().min(drawing.height()));
    } else if orbiting {
        view.orbit(response.drag_delta());
    }

    if response.hovered() {
        let zoom_factor = ui.input(|input| {
            let wheel_factor = (input.smooth_scroll_delta.y * 0.002).exp();
            wheel_factor * input.zoom_delta()
        });
        if (zoom_factor - 1.0).abs() > f32::EPSILON {
            view.zoom_by(zoom_factor);
        }
    }

    let Some(scene) = Scene3d::from_project(model, plan, selected_wall, selection, workspace_mode)
    else {
        draw_view_empty(&painter, rect, "No 3D geometry");
        return None;
    };
    let Some(projector) = OrbitProjector::from_points(&scene.points, drawing, *view) else {
        draw_view_empty(&painter, rect, "No wall segments");
        return None;
    };

    let clicked = pointer
        .filter(|position| !cube_rect.contains(*position))
        .and_then(|position| scene.pick(position, &projector))
        .filter(|_| response.clicked());

    if let Some(target_format) = gpu_target_format {
        let callback = egui_wgpu::Callback::new_paint_callback(
            drawing,
            Framer3dCallback {
                frame_key: Framer3dFrameKey::MODEL,
                vertices: scene.vertices,
                indices: scene.indices,
                opaque_index_count: scene.opaque_index_count,
                transparent_index_count: scene.transparent_index_count,
                uniforms: GpuUniforms::from_projector(&projector, drawing),
                target_format,
            },
        );
        painter.add(callback);
    } else {
        draw_view_empty(&painter, drawing, "WGPU renderer unavailable");
    }

    let cube_action = draw_view_cube(
        &painter,
        cube_rect,
        if dragging_from_cube {
            pointer.or(cube_hover_pointer)
        } else {
            cube_hover_pointer
        },
        response.clicked() && !dragging_from_cube,
        *view,
        gpu_target_format,
    );
    if let Some(action) = cube_action {
        view.snap_to(action);
        return None;
    }

    draw_view_title(&painter, drawing, "3D workspace");

    clicked
}
