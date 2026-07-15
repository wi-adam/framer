//! In-memory undo/redo history for the desktop app.
//!
//! A pure, UI-agnostic engine generic over a snapshot/state type `S`. The app
//! instantiates it with `Snapshot` (authored `BuildingModel` + selection +
//! component visibility). The engine knows nothing about the document, the
//! solver, or egui, which keeps it exhaustively unit-testable with a trivial
//! state type.
//!
//! History is ephemeral presentation state: it is never serialized and never
//! written to a `.framer` file. See
//! `docs/specs/undo-redo.md`.

/// A stored state paired with the label of the action that moved *away* from
/// it (e.g. "Add opening"). The label drives the toolbar tooltip.
pub(super) struct HistoryEntry<S> {
    pub snapshot: S,
    pub label: String,
}

/// Linear undo/redo history with coalesced transactions and a bounded depth.
pub(super) struct History<S> {
    undo: Vec<HistoryEntry<S>>,
    redo: Vec<HistoryEntry<S>>,
    /// An open transaction captured at gesture/edit start, finalized on commit.
    /// Used to coalesce a continuous gesture (a drag, an inspector edit run)
    /// into a single undo step.
    pending: Option<HistoryEntry<S>>,
    /// Maximum number of entries retained on the undo stack; oldest evicted.
    limit: usize,
}

