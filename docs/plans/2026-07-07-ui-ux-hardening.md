# UI/UX Hardening — Implementation Plan (2026-07-07)

> **Implementation plan** (point-in-time). **Specs:**
> [design-system.md](../specs/design-system.md) and
> [command-surfaces.md](../specs/command-surfaces.md) (both updated 2026-07-07 with the
> locked decisions this plan implements). This file is an archival record of how the
> work was sequenced; the specs are the durable source of truth.

## Goal

Fix everything found by the 2026-07-07 full UI/UX review (screenshot analysis + live
walkthrough of every mode, view, menu, and selection type in the installed app):
rendering bugs that make the app look broken, dishonest/dead status readouts, a
three-row navigation structure with coupled state, and cross-panel inconsistencies in
units, labels, naming, and widget styling. Split into ten independent PRs, ordered by
user-facing severity. Each PR leaves the workspace green.

## How to work this plan (read first)

- Read [AGENTS.md](../../AGENTS.md) before starting. All work is in `framer-app` only —
  **never add UI dependencies or UI-driven changes to `framer-core`, `framer-library`,
  `framer-solver`, `framer-standards`, or `framer-render`**. The one core type this plan
  *uses* (read-only) is `framer_core::units::Length`'s existing `Display`.
- Gates before every commit (from the workspace root):
  `cargo fmt --all -- --check` ·
  `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` ·
  `cargo test --workspace --all-features --locked`.
- **Visual verification:** prefer `scripts/ui-shots.sh` (the `ui-shots` skill) —
  it renders an 18-frame deck of the real UI off-screen to `target/ui-shots/*.png`
  in ~15 s, covering every workflow tab, view, selection state, the palette, the
  Project menu, and dark shots. Where a task below says "screenshot", regenerate
  the deck and read the relevant PNG; add states to
  `crates/framer-app/src/app/ui_shots_tests.rs` if the deck doesn't cover what you
  changed. Only *interactive* checks (drags, hover, live toggling, camera feel)
  need the `.claude/skills/install-app` skill (`scripts/install-app.sh`, then drive
  `~/Applications/Framer.app`, bundle id `dev.framer.app`; `cargo run` windows are
  invisible to screenshot tooling).
- Headless UI tests live in `crates/framer-app/src/app/ui_harness_tests.rs`
  (egui_kittest). Gotchas: build the UI through `FramerApp::ui_root`, and warm up
  fonts before asserting on text (see existing tests for the pattern).
- File/line anchors below were verified on 2026-07-07 at commit `f5951aa`. Lines
  drift — re-locate with `grep -n` before editing; the symbol names are stable.
- One PR per slice, branch `fix/ui-<slice-name>` or `feat(app)` style used by recent
  history; commit messages given per task. Do not fold slices together.

## Architecture / stack summary

- `crates/framer-app/src/app/design/` — token system: `tokens.rs` (roles + scales),
  `palette.rs` (`studio_light`/`studio_dark`, all hex lives here), `widgets.rs`
  (reusable controls), `mod.rs` (`active()`, `set_theme`, `toggle_theme`,
  `configure_style` → `ctx.set_global_style`).
- `crates/framer-app/src/app/panels.rs` — `app_header` (:33), `toolbar` (:179),
  `model_tree` (:441), `inspector` (:1398), `status_bar` (:2782), plus helpers
  (`panel_header` :3360, `panel_subheader` :3458, `property_row` :5501,
  `length_drag_spec` :6010).
- `crates/framer-app/src/app/viewport/mod.rs` — `workspace` (:92),
  `canvas_view_controls` (:295), `canvas_context_toolbar` (:336),
  `workspace_header` (:420), `workspace_switcher` (:446), `viewport_tabs` (:458).
  2D drawing overlays in `viewport/plan.rs`, `viewport/view_common.rs`; elevation in
  `viewport/elevation_*.rs`; 3D in `viewport/axonometric.rs` + `view_cube.rs`.
- `crates/framer-app/src/app/actions.rs` — `ActionId` + `ActionMetadata` (label,
  tooltip, surfaces, workflow tab). Command search reads this.
