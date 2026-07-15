//! Session-local tiled viewport topology and app-local named layout presets.
//!
//! Runtime pane identities are intentionally distinct from persisted preset
//! identities. Applying a preset always allocates fresh monotonic [`PaneId`]s;
//! only view configuration and a sanitized 3D camera pose cross the storage
//! boundary. Live render state and project-dependent 2D cameras never do.

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
use std::fmt;

use eframe::Storage;
use framer_render::math::Vec3;
use serde::{Deserialize, Serialize};

use super::camera_3d::{
    DOLLY_MAX, DOLLY_MIN, PAN_MAX_RADII, View3dState, ZOOM_MAX_3D, ZOOM_MIN_3D,
};
use crate::app::ViewportMode;

pub(in crate::app) const MAX_LAYOUT_PANES: usize = 16;
pub(in crate::app) const MAX_LAYOUT_DEPTH: usize = 8;
pub(in crate::app) const MAX_USER_PRESETS: usize = 32;
pub(in crate::app) const MAX_PRESET_NAME_SCALARS: usize = 64;
pub(in crate::app) const VIEWPORT_PRESETS_STORAGE_KEY: &str = "framer.viewport-layout-presets.v1";

const PRESET_STORAGE_VERSION: u32 = 1;
const MAX_PRESET_STORAGE_BYTES: usize = 512 * 1024;
const MIN_SPLIT_RATIO: f32 = 0.1;
const MAX_SPLIT_RATIO: f32 = 0.9;
// View-cube Top/Bottom snaps intentionally reach the poles exactly. Orbit input
// keeps its own small epsilon clamp, but preset snapshots must preserve a
// deliberate canonical top/bottom view.
const MIN_PITCH: f32 = -FRAC_PI_2;
const MAX_PITCH: f32 = FRAC_PI_2;

/// Stable session identity for input, native-window, render-job, and GPU state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::app) struct PaneId(u64);

impl PaneId {
    pub(in crate::app) fn get(self) -> u64 {
        self.0
    }
}

/// Session-wide monotonic pane identity source.
///
/// Keep one allocator while replacing layouts so applying a preset cannot reuse
/// the identity of a closed pane and accidentally inherit its native/GPU state.
#[derive(Debug, Clone)]
pub(in crate::app) struct PaneIdGenerator {
    next: u64,
}

impl Default for PaneIdGenerator {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl PaneIdGenerator {
    pub(in crate::app) fn allocate(&mut self) -> Result<PaneId, LayoutError> {
        let following = self
            .next
            .checked_add(1)
            .ok_or(LayoutError::PaneIdExhausted)?;
        let id = PaneId(self.next);
        self.next = following;
        Ok(id)
    }
}

/// Direction in which a split allocates its two children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum SplitAxis {
    /// First child on the left, second child on the right.
    Horizontal,
    /// First child above, second child below.
    Vertical,
}

/// Child selector used to address a split without giving transient split nodes IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::app) enum SplitSide {
    First,
    Second,
}

/// Persistable subset of a live 3D/Render camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::app) struct View3dPose {
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pan: Vec3,
    dolly: f32,
}

impl Default for View3dPose {
    fn default() -> Self {
        Self::from_view_state(&View3dState::default())
    }
}

impl View3dPose {
    /// Snapshot and sanitize live camera state for an explicit preset save.
    pub(in crate::app) fn from_view_state(view: &View3dState) -> Self {
        let fallback = View3dState::default();
        let yaw = if view.yaw.is_finite() {
            normalize_yaw(view.yaw)
        } else {
            fallback.yaw
        };
        let pitch = finite_or(view.pitch, fallback.pitch).clamp(MIN_PITCH, MAX_PITCH);
        let zoom = finite_or(view.zoom, fallback.zoom).clamp(ZOOM_MIN_3D, ZOOM_MAX_3D);
        let dolly = finite_or(view.dolly, fallback.dolly).clamp(DOLLY_MIN, DOLLY_MAX);
        let pan = sanitize_pan(view.pan);
        Self {
            yaw,
            pitch,
            zoom,
            pan,
            dolly,
        }
    }

    /// Construct a canonical bounded pose. Used by typed built-in presets.
    pub(in crate::app) fn canonical(yaw: f32, pitch: f32) -> Self {
        let view = View3dState {
            yaw,
            pitch,
            ..View3dState::default()
        };
        Self::from_view_state(&view)
    }