impl<S> History<S> {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            limit: limit.max(1),
        }
    }

    pub(super) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
    }

    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(super) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(super) fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|entry| entry.label.as_str())
    }

    pub(super) fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|entry| entry.label.as_str())
    }

    /// Record a discrete edit: push the pre-edit `before` state under `label`
    /// and discard the redo branch (linear history).
    pub(super) fn record(&mut self, before: S, label: impl Into<String>) {
        self.push_undo(HistoryEntry {
            snapshot: before,
            label: label.into(),
        });
        self.redo.clear();
    }

    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Open a transaction with `before` as its base. No-op if one is already
    /// open, so the first state of a gesture is retained across coalesced
    /// updates.
    pub(super) fn begin(&mut self, before: S, label: impl Into<String>) {
        if self.pending.is_none() {
            self.pending = Some(HistoryEntry {
                snapshot: before,
                label: label.into(),
            });
        }
    }

    /// The base state of the open transaction, if any. Lets the caller decide
    /// whether a settled gesture actually changed anything before committing.
    pub(super) fn pending_base(&self) -> Option<&S> {
        self.pending.as_ref().map(|entry| &entry.snapshot)
    }

    /// Finalize the open transaction into a single undo entry. No-op if none.
    pub(super) fn commit(&mut self) {
        if let Some(entry) = self.pending.take() {
            self.push_undo(entry);
            self.redo.clear();
        }
    }

    /// Discard the open transaction without recording it (e.g. a gesture that
    /// returned to its starting state).
    pub(super) fn cancel_pending(&mut self) {
        self.pending = None;
    }

    /// Undo one step. The caller supplies the `current` state (moved onto the
    /// redo stack); the state to restore to is returned. `None` if nothing to
    /// undo.
    pub(super) fn undo(&mut self, current: S) -> Option<S> {
        let entry = self.undo.pop()?;
        self.redo.push(HistoryEntry {
            snapshot: current,
            label: entry.label,
        });
        Some(entry.snapshot)
    }

    /// Redo one step. Mirror of [`undo`].
    pub(super) fn redo(&mut self, current: S) -> Option<S> {
        let entry = self.redo.pop()?;
        self.undo.push(HistoryEntry {
            snapshot: current,
            label: entry.label,
        });
        Some(entry.snapshot)
    }

    /// Push onto the undo stack, evicting the oldest entries past `limit`.
    fn push_undo(&mut self, entry: HistoryEntry<S>) {
        self.undo.push(entry);
        if self.undo.len() > self.limit {
            let overflow = self.undo.len() - self.limit;
            self.undo.drain(0..overflow);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_labels<S>(stack: &[HistoryEntry<S>]) -> Vec<&str> {
        stack.iter().map(|e| e.label.as_str()).collect()
    }

    #[test]
    fn new_history_is_empty() {
        let history: History<i32> = History::new(100);
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert_eq!(history.undo_label(), None);
        assert_eq!(history.redo_label(), None);
        assert!(!history.is_pending());
    }

    #[test]
    fn record_then_undo_restores_previous_state() {
        let mut history = History::new(100);
        history.record(1, "edit");
        assert!(history.can_undo());
        // Current document is now 2; undoing restores the recorded 1.
        assert_eq!(history.undo(2), Some(1));
        assert!(!history.can_undo());
        assert!(history.can_redo());
    }

    #[test]
    fn undo_then_redo_round_trips() {
        let mut history = History::new(100);
        history.record(1, "edit");
        let restored = history.undo(2).unwrap();
        assert_eq!(restored, 1);
        // Redoing from the restored state returns the post-edit state.
        assert_eq!(history.redo(restored), Some(2));
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn undo_on_empty_returns_none() {
        let mut history: History<i32> = History::new(100);
        assert_eq!(history.undo(5), None);
    }

    #[test]
    fn redo_on_empty_returns_none() {
        let mut history: History<i32> = History::new(100);
        assert_eq!(history.redo(5), None);
    }

    #[test]
    fn new_record_truncates_redo_branch() {
        let mut history = History::new(100);
        history.record(1, "a");
        history.undo(2); // redo now holds the post-"a" state
        assert!(history.can_redo());
        // A fresh edit from the restored state discards the redo branch.
        history.record(1, "b");
        assert!(!history.can_redo());
    }

    #[test]
    fn labels_track_the_action_on_each_side() {
        let mut history = History::new(100);
        history.record(1, "Add opening");
        assert_eq!(history.undo_label(), Some("Add opening"));
        assert_eq!(history.redo_label(), None);
        history.undo(2);
        assert_eq!(history.undo_label(), None);
        assert_eq!(history.redo_label(), Some("Add opening"));
    }

    #[test]
    fn depth_cap_evicts_oldest_entries() {
        let mut history = History::new(2);
        history.record(10, "a");
        history.record(20, "b");
        history.record(30, "c"); // "a" should be evicted
        // Only the two most-recent steps remain reachable.
        assert_eq!(history.undo(40), Some(30));
        assert_eq!(history.undo_label(), Some("b"));
        assert_eq!(history.undo(30), Some(20));
        assert_eq!(history.undo(20), None); // "a" was evicted, not reachable
    }

    #[test]
    fn transaction_coalesces_to_a_single_entry() {
        let mut history = History::new(100);
        history.begin(100, "drag");
        assert!(history.is_pending());
        // Subsequent begins during the gesture are ignored (base/label kept).
        history.begin(101, "drag-again");
        assert_eq!(history.pending_base(), Some(&100));
        history.commit();
        assert!(!history.is_pending());
        assert!(history.can_undo());
        assert_eq!(history.undo_label(), Some("drag"));
        assert_eq!(history.undo(200), Some(100));
    }

    #[test]
    fn commit_without_pending_is_noop() {
        let mut history: History<i32> = History::new(100);
        history.commit();
        assert!(!history.can_undo());
    }

    #[test]
    fn cancel_pending_discards_the_transaction() {
        let mut history = History::new(100);
        history.begin(5, "x");
        history.cancel_pending();
        assert!(!history.is_pending());
        history.commit();
        assert!(!history.can_undo());
    }

    #[test]
    fn commit_truncates_redo_branch() {
        let mut history = History::new(100);
        history.record(1, "a");
        history.undo(2);
        assert!(history.can_redo());
        history.begin(3, "b");
        history.commit();
        assert!(!history.can_redo());
    }

    #[test]
    fn clear_empties_every_stack() {
        let mut history = History::new(100);
        history.record(1, "a");
        history.undo(2);
        history.begin(3, "b");
        history.clear();
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert!(!history.is_pending());
    }

    #[test]
    fn undo_moves_current_state_onto_redo() {
        let mut history = History::new(100);
        history.record(1, "a");
        // The *current* state (2) is what redo restores, not the recorded one.
        history.undo(2);
        assert_eq!(history.redo(1), Some(2));
    }

    #[test]
    fn internal_stacks_stay_within_limit() {
        let mut history = History::new(3);
        for n in 0..10 {
            history.record(n, format!("edit-{n}"));
        }
        assert_eq!(history.undo.len(), 3);
        assert_eq!(
            entry_labels(&history.undo),
            vec!["edit-7", "edit-8", "edit-9"]
        );
    }
}
