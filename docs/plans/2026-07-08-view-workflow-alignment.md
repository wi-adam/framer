# View–Workflow Alignment — Implementation Plan (2026-07-08)

> **Implementation plan** (point-in-time). **Specs:**
> [command-surfaces.md](../specs/command-surfaces.md) and
> [render-view.md](../specs/render-view.md) (both updated 2026-07-08 with the
> locked decisions this plan implements). This file is an archival record of how
> the work was sequenced; the specs are the durable source of truth.

## Goal

Resolve the workflow-tab × view-tab mismatch found on 2026-07-08: the ribbon
today shows modeling tools over the Render view, Openings tools while in Shell
view, and the Shell/Wall/Roof/3D/Render strip reads as floating text rather than
a control. Four coordinated changes (locked in
[command-surfaces.md → Decisions](../specs/command-surfaces.md#decisions-locked)):

- **A — Render becomes an output workflow tab** next to `Plan` (leaves the view
  strip; gets a Render workspace and a render-settings strip).
- **B — Authoring tabs set a soft default view** on tab switch (Openings → the
  selected wall's elevation, Roofs → Roof, …); views stay freely switchable.
- **C — Registry-driven applicability gating**: authoring tools disabled with
  explanatory tooltips in output workspaces; auto-snap kept between authoring
  views; no surface hard-codes "enabled".
- **E — View strip restyle** as a compact segmented control (authoring cameras
  only).

Five code PRs plus a docs PR, ordered so each builds on the last. Each PR leaves
the workspace green.

## How to work this plan (read first)

- Read [AGENTS.md](../../AGENTS.md) before starting. All work is in `framer-app`
  only — **never add UI dependencies or UI-driven changes to `framer-core`,
  `framer-solver`, `framer-standards`, `framer-library`, or `framer-render`**.
  PR 4 *reads* existing `framer-render` types (`RenderOptions`, `DirectionalSun`,
  `Sky` in `crates/framer-render/src/build.rs`) but must not modify that crate.
- Gates before every commit (workspace root):
  `cargo fmt --all -- --check` ·
  `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` ·
  `cargo test --workspace --all-features --locked`.
- **Visual verification:** prefer `scripts/ui-shots.sh` (the `ui-shots` skill) —
  it renders the whole UI off-screen to `target/ui-shots/*.png` in ~15 s. Add
  states to `crates/framer-app/src/app/ui_shots_tests.rs` when the deck doesn't
  cover what you changed (this plan adds Render-tab and segmented-control
  states). Only *interactive* checks (tool auto-snap feel, drag a sun slider and
  watch re-accumulation) need the `install-app` skill
  (`scripts/install-app.sh` → drive `~/Applications/Framer.app`; `cargo run`
  windows are invisible to screenshot tooling).
- Headless UI tests live in `crates/framer-app/src/app/ui_harness_tests.rs`
  (egui_kittest). Gotchas: build the UI through `FramerApp::ui_root` and warm up
  fonts before asserting on text (copy the existing pattern).
- Pure metadata tests (duplicate ids, routing, strip budget) live next to
  `actions.rs`; state-machine tests (workspace/view/tool transitions) live in
  `mod.rs` tests. Follow the `viewport/` module conventions (single tests
  module, `cfg(test)` imports, `pub(super)` visibility) when touching
  `viewport/mod.rs`.
- File/line anchors below were verified on 2026-07-08 at commit `6105f26`.
  Lines drift — re-locate with `grep -n` before editing; symbol names are
  stable.
- One PR per slice, `feat(app)`/`fix(app)` commit style per recent history.
  Do not fold slices together.

## Architecture / stack summary

The three state axes this plan aligns (all in `crates/framer-app/src/app/`):

- **`WorkflowTab`** (`actions.rs:94`) — `Design, Frame, Openings, Roofs,
  Annotate, Inspect, Plan`. Held as `FramerApp::command_tab` (`mod.rs:66`).
  Tab strip rendered by `toolbar` (`panels.rs:193`) from
  `AUTHORING_WORKFLOW_TABS` (`panels.rs:3589`) + `OUTPUT_WORKFLOW_TABS = [Plan]`
  (`panels.rs:3599`); labels at `panels.rs:3602`. Switching calls
  `select_workflow_tab` (`panels.rs:287`). Ribbon body is the
  `match self.command_tab` in `workflow_command_panels` (`panels.rs:300`).
- **`ViewportMode`** (`mod.rs:180`) — `Plan, RoofPlan, Elevation, Axonometric,
  Render`. Held as `FramerApp::viewport_mode` (`mod.rs:69`). View strip rendered
  by `viewport_tabs` (`viewport/mod.rs:442`, `view_tab` helper `:641`) inside
  `workspace_header` (`:430`); renderer dispatch is the big match at
  `viewport/mod.rs:112-276`. Also switchable via `execute_action`
  (`mod.rs:1221-1225`, `ActionId::ViewPlan/…/ViewRender`).
- **`WorkspaceMode`** (`mod.rs:165`) — `Design, Plan`, with
  `allows_design_edits()` (`mod.rs:171`) used as the edit gate throughout
  `mod.rs`, and `shows_generated_plan()` (`mod.rs:175`). `set_workspace_mode`
  (`mod.rs:1067`) keeps `command_tab` consistent.

Coupling today: **none** between the first two except that every tool toggle
(`toggle_draw_wall_tool` `mod.rs:1721`, `toggle_room_tool` `:2293`,
`toggle_ceiling_tool` `:2317`, `toggle_vault_tool` `:2336`, `toggle_floor_tool`
`:2354`, `toggle_dimension_tool` `panels.rs:492`) force-sets workspace +
`command_tab` + `viewport_mode` on activation. `handle_view_click`
(`mod.rs:3341`) promotes Plan→Elevation when a wall is clicked in design mode.

Gating today: `action_enabled` (`mod.rs:1116`) / `action_disabled_reason`
(`mod.rs:1156`) gate Undo/Redo, exports (Plan-workspace-only), Delete, and the
Add-opening/roof actions (Design-workspace-only) — but the ribbon's
`action_tool_button` calls in `workflow_command_panels` pass a literal `true`,
and reasons only surface in header menus + command search (`panels.rs:466-472`).

Render state: `render_view` (CPU fallback), `render_gpu`, cooldown at
`mod.rs:77-82`; `draw_project_render` (`viewport/render.rs:61`) builds
`framer_render::RenderOptions` (`render.rs:135`) from camera fields only,
filling sun/sky/exposure with `..RenderOptions::default()`.

---

## PR 0 — Docs: spec updates + this plan

Already prepared in the working tree alongside this file (command-surfaces
requirements/matrix/decisions for A+B+C+E; render-view "In-app render settings"
section).

- **Task 0.1** — Commit the spec updates and this plan; add the plan row to
  [docs/plans/README.md](README.md).
  - Files: `docs/specs/command-surfaces.md`, `docs/specs/render-view.md`,
    `docs/plans/2026-07-08-view-workflow-alignment.md`, `docs/plans/README.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: `docs(specs): lock view–workflow alignment decisions; add plan`

## PR 1 — Render becomes an output workflow tab (A)

After this PR: the tab row reads `Design Frame Openings Roofs Annotate │ Render
Plan`; clicking `Render` shows the path-traced viewport with (for now) an empty
ribbon; the view strip no longer contains `Render` and is hidden while in the
Render workspace; leaving Render restores the previous authoring view.

- **Task 1.1** — Add the enum variants and workspace plumbing.
  - Add `WorkflowTab::Render` (`actions.rs:94`) and its label ("Render",
    `panels.rs:3602`); move it into `OUTPUT_WORKFLOW_TABS = [Render, Plan]`
    (`panels.rs:3599`). Add `WorkspaceMode::Render` (`mod.rs:165`) —
    `allows_design_edits()` stays `matches!(self, Self::Design)` and
    `shows_generated_plan()` stays Plan-only, which automatically blocks model
    edits in the Render workspace via the existing ~20 `allows_design_edits`
    guards. Grep for every `match` on `WorkspaceMode` and handle the new
    variant deliberately (no `_ =>` catch-alls that silently treat Render as
    Design).
  - Extend `select_workflow_tab` (`panels.rs:287`) and `set_workspace_mode`
    (`mod.rs:1067`): `WorkflowTab::Render` ↔ `WorkspaceMode::Render`. On
    entering the Render workspace: deactivate active tools (mirror the existing
    Plan-entry dimension-tool deactivation), stash the current authoring view
    in a new field (e.g. `last_authoring_viewport: ViewportMode`, default
    `Plan`, never stores `Render`), and set
    `viewport_mode = ViewportMode::Render`. On leaving (to an authoring tab or
    Plan): restore the stashed view.
  - Files: `crates/framer-app/src/app/actions.rs`,
    `crates/framer-app/src/app/panels.rs`, `crates/framer-app/src/app/mod.rs`
  - Verify: `cargo test -p framer-app` — add state tests: select Render tab →
    workspace Render + viewport Render; select Frame → previous view restored;
    tool flags cleared on entry.
  - Commit: `feat(app): make Render an output workflow tab with its own workspace`
- **Task 1.2** — Remove Render from the view strip; keep palette routing.
  - `viewport_tabs` (`viewport/mod.rs:442`): drop the Render tab; return early
    (render nothing) when the workspace is Render. Keep the existing
    workspace-dependent `Shell/Plan` + `Wall/Elevation` label logic
    (`viewport/mod.rs:445-447`).
  - `execute_action` (`mod.rs:1221-1225`): `ActionId::ViewRender` now routes
    through `select_workflow_tab(WorkflowTab::Render)` (so command search still
    reaches Render); update its `ActionMetadata` category/tooltip in
    `actions.rs` so search shows it as a workspace, not a view. The other
    `View*` actions must not be executable from the Render workspace unless
    they also leave it — simplest is: executing an authoring `View*` action
    from Render first restores the authoring workspace.
  - Files: `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/actions.rs`
  - Verify: ui_harness test — Render workspace shows no view strip; metadata
    tests still pass; `scripts/ui-shots.sh` and read the toolbar + Render
    frames (add a "render workspace" state to `ui_shots_tests.rs`).
  - Commit: `feat(app): remove Render from the view strip; route via workflow tab`

## PR 2 — Soft default views per authoring tab (B)

- **Task 2.1** — Pure mapping + "does the current view serve this tab" rule.
  - Add a small pure function (in `mod.rs` or a tiny helper module), e.g.
    `fn default_view_for_tab(tab, has_selected_wall) -> Option<ViewportMode>`
    plus `fn view_serves_tab(tab, view) -> bool`, per the spec table:
    Design/Frame → `Plan`; Openings/Annotate → `Elevation` **only when a wall
    is selected** (an elevation needs a wall camera — with none selected, stay
    put; `handle_view_click` `mod.rs:3341` already promotes Plan→Elevation on
    wall click); Roofs → `RoofPlan`. `Axonometric` serves every authoring tab;
    a tab's own default serves it; `Elevation` serves Openings/Annotate;
    `Plan` serves Design/Frame; `RoofPlan` serves Roofs.
  - Call it from `select_workflow_tab` after the workspace handling: switch
    `viewport_mode` only when `!view_serves_tab(new_tab, current_view)`. Never
    applied for output tabs (PR 1 owns those).
  - Files: `crates/framer-app/src/app/mod.rs`,
    `crates/framer-app/src/app/panels.rs`
  - Verify: unit-test the full tab × view matrix of the pure functions; state
    tests for two flows: (Frame, 3D) → Openings keeps 3D; (Frame, Shell) →
    Roofs switches to Roof.
  - Commit: `feat(app): workflow tabs set a soft default view`

## PR 3 — Registry-driven applicability gating (C)

- **Task 3.1** — Enabling context as metadata.
  - Add an enabling-context field to `ActionMetadata` (`actions.rs:127`) — a
    small enum is enough (e.g. `EnabledContext { Always, Authoring,
    PlanWorkspace, … }`), not a predicate framework. Populate it for every
    entry in `ACTIONS` (`actions.rs:208`): all modal tools
    (Wall/Room/Ceiling/Vault/Floor/Dimension) and insertion variants
    (doors/windows/garage/roof forms) → `Authoring`; exports stay
    `PlanWorkspace`; views/undo/redo keep their existing special-case logic.
  - Rewrite `action_enabled` (`mod.rs:1116`) to read the metadata for the
    context-gated cases (keeping the stateful gates — undo stack, selection —
    as code), and extend `action_disabled_reason` (`mod.rs:1156`) with the
    matching copy, e.g. "Available in an authoring workflow tab — Render and
    Plan are output workspaces."
  - Files: `crates/framer-app/src/app/actions.rs`,
    `crates/framer-app/src/app/mod.rs`
  - Verify: metadata test — every `ActionId` has an enabling context; state
    test — `action_enabled(ToolWall)` is false in Render/Plan workspaces, true
    in Design.
  - Commit: `feat(app): move command enabling context into action metadata`
- **Task 3.2** — Every surface honors it (kills the hard-coded `true`).
  - `workflow_command_panels` (`panels.rs:300`): pass
    `self.action_enabled(id)` instead of literal `true` to every
    `action_tool_button`; disabled buttons show the
    `action_disabled_reason` tooltip (today reasons only surface in menus +
    search, `panels.rs:466-472`). Guard `execute_action` centrally so a
    disabled action dispatched from any surface (palette, shortcut) is a no-op.
  - Keep auto-snap: tool toggles still force view/tab *within* the authoring
    workspace (`toggle_draw_wall_tool` `mod.rs:1721` etc. keep their
    `set_workspace_mode(Design)` + view-snap behavior — reachable only when
    the action is enabled, i.e. never from output workspaces).
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/mod.rs`
  - Verify: ui_harness — in the Plan workspace the Frame tab's Wall button
    renders disabled with a tooltip; palette dispatch of a disabled action does
    nothing; full gates.
  - Commit: `feat(app): gate command surfaces on action enabling context`

## PR 4 — Render settings ribbon (sun / environment)

The Render tab's strip gets its first real contents, surfacing the engine
fields the app currently defaults (`RenderOptions { exposure, sun, sky }`,
`crates/framer-render/src/build.rs:56-71`). These are value controls in compact
panels (tool-options style), not action buttons — the ≤5 top-level action
budget is unaffected.

- **Task 4.1** — Session settings state.
  - Add a `RenderSettings` struct on `FramerApp` near the render state
    (`mod.rs:77-82`): sun azimuth °, sun elevation °, exposure (f32s are fine —
    this is UI/view state, not the authored model). Defaults must reproduce
    `RenderOptions::default()` exactly so default output (goldens, GPU↔CPU
    parity) is unchanged. Not serialized anywhere.
  - Files: `crates/framer-app/src/app/mod.rs`
  - Verify: unit test — `RenderSettings::default()` → `RenderOptions` equals
    `RenderOptions::default()` for the mapped fields.
  - Commit: `feat(app): add session render settings state`
- **Task 4.2** — Ribbon panels + plumbing into both render paths.
  - In `workflow_command_panels` (`panels.rs:300`), give the
    `WorkflowTab::Render` arm two compact panels — "Sun" (azimuth 0–360°,
    elevation, drag values) and "Environment" (exposure) — using the design
    tokens/widgets (`design/widgets.rs`); match the Structure panel's density.
  - Plumb `RenderSettings` into the `RenderOptions` built at
    `viewport/render.rs:135` for **both** the GPU path and the CPU fallback,
    and fold the settings into the existing camera/scene-change detection that
    resets progressive accumulation (find the reset/hash mechanism in
    `viewport/render.rs` / `app/render/` — a settings change must restart
    accumulation like a camera move does).
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/render.rs`,
    `crates/framer-app/src/app/design/widgets.rs` (only if a new drag-row
    widget is needed)
  - Verify: `cargo test --workspace --all-features --locked` (goldens + parity
    untouched at defaults); ui-shots state for the Render tab strip;
    interactive: install-app, drag sun azimuth → image visibly re-lights and
    re-accumulates, CPU fallback behaves identically with GPU disabled.
  - Commit: `feat(app): sun and environment controls on the Render tab`

## PR 5 — View strip restyle as a segmented control (E)

- **Task 5.1** — A real segmented control, authoring cameras only.
  - Add a compact segmented-control widget to
    `crates/framer-app/src/app/design/widgets.rs` (token-driven, both themes,
    low-radius per the design system's CAD-density direction) and use it in
    `viewport_tabs` (`viewport/mod.rs:442`, `view_tab` helper `:641`),
    preserving the workspace-dependent `Shell/Plan` + `Wall/Elevation` labels
    and the registry-driven tooltips. If this introduces a new widget
    convention, add a line to
    [design-system.md](../specs/design-system.md).
  - Files: `crates/framer-app/src/app/design/widgets.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`,
    `docs/specs/design-system.md` (possibly)
  - Verify: ui-shots before/after on light + dark decks (all view states);
    ui_harness reachability at the minimum `1040×680` viewport.
  - Commit: `feat(app): render view tabs as a segmented control`

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
scripts/ui-shots.sh   # read the full deck, light + dark
```

Plus: GPU↔CPU parity tests still pass (`framer-app/tests/gpu_parity.rs`,
skips without an adapter); one interactive install-app pass over the whole
flow — tab switching (default views), Render tab (settings + accumulation
reset), disabled tooltips in output workspaces.

Close the loop: update the **Status** / **Last reviewed** lines in
[command-surfaces.md](../specs/command-surfaces.md) and
[render-view.md](../specs/render-view.md) (drop the "not yet built" notes),
and refresh [code-map.md](../code-map.md) for any new files/types
(`RenderSettings`, segmented control, enabling-context enum).
