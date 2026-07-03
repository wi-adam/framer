# Command Surfaces — Implementation Plan (2026-07-03)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/command-surfaces.md](../specs/command-surfaces.md). This file is an archival
> record of how the work was sequenced; the spec is the durable source of truth.

## Goal

Define and implement a scalable command-surface system so Framer stops absorbing every new
action into one oversized button row. The target is a compact parametric CAD workbench:
workflow command strip, quick-access app bar, browser/catalog panels, PropertyManager-style
inspector, marking/context menus, status/view controls, and command search. This plan starts
with documentation and mockups, then migrates the app without changing the `.framer` schema.

## Architecture / stack summary

- `crates/framer-app/src/app/panels.rs` currently renders the app header, current toolbar,
  inspector, and status bar. The current `toolbar()` becomes the workflow command-strip
  migration point.
- `crates/framer-app/src/app/design/widgets.rs` provides visual primitives for toolbar
  buttons, groups, toggles, chips, and inspector sections. It needs compact command-strip
  primitives, split/flyout buttons, panel dividers, and PropertyManager rows.
- `crates/framer-app/src/app/viewport/mod.rs` owns the workspace header and canvas floating
  toolbar. It is the seam for marking/context menus, ViewCube/navigation affordances, and
  placement feedback.
- `crates/framer-app/src/app/ui_harness_tests.rs` is the existing headless UI smoke-test seam.
- Durable policy lives in [command-surfaces.md](../specs/command-surfaces.md); styling tokens
  stay in [design-system.md](../specs/design-system.md).

## Slices / phases

### Slice 1 — Name the system and lock the CAD target

- **Task 1.1** — Add the durable command-surface spec and this dated plan.
  - Files: `docs/specs/command-surfaces.md`,
    `docs/plans/2026-07-03-command-surfaces.md`, `docs/specs/README.md`,
    `docs/plans/README.md`, `docs/specs/design-system.md`, `docs/code-map.md`,
    `crates/framer-app/README.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: `docs(app): define command surfaces`
- **Task 1.2** — Revise the durable direction from spacious studio UI toward a compact CAD
  workbench informed by Fusion, Inventor, SOLIDWORKS, and Onshape.
  - Files: `docs/specs/command-surfaces.md`, `docs/specs/design-system.md`,
    `docs/plans/2026-07-03-command-surfaces.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: `docs(app): align command surfaces with cad workbench`
- **Task 1.3** — Generate static mockups that exercise command-strip density, browser/catalog,
  PropertyManager, marking/context menus, status/view controls, and command search.
  - Files: disposable mockup artifacts under `/tmp` unless a later task promotes one into
    checked-in design documentation
  - Verify: browser screenshot at 1800x1080 and visual inspection
  - Commit: none unless promoted into `docs/`
- **Task 1.4** — Add the art direction layer: CAD density plus Framer craft, including custom
  framing icons, warm drawing-paper canvas, construction semantics, and refined command-strip
  styling.
  - Files: `docs/specs/command-surfaces.md`, `docs/specs/design-system.md`,
    disposable mockup artifacts under `/tmp`
  - Verify: `python3 scripts/check-markdown-links.py`; browser screenshot at 1800x1080 and
    visual inspection
  - Commit: `docs(app): add framer command surface art direction`
- **Task 1.5** — Inventory current toolbar commands against the routing matrix.
  - Files: `docs/plans/2026-07-03-command-surfaces.md`
  - Verify: manual cross-check against `FramerApp::toolbar`
  - Commit: `docs(app): inventory toolbar commands`

### Slice 2 — Add lightweight action metadata

- **Task 2.1** — Introduce a UI-only action metadata seam.
  - Files: `crates/framer-app/src/app/actions.rs`, `crates/framer-app/src/app/mod.rs`,
    `crates/framer-app/src/app/panels.rs`, `docs/code-map.md`
  - Verify: `cargo test -p framer-app --all-features --locked`
  - Commit: `refactor(app): add command metadata`
- **Task 2.2** — Add focused tests for duplicate action ids, missing accessible labels/tooltips,
  flyout reachability, and command-strip eligibility.
  - Files: `crates/framer-app/src/app/actions.rs`,
    `crates/framer-app/src/app/ui_harness_tests.rs`
  - Verify: `cargo test -p framer-app --all-features --locked`
  - Commit: `test(app): cover command metadata`

### Slice 3 — Replace the button row with a workflow command strip