- `crates/framer-app/src/app/mod.rs` — `set_workspace_mode` (:1035),
  `action_enabled` (:1088), Cmd/Ctrl+K (:1169), global Escape consumption (:1195),
  `ui_root` (:3420, panel frames).

---

## PR 0 — Docs: spec corrections + this plan

Already prepared in the working tree alongside this file (spec "UI conventions
(locked)" section, corrected theme-storage section, workspace-control and
diagnostics-routing decisions).

- **Task 0.1** — Commit the spec updates and this plan; add the plan row to
  [docs/plans/README.md](README.md).
  - Files: `docs/specs/design-system.md`, `docs/specs/command-surfaces.md`,
    `docs/plans/2026-07-07-ui-ux-hardening.md`, `docs/plans/README.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: `docs(specs): lock UI conventions + workspace-control decisions; add UI/UX hardening plan`

## PR 1 — Theme & popup rendering correctness (P0)

The observed bugs, all confirmed live on 2026-07-07: (a) the header's Project and
Examples menu-button labels are invisible (ghost text on a light pill); (b) menus,
the Examples dropdown, and the command palette render translucent — canvas/tabs bleed
through the item rows; (c) clicking the header theme toggle flips its icon to a moon
but the app body stays light; (d) some section labels never render on the default
light theme — "Wall joins" and "Systems" in the Model Browser and "Join point" in the
corner inspector — yet appear after toggling the theme and *remain* after toggling
back; (e) the theme choice does not persist across launches.

- **Task 1.1** — Root-cause where egui 0.35 actually reads styles from and make
  `configure_style` land there. `design::set_theme` → `configure_style` →
  `ctx.set_global_style(style)` (`design/mod.rs:49-158`) looks correct but the body
  provably does not restyle on toggle. Prime suspect: egui 0.35 keeps **per-theme
  style slots** selected by `ThemePreference` (which eframe defaults to following the
  OS), so the single global style is overridden by egui's own light/dark selection.
  Fix so that after `toggle_theme` every surface (panel fills, menus, popups, text)
  flips. Setting `ctx.set_theme(...)` / pinning `ThemePreference` to our choice is an
  acceptable mechanism.
  - Files: `crates/framer-app/src/app/design/mod.rs`
  - Verify: the ui-shots dark deck (`17-dark-frame-shell.png`,
    `18-dark-wall-selected.png`) currently reproduces the bug exactly — dark
    header, white panels, phantom "Wall joins" header — so iterate against it;
    after the fix those shots must render fully dark. Then install-app once to
    confirm the *live* toggle flips both ways.
  - Commit: `fix(app): make theme switching restyle the whole app`
- **Task 1.2** — Opaque, elevated popups. After 1.1, open Project menu, Examples
  menu, the Level dropdowns, snap combo, layers popover, and command palette on both
  themes: every popup must be fully opaque (`theme.overlay`/`theme.panel` — both are
  opaque RGB in `palette.rs`) with a visible shadow. If any still bleed, fix the
  popup/menu fill wiring in `configure_style` (`window_fill`, menu styling) rather
  than per-call-site hacks.
  - Files: `crates/framer-app/src/app/design/mod.rs`
  - Verify: ui-shots (`15-command-palette.png`, `16-project-menu.png`, dark shots)
    show opaque popups — note kittest already renders the Project menu opaque
    while the live app shows bleed-through, so the live symptom likely involves
    eframe window compositing/transparency; confirm the final result with
    install-app, which ui-shots cannot substitute for here.
  - Commit: `fix(app): opaque elevated menus and popups`
- **Task 1.3** — One palette per widget in the forced-dark header. `app_header`
  (`panels.rs:33`) builds `head = design::studio_dark()` and passes it into helpers;
  the menu *labels* (`header_menu_text`) take `head.text` (near-white) while the
  MenuButton pill *fill* comes from the global (light) style — hence invisible
  labels. Make every header child style text AND fill from `head` (extend
  `header_menu_text`/the MenuButton usage or add a `header_menu_button` widget in
  `design/widgets.rs`).
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/design/widgets.rs`
  - Verify: ui-shots `01-frame-shell.png` (and the dark shots) show "Project" and
    "Examples" crisply legible in the dark header. Add a `ui_harness_tests.rs`
    case asserting the header contains visible "Project"/"Examples" button labels.
  - Commit: `fix(app): readable header menu buttons on both themes`
