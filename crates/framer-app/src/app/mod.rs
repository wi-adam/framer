mod actions;
mod design;
mod draw_wall;
mod history;
#[cfg(test)]
mod history_integration_tests;
mod labels;
mod model_edit;
mod panels;
mod project_io;
mod render;
mod render_job;
mod theme;
#[cfg(test)]
mod ui_harness_tests;
mod viewport;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use eframe::egui::{self, CentralPanel, Frame, Panel, ScrollArea};
use framer_core::{
    BuildingModel, DimensionAnchor, DimensionAxis, DimensionConstraint, DimensionDirection,
    DimensionKind, ElementId, Length, Opening, OpeningKind, Point2, Room, RoomUsage, Wall,
    concave_polygon_corners, level_wall_loop_outline, load_project as load_project_document,
    save_project as save_project_document,
};
use framer_solver::{
    DiagnosticSeverity, FrameMember, PlanDiagnostic, ProjectFramePlan, export_bom_csv,
    export_project_svg, generate_project_plan,
};
use framer_standards::ComplianceReport;

use draw_wall::{SnapResult, joins_for_new_wall};
use history::History;
use model_edit::{
    OpeningDragConstraints, OpeningDragState, OpeningEditHandle, WallDragState, WallEditHandle,
    apply_opening_drag, endpoint_move_keeps_ortho, endpoint_move_keeps_positive_length,
    next_ceiling_id, next_dimension_id, next_floor_id, next_material_id, next_opening_id,
    next_roof_id, next_room_id, next_standards_pack_id, next_system_id, next_wall_id,
    translate_keeps_ortho, translate_keeps_positive_length,
};
use project_io::{DEFAULT_PROJECT_PATH, compliance_report_path, export_paths, write_text_file};
use viewport::{View2dState, View3dState, WallDragEvent};

pub(crate) struct FramerApp {
    model: BuildingModel,
    selected_wall: usize,
    selected: Selection,
    project_plan: Option<ProjectFramePlan>,
    compliance_report: Option<ComplianceReport>,
    library_issues: Vec<framer_library::LibraryIssue>,
    library_issue_error: Option<String>,
    error: Option<String>,
    project_path: String,
    file_status: Option<String>,
    artifact_status: Option<String>,
    dimension_status: Option<String>,
    command_tab: actions::WorkflowTab,
    command_search: CommandSearchState,
    workspace_mode: WorkspaceMode,
    viewport_mode: ViewportMode,
    view_3d: View3dState,
    /// Pan/zoom camera for the whole-project Plan ("shell") view.
    plan_view: View2dState,
    /// Per-wall pan/zoom cameras for the Elevation ("wall") views, keyed by wall
    /// id and shared across the Design- and Plan-workspace elevation variants.
    /// Presentation state: never serialized, cleared on new/load.
    elevation_views: HashMap<String, View2dState>,
    render_view: render_job::RenderViewState,
    render_gpu: render::GpuRenderState,
    /// Frames remaining in "camera moving" mode (hysteresis after the last orbit/
    /// zoom input), used to drop the Render view to a lower internal resolution
    /// while interacting so orbiting stays smooth. 0 = settled / full resolution.
    render_motion_cooldown: u32,
    dimension_tool: DimensionToolState,
    draw_wall_tool: DrawWallToolState,
    room_tool_active: bool,
    /// The flat-ceiling and floor-deck placement tools: like the room tool, each
    /// commits its object when the user clicks inside an enclosed wall loop.
    ceiling_tool_active: bool,
    floor_tool_active: bool,
    /// The vault tool: a region-gated tool that authors a scissor/vault as two
    /// opposing sloped ceilings over the enclosed loop under the click.
    vault_tool_active: bool,
    opening_drag: Option<OpeningDragState>,
    /// In-progress drag of a wall endpoint handle in the plan view.
    wall_drag: Option<WallDragState>,
    gpu_target_format: Option<eframe::wgpu::TextureFormat>,
    /// Whether the active adapter supports compute shaders (GPU path tracer);
    /// when false the Render view falls back to the CPU renderer.
    gpu_compute_ok: bool,
    /// Smoke test for the GPU path-trace callback: when `FRAMER_RENDER_SMOKE=N`
    /// is set, force the Render view for N frames then close. `None` normally.
    render_smoke: Option<u32>,
    show_section: bool,
    /// Visual-layering state for the Plan and 3D views (wall display mode +
    /// per-layer visibility), driven by the Layers popover.
    layers: ViewLayers,
    /// Active drafting level for newly authored level-owned objects. Presentation
    /// state only: never serialized and clamped to the current model on rebuild.
    active_level: Option<ElementId>,
    ortho: bool,
    snap_step: Option<Length>,
    cursor_model: Option<Point2>,
    /// Undo/redo history of authored-model edits. Ephemeral presentation state:
    /// never serialized, cleared on load/new/reset. See
    /// `docs/plans/2026-06-17-undo-redo-design.md`.
    history: History<Snapshot>,
}

/// Maximum number of undo steps retained; oldest evicted past this. Snapshots
/// are KB-scale clones, so a deep history is cheap.
const HISTORY_LIMIT: usize = 200;

/// One restorable point: the authored document plus the transient selection we
/// restore alongside it. Not serialized.
#[derive(Clone)]
struct Snapshot {
    model: BuildingModel,
    selected: Selection,
    selected_wall: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Site,
    Level(String),
    Wall,
    Opening(String),
    Dimension(String),
    Join(String),
    Room(String),
    Member { wall_id: String, member_id: String },
    RoofPlane(String),
    Ceiling(String),
    FloorDeck(String),
    System(String),
    Material(String),
    Furnishing(String),
    MepObject(String),
    StandardsPack(String),
    FurnishingInstance(String),
    MepInstance(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceMode {
    Design,
    Plan,
}

impl WorkspaceMode {
    fn allows_design_edits(self) -> bool {
        matches!(self, Self::Design)
    }

    fn shows_generated_plan(self) -> bool {
        matches!(self, Self::Plan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewportMode {
    Plan,
    /// A top-down roof authoring view: the plan footprint with the authored roof
    /// planes drawn and selectable on top. Reuses the 2-D plan machinery.
    RoofPlan,
    Elevation,
    Axonometric,
    Render,
}

/// The roof form the auto-from-footprint roof tool generates. The tool writes
/// explicit roof planes into the model; the form choice is transient authoring
/// input, not a persisted roof-assembly parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoofForm {
    Gable,
    Shed,
    Hip,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CommandSearchState {
    open: bool,
    focus_input: bool,
    query: String,
}

/// One generated roof-plane outline: its plan-projected polygon and the index of
/// its eave (downslope) edge. The rest of a [`framer_core::RoofPlane`] (id, name,
/// level, system, pitch, springing) is filled in by `add_roof`.
type RoofPlaneSpec = (Vec<Point2>, u32);

fn orthogonal_valley_roof_specs(outline: &[Point2]) -> Option<Vec<RoofPlaneSpec>> {
    let concave_corners = concave_polygon_corners(outline);
    let [c] = concave_corners.as_slice() else {
        return None;
    };

    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for point in outline {
        min_x = min_x.min(point.x.ticks());
        min_y = min_y.min(point.y.ticks());
        max_x = max_x.max(point.x.ticks());
        max_y = max_y.max(point.y.ticks());
    }

    let concave_index = outline.iter().position(|point| point == c)?;
    let c = *c;
    let previous = outline[(concave_index + outline.len() - 1) % outline.len()];
    let next = outline[(concave_index + 1) % outline.len()];
    let horizontal = if previous.y == c.y {
        previous
    } else if next.y == c.y {
        next
    } else {
        return None;
    };
    let vertical = if previous.x == c.x {
        previous
    } else if next.x == c.x {
        next
    } else {
        return None;
    };
    let sign_x = (horizontal.x - c.x).ticks().signum();
    let sign_y = (vertical.y - c.y).ticks().signum();
    if sign_x == 0 || sign_y == 0 {
        return None;
    }

    let corner_x = if sign_x > 0 { min_x } else { max_x };
    let corner_y = if sign_y > 0 { min_y } else { max_y };
    let far_x = if sign_x > 0 { max_x } else { min_x };
    let far_y = if sign_y > 0 { max_y } else { min_y };
    if (c.x.ticks() - corner_x).abs() != (c.y.ticks() - corner_y).abs()
        || (far_x - c.x.ticks()).abs() != (far_y - c.y.ticks()).abs()
    {
        return None;
    }
    let p = |x, y| Point2::new(Length::from_ticks(x), Length::from_ticks(y));
    let shared_low = p(corner_x, corner_y);
    let horizontal_low = p(far_x, corner_y);
    let vertical_low = p(corner_x, far_y);

    Some(vec![
        (vec![shared_low, horizontal_low, horizontal, c], 0),
        (vec![shared_low, vertical_low, vertical, c], 0),
    ])
}

/// Split a region outline's bounding box into the two opposing halves of a scissor
/// vault, divided by a ridge along the longer span. Each half is a rectangle whose
/// **edge 0 is its outer (spring) wall**, so a [`framer_core::CeilingSlope`] with
/// `low_edge: 0` springs there and rises to the shared ridge. `None` for a
/// degenerate (zero-area) region. v2 vaults the axis-aligned bounding box (like the
/// rest of the framing); a non-rectangular room is covered by its bbox.
fn scissor_halves(outline: &[Point2]) -> Option<(Vec<Point2>, Vec<Point2>)> {
    if outline.len() < 3 {
        return None;
    }
    let (mut min, mut max) = (outline[0], outline[0]);
    for p in outline {
        min = Point2::new(min.x.min(p.x), min.y.min(p.y));
        max = Point2::new(max.x.max(p.x), max.y.max(p.y));
    }
    let (width, depth) = (max.x - min.x, max.y - min.y);
    if width <= Length::ZERO || depth <= Length::ZERO {
        return None;
    }
    let p = Point2::new;
    if width >= depth {
        // Ridge along x at mid-depth; halves spring from the y-min / y-max walls.
        let mid = min.y + depth / 2;
        let low = vec![
            p(min.x, min.y),
            p(max.x, min.y),
            p(max.x, mid),
            p(min.x, mid),
        ];
        let high = vec![
            p(min.x, max.y),
            p(max.x, max.y),
            p(max.x, mid),
            p(min.x, mid),
        ];
        Some((low, high))
    } else {
        // Ridge along y at mid-width; halves spring from the x-min / x-max walls.
        let mid = min.x + width / 2;
        let low = vec![
            p(min.x, min.y),
            p(min.x, max.y),
            p(mid, max.y),
            p(mid, min.y),
        ];
        let high = vec![
            p(max.x, min.y),
            p(max.x, max.y),
            p(mid, max.y),
            p(mid, min.y),
        ];
        Some((low, high))
    }
}

/// How walls are drawn in the Plan and 3D views. A single shared presentation
/// setting (not per-view) so toggling it reads consistently across both. The
/// cleanest mode is the default so a fresh shell reads as an outline first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum WallDisplay {
    /// A single line per wall: no thickness, no color. Default.
    #[default]
    Outline,
    /// Wall thickness without color — 2D: two dashed face lines; 3D: a
    /// monochrome full-thickness volume.
    Width,
    /// True-thickness colored construction-layer bands (2D opaque, 3D
    /// translucent so framing shows through).
    Full,
}

impl WallDisplay {
    /// Short label for the Layers popover selector.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Outline => "Outline",
            Self::Width => "Width",
            Self::Full => "Full",
        }
    }
}

/// The visual-layering state for the Plan and 3D views: the wall display mode
/// plus per-layer visibility toggles. Presentation state only — never
/// serialized; re-initialized to defaults each launch (everything visible,
/// walls as outlines).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ViewLayers {
    pub(crate) wall_display: WallDisplay,
    pub(crate) grid: bool,
    pub(crate) rooms: bool,
    pub(crate) joins: bool,
    pub(crate) wall_labels: bool,
}

impl Default for ViewLayers {
    fn default() -> Self {
        Self {
            wall_display: WallDisplay::Outline,
            grid: true,
            rooms: true,
            joins: true,
            wall_labels: true,
        }
    }
}

#[derive(Clone)]
enum ViewClick {
    Wall(usize),
    Opening {
        wall_index: usize,
        opening_id: String,
    },
    Dimension {
        wall_index: usize,
        dimension_id: String,
    },
    DimensionAnchor {
        wall_index: usize,
        anchor: DimensionAnchor,
    },
    DimensionPlacement {
        wall_index: usize,
        axis: DimensionAxis,
        line_offset: Length,
    },
    /// A draw-wall tool click committing the next polyline point (already
    /// resolved through ortho/grid/endpoint snapping in the plan view).
    DrawWallPoint {
        point: Point2,
    },
    /// Cancel the in-progress draw-wall run (e.g. right-click) without leaving
    /// the tool.
    DrawWallCancel,
    /// A room-tool click placing a room at a model point inside a closed loop.
    PlaceRoom {
        point: Point2,
    },
    /// A ceiling-tool click placing a flat ceiling over the loop under the point.
    PlaceCeiling {
        point: Point2,
    },
    /// A floor-tool click placing a floor deck over the loop under the point.
    PlaceFloor {
        point: Point2,
    },
    /// A vault-tool click authoring a scissor/vault (two opposing sloped ceilings)
    /// over the loop under the point.
    PlaceVault {
        point: Point2,
    },
    /// Select an existing room (e.g. clicking its fill in the plan).
    Room {
        room_id: String,
    },
    FurnishingInstance {
        instance_id: String,
    },
    MepInstance {
        instance_id: String,
    },
    Member {
        wall_id: String,
        member_id: String,
    },
    /// Select an authored roof plane (its surface solid in the 3D view).
    RoofPlane {
        id: String,
    },
    /// Select an authored flat ceiling (its surface slab in the 3D view).
    Ceiling {
        id: String,
    },
    /// Select an authored floor deck (its surface slab in the 3D view).
    FloorDeck {
        id: String,
    },
}

#[derive(Debug, Clone)]
struct DimensionToolState {
    active: bool,
    kind: DimensionKind,
    axis: DimensionAxis,
    first_anchor: Option<DimensionAnchorPick>,
    second_anchor: Option<DimensionAnchorPick>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DimensionAnchorPick {
    wall_index: usize,
    anchor: DimensionAnchor,
}

impl Default for DimensionToolState {
    fn default() -> Self {
        Self {
            active: false,
            kind: DimensionKind::Driving,
            axis: DimensionAxis::Horizontal,
            first_anchor: None,
            second_anchor: None,
        }
    }
}

impl DimensionToolState {
    fn clear_picks(&mut self) {
        self.first_anchor = None;
        self.second_anchor = None;
    }
}

/// State for the interactive draw-wall tool. `start` holds the first committed
/// endpoint while a wall (or polyline run) is being drawn; `None` means the next
/// click begins a new segment. `previous_snap` carries the last frame's snap so
/// the plan view can apply sticky hysteresis.
#[derive(Debug, Clone, Default)]
struct DrawWallToolState {
    active: bool,
    start: Option<Point2>,
    previous_snap: Option<SnapResult>,
}

fn dimension_kind_name(kind: DimensionKind) -> &'static str {
    match kind {
        DimensionKind::Driving => "driving",
        DimensionKind::Reference => "reference",
    }
}

fn dimension_axis_name(axis: DimensionAxis) -> &'static str {
    match axis {
        DimensionAxis::Horizontal => "horizontal",
        DimensionAxis::Vertical => "vertical",
    }
}

impl Default for FramerApp {
    fn default() -> Self {
        let mut app = Self {
            model: BuildingModel::demo_shell(),
            selected_wall: 0,
            selected: Selection::Wall,
            project_plan: None,
            compliance_report: None,
            library_issues: Vec::new(),
            library_issue_error: None,
            error: None,
            project_path: DEFAULT_PROJECT_PATH.to_owned(),
            file_status: None,
            artifact_status: None,
            dimension_status: None,
            command_tab: actions::WorkflowTab::Frame,
            command_search: CommandSearchState::default(),
            workspace_mode: WorkspaceMode::Design,
            viewport_mode: ViewportMode::Plan,
            view_3d: View3dState::default(),
            plan_view: View2dState::default(),
            elevation_views: HashMap::new(),
            render_view: render_job::RenderViewState::default(),
            render_gpu: render::GpuRenderState::default(),
            render_motion_cooldown: 0,
            dimension_tool: DimensionToolState::default(),
            draw_wall_tool: DrawWallToolState::default(),
            room_tool_active: false,
            ceiling_tool_active: false,
            vault_tool_active: false,
            floor_tool_active: false,
            opening_drag: None,
            wall_drag: None,
            gpu_target_format: None,
            gpu_compute_ok: false,
            render_smoke: None,
            show_section: true,
            layers: ViewLayers::default(),
            active_level: Some(ElementId::new("level-1")),
            ortho: true,
            snap_step: Some(Length::from_whole_inches(1)),
            cursor_model: None,
            history: History::new(HISTORY_LIMIT),
        };
        app.rebuild();
        app
    }
}

