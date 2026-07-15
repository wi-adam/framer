//! Mutable presentation runtime owned by one logical viewport pane.
//!
//! The split topology and serializable pane configuration live in `layout`; this
//! type deliberately contains the non-serializable camera caches and CPU/GPU
//! progressive render state that must never be shared between two panes.

use std::collections::{HashMap, HashSet};

use framer_core::Point2;

use super::camera_2d::View2dState;
use super::camera_3d::View3dState;
use super::render::RenderPaneState;
use crate::app::draw_wall::SnapResult;

#[derive(Default)]
pub(in crate::app) struct ViewportPaneRuntime {
    pub(in crate::app) view_3d: View3dState,
    pub(in crate::app) plan_view: View2dState,
    pub(in crate::app) elevation_views: HashMap<String, View2dState>,
    pub(in crate::app) render: RenderPaneState,
    pub(in crate::app) previous_snap: Option<SnapResult>,
    pub(in crate::app) cursor_model: Option<Point2>,
}

impl ViewportPaneRuntime {
    pub(in crate::app) fn reset_2d_cameras(&mut self) {
        self.plan_view = View2dState::default();
        self.elevation_views.clear();
        self.previous_snap = None;
        self.cursor_model = None;
    }

    pub(in crate::app) fn retain_elevation_cameras<'a>(
        &mut self,
        live_wall_ids: impl IntoIterator<Item = &'a str>,
    ) {
        if self.elevation_views.is_empty() {
            return;
        }
        let live = live_wall_ids.into_iter().collect::<HashSet<_>>();
        self.elevation_views
            .retain(|id, _| live.contains(id.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_runtimes_keep_same_type_cameras_independent() {
        let mut left = ViewportPaneRuntime::default();
        let right = ViewportPaneRuntime::default();

        left.view_3d.orbit(eframe::egui::Vec2::new(12.0, -8.0));
        left.elevation_views.entry("wall-a".to_owned()).or_default();

        assert_ne!(left.view_3d.yaw.to_bits(), right.view_3d.yaw.to_bits());
        assert!(right.elevation_views.is_empty());
    }

    #[test]
    fn pruning_elevation_cameras_preserves_only_live_wall_ids() {
        let mut pane = ViewportPaneRuntime::default();
        pane.elevation_views
            .insert("wall-a".to_owned(), View2dState::default());
        pane.elevation_views
            .insert("wall-b".to_owned(), View2dState::default());

        pane.retain_elevation_cameras(["wall-b"]);

        assert_eq!(pane.elevation_views.len(), 1);
        assert!(pane.elevation_views.contains_key("wall-b"));
    }

    #[test]
    fn pane_runtime_can_cross_the_deferred_viewport_bridge() {
        fn assert_send<T: Send>() {}
        assert_send::<ViewportPaneRuntime>();
    }
}