- **Task 1.4** — Diagnose and fix the never-rendering section labels: "Wall joins",
  "Systems" (`panel_subheader`, `panels.rs:3458`, used in `model_tree`) and the
  "Join point" heading in the corner inspector. Symptom is theme-toggle-history
  dependent, so suspect stale/wrongly-sourced color or a caching effect rather than
  conditional logic — but verify by reproducing first (the ui-shots light deck:
  `01-frame-shell.png` should show "Wall joins" above the corners and
  `10-corner-selected.png` a "Join point" heading; today they're missing).
  - Files: `crates/framer-app/src/app/panels.rs` (+ wherever the root cause lives)
  - Verify: ui-shots light deck shows all three labels; add a
    `ui_harness_tests.rs` case asserting "Wall joins" renders in the default state.
  - Commit: `fix(app): render tree and inspector section headers on light theme`
- **Task 1.5** — Persist the theme choice via eframe storage (there is currently no
  `eframe::App::save` persistence of it) and restore it in `FramerApp::new`
  (`app/mod.rs:545` currently hardcodes `studio_light`).
  - Files: `crates/framer-app/src/main.rs`, `crates/framer-app/src/app/mod.rs`,
    `crates/framer-app/src/app/design/mod.rs`
  - Verify: toggle dark, quit, relaunch → still dark.
  - Commit: `feat(app): persist theme choice across launches`
- **Task 1.6** — Guardrail test: add a unit test in `design/` that walks every
  text-role/surface-role pairing actually used (text on panel, text_secondary on
  panel, text on control, text_on_accent on accent, etc., for both palettes) and
  asserts a minimum WCAG-ish contrast ratio (≥ 3.0 relative luminance ratio is
  enough to catch white-on-white regressions).
  - Files: `crates/framer-app/src/app/design/tokens.rs` (tests module)
  - Verify: `cargo test -p framer-app design`
  - Commit: `test(app): contrast guardrail for theme token pairings`

## PR 2 — Honest status bar + reachable diagnostics (P0)

Observed: zoom is a hardcoded `"100%"` string; "Ready" is a hardcoded label
duplicating the header save pill; the cursor readout hardcodes `Z 0.000 ft` and uses
decimal feet (a third unit format); the errors/warnings/unsupported/info counters are
plain labels — clicking them does nothing, and diagnostics are only readable in the
Plan-mode inspector.

- **Task 2.1** — Live zoom. Replace the literal (`status_bar`, `panels.rs:2828-2832`)
  with the actual 2D camera zoom percentage (see `viewport/camera_2d.rs` for the
  scale state; 100% = the default fit scale) and the 3D view's equivalent or hide it
  in 3D. Update it as the camera changes.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/camera_2d.rs` (read-only accessor)
  - Verify: unit-test the accessor (camera scale → displayed percentage); then
    install-app and zoom the plan by hand (pinch/Cmd+scroll) to watch the readout
    change — this one is inherently interactive. Note: synthetic Cmd+scroll from
    screenshot tooling pans instead of zooming.
  - Commit: `fix(app): status-bar zoom reflects the real camera`
- **Task 2.2** — Remove the hardcoded "Ready" status metric (`panels.rs:2789`) — the
  header save pill already owns app/file status — and drop the fake `Z 0.000 ft`
  from the cursor readout, formatting X/Y with `framer_core::units::Length` display
  (ft-in-fraction) instead of decimal feet (`panels.rs:2812-2824`).
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: status bar shows `X 14' 3 5/8"  Y 24' 0"`-style readouts; no Ready, no Z.
  - Commit: `fix(app): truthful status-bar readouts in canonical units`
- **Task 2.3** — Diagnostics popover. Make the four counters
  (`panels.rs:2914-2932`) one clickable control that opens a popover listing the
  current diagnostics (the same data `diagnostic_counts()` and the Plan inspector
  use — see the Diagnostics block rendered in Plan mode), each row: severity icon,
  human message, affected object name; clicking a row selects that object when it
  maps to one. Keep the counts visible in the bar. This implements the new
  "Diagnostics" routing-matrix row in
  [command-surfaces.md](../specs/command-surfaces.md).
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: from the **Design** workspace, click the warnings counter → popover lists
    the demo project's 3 warnings; clicking one selects the affected opening. Add a
    `ui_harness_tests.rs` reachability case.
  - Commit: `feat(app): clickable status-bar diagnostics popover`

## PR 3 — Command-surface behavior (P0/P1)

- **Task 3.1** — Escape closes the command palette. Today the global Escape handler
  (`app/mod.rs:1195`) consumes the key, so the palette stays open (observed live;
  only the ✕ closes it). Give the open palette first claim on Escape (check
  `command_search.open` before the global consume, or handle inside
  `command_search_overlay`).
  - Files: `crates/framer-app/src/app/mod.rs`
  - Verify: open palette (⌘K or the search icon), press Escape → closes; Escape
    still cancels an active wall tool when the palette is closed.
  - Commit: `fix(app): Escape dismisses the command palette first`
- **Task 3.2** — Humanize palette entries. Rows currently show internal routing
  like "Project / App header" and "Shell — Examples / Examples". Show a
  human category (e.g. "Project", "Edit", "Examples", "View") and the shortcut
  where one exists (⌘Z, ⌘K…); drop surface names entirely. The strings come from
  `ActionMetadata` in `actions.rs` — add a `search_category` (or reuse the workflow
  tab/menu name) rather than printing the surface enum.
  - Files: `crates/framer-app/src/app/actions.rs`, palette rendering in
    `crates/framer-app/src/app/mod.rs`
  - Verify: palette rows read like "New — Project ⌘N"-style; no "App header" text
    anywhere. Metadata unit tests in `actions.rs` still pass.
  - Commit: `fix(app): human-readable command palette entries`
- **Task 3.3** — Disabled-reason tooltips. `Export` and `Compliance CSV` are
  disabled outside the Plan workspace (`project_header_menu`, `panels.rs:128-160`;
  gating at `app/mod.rs:1088-1090`) with no explanation. Add hover text on disabled
  menu items/palette rows stating the enabling context ("Available in the Plan
  workspace"). Apply the same pattern to any other `action_enabled`-gated command.
  - Files: `crates/framer-app/src/app/panels.rs`, `crates/framer-app/src/app/mod.rs`
  - Verify: hover disabled Export in Design workspace → tooltip explains; switch to
    Plan tab → enabled and working.
  - Commit: `fix(app): disabled commands explain their enabling context`
- **Task 3.4** — Ribbon tooltips. The workflow-strip tool buttons (Wall/Ceiling/
  Vault/Floor etc., `action_tool_button`, `panels.rs:3294`) show no tooltip on
  hover (verified live, 2s hover). Wire `ActionMetadata.tooltip` (+ shortcut if
  any) onto every strip button; fill in missing tooltip strings in `actions.rs`.
  - Files: `crates/framer-app/src/app/panels.rs`, `crates/framer-app/src/app/actions.rs`
  - Verify: hover each strip tool on each tab → tooltip appears; metadata test that
    asserts every strip-routed action has a non-empty tooltip.
  - Commit: `fix(app): tooltips on all workflow-strip tools`

## PR 4 — Navigation merge + stable chrome (P1, structural)

Locked decision (see [command-surfaces.md](../specs/command-surfaces.md#decisions-locked)):
the workflow tabs are the single workspace control; the standalone workspace switcher
row is removed. The coupling already exists one-way (`panels.rs:227-233`: Plan tab →
Plan workspace, other tabs → Design workspace) — this PR removes the redundant second
control and stabilizes the chrome.

- **Task 4.1** — Delete the workspace switcher. Remove `workspace_switcher`
  (`viewport/mod.rs:446-456`) and `workspace_mode_tab` (:657) from
  `workspace_header` (:420); keep `viewport_tabs`. Keep
  `ActionId::WorkspaceDesign/WorkspacePlan` working via command search
  (`app/mod.rs:1140-1141`) — `set_workspace_mode` must sync `command_tab` **both**
  ways (`app/mod.rs:1035-1051` currently syncs Plan; ensure switching to Design
  picks a sensible authoring tab, e.g. the last-used or `Design`).
  - Files: `crates/framer-app/src/app/viewport/mod.rs`, `crates/framer-app/src/app/mod.rs`
  - Verify: no "Design Workspace / Plan Workspace" row; clicking Plan tab shows Plan
    workspace + relabeled view tabs; clicking Frame returns; command-search
    "Workspace: Plan" still works; existing workspace-mode unit tests updated/green.
  - Commit: `feat(app): workflow tabs are the single workspace control`
- **Task 4.2** — Style Plan as an output tab: right-align it (or divider-separate)
  in the tab row (`toolbar`, `panels.rs:182-191`) so authored-vs-generated reads at
  a glance. Hide the `Inspect` tab until it has commands (it currently renders an
  empty strip — verified live; keep the enum, filter it from `WORKFLOW_TABS`
  rendering with a comment).
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: screenshot; Plan visually separated; no Inspect tab; command-strip
    budget metadata tests still pass.
  - Commit: `feat(app): separate Plan output tab; hide empty Inspect tab`
- **Task 4.3** — Stable chrome heights. Switching tabs must not reflow the panels:
  give the command-panel row a constant height across tabs (including Plan). Render
  the transient status chips (`panels.rs:199-221`, e.g. "Reset to multi-wall demo
  shell") as an overlay toast (egui `Area` over the canvas, auto-dismiss) instead of
  an inline row that pushes everything down (observed: Model Browser jumps ~33px).
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: click through all tabs watching the Model Browser header — it must not
    move; trigger a toast (Examples → Shell) — panels don't shift.
  - Commit: `fix(app): constant chrome height; transient status as overlay toast`

## PR 5 — Inspector consistency (P1)

Observed: mixed unit formats *within one section* (opening Center `7.0 ft`, Width
`48 in`) and across modes (wall Length `28.0 ft` in Design, `28' 0"` in Plan);
dropdown rows put the label *after* the control (Level, Kind, First/Second wall);
value pills look disabled; "Remove Opening" is styled like any button; raw id shown
as the first line; no empty-selection state (and clicking empty canvas doesn't
deselect — canvas part lands in PR 7).

- **Task 5.1** — Canonical length display. All inspector length fields display via
  `framer_core::units::Length`'s `Display` (ft-in-fraction). The drag-value helpers
  (`length_drag_spec` :6010, `driven_length_drag` :6022, `length_drag` :6135) take
  per-call unit literals (`"ft"`/`"in"` at :1655, :1666, :1811-1838) — replace the
  display path with the canonical formatter (entry can still accept plain numbers
  in the field's native unit; parsing stays as-is).
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: wall Length shows `28' 0"`; opening Width shows `4' 0"`; unit tests for
    the format helper; Plan-mode summary and Design-mode fields now agree.
  - Commit: `fix(app): canonical ft-in length display across the inspector`
- **Task 5.2** — Label-left rows. Rebuild the Level/Kind/First wall/Second wall rows
  with `property_row` (label left at `PROPERTY_LABEL_WIDTH`, :5499) instead of
  control-then-label.
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: every inspector row reads label-left; screenshot.
  - Commit: `fix(app): label-left property rows for dropdown fields`
- **Task 5.3** — Editable affordance + destructive tone. Give value fields a
  visible field treatment (field fill + border + hover cursor via tokens — they
  currently read as disabled gray pills), and style `Remove Opening` (and any other
  destructive inspector action) with the `danger` tone.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/design/widgets.rs`
  - Verify: fields visibly editable at a glance; Remove Opening reads destructive.
  - Commit: `fix(app): visible edit affordance; danger tone for destructive actions`
- **Task 5.4** — Ids demoted, empty state added. Replace the raw-id first line
  (`inspector_object_id`, :3389) with a muted "ID: …" row at the *bottom* of the
  inspector (click-to-copy is a nice-to-have), and render a friendly empty state
  when nothing is selected ("Nothing selected — click a wall, opening, or corner").
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: select wall → name first, id at bottom; clear selection (after PR 7,
    Escape) → empty-state text.
  - Commit: `fix(app): demote object ids; inspector empty state`

## PR 6 — Naming & identity (P1)

- **Task 6.1** — One name: **Corner**. The inspector badge says "Join", the
  breadcrumb says `Join: join-back-left`, the tree says "Corner". Rename all
  user-facing "Join" strings to "Corner" (inspector badge, breadcrumb,
  `selection_status`, labels in `app/labels.rs`); "join" stays in code/ids.
  - Files: `crates/framer-app/src/app/panels.rs`, `crates/framer-app/src/app/labels.rs`
  - Verify: grep user-visible strings; select a corner → every surface says Corner.
  - Commit: `fix(app): one user-facing name for wall corners`
- **Task 6.2** — Names, not ids, in the breadcrumb. `selection_status`
  (used at `panels.rs:2807`) prints raw ids for openings/joins
  (`Opening: opening-back-left-window` observed). Use the object's display name.
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: select the back-left window → status bar shows `Opening: Back left window`.
  - Commit: `fix(app): status-bar breadcrumb uses object names`
- **Task 6.3** — Tree rows: icon + name. Replace the `"Type: Name"` label pattern
  in `model_tree` (e.g. `"Wall segment: {}"` at :602, `"Corner: {}"`,
  `"Level: {}"`, and Library's `"Name (Type)"`) with a type glyph (existing `Icon`
  enum; add domain glyphs where Lucide is too generic) followed by the name; keep
  the type in the row tooltip. Keep row height compact per the CAD-density spec.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/design/icons.rs`
  - Verify: screenshot tree — names lead, types read from icons; existing tree
    harness tests updated.
  - Commit: `feat(app): icon-led model tree rows`
- **Task 6.4** — Explain "unsupported". Keep the term (it is product language per
  the vision) but add a tooltip on the counter/popover rows: "Conditions outside
  the supported prescriptive scope — see diagnostics".
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: hover the unsupported counter → explanation.
  - Commit: `fix(app): explain the unsupported diagnostics category`

## PR 7 — 2D canvas overlays + selection lifecycle (P1/P2)

- **Task 7.1** — Corner labels only when relevant. The four always-on blue "Corner"
  labels (`plan.rs:631-642`) collide with the ruler and read as permanent selection.
  Show a corner's label only on hover, when selected, or when the Layers "Joins"
  toggle is on; render in a quiet color (not the selection accent).
  - Files: `crates/framer-app/src/app/viewport/plan.rs`
  - Verify: default plan shows no Corner labels; hover/select a corner → label;
    Layers → Joins on → all labels.
  - Commit: `fix(app): corner labels on demand, not always-on`
- **Task 7.2** — Context toolbar placement. The floating duplicate/delete toolbar
  (`canvas_context_toolbar`, `viewport/mod.rs:336-389`, fixed `anchor - (40,44)`)
  overlaps the selected wall's name label. Offset it above the selection's bounding
  box and clamp inside the canvas; never cover the selection or its label.
  - Files: `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: select Back wall → toolbar sits clear of the label; select an opening →
    same.
  - Commit: `fix(app): context toolbar avoids selection labels`
- **Task 7.3** — Remove the floating 2D/3D dropdown (`canvas_view_controls`,
  `viewport/mod.rs:310-333`) per the routing decision — view tabs + nav cube cover
  it (the dropdown's popup also opened *over* the nav cube). Keep the nav cube, and
  fix its clipped N/S/E/W letters (`draw_nav_cube`, `viewport/mod.rs:730-783` —
  letters are drawn at the widget edge; inset them).
  - Files: `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: no dropdown; nav cube letters fully visible; cube click still enters 3D.
  - Commit: `fix(app): drop duplicate 2D/3D dropdown; unclip nav cube labels`
- **Task 7.4** — Selection lifecycle. Clicking empty canvas clears the selection;
  Escape also clears it when no tool/palette is active (mind the Escape priority
  order from Task 3.1). Pairs with the PR 5 inspector empty state.
  - Files: `crates/framer-app/src/app/viewport/mod.rs` (canvas hit handling),
    `crates/framer-app/src/app/mod.rs`
  - Verify: select wall → click empty canvas → tree deselects, inspector shows
    empty state; add a harness test if the hit path is testable headless.
  - Commit: `feat(app): click-empty and Escape clear the selection`
- **Task 7.5** — Axis gizmo/scale-bar tidy: route the gizmo's hardcoded RGB axis
  colors (`view_common.rs:201-228`) through tokens, and nudge the gizmo so it
  doesn't overlap the origin corner marker.
  - Files: `crates/framer-app/src/app/viewport/view_common.rs`
  - Verify: screenshot bottom-left; no overlap; colors follow theme.
  - Commit: `fix(app): tokenized, non-overlapping axis gizmo`

## PR 8 — View completeness: elevation, roof, 3D, render (P2)

- **Task 8.1** — Elevation (Wall view) labels: openings show their names (currently
  generic "Window"), and the stray `28' 0" x 8' 0"` caption floating mid-canvas gets
  a proper home (view title area or dimension line, not free-floating). See
  `viewport/elevation_openings.rs` / `elevation_dimensions.rs` / `elevation_design.rs`.
  - Files: `crates/framer-app/src/app/viewport/elevation_*.rs`
  - Verify: Wall view of Back wall names both windows; caption anchored sensibly.
  - Commit: `fix(app): named openings and anchored caption in elevation view`
- **Task 8.2** — Roof view empty state. With no roofs, the Roof view silently
  renders the wall plan under the title "Roof plan" (observed). Use
  `draw_view_empty` (`view_common.rs:75-83`): "No roofs yet — add one in the Roofs
  tab", with the shell drawn dimmed underneath if cheap.
  - Files: `crates/framer-app/src/app/viewport/plan.rs` (roof branch)
  - Verify: demo shell (no roof) → Roof view shows the empty-state message.
  - Commit: `fix(app): roof view empty state`
- **Task 8.3** — 3D and Render fill the viewport. Both draw into a letterboxed
  inset box floating in dead gray space (observed ~500px box in a much larger
  canvas). Make them fill the central panel like the 2D views (keep aspect handling
  inside the render target sizing, not by shrinking the widget). Check `workspace`
  (`viewport/mod.rs:92-293`) for where the inset rect is allocated, then
  `axonometric.rs` / `render.rs`.
  - Files: `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/viewport/axonometric.rs`,
    `crates/framer-app/src/app/viewport/render.rs`
  - Verify: 3D and Render occupy the full canvas at multiple window sizes; GPU
    parity test still green (`cargo test -p framer-app --test gpu_parity`).
  - Commit: `fix(app): 3D and render views fill the viewport`
- **Task 8.4** — Render progress legibility: the "Rendering — N/M spp" caption is
  light-gray-on-light (near invisible). Draw it in a readable chip (overlay fill +
  text token), same spot.
  - Files: `crates/framer-app/src/app/viewport/render.rs`
  - Verify: progress readable over a bright render on both themes.
  - Commit: `fix(app): readable render progress indicator`

## PR 9 — Visual hierarchy & widget consistency (P2, after PR 4)

- **Task 9.1** — Surface hierarchy: panels get a slightly deeper ground than the
  canvas so the drawing is the brightest surface (adjust `panel`/`app_bg`/`canvas`
  relationships in `palette.rs` only — both palettes). Keep the warm-paper canvas.
  - Files: `crates/framer-app/src/app/design/palette.rs`
  - Verify: screenshot both themes; contrast test from Task 1.6 still green.
  - Commit: `feat(app): panel/canvas surface hierarchy`
- **Task 9.2** — Accent discipline: selection keeps the accent; tabs stop looking
  like hyperlinks (restyle `widgets::tab` :235 and `workflow_tab` :115 — selected
  state via fill/underline weight in neutral+accent, unselected in text colors, no
  link-blue text), and non-selection canvas elements (corner dots/labels,
  dimension text) move to quieter tokens.
  - Files: `crates/framer-app/src/app/design/widgets.rs`,
    `crates/framer-app/src/app/viewport/plan.rs`
  - Verify: screenshot — the only saturated blue on a default plan is the selection.
  - Commit: `feat(app): reserve the accent for selection and active state`
- **Task 9.3** — Widget unification: Catalog `+ Door`/`+ Window`/`+ Garage Door`
  (`panels.rs:823-831`) use plain `ui.button` — restyle via `design::widgets`
  (or fold into an "Add" flyout consistent with the Openings tab); inert badges
  (panel-header chips "Design"/"Wall"/"Join" at `panel_header` :3360) restyle as
  flat labels so they stop reading as buttons; `status_chip`'s raw
  `Color32::from_rgb` fills (:4013-4020) route through tokens; hardcoded font sizes
  in chrome (`panel_header` 17.0 :3363, `status_metric` 13.0 :4038, subheader 12.0
  :3463, nav-cube/canvas `FontId` literals) move to `design::text_size` constants.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/design/*`, `crates/framer-app/src/app/viewport/*`
  - Verify: `grep -rn "Color32::from_rgb" crates/framer-app/src/app --include=*.rs`
    finds only palette.rs + data-driven material swatches;
    `grep -rn "FontId::proportional" crates/framer-app/src/app/viewport` clean or
    token-routed. Screenshot pass.
  - Commit: `refactor(app): route stray chrome styling through design tokens`
- **Task 9.4** — Ribbon density: drop group captions that repeat the tab name
  (Openings→"Openings", Roofs→"Roofs" in `workflow_command_panels` :237-307), and
  trim the strip's vertical padding toward the CAD-density target.
  - Files: `crates/framer-app/src/app/panels.rs`
  - Verify: screenshot vs current; strip visibly tighter; no repeated captions.
  - Commit: `fix(app): denser command strip without redundant captions`

---

## Review-finding → PR traceability

| Review finding (2026-07-07) | PR |
| --- | --- |
| Invisible Project/Examples labels; invisible section headers | 1 |
| Translucent menus/palette; theme toggle no-op; no persistence | 1 |
| Fake 100% zoom; hardcoded Ready; fake Z; dead diagnostics counters | 2 |
| Escape doesn't close palette; internal palette metadata; disabled Export unexplained; no ribbon tooltips | 3 |
| Three nav rows / Plan-tab workspace hijack; empty Inspect tab; layout jumps | 4 |
| Mixed units; label-after-control; disabled-looking fields; raw ids; no empty inspector state | 5 |
| Corner vs Join; ids in breadcrumb; `Type: Name` tree rows; "unsupported" jargon | 6 |
| Always-on Corner labels; trash button overlap; 2D/3D dropdown dup; clipped nav cube; no deselect | 7 |
| Generic "Window" labels; floating caption; Roof view masquerade; letterboxed 3D/Render; invisible progress | 8 |
| Flat surface hierarchy; blue overload; link-like tabs; plain Catalog buttons; inert chips look clickable; magic sizes/colors | 9 |

## Final verification

After each PR and at the end:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Plus: GPU parity after PR 8 (`cargo test -p framer-app --test gpu_parity`), the
markdown link check for PR 0, a `scripts/ui-shots.sh` deck review after every PR
(both themes are in the deck), and one final install-app *interactive* pass at the
end — drags, hover, snapping, live theme toggle, camera feel — which the deck
cannot cover. When done, update both specs' **Status**/**Last reviewed** and
`docs/code-map.md` where module responsibilities moved (workspace switcher removal,
diagnostics popover).
