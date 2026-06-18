# Undo / Redo Infrastructure — Design

Status: accepted (2026-06-17)
Branch: `feat/undo-redo` (proposed)

## Goal

Give Framer proper, sustainable undo/redo for authored design edits. Today the
app has no history of any kind: every mutation writes straight into the live
model and is irreversible. We want a single, explicit edit boundary that every
mutation flows through, so that undo/redo is correct by construction and future
edits inherit it for free — not a bolt-on that each new feature has to remember
to wire up.

## Decisions (from product owner)

- **Mechanism:** whole-document **snapshot** of `BuildingModel`, behind an
  explicit `edit(label, |m| …)` transaction boundary. Not a command/inverse
  system. (Snapshots are correct-by-construction here — no hand-written inverses,
  solver-safe — and cheap at Framer's KB-scale documents.)
- **Restore scope:** the document **model + selection** (`selected`,
  `selected_wall`). The camera, view mode, and workspace mode are *not* restored —
  a stable viewpoint that re-focuses the edited object is the standard CAD feel.
- **Load / New / Reset:** **clear** the history (Open/New/Reset start fresh; you
  cannot undo across a file load).
- **UI surface:** keyboard shortcuts (`⌘Z` / `⌘⇧Z`, plus `Ctrl` variants and
  `Ctrl+Y`) **and** labeled toolbar Undo/Redo buttons that show the next action
  ("Undo Add Opening") and disable when the stack is empty.
- **History model:** linear (a new edit after undo truncates the redo branch);
  bounded depth (default 200).

## Why snapshots, not commands

Grounded in the current codebase:

- The entire editable document is one value — `FramerApp.model: BuildingModel`
  (`crates/framer-app/src/app/mod.rs:32`). `BuildingModel`
  (`crates/framer-core/src/model.rs:31`) is a clean tree (`levels`,
  `walls` → `openings` + `dimensions`, `wall_joins`) deriving `Clone`,
  `PartialEq`, `Eq` with **no `Rc`/`Arc`/`RefCell`** anywhere. A snapshot is one
  `model.clone()`; a restore is one assignment.
- Real documents are tiny — demo-wall ~3.5 KB, demo-shell ~9.5 KB JSON; dozens
  to low-hundreds of value-type elements. Per-step clone cost is negligible and a
  deep history fits in memory easily. The memory advantage of commands is
  irrelevant at this scale.
- There is currently **no** command/action layer — mutations are ~40 scattered
  field assignments in `panels.rs` plus a handful of handlers in `mod.rs`.
  Hand-written inverses would be high-effort and error-prone because the
  constraint solver makes inverses non-local: deleting an opening cascades to
  removing dependent dimensions (`Wall::remove_opening`,
  `crates/framer-core/src/model.rs:688`), and applying a driving dimension
  re-solves and rewrites multiple geometry fields in place
  (`apply_driving_dimensions`, `model.rs:294`, `model.rs:931`). A snapshot
  captures the solved result for free.
- `rebuild()` (`crates/framer-app/src/app/mod.rs:229`) already funnels nearly
  every mutation (re-solve + regenerate derived state), giving one natural place
  to keep history consistent.

`PartialEq` on `BuildingModel` is derived, so no-op edits (egui `DragValue`
fires `.changed()` every frame even when the value is identical) are dropped for
free with a `model != before.model` guard.

## Architectural constraints honored

From `docs/architecture.md` and `docs/project-files.md`:

- **Three-layer separation.** Undo/redo tracks **only** the authored
  `BuildingModel`. It never snapshots or restores derived framing
  (`project_plan`) or render/viewport caches — `rebuild()` regenerates those from
  the model after every restore.
- **History is presentation/ephemeral state.** It lives entirely in
  `framer-app`, never in `framer-core`, and is never written to `.framer`. (The
  on-disk schema rejects unknown top-level keys via `deny_unknown_fields`, so it
  physically cannot leak in.)
- **Stable IDs preserved.** `ElementId` is the cross-reference mechanism for
  every relationship. Clone-snapshots restore exact prior IDs trivially.
- **Restored states are valid and re-solved.** Snapshots are taken from
  previously-valid models, so `BuildingModel::validate()` is satisfied for free;
  every undo/redo ends with `rebuild()` so the solver output and derived state
  match the restored intent.

## Architecture

```
crates/framer-core    (unchanged) — BuildingModel is the document; no history here
crates/framer-app
  └── app/history.rs  (NEW) — History, Snapshot, HistoryEntry: the undo/redo engine
  └── app/mod.rs       — FramerApp gains `history: History`; edit()/undo()/redo();
                         existing handlers route through edit(); rebuild() unchanged
  └── app/panels.rs    — toolbar Undo/Redo buttons; inspector capture+settle
```

### Data structures (`app/history.rs`)

```rust
/// One restorable point: the authored document plus the transient selection
/// we restore alongside it. NOT serialized; lives only in memory.
struct Snapshot {
    model: BuildingModel,
    selected: Selection,
    selected_wall: usize,
}

/// A prior Snapshot paired with the label of the action that moved *away* from
/// it (e.g. "Add opening"). The label drives the toolbar tooltip.
struct HistoryEntry {
    snapshot: Snapshot,
    label: String,
}

pub(super) struct History {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    /// An open transaction captured at gesture/edit start, committed on settle.
    pending: Option<HistoryEntry>,
    limit: usize, // depth cap (default 200)
}
```

`FramerApp` gains exactly one field: `history: History`. `Selection` already
derives `Clone`.

**Label semantics.** An undo-stack entry holds the state *before* an action,
tagged with that action's name. "Undo" restores `undo.last()` and the button
reads `Undo {label}`. On undo, the current state moves to `redo` carrying the
same label, so "Redo" reads symmetrically.

### Edit API surface

Three entry patterns, matched to how each mutation actually arrives, all
funneling into `History`:

**(a) Discrete actions — `edit()` wrapper.** For single-shot handlers
(`add_opening` `mod.rs:467`, `delete_selected_opening`,
`duplicate_selected_opening`, `handle_dimension_placement_click`, combobox/text
commits):

```rust
fn edit(&mut self, label: &str, f: impl FnOnce(&mut Self)) {
    let before = self.snapshot();        // {model, selected, selected_wall}.clone()
    f(self);                             // existing mutation body, verbatim
    if self.model != before.model {      // derived PartialEq → free no-op drop
        self.history.push(before, label); // clears redo, enforces depth cap
        self.rebuild();
    }
}
```

`add_opening` becomes `self.edit("Add opening", |app| { …existing body… })`.
The guard means a handler that no-ops (e.g. add blocked outside Design mode)
records nothing.

**(b) Bracketed gestures — explicit begin/commit.** The viewport opening-drag
already has clean lifecycle hooks. `begin_opening_drag` (`mod.rs:564`) opens a
transaction (`history.begin(snapshot, "Move opening")`); `update_opening_drag`
(`mod.rs:588`) keeps calling `rebuild()` as today with **no** history change;
`finish_opening_drag` (`mod.rs:614`) calls `history.commit()`. One undo step per
drag, regardless of frame count.

**(c) Immediate-mode inspector — capture + settle.** The inspector's ~40 edits
funnel through one `if changed { self.rebuild() }` (`panels.rs:966`). Pattern:

- On the first frame `changed` becomes true with no open transaction, open one
  whose base is the *pre-frame* model and whose label is derived from the active
  `Selection` ("Edit wall", "Edit opening", "Edit dimension", "Edit level",
  "Edit join").
- Keep `rebuild()`-ing each frame; push nothing.
- A `history.settle()` call near end-of-frame commits the open transaction once
  the interaction ends — detected via egui pointer-up + no text-field focus. This
  coalesces a whole `DragValue` drag into one undo step.

The base snapshot for (c) is captured lazily — only on frames where an inspector
edit could begin or commit (pointer down, a click this frame, or a focused text
field) and no transaction is already open. A truly idle inspector clones
nothing. (A focused-but-idle text field still triggers egui's ~2 Hz cursor-blink
repaints, each cloning the KB-scale model once and dropping it — a negligible
cost, not a per-frame hot path.)

> Implementation note: the commit gate must include egui's `pointer.any_click()`,
> not just `pointer.any_down()` — ComboBox selections and buttons commit on the
> pointer-*release* frame, when `any_down()` is already false. See
> `should_capture_edit_base` in `panels.rs`.

### Performing undo / redo

```rust
fn undo(&mut self) {
    self.history.settle_force();          // commit any open transaction first
    if let Some(entry) = self.history.pop_undo(self.snapshot()) {
        self.restore(entry.snapshot);     // swap model + selected + selected_wall
        self.rebuild();                   // re-solve; regenerate derived state
    }
}
```

`pop_undo` pushes the *current* state onto `redo` (with the popped entry's
label) and returns the entry. `redo()` is the mirror. `restore()` reassigns
`model`, `selected`, `selected_wall` only — camera/view/workspace untouched.
`rebuild()` already clamps `selected_wall` when out of range (`mod.rs:230`), so a
restored selection that dangles is handled.

### UI wiring

- **Keyboard** — in `handle_keyboard_shortcuts()` (`mod.rs:381`, called from
  `logic()`): consume `Cmd/Ctrl+Z` → undo, `Cmd/Ctrl+Shift+Z` and `Ctrl+Y` →
  redo. The existing `if ctx.text_edit_focused() { return; }` guard means that
  while typing in a field, `⌘Z` is the text widget's own character-undo
  (standard); field edits are committed — and become undoable — on settle /
  focus-loss anyway.
- **Toolbar** — in `toolbar()` (`panels.rs:72`): `↶` / `↷` buttons,
  `add_enabled(history.can_undo())` / `can_redo()`, tooltip = `Undo {label}` /
  `Redo {label}`.

## Lifecycle & edge cases

- `new_project` / `reset_demo` / `reset_wall_demo` / `load_project_file`
  (`mod.rs:249–344`) call `history.clear()` and keep their direct mutation — not
  routed through `edit()`.
- `save_project_file` records nothing (no model change).
- View/workspace-mode switches, camera orbit, and render interaction make no
  history — they are not document edits.
- A new `edit()`/commit truncates the redo branch (linear history).
- Depth cap (200) evicts oldest undo entries.
- No-op edits (`model == before.model`) record nothing.

## Testing

`history.rs` unit tests, exercising `History` directly with a tiny
`BuildingModel` (no egui required):

- push → undo → redo round-trips a byte-identical model (`PartialEq`);
- a no-op edit records nothing;
- a new edit after undo truncates redo;
- depth-cap eviction drops the oldest entry;
- transaction coalescing: `begin` → N updates → `commit` yields exactly one entry;
- `clear()` empties both stacks and any pending transaction.

One integration-style test drives `FramerApp::edit` then `undo`/`redo` and
asserts that `rebuild()` re-ran and `project_plan` regenerated, and that
selection was restored.

## Out of scope (YAGNI)

- A typed command/inverse system (reconsider only if documents grow large enough
  that snapshot memory matters; stable `ElementId`s keep it expressible later).
- Restoring camera/view/workspace state on undo.
- Persisting history to disk or across sessions.
- A non-linear (tree) history or a visible history panel beyond the labeled
  Undo/Redo buttons.
