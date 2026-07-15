//! Cohesive session state for a tiled viewport workspace.
//!
//! The tree and preset catalog are lightweight presentation data. Every leaf
//! owns a separately locked runtime so the same handle can be painted either in
//! the root window or from an egui deferred viewport callback.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use eframe::{Storage, egui};

use super::deferred::{DeferredPaneEvent, DeferredPaneHandle, show_deferred_pane};
use super::layout::{
    BuiltInPreset, LayoutError, PaneConfig, PaneId, PaneIdGenerator, PresetCatalog, SplitAxis,
    UserPreset, ViewportLayout,
};
use super::pane::ViewportPaneRuntime;
use super::pane_view::OwnedPaneFrame;
use crate::app::ViewportMode;
use crate::app::context_menu::ContextMenuModel;

pub(in crate::app) type PaneRuntimeHandle = Arc<Mutex<ViewportPaneRuntime>>;

pub(in crate::app) struct ViewportWorkspaceState {
    pub(in crate::app) layout: ViewportLayout,
    ids: PaneIdGenerator,
    runtimes: BTreeMap<PaneId, PaneRuntimeHandle>,
    deferred: BTreeMap<PaneId, DeferredPaneHandle>,
    deferred_tx: Sender<DeferredPaneEvent>,
    deferred_rx: Receiver<DeferredPaneEvent>,
    retired_targets: Vec<PaneId>,
    pub(in crate::app) presets: PresetCatalog,
    pub(in crate::app) save_preset_open: bool,
    pub(in crate::app) preset_name: String,
    pub(in crate::app) storage_dirty: bool,
}

impl ViewportWorkspaceState {
    pub(in crate::app) fn new(mode: ViewportMode, storage: Option<&dyn Storage>) -> Self {
        let mut ids = PaneIdGenerator::default();
        let layout = ViewportLayout::focus(&mut ids, PaneConfig::new(mode))
            .expect("the initial viewport pane ID must be available");
        let (deferred_tx, deferred_rx) = mpsc::channel();
        let mut state = Self {
            layout,
            ids,
            runtimes: BTreeMap::new(),
            deferred: BTreeMap::new(),
            deferred_tx,
            deferred_rx,
            retired_targets: Vec::new(),
            presets: PresetCatalog::load(storage),
            save_preset_open: false,
            preset_name: String::new(),
            storage_dirty: false,
        };
        state.reconcile_runtimes();
        state
    }

    pub(in crate::app) fn active_id(&self) -> PaneId {
        self.layout.active_id()
    }

    pub(in crate::app) fn active_mode(&self) -> ViewportMode {
        self.layout.active().config().mode()
    }

    pub(in crate::app) fn runtime(&self, id: PaneId) -> Option<PaneRuntimeHandle> {
        self.runtimes.get(&id).cloned()
    }

    pub(in crate::app) fn active_runtime(&self) -> PaneRuntimeHandle {
        self.runtime(self.active_id())
            .expect("active viewport pane must have a runtime")
    }

    pub(in crate::app) fn lock_active(&self) -> MutexGuard<'_, ViewportPaneRuntime> {
        self.runtimes
            .get(&self.active_id())
            .expect("active viewport pane must have a runtime")
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(in crate::app) fn set_active(&mut self, id: PaneId) -> Result<(), LayoutError> {
        self.layout.set_active(id)
    }

    pub(in crate::app) fn set_active_mode(&mut self, mode: ViewportMode) {
        self.layout.active_mut().config_mut().set_mode(mode);
    }

    pub(in crate::app) fn set_mode(
        &mut self,
        id: PaneId,
        mode: ViewportMode,
    ) -> Result<(), LayoutError> {
        self.layout
            .pane_mut(id)
            .ok_or(LayoutError::PaneNotFound(id))?
            .config_mut()
            .set_mode(mode);
        if let Some(handle) = self.deferred.get(&id) {
            handle.set_mode(mode);
        }
        Ok(())
    }

    pub(in crate::app) fn set_popped_out(
        &mut self,
        id: PaneId,
        popped_out: bool,
    ) -> Result<(), LayoutError> {
        self.layout
            .pane_mut(id)
            .ok_or(LayoutError::PaneNotFound(id))?
            .config_mut()
            .set_popped_out(popped_out);
        self.reconcile_deferred();
        Ok(())
    }

