//! App-owned lifecycle for explicit, disposable authored-change options.
//!
//! Candidate synthesis and patch validation stay in `framer-analysis`. This module only binds
//! those UI-free options to the app's process-local document generation, preview overlay, and
//! ordinary undoable edit path.

use framer_analysis::{
    DocumentRevision, ResolutionOptionSet, ResolutionRequest, ResolutionRevision,
    generate_resolution_options, stage_resolution_option,
};
use framer_core::AuthoredIntentId;

use super::{FramerApp, ViewportMode, viewport};

pub(super) struct ResolutionUiState {
    pub(super) target: AuthoredIntentId,
    pub(super) origin: Option<ResolutionRevision>,
    pub(super) options: Option<ResolutionOptionSet>,
    pub(super) error: Option<String>,
    pub(super) selected: usize,
    pub(super) stale: bool,
}

impl ResolutionUiState {
    fn error(target: AuthoredIntentId, message: impl Into<String>) -> Self {
        Self {
            target,
            origin: None,
            options: None,
            error: Some(message.into()),
            selected: 0,
            stale: false,
        }
    }

    fn options(target: AuthoredIntentId, options: ResolutionOptionSet) -> Self {
        Self {
            target,
            origin: Some(options.origin()),
            options: Some(options),
            error: None,
            selected: 0,
            stale: false,
        }
    }
}

impl FramerApp {
    pub(super) fn current_resolution_revision(&self) -> Option<ResolutionRevision> {
        let graph = self.project_graph.as_ref()?.revision();
        let report = self.intent_report.as_ref()?.as_ref().ok()?;
        (report.revision() == graph)
            .then(|| ResolutionRevision::new(graph, DocumentRevision::new(self.document_revision)))
    }

    pub(super) fn request_resolution_options(&mut self, target: AuthoredIntentId) -> bool {
        self.placement_resolution_preview = None;
        let Some(revision) = self.current_resolution_revision() else {
            self.resolution_ui = Some(ResolutionUiState::error(
                target,
                "Resolution analysis is unavailable until the current intent report and project graph agree.",
            ));
            return false;
        };
        let request = ResolutionRequest::placement_clearance(target.clone());
        match generate_resolution_options(
            &self.model,
            revision,
            &request,
            &mut self.resolution_cache,
        ) {
            Ok(options) => {
                let count = options.options().len();
                self.resolution_ui = Some(ResolutionUiState::options(target, options));
                self.file_status = Some(if count == 0 {
                    "No feasible placement option was found within the bounded search".to_owned()
                } else {
                    format!("Generated {count} placement resolution option(s)")
                });
                true
            }
            Err(error) => {
                self.file_status = Some(format!("Resolution options unavailable: {error}"));
                self.resolution_ui = Some(ResolutionUiState {
                    target,
                    origin: Some(revision),
                    options: None,
                    error: Some(error.to_string()),
                    selected: 0,
                    stale: false,
                });
                false
            }
        }
    }

    pub(super) fn select_resolution_option(&mut self, index: usize) -> bool {
        let Some(state) = self.resolution_ui.as_mut() else {
            return false;
        };
        let Some(options) = state.options.as_ref() else {
            return false;
        };
        if state.stale || index >= options.options().len() {
            return false;
        }
        state.selected = index;
        self.placement_resolution_preview = None;
        true
    }

    pub(super) fn preview_resolution_option(&mut self, index: usize) -> bool {
        let Some(current) = self.current_resolution_revision() else {
            self.file_status =
                Some("Resolution preview rejected: analysis is unavailable".to_owned());
            return false;
        };
        let Some(state) = self.resolution_ui.as_ref() else {
            return false;
        };
        if state.stale || state.origin != Some(current) {
            self.file_status =
                Some("Resolution preview rejected: options are stale; regenerate them".to_owned());
            return false;
        }
        let Some(option) = state
            .options
            .as_ref()
            .and_then(|options| options.options().get(index))
        else {
            return false;
        };
        if let Err(error) = stage_resolution_option(&self.model, option, current) {
            self.file_status = Some(format!("Resolution preview rejected: {error}"));
            return false;
        }
        let patch = option.patch();
        self.placement_resolution_preview = Some(viewport::PlacementResolutionPreview {
            target: patch.target.authored(),
            before_position: patch.expected.position,
            before_rotation: patch.expected.rotation,
            after_position: patch.replacement.position,
            after_rotation: patch.replacement.rotation,
        });
        if let Some(state) = self.resolution_ui.as_mut() {
            state.selected = index;
        }
        self.set_authoring_viewport_mode(ViewportMode::Plan);
        self.file_status = Some(format!(
            "Previewing placement resolution option {}",
            index + 1
        ));
        true
    }

    pub(super) fn accept_resolution_option(&mut self, index: usize) -> bool {
        if !self.workspace_mode.allows_design_edits() {
            self.file_status =
                Some("Resolution acceptance is available only in Design workspace".to_owned());
            return false;
        }
        let Some(current) = self.current_resolution_revision() else {
            self.file_status =
                Some("Resolution acceptance rejected: analysis is unavailable".to_owned());
            return false;
        };
        let Some(state) = self.resolution_ui.as_ref() else {
            return false;
        };
        if state.stale || state.origin != Some(current) {
            self.file_status = Some(
                "Resolution acceptance rejected: options are stale; regenerate them".to_owned(),
            );
            return false;
        }
        let Some(option) = state
            .options
            .as_ref()
            .and_then(|options| options.options().get(index))
            .cloned()
        else {
            return false;
        };
        let candidate = match stage_resolution_option(&self.model, &option, current) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.file_status = Some(format!("Resolution acceptance rejected: {error}"));
                return false;
            }
        };
        let target = option.patch().target.element().0.clone();
        self.dismiss_resolution_options();
        self.commit_validated_model(
            "Apply resolution option",
            candidate,
            format!("Applied placement resolution for '{target}'"),
        )
    }

    pub(super) fn dismiss_resolution_options(&mut self) -> bool {
        let had_options = self.resolution_ui.take().is_some();
        let had_preview = self.placement_resolution_preview.take().is_some();
        had_options || had_preview
    }
}