    /// Recreate live presentation state without carrying render accumulation.
    pub(in crate::app) fn to_view_state(self) -> View3dState {
        View3dState {
            yaw: self.yaw,
            pitch: self.pitch,
            zoom: self.zoom,
            pan: self.pan,
            dolly: self.dolly,
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn yaw(self) -> f32 {
        self.yaw
    }

    #[cfg(test)]
    pub(in crate::app) fn pitch(self) -> f32 {
        self.pitch
    }

    #[cfg(test)]
    pub(in crate::app) fn zoom(self) -> f32 {
        self.zoom
    }

    #[cfg(test)]
    pub(in crate::app) fn pan(self) -> Vec3 {
        self.pan
    }

    #[cfg(test)]
    pub(in crate::app) fn dolly(self) -> f32 {
        self.dolly
    }

    fn try_new(yaw: f32, pitch: f32, zoom: f32, pan: Vec3, dolly: f32) -> Option<Self> {
        let finite = yaw.is_finite()
            && pitch.is_finite()
            && zoom.is_finite()
            && pan.x.is_finite()
            && pan.y.is_finite()
            && pan.z.is_finite()
            && dolly.is_finite();
        let bounded = (-PI..=PI).contains(&yaw)
            && (MIN_PITCH..=MAX_PITCH).contains(&pitch)
            && (ZOOM_MIN_3D..=ZOOM_MAX_3D).contains(&zoom)
            && pan.length() <= PAN_MAX_RADII
            && (DOLLY_MIN..=DOLLY_MAX).contains(&dolly);
        (finite && bounded).then_some(Self {
            yaw,
            pitch,
            zoom,
            pan,
            dolly,
        })
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn normalize_yaw(yaw: f32) -> f32 {
    if (-PI..=PI).contains(&yaw) {
        // Preserve already-canonical values bit-for-bit so an explicit preset
        // save/apply is idempotent and a duplicated pane starts at the exact
        // same angle.
        yaw
    } else {
        (yaw + PI).rem_euclid(TAU) - PI
    }
}

fn sanitize_pan(pan: Vec3) -> Vec3 {
    if !pan.x.is_finite() || !pan.y.is_finite() || !pan.z.is_finite() {
        return Vec3::ZERO;
    }
    let length = pan.length();
    if !length.is_finite() {
        return Vec3::ZERO;
    }
    if length > PAN_MAX_RADII {
        pan * (PAN_MAX_RADII / length)
    } else {
        pan
    }
}

/// Serializable pane-level configuration. Runtime cameras and render resources
/// are created later from this seed and remain independent per pane.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct PaneConfig {
    mode: ViewportMode,
    popped_out: bool,
    pose_3d: View3dPose,
}

impl PaneConfig {
    pub(in crate::app) fn new(mode: ViewportMode) -> Self {
        Self {
            mode,
            popped_out: false,
            pose_3d: View3dPose::default(),
        }
    }

    pub(in crate::app) fn with_pose(mut self, pose: View3dPose) -> Self {
        self.pose_3d = pose;
        self
    }

    pub(in crate::app) fn mode(&self) -> ViewportMode {
        self.mode
    }

    pub(in crate::app) fn set_mode(&mut self, mode: ViewportMode) {
        self.mode = mode;
    }

    pub(in crate::app) fn is_popped_out(&self) -> bool {
        self.popped_out
    }

    pub(in crate::app) fn set_popped_out(&mut self, popped_out: bool) {
        self.popped_out = popped_out;
    }

    pub(in crate::app) fn pose_3d(&self) -> View3dPose {
        self.pose_3d
    }

    pub(in crate::app) fn set_pose_3d(&mut self, view: &View3dState) {
        self.pose_3d = View3dPose::from_view_state(view);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct ViewportPane {
    id: PaneId,
    config: PaneConfig,
}

impl ViewportPane {
    pub(in crate::app) fn id(&self) -> PaneId {
        self.id
    }

    pub(in crate::app) fn config(&self) -> &PaneConfig {
        &self.config
    }

    pub(in crate::app) fn config_mut(&mut self) -> &mut PaneConfig {
        &mut self.config
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum LayoutNode {
    Pane(ViewportPane),
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<Self>,
        second: Box<Self>,
    },
}

impl LayoutNode {
    pub(in crate::app) fn pane_count(&self) -> usize {
        match self {
            Self::Pane(_) => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn depth(&self) -> usize {
        match self {
            Self::Pane(_) => 1,
            Self::Split { first, second, .. } => 1 + first.depth().max(second.depth()),
        }
    }

    fn pane(&self, id: PaneId) -> Option<&ViewportPane> {
        match self {
            Self::Pane(pane) => (pane.id == id).then_some(pane),
            Self::Split { first, second, .. } => first.pane(id).or_else(|| second.pane(id)),
        }
    }

    fn pane_mut(&mut self, id: PaneId) -> Option<&mut ViewportPane> {
        match self {
            Self::Pane(pane) => (pane.id == id).then_some(pane),
            Self::Split { first, second, .. } => first.pane_mut(id).or_else(|| second.pane_mut(id)),
        }
    }

    fn leaf_depth(&self, id: PaneId, depth: usize) -> Option<usize> {
        match self {
            Self::Pane(pane) => (pane.id == id).then_some(depth),
            Self::Split { first, second, .. } => first
                .leaf_depth(id, depth + 1)
                .or_else(|| second.leaf_depth(id, depth + 1)),
        }
    }

    fn collect_ids(&self, output: &mut Vec<PaneId>) {
        match self {
            Self::Pane(pane) => output.push(pane.id),
            Self::Split { first, second, .. } => {
                first.collect_ids(output);
                second.collect_ids(output);
            }
        }
    }

    fn split_leaf(
        &mut self,
        target: PaneId,
        axis: SplitAxis,
        ratio: f32,
        pane: ViewportPane,
    ) -> bool {
        match self {
            Self::Pane(existing) if existing.id == target => {
                let existing = existing.clone();
                *self = Self::Split {
                    axis,
                    ratio,
                    first: Box::new(Self::Pane(existing)),
                    second: Box::new(Self::Pane(pane)),
                };
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.split_leaf(target, axis, ratio, pane.clone())
                    || second.split_leaf(target, axis, ratio, pane)
            }
        }
    }

    fn remove_leaf(&mut self, target: PaneId) -> bool {
        let Self::Split { first, second, .. } = self else {
            return false;
        };
        if matches!(first.as_ref(), Self::Pane(pane) if pane.id == target) {
            *self = (**second).clone();
            return true;
        }
        if matches!(second.as_ref(), Self::Pane(pane) if pane.id == target) {
            *self = (**first).clone();
            return true;
        }
        first.remove_leaf(target) || second.remove_leaf(target)
    }

    fn split_at_path_mut(&mut self, path: &[SplitSide]) -> Option<&mut Self> {
        let mut node = self;
        for side in path {
            node = match (node, side) {
                (Self::Split { first, .. }, SplitSide::First) => first,
                (Self::Split { second, .. }, SplitSide::Second) => second,
                (Self::Pane(_), _) => return None,
            };
        }
        matches!(node, Self::Split { .. }).then_some(node)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct ViewportLayout {
    root: LayoutNode,
    active: PaneId,
}

impl ViewportLayout {
    pub(in crate::app) fn focus(
        ids: &mut PaneIdGenerator,
        config: PaneConfig,
    ) -> Result<Self, LayoutError> {
        let id = ids.allocate()?;
        Ok(Self {
            root: LayoutNode::Pane(ViewportPane { id, config }),
            active: id,
        })
    }

    pub(in crate::app) fn root(&self) -> &LayoutNode {
        &self.root
    }

    pub(in crate::app) fn active_id(&self) -> PaneId {
        self.active
    }

    pub(in crate::app) fn active(&self) -> &ViewportPane {
        self.root
            .pane(self.active)
            .expect("ViewportLayout invariant: active pane is a leaf")
    }

    pub(in crate::app) fn active_mut(&mut self) -> &mut ViewportPane {
        self.root
            .pane_mut(self.active)
            .expect("ViewportLayout invariant: active pane is a leaf")
    }

    pub(in crate::app) fn pane(&self, id: PaneId) -> Option<&ViewportPane> {
        self.root.pane(id)
    }

    pub(in crate::app) fn pane_mut(&mut self, id: PaneId) -> Option<&mut ViewportPane> {
        self.root.pane_mut(id)
    }

    pub(in crate::app) fn pane_count(&self) -> usize {
        self.root.pane_count()
    }

    pub(in crate::app) fn pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::with_capacity(self.pane_count());
        self.root.collect_ids(&mut ids);
        ids
    }

    pub(in crate::app) fn set_active(&mut self, id: PaneId) -> Result<(), LayoutError> {
        if self.root.pane(id).is_none() {
            return Err(LayoutError::PaneNotFound(id));
        }
        self.active = id;
        Ok(())
    }

    pub(in crate::app) fn split(
        &mut self,
        target: PaneId,
        axis: SplitAxis,
        ratio: f32,
        config: PaneConfig,
        ids: &mut PaneIdGenerator,
    ) -> Result<PaneId, LayoutError> {
        self.ensure_can_split(target)?;
        let id = ids.allocate()?;
        let inserted = self.root.split_leaf(
            target,
            axis,
            clamp_split_ratio(ratio),
            ViewportPane { id, config },
        );
        debug_assert!(inserted);
        self.active = id;
        Ok(id)
    }

    pub(in crate::app) fn duplicate(
        &mut self,
        target: PaneId,
        axis: SplitAxis,
        ratio: f32,
        ids: &mut PaneIdGenerator,
    ) -> Result<PaneId, LayoutError> {
        let config = self
            .pane(target)
            .ok_or(LayoutError::PaneNotFound(target))?
            .config
            .clone();
        self.split(target, axis, ratio, config, ids)
    }

    pub(in crate::app) fn remove(&mut self, target: PaneId) -> Result<(), LayoutError> {
        if self.pane_count() == 1 {
            return if self.root.pane(target).is_some() {
                Err(LayoutError::CannotRemoveLastPane)
            } else {
                Err(LayoutError::PaneNotFound(target))
            };
        }
        let ordered = self.pane_ids();
        let index = ordered
            .iter()
            .position(|id| *id == target)
            .ok_or(LayoutError::PaneNotFound(target))?;
        let fallback = ordered
            .get(index + 1)
            .copied()
            .or_else(|| {
                index
                    .checked_sub(1)
                    .and_then(|previous| ordered.get(previous).copied())
            })
            .expect("more than one pane has a removal fallback");
        let removed = self.root.remove_leaf(target);
        debug_assert!(removed);
        if self.active == target {
            self.active = fallback;
        }
        Ok(())
    }

    pub(in crate::app) fn set_split_ratio(
        &mut self,
        path: &[SplitSide],
        ratio: f32,
    ) -> Result<(), LayoutError> {
        let node = self
            .root
            .split_at_path_mut(path)
            .ok_or(LayoutError::SplitNotFound)?;
        let LayoutNode::Split { ratio: current, .. } = node else {
            unreachable!();
        };
        *current = clamp_split_ratio(ratio);
        Ok(())
    }

    fn ensure_can_split(&self, target: PaneId) -> Result<(), LayoutError> {
        if self.pane_count() >= MAX_LAYOUT_PANES {
            return Err(LayoutError::PaneLimitReached);
        }
        let depth = self
            .root
            .leaf_depth(target, 1)
            .ok_or(LayoutError::PaneNotFound(target))?;
        if depth >= MAX_LAYOUT_DEPTH {
            return Err(LayoutError::DepthLimitReached);
        }
        Ok(())
    }
}

fn clamp_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
    } else {
        0.5
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum BuiltInPreset {
    Focus,
    PlanAnd3d,
    DesignStudy,
    FourUp,
    DesignAndRender,
}

impl BuiltInPreset {
    pub(in crate::app) const ALL: [Self; 5] = [
        Self::Focus,
        Self::PlanAnd3d,
        Self::DesignStudy,
        Self::FourUp,
        Self::DesignAndRender,
    ];

    pub(in crate::app) fn name(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::PlanAnd3d => "Plan + 3D",
            Self::DesignStudy => "Design Study",
            Self::FourUp => "Four Up",
            Self::DesignAndRender => "Design + Render",
        }
    }

    pub(in crate::app) fn instantiate(
        self,
        current: &ViewportLayout,
        ids: &mut PaneIdGenerator,
    ) -> Result<ViewportLayout, LayoutError> {
        if self == Self::Focus {
            let mut config = current.active().config.clone();
            config.popped_out = false;
            return ViewportLayout::focus(ids, config);
        }

        let specs: Vec<(ViewportMode, View3dPose)> = match self {
            Self::Focus => unreachable!(),
            Self::PlanAnd3d => vec![
                (ViewportMode::Plan, View3dPose::default()),
                (ViewportMode::Axonometric, View3dPose::default()),
            ],
            Self::DesignStudy => vec![
                (ViewportMode::Plan, View3dPose::default()),
                (ViewportMode::Elevation, View3dPose::default()),
                (ViewportMode::Axonometric, View3dPose::default()),
            ],
            Self::FourUp => vec![
                (ViewportMode::Plan, View3dPose::default()),
                (ViewportMode::Elevation, View3dPose::default()),
                (
                    ViewportMode::Axonometric,
                    View3dPose::canonical(-FRAC_PI_4, 0.55),
                ),
                (
                    ViewportMode::Axonometric,
                    View3dPose::canonical(FRAC_PI_4, 0.42),
                ),
            ],
            Self::DesignAndRender => vec![
                (ViewportMode::Axonometric, View3dPose::default()),
                (ViewportMode::Render, View3dPose::default()),
            ],
        };
        let mut panes = Vec::with_capacity(specs.len());
        for (mode, pose) in specs {
            panes.push(ViewportPane {
                id: ids.allocate()?,
                config: PaneConfig::new(mode).with_pose(pose),
            });
        }
        let active = panes[0].id;
        let root = match self {
            Self::PlanAnd3d | Self::DesignAndRender => pair(
                SplitAxis::Horizontal,
                0.5,
                LayoutNode::Pane(panes.remove(0)),
                LayoutNode::Pane(panes.remove(0)),
            ),
            Self::DesignStudy => {
                let plan = LayoutNode::Pane(panes.remove(0));
                let elevation = LayoutNode::Pane(panes.remove(0));
                let view_3d = LayoutNode::Pane(panes.remove(0));
                pair(
                    SplitAxis::Horizontal,
                    0.5,
                    plan,
                    pair(SplitAxis::Vertical, 0.5, elevation, view_3d),
                )
            }
            Self::FourUp => {
                let top_left = LayoutNode::Pane(panes.remove(0));
                let top_right = LayoutNode::Pane(panes.remove(0));
                let bottom_left = LayoutNode::Pane(panes.remove(0));
                let bottom_right = LayoutNode::Pane(panes.remove(0));
                pair(
                    SplitAxis::Vertical,
                    0.5,
                    pair(SplitAxis::Horizontal, 0.5, top_left, top_right),
                    pair(SplitAxis::Horizontal, 0.5, bottom_left, bottom_right),
                )
            }
            Self::Focus => unreachable!(),
        };
        Ok(ViewportLayout { root, active })
    }
}

fn pair(axis: SplitAxis, ratio: f32, first: LayoutNode, second: LayoutNode) -> LayoutNode {
    LayoutNode::Split {
        axis,
        ratio,
        first: Box::new(first),
        second: Box::new(second),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum LayoutError {
    PaneNotFound(PaneId),
    SplitNotFound,
    CannotRemoveLastPane,
    PaneLimitReached,
    DepthLimitReached,
    PaneIdExhausted,
    InvalidPresetName,
    PresetLimitReached,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PaneNotFound(id) => write!(formatter, "viewport pane {} was not found", id.get()),
            Self::SplitNotFound => formatter.write_str("viewport split was not found"),
            Self::CannotRemoveLastPane => {
                formatter.write_str("the last viewport pane cannot close")
            }
            Self::PaneLimitReached => write!(
                formatter,
                "a layout supports at most {MAX_LAYOUT_PANES} panes"
            ),
            Self::DepthLimitReached => write!(
                formatter,
                "a layout supports at most {MAX_LAYOUT_DEPTH} nested levels"
            ),
            Self::PaneIdExhausted => formatter.write_str("viewport pane identities are exhausted"),
            Self::InvalidPresetName => write!(
                formatter,
                "preset names must contain 1 to {MAX_PRESET_NAME_SCALARS} characters"
            ),
            Self::PresetLimitReached => write!(
                formatter,
                "at most {MAX_USER_PRESETS} custom presets may be saved"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PresetNode {
    Pane {
        local_id: u32,
        config: PaneConfig,
    },
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<Self>,
        second: Box<Self>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct UserPreset {
    name: String,
    root: PresetNode,
    active: u32,
}

impl UserPreset {
    pub(in crate::app) fn name(&self) -> &str {
        &self.name
    }

    pub(in crate::app) fn instantiate(
        &self,
        ids: &mut PaneIdGenerator,
    ) -> Result<ViewportLayout, LayoutError> {
        let mut active = None;
        let root = instantiate_preset_node(&self.root, self.active, ids, &mut active)?;
        Ok(ViewportLayout {
            root,
            active: active.expect("validated preset active pane exists"),
        })
    }

    fn capture(name: String, layout: &ViewportLayout) -> Self {
        let mut next = 1;
        let mut active = None;
        let root = capture_node(&layout.root, layout.active, &mut next, &mut active);
        Self {
            name,
            root,
            active: active.expect("layout active pane exists"),
        }
    }
}

fn capture_node(
    node: &LayoutNode,
    active_id: PaneId,
    next: &mut u32,
    active: &mut Option<u32>,
) -> PresetNode {
    match node {
        LayoutNode::Pane(pane) => {
            let local_id = *next;
            *next += 1;
            if pane.id == active_id {
                *active = Some(local_id);
            }
            PresetNode::Pane {
                local_id,
                config: pane.config.clone(),
            }
        }
        LayoutNode::Split {
            axis,
            ratio,
            first,
            second,
        } => PresetNode::Split {
            axis: *axis,
            ratio: *ratio,
            first: Box::new(capture_node(first, active_id, next, active)),
            second: Box::new(capture_node(second, active_id, next, active)),
        },
    }
}

fn instantiate_preset_node(
    node: &PresetNode,
    active_local: u32,
    ids: &mut PaneIdGenerator,
    active: &mut Option<PaneId>,
) -> Result<LayoutNode, LayoutError> {
    Ok(match node {
        PresetNode::Pane { local_id, config } => {
            let id = ids.allocate()?;
            if *local_id == active_local {
                *active = Some(id);
            }
            LayoutNode::Pane(ViewportPane {
                id,
                config: config.clone(),
            })
        }
        PresetNode::Split {
            axis,
            ratio,
            first,
            second,
        } => pair(
            *axis,
            *ratio,
            instantiate_preset_node(first, active_local, ids, active)?,
            instantiate_preset_node(second, active_local, ids, active)?,
        ),
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(in crate::app) struct PresetCatalog {
    presets: Vec<UserPreset>,
}

impl PresetCatalog {
    pub(in crate::app) fn load(storage: Option<&dyn Storage>) -> Self {
        let Some(encoded) = storage
            .and_then(|storage| storage.get_string(VIEWPORT_PRESETS_STORAGE_KEY))
            .filter(|encoded| encoded.len() <= MAX_PRESET_STORAGE_BYTES)
        else {
            return Self::default();
        };
        let Ok(dto) = ron::from_str::<RawPresetCatalogDto>(&encoded) else {
            return Self::default();
        };
        Self::from_raw_dto(dto)
    }

    pub(in crate::app) fn save(&self, storage: &mut dyn Storage) {
        eframe::set_value(storage, VIEWPORT_PRESETS_STORAGE_KEY, &self.to_dto());
    }

    pub(in crate::app) fn presets(&self) -> &[UserPreset] {
        &self.presets
    }

    pub(in crate::app) fn upsert(
        &mut self,
        name: &str,
        layout: &ViewportLayout,
    ) -> Result<&UserPreset, LayoutError> {
        let name = valid_name(name).ok_or(LayoutError::InvalidPresetName)?;
        if let Some(index) = self.presets.iter().position(|preset| preset.name == name) {
            self.presets[index] = UserPreset::capture(name, layout);
            return Ok(&self.presets[index]);
        }
        if self.presets.len() >= MAX_USER_PRESETS {
            return Err(LayoutError::PresetLimitReached);
        }
        self.presets.push(UserPreset::capture(name, layout));
        Ok(self.presets.last().expect("preset was just inserted"))
    }

    pub(in crate::app) fn delete(&mut self, name: &str) -> bool {
        let Some(name) = valid_name(name) else {
            return false;
        };
        let Some(index) = self.presets.iter().position(|preset| preset.name == name) else {
            return false;
        };
        self.presets.remove(index);
        true
    }

    #[cfg(test)]
    fn from_dto(dto: PresetCatalogDto) -> Self {
        if dto.version != PRESET_STORAGE_VERSION {
            return Self::default();
        }
        Self::from_entries(dto.presets)
    }

    fn from_raw_dto(dto: RawPresetCatalogDto) -> Self {
        if dto.version != PRESET_STORAGE_VERSION {
            return Self::default();
        }
        Self::from_entries(
            dto.presets
                .into_iter()
                .filter_map(|value| value.into_rust::<UserPresetDto>().ok()),
        )
    }

    fn from_entries(entries: impl IntoIterator<Item = UserPresetDto>) -> Self {
        let mut presets = Vec::new();
        let mut names = BTreeSet::new();
        for dto in entries {
            if presets.len() == MAX_USER_PRESETS {
                break;
            }
            let Some(preset) = UserPreset::from_dto(dto) else {
                continue;
            };
            if names.insert(preset.name.clone()) {
                presets.push(preset);
            }
        }
        Self { presets }
    }

    fn to_dto(&self) -> PresetCatalogDto {
        PresetCatalogDto {
            version: PRESET_STORAGE_VERSION,
            presets: self.presets.iter().map(UserPreset::to_dto).collect(),
        }
    }
}

fn valid_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    (!trimmed.is_empty() && trimmed.chars().count() <= MAX_PRESET_NAME_SCALARS)
        .then(|| trimmed.to_owned())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PresetCatalogDto {
    version: u32,
    presets: Vec<UserPresetDto>,
}

#[derive(Debug, Deserialize)]
struct RawPresetCatalogDto {
    version: u32,
    presets: Vec<ron::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserPresetDto {
    name: String,
    active_pane: u32,
    root_node: u32,
    nodes: Vec<NodeDto>,
    panes: Vec<PaneDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeDto {
    id: u32,
    kind: String,
    pane: Option<u32>,
    axis: Option<String>,
    ratio: Option<f32>,
    first: Option<u32>,
    second: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaneDto {
    id: u32,
    mode: String,
    popped_out: bool,
    pose_3d: PoseDto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct PoseDto {
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pan: [f32; 3],
    dolly: f32,
}

impl UserPreset {
    fn from_dto(dto: UserPresetDto) -> Option<Self> {
        let name = valid_name(&dto.name)?;
        if dto.panes.is_empty()
            || dto.panes.len() > MAX_LAYOUT_PANES
            || dto.nodes.is_empty()
            || dto.nodes.len() > MAX_LAYOUT_PANES * 2 - 1
            || dto.root_node == 0
            || dto.active_pane == 0
        {
            return None;
        }
        let mut panes = BTreeMap::new();
        for pane in dto.panes {
            if pane.id == 0 || panes.insert(pane.id, pane).is_some() {
                return None;
            }
        }
        let mut nodes = BTreeMap::new();
        for node in dto.nodes {
            if node.id == 0 || nodes.insert(node.id, node).is_some() {
                return None;
            }
        }
        let mut visited_nodes = BTreeSet::new();
        let mut visited_panes = BTreeSet::new();
        let root = decode_node(
            dto.root_node,
            1,
            &nodes,
            &panes,
            &mut visited_nodes,
            &mut visited_panes,
        )?;
        if visited_nodes.len() != nodes.len()
            || visited_panes.len() != panes.len()
            || !visited_panes.contains(&dto.active_pane)
        {
            return None;
        }
        Some(Self {
            name,
            root,
            active: dto.active_pane,
        })
    }

    fn to_dto(&self) -> UserPresetDto {
        let mut nodes = Vec::new();
        let mut panes = Vec::new();
        let mut next_node = 1;
        let root_node = encode_node(&self.root, &mut next_node, &mut nodes, &mut panes);
        UserPresetDto {
            name: self.name.clone(),
            active_pane: self.active,
            root_node,
            nodes,
            panes,
        }
    }
}

fn decode_node(
    id: u32,
    depth: usize,
    nodes: &BTreeMap<u32, NodeDto>,
    panes: &BTreeMap<u32, PaneDto>,
    visited_nodes: &mut BTreeSet<u32>,
    visited_panes: &mut BTreeSet<u32>,
) -> Option<PresetNode> {
    if depth > MAX_LAYOUT_DEPTH || !visited_nodes.insert(id) {
        return None;
    }
    let node = nodes.get(&id)?;
    match node.kind.as_str() {
        "pane"
            if node.axis.is_none()
                && node.ratio.is_none()
                && node.first.is_none()
                && node.second.is_none() =>
        {
            let pane_id = node.pane?;
            if !visited_panes.insert(pane_id) {
                return None;
            }
            Some(PresetNode::Pane {
                local_id: pane_id,
                config: decode_pane(panes.get(&pane_id)?)?,
            })
        }
        "split" if node.pane.is_none() => {
            let axis = match node.axis.as_deref()? {
                "horizontal" => SplitAxis::Horizontal,
                "vertical" => SplitAxis::Vertical,
                _ => return None,
            };
            let ratio = node.ratio?;
            if !ratio.is_finite() || !(MIN_SPLIT_RATIO..=MAX_SPLIT_RATIO).contains(&ratio) {
                return None;
            }
            Some(PresetNode::Split {
                axis,
                ratio,
                first: Box::new(decode_node(
                    node.first?,
                    depth + 1,
                    nodes,
                    panes,
                    visited_nodes,
                    visited_panes,
                )?),
                second: Box::new(decode_node(
                    node.second?,
                    depth + 1,
                    nodes,
                    panes,
                    visited_nodes,
                    visited_panes,
                )?),
            })
        }
        _ => None,
    }
}

fn decode_pane(pane: &PaneDto) -> Option<PaneConfig> {
    let mode = match pane.mode.as_str() {
        "plan" => ViewportMode::Plan,
        "roof_plan" => ViewportMode::RoofPlan,
        "elevation" => ViewportMode::Elevation,
        "3d" => ViewportMode::Axonometric,
        "render" => ViewportMode::Render,
        _ => return None,
    };
    let pose = View3dPose::try_new(
        pane.pose_3d.yaw,
        pane.pose_3d.pitch,
        pane.pose_3d.zoom,
        Vec3::new(
            pane.pose_3d.pan[0],
            pane.pose_3d.pan[1],
            pane.pose_3d.pan[2],
        ),
        pane.pose_3d.dolly,
    )?;
    Some(PaneConfig {
        mode,
        popped_out: pane.popped_out,
        pose_3d: pose,
    })
}

fn encode_node(
    node: &PresetNode,
    next_node: &mut u32,
    nodes: &mut Vec<NodeDto>,
    panes: &mut Vec<PaneDto>,
) -> u32 {
    let id = *next_node;
    *next_node += 1;
    match node {
        PresetNode::Pane { local_id, config } => {
            nodes.push(NodeDto {
                id,
                kind: "pane".to_owned(),
                pane: Some(*local_id),
                axis: None,
                ratio: None,
                first: None,
                second: None,
            });
            panes.push(encode_pane(*local_id, config));
        }
        PresetNode::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            // Reserve the parent position so the RON remains root-first while
            // child node IDs are assigned recursively.
            let parent_index = nodes.len();
            nodes.push(NodeDto {
                id,
                kind: "split".to_owned(),
                pane: None,
                axis: Some(axis_name(*axis).to_owned()),
                ratio: Some(*ratio),
                first: None,
                second: None,
            });
            let first_id = encode_node(first, next_node, nodes, panes);
            let second_id = encode_node(second, next_node, nodes, panes);
            nodes[parent_index].first = Some(first_id);
            nodes[parent_index].second = Some(second_id);
        }
    }
    id
}

fn encode_pane(id: u32, config: &PaneConfig) -> PaneDto {
    let pose = config.pose_3d;
    PaneDto {
        id,
        mode: mode_name(config.mode).to_owned(),
        popped_out: config.popped_out,
        pose_3d: PoseDto {
            yaw: pose.yaw,
            pitch: pose.pitch,
            zoom: pose.zoom,
            pan: [pose.pan.x, pose.pan.y, pose.pan.z],
            dolly: pose.dolly,
        },
    }
}

fn axis_name(axis: SplitAxis) -> &'static str {
    match axis {
        SplitAxis::Horizontal => "horizontal",
        SplitAxis::Vertical => "vertical",
    }
}

fn mode_name(mode: ViewportMode) -> &'static str {
    match mode {
        ViewportMode::Plan => "plan",
        ViewportMode::RoofPlan => "roof_plan",
        ViewportMode::Elevation => "elevation",
        ViewportMode::Axonometric => "3d",
        ViewportMode::Render => "render",
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

    fn default_pose_dto() -> PoseDto {
        let pose = View3dPose::default();
        PoseDto {
            yaw: pose.yaw,
            pitch: pose.pitch,
            zoom: pose.zoom,
            pan: [pose.pan.x, pose.pan.y, pose.pan.z],
            dolly: pose.dolly,
        }
    }

    fn single_pane_dto(name: &str) -> UserPresetDto {
        UserPresetDto {
            name: name.to_owned(),
            active_pane: 1,
            root_node: 1,
            nodes: vec![NodeDto {
                id: 1,
                kind: "pane".to_owned(),
                pane: Some(1),
                axis: None,
                ratio: None,
                first: None,
                second: None,
            }],
            panes: vec![PaneDto {
                id: 1,
                mode: "plan".to_owned(),
                popped_out: false,
                pose_3d: default_pose_dto(),
            }],
        }
    }

    fn store_dto(storage: &mut MemoryStorage, presets: Vec<UserPresetDto>) {
        eframe::set_value(
            storage,
            VIEWPORT_PRESETS_STORAGE_KEY,
            &PresetCatalogDto {
                version: PRESET_STORAGE_VERSION,
                presets,
            },
        );
    }

    fn focus_layout(ids: &mut PaneIdGenerator) -> ViewportLayout {
        ViewportLayout::focus(ids, PaneConfig::new(ViewportMode::Plan)).unwrap()
    }

    #[test]
    fn pane_ids_are_monotonic_across_split_remove_and_preset_apply() {
        let mut ids = PaneIdGenerator::default();
        let mut layout = focus_layout(&mut ids);
        let first = layout.active_id();
        let second = layout
            .duplicate(first, SplitAxis::Horizontal, 0.5, &mut ids)
            .unwrap();
        layout.remove(second).unwrap();

        let mut catalog = PresetCatalog::default();
        catalog.upsert("Focus copy", &layout).unwrap();
        let applied = catalog.presets()[0].instantiate(&mut ids).unwrap();

        assert_eq!(first.get(), 1);
        assert_eq!(second.get(), 2);
        assert_eq!(applied.active_id().get(), 3);
        assert!(
            applied
                .pane_ids()
                .into_iter()
                .all(|id| id != first && id != second)
        );
    }

    #[test]
    fn split_duplicate_and_ratio_updates_preserve_config_and_bounds() {
        let mut ids = PaneIdGenerator::default();
        let mut layout = focus_layout(&mut ids);
        let first = layout.active_id();
        layout.active_mut().config_mut().set_popped_out(true);
        let duplicate = layout
            .duplicate(first, SplitAxis::Horizontal, f32::INFINITY, &mut ids)
            .unwrap();

        assert_eq!(
            layout.pane(duplicate).unwrap().config(),
            layout.pane(first).unwrap().config()
        );
        let LayoutNode::Split { ratio, .. } = layout.root() else {
            panic!("duplicate must split the leaf");
        };
        assert_eq!(*ratio, 0.5);

        layout.set_split_ratio(&[], -20.0).unwrap();
        let LayoutNode::Split { ratio, .. } = layout.root() else {
            unreachable!();
        };
        assert_eq!(*ratio, MIN_SPLIT_RATIO);
        assert_eq!(
            layout.set_split_ratio(&[SplitSide::First], 0.5),
            Err(LayoutError::SplitNotFound)
        );
    }

    #[test]
    fn split_enforces_pane_and_depth_limits_without_consuming_an_id() {
        let mut ids = PaneIdGenerator::default();
        let mut balanced = focus_layout(&mut ids);
        while balanced.pane_count() < MAX_LAYOUT_PANES {
            let round = balanced.pane_ids();
            for target in round {
                if balanced.pane_count() == MAX_LAYOUT_PANES {
                    break;
                }
                balanced
                    .duplicate(target, SplitAxis::Horizontal, 0.5, &mut ids)
                    .unwrap();
            }
        }
        let target = balanced.active_id();
        assert_eq!(
            balanced.duplicate(target, SplitAxis::Horizontal, 0.5, &mut ids),
            Err(LayoutError::PaneLimitReached)
        );

        let mut depth_ids = PaneIdGenerator::default();
        let mut deep = focus_layout(&mut depth_ids);
        let mut deepest = deep.active_id();
        for _ in 1..MAX_LAYOUT_DEPTH {
            deepest = deep
                .duplicate(deepest, SplitAxis::Vertical, 0.5, &mut depth_ids)
                .unwrap();
        }
        assert_eq!(deep.root().depth(), MAX_LAYOUT_DEPTH);
        assert_eq!(
            deep.duplicate(deepest, SplitAxis::Vertical, 0.5, &mut depth_ids),
            Err(LayoutError::DepthLimitReached)
        );
    }

    #[test]
    fn remove_guards_last_pane_and_uses_depth_first_successor_then_predecessor() {
        let mut ids = PaneIdGenerator::default();
        let mut layout = focus_layout(&mut ids);
        let first = layout.active_id();
        assert_eq!(layout.remove(first), Err(LayoutError::CannotRemoveLastPane));

        let second = layout
            .duplicate(first, SplitAxis::Horizontal, 0.5, &mut ids)
            .unwrap();
        let third = layout
            .duplicate(second, SplitAxis::Vertical, 0.5, &mut ids)
            .unwrap();
        layout.set_active(second).unwrap();
        layout.remove(second).unwrap();
        assert_eq!(layout.active_id(), third, "next depth-first leaf wins");
        layout.remove(third).unwrap();
        assert_eq!(
            layout.active_id(),
            first,
            "last leaf falls back to predecessor"
        );
        assert_eq!(
            layout.set_active(PaneId(999)),
            Err(LayoutError::PaneNotFound(PaneId(999)))
        );
    }

    #[test]
    fn built_ins_have_locked_topologies_modes_and_distinct_four_up_angles() {
        let mut ids = PaneIdGenerator::default();
        let current = focus_layout(&mut ids);
        let expected = [
            (BuiltInPreset::PlanAnd3d, 2, 2),
            (BuiltInPreset::DesignStudy, 3, 3),
            (BuiltInPreset::FourUp, 4, 3),
            (BuiltInPreset::DesignAndRender, 2, 2),
        ];
        for (preset, panes, depth) in expected {
            let layout = preset.instantiate(&current, &mut ids).unwrap();
            assert_eq!(layout.pane_count(), panes, "{}", preset.name());
            assert_eq!(layout.root().depth(), depth, "{}", preset.name());
        }

        let four_up = BuiltInPreset::FourUp
            .instantiate(&current, &mut ids)
            .unwrap();
        let modes: Vec<_> = four_up
            .pane_ids()
            .into_iter()
            .map(|id| four_up.pane(id).unwrap().config().mode())
            .collect();
        assert_eq!(
            modes,
            vec![
                ViewportMode::Plan,
                ViewportMode::Elevation,
                ViewportMode::Axonometric,
                ViewportMode::Axonometric,
            ]
        );
        let angles: Vec<_> = four_up
            .pane_ids()
            .into_iter()
            .filter_map(|id| {
                let config = four_up.pane(id).unwrap().config();
                (config.mode() == ViewportMode::Axonometric).then(|| config.pose_3d().yaw())
            })
            .collect();
        assert_eq!(angles.len(), 2);
        assert_ne!(angles[0].to_bits(), angles[1].to_bits());

        let design_render = BuiltInPreset::DesignAndRender
            .instantiate(&current, &mut ids)
            .unwrap();
        let modes: Vec<_> = design_render
            .pane_ids()
            .into_iter()
            .map(|id| design_render.pane(id).unwrap().config().mode())
            .collect();
        assert_eq!(modes, vec![ViewportMode::Axonometric, ViewportMode::Render]);
    }

    #[test]
    fn focus_preserves_active_view_config_but_docks_it() {
        let mut ids = PaneIdGenerator::default();
        let mut current = focus_layout(&mut ids);
        let view = View3dState {
            yaw: 2.0,
            zoom: 7.0,
            ..View3dState::default()
        };
        current
            .active_mut()
            .config_mut()
            .set_mode(ViewportMode::Render);
        current.active_mut().config_mut().set_pose_3d(&view);
        current.active_mut().config_mut().set_popped_out(true);

        let focus = BuiltInPreset::Focus
            .instantiate(&current, &mut ids)
            .unwrap();
        assert_eq!(focus.active().config().mode(), ViewportMode::Render);
        assert_eq!(focus.active().config().pose_3d().zoom(), 7.0);
        assert!(!focus.active().config().is_popped_out());
    }

    #[test]
    fn pose_snapshot_sanitizes_every_runtime_camera_component() {
        let view = View3dState {
            yaw: 9.0 * PI,
            pitch: 99.0,
            zoom: f32::INFINITY,
            pan: Vec3::new(PAN_MAX_RADII * 4.0, 0.0, 0.0),
            dolly: -5.0,
        };
        let pose = View3dPose::from_view_state(&view);
        assert!((-PI..=PI).contains(&pose.yaw()));
        assert_eq!(pose.pitch(), MAX_PITCH);
        assert_eq!(pose.zoom(), View3dState::default().zoom);
        assert!((pose.pan().length() - PAN_MAX_RADII).abs() < 1.0e-5);
        assert_eq!(pose.dolly(), DOLLY_MIN);

        let restored = pose.to_view_state();
        assert_eq!(View3dPose::from_view_state(&restored), pose);

        let corrupt = View3dState {
            yaw: f32::NAN,
            pitch: f32::NAN,
            zoom: f32::NAN,
            pan: Vec3::new(f32::NAN, 1.0, 2.0),
            dolly: f32::NAN,
        };
        assert_eq!(View3dPose::from_view_state(&corrupt), View3dPose::default());
    }

    #[test]
    fn catalog_round_trips_through_eframe_ron_with_fresh_runtime_ids() {
        let mut ids = PaneIdGenerator::default();
        let mut layout = focus_layout(&mut ids);
        let first = layout.active_id();
        let second = layout
            .split(
                first,
                SplitAxis::Horizontal,
                0.37,
                PaneConfig::new(ViewportMode::Render),
                &mut ids,
            )
            .unwrap();
        layout
            .pane_mut(second)
            .unwrap()
            .config_mut()
            .set_popped_out(true);
        layout.set_active(first).unwrap();

        let mut catalog = PresetCatalog::default();
        catalog.upsert("  My layout  ", &layout).unwrap();
        let mut storage = MemoryStorage::default();
        catalog.save(&mut storage);
        let encoded = storage.get_string(VIEWPORT_PRESETS_STORAGE_KEY).unwrap();
        assert!(!encoded.contains("plan_view"));
        assert!(!encoded.contains("render_gpu"));

        let loaded = PresetCatalog::load(Some(&storage));
        assert_eq!(loaded.presets()[0].name(), "My layout");
        let applied = loaded.presets()[0].instantiate(&mut ids).unwrap();
        assert_eq!(applied.pane_count(), 2);
        assert_eq!(applied.active().config().mode(), ViewportMode::Plan);
        assert!(
            applied
                .pane_ids()
                .into_iter()
                .map(|id| applied.pane(id).unwrap())
                .any(|pane| pane.config().mode() == ViewportMode::Render
                    && pane.config().is_popped_out())
        );
        assert!(
            applied
                .pane_ids()
                .into_iter()
                .all(|id| id.get() > second.get())
        );
    }

    #[test]
    fn malformed_missing_and_unsupported_storage_fall_back_to_empty_catalog() {
        assert!(PresetCatalog::load(None).presets().is_empty());
        let mut storage = MemoryStorage::default();
        storage.set_string(VIEWPORT_PRESETS_STORAGE_KEY, "not valid RON (((".to_owned());
        assert!(PresetCatalog::load(Some(&storage)).presets().is_empty());

        eframe::set_value(
            &mut storage,
            VIEWPORT_PRESETS_STORAGE_KEY,
            &PresetCatalogDto {
                version: PRESET_STORAGE_VERSION + 1,
                presets: vec![single_pane_dto("Future")],
            },
        );
        assert!(PresetCatalog::load(Some(&storage)).presets().is_empty());
    }

    #[test]
    fn type_malformed_entry_does_not_discard_valid_siblings() {
        let valid_a = ron::to_string(&single_pane_dto("Valid A")).unwrap();
        let valid_b = ron::to_string(&single_pane_dto("Valid B")).unwrap();
        let mut storage = MemoryStorage::default();
        storage.set_string(
            VIEWPORT_PRESETS_STORAGE_KEY,
            format!("(version:{PRESET_STORAGE_VERSION},presets:[{valid_a},42,{valid_b}])"),
        );

        let catalog = PresetCatalog::load(Some(&storage));
        let names = catalog
            .presets()
            .iter()
            .map(UserPreset::name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Valid A", "Valid B"]);
    }

    #[test]
    fn oversized_preset_storage_is_rejected_before_catalog_decode() {
        let encoded = ron::to_string(&PresetCatalogDto {
            version: PRESET_STORAGE_VERSION,
            presets: vec![single_pane_dto("Would otherwise load")],
        })
        .unwrap();
        let mut storage = MemoryStorage::default();
        storage.set_string(
            VIEWPORT_PRESETS_STORAGE_KEY,
            format!("{encoded}{}", " ".repeat(MAX_PRESET_STORAGE_BYTES)),
        );

        assert!(PresetCatalog::load(Some(&storage)).presets().is_empty());
    }

    #[test]
    fn save_replaces_same_trimmed_name_delete_is_trimmed_and_limits_are_enforced() {
        let mut ids = PaneIdGenerator::default();
        let mut layout = focus_layout(&mut ids);
        let mut catalog = PresetCatalog::default();
        catalog.upsert(" Study ", &layout).unwrap();
        layout
            .active_mut()
            .config_mut()
            .set_mode(ViewportMode::Render);
        catalog.upsert("Study", &layout).unwrap();
        assert_eq!(catalog.presets().len(), 1);
        let applied = catalog.presets()[0].instantiate(&mut ids).unwrap();
        assert_eq!(applied.active().config().mode(), ViewportMode::Render);
        assert!(catalog.delete(" Study "));
        assert!(!catalog.delete("Study"));

        assert_eq!(
            catalog.upsert("   ", &layout),
            Err(LayoutError::InvalidPresetName)
        );
        assert_eq!(
            catalog.upsert(&"x".repeat(MAX_PRESET_NAME_SCALARS + 1), &layout),
            Err(LayoutError::InvalidPresetName)
        );
        let unicode_limit = "💡".repeat(MAX_PRESET_NAME_SCALARS);
        catalog.upsert(&unicode_limit, &layout).unwrap();

        while catalog.presets().len() < MAX_USER_PRESETS {
            let name = format!("Preset {}", catalog.presets().len());
            catalog.upsert(&name, &layout).unwrap();
        }
        assert_eq!(
            catalog.upsert("One too many", &layout),
            Err(LayoutError::PresetLimitReached)
        );
        catalog.upsert("Preset 2", &layout).unwrap();
    }

    #[test]
    fn invalid_entries_are_atomic_and_valid_siblings_survive() {
        let valid_a = single_pane_dto("Valid A");
        let valid_b = single_pane_dto("Valid B");

        let mut duplicate_pane_id = single_pane_dto("duplicate pane id");
        duplicate_pane_id
            .panes
            .push(duplicate_pane_id.panes[0].clone());

        let mut duplicate_node_id = single_pane_dto("duplicate node id");
        duplicate_node_id
            .nodes
            .push(duplicate_node_id.nodes[0].clone());

        let mut missing_active = single_pane_dto("missing active");
        missing_active.active_pane = 99;

        let mut unknown_mode = single_pane_dto("unknown mode");
        unknown_mode.panes[0].mode = "future-mode".to_owned();

        let mut bad_pose = single_pane_dto("bad pose");
        bad_pose.panes[0].pose_3d.zoom = ZOOM_MAX_3D + 1.0;

        let mut empty = single_pane_dto("empty");
        empty.nodes.clear();

        let mut disconnected = single_pane_dto("disconnected");
        disconnected.nodes.push(NodeDto {
            id: 2,
            kind: "pane".to_owned(),
            pane: Some(2),
            axis: None,
            ratio: None,
            first: None,
            second: None,
        });
        disconnected.panes.push(PaneDto {
            id: 2,
            mode: "render".to_owned(),
            popped_out: false,
            pose_3d: default_pose_dto(),
        });

        let mut storage = MemoryStorage::default();
        store_dto(
            &mut storage,
            vec![
                valid_a,
                duplicate_pane_id,
                duplicate_node_id,
                missing_active,
                unknown_mode,
                bad_pose,
                empty,
                disconnected,
                valid_b,
            ],
        );
        let catalog = PresetCatalog::load(Some(&storage));
        let names: Vec<_> = catalog.presets().iter().map(UserPreset::name).collect();
        assert_eq!(names, vec!["Valid A", "Valid B"]);
    }

    #[test]
    fn tree_validation_rejects_duplicate_leaves_bad_ratios_cycles_and_over_depth() {
        let mut duplicate_leaf = single_pane_dto("duplicate leaf");
        duplicate_leaf.root_node = 2;
        duplicate_leaf.nodes.push(NodeDto {
            id: 2,
            kind: "split".to_owned(),
            pane: None,
            axis: Some("horizontal".to_owned()),
            ratio: Some(0.5),
            first: Some(1),
            second: Some(1),
        });
        assert!(UserPreset::from_dto(duplicate_leaf).is_none());

        let mut bad_ratio = two_pane_dto("bad ratio");
        bad_ratio.nodes[0].ratio = Some(0.99);
        assert!(UserPreset::from_dto(bad_ratio).is_none());

        let mut unknown_axis = two_pane_dto("unknown axis");
        unknown_axis.nodes[0].axis = Some("diagonal".to_owned());
        assert!(UserPreset::from_dto(unknown_axis).is_none());

        let mut cycle = two_pane_dto("cycle");
        cycle.nodes[0].first = Some(1);
        assert!(UserPreset::from_dto(cycle).is_none());

        assert!(UserPreset::from_dto(deep_dto(MAX_LAYOUT_DEPTH + 1)).is_none());
        assert!(UserPreset::from_dto(over_count_dto()).is_none());
    }

    #[test]
    fn duplicate_names_keep_first_valid_entry_and_catalog_load_caps_at_32() {
        let mut duplicate = single_pane_dto(" Same ");
        duplicate.panes[0].mode = "render".to_owned();
        let mut entries = vec![single_pane_dto("Same"), duplicate];
        entries.extend((0..40).map(|index| single_pane_dto(&format!("Unique {index}"))));
        let catalog = PresetCatalog::from_dto(PresetCatalogDto {
            version: PRESET_STORAGE_VERSION,
            presets: entries,
        });
        assert_eq!(catalog.presets().len(), MAX_USER_PRESETS);
        let mut ids = PaneIdGenerator::default();
        let first = catalog.presets()[0].instantiate(&mut ids).unwrap();
        assert_eq!(first.active().config().mode(), ViewportMode::Plan);
    }

    #[test]
    fn pose_validator_rejects_non_finite_and_every_out_of_range_component() {
        let valid = default_pose_dto();
        let invalid = [
            PoseDto {
                yaw: f32::NAN,
                ..valid
            },
            PoseDto {
                yaw: PI + 0.01,
                ..valid
            },
            PoseDto {
                pitch: MAX_PITCH + 0.01,
                ..valid
            },
            PoseDto {
                zoom: ZOOM_MIN_3D - 0.01,
                ..valid
            },
            PoseDto {
                pan: [PAN_MAX_RADII + 0.01, 0.0, 0.0],
                ..valid
            },
            PoseDto {
                dolly: DOLLY_MAX + 0.01,
                ..valid
            },
        ];
        for (index, pose) in invalid.into_iter().enumerate() {
            let mut dto = single_pane_dto(&format!("invalid pose {index}"));
            dto.panes[0].pose_3d = pose;
            assert!(UserPreset::from_dto(dto).is_none(), "case {index}");
        }
    }

    fn two_pane_dto(name: &str) -> UserPresetDto {
        let mut dto = single_pane_dto(name);
        dto.root_node = 1;
        dto.nodes = vec![
            NodeDto {
                id: 1,
                kind: "split".to_owned(),
                pane: None,
                axis: Some("horizontal".to_owned()),
                ratio: Some(0.5),
                first: Some(2),
                second: Some(3),
            },
            NodeDto {
                id: 2,
                kind: "pane".to_owned(),
                pane: Some(1),
                axis: None,
                ratio: None,
                first: None,
                second: None,
            },
            NodeDto {
                id: 3,
                kind: "pane".to_owned(),
                pane: Some(2),
                axis: None,
                ratio: None,
                first: None,
                second: None,
            },
        ];
        dto.panes.push(PaneDto {
            id: 2,
            mode: "3d".to_owned(),
            popped_out: false,
            pose_3d: default_pose_dto(),
        });
        dto
    }

    fn deep_dto(depth: usize) -> UserPresetDto {
        fn build(
            depth: usize,
            target: usize,
            next_node: &mut u32,
            next_pane: &mut u32,
            nodes: &mut Vec<NodeDto>,
            panes: &mut Vec<PaneDto>,
        ) -> u32 {
            let id = *next_node;
            *next_node += 1;
            if depth == target {
                let pane = *next_pane;
                *next_pane += 1;
                nodes.push(NodeDto {
                    id,
                    kind: "pane".to_owned(),
                    pane: Some(pane),
                    axis: None,
                    ratio: None,
                    first: None,
                    second: None,
                });
                panes.push(PaneDto {
                    id: pane,
                    mode: "plan".to_owned(),
                    popped_out: false,
                    pose_3d: default_pose_dto(),
                });
                return id;
            }
            let index = nodes.len();
            nodes.push(NodeDto {
                id,
                kind: "split".to_owned(),
                pane: None,
                axis: Some("vertical".to_owned()),
                ratio: Some(0.5),
                first: None,
                second: None,
            });
            let first = build(depth + 1, target, next_node, next_pane, nodes, panes);
            let second = build(target, target, next_node, next_pane, nodes, panes);
            nodes[index].first = Some(first);
            nodes[index].second = Some(second);
            id
        }

        let mut nodes = Vec::new();
        let mut panes = Vec::new();
        let mut next_node = 1;
        let mut next_pane = 1;
        let root = build(
            1,
            depth,
            &mut next_node,
            &mut next_pane,
            &mut nodes,
            &mut panes,
        );
        UserPresetDto {
            name: format!("depth {depth}"),
            active_pane: 1,
            root_node: root,
            nodes,
            panes,
        }
    }

    fn over_count_dto() -> UserPresetDto {
        let mut dto = deep_dto(MAX_LAYOUT_DEPTH);
        dto.name = "too many panes".to_owned();
        while dto.panes.len() <= MAX_LAYOUT_PANES {
            let pane_id = dto.panes.len() as u32 + 1;
            dto.panes.push(PaneDto {
                id: pane_id,
                mode: "plan".to_owned(),
                popped_out: false,
                pose_3d: default_pose_dto(),
            });
        }
        dto
    }
}