- **Task 3.1** — Move project/document actions and sample loaders out of the command strip into
  the app/quick-access bar and project menu.
  - Status: implemented for header quick-access New/Open/Save/Undo/Redo, Project menu
    New/Open/Save/Export, and Examples menu demo loaders.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/design/widgets.rs`
  - Verify: `cargo test -p framer-app --all-features --locked`; manual app run
  - Commit: `refactor(app): move project actions to header`
- **Task 3.2** — Replace the existing broad toolbar groups with workflow tabs and compact
  command panels (`Design`, `Frame`, `Openings`, `Roofs`, `Annotate`, `Inspect`, `Plan`).
  - Status: implemented as a workflow tab row plus compact View, Structure,
    Openings, Roofs, Dimensions, and Generated command panels. View controls stay
    in the strip until Task 3.3, and active dimension options stay inline until
    Task 3.4.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/design/widgets.rs`
  - Verify: `cargo test -p framer-app --all-features --locked`; manual app run
  - Commit: `refactor(app): add workflow command strip`
- **Task 3.3** — Move view/workspace switching into the workspace/view bar.
  - Status: implemented as clickable `Design Workspace` / `Plan Workspace` and
    view tabs (`Shell`/`Plan`, `Wall`/`Elevation`, `Roof`, `3D`, `Render`) in
    the workspace header. The temporary workflow-strip View panel is removed.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: `cargo test -p framer-app --all-features --locked`; manual app run
  - Commit: `refactor(app): move view switching to workspace chrome`
- **Task 3.4** — Move active tool settings into a contextual tab or options strip. Wall should
  expose type, baseline, height, level, and placement compactly while active.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: `cargo test -p framer-app --all-features --locked`; manual Wall/Room/Ceiling/Floor/
    Dimension placement check
  - Commit: `refactor(app): add contextual tool options`

### Slice 4 — Add CAD-native contextual homes

- **Task 4.1** — Move Delete and other selection lifecycle commands to a marking menu /
  shortcut menu / compact context toolbar while preserving shortcuts.
  - Files: `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/panels.rs`
  - Verify: `cargo test -p framer-app --all-features --locked`; manual selection/delete undo check
  - Commit: `feat(app): surface selection actions contextually`
- **Task 4.2** — Move Door/Window/Garage and roof-form insertion to command-strip flyouts,
  catalog rows, or host-aware Insert menus.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: `cargo test -p framer-app --all-features --locked`; manual add/undo checks for each
    insertion type
  - Commit: `feat(app): add contextual insertion surface`
- **Task 4.3** — Add command search as the searchable backstop for commands that no longer
  occupy permanent chrome.
  - Files: `crates/framer-app/src/app/actions.rs`,
    `crates/framer-app/src/app/panels.rs`
  - Verify: `cargo test -p framer-app --all-features --locked`; manual keyboard/open/execute check
  - Commit: `feat(app): add command search`

### Slice 5 — Visual and accessibility polish

- **Task 5.1** — Add or update headless UI smoke coverage for the app/quick-access bar, workflow
  command strip, contextual surface, and command-search reachability.
  - Files: `crates/framer-app/src/app/ui_harness_tests.rs`
  - Verify: `cargo test -p framer-app --all-features --locked`
  - Commit: `test(app): cover command surfaces`
- **Task 5.2** — Manual visual pass against default desktop width and a narrow window, confirming
  the workflow command strip stays dense and CAD-like rather than reverting to large buttons.
  - Files: `docs/specs/command-surfaces.md` if the budget decision changes
  - Verify: manual run and screenshots
  - Commit: `docs(app): record command surface budget`

## Current command-surface inventory

| Current surface/group | Current commands | Spec route |
| --- | --- | --- |
| App header quick access | New, Open, Save, Undo, Redo | App/quick-access bar |
| Project menu | New, Open, Save, Export | Project menu; Export also Plan workspace |
| Examples menu | Shell, Wall demo loaders | Examples picker / Project menu |
| Workflow tab row | Design, Frame, Openings, Roofs, Annotate, Inspect, Plan | Workflow command strip tabs; Plan switches to Plan workspace |
| Workspace/view bar | Design Workspace, Plan Workspace, Shell/Plan, Wall/Elevation, Roof, 3D, Render | Workspace/view bar |
| Workflow strip: Design / Structure panel | Room | Workflow command strip: Design panel |
| Workflow strip: Frame / Structure panel | Wall, Ceiling, Vault, Floor | Workflow command strip: Frame panel |
| Shortcut / contextual route | Delete | Marking menu / shortcut menu / shortcut; permanent context surface lands in Slice 4.1 |
| Workflow strip: Openings panel | Door, Window, Garage | Temporary top-level variants until flyout/catalog migration |
| Workflow strip: Roofs panel | Gable, Shed, Hip | Temporary top-level variants until roof flyout/options migration |
| Workflow strip: Annotate / Dimensions panel | Linear | Workflow command strip: Annotate panel |
| Workflow strip: Annotate active options | Driving/Reference, Horizontal/Vertical | Temporary inline options until contextual options strip migration |
| Workflow strip: Plan / Generated panel | Section | Plan command tab or view-control bar |

## Final verification

Docs-only slices:

```sh
python3 scripts/check-markdown-links.py
```

Code slices:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
```

When the UI migration lands, update the spec's **Status** and **Last reviewed**, refresh
`docs/code-map.md`, and keep the command inventory consistent with the app.