    pub(in crate::app) fn split(
        &mut self,
        id: PaneId,
        axis: SplitAxis,
    ) -> Result<PaneId, LayoutError> {
        let mode = self
            .layout
            .pane(id)
            .ok_or(LayoutError::PaneNotFound(id))?
            .config()
            .mode();
        let new_id = self
            .layout
            .split(id, axis, 0.5, PaneConfig::new(mode), &mut self.ids)?;
        self.reconcile_runtimes();
        Ok(new_id)
    }

    pub(in crate::app) fn duplicate(
        &mut self,
        id: PaneId,
        axis: SplitAxis,
    ) -> Result<PaneId, LayoutError> {
        self.capture_pose(id);
        let new_id = self.layout.duplicate(id, axis, 0.5, &mut self.ids)?;
        self.reconcile_runtimes();
        Ok(new_id)
    }

    pub(in crate::app) fn remove(&mut self, id: PaneId) -> Result<(), LayoutError> {
        self.layout.remove(id)?;
        self.reconcile_runtimes();
        Ok(())
    }

    pub(in crate::app) fn apply_builtin(
        &mut self,
        preset: BuiltInPreset,
    ) -> Result<(), LayoutError> {
        self.capture_all_poses();
        self.layout = preset.instantiate(&self.layout, &mut self.ids)?;
        self.reconcile_runtimes();
        Ok(())
    }

    pub(in crate::app) fn apply_user(&mut self, preset: &UserPreset) -> Result<(), LayoutError> {
        self.layout = preset.instantiate(&mut self.ids)?;
        self.reconcile_runtimes();
        Ok(())
    }

    pub(in crate::app) fn save_named_preset(&mut self, name: &str) -> Result<(), LayoutError> {
        self.capture_all_poses();
        self.presets.upsert(name, &self.layout)?;
        self.storage_dirty = true;
        Ok(())
    }

    pub(in crate::app) fn delete_preset(&mut self, name: &str) -> bool {
        let deleted = self.presets.delete(name);
        self.storage_dirty |= deleted;
        deleted
    }

    pub(in crate::app) fn save(&mut self, storage: &mut dyn Storage) {
        self.presets.save(storage);
        self.storage_dirty = false;
    }

    pub(super) fn drain_deferred_events(&self) -> Vec<DeferredPaneEvent> {
        self.deferred_rx.try_iter().collect()
    }

    pub(super) fn set_deferred_context_menu(
        &self,
        ctx: &egui::Context,
        id: PaneId,
        model: Option<ContextMenuModel>,
    ) {
        if let Some(handle) = self.deferred.get(&id) {
            handle.update_context_menu(model);
            ctx.request_repaint_of(handle.viewport_id());
        }
    }

    pub(super) fn has_deferred_panes(&self) -> bool {
        !self.deferred.is_empty()
    }

    pub(super) fn deferred_pane_modes(&self) -> Vec<(PaneId, ViewportMode)> {
        self.layout
            .pane_ids()
            .into_iter()
            .filter_map(|id| {
                let pane = self.layout.pane(id)?;
                pane.config()
                    .is_popped_out()
                    .then_some((id, pane.config().mode()))
            })
            .collect()
    }

    pub(super) fn show_deferred(
        &mut self,
        ctx: &egui::Context,
        snapshots: &[(PaneId, Arc<OwnedPaneFrame>)],
    ) {
        self.reconcile_deferred();
        for (id, handle) in &self.deferred {
            let mode = self
                .layout
                .pane(*id)
                .expect("deferred pane remains in layout")
                .config()
                .mode();
            handle.set_mode(mode);
            let snapshot = snapshots.iter().find_map(|(snapshot_id, snapshot)| {
                (*snapshot_id == *id).then(|| Arc::clone(snapshot))
            });
            handle.update_snapshot(snapshot);
            show_deferred_pane(ctx, handle);
        }
    }

    pub(super) fn take_retired_targets(&mut self) -> Vec<PaneId> {
        std::mem::take(&mut self.retired_targets)
    }

    pub(in crate::app) fn reset_2d_cameras(&self) {
        for runtime in self.runtimes.values() {
            runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .reset_2d_cameras();
        }
    }