impl FramerApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        design::install(&cc.egui_ctx, design::studio_light());

        let render_state = cc.wgpu_render_state.as_ref();
        Self {
            gpu_target_format: render_state.map(|rs| rs.target_format),
            // The GPU path tracer needs compute shaders; otherwise fall back to CPU.
            gpu_compute_ok: render_state.is_some_and(|rs| {
                rs.adapter
                    .get_downlevel_capabilities()
                    .flags
                    .contains(eframe::wgpu::DownlevelFlags::COMPUTE_SHADERS)
            }),
            render_smoke: std::env::var("FRAMER_RENDER_SMOKE")
                .ok()
                .map(|v| v.parse().unwrap_or(180)),
            ..Self::default()
        }
    }

    fn rebuild(&mut self) {
        if self.selected_wall >= self.model.walls.len() {
            self.selected_wall = 0;
            self.selected = Selection::Wall;
        }
        self.reconcile_active_level();

        // Drop per-wall cameras whose wall no longer exists, so `elevation_views`
        // stays in sync with the model however a wall is removed (keys are wall
        // ids). new/load clear it wholesale via `reset_2d_cameras`; this covers
        // any future single-wall deletion without it having to remember to prune.
        if !self.elevation_views.is_empty() {
            let live: std::collections::HashSet<&str> = self
                .model
                .walls
                .iter()
                .map(|wall| wall.id.0.as_str())
                .collect();
            self.elevation_views
                .retain(|id, _| live.contains(id.as_str()));
        }

        self.model.apply_driving_dimensions();

        match collect_library_lifecycle_issues(&self.model) {
            Ok(issues) => {
                self.library_issues = issues;
                self.library_issue_error = None;
            }
            Err(error) => {
                self.library_issues.clear();
                self.library_issue_error = Some(error);
            }
        }

        match generate_project_plan(&self.model) {
            Ok(mut plan) => {
                let resolved_standards = self.model.resolved_standards();
                let report = framer_standards::evaluate(&self.model, &resolved_standards, &plan);
                plan.diagnostics
                    .extend(framer_standards::diagnostics(&report));
                append_library_diagnostics(
                    &mut plan,
                    &self.library_issues,
                    self.library_issue_error.as_deref(),
                );
                self.compliance_report = Some(report);
                self.project_plan = Some(plan);
                self.error = None;
            }
            Err(error) => {
                self.project_plan = None;
                self.compliance_report = None;
                self.error = Some(error.to_string());
            }
        }
    }

    /// Capture the current restorable state (authored model + selection).
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            model: self.model.clone(),
            selected: self.selected.clone(),
            selected_wall: self.selected_wall,
        }
    }

    /// Restore a snapshot's model and selection. Does not re-solve; callers
    /// follow with `rebuild()`.
    fn restore(&mut self, snapshot: Snapshot) {
        self.model = snapshot.model;
        self.selected = snapshot.selected;
        self.selected_wall = snapshot.selected_wall;
    }

    /// Run a discrete document edit, recording one undo step labelled `label`
    /// iff the authored model actually changed. Always re-solves afterwards,
    /// matching the previous unconditional `rebuild()` on every mutation.
    fn edit(&mut self, label: &str, f: impl FnOnce(&mut Self)) {
        let before = self.snapshot();
        f(self);
        self.rebuild();
        if self.model != before.model {
            self.history.record(before, label);
        }
    }

    /// Open or refresh the edit transaction for an in-progress immediate-mode
    /// edit (inspector field run). `base` is the pre-edit state captured at the
    /// start of the interaction; `label` describes the edit. Coalesces a whole
    /// gesture into one undo step — the first call opens it, the rest are
    /// absorbed.
    fn begin_inspector_edit(&mut self, base: Snapshot, label: &str) {
        self.history.begin(base, label);
    }

    /// Finalize any open edit transaction. `interaction_active` is true while
    /// the user is still mid-gesture (pointer down or a widget focused); the
    /// transaction is only settled once the gesture ends. A settled gesture
    /// that left the model unchanged is dropped rather than recorded.
    fn settle_history(&mut self, interaction_active: bool) {
        if interaction_active {
            return;
        }
        let changed = match self.history.pending_base() {
            None => return,
            Some(base) => self.model != base.model,
        };
        if changed {
            self.history.commit();
        } else {
            self.history.cancel_pending();
        }
    }

    fn undo(&mut self) {
        self.settle_history(false);
        let current = self.snapshot();
        if let Some(previous) = self.history.undo(current) {
            self.restore(previous);
            self.rebuild();
        }
    }

    fn redo(&mut self) {
        self.settle_history(false);
        let current = self.snapshot();
        if let Some(next) = self.history.redo(current) {
            self.restore(next);
            self.rebuild();
        }
    }

    /// Clears the transient 2D view cameras (pan/zoom). Called whenever the
    /// model is replaced wholesale, so cameras don't carry stale framing or
    /// dangling wall-id keys into a different document.
    fn reset_2d_cameras(&mut self) {
        self.plan_view = View2dState::default();
        self.elevation_views.clear();
    }

    fn reset_active_level(&mut self) {
        self.active_level = self.model.levels.first().map(|level| level.id.clone());
    }

    fn has_level(&self, id: &ElementId) -> bool {
        self.model.levels.iter().any(|level| &level.id == id)
    }

    fn reconcile_active_level(&mut self) {
        if self
            .active_level
            .as_ref()
            .is_some_and(|active| self.has_level(active))
        {
            return;
        }
        self.reset_active_level();
    }

    fn active_level_id(&self) -> ElementId {
        self.active_level
            .as_ref()
            .filter(|active| self.has_level(active))
            .cloned()
            .or_else(|| self.model.levels.first().map(|level| level.id.clone()))
            .unwrap_or_else(|| ElementId::new("level-1"))
    }

    fn active_level_name(&self) -> String {
        let active = self.active_level_id();
        self.model
            .levels
            .iter()
            .find(|level| level.id == active)
            .map(|level| level.name.clone())
            .unwrap_or_else(|| active.0)
    }

    fn set_active_level(&mut self, level: ElementId) {
        if self.has_level(&level) {
            self.active_level = Some(level);
        }
    }

    /// Clears all transient interaction tools. Called whenever the document is
    /// replaced wholesale (new/open/reset), so no in-progress draw, dimension, or
    /// drag gesture carries into a different document.
    fn reset_tools(&mut self) {
        self.command_tab = actions::WorkflowTab::Frame;
        self.dimension_tool = DimensionToolState::default();
        self.draw_wall_tool = DrawWallToolState::default();
        self.room_tool_active = false;
        self.opening_drag = None;
        self.wall_drag = None;
    }

    fn new_project(&mut self) {
        let code = framer_core::FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls.push(Wall::new(
            "wall-1",
            "Untitled wall",
            Length::from_feet(12.0),
            &code,
        ));
        self.model = model;
        self.selected_wall = 0;
        self.selected = Selection::Wall;
        self.project_path = "untitled-alpha.framer".to_owned();
        self.file_status = Some("Created new project".to_owned());
        self.artifact_status = None;
        self.dimension_status = None;
        self.reset_active_level();
        self.reset_tools();
        self.workspace_mode = WorkspaceMode::Design;
        self.history.clear();
        self.reset_2d_cameras();
        self.rebuild();
    }

    fn reset_demo(&mut self) {
        self.model = BuildingModel::demo_shell();
        self.selected_wall = 0;
        self.selected = Selection::Wall;
        self.project_path = DEFAULT_PROJECT_PATH.to_owned();
        self.file_status = Some("Reset to multi-wall demo shell".to_owned());
        self.artifact_status = None;
        self.dimension_status = None;
        self.reset_active_level();
        self.reset_tools();
        self.workspace_mode = WorkspaceMode::Design;
        self.history.clear();
        self.reset_2d_cameras();
        self.rebuild();
    }

    fn reset_wall_demo(&mut self) {
        self.model = BuildingModel::demo_wall();
        self.selected_wall = 0;
        self.selected = Selection::Wall;
        self.project_path = "examples/projects/demo-wall.framer".to_owned();
        self.file_status = Some("Reset to Phase 1 demo wall".to_owned());
        self.artifact_status = None;
        self.dimension_status = None;
        self.reset_active_level();
        self.reset_tools();
        self.workspace_mode = WorkspaceMode::Design;
        self.history.clear();
        self.reset_2d_cameras();
        self.rebuild();
    }

    fn save_project_file(&mut self) {
        let path = PathBuf::from(self.project_path.trim());
        if path.as_os_str().is_empty() {
            self.file_status = Some("Choose a project path before saving".to_owned());
            return;
        }

        let result = save_project_document(&self.model)
            .map_err(|error| error.to_string())
            .and_then(|document| write_text_file(&path, document));

        self.file_status = Some(match result {
            Ok(()) => format!("Saved {}", path.display()),
            Err(error) => format!("Save failed: {error}"),
        });
    }

    fn load_project_file(&mut self) {
        let path = PathBuf::from(self.project_path.trim());
        if path.as_os_str().is_empty() {
            self.file_status = Some("Choose a project path before opening".to_owned());
            return;
        }

        let result = fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|source| load_project_document(&source).map_err(|error| error.to_string()));

        match result {
            Ok(model) => {
                self.model = model;
                self.selected_wall = 0;
                self.selected = Selection::Wall;
                self.workspace_mode = WorkspaceMode::Design;
                self.reset_active_level();
                self.history.clear();
                self.reset_2d_cameras();
                self.rebuild();
                self.file_status = Some(format!("Opened {}", path.display()));
                self.artifact_status = None;
                self.dimension_status = None;
                self.reset_tools();
            }
            Err(error) => {
                self.file_status = Some(format!("Open failed: {error}"));
            }
        }
    }

    fn export_current_artifacts(&mut self) {
        let Some(plan) = &self.project_plan else {
            self.artifact_status =
                Some("Export failed: regenerate a valid framing plan first".to_owned());
            return;
        };

        let (svg_path, csv_path) = export_paths(&self.project_path);
        let svg = export_project_svg(&self.model, plan);
        let csv = export_bom_csv(&plan.bom(), &plan.fasteners);

        let result = write_text_file(&svg_path, svg).and_then(|()| write_text_file(&csv_path, csv));
        self.artifact_status = Some(match result {
            Ok(()) => format!("Exported {} and {}", svg_path.display(), csv_path.display()),
            Err(error) => format!("Export failed: {error}"),
        });
    }

    fn export_compliance_report(&mut self) {
        let Some(report) = &self.compliance_report else {
            self.artifact_status =
                Some("Export failed: regenerate a valid compliance report first".to_owned());
            return;
        };

        let csv_path = compliance_report_path(&self.project_path);
        self.artifact_status = Some(match write_text_file(&csv_path, report.to_csv()) {
            Ok(()) => format!("Exported {}", csv_path.display()),
            Err(error) => format!("Export failed: {error}"),
        });
    }

    fn focus_compliance_source(&mut self, id: ElementId) {
        self.file_status = Some(if self.select_model_element(&id) {
            format!("Selected compliance source {}", id.0)
        } else {
            format!("No selectable compliance source for {}", id.0)
        });
    }

    fn select_model_element(&mut self, id: &ElementId) -> bool {
        if let Some((index, level)) = self
            .model
            .walls
            .iter()
            .enumerate()
            .find_map(|(index, wall)| (&wall.id == id).then(|| (index, wall.level.clone())))
        {
            self.selected_wall = index;
            self.set_active_level(level);
            self.selected = Selection::Wall;
            return true;
        }

        for (index, wall) in self.model.walls.iter().enumerate() {
            if wall.openings.iter().any(|opening| &opening.id == id) {
                self.selected_wall = index;
                self.set_active_level(wall.level.clone());
                self.selected = Selection::Opening(id.0.clone());
                return true;
            }
            if wall.dimensions.iter().any(|dimension| &dimension.id == id) {
                self.selected_wall = index;
                self.set_active_level(wall.level.clone());
                self.selected = Selection::Dimension(id.0.clone());
                return true;
            }
            if wall.bracing.iter().any(|panel| &panel.id == id) {
                self.selected_wall = index;
                self.set_active_level(wall.level.clone());
                self.selected = Selection::Wall;
                return true;
            }
        }

        if let Some(level) = self
            .model
            .levels
            .iter()
            .find_map(|level| (&level.id == id).then(|| level.id.clone()))
        {
            self.set_active_level(level.clone());
            self.selected = Selection::Level(level.0);
            return true;
        }
        if let Some(room) = self.model.rooms.iter().find(|room| &room.id == id) {
            self.set_active_level(room.level.clone());
            self.selected = Selection::Room(id.0.clone());
            return true;
        }
        if let Some(plane) = self.model.roof_planes.iter().find(|plane| &plane.id == id) {
            self.set_active_level(plane.level.clone());
            self.selected = Selection::RoofPlane(id.0.clone());
            return true;
        }
        if let Some(ceiling) = self.model.ceilings.iter().find(|ceiling| &ceiling.id == id) {
            self.set_active_level(ceiling.level.clone());
            self.selected = Selection::Ceiling(id.0.clone());
            return true;
        }
        if let Some(deck) = self.model.floor_decks.iter().find(|deck| &deck.id == id) {
            self.set_active_level(deck.level.clone());
            self.selected = Selection::FloorDeck(id.0.clone());
            return true;
        }
        if self.model.systems.iter().any(|system| &system.id == id) {
            self.selected = Selection::System(id.0.clone());
            return true;
        }
        if self
            .model
            .materials
            .iter()
            .any(|material| &material.id == id)
        {
            self.selected = Selection::Material(id.0.clone());
            return true;
        }
        if self
            .model
            .furnishings
            .iter()
            .any(|furnishing| &furnishing.id == id)
        {
            self.selected = Selection::Furnishing(id.0.clone());
            return true;
        }
        if self.model.mep_objects.iter().any(|object| &object.id == id) {
            self.selected = Selection::MepObject(id.0.clone());
            return true;
        }
        if self.model.standards_packs.iter().any(|pack| &pack.id == id) {
            self.selected = Selection::StandardsPack(id.0.clone());
            return true;
        }
        if let Some(instance) = self
            .model
            .furnishing_instances
            .iter()
            .find(|instance| &instance.id == id)
        {
            self.set_active_level(instance.level.clone());
            self.selected = Selection::FurnishingInstance(id.0.clone());
            return true;
        }
        if let Some(instance) = self
            .model
            .mep_instances
            .iter()
            .find(|instance| &instance.id == id)
        {
            self.set_active_level(instance.level.clone());
            self.selected = Selection::MepInstance(id.0.clone());
            return true;
        }
        if let Some(level) = self
            .model
            .braced_wall_lines
            .iter()
            .find_map(|line| (&line.id == id).then(|| line.level.clone()))
        {
            self.set_active_level(level.clone());
            self.selected = Selection::Level(level.0);
            return true;
        }

        false
    }

    fn set_workspace_mode(&mut self, mode: WorkspaceMode) {
        match mode {
            WorkspaceMode::Plan => self.command_tab = actions::WorkflowTab::Plan,
            WorkspaceMode::Design if self.command_tab == actions::WorkflowTab::Plan => {
                self.command_tab = actions::WorkflowTab::Frame;
            }
            WorkspaceMode::Design => {}
        }

        if self.workspace_mode == mode {
            return;
        }
        self.workspace_mode = mode;
        self.opening_drag = None;
        match mode {
            WorkspaceMode::Design => self.select_authored_for_design_mode(),
            WorkspaceMode::Plan => {
                self.dimension_tool.active = false;
                self.dimension_tool.clear_picks();
                self.rebuild();
            }
        }
    }

    fn open_command_search(&mut self) {
        self.command_search.open = true;
        self.command_search.focus_input = true;
        self.command_search.query.clear();
    }

    fn is_selection_deletable(&self) -> bool {
        matches!(
            self.selected,
            Selection::Opening(_)
                | Selection::Wall
                | Selection::Room(_)
                | Selection::RoofPlane(_)
                | Selection::Ceiling(_)
                | Selection::FloorDeck(_)
                | Selection::System(_)
                | Selection::Material(_)
                | Selection::Furnishing(_)
                | Selection::MepObject(_)
                | Selection::StandardsPack(_)
                | Selection::FurnishingInstance(_)
                | Selection::MepInstance(_)
        )
    }

    fn action_enabled(&self, id: actions::ActionId) -> bool {
        match id {
            actions::ActionId::Undo => self.history.can_undo(),
            actions::ActionId::Redo => self.history.can_redo(),
            actions::ActionId::ExportArtifacts => self.workspace_mode.shows_generated_plan(),
            actions::ActionId::ExportComplianceReport => {
                self.workspace_mode.shows_generated_plan() && self.compliance_report.is_some()
            }
            actions::ActionId::DeleteSelection => self.is_selection_deletable(),
            actions::ActionId::AddDoor
            | actions::ActionId::AddWindow
            | actions::ActionId::AddGarageDoor
            | actions::ActionId::AddGableRoof
            | actions::ActionId::AddShedRoof
            | actions::ActionId::AddHipRoof => self.workspace_mode.allows_design_edits(),
            actions::ActionId::CommandSearch
            | actions::ActionId::NewProject
            | actions::ActionId::OpenProject
            | actions::ActionId::SaveProject
            | actions::ActionId::LoadShellDemo
            | actions::ActionId::LoadWallDemo
            | actions::ActionId::WorkspaceDesign
            | actions::ActionId::WorkspacePlan
            | actions::ActionId::ViewPlan
            | actions::ActionId::ViewElevation
            | actions::ActionId::ViewRoof
            | actions::ActionId::View3d
            | actions::ActionId::ViewRender
            | actions::ActionId::ToolWall
            | actions::ActionId::ToolRoom
            | actions::ActionId::ToolCeiling
            | actions::ActionId::ToolVault
            | actions::ActionId::ToolFloor
            | actions::ActionId::ToolDimensionLinear
            | actions::ActionId::DimensionKind
            | actions::ActionId::DimensionAxis
            | actions::ActionId::ToggleSection => true,
        }
    }

    fn execute_action(&mut self, id: actions::ActionId) {
        if !self.action_enabled(id) {
            return;
        }

        match id {
            actions::ActionId::CommandSearch => self.open_command_search(),
            actions::ActionId::NewProject => self.new_project(),
            actions::ActionId::OpenProject => self.load_project_file(),
            actions::ActionId::SaveProject => self.save_project_file(),
            actions::ActionId::ExportArtifacts => self.export_current_artifacts(),
            actions::ActionId::ExportComplianceReport => self.export_compliance_report(),
            actions::ActionId::Undo => self.undo(),
            actions::ActionId::Redo => self.redo(),
            actions::ActionId::LoadShellDemo => self.reset_demo(),
            actions::ActionId::LoadWallDemo => self.reset_wall_demo(),
            actions::ActionId::WorkspaceDesign => self.set_workspace_mode(WorkspaceMode::Design),
            actions::ActionId::WorkspacePlan => self.set_workspace_mode(WorkspaceMode::Plan),
            actions::ActionId::ViewPlan => self.viewport_mode = ViewportMode::Plan,
            actions::ActionId::ViewElevation => self.viewport_mode = ViewportMode::Elevation,
            actions::ActionId::ViewRoof => self.viewport_mode = ViewportMode::RoofPlan,
            actions::ActionId::View3d => self.viewport_mode = ViewportMode::Axonometric,
            actions::ActionId::ViewRender => self.viewport_mode = ViewportMode::Render,
            actions::ActionId::ToolWall => self.toggle_draw_wall_tool(),
            actions::ActionId::ToolRoom => self.toggle_room_tool(),
            actions::ActionId::ToolCeiling => self.toggle_ceiling_tool(),
            actions::ActionId::ToolVault => self.toggle_vault_tool(),
            actions::ActionId::ToolFloor => self.toggle_floor_tool(),
            actions::ActionId::DeleteSelection => self.delete_selected(),
            actions::ActionId::AddDoor => self.add_opening(OpeningKind::Door),
            actions::ActionId::AddWindow => self.add_opening(OpeningKind::Window),
            actions::ActionId::AddGarageDoor => self.add_opening(OpeningKind::GarageDoor),
            actions::ActionId::AddGableRoof => self.add_roof(RoofForm::Gable),
            actions::ActionId::AddShedRoof => self.add_roof(RoofForm::Shed),
            actions::ActionId::AddHipRoof => self.add_roof(RoofForm::Hip),
            actions::ActionId::ToolDimensionLinear => self.activate_dimension_tool(),
            actions::ActionId::DimensionKind | actions::ActionId::DimensionAxis => {
                self.activate_dimension_tool();
            }
            actions::ActionId::ToggleSection => self.show_section = !self.show_section,
        }
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        let command_search_pressed =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::K));
        if command_search_pressed {
            self.open_command_search();
            return;
        }

        if self.command_search.open {
            return;
        }

        if ctx.text_edit_focused() {
            return;
        }

        let (
            escape_pressed,
            dimension_pressed,
            draw_wall_pressed,
            room_pressed,
            ceiling_pressed,
            floor_pressed,
            vault_pressed,
            delete_pressed,
            redo_pressed,
            undo_pressed,
        ) = ctx.input_mut(|input| {
            let escape = input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            let dimension = input.consume_key(egui::Modifiers::NONE, egui::Key::D);
            let draw_wall = input.consume_key(egui::Modifiers::NONE, egui::Key::W);
            let room = input.consume_key(egui::Modifiers::NONE, egui::Key::R);
            let ceiling = input.consume_key(egui::Modifiers::NONE, egui::Key::C);
            let floor = input.consume_key(egui::Modifiers::NONE, egui::Key::F);
            let vault = input.consume_key(egui::Modifiers::NONE, egui::Key::V);
            let delete = input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
            // Redo MUST be consumed before undo: egui's consume_key matches
            // modifiers *logically* (a pattern without Shift still matches a
            // Shift-held event), so Cmd+Z would otherwise swallow Cmd+Shift+Z.
            // Redo: Cmd/Ctrl+Shift+Z, or Ctrl+Y.
            let redo = input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Z,
            ) || input.consume_key(egui::Modifiers::CTRL, egui::Key::Y);
            // Undo: Cmd+Z on macOS, Ctrl+Z elsewhere (COMMAND is platform-aware).
            let undo = input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z);
            (
                escape, dimension, draw_wall, room, ceiling, floor, vault, delete, redo, undo,
            )
        });

        if undo_pressed {
            self.undo();
        } else if redo_pressed {
            self.redo();
        } else if escape_pressed {
            self.exit_current_context();
        } else if delete_pressed {
            self.delete_selected();
        } else if dimension_pressed {
            self.activate_dimension_tool();
        } else if draw_wall_pressed {
            self.toggle_draw_wall_tool();
        } else if room_pressed {
            self.toggle_room_tool();
        } else if ceiling_pressed {
            self.toggle_ceiling_tool();
        } else if floor_pressed {
            self.toggle_floor_tool();
        } else if vault_pressed {
            self.toggle_vault_tool();
        }
    }

    /// Delete whatever authored element is selected (wall, opening, room,
    /// construction system, material, object family, or placed object).
    fn delete_selected(&mut self) {
        match &self.selected {
            Selection::Opening(_) => self.delete_selected_opening(),
            Selection::Wall => self.delete_selected_wall(),
            Selection::Room(_) => self.delete_selected_room(),
            Selection::RoofPlane(_) => self.delete_selected_roof_plane(),
            Selection::Ceiling(_) => self.delete_selected_ceiling(),
            Selection::FloorDeck(_) => self.delete_selected_floor_deck(),
            Selection::System(_) => self.delete_selected_system(),
            Selection::Material(_) => self.delete_selected_material(),
            Selection::Furnishing(_) => self.delete_selected_furnishing(),
            Selection::MepObject(_) => self.delete_selected_mep_object(),
            Selection::StandardsPack(_) => self.delete_selected_standards_pack(),
            Selection::FurnishingInstance(_) => self.delete_selected_furnishing_instance(),
            Selection::MepInstance(_) => self.delete_selected_mep_instance(),
            _ => {}
        }
    }

    /// Delete the selected construction system as one undo step, refusing when any
    /// wall, roof plane, ceiling, or floor deck still references it (deleting it
    /// would dangle that object's `system`).
    fn delete_selected_system(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::System(id) = self.selected.clone() else {
            return;
        };
        let referenced = self.model.walls.iter().any(|wall| wall.system.0 == id)
            || self
                .model
                .roof_planes
                .iter()
                .any(|plane| plane.system.0 == id)
            || self
                .model
                .ceilings
                .iter()
                .any(|ceiling| ceiling.system.0 == id)
            || self
                .model
                .floor_decks
                .iter()
                .any(|deck| deck.system.0 == id);
        if referenced {
            self.error = Some(format!(
                "Cannot delete system '{id}': it is still applied to one or more \
                 walls, roofs, ceilings, or floors"
            ));
            return;
        }
        self.edit("Delete system", |app| {
            let before = app.model.systems.len();
            app.model.systems.retain(|system| system.id.0 != id);
            if app.model.systems.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    /// Delete the selected material as one undo step, refusing when any layer or
    /// framing cavity still references it (deleting it would dangle the reference).
    fn delete_selected_material(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::Material(id) = self.selected.clone() else {
            return;
        };
        let referenced = self.model.systems.iter().any(|system| {
            system.layers.iter().any(|layer| {
                layer.material.0 == id
                    || layer
                        .framing
                        .as_ref()
                        .and_then(|framing| framing.cavity_material.as_ref())
                        .is_some_and(|cavity| cavity.0 == id)
            })
        });
        if referenced {
            self.error = Some(format!(
                "Cannot delete material '{id}': it is still used by one or more layers"
            ));
            return;
        }
        self.edit("Delete material", |app| {
            let before = app.model.materials.len();
            app.model.materials.retain(|material| material.id.0 != id);
            if app.model.materials.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    fn delete_selected_furnishing(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::Furnishing(id) = self.selected.clone() else {
            return;
        };
        if self
            .model
            .furnishing_instances
            .iter()
            .any(|instance| instance.family.0 == id)
        {
            self.error = Some(format!(
                "Cannot delete furnishing '{id}': it is still placed in the model"
            ));
            return;
        }
        self.edit("Delete furnishing", |app| {
            let before = app.model.furnishings.len();
            app.model
                .furnishings
                .retain(|furnishing| furnishing.id.0 != id);
            if app.model.furnishings.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    fn delete_selected_mep_object(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::MepObject(id) = self.selected.clone() else {
            return;
        };
        if self
            .model
            .mep_instances
            .iter()
            .any(|instance| instance.family.0 == id)
        {
            self.error = Some(format!(
                "Cannot delete MEP object '{id}': it is still placed in the model"
            ));
            return;
        }
        self.edit("Delete MEP object", |app| {
            let before = app.model.mep_objects.len();
            app.model.mep_objects.retain(|object| object.id.0 != id);
            if app.model.mep_objects.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    fn delete_selected_standards_pack(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::StandardsPack(id) = self.selected.clone() else {
            return;
        };
        if self.model.standards.iter().any(|pack| pack.0 == id) {
            self.error = Some(format!(
                "Cannot delete standards pack '{id}': remove it from the stack first"
            ));
            return;
        }
        self.edit("Delete standards pack", |app| {
            let before = app.model.standards_packs.len();
            app.model.standards_packs.retain(|pack| pack.id.0 != id);
            if app.model.standards_packs.len() != before {
                app.selected = Selection::Site;
            }
        });
    }

    fn add_project_standards_pack(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        self.edit("Add standards pack", |app| {
            let (id, index) = next_standards_pack_id(&app.model);
            let pack = project_local_standards_pack(&app.model, id.clone(), index);
            app.model.standards_packs.push(pack);
            app.model.standards.push(ElementId::new(id.clone()));
            app.model.sort_deterministically();
            app.selected = Selection::StandardsPack(id);
        });
    }

    fn insert_starter_standards_pack(&mut self, id: String) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let loaded = match framer_library::starter_library() {
            Ok(loaded) => loaded,
            Err(error) => {
                self.file_status = Some(format!("Library import failed: {error}"));
                return;
            }
        };

        let mut result = None;
        self.edit("Insert library standards pack", |app| {
            let imported = framer_library::import_standards_pack(
                &mut app.model,
                &loaded.library,
                &loaded.content_hash,
                &ElementId::new(&id),
            );
            if let Ok(imported) = &imported
                && let Some(pack) = &imported.standards_pack
            {
                if !app.model.standards.iter().any(|id| id == pack) {
                    app.model.standards.push(pack.clone());
                }
                app.selected = Selection::StandardsPack(pack.0.clone());
            }
            result = Some(imported);
        });
        self.file_status = Some(match result.expect("import closure should run") {
            Ok(imported) => {
                let id = imported
                    .standards_pack
                    .map(|id| id.0)
                    .unwrap_or_else(|| "standards pack".to_owned());
                format!("Inserted {id} from starter library")
            }
            Err(error) => format!("Library import failed: {error}"),
        });
    }

    fn move_standards_pack_in_stack(&mut self, id: String, dir: isize) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        self.edit("Reorder standards stack", |app| {
            let Some(index) = app.model.standards.iter().position(|pack| pack.0 == id) else {
                return;
            };
            let new_index = if dir < 0 {
                index.checked_sub(1)
            } else {
                (index + 1 < app.model.standards.len()).then_some(index + 1)
            };
            if let Some(new_index) = new_index {
                app.model.standards.swap(index, new_index);
                app.selected = Selection::StandardsPack(id);
            }
        });
    }

    fn remove_standards_pack_from_stack(&mut self, id: String) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        self.edit("Remove standards pack from stack", |app| {
            let before = app.model.standards.len();
            app.model.standards.retain(|pack| pack.0 != id);
            if app.model.standards.len() != before {
                app.selected = Selection::StandardsPack(id);
            }
        });
    }

    fn add_standards_pack_to_stack(&mut self, id: String) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        self.edit("Add standards pack to stack", |app| {
            let pack_id = ElementId::new(id.clone());
            if app
                .model
                .standards
                .iter()
                .any(|existing| existing == &pack_id)
                || !app
                    .model
                    .standards_packs
                    .iter()
                    .any(|pack| pack.id == pack_id)
            {
                return;
            }
            app.model.standards.push(pack_id);
            app.selected = Selection::StandardsPack(id);
        });
    }

    fn waive_standards_rule(&mut self, target: String, reason: String) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let reason = reason.trim().to_owned();
        if reason.is_empty() {
            return;
        }
        self.edit("Waive standards rule", |app| {
            let pack_id = app.ensure_project_local_standards_pack();
            if let Some(pack) = app
                .model
                .standards_packs
                .iter_mut()
                .find(|pack| pack.id == pack_id)
            {
                if let Some(framer_core::RuleOverlay::Waive {
                    reason: existing, ..
                }) = pack.overlays.iter_mut().find(|overlay| match overlay {
                    framer_core::RuleOverlay::Waive {
                        target: existing, ..
                    } => existing == &target,
                    framer_core::RuleOverlay::Severity { .. } => false,
                }) {
                    *existing = reason;
                } else {
                    pack.overlays
                        .push(framer_core::RuleOverlay::Waive { target, reason });
                }
            }
            app.selected = Selection::StandardsPack(pack_id.0);
        });
    }

    fn ensure_project_local_standards_pack(&mut self) -> ElementId {
        if let Some(id) = self.model.standards.iter().rev().find(|id| {
            is_project_local_standards_pack_id(id)
                && self
                    .model
                    .standards_packs
                    .iter()
                    .any(|pack| pack.id == **id && pack.source.is_none())
        }) {
            return id.clone();
        }

        let (id, index) = next_standards_pack_id(&self.model);
        let pack_id = ElementId::new(id.clone());
        let pack = project_local_standards_pack(&self.model, id, index);
        self.model.standards_packs.push(pack);
        self.model.standards.push(pack_id.clone());
        self.model.sort_deterministically();
        pack_id
    }

    fn delete_selected_furnishing_instance(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::FurnishingInstance(id) = self.selected.clone() else {
            return;
        };
        self.edit("Delete furnishing instance", |app| {
            let before = app.model.furnishing_instances.len();
            app.model
                .furnishing_instances
                .retain(|instance| instance.id.0 != id);
            if app.model.furnishing_instances.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    fn delete_selected_mep_instance(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::MepInstance(id) = self.selected.clone() else {
            return;
        };
        self.edit("Delete MEP instance", |app| {
            let before = app.model.mep_instances.len();
            app.model
                .mep_instances
                .retain(|instance| instance.id.0 != id);
            if app.model.mep_instances.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    fn activate_dimension_tool(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            self.set_workspace_mode(WorkspaceMode::Design);
        }
        self.command_tab = actions::WorkflowTab::Annotate;

        self.dimension_tool.active = true;
        self.dimension_tool.clear_picks();
        self.draw_wall_tool = DrawWallToolState::default();
        self.room_tool_active = false;
        self.ceiling_tool_active = false;
        self.vault_tool_active = false;
        self.floor_tool_active = false;
        self.opening_drag = None;
        self.dimension_status =
            Some("Pick two anchors, then move the pointer to place the dimension".to_owned());
        self.viewport_mode = ViewportMode::Elevation;
    }

    /// Toggle the draw-wall tool. Activating it switches to the Plan view (where
    /// walls are authored), enters Design mode, and disables the dimension tool.
    fn toggle_draw_wall_tool(&mut self) {
        let activate = !self.draw_wall_tool.active;
        self.draw_wall_tool = DrawWallToolState {
            active: activate,
            start: None,
            previous_snap: None,
        };
        if activate {
            if !self.workspace_mode.allows_design_edits() {
                self.set_workspace_mode(WorkspaceMode::Design);
            }
            self.command_tab = actions::WorkflowTab::Frame;
            self.dimension_tool.active = false;
            self.dimension_tool.clear_picks();
            self.room_tool_active = false;
            self.ceiling_tool_active = false;
            self.vault_tool_active = false;
            self.floor_tool_active = false;
            self.opening_drag = None;
            self.viewport_mode = ViewportMode::Plan;
            self.dimension_status =
                Some("Click to place wall endpoints; right-click or Esc ends the run".to_owned());
        } else {
            self.dimension_status = None;
        }
    }

    /// Commit one draw-wall click. The first click sets the run's start point;
    /// each subsequent click draws a wall from the previous point and continues
    /// the polyline from the new point.
    fn handle_draw_wall_point(&mut self, point: Point2) {
        if !self.draw_wall_tool.active {
            return;
        }
        // Drop the held snap so the committed point's stale guides don't carry
        // into the next segment's first frame.
        self.draw_wall_tool.previous_snap = None;
        match self.draw_wall_tool.start {
            None => self.draw_wall_tool.start = Some(point),
            Some(start) => {
                if start != point {
                    // If committing this segment closes a loop — the count of
                    // enclosed rooms (bounded faces) rises — the user just drew a
                    // full room, so finish the run and leave the tool, the way
                    // Revit and Chief Architect end a closed wall sketch.
                    let level = self.active_level_id();
                    let faces_before =
                        framer_core::enclosed_room_count_on_level(&self.model, &level);
                    self.add_wall(start, point);
                    if framer_core::enclosed_room_count_on_level(&self.model, &level) > faces_before
                    {
                        self.finish_draw_wall_on_enclosure();
                        return;
                    }
                }
                self.draw_wall_tool.start = Some(point);
            }
        }
    }

    /// Leave the draw-wall tool because the last committed segment enclosed a
    /// room. Mirrors the deactivation half of [`Self::toggle_draw_wall_tool`],
    /// but reports the closure in the status bar rather than clearing it — the
    /// status line is the only cue that the tool turned itself off.
    fn finish_draw_wall_on_enclosure(&mut self) {
        self.draw_wall_tool = DrawWallToolState::default();
        self.dimension_status = Some("Room enclosed — draw-wall tool off".to_owned());
    }

    fn exit_current_context(&mut self) {
        if self.draw_wall_tool.active {
            // Esc cancels the current polyline run first, then leaves the tool.
            if self.draw_wall_tool.start.take().is_none() {
                self.draw_wall_tool.active = false;
                self.dimension_status = None;
            }
            return;
        }

        if self.room_tool_active {
            self.room_tool_active = false;
            self.dimension_status = None;
            return;
        }

        if self.opening_drag.is_some() {
            self.opening_drag = None;
            return;
        }

        let dimension_tool_was_active = self.dimension_tool.active;
        let dimension_was_selected = matches!(self.selected, Selection::Dimension(_));
        self.dimension_tool.active = false;
        self.dimension_tool.clear_picks();
        if dimension_was_selected {
            self.dimension_status = None;
            self.selected = Selection::Wall;
            return;
        }
        if dimension_tool_was_active {
            self.dimension_status = None;
            return;
        }

        match &self.selected {
            Selection::Wall => {}
            Selection::Member { wall_id, .. } => {
                if let Some(index) = self
                    .model
                    .walls
                    .iter()
                    .position(|wall| wall.id.0 == *wall_id)
                {
                    self.selected_wall = index;
                }
                self.selected = Selection::Wall;
            }
            Selection::Level(_)
            | Selection::Opening(_)
            | Selection::Join(_)
            | Selection::Room(_)
            | Selection::RoofPlane(_)
            | Selection::Ceiling(_)
            | Selection::FloorDeck(_)
            | Selection::System(_)
            | Selection::Material(_)
            | Selection::Furnishing(_)
            | Selection::MepObject(_)
            | Selection::StandardsPack(_)
            | Selection::FurnishingInstance(_)
            | Selection::MepInstance(_) => {
                self.selected = Selection::Wall;
            }
            Selection::Site => {}
            Selection::Dimension(_) => unreachable!("dimension selections exit above"),
        }
    }

    fn select_authored_for_design_mode(&mut self) {
        if let Selection::Member { wall_id, .. } = &self.selected {
            if let Some(index) = self
                .model
                .walls
                .iter()
                .position(|wall| wall.id.0 == *wall_id)
            {
                self.selected_wall = index;
            }
            self.selected = Selection::Wall;
        }
    }

    /// Add a wall between two authored endpoints as one undo step, auto-creating
    /// corner joins to any existing walls that share an endpoint. Endpoints are
    /// expected to be ortho-snapped by the draw tool; a zero-length segment is a
    /// no-op.
    fn add_wall(&mut self, start: Point2, end: Point2) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        if start == end {
            return;
        }
        // Walls must be axis-aligned (Slice 1). Reject a diagonal segment up front
        // so an invalid model never enters the document or undo history.
        if start.x != end.x && start.y != end.y {
            return;
        }
        self.edit("Draw wall", |app| {
            let (id, index) = next_wall_id(&app.model);
            let level = app.active_level_id().0;
            let wall = Wall::new(
                id,
                format!("Wall {index}"),
                Length::from_feet(1.0),
                &app.model.framing_defaults(),
            )
            .with_placement(level, start, end);
            let joins = joins_for_new_wall(&app.model, &wall);
            app.model.walls.push(wall);
            app.model.wall_joins.extend(joins);
            app.selected_wall = app.model.walls.len() - 1;
            app.selected = Selection::Wall;
        });
    }

    /// Delete the selected wall and every join referencing it as one undo step.
    fn delete_selected_wall(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        if !matches!(self.selected, Selection::Wall) {
            return;
        }
        let Some(wall_id) = self
            .model
            .walls
            .get(self.selected_wall)
            .map(|wall| wall.id.clone())
        else {
            return;
        };
        self.edit("Delete wall", |app| {
            if app.model.remove_wall(&wall_id) {
                app.selected_wall = 0;
                app.selected = Selection::Wall;
            }
        });
    }

    /// Add a room with the given seed point as one undo step. The seed locates the
    /// bounding wall loop; enclosure is checked by the caller (the room tool).
    fn add_room(&mut self, seed: Point2) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        self.edit("Add room", |app| {
            let (id, index) = next_room_id(&app.model);
            let level = app.active_level_id().0;
            let room = Room::new(
                id.clone(),
                format!("Room {index}"),
                RoomUsage::Unspecified,
                level,
                seed,
            );
            app.model.rooms.push(room);
            app.selected = Selection::Room(id);
        });
    }

    /// Delete the selected room as one undo step.
    fn delete_selected_room(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::Room(id) = self.selected.clone() else {
            return;
        };
        self.edit("Delete room", |app| {
            let before = app.model.rooms.len();
            app.model.rooms.retain(|room| room.id.0 != id);
            if app.model.rooms.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    /// Delete the selected roof plane as one undo step.
    fn delete_selected_roof_plane(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::RoofPlane(id) = self.selected.clone() else {
            return;
        };
        self.edit("Delete roof plane", |app| {
            let before = app.model.roof_planes.len();
            app.model.roof_planes.retain(|plane| plane.id.0 != id);
            if app.model.roof_planes.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    /// Delete the selected ceiling as one undo step.
    fn delete_selected_ceiling(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::Ceiling(id) = self.selected.clone() else {
            return;
        };
        self.edit("Delete ceiling", |app| {
            let before = app.model.ceilings.len();
            app.model.ceilings.retain(|ceiling| ceiling.id.0 != id);
            if app.model.ceilings.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    /// Delete the selected floor deck as one undo step.
    fn delete_selected_floor_deck(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::FloorDeck(id) = self.selected.clone() else {
            return;
        };
        self.edit("Delete floor deck", |app| {
            let before = app.model.floor_decks.len();
            app.model.floor_decks.retain(|deck| deck.id.0 != id);
            if app.model.floor_decks.len() != before {
                app.selected = Selection::Wall;
            }
        });
    }

    /// Add a new wall-kind construction system as one undo step, seeded with a
    /// minimal valid stack so it passes validation immediately and renders
    /// sensibly. `exterior` adds an outboard cladding layer (so the derived
    /// exposure is `Exterior`); otherwise it is a drywall/framing/drywall
    /// partition. Selects the new system.
    fn add_wall_system(&mut self, exterior: bool) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        use framer_core::{
            BoardProfile, ConstructionLayer, ConstructionSystem, ElementId, FramingPattern,
            FramingSpec, LayerFunction, SystemKind,
        };
        self.edit("Add system", |app| {
            let (id, index) = next_system_id(&app.model);
            // Resolve seed material ids, falling back to the library when present.
            let finish = app.first_material_with_tag("finish", "mat-drywall");
            let framing = app.first_material_with_tag("framing", "mat-spf");
            let cladding = app.first_material_with_tag("cladding", "mat-fiber-cement");

            let mut layers = vec![
                ConstructionLayer::new(
                    LayerFunction::InteriorFinish,
                    finish.clone(),
                    Length::from_inches(0.625),
                ),
                ConstructionLayer::new(
                    LayerFunction::Framing,
                    framing,
                    BoardProfile::TwoByFour.nominal_depth(),
                )
                .with_framing(FramingSpec {
                    member: BoardProfile::TwoByFour,
                    spacing: Length::from_whole_inches(16),
                    pattern: FramingPattern::Single,
                    member_family: framer_core::MemberFamily::Stud,
                    cavity_material: None,
                }),
            ];
            if exterior {
                layers.push(ConstructionLayer::new(
                    LayerFunction::Cladding,
                    cladding,
                    Length::from_inches(0.3125),
                ));
            } else {
                layers.push(ConstructionLayer::new(
                    LayerFunction::InteriorFinish,
                    finish,
                    Length::from_inches(0.625),
                ));
            }

            let name = if exterior {
                format!("Exterior wall {index}")
            } else {
                format!("Interior partition {index}")
            };
            app.model.systems.push(ConstructionSystem {
                id: ElementId::new(id.clone()),
                name,
                kind: SystemKind::Wall,
                source: None,
                layers,
            });
            app.selected = Selection::System(id);
        });
    }

    /// Add a new roof/floor/ceiling construction system as one undo step, seeded
    /// with a minimal valid stack. Selects the new system. Walls keep their
    /// dedicated [`add_wall_system`] (which adds the exterior/interior cladding
    /// choice).
    fn add_surface_system(&mut self, kind: framer_core::SystemKind) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        self.edit("Add system", |app| {
            let system = app.default_surface_system(kind);
            let id = system.id.0.clone();
            app.model.systems.push(system);
            app.selected = Selection::System(id);
        });
    }

    /// Build a minimal valid roof/floor/ceiling construction system: exactly one
    /// framing layer (member sized and named for its family) plus a finish/skin
    /// layer, ordered conditioned-side -> weather-side like a wall stack. Materials
    /// resolve from the project library by tag, falling back when absent. Does not
    /// mutate the model; callers push it and wrap the change in an `edit`.
    fn default_surface_system(
        &self,
        kind: framer_core::SystemKind,
    ) -> framer_core::ConstructionSystem {
        use framer_core::{
            BoardProfile, ConstructionLayer, ConstructionSystem, ElementId, FramingPattern,
            FramingSpec, LayerFunction, MemberFamily, SystemKind,
        };
        let (id, index) = next_system_id(&self.model);
        let framing_material = self.first_material_with_tag("framing", "mat-spf");
        let sheathing_material = self.first_material_with_tag("sheathing", "mat-plywood");
        let finish_material = self.first_material_with_tag("finish", "mat-drywall");

        let (member, family, name) = match kind {
            SystemKind::Roof => (BoardProfile::TwoByEight, MemberFamily::Rafter, "Roof"),
            SystemKind::Floor => (BoardProfile::TwoByTen, MemberFamily::FloorJoist, "Floor"),
            SystemKind::Ceiling => (
                BoardProfile::TwoBySix,
                MemberFamily::CeilingJoist,
                "Ceiling",
            ),
            SystemKind::Wall => (BoardProfile::TwoByFour, MemberFamily::Stud, "Wall"),
        };
        let framing_layer = ConstructionLayer::new(
            LayerFunction::Framing,
            framing_material,
            member.nominal_depth(),
        )
        .with_framing(FramingSpec {
            member,
            spacing: Length::from_whole_inches(16),
            pattern: FramingPattern::Single,
            member_family: family,
            cavity_material: None,
        });
        let layers = match kind {
            // Roof: rafters under the deck, then the weather skin.
            SystemKind::Roof => vec![
                framing_layer,
                ConstructionLayer::new(
                    LayerFunction::Sheathing,
                    sheathing_material,
                    Length::from_inches(0.5),
                ),
                ConstructionLayer::new(
                    LayerFunction::Roofing,
                    self.first_material_with_tag("roofing", "mat-asphalt-shingle"),
                    Length::from_inches(0.25),
                ),
            ],
            // Floor: subfloor deck over the joists.
            SystemKind::Floor => vec![
                ConstructionLayer::new(
                    LayerFunction::Sheathing,
                    sheathing_material,
                    Length::from_inches(0.75),
                ),
                framing_layer,
            ],
            // Ceiling (and the unreachable Wall fallback): finished underside below
            // the joists.
            SystemKind::Ceiling | SystemKind::Wall => vec![
                ConstructionLayer::new(
                    LayerFunction::CeilingFinish,
                    finish_material,
                    Length::from_inches(0.625),
                ),
                framing_layer,
            ],
        };

        ConstructionSystem {
            id: ElementId::new(id),
            name: format!("{name} {index}"),
            kind,
            source: None,
            layers,
        }
    }

    /// The id of an existing system of `kind`, or — when the model has none yet —
    /// a freshly built default one pushed into the model (so the first ceiling or
    /// floor placed on a system-less project still validates and frames). Call
    /// inside an `edit` closure; the pushed system rides that same undo step.
    fn ensure_surface_system(&mut self, kind: framer_core::SystemKind) -> String {
        if let Some(id) = self
            .model
            .systems
            .iter()
            .find(|system| system.kind == kind)
            .map(|system| system.id.0.clone())
        {
            return id;
        }
        let system = self.default_surface_system(kind);
        let id = system.id.0.clone();
        self.model.systems.push(system);
        id
    }

    /// The id of the first material carrying `tag`, or `fallback` if none does
    /// (e.g. an empty library). Used to seed new systems with sensible materials.
    fn first_material_with_tag(&self, tag: &str, fallback: &str) -> String {
        self.model
            .materials
            .iter()
            .find(|material| material.tags.iter().any(|t| t == tag))
            .map(|material| material.id.0.clone())
            .unwrap_or_else(|| {
                self.model
                    .materials
                    .first()
                    .map(|material| material.id.0.clone())
                    .unwrap_or_else(|| fallback.to_owned())
            })
    }

    /// Add a new project material as one undo step, with a neutral grey solid
    /// color and no extra properties. Selects the new material.
    fn add_material(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        self.edit("Add material", |app| {
            let (id, index) = next_material_id(&app.model);
            app.model.materials.push(framer_core::Material::solid_color(
                id.clone(),
                format!("Material {index}"),
                [190, 190, 190],
            ));
            app.selected = Selection::Material(id);
        });
    }

    /// Append a new layer to the system with id `system_id` as one undo step. The
    /// layer is a non-framing `Other` 1" stub referencing the first material, kept
    /// minimal and valid; the system inspector edits it inline afterwards.
    fn add_layer(&mut self, system_id: &str) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let system_id = system_id.to_owned();
        self.edit("Add layer", |app| {
            let material = app
                .model
                .materials
                .first()
                .map(|material| material.id.0.clone())
                .unwrap_or_else(|| "mat-drywall".to_owned());
            if let Some(system) = app
                .model
                .systems
                .iter_mut()
                .find(|system| system.id.0 == system_id)
            {
                system.layers.push(framer_core::ConstructionLayer::new(
                    framer_core::LayerFunction::Other,
                    material,
                    Length::from_whole_inches(1),
                ));
            }
        });
    }

    /// Reorder a layer within its system by swapping it with its neighbour as one
    /// undo step. `index` is the layer's current position; `dir` is -1 (up /
    /// toward interior) or +1 (down / toward exterior). Out-of-range moves no-op.
    fn move_layer(&mut self, system_id: &str, index: usize, dir: isize) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let system_id = system_id.to_owned();
        self.edit("Reorder layer", |app| {
            if let Some(system) = app
                .model
                .systems
                .iter_mut()
                .find(|system| system.id.0 == system_id)
            {
                let Some(target) = index.checked_add_signed(dir) else {
                    return;
                };
                if target < system.layers.len() {
                    system.layers.swap(index, target);
                }
            }
        });
    }

    /// Remove the layer at `index` from the system with id `system_id` as one undo
    /// step. The last remaining layer is kept (an empty system fails validation).
    fn remove_layer(&mut self, system_id: &str, index: usize) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let system_id = system_id.to_owned();
        self.edit("Remove layer", |app| {
            if let Some(system) = app
                .model
                .systems
                .iter_mut()
                .find(|system| system.id.0 == system_id)
                && system.layers.len() > 1
                && index < system.layers.len()
            {
                system.layers.remove(index);
            }
        });
    }

    /// Toggle the room tool. Activating it switches to the Plan view, enters
    /// Design mode, and disables the other tools.
    fn toggle_room_tool(&mut self) {
        self.room_tool_active = !self.room_tool_active;
        if self.room_tool_active {
            if !self.workspace_mode.allows_design_edits() {
                self.set_workspace_mode(WorkspaceMode::Design);
            }
            self.command_tab = actions::WorkflowTab::Design;
            self.dimension_tool.active = false;
            self.dimension_tool.clear_picks();
            self.draw_wall_tool = DrawWallToolState::default();
            self.ceiling_tool_active = false;
            self.vault_tool_active = false;
            self.floor_tool_active = false;
            self.opening_drag = None;
            self.viewport_mode = ViewportMode::Plan;
            self.dimension_status =
                Some("Click inside an enclosed area to place a room".to_owned());
        } else {
            self.dimension_status = None;
        }
    }

    /// Toggle the flat-ceiling tool. Like the room tool, it is region-gated: a
    /// click inside a closed wall loop drops a ceiling over that region.
    fn toggle_ceiling_tool(&mut self) {
        let activate = !self.ceiling_tool_active;
        self.deactivate_placement_tools();
        self.ceiling_tool_active = activate;
        if activate {
            if !self.workspace_mode.allows_design_edits() {
                self.set_workspace_mode(WorkspaceMode::Design);
            }
            self.command_tab = actions::WorkflowTab::Frame;
            self.viewport_mode = ViewportMode::Plan;
            self.dimension_status =
                Some("Click inside an enclosed area to place a flat ceiling".to_owned());
        } else {
            self.dimension_status = None;
        }
    }

    /// Toggle the vault tool — region-gated like the ceiling tool, but it authors a
    /// scissor/vault (two opposing sloped ceilings) rather than one flat ceiling.
    fn toggle_vault_tool(&mut self) {
        let activate = !self.vault_tool_active;
        self.deactivate_placement_tools();
        self.vault_tool_active = activate;
        if activate {
            if !self.workspace_mode.allows_design_edits() {
                self.set_workspace_mode(WorkspaceMode::Design);
            }
            self.command_tab = actions::WorkflowTab::Frame;
            self.viewport_mode = ViewportMode::Plan;
            self.dimension_status =
                Some("Click inside an enclosed area to vault it (two opposing slopes)".to_owned());
        } else {
            self.dimension_status = None;
        }
    }

    /// Toggle the floor-deck tool — region-gated like the ceiling tool.
    fn toggle_floor_tool(&mut self) {
        let activate = !self.floor_tool_active;
        self.deactivate_placement_tools();
        self.floor_tool_active = activate;
        if activate {
            if !self.workspace_mode.allows_design_edits() {
                self.set_workspace_mode(WorkspaceMode::Design);
            }
            self.command_tab = actions::WorkflowTab::Frame;
            self.viewport_mode = ViewportMode::Plan;
            self.dimension_status =
                Some("Click inside an enclosed area to place a floor deck".to_owned());
        } else {
            self.dimension_status = None;
        }
    }

    /// Clear every wall/room/ceiling/floor/dimension placement tool. Used by the
    /// region tools so activating one cancels the others.
    fn deactivate_placement_tools(&mut self) {
        self.draw_wall_tool = DrawWallToolState::default();
        self.dimension_tool.active = false;
        self.dimension_tool.clear_picks();
        self.room_tool_active = false;
        self.ceiling_tool_active = false;
        self.vault_tool_active = false;
        self.floor_tool_active = false;
        self.opening_drag = None;
    }

    /// Place a room from a room-tool click, but only when the point is inside a
    /// closed wall loop.
    fn handle_place_room(&mut self, point: Point2) {
        if !self.room_tool_active {
            return;
        }
        let level = self.active_level_id();
        if framer_core::room_boundary_on_level(&self.model, &level, point).is_some() {
            self.add_room(point);
        } else {
            self.dimension_status =
                Some("No enclosed area here — close a wall loop first".to_owned());
        }
    }

    /// Place a flat ceiling from a ceiling-tool click, gated on an enclosed loop.
    fn handle_place_ceiling(&mut self, point: Point2) {
        if !self.ceiling_tool_active {
            return;
        }
        match self.surface_region_at(point) {
            Some(region) => self.add_ceiling(region),
            None => {
                self.dimension_status =
                    Some("No enclosed area here — close a wall loop first".to_owned());
            }
        }
    }

    /// Place a floor deck from a floor-tool click, gated on an enclosed loop.
    fn handle_place_floor(&mut self, point: Point2) {
        if !self.floor_tool_active {
            return;
        }
        match self.surface_region_at(point) {
            Some(region) => self.add_floor(region),
            None => {
                self.dimension_status =
                    Some("No enclosed area here — close a wall loop first".to_owned());
            }
        }
    }

    /// Vault the enclosed loop under a vault-tool click: author the two opposing
    /// sloped ceilings of a scissor/vault, gated on a closed loop.
    fn handle_place_vault(&mut self, point: Point2) {
        if !self.vault_tool_active {
            return;
        }
        let level = self.active_level_id();
        match framer_core::room_boundary_on_level(&self.model, &level, point) {
            Some(boundary) => self.add_vault(&boundary.vertices),
            None => {
                self.dimension_status =
                    Some("No enclosed area here — close a wall loop first".to_owned());
            }
        }
    }

    /// Author a scissor/vault over `outline` as one undo step: two opposing sloped
    /// `Ceiling` planes (each a polygon half springing from its outer wall up to the
    /// shared ridge along the region's longer span), at a default 4:12 pitch, both
    /// editable in the inspector. The cathedral case is just "no ceiling", so this
    /// tool only authors the scissor form.
    fn add_vault(&mut self, outline: &[Point2]) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Some((low_half, high_half)) = scissor_halves(outline) else {
            self.dimension_status = Some("This area is too small to vault".to_owned());
            return;
        };
        // A default 4:12 pitch; the spring (low) edge is index 0 of each half by
        // construction. Both editable per ceiling in the inspector.
        let pitch =
            framer_core::Slope::new(Length::from_whole_inches(4), Length::from_whole_inches(12));
        let slope = framer_core::CeilingSlope::new(pitch, 0);
        self.edit("Add vault", |app| {
            let system = app.ensure_surface_system(framer_core::SystemKind::Ceiling);
            let level = app.active_level_id().0;
            let mut first_id = None;
            for half in [low_half, high_half] {
                let (id, index) = next_ceiling_id(&app.model);
                let mut ceiling = framer_core::Ceiling::new(
                    id.clone(),
                    format!("Vault {index}"),
                    level.clone(),
                    system.clone(),
                    framer_core::SurfaceRegion::Polygon(half),
                    Length::ZERO,
                );
                ceiling.slope = Some(slope);
                app.model.ceilings.push(ceiling);
                first_id.get_or_insert(id);
            }
            if let Some(id) = first_id {
                app.selected = Selection::Ceiling(id);
            }
        });
    }

    /// Resolve the enclosed wall loop under `point` to a [`SurfaceRegion`]: a
    /// `Room` reference when a room already occupies that loop (so the surface
    /// tracks the room as walls move), otherwise the loop's frozen `Polygon`.
    /// `None` when `point` is not inside any closed loop.
    fn surface_region_at(&self, point: Point2) -> Option<framer_core::SurfaceRegion> {
        use framer_core::SurfaceRegion;
        let level = self.active_level_id();
        // Resolve the loop under `point` and every room's loop in ONE batched pass:
        // `room_boundaries_on_level` derives the active level's bounded faces a
        // single time. `point` is the first seed; same-level rooms follow in order.
        let rooms: Vec<_> = self
            .model
            .rooms
            .iter()
            .filter(|room| room.level == level)
            .collect();
        let mut seeds = Vec::with_capacity(rooms.len() + 1);
        seeds.push(point);
        seeds.extend(rooms.iter().map(|room| room.seed));
        let mut boundaries =
            framer_core::room_boundaries_on_level(&self.model, &level, &seeds).into_iter();
        let boundary = boundaries.next().flatten()?; // the loop under `point`
        let room = rooms.iter().zip(boundaries).find_map(|(room, other)| {
            other
                .filter(|other| other.vertices == boundary.vertices)
                .map(|_| room.id.clone())
        });
        Some(match room {
            Some(id) => SurfaceRegion::Room(id),
            None => SurfaceRegion::Polygon(boundary.vertices),
        })
    }

    /// Add a flat ceiling over `region` as one undo step, seeding a ceiling system
    /// if the project has none yet. Defaults to flush with the level top (height 0,
    /// editable in the inspector).
    fn add_ceiling(&mut self, region: framer_core::SurfaceRegion) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        self.edit("Add ceiling", |app| {
            let system = app.ensure_surface_system(framer_core::SystemKind::Ceiling);
            let (id, index) = next_ceiling_id(&app.model);
            let level = app.active_level_id().0;
            let ceiling = framer_core::Ceiling::new(
                id.clone(),
                format!("Ceiling {index}"),
                level,
                system,
                region,
                Length::ZERO,
            );
            app.model.ceilings.push(ceiling);
            app.selected = Selection::Ceiling(id);
        });
    }

    /// Add a floor deck over `region` as one undo step, seeding a floor system if
    /// the project has none yet. Joists default to the shorter clear span.
    fn add_floor(&mut self, region: framer_core::SurfaceRegion) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        self.edit("Add floor", |app| {
            let system = app.ensure_surface_system(framer_core::SystemKind::Floor);
            let (id, index) = next_floor_id(&app.model);
            let level = app.active_level_id().0;
            let deck = framer_core::FloorDeck::new(
                id.clone(),
                format!("Floor {index}"),
                level,
                system,
                region,
            );
            app.model.floor_decks.push(deck);
            app.selected = Selection::FloorDeck(id);
        });
    }

    /// Auto-generate a roof of `form` over the project's wall footprint as one undo
    /// step (the hybrid roof tool: generate planes, then store them as editable
    /// objects), seeding a Roof system if the project has none. Switches to the
    /// roof-plan view so the result is visible. No-ops with a status hint when
    /// there is no footprint yet.
    fn add_roof(&mut self, form: RoofForm) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let level = self.active_level_id().0;
        let Some((specs, springing)) = self.footprint_roof_specs(&level, form) else {
            self.dimension_status =
                Some("Draw walls to enclose a footprint before adding a roof".to_owned());
            return;
        };
        // A default 4:12 pitch with modest overhangs; all editable per plane.
        let slope =
            framer_core::Slope::new(Length::from_whole_inches(4), Length::from_whole_inches(12));
        let eave_overhang = Length::from_whole_inches(12);
        let rake_overhang = Length::from_whole_inches(8);
        self.edit("Add roof", |app| {
            let system = app.ensure_surface_system(framer_core::SystemKind::Roof);
            let mut last_id = None;
            for (outline, eave_edge) in specs {
                let (id, index) = next_roof_id(&app.model);
                let plane = framer_core::RoofPlane::new(
                    id.clone(),
                    format!("Roof plane {index}"),
                    level.clone(),
                    system.clone(),
                    outline,
                    slope,
                    eave_edge,
                    springing,
                )
                .with_eave_overhang(eave_overhang)
                .with_rake_overhang(rake_overhang);
                app.model.roof_planes.push(plane);
                last_id = Some(id);
            }
            if let Some(id) = last_id {
                app.selected = Selection::RoofPlane(id);
            }
        });
        self.viewport_mode = ViewportMode::RoofPlan;
    }

    /// The plane outlines (plan polygon + eave-edge index) for an auto-generated
    /// `form` roof over the footprint of the walls on `level`, plus the springing
    /// elevation (the bearing line). `None` when the level has no walls. The
    /// springing prefers the authored level top (`elevation + height`) and falls
    /// back to the tallest wall on the level when the level has no authored height,
    /// so a one-click roof on a fresh shell still sits on top of the walls.
    fn footprint_roof_specs(
        &self,
        level: &str,
        form: RoofForm,
    ) -> Option<(Vec<RoofPlaneSpec>, Length)> {
        let walls: Vec<&Wall> = self
            .model
            .walls
            .iter()
            .filter(|wall| wall.level.0 == level)
            .collect();
        if walls.is_empty() {
            return None;
        }
        let mut min_x = i64::MAX;
        let mut min_y = i64::MAX;
        let mut max_x = i64::MIN;
        let mut max_y = i64::MIN;
        for wall in &walls {
            for point in [wall.start, wall.end] {
                min_x = min_x.min(point.x.ticks());
                min_y = min_y.min(point.y.ticks());
                max_x = max_x.max(point.x.ticks());
                max_y = max_y.max(point.y.ticks());
            }
        }
        if min_x >= max_x || min_y >= max_y {
            return None;
        }
        let footprint_outline = level_wall_loop_outline(&self.model, &ElementId::new(level));
        let p = |x: i64, y: i64| Point2::new(Length::from_ticks(x), Length::from_ticks(y));

        let specs = match form {
            // A shed roof is one plane over the whole footprint, sloping up from the
            // low (min-y) eave to the opposite ridge.
            RoofForm::Shed => vec![(
                vec![
                    p(min_x, min_y),
                    p(max_x, min_y),
                    p(max_x, max_y),
                    p(min_x, max_y),
                ],
                0,
            )],
            // A gable is two opposing planes meeting at a ridge along the longer
            // axis; each plane's eave is its outer footprint edge (the up-slope
            // direction toward the ridge is derived by RoofPlane::frame()).
            RoofForm::Gable => {
                if max_x - min_x >= max_y - min_y {
                    let mid = (min_y + max_y) / 2;
                    vec![
                        (
                            vec![
                                p(min_x, min_y),
                                p(max_x, min_y),
                                p(max_x, mid),
                                p(min_x, mid),
                            ],
                            0,
                        ),
                        (
                            vec![
                                p(min_x, mid),
                                p(max_x, mid),
                                p(max_x, max_y),
                                p(min_x, max_y),
                            ],
                            2,
                        ),
                    ]
                } else {
                    let mid = (min_x + max_x) / 2;
                    vec![
                        (
                            vec![
                                p(min_x, min_y),
                                p(mid, min_y),
                                p(mid, max_y),
                                p(min_x, max_y),
                            ],
                            3,
                        ),
                        (
                            vec![
                                p(mid, min_y),
                                p(max_x, min_y),
                                p(max_x, max_y),
                                p(mid, max_y),
                            ],
                            1,
                        ),
                    ]
                }
            }
            // A rectangular hip roof has four authored planes. On the longer axis,
            // the two long sides are trapezoids whose high edge is the central
            // ridge; the two short ends are hip triangles. A square footprint
            // degenerates to four triangles meeting at one peak.
            RoofForm::Hip => {
                if let Some(specs) = footprint_outline
                    .as_deref()
                    .and_then(orthogonal_valley_roof_specs)
                {
                    specs
                } else {
                    let width = max_x - min_x;
                    let depth = max_y - min_y;
                    if width == depth {
                        let peak = p((min_x + max_x) / 2, (min_y + max_y) / 2);
                        vec![
                            (vec![p(min_x, min_y), p(max_x, min_y), peak], 0),
                            (vec![p(max_x, min_y), p(max_x, max_y), peak], 0),
                            (vec![p(max_x, max_y), p(min_x, max_y), peak], 0),
                            (vec![p(min_x, max_y), p(min_x, min_y), peak], 0),
                        ]
                    } else if width > depth {
                        let inset = depth / 2;
                        let mid_y = (min_y + max_y) / 2;
                        let ridge_west = p(min_x + inset, mid_y);
                        let ridge_east = p(max_x - inset, mid_y);
                        vec![
                            (
                                vec![p(min_x, min_y), p(max_x, min_y), ridge_east, ridge_west],
                                0,
                            ),
                            (
                                vec![p(max_x, max_y), p(min_x, max_y), ridge_west, ridge_east],
                                0,
                            ),
                            (vec![p(max_x, min_y), p(max_x, max_y), ridge_east], 0),
                            (vec![p(min_x, max_y), p(min_x, min_y), ridge_west], 0),
                        ]
                    } else {
                        let inset = width / 2;
                        let mid_x = (min_x + max_x) / 2;
                        let ridge_south = p(mid_x, min_y + inset);
                        let ridge_north = p(mid_x, max_y - inset);
                        vec![
                            (
                                vec![p(min_x, max_y), p(min_x, min_y), ridge_south, ridge_north],
                                0,
                            ),
                            (
                                vec![p(max_x, min_y), p(max_x, max_y), ridge_north, ridge_south],
                                0,
                            ),
                            (vec![p(min_x, min_y), p(max_x, min_y), ridge_south], 0),
                            (vec![p(max_x, max_y), p(min_x, max_y), ridge_north], 0),
                        ]
                    }
                }
            }
        };

        let level_def = self.model.levels.iter().find(|lvl| lvl.id.0 == level);
        let elevation = level_def.map(|lvl| lvl.elevation).unwrap_or(Length::ZERO);
        let height = level_def.map(|lvl| lvl.height).unwrap_or(Length::ZERO);
        let springing = if height > Length::ZERO {
            elevation + height
        } else {
            let tallest = walls
                .iter()
                .map(|wall| wall.height)
                .max()
                .unwrap_or(Length::ZERO);
            elevation + tallest
        };
        Some((specs, springing))
    }

    fn add_opening(&mut self, kind: OpeningKind) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }

        self.edit("Add opening", |app| {
            let Some(wall) = app.model.walls.get_mut(app.selected_wall) else {
                return;
            };

            let center = (wall.length / 2).max(Length::from_inches(24.0));
            let opening = match kind {
                OpeningKind::Door => {
                    let (id, index) = next_opening_id(wall, "opening-door");
                    Opening::door(
                        id,
                        format!("Door {index}"),
                        center,
                        Length::from_inches(36.0),
                        Length::from_inches(80.0),
                    )
                }
                OpeningKind::GarageDoor => {
                    let (id, index) = next_opening_id(wall, "opening-garage");
                    Opening::door(
                        id,
                        format!("Garage Door {index}"),
                        center,
                        Length::from_feet(8.0),
                        Length::from_inches(84.0),
                    )
                    .with_kind(OpeningKind::GarageDoor)
                }
                OpeningKind::Window => {
                    let (id, index) = next_opening_id(wall, "opening-window");
                    Opening::window(
                        id,
                        format!("Window {index}"),
                        center,
                        Length::from_inches(36.0),
                        Length::from_inches(42.0),
                        Length::from_inches(36.0),
                    )
                }
                // Skylight and stair openings are window-shaped on a wall but keep
                // their own kind (rather than being coerced to a window): a skylight
                // hosted on a wall reads as a skylight, and the BOM/render see the
                // true kind. Roof-hosted skylights are a separate RoofOpening.
                OpeningKind::Skylight => {
                    let (id, index) = next_opening_id(wall, "opening-skylight");
                    Opening::window(
                        id,
                        format!("Skylight {index}"),
                        center,
                        Length::from_inches(36.0),
                        Length::from_inches(42.0),
                        Length::from_inches(36.0),
                    )
                    .with_kind(OpeningKind::Skylight)
                }
                OpeningKind::Stair => {
                    let (id, index) = next_opening_id(wall, "opening-stair");
                    Opening::window(
                        id,
                        format!("Stair {index}"),
                        center,
                        Length::from_inches(36.0),
                        Length::from_inches(42.0),
                        Length::from_inches(36.0),
                    )
                    .with_kind(OpeningKind::Stair)
                }
            };

            app.selected = Selection::Opening(opening.id.0.clone());
            wall.openings.push(opening);
        });
    }

    fn duplicate_selected_opening(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::Opening(id) = self.selected.clone() else {
            return;
        };
        self.edit("Duplicate opening", |app| {
            let Some(wall) = app.model.walls.get_mut(app.selected_wall) else {
                return;
            };
            let Some(source) = wall
                .openings
                .iter()
                .find(|opening| opening.id.0 == id)
                .cloned()
            else {
                return;
            };
            let (new_id, _) = next_opening_id(wall, "opening-copy");
            let mut clone = source.clone();
            clone.id = ElementId::new(new_id.clone());
            clone.name = format!("{} copy", source.name);
            let half_width = source.width / 2;
            clone.center = (source.center + source.width)
                .min(wall.length - half_width)
                .max(half_width);
            wall.openings.push(clone);
            app.selected = Selection::Opening(new_id);
        });
    }

    fn delete_selected_opening(&mut self) {
        if !self.workspace_mode.allows_design_edits() {
            return;
        }
        let Selection::Opening(id) = self.selected.clone() else {
            return;
        };
        self.edit("Delete opening", |app| {
            let Some(wall) = app.model.walls.get_mut(app.selected_wall) else {
                return;
            };
            if wall.remove_opening(&ElementId::new(id)) {
                app.selected = Selection::Wall;
            }
        });
    }

    fn begin_opening_drag(
        &mut self,
        wall_index: usize,
        opening_id: String,
        handle: OpeningEditHandle,
    ) {
        let Some(wall) = self.model.walls.get(wall_index) else {
            return;
        };
        let Some(opening) = wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == opening_id)
        else {
            return;
        };

        // Build the drag state (copies the opening's geometry) so the borrow of
        // the model ends before we snapshot it for the undo transaction.
        let drag = OpeningDragState::new(wall_index, opening_id.clone(), handle, opening);
        self.selected_wall = wall_index;
        self.selected = Selection::Opening(opening_id);
        // Open one coalesced undo step for the whole drag, capturing the base
        // *after* selecting the dragged opening so undo restores it as selected.
        // update_opening_drag mutates within it; finish_opening_drag settles it.
        self.history.begin(self.snapshot(), "Move opening");
        self.opening_drag = Some(drag);
    }

    fn update_opening_drag(&mut self, delta_x: Length, delta_y: Length) {
        let Some(drag) = self.opening_drag.clone() else {
            return;
        };
        let constraints = OpeningDragConstraints::from_code(&self.model.framing_defaults())
            .with_modifiers(self.snap_step, self.ortho);
        let Some(wall) = self.model.walls.get_mut(drag.wall_index) else {
            self.opening_drag = None;
            return;
        };

        if apply_opening_drag(
            wall,
            &drag.opening_id,
            drag.handle,
            drag.start,
            delta_x,
            delta_y,
            constraints,
        ) {
            self.selected_wall = drag.wall_index;
            self.selected = Selection::Opening(drag.opening_id);
            self.rebuild();
        }
    }

    fn finish_opening_drag(&mut self) {
        self.opening_drag = None;
        // Settle the drag's transaction: commit it as one undo step if the
        // opening actually moved, otherwise discard it.
        self.settle_history(false);
    }

    fn handle_wall_drag_event(&mut self, event: WallDragEvent) {
        match event {
            WallDragEvent::Started { wall_index, handle } => {
                self.begin_wall_drag(wall_index, handle)
            }
            WallDragEvent::Updated { point } => self.update_wall_drag(point),
            WallDragEvent::Translated { dx, dy } => self.translate_wall_drag(dx, dy),
            WallDragEvent::Stopped => self.finish_wall_drag(),
        }
    }

    fn begin_wall_drag(&mut self, wall_index: usize, handle: WallEditHandle) {
        if self.model.walls.get(wall_index).is_none() {
            return;
        }
        self.selected_wall = wall_index;
        self.selected = Selection::Wall;
        // One coalesced undo step for the whole gesture, captured after selecting
        // the wall so undo restores it as selected.
        self.history.begin(self.snapshot(), "Move wall");
        self.wall_drag = Some(WallDragState {
            wall_index,
            handle,
            applied: (Length::ZERO, Length::ZERO),
        });
    }

    fn update_wall_drag(&mut self, point: Point2) {
        let Some(drag) = self.wall_drag else {
            return;
        };
        let Some(which_end) = drag.handle.as_wall_end() else {
            return; // body translations arrive via translate_wall_drag
        };
        let Some(wall) = self.model.walls.get(drag.wall_index) else {
            self.wall_drag = None;
            return;
        };
        let old_point = match which_end {
            framer_core::WallEnd::Start => wall.start,
            framer_core::WallEnd::End => wall.end,
        };
        if old_point == point {
            return;
        }
        // Clamp a move that would skew a perpendicular neighbour off-axis (the
        // model forbids non-orthogonal walls).
        if !endpoint_move_keeps_ortho(&self.model, old_point, point) {
            return;
        }
        // Clamp a move that would collapse an affected wall to zero length (which
        // would fail validation and drop the framing plan).
        if !endpoint_move_keeps_positive_length(&self.model, old_point, point) {
            return;
        }
        // Clamp a move that a driving dimension would fight back on the next solve
        // (it rewrites the wall's length/end, which would tear the moved corner).
        if self.nodes_touch_driving_dimension(&[old_point]) {
            return;
        }
        let wall_id = wall.id.clone();
        let moved = self.model.move_wall_endpoint(&wall_id, which_end, point);
        if moved.is_empty() {
            return;
        }
        self.settle_wall_geometry(drag.wall_index);
    }

    /// Translate the whole dragged wall to track the cursor. `total` is the model
    /// delta from drag start; it is projected onto the wall's perpendicular (so it
    /// slides sideways — the ortho-safe reposition) and applied as the increment
    /// since the last accepted frame, clamped if it would skew or collapse a
    /// neighbour. The accepted total is recorded so a clamped frame is retried
    /// (not lost) on the next.
    fn translate_wall_drag(&mut self, total_dx: Length, total_dy: Length) {
        let Some(drag) = self.wall_drag else {
            return;
        };
        let Some(wall) = self.model.walls.get(drag.wall_index) else {
            self.wall_drag = None;
            return;
        };
        // Perpendicular projection: a horizontal wall slides in y, a vertical in x.
        let target = if wall.start.y == wall.end.y {
            (Length::ZERO, total_dy)
        } else {
            (total_dx, Length::ZERO)
        };
        let inc_x = target.0 - drag.applied.0;
        let inc_y = target.1 - drag.applied.1;
        if inc_x == Length::ZERO && inc_y == Length::ZERO {
            return;
        }
        let wall_id = wall.id.clone();
        let (start, end) = (wall.start, wall.end);
        if !translate_keeps_ortho(&self.model, &wall_id, start, end, inc_x, inc_y)
            || !translate_keeps_positive_length(&self.model, &wall_id, start, end, inc_x, inc_y)
            || self.nodes_touch_driving_dimension(&[start, end])
        {
            return; // clamp this frame; `applied` unchanged so the next frame retries
        }
        let moved = self.model.translate_wall(&wall_id, inc_x, inc_y);
        if moved.is_empty() {
            return;
        }
        if let Some(state) = self.wall_drag.as_mut() {
            state.applied = target;
        }
        self.settle_wall_geometry(drag.wall_index);
    }

    /// Whether any wall touching one of `nodes` carries a driving dimension. Such
    /// a wall's length/end is rewritten by the next solve, so a geometry drag that
    /// moves its endpoint would be undone (and could tear a corner) — we clamp it.
    fn nodes_touch_driving_dimension(&self, nodes: &[Point2]) -> bool {
        self.model.walls.iter().any(|wall| {
            nodes
                .iter()
                .any(|node| wall.start == *node || wall.end == *node)
                && wall
                    .dimensions
                    .iter()
                    .any(|dimension| dimension.kind == DimensionKind::Driving)
        })
    }

    /// Shared tail of a wall-geometry edit frame: keep joins consistent (so the
    /// model stays valid and the plan keeps generating) and re-solve.
    fn settle_wall_geometry(&mut self, wall_index: usize) {
        self.model.reconcile_joins();
        self.selected_wall = wall_index;
        self.selected = Selection::Wall;
        self.rebuild();
    }

    fn finish_wall_drag(&mut self) {
        self.wall_drag = None;
        self.settle_history(false);
    }

    fn handle_dimension_anchor_click(&mut self, wall_index: usize, anchor: DimensionAnchor) {
        if !self.workspace_mode.allows_design_edits() || !self.dimension_tool.active {
            return;
        }

        let pick = DimensionAnchorPick { wall_index, anchor };
        let Some(first) = self.dimension_tool.first_anchor.clone() else {
            self.dimension_status = Some("Pick a second dimension anchor".to_owned());
            self.dimension_tool.first_anchor = Some(pick);
            self.dimension_tool.second_anchor = None;
            return;
        };

        if first.wall_index != pick.wall_index {
            self.dimension_status =
                Some("Dimension anchors must be on the same wall for now".to_owned());
            self.dimension_tool.first_anchor = Some(pick);
            self.dimension_tool.second_anchor = None;
            return;
        }

        if first.anchor == pick.anchor {
            self.dimension_status = Some("Pick a different anchor".to_owned());
            self.dimension_tool.second_anchor = None;
            return;
        }

        self.dimension_tool.second_anchor = Some(pick);
        self.dimension_status = Some(
            "Move the pointer to choose the dimension axis and line position, then click to place"
                .to_owned(),
        );
    }

    fn handle_dimension_placement_click(
        &mut self,
        wall_index: usize,
        axis: DimensionAxis,
        line_offset: Length,
    ) {
        if !self.workspace_mode.allows_design_edits() || !self.dimension_tool.active {
            return;
        }

        self.edit("Add dimension", |app| {
            let Some(first) = app.dimension_tool.first_anchor.clone() else {
                return;
            };
            let Some(second) = app.dimension_tool.second_anchor.clone() else {
                return;
            };
            if first.wall_index != wall_index || second.wall_index != wall_index {
                app.dimension_status =
                    Some("Dimension anchors must be on the same wall for now".to_owned());
                app.dimension_tool.clear_picks();
                return;
            }

            let Some(wall) = app.model.walls.get_mut(wall_index) else {
                return;
            };
            let Some(start_coordinate) = first.anchor.coordinate(wall, axis) else {
                app.dimension_status =
                    Some("The first dimension anchor no longer exists".to_owned());
                return;
            };
            let Some(end_coordinate) = second.anchor.coordinate(wall, axis) else {
                app.dimension_status =
                    Some("The second dimension anchor no longer exists".to_owned());
                return;
            };
            let measured = (end_coordinate - start_coordinate).abs();
            if measured <= Length::ZERO {
                app.dimension_status =
                    Some("Move the pointer to place a non-zero dimension".to_owned());
                return;
            }

            let kind = app.dimension_tool.kind;
            let (id, index) = next_dimension_id(wall);
            let direction = if end_coordinate >= start_coordinate {
                DimensionDirection::Forward
            } else {
                DimensionDirection::Backward
            };
            let value = if kind == DimensionKind::Driving {
                Some(measured)
            } else {
                None
            };
            let dimension = DimensionConstraint::new(
                id.clone(),
                format!("Dimension {index}"),
                kind,
                first.anchor,
                second.anchor,
                direction,
                value,
            )
            .with_axis(axis)
            .with_line_offset(line_offset);
            if wall.would_overconstrain_driving_dimension(&dimension) {
                app.dimension_status =
                    Some("Driving dimension would overconstrain this wall".to_owned());
                return;
            }
            wall.dimensions.push(dimension);
            app.dimension_tool.axis = axis;
            app.dimension_tool.clear_picks();

            app.selected_wall = wall_index;
            app.selected = Selection::Dimension(id);
            app.dimension_status = Some(format!(
                "Added {} {} dimension",
                dimension_axis_name(axis),
                dimension_kind_name(kind)
            ));
        });
    }

    fn selected_member(&self, wall_id: &str, member_id: &str) -> Option<&FrameMember> {
        self.project_plan
            .as_ref()?
            .wall_plans
            .iter()
            .find(|wall_plan| wall_plan.wall.0 == wall_id)?
            .members
            .iter()
            .find(|member| member.id == member_id)
    }

    fn handle_view_click(&mut self, click: ViewClick) {
        self.opening_drag = None;
        match click {
            ViewClick::Wall(index) => {
                self.selected_wall = index;
                self.selected = Selection::Wall;
                self.open_wall_view_from_design_shell();
            }
            ViewClick::Opening {
                wall_index,
                opening_id,
            } => {
                self.selected_wall = wall_index;
                self.selected = Selection::Opening(opening_id);
                self.open_wall_view_from_design_shell();
            }
            ViewClick::Dimension {
                wall_index,
                dimension_id,
            } => {
                self.selected_wall = wall_index;
                self.selected = Selection::Dimension(dimension_id);
                self.open_wall_view_from_design_shell();
            }
            ViewClick::DimensionAnchor { wall_index, anchor } => {
                self.handle_dimension_anchor_click(wall_index, anchor);
            }
            ViewClick::DimensionPlacement {
                wall_index,
                axis,
                line_offset,
            } => {
                self.handle_dimension_placement_click(wall_index, axis, line_offset);
            }
            ViewClick::DrawWallPoint { point } => {
                self.handle_draw_wall_point(point);
            }
            ViewClick::DrawWallCancel => {
                self.draw_wall_tool.start = None;
            }
            ViewClick::PlaceRoom { point } => {
                self.handle_place_room(point);
            }
            ViewClick::PlaceCeiling { point } => {
                self.handle_place_ceiling(point);
            }
            ViewClick::PlaceFloor { point } => {
                self.handle_place_floor(point);
            }
            ViewClick::PlaceVault { point } => {
                self.handle_place_vault(point);
            }
            ViewClick::Room { room_id } => {
                self.selected = Selection::Room(room_id);
            }
            ViewClick::FurnishingInstance { instance_id } => {
                self.selected = Selection::FurnishingInstance(instance_id);
            }
            ViewClick::MepInstance { instance_id } => {
                self.selected = Selection::MepInstance(instance_id);
            }
            ViewClick::Member { wall_id, member_id } => {
                if self.workspace_mode.shows_generated_plan() {
                    if let Some(index) = self
                        .model
                        .walls
                        .iter()
                        .position(|wall| wall.id.0 == wall_id)
                    {
                        self.selected_wall = index;
                    }
                    self.selected = Selection::Member { wall_id, member_id };
                }
            }
            ViewClick::RoofPlane { id } => {
                self.selected = Selection::RoofPlane(id);
            }
            ViewClick::Ceiling { id } => {
                self.selected = Selection::Ceiling(id);
            }
            ViewClick::FloorDeck { id } => {
                self.selected = Selection::FloorDeck(id);
            }
        }
    }

    fn open_wall_view_from_design_shell(&mut self) {
        if self.workspace_mode.allows_design_edits() && self.viewport_mode == ViewportMode::Plan {
            self.viewport_mode = ViewportMode::Elevation;
        }
    }
}

fn collect_library_lifecycle_issues(
    model: &BuildingModel,
) -> Result<Vec<framer_library::LibraryIssue>, String> {
    let current_libraries = framer_library::starter_library_ref()
        .ok()
        .map(|loaded| std::slice::from_ref(&loaded.library))
        .unwrap_or_default();
    framer_library::library_lifecycle_issues(model, current_libraries)
        .map_err(|error| error.to_string())
}

fn is_project_local_standards_pack_id(id: &ElementId) -> bool {
    id.0.starts_with("std-local-")
}

fn project_local_standards_pack(
    model: &BuildingModel,
    id: String,
    index: usize,
) -> framer_core::StandardsPack {
    framer_core::StandardsPack {
        id: ElementId::new(id),
        name: format!("Project standards {index}"),
        edition: "Project".to_owned(),
        source: None,
        tables: framer_core::StandardsTables {
            defaults: model.framing_defaults(),
            studs: Vec::new(),
            headers: Vec::new(),
            fastening: Vec::new(),
            bracing: Vec::new(),
        },
        checks: Vec::new(),
        overlays: Vec::new(),
        tags: Vec::new(),
        properties: Default::default(),
    }
}

fn append_library_diagnostics(
    plan: &mut ProjectFramePlan,
    issues: &[framer_library::LibraryIssue],
    check_error: Option<&str>,
) {
    if let Some(error) = check_error {
        plan.diagnostics.push(PlanDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "library.lifecycle.check-failed".to_owned(),
            source: None,
            message: format!("Library lifecycle status could not be checked: {error}."),
            rule: None,
        });
        return;
    }

    for issue in issues {
        let item_kind = match &issue.item {
            framer_library::LibraryItem::Material(_) => "material",
            framer_library::LibraryItem::System(_) => "system",
            framer_library::LibraryItem::Furnishing(_) => "furnishing",
            framer_library::LibraryItem::MepObject(_) => "MEP object",
            framer_library::LibraryItem::StandardsPack(_) => "standards pack",
        };
        let code = match issue.kind {
            framer_library::LibraryIssueKind::Diverged => "library.item.diverged",
            framer_library::LibraryIssueKind::OutOfDate => "library.item.out-of-date",
            framer_library::LibraryIssueKind::SourceMissing => "library.item.source-missing",
        };
        let message = match issue.kind {
            framer_library::LibraryIssueKind::Diverged => format!(
                "Library {item_kind} '{}' has local edits; detach it to keep those edits or re-sync it to overwrite them from the source library.",
                issue.item_id().0
            ),
            framer_library::LibraryIssueKind::OutOfDate => format!(
                "Library {item_kind} '{}' is out of date with source item '{}'.",
                issue.item_id().0,
                issue.source_id.0
            ),
            framer_library::LibraryIssueKind::SourceMissing => format!(
                "Library {item_kind} '{}' references source item '{}' which is not present in the available library.",
                issue.item_id().0,
                issue.source_id.0
            ),
        };
        plan.diagnostics.push(PlanDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: code.to_owned(),
            source: Some(issue.item_id().clone()),
            message,
            rule: None,
        });
    }
}

impl eframe::App for FramerApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_keyboard_shortcuts(ctx);

        // Smoke test: drive the GPU Render view for a fixed number of frames,
        // then close. Exercises the egui_wgpu compute+blit callback on the real
        // device (which the headless tests can't reach). Enable with
        // `FRAMER_RENDER_SMOKE=<frames> cargo run -p framer-app`.
        if let Some(frames_left) = self.render_smoke {
            self.viewport_mode = ViewportMode::Render;
            if frames_left == 0 {
                eprintln!(
                    "render smoke complete: {} samples accumulated",
                    self.render_gpu.samples()
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.render_smoke = Some(frames_left - 1);
                ctx.request_repaint();
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui_root(ui);
    }
}

impl FramerApp {
    /// Render one full UI frame into a central, margin-less [`egui::Ui`].
    ///
    /// This is the body of [`eframe::App::ui`], factored out so the headless
    /// `egui_kittest` UI harness (see `ui_harness_tests`) can drive the exact
    /// same panel layout without an [`eframe::Frame`], which can't be
    /// constructed outside the eframe runtime.
    pub(crate) fn ui_root(&mut self, ui: &mut egui::Ui) {
        let t = design::active();
        Panel::top("app-header")
            .frame(
                Frame::new()
                    .fill(t.title_bar)
                    .inner_margin(egui::Margin::symmetric(8, 5)),
            )
            .show(ui, |ui| self.app_header(ui));
        Panel::top("toolbar")
            .frame(
                Frame::new()
                    .fill(t.toolbar)
                    .stroke(t.soft_stroke())
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| self.toolbar(ui));
        Panel::bottom("status-bar")
            .frame(
                Frame::new()
                    .fill(theme::chrome_top())
                    .stroke(theme::soft_stroke())
                    .inner_margin(egui::Margin::symmetric(10, 5)),
            )
            .show(ui, |ui| self.status_bar(ui));
        Panel::left("model-tree")
            .resizable(true)
            .default_size(280.0)
            .size_range(240.0..=380.0)
            .frame(
                Frame::new()
                    .fill(theme::panel_bg())
                    .stroke(theme::soft_stroke())
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| self.model_tree(ui));
        Panel::right("inspector")
            .resizable(true)
            .default_size(360.0)
            .size_range(300.0..=520.0)
            .frame(
                Frame::new()
                    .fill(theme::panel_bg())
                    .stroke(theme::soft_stroke())
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.inspector(ui));
            });
        CentralPanel::default()
            .frame(Frame::new().fill(theme::workspace_bg()))
            .show(ui, |ui| self.workspace(ui));
        self.command_search_overlay(ui.ctx());

        // All panels have rendered; any inspector edit run has opened its
        // transaction. Settle it into a single undo step once the interaction
        // ends (pointer released and no text field focused).
        let interacting =
            ui.ctx().input(|input| input.pointer.any_down()) || ui.ctx().text_edit_focused();
        self.settle_history(interacting);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, process};

    use framer_core::{
        BracedWallLine, DimensionHorizontalReference, DimensionVerticalReference, Furnishing,
        FurnishingInstance, MepInstance, MepObject, MepObjectKind, RuleOverlay,
    };

    use super::*;

    fn place_pending_dimension(app: &mut FramerApp, axis: DimensionAxis) {
        app.handle_view_click(ViewClick::DimensionPlacement {
            wall_index: 0,
            axis,
            line_offset: dimension_line_offset(axis),
        });
    }

    fn dimension_line_offset(axis: DimensionAxis) -> Length {
        match axis {
            DimensionAxis::Horizontal => Length::from_inches(72.0),
            DimensionAxis::Vertical => Length::from_inches(96.0),
        }
    }

    fn pt_ft(x_ft: f64, y_ft: f64) -> Point2 {
        Point2::new(Length::from_feet(x_ft), Length::from_feet(y_ft))
    }

    #[test]
    fn reset_tools_returns_workflow_commands_to_frame() {
        let mut app = FramerApp {
            command_tab: actions::WorkflowTab::Plan,
            dimension_tool: DimensionToolState {
                active: true,
                ..Default::default()
            },
            draw_wall_tool: DrawWallToolState {
                active: true,
                ..Default::default()
            },
            room_tool_active: true,
            ..Default::default()
        };

        app.reset_tools();

        assert_eq!(app.command_tab, actions::WorkflowTab::Frame);
        assert!(!app.dimension_tool.active);
        assert!(!app.draw_wall_tool.active);
        assert!(!app.room_tool_active);
    }

    #[test]
    fn orthogonal_valley_roof_specs_accepts_symmetric_l() {
        let shared_low = pt_ft(0.0, 0.0);
        let reentrant = pt_ft(12.0, 12.0);
        let specs = orthogonal_valley_roof_specs(&[
            shared_low,
            pt_ft(24.0, 0.0),
            pt_ft(24.0, 12.0),
            reentrant,
            pt_ft(12.0, 24.0),
            pt_ft(0.0, 24.0),
        ])
        .unwrap();

        assert_eq!(
            specs,
            vec![
                (
                    vec![shared_low, pt_ft(24.0, 0.0), pt_ft(24.0, 12.0), reentrant],
                    0,
                ),
                (
                    vec![shared_low, pt_ft(0.0, 24.0), pt_ft(12.0, 24.0), reentrant],
                    0,
                ),
            ]
        );
        assert!(
            specs
                .iter()
                .all(|(outline, _)| outline.contains(&shared_low) && outline.contains(&reentrant))
        );
    }

    #[test]
    fn orthogonal_valley_roof_specs_accepts_mirrored_l_orientations() {
        let cases = vec![
            (
                vec![
                    pt_ft(0.0, 0.0),
                    pt_ft(24.0, 0.0),
                    pt_ft(24.0, 24.0),
                    pt_ft(12.0, 24.0),
                    pt_ft(12.0, 12.0),
                    pt_ft(0.0, 12.0),
                ],
                vec![
                    (
                        vec![
                            pt_ft(24.0, 0.0),
                            pt_ft(0.0, 0.0),
                            pt_ft(0.0, 12.0),
                            pt_ft(12.0, 12.0),
                        ],
                        0,
                    ),
                    (
                        vec![
                            pt_ft(24.0, 0.0),
                            pt_ft(24.0, 24.0),
                            pt_ft(12.0, 24.0),
                            pt_ft(12.0, 12.0),
                        ],
                        0,
                    ),
                ],
            ),
            (
                vec![
                    pt_ft(0.0, 0.0),
                    pt_ft(12.0, 0.0),
                    pt_ft(12.0, 12.0),
                    pt_ft(24.0, 12.0),
                    pt_ft(24.0, 24.0),
                    pt_ft(0.0, 24.0),
                ],
                vec![
                    (
                        vec![
                            pt_ft(0.0, 24.0),
                            pt_ft(24.0, 24.0),
                            pt_ft(24.0, 12.0),
                            pt_ft(12.0, 12.0),
                        ],
                        0,
                    ),
                    (
                        vec![
                            pt_ft(0.0, 24.0),
                            pt_ft(0.0, 0.0),
                            pt_ft(12.0, 0.0),
                            pt_ft(12.0, 12.0),
                        ],
                        0,
                    ),
                ],
            ),
            (
                vec![
                    pt_ft(0.0, 12.0),
                    pt_ft(12.0, 12.0),
                    pt_ft(12.0, 0.0),
                    pt_ft(24.0, 0.0),
                    pt_ft(24.0, 24.0),
                    pt_ft(0.0, 24.0),
                ],
                vec![
                    (
                        vec![
                            pt_ft(24.0, 24.0),
                            pt_ft(0.0, 24.0),
                            pt_ft(0.0, 12.0),
                            pt_ft(12.0, 12.0),
                        ],
                        0,
                    ),
                    (
                        vec![
                            pt_ft(24.0, 24.0),
                            pt_ft(24.0, 0.0),
                            pt_ft(12.0, 0.0),
                            pt_ft(12.0, 12.0),
                        ],
                        0,
                    ),
                ],
            ),
        ];

        for (outline, expected) in cases {
            assert_eq!(orthogonal_valley_roof_specs(&outline).unwrap(), expected);
        }
    }

    #[test]
    fn orthogonal_valley_roof_specs_rejects_unequal_leg_l() {
        assert!(
            orthogonal_valley_roof_specs(&[
                pt_ft(0.0, 0.0),
                pt_ft(24.0, 0.0),
                pt_ft(24.0, 12.0),
                pt_ft(12.0, 12.0),
                pt_ft(12.0, 36.0),
                pt_ft(0.0, 36.0),
            ])
            .is_none()
        );
    }

    #[test]
    fn orthogonal_valley_roof_specs_rejects_rectangle() {
        assert!(
            orthogonal_valley_roof_specs(&[
                pt_ft(0.0, 0.0),
                pt_ft(24.0, 0.0),
                pt_ft(24.0, 12.0),
                pt_ft(0.0, 12.0),
            ])
            .is_none()
        );
    }

    #[test]
    fn app_saves_and_reopens_demo_project() {
        let path = std::env::temp_dir().join(format!("framer-demo-wall-{}.framer", process::id()));
        let mut app = FramerApp {
            project_path: path.display().to_string(),
            ..FramerApp::default()
        };

        app.save_project_file();
        assert!(matches!(app.file_status.as_deref(), Some(status) if status.starts_with("Saved ")));

        app.model.walls[0].name = "Changed wall".to_owned();
        app.load_project_file();

        assert!(
            matches!(app.file_status.as_deref(), Some(status) if status.starts_with("Opened "))
        );
        assert_eq!(app.model, BuildingModel::demo_shell());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn app_exports_svg_and_csv_artifacts() {
        let path =
            std::env::temp_dir().join(format!("framer-demo-export-{}.framer", process::id()));
        let svg_path = path.with_extension("svg");
        let csv_path = path.with_extension("csv");
        let mut app = FramerApp {
            project_path: path.display().to_string(),
            ..FramerApp::default()
        };

        app.export_current_artifacts();

        assert!(
            matches!(app.artifact_status.as_deref(), Some(status) if status.starts_with("Exported "))
        );
        let svg = fs::read_to_string(&svg_path).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("data-wall=\"wall-front\""));
        assert!(
            fs::read_to_string(&csv_path)
                .unwrap()
                .contains("quantity,profile,kind")
        );

        let _ = fs::remove_file(svg_path);
        let _ = fs::remove_file(csv_path);
    }

    #[test]
    fn app_regenerates_and_exports_compliance_report() {
        let path =
            std::env::temp_dir().join(format!("framer-compliance-export-{}.framer", process::id()));
        let csv_path = path.with_extension("compliance.csv");
        let mut app = FramerApp {
            project_path: path.display().to_string(),
            workspace_mode: WorkspaceMode::Plan,
            ..FramerApp::default()
        };

        let report = app
            .compliance_report
            .as_ref()
            .expect("rebuild should populate a derived compliance report");
        assert!(!report.entries.is_empty());

        app.export_compliance_report();

        assert!(
            matches!(app.artifact_status.as_deref(), Some(status) if status.starts_with("Exported "))
        );
        let csv = fs::read_to_string(&csv_path).unwrap();
        assert!(csv.starts_with("rule,citation,pack,outcome,element,message,chain\n"));
        assert!(csv.contains("irc2021."));

        let _ = fs::remove_file(csv_path);
    }

    #[test]
    fn compliance_export_requires_regenerated_report() {
        let path = std::env::temp_dir().join(format!(
            "framer-compliance-export-missing-{}.framer",
            process::id()
        ));
        let csv_path = path.with_extension("compliance.csv");
        let _ = fs::remove_file(&csv_path);
        let mut app = FramerApp {
            project_path: path.display().to_string(),
            compliance_report: None,
            ..FramerApp::default()
        };

        app.export_compliance_report();

        assert_eq!(
            app.artifact_status.as_deref(),
            Some("Export failed: regenerate a valid compliance report first")
        );
        assert!(!csv_path.exists());
    }

    #[test]
    fn rebuild_lowers_standards_report_issues_into_plan_diagnostics() {
        let mut app = FramerApp::default();
        app.model.walls[0].height = Length::from_feet(20.0);

        app.rebuild();

        let plan = app
            .project_plan
            .as_ref()
            .expect("high-wall model should still generate a framing plan");
        assert!(plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Violation
                && diagnostic.rule.as_ref().is_some_and(|rule| {
                    rule.rule == "irc2021.r602.3-5.stud-height"
                        && rule.citation == "IRC 2021 Table R602.3(5)"
                })
        }));
    }

    #[test]
    fn compliance_source_focus_selects_existing_model_elements() {
        let mut app = FramerApp::default();
        let (wall_index, wall_id, opening_id, wall_level) = app
            .model
            .walls
            .iter()
            .enumerate()
            .find_map(|(index, wall)| {
                wall.openings.first().map(|opening| {
                    (
                        index,
                        wall.id.clone(),
                        opening.id.clone(),
                        wall.level.clone(),
                    )
                })
            })
            .expect("demo shell should include at least one opening");

        app.focus_compliance_source(wall_id.clone());

        assert_eq!(app.selected_wall, wall_index);
        assert_eq!(app.selected, Selection::Wall);
        assert_eq!(app.active_level_id(), wall_level);
        assert!(matches!(
            app.file_status.as_deref(),
            Some(status) if status.contains(wall_id.0.as_str())
        ));

        app.focus_compliance_source(opening_id.clone());

        assert_eq!(app.selected_wall, wall_index);
        assert_eq!(app.selected, Selection::Opening(opening_id.0.clone()));
        assert_eq!(app.active_level_id(), wall_level);
        assert!(matches!(
            app.file_status.as_deref(),
            Some(status) if status.contains(opening_id.0.as_str())
        ));

        let braced_line_id = ElementId::new("bwl-test");
        app.model.braced_wall_lines.push(BracedWallLine {
            id: braced_line_id.clone(),
            name: "Test braced line".to_owned(),
            level: wall_level.clone(),
            start: pt_ft(0.0, 0.0),
            end: pt_ft(24.0, 0.0),
        });

        app.focus_compliance_source(braced_line_id.clone());

        assert_eq!(app.selected, Selection::Level(wall_level.0.clone()));
        assert_eq!(app.active_level_id(), wall_level);
        assert!(matches!(
            app.file_status.as_deref(),
            Some(status) if status.contains(braced_line_id.0.as_str())
        ));
    }

    #[test]
    fn new_project_creates_schema_backed_wall_intent() {
        let mut app = FramerApp::default();
        app.new_project();

        assert_eq!(app.model.walls.len(), 1);
        assert!(save_project_document(&app.model).is_ok());
    }

    #[test]
    fn library_diagnostics_cover_lifecycle_kinds_and_check_failure() {
        let issues = vec![
            framer_library::LibraryIssue {
                item: framer_library::LibraryItem::Material(ElementId::new("local-material")),
                source_id: ElementId::new("source-material"),
                library_uid: "11111111-1111-4111-8111-111111111111".to_owned(),
                version_id: "2026.06".to_owned(),
                kind: framer_library::LibraryIssueKind::OutOfDate,
                expected_hash: "blake3:expected-material".to_owned(),
                actual_hash: Some("blake3:actual-material".to_owned()),
            },
            framer_library::LibraryIssue {
                item: framer_library::LibraryItem::System(ElementId::new("local-system")),
                source_id: ElementId::new("source-system"),
                library_uid: "11111111-1111-4111-8111-111111111111".to_owned(),
                version_id: "2026.06".to_owned(),
                kind: framer_library::LibraryIssueKind::SourceMissing,
                expected_hash: "blake3:expected-system".to_owned(),
                actual_hash: None,
            },
        ];
        let mut plan = ProjectFramePlan {
            wall_plans: Vec::new(),
            floor_plans: Vec::new(),
            ceiling_plans: Vec::new(),
            roof_plans: Vec::new(),
            diagnostics: Vec::new(),
            rooms: Vec::new(),
            layers: Vec::new(),
            fasteners: Vec::new(),
        };

        append_library_diagnostics(&mut plan, &issues, None);

        assert_eq!(plan.diagnostics.len(), 2);
        assert_eq!(
            plan.diagnostics[0].code,
            "library.item.out-of-date".to_owned()
        );
        assert_eq!(plan.diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(
            plan.diagnostics[0].source,
            Some(ElementId::new("local-material"))
        );
        assert!(plan.diagnostics[0].message.contains("source-material"));
        assert_eq!(
            plan.diagnostics[1].code,
            "library.item.source-missing".to_owned()
        );
        assert_eq!(plan.diagnostics[1].severity, DiagnosticSeverity::Warning);
        assert_eq!(
            plan.diagnostics[1].source,
            Some(ElementId::new("local-system"))
        );
        assert!(plan.diagnostics[1].message.contains("source-system"));

        let mut failed_plan = ProjectFramePlan {
            wall_plans: Vec::new(),
            floor_plans: Vec::new(),
            ceiling_plans: Vec::new(),
            roof_plans: Vec::new(),
            diagnostics: Vec::new(),
            rooms: Vec::new(),
            layers: Vec::new(),
            fasteners: Vec::new(),
        };
        append_library_diagnostics(&mut failed_plan, &issues, Some("resolver failed"));

        assert_eq!(failed_plan.diagnostics.len(), 1);
        assert_eq!(
            failed_plan.diagnostics[0].code,
            "library.lifecycle.check-failed".to_owned()
        );
        assert_eq!(
            failed_plan.diagnostics[0].severity,
            DiagnosticSeverity::Warning
        );
        assert_eq!(failed_plan.diagnostics[0].source, None);
        assert!(
            failed_plan.diagnostics[0]
                .message
                .contains("resolver failed")
        );
    }

    #[test]
    fn design_mode_keeps_generated_members_out_of_the_editing_selection() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Plan);
        let wall_id = app.model.walls[0].id.0.clone();
        let member_id = app.project_plan.as_ref().unwrap().wall_plans[0].members[0]
            .id
            .clone();
        app.selected = Selection::Member { wall_id, member_id };

        app.set_workspace_mode(WorkspaceMode::Design);

        assert_eq!(app.workspace_mode, WorkspaceMode::Design);
        assert_eq!(app.selected, Selection::Wall);
    }

    #[test]
    fn plan_mode_does_not_mutate_authored_openings_from_catalog_actions() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Plan);
        let opening_count = app.model.walls[0].openings.len();

        app.add_opening(OpeningKind::Door);

        assert_eq!(app.model.walls[0].openings.len(), opening_count);
    }

    #[test]
    fn plan_mode_does_not_mutate_standards_from_authoring_actions() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Plan);
        let before = (
            app.model.standards.clone(),
            app.model.standards_packs.clone(),
        );

        app.add_project_standards_pack();
        app.insert_starter_standards_pack("std-irc-2021".to_owned());
        app.remove_standards_pack_from_stack("std-irc-2021".to_owned());
        app.waive_standards_rule(
            "irc2021.r602.3-5.studs".to_owned(),
            "engineered alternative".to_owned(),
        );

        assert_eq!(app.model.standards, before.0);
        assert_eq!(app.model.standards_packs, before.1);
    }

    #[test]
    fn standards_stack_edit_ops_add_reorder_remove_and_readd_pack() {
        let mut app = FramerApp::default();
        let base = app.model.standards[0].clone();

        app.add_project_standards_pack();

        let local = match &app.selected {
            Selection::StandardsPack(id) => ElementId::new(id.clone()),
            other => panic!("new standards pack should be selected, got {other:?}"),
        };
        assert!(local.0.starts_with("std-local-"));
        assert_eq!(app.model.standards, vec![base.clone(), local.clone()]);
        assert!(
            app.model
                .standards_packs
                .iter()
                .any(|pack| pack.id == local && pack.source.is_none())
        );
        assert!(save_project_document(&app.model).is_ok());

        app.move_standards_pack_in_stack(local.0.clone(), -1);
        assert_eq!(app.model.standards, vec![local.clone(), base.clone()]);

        app.move_standards_pack_in_stack(local.0.clone(), 1);
        assert_eq!(app.model.standards, vec![base.clone(), local.clone()]);
        app.move_standards_pack_in_stack(local.0.clone(), 1);
        assert_eq!(app.model.standards, vec![base, local.clone()]);

        app.remove_standards_pack_from_stack(local.0.clone());
        assert!(!app.model.standards.iter().any(|id| id == &local));
        assert!(
            app.model
                .standards_packs
                .iter()
                .any(|pack| pack.id == local)
        );

        app.add_standards_pack_to_stack(local.0.clone());
        assert_eq!(app.model.standards.last(), Some(&local));
        let readded_stack = app.model.standards.clone();
        app.add_standards_pack_to_stack(local.0.clone());
        assert_eq!(app.model.standards, readded_stack);
        app.add_standards_pack_to_stack("std-does-not-exist".to_owned());
        assert_eq!(app.model.standards, readded_stack);
        assert!(save_project_document(&app.model).is_ok());
    }

    #[test]
    fn starter_standards_pack_import_vendors_and_stacks_pack() {
        let mut app = FramerApp::default();
        let before_packs = app.model.standards_packs.len();
        let before_stack = app.model.standards.len();

        app.insert_starter_standards_pack("std-irc-2021".to_owned());

        let imported = match &app.selected {
            Selection::StandardsPack(id) => ElementId::new(id.clone()),
            other => panic!("imported standards pack should be selected, got {other:?}"),
        };
        assert_eq!(app.model.standards_packs.len(), before_packs + 1);
        assert_eq!(app.model.standards.len(), before_stack + 1);
        assert!(app.model.standards.iter().any(|id| id == &imported));
        let pack = app
            .model
            .standards_packs
            .iter()
            .find(|pack| pack.id == imported)
            .expect("imported pack exists");
        assert!(pack.source.is_some());
        assert!(
            matches!(app.file_status.as_deref(), Some(status) if status.starts_with("Inserted "))
        );
        assert!(save_project_document(&app.model).is_ok());
    }

    #[test]
    fn standards_waiver_creates_project_local_overlay_pack_and_updates_reason() {
        let mut app = FramerApp::default();
        let rule = "irc2021.r602.3-5.studs".to_owned();

        app.waive_standards_rule(rule.clone(), "   ".to_owned());
        assert_eq!(app.model.standards_packs.len(), 1);

        app.waive_standards_rule(rule.clone(), "accepted by AHJ".to_owned());

        let local = match &app.selected {
            Selection::StandardsPack(id) => ElementId::new(id.clone()),
            other => panic!("local waiver pack should be selected, got {other:?}"),
        };
        assert!(local.0.starts_with("std-local-"));
        assert_eq!(app.model.standards.last(), Some(&local));
        let pack = app
            .model
            .standards_packs
            .iter()
            .find(|pack| pack.id == local)
            .expect("local waiver pack exists");
        assert_eq!(pack.overlays.len(), 1);
        assert!(matches!(
            &pack.overlays[0],
            RuleOverlay::Waive { target, reason }
                if target == &rule && reason == "accepted by AHJ"
        ));
        assert_eq!(
            app.model
                .resolved_standards()
                .rules
                .iter()
                .find(|resolved| resolved.rule == rule)
                .and_then(|resolved| resolved.waived.as_deref()),
            Some("accepted by AHJ")
        );

        app.waive_standards_rule(rule.clone(), "revised alternate design".to_owned());
        let pack = app
            .model
            .standards_packs
            .iter()
            .find(|pack| pack.id == local)
            .expect("local waiver pack exists");
        assert_eq!(pack.overlays.len(), 1);
        assert!(matches!(
            &pack.overlays[0],
            RuleOverlay::Waive { target, reason }
                if target == &rule && reason == "revised alternate design"
        ));
        assert!(save_project_document(&app.model).is_ok());
    }

    #[test]
    fn standards_pack_delete_refuses_active_stack_entry() {
        let mut app = FramerApp::default();
        let active = app.model.standards[0].0.clone();
        app.selected = Selection::StandardsPack(active.clone());

        app.delete_selected();

        assert!(
            app.model
                .standards_packs
                .iter()
                .any(|pack| pack.id.0 == active)
        );
        assert!(
            matches!(app.error.as_deref(), Some(error) if error.contains("remove it from the stack first"))
        );

        app.error = None;
        app.remove_standards_pack_from_stack(active.clone());
        app.delete_selected();

        assert!(
            !app.model
                .standards_packs
                .iter()
                .any(|pack| pack.id.0 == active)
        );
        assert!(save_project_document(&app.model).is_ok());
    }

    #[test]
    fn view_click_selects_placed_object_instances() {
        let mut app = FramerApp::default();

        app.handle_view_click(ViewClick::FurnishingInstance {
            instance_id: "furnishing-instance-7".to_owned(),
        });
        assert_eq!(
            app.selected,
            Selection::FurnishingInstance("furnishing-instance-7".to_owned())
        );

        app.handle_view_click(ViewClick::MepInstance {
            instance_id: "mep-instance-2".to_owned(),
        });
        assert_eq!(
            app.selected,
            Selection::MepInstance("mep-instance-2".to_owned())
        );
    }

    #[test]
    fn delete_refuses_furnishing_family_that_is_still_placed() {
        let mut app = FramerApp::default();
        let level_id = app.model.levels[0].id.0.clone();
        app.model.furnishings.push(Furnishing::new(
            "furnishing-test",
            "Test furnishing",
            Length::from_inches(24.0),
            Length::from_inches(18.0),
            Length::from_inches(34.5),
        ));
        app.model.furnishing_instances.push(FurnishingInstance::new(
            "furnishing-instance-test",
            "Placed test furnishing",
            "furnishing-test",
            level_id,
            Point2::new(Length::ZERO, Length::ZERO),
        ));
        app.selected = Selection::Furnishing("furnishing-test".to_owned());

        app.delete_selected();

        assert_eq!(app.model.furnishings.len(), 1);
        assert_eq!(
            app.selected,
            Selection::Furnishing("furnishing-test".to_owned())
        );
        assert!(
            app.error
                .as_deref()
                .is_some_and(|message| message.contains("still placed"))
        );
        app.model.validate().unwrap();
    }

    #[test]
    fn delete_refuses_mep_family_that_is_still_placed() {
        let mut app = FramerApp::default();
        let level_id = app.model.levels[0].id.0.clone();
        app.model.mep_objects.push(MepObject::new(
            "mep-test",
            "Test MEP object",
            MepObjectKind::Electrical,
            Length::from_inches(14.0),
            Length::from_inches(4.0),
            Length::from_inches(24.0),
        ));
        app.model.mep_instances.push(MepInstance::new(
            "mep-instance-test",
            "Placed test MEP object",
            "mep-test",
            level_id,
            Point2::new(Length::ZERO, Length::ZERO),
        ));
        app.selected = Selection::MepObject("mep-test".to_owned());

        app.delete_selected();

        assert_eq!(app.model.mep_objects.len(), 1);
        assert_eq!(app.selected, Selection::MepObject("mep-test".to_owned()));
        assert!(
            app.error
                .as_deref()
                .is_some_and(|message| message.contains("still placed"))
        );
        app.model.validate().unwrap();
    }

    #[test]
    fn design_shell_wall_click_opens_wall_layout_view() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Design);
        app.viewport_mode = ViewportMode::Plan;

        app.handle_view_click(ViewClick::Wall(1));

        assert_eq!(app.selected_wall, 1);
        assert_eq!(app.selected, Selection::Wall);
        assert_eq!(app.viewport_mode, ViewportMode::Elevation);
    }

    #[test]
    fn plan_wall_click_keeps_plan_view() {
        let mut app = FramerApp::default();
        app.set_workspace_mode(WorkspaceMode::Plan);
        app.viewport_mode = ViewportMode::Plan;

        app.handle_view_click(ViewClick::Wall(1));

        assert_eq!(app.selected_wall, 1);
        assert_eq!(app.selected, Selection::Wall);
        assert_eq!(app.viewport_mode, ViewportMode::Plan);
    }

    #[test]
    fn dimension_shortcut_enters_dimension_tool_in_wall_view() {
        let mut app = FramerApp {
            viewport_mode: ViewportMode::Plan,
            ..Default::default()
        };
        app.dimension_tool.first_anchor = Some(DimensionAnchorPick {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });

        app.activate_dimension_tool();

        assert!(app.dimension_tool.active);
        assert_eq!(app.dimension_tool.first_anchor, None);
        assert_eq!(app.dimension_tool.second_anchor, None);
        assert_eq!(app.viewport_mode, ViewportMode::Elevation);
        assert!(
            app.dimension_status
                .as_deref()
                .is_some_and(|status| status.contains("Pick two anchors"))
        );
    }

    #[test]
    fn escape_exits_selected_dimension() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.first_anchor = Some(DimensionAnchorPick {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.selected = Selection::Dimension("dimension-1".to_owned());
        app.dimension_status = Some("Pick a second dimension anchor".to_owned());

        app.exit_current_context();

        assert!(!app.dimension_tool.active);
        assert_eq!(app.dimension_tool.first_anchor, None);
        assert_eq!(app.dimension_tool.second_anchor, None);
        assert_eq!(app.selected, Selection::Wall);
        assert_eq!(app.dimension_status, None);
    }

    #[test]
    fn escape_cancels_dimension_tool_without_dropping_other_selection() {
        let mut app = FramerApp::default();
        let opening = app.model.walls[0].openings[0].id.0.clone();
        app.dimension_tool.active = true;
        app.selected = Selection::Opening(opening.clone());
        app.dimension_status = Some("Pick two anchors in the wall view".to_owned());

        app.exit_current_context();

        assert!(!app.dimension_tool.active);
        assert_eq!(app.selected, Selection::Opening(opening));
        assert_eq!(app.dimension_status, None);
    }

    #[test]
    fn dimension_tool_clicks_create_driving_dimension() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Driving;
        let opening = app.model.walls[0].openings[0].id.clone();
        let expected = app.model.walls[0].openings[0].center;

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningCenter { opening },
        });
        assert_eq!(app.model.walls[0].dimensions.len(), 0);
        assert!(app.dimension_tool.second_anchor.is_some());

        place_pending_dimension(&mut app, DimensionAxis::Horizontal);

        let dimension = &app.model.walls[0].dimensions[0];
        assert_eq!(dimension.axis, DimensionAxis::Horizontal);
        assert_eq!(dimension.kind, DimensionKind::Driving);
        assert_eq!(dimension.value, Some(expected));
        assert_eq!(
            dimension.line_offset,
            Some(dimension_line_offset(DimensionAxis::Horizontal))
        );
        assert_eq!(app.dimension_tool.first_anchor, None);
        assert_eq!(app.dimension_tool.second_anchor, None);
        assert_eq!(app.selected, Selection::Dimension(dimension.id.0.clone()));
    }

    #[test]
    fn reference_dimensions_store_no_driving_value() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Reference;
        let opening = app.model.walls[0].openings[0].id.clone();

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningLeft { opening },
        });
        place_pending_dimension(&mut app, DimensionAxis::Horizontal);

        let dimension = &app.model.walls[0].dimensions[0];
        assert_eq!(dimension.kind, DimensionKind::Reference);
        assert_eq!(dimension.value, None);
    }

    #[test]
    fn vertical_dimension_tool_clicks_create_height_dimension() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Driving;
        app.dimension_tool.axis = DimensionAxis::Vertical;
        let opening = app.model.walls[0].openings[0].id.clone();
        let expected = app.model.walls[0].openings[0].height;

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningPoint {
                opening: opening.clone(),
                horizontal: DimensionHorizontalReference::Center,
                vertical: DimensionVerticalReference::Bottom,
            },
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningPoint {
                opening,
                horizontal: DimensionHorizontalReference::Center,
                vertical: DimensionVerticalReference::Top,
            },
        });
        place_pending_dimension(&mut app, DimensionAxis::Vertical);

        let dimension = &app.model.walls[0].dimensions[0];
        assert_eq!(dimension.axis, DimensionAxis::Vertical);
        assert_eq!(dimension.kind, DimensionKind::Driving);
        assert_eq!(dimension.value, Some(expected));
        assert_eq!(
            dimension.line_offset,
            Some(dimension_line_offset(DimensionAxis::Vertical))
        );
        assert_eq!(app.selected, Selection::Dimension(dimension.id.0.clone()));
    }

    #[test]
    fn dimension_tool_uses_placement_axis_for_pending_dimension() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Driving;
        app.dimension_tool.axis = DimensionAxis::Horizontal;
        let opening = app.model.walls[0].openings[0].id.clone();
        let expected =
            (app.model.walls[0].height - app.model.walls[0].openings[0].sill_height).abs();

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallPoint {
                horizontal: DimensionHorizontalReference::Left,
                vertical: DimensionVerticalReference::Top,
            },
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningPoint {
                opening,
                horizontal: DimensionHorizontalReference::Center,
                vertical: DimensionVerticalReference::Bottom,
            },
        });
        assert_eq!(app.model.walls[0].dimensions.len(), 0);

        place_pending_dimension(&mut app, DimensionAxis::Vertical);

        let dimension = &app.model.walls[0].dimensions[0];
        assert_eq!(dimension.axis, DimensionAxis::Vertical);
        assert_eq!(dimension.value, Some(expected));
        assert_eq!(app.dimension_tool.axis, DimensionAxis::Vertical);
        assert!(
            app.dimension_status
                .as_deref()
                .is_some_and(|status| status.contains("vertical"))
        );
    }

    #[test]
    fn dimension_tool_rejects_overconstrained_driving_dimension_on_creation() {
        let mut app = FramerApp::default();
        app.dimension_tool.active = true;
        app.dimension_tool.kind = DimensionKind::Driving;
        let opening = app.model.walls[0].openings[0].id.clone();

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningLeft {
                opening: opening.clone(),
            },
        });
        place_pending_dimension(&mut app, DimensionAxis::Horizontal);
        assert_eq!(app.model.walls[0].dimensions.len(), 1);

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::WallStart,
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningRight {
                opening: opening.clone(),
            },
        });
        place_pending_dimension(&mut app, DimensionAxis::Horizontal);
        assert_eq!(app.model.walls[0].dimensions.len(), 2);

        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningLeft {
                opening: opening.clone(),
            },
        });
        app.handle_view_click(ViewClick::DimensionAnchor {
            wall_index: 0,
            anchor: DimensionAnchor::OpeningRight { opening },
        });
        place_pending_dimension(&mut app, DimensionAxis::Horizontal);

        assert_eq!(app.model.walls[0].dimensions.len(), 2);
        assert!(
            app.dimension_status
                .as_deref()
                .is_some_and(|status| status.contains("overconstrain"))
        );
    }

    fn pt(x_in: f64, y_in: f64) -> Point2 {
        Point2::new(Length::from_inches(x_in), Length::from_inches(y_in))
    }

    fn add_second_level(app: &mut FramerApp) -> ElementId {
        let level = framer_core::Level::new("level-2", "Level 2", Length::from_feet(10.0));
        let id = level.id.clone();
        app.model.levels.push(level);
        app.set_active_level(id.clone());
        id
    }

    #[test]
    fn active_level_falls_back_when_the_selected_level_disappears() {
        let mut app = FramerApp::default();
        let level = add_second_level(&mut app);
        assert_eq!(app.active_level_id(), level);

        app.model.levels.retain(|candidate| candidate.id != level);
        app.rebuild();

        assert_eq!(app.active_level_id(), ElementId::new("level-1"));
    }

    #[test]
    fn active_level_controls_new_walls_rooms_and_surfaces() {
        let mut app = FramerApp::default();
        let level = add_second_level(&mut app);
        let outline = vec![
            pt(360.0, 360.0),
            pt(480.0, 360.0),
            pt(480.0, 480.0),
            pt(360.0, 480.0),
        ];
        let region = framer_core::SurfaceRegion::Polygon(outline.clone());

        app.add_wall(pt(360.0, 360.0), pt(480.0, 360.0));
        assert_eq!(app.model.walls.last().unwrap().level, level);

        app.add_room(pt(420.0, 420.0));
        assert_eq!(app.model.rooms.last().unwrap().level, level);

        app.add_ceiling(region.clone());
        assert_eq!(app.model.ceilings.last().unwrap().level, level);

        app.add_vault(&outline);
        assert!(
            app.model
                .ceilings
                .iter()
                .rev()
                .take(2)
                .all(|ceiling| ceiling.level == level),
            "both vault halves land on the active level"
        );

        app.add_floor(region);
        assert_eq!(app.model.floor_decks.last().unwrap().level, level);
    }

    #[test]
    fn active_level_controls_roof_generation() {
        let mut app = FramerApp {
            model: BuildingModel::new(),
            ..FramerApp::default()
        };
        let level = add_second_level(&mut app);

        app.add_wall(pt(0.0, 0.0), pt(240.0, 0.0));
        app.add_wall(pt(240.0, 0.0), pt(240.0, 120.0));
        app.add_wall(pt(240.0, 120.0), pt(0.0, 120.0));
        app.add_wall(pt(0.0, 120.0), pt(0.0, 0.0));
        app.add_roof(RoofForm::Gable);

        assert!(!app.model.roof_planes.is_empty());
        assert!(
            app.model
                .roof_planes
                .iter()
                .all(|plane| plane.level == level)
        );
    }

    fn draw_demo_shell_footprint(app: &mut FramerApp) {
        app.add_wall(pt(0.0, 0.0), pt(336.0, 0.0));
        app.add_wall(pt(336.0, 0.0), pt(336.0, 240.0));
        app.add_wall(pt(336.0, 240.0), pt(0.0, 240.0));
        app.add_wall(pt(0.0, 240.0), pt(0.0, 0.0));
    }

    #[test]
    fn active_level_region_tools_ignore_enclosures_on_other_levels() {
        let mut app = FramerApp::default();
        let seed = pt(168.0, 120.0);
        add_second_level(&mut app);

        let rooms_before = app.model.rooms.len();
        let ceilings_before = app.model.ceilings.len();
        let floors_before = app.model.floor_decks.len();

        app.toggle_room_tool();
        app.handle_place_room(seed);
        assert_eq!(
            app.model.rooms.len(),
            rooms_before,
            "the level-1 demo shell must not accept a level-2 room click"
        );

        app.toggle_ceiling_tool();
        app.handle_place_ceiling(seed);
        assert_eq!(
            app.model.ceilings.len(),
            ceilings_before,
            "the level-1 demo shell must not accept a level-2 ceiling click"
        );

        app.toggle_floor_tool();
        app.handle_place_floor(seed);
        assert_eq!(
            app.model.floor_decks.len(),
            floors_before,
            "the level-1 demo shell must not accept a level-2 floor click"
        );

        app.toggle_vault_tool();
        app.handle_place_vault(seed);
        assert_eq!(
            app.model.ceilings.len(),
            ceilings_before,
            "the level-1 demo shell must not accept a level-2 vault click"
        );
        assert_eq!(
            app.dimension_status.as_deref(),
            Some("No enclosed area here — close a wall loop first")
        );
    }

    #[test]
    fn surface_region_at_prefers_same_level_room_when_footprints_stack() {
        let mut app = FramerApp::default();
        let seed = pt(168.0, 120.0);

        app.add_room(seed);
        let level_1_room = match &app.selected {
            Selection::Room(id) => id.clone(),
            other => panic!("expected a level-1 room selected, got {other:?}"),
        };

        let level_2 = add_second_level(&mut app);
        draw_demo_shell_footprint(&mut app);
        app.add_room(seed);
        let level_2_room = match &app.selected {
            Selection::Room(id) => id.clone(),
            other => panic!("expected a level-2 room selected, got {other:?}"),
        };

        let region = app
            .surface_region_at(seed)
            .expect("the active level footprint is enclosed");
        assert!(
            matches!(&region, framer_core::SurfaceRegion::Room(id) if id.0 == level_2_room),
            "stacked footprints should attach to the active-level room, got {region:?}"
        );
        assert_ne!(level_1_room, level_2_room);
        assert_eq!(app.model.rooms.last().unwrap().level, level_2);
    }

    #[test]
    fn draw_wall_enclosure_delta_is_active_level_scoped() {
        let mut app = FramerApp::default();
        let level_2 = add_second_level(&mut app);

        app.toggle_draw_wall_tool();
        assert!(app.draw_wall_tool.active);
        click_wall_point(&mut app, 0.0, 0.0);
        click_wall_point(&mut app, 336.0, 0.0);
        click_wall_point(&mut app, 336.0, 240.0);
        click_wall_point(&mut app, 0.0, 240.0);
        assert!(app.draw_wall_tool.active);

        click_wall_point(&mut app, 0.0, 0.0);

        assert!(!app.draw_wall_tool.active);
        assert_eq!(
            framer_core::enclosed_room_count_on_level(&app.model, &level_2),
            1
        );
        assert_eq!(
            app.dimension_status.as_deref(),
            Some("Room enclosed — draw-wall tool off")
        );
        let level_1_walls = ["wall-front", "wall-right", "wall-back", "wall-left"];
        assert!(
            app.model.wall_joins.iter().all(|join| {
                let first_is_level_1 = level_1_walls.contains(&join.first_wall.0.as_str());
                let second_is_level_1 = level_1_walls.contains(&join.second_wall.0.as_str());
                first_is_level_1 == second_is_level_1
            }),
            "new level-2 walls must not auto-join to the pre-existing level-1 shell"
        );
    }

    /// A draw-wall session over an empty model. Starting empty avoids the demo
    /// shell's pre-existing room (1 bounded face) skewing the enclosure delta.
    /// `toggle_draw_wall_tool` both activates the tool and enters Design mode,
    /// which `add_wall` requires.
    fn empty_draw_wall_app() -> FramerApp {
        let mut app = FramerApp {
            model: BuildingModel::new(),
            ..FramerApp::default()
        };
        app.toggle_draw_wall_tool();
        assert!(app.draw_wall_tool.active);
        app
    }

    fn click_wall_point(app: &mut FramerApp, x_in: f64, y_in: f64) {
        app.handle_view_click(ViewClick::DrawWallPoint {
            point: pt(x_in, y_in),
        });
    }

    #[test]
    fn closing_a_rectangle_exits_the_draw_wall_tool() {
        let mut app = empty_draw_wall_app();

        // Three corners of a rectangle: the run stays active, chaining segments.
        click_wall_point(&mut app, 0.0, 0.0);
        click_wall_point(&mut app, 120.0, 0.0);
        click_wall_point(&mut app, 120.0, 96.0);
        click_wall_point(&mut app, 0.0, 96.0);
        assert!(app.draw_wall_tool.active);
        assert_eq!(app.draw_wall_tool.start, Some(pt(0.0, 96.0)));
        assert_eq!(app.model.walls.len(), 3);

        // Closing back onto the start encloses the room: tool turns itself off.
        click_wall_point(&mut app, 0.0, 0.0);

        assert!(!app.draw_wall_tool.active);
        assert_eq!(app.draw_wall_tool.start, None);
        assert_eq!(app.draw_wall_tool.previous_snap, None);
        assert_eq!(app.model.walls.len(), 4);
        assert_eq!(framer_core::enclosed_room_count(&app.model), 1);
        assert_eq!(
            app.dimension_status.as_deref(),
            Some("Room enclosed — draw-wall tool off")
        );
    }

    #[test]
    fn open_wall_chain_keeps_the_draw_wall_tool_active() {
        let mut app = empty_draw_wall_app();

        // An open U: three segments, never closed.
        click_wall_point(&mut app, 0.0, 0.0);
        click_wall_point(&mut app, 120.0, 0.0);
        click_wall_point(&mut app, 120.0, 96.0);
        click_wall_point(&mut app, 0.0, 96.0);

        assert!(app.draw_wall_tool.active);
        assert_eq!(app.draw_wall_tool.start, Some(pt(0.0, 96.0)));
        assert_eq!(app.model.walls.len(), 3);
        assert_eq!(framer_core::enclosed_room_count(&app.model), 0);
    }

    #[test]
    fn repeated_point_is_a_noop_and_keeps_the_run_going() {
        let mut app = empty_draw_wall_app();

        click_wall_point(&mut app, 0.0, 0.0);
        click_wall_point(&mut app, 120.0, 0.0);
        // Clicking the same point again is a zero-length no-op, not a closure.
        click_wall_point(&mut app, 120.0, 0.0);

        assert!(app.draw_wall_tool.active);
        assert_eq!(app.draw_wall_tool.start, Some(pt(120.0, 0.0)));
        assert_eq!(app.model.walls.len(), 1);
    }

    #[test]
    fn splitting_a_room_with_a_partition_exits_the_draw_wall_tool() {
        let mut app = empty_draw_wall_app();

        // Draw and close a rectangle; the tool auto-exits on the closing click.
        click_wall_point(&mut app, 0.0, 0.0);
        click_wall_point(&mut app, 120.0, 0.0);
        click_wall_point(&mut app, 120.0, 96.0);
        click_wall_point(&mut app, 0.0, 96.0);
        click_wall_point(&mut app, 0.0, 0.0);
        assert!(!app.draw_wall_tool.active);
        assert_eq!(framer_core::enclosed_room_count(&app.model), 1);

        // Re-arm the tool and run a partition across the room. Its endpoints land
        // mid-span on the top and bottom walls (a Tee at each end), dividing the
        // room into two — another enclosure, so the tool exits again.
        app.toggle_draw_wall_tool();
        assert!(app.draw_wall_tool.active);
        click_wall_point(&mut app, 60.0, 0.0);
        click_wall_point(&mut app, 60.0, 96.0);

        assert!(!app.draw_wall_tool.active);
        assert_eq!(app.draw_wall_tool.start, None);
        assert_eq!(framer_core::enclosed_room_count(&app.model), 2);
        assert_eq!(
            app.dimension_status.as_deref(),
            Some("Room enclosed — draw-wall tool off")
        );
    }
}