    pub(in crate::app) fn retain_elevation_cameras<'a>(
        &self,
        wall_ids: impl IntoIterator<Item = &'a str>,
    ) {
        let wall_ids = wall_ids.into_iter().map(str::to_owned).collect::<Vec<_>>();
        for runtime in self.runtimes.values() {
            runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .retain_elevation_cameras(wall_ids.iter().map(String::as_str));
        }
    }

    fn capture_pose(&mut self, id: PaneId) {
        let Some(runtime) = self.runtimes.get(&id) else {
            return;
        };
        let runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(pane) = self.layout.pane_mut(id) {
            pane.config_mut().set_pose_3d(&runtime.view_3d);
        }
    }

    fn capture_all_poses(&mut self) {
        for id in self.layout.pane_ids() {
            self.capture_pose(id);
        }
    }

    fn reconcile_runtimes(&mut self) {
        let live = self.layout.pane_ids().into_iter().collect::<BTreeSet<_>>();
        self.retired_targets.extend(
            self.runtimes
                .keys()
                .filter(|id| !live.contains(id))
                .copied(),
        );
        self.runtimes.retain(|id, _| live.contains(id));
        for id in live {
            self.runtimes.entry(id).or_insert_with(|| {
                let runtime = ViewportPaneRuntime {
                    view_3d: self
                        .layout
                        .pane(id)
                        .expect("live pane ID came from the layout")
                        .config()
                        .pose_3d()
                        .to_view_state(),
                    ..ViewportPaneRuntime::default()
                };
                Arc::new(Mutex::new(runtime))
            });
        }
        self.reconcile_deferred();
    }

    fn reconcile_deferred(&mut self) {
        let popped = self
            .layout
            .pane_ids()
            .into_iter()
            .filter_map(|id| {
                let pane = self.layout.pane(id)?;
                pane.config()
                    .is_popped_out()
                    .then_some((id, pane.config().mode()))
            })
            .collect::<BTreeMap<_, _>>();
        self.deferred.retain(|id, _| popped.contains_key(id));
        for (id, mode) in popped {
            if let Some(handle) = self.deferred.get(&id) {
                handle.set_mode(mode);
                continue;
            }
            let runtime = self
                .runtime(id)
                .expect("popped-out pane must have a runtime");
            self.deferred.insert(
                id,
                DeferredPaneHandle::new(id, mode, runtime, self.deferred_tx.clone()),
            );
        }
    }
}

impl Default for ViewportWorkspaceState {
    fn default() -> Self {
        Self::new(ViewportMode::Plan, None)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn duplicate_allocates_an_independent_runtime_with_the_captured_pose() {
        let mut state = ViewportWorkspaceState::default();
        let original = state.active_id();
        state
            .runtime(original)
            .unwrap()
            .lock()
            .unwrap()
            .view_3d
            .orbit(eframe::egui::Vec2::new(18.0, -6.0));

        let duplicate = state.duplicate(original, SplitAxis::Horizontal).unwrap();
        let original_runtime = state.runtime(original).unwrap();
        let duplicate_runtime = state.runtime(duplicate).unwrap();

        assert!(!Arc::ptr_eq(&original_runtime, &duplicate_runtime));
        assert_eq!(
            original_runtime.lock().unwrap().view_3d.yaw.to_bits(),
            duplicate_runtime.lock().unwrap().view_3d.yaw.to_bits()
        );
    }

    #[test]
    fn applying_a_preset_replaces_runtime_identities() {
        let mut state = ViewportWorkspaceState::default();
        let old = state.active_runtime();

        state.apply_builtin(BuiltInPreset::PlanAnd3d).unwrap();

        assert_eq!(state.layout.pane_count(), 2);
        assert!(
            state
                .layout
                .pane_ids()
                .into_iter()
                .all(|id| !Arc::ptr_eq(&old, &state.runtime(id).unwrap()))
        );
    }

    #[test]
    fn explicit_named_preset_save_marks_dirty_and_persists_immediately() {
        let mut state = ViewportWorkspaceState::default();
        state.save_named_preset(" Desk ").unwrap();
        assert!(state.storage_dirty);

        let mut storage = MemoryStorage::default();
        state.save(&mut storage);

        assert!(!state.storage_dirty);
        let loaded = PresetCatalog::load(Some(&storage));
        assert_eq!(loaded.presets().len(), 1);
        assert_eq!(loaded.presets()[0].name(), "Desk");
    }

    #[test]
    fn removed_and_replaced_panes_are_reported_for_gpu_cleanup_once() {
        let mut state = ViewportWorkspaceState::default();
        let first = state.active_id();
        let second = state.split(first, SplitAxis::Horizontal).unwrap();
        let runtime_handle = state.runtime(second).unwrap();
        let second_runtime = Arc::downgrade(&runtime_handle);
        drop(runtime_handle);

        state.remove(second).unwrap();
        assert!(second_runtime.upgrade().is_none());
        assert_eq!(state.take_retired_targets(), vec![second]);
        assert!(state.take_retired_targets().is_empty());

        state.apply_builtin(BuiltInPreset::PlanAnd3d).unwrap();
        assert_eq!(state.take_retired_targets(), vec![first]);
    }
}
