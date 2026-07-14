# Design System

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented (evolving) · **Linked goal:** G-011 (CAD Workspace UX) ·
> **Plan:** [2026-07-07 UI/UX hardening](../plans/2026-07-07-ui-ux-hardening.md),
> [2026-07-12 component visibility and isolation](../plans/2026-07-12-component-visibility-and-isolation.md) ·
> **Last reviewed:** 2026-07-12

## Goal

Framer's UI grew organically and has no logically organized design system: `theme.rs`
is ~30 loose, dark-only color functions, spacing/typography/sizing are inline magic
numbers, and components are ad-hoc helpers. Bring the app in line with the approved
direction: a compact parametric CAD workbench with a dark app/quick-access bar,
dense workflow command strip, sectioned browser/property panels, status/view
controls, on-canvas ViewCube/navigation, and restrained technical styling.
Command placement is governed by [Command Surfaces](command-surfaces.md); this spec
owns the visual system and reusable widgets.

## Decisions

- **Theme:** Support **both** a light palette (matching the mockup) and a refreshed
  dark palette, switchable at runtime. Tokens are theme-agnostic; palettes are data.
- **Scope:** **Full functional parity** with the mockup's interactive chrome — Grid /
  Ortho / Snap become real backed state that drives drafting; Snap / Level / 2D-3D
  dropdowns work; the view cube (already present in 3D) becomes a shared on-canvas nav
  widget; the floating canvas toolbar wires to existing ops.
- **Icons:** **Bundle an icon font** (Lucide, MIT, thin-line) registered into egui's
  font stack, referenced through a typed `Icon` enum.
- **Title bar:** Keep native window decorations (real macOS traffic lights) and render
  the dark app header directly beneath — visually equivalent to the mock without going
  frameless and reimplementing window drag/resize.
- **Visual target:** Prefer Fusion/Inventor/SOLIDWORKS/Onshape-like CAD density over
  SaaS-style spacious cards: compact controls, low-radius chrome, small icons,
  subdued gray panels, sparse accent color, and canvas-first composition.
- **Art direction:** Use CAD density plus Framer craft. The UI should feel like a
  technical instrument for framing: graphite chrome, cool gray panels, warm
  drawing-paper canvas, blue selection, construction amber authored intent,
  green valid completion, red conflicts, and custom framing-aware icons.

## Architecture

Replace flat `theme.rs` with a `design/` module in `framer-app`:

- `design/tokens.rs` — `Theme`, a cheap `Copy` struct of semantic tokens + metrics.
- `design/palette.rs` — `Theme::studio_light()` and `Theme::studio_dark()`.
- `design/icons.rs` — bundled font registration + `Icon` enum (`Icon::Save.glyph()`).
  Custom framing icon glyphs or vector paths should live beside the bundled icon
  font and be wrapped by the same typed `Icon` enum.
- `design/widgets.rs` — reusable component builders.
- `design/mod.rs` — re-exports + the active-theme accessor.

### Token model (`Theme`)

Grouped by role so call sites read intent, not hex:

- **Surfaces:** `title_bar`, `toolbar`, `panel`, `panel_header`, `canvas`, `field`,
  `control`, `control_hover`, `overlay`.
- **Text:** `text`, `text_secondary`, `text_muted`, `text_on_accent`.
- **Accent + semantic:** `accent`, `accent_soft`, `success`, `warning`, `danger`.
- **Framing semantics:** `authored`, `generated`, `construction`, `snap`,
  `constraint`, `conflict`, `paper_warm`.
- **Lines:** `divider`, `divider_soft`, `border` (+ `Stroke` helpers).
- **Drawing/canvas:** `paper`, `grid_minor`, `grid_major`, `ruler`, `framing`,
  `framing_dark`, `selection`, `dimension`.
- **Metrics:** spacing scale (`xs=2, sm=4, md=8, lg=12, xl=16`), `radius` (sm/md),
  type scale (`title/heading/body/label/mono`), control sizes (`tool_btn`, `icon_btn`,
  `row_h`), icon sizes.

### Theme storage & switching

The active `Theme` is process state behind `design::active()` (implemented today as a
thread-local `Cell` in `design/mod.rs`; `design::theme(ui)` wraps it). Existing free
functions migrated with a one-line swap (`theme::panel_bg()` → `t.panel`) instead of
threading `&Theme` through every signature. Requirements, independent of the storage
mechanism:

- **Single styling source per widget.** A widget's text, fill, and stroke colors come
  from the same palette. Chrome that forces a fixed palette (the app header renders
  with `studio_dark()` regardless of the active theme) must style *all* of its child
  widgets from that forced palette — never a forced palette's text over fills
  inherited from the global egui style. (This mixing made the header's
  Project/Examples menu labels unreadable on light theme.)
- **Switching restyles everything.** `design::set_theme` must rebuild the style egui
  *actually renders from* — on egui 0.35 that includes the per-theme style slots /
  `ThemePreference` handling, so panel fills, menus, and popups flip together with
  token-reading widgets. The toggle lives in the app header.
- **Persistence.** The chosen theme persists across launches via eframe storage.

## Component library (`design/widgets.rs`)

Each reads `theme(ui)` so it restyles for free:

- `tool_button(Icon, label, active, enabled)` — compact CAD command button; labels
  stay short and buttons avoid card-like proportions.
- `workflow_tab(label, selected)` — compact command-strip workflow tab with a
  restrained active state.
- `split_tool_button(Icon, label, active, enabled, menu)` — command plus flyout for
  variants.
- `command_panel(label, add)` — captioned compact command-strip row for related
  actions.
- `icon_button(Icon, tooltip)` — bare icon (tree footer, status bar, help).
- `toggle_switch(&mut bool, label)` — sliding Grid/Ortho/Snap switch.
- `segmented(Segment[])` — compact mutually exclusive mode control (authoring
  view segments).
- `combo(Icon?, label, …)` — themed dropdown.
- `section(title, default_open, body)` — collapsible inspector group.
- `property_row(label, value_widget)` — left label / right-aligned value field.
- `swatch_field(...)` — wall-type visual stripe + dropdown.
- `chip(text, tone)`, `status_item(...)`, `tab_bar(...)`, `panel_header(...)`.
- Canvas overlays: `marking_menu(actions)`, `context_toolbar(actions)`,
  `view_cube`, `axis_gizmo`, `scale_bar`, `nav_widget`.
- Domain visuals: `wall_glyph`, `opening_glyph`, `roof_glyph`, `layer_stack_glyph`,
  `section_cut_glyph`, `framing_swatch`, and `validation_badge`.

## UI conventions (locked)

Cross-cutting rules every surface follows. A violation of one of these is a bug, not
a style preference:

- **One length format.** User-visible lengths use `framer-core`'s `Length` display
  (feet-inch-fraction: `28' 0"`, `0' 8 3/16"`) everywhere — inspector fields, plan
  summaries, status-bar readouts, canvas dimensions. Entry may accept decimal
  feet/inches; display normalizes. No decimal-feet in one panel and bare inches in
  another for the same kind of quantity.
- **Labels lead.** Property rows are label-left / value-right, including rows whose
  value widget is a dropdown (Level, Kind, First/Second wall).
- **Names, not ids.** Raw model ids (`wall-back`, `opening-back-left-window`) never
  appear as primary UI text in inspector headers, breadcrumbs, or the status bar.
  Ids stay reachable via a muted secondary row or tooltip.
- **One name per concept.** Each concept has a single user-facing name across tree,
  inspector, breadcrumb, and diagnostics — the wall-junction object is **"Corner"**
  in the UI; "join" remains internal vocabulary.
- **Tooltips everywhere.** Every command control — ribbon/command-strip tools
  included, not only icon-only buttons — has a tooltip; commands with shortcuts show
  them. Disabled controls explain their enabling context ("Available in the Plan
  workspace").
- **Empty states.** A view with nothing to show says so and points at the remedy
  ("No roofs yet — add one in the Roofs tab") instead of silently rendering another
  view's content under its own title.
- **Stable chrome.** Chrome heights do not change when switching workflow tabs, and
  transient status (toasts/chips) overlays the canvas rather than reflowing panels.
- **Opaque elevation.** Menus, popups, and the command palette render fully opaque
  with a shadow; underlying content never bleeds through. Escape dismisses the
  topmost transient surface first.
- **Selection lifecycle.** Plain click replaces component selection; Command/Ctrl-
  click toggles stable authored/generated component identities and the most recent
  item becomes primary. Clicking empty canvas or pressing Escape clears the set.
  A multi-selection uses a read-only summary instead of exposing single-object edit,
  duplicate, or delete controls. Destructive actions use the danger tone.
- **Visibility controls.** Renderable authored and generated Model Browser rows
  expose a trailing accessible `Show …` / `Hide …` eye independent of row
  selection. Hidden rows stay in the tree, and active isolation is named in the
  status bar rather than encoded by color alone.
- **Status truth.** The status bar shows live values only — no hardcoded readouts.
  (Zoom must track the camera; readouts that cannot be real yet are removed, not
  faked.)

## Screen-by-screen reskin

- **App / quick-access bar:** dark strip — wordmark · project/document controls ·
  save/undo/redo · command search · profile/code/help/theme.
- **Workflow command strip:** compact tabbed command panels (`Design`, `Frame`,
  `Openings`, `Roofs`, `Annotate`, `Inspect`, `Plan`), small icon buttons,
  flyouts for variants, custom framing icons where Lucide is too generic, and
  contextual tabs/options while a tool is active.
- **Workspace/view bar:** current workspace, view tabs, level selector, display
  preset, and view-layout controls close to the viewport.
- **Model Browser:** search + filter icon, dense disclosure tree, independent
  per-component visibility eyes, multi-selected authored/generated rows, Corners /
  Catalog sections, and compact footer icons.
- **Property Manager / Inspector:** dense field rows, accept/cancel affordances for
  active tools, collapsible sections (Dimensions, Placement, Wall Type, Materials,
  Tags), and wall-type swatches.
- **Canvas:** model-first drawing area; warm drawing-paper surface, rulers, major/
  minor technical grid, authored construction highlights, marking menu / compact
  context toolbar on selection, axis gizmo, ViewCube, scale bar, navigation widget.
- **Status / view-control bar:** `Level ▾` · X/Y live readout · `Snap ▾` ·
  Grid/Ortho toggles · layer/display controls · diagnostics counters that open the
  diagnostics popover · live zoom.
- **Active drafting state:** `Grid`, `Ortho`, `Snap`, cursor readout, and `Level`
  are backed by `FramerApp` presentation state. The active level drives newly authored
  level-owned objects and same-level region picking for room, ceiling, floor, and vault
  tools.

## Phasing (each phase compiles & runs; existing tests stay green)

1. **Foundation** — `design/` module, tokens, light+dark palettes, icon font,
   `configure_style`, theme accessor + toggle. App reskins via the accessor swap.
2. **Chrome** — app/quick-access bar, workflow command strip, workspace/view bar,
   sectioned browser/property panels, status/view-control bar, tree footer.
3. **Wire real state** — add `grid`, `ortho`, `snap_step`, live cursor X/Y/Z, active
   level; make Grid/Ortho/Snap + Snap/Level/2D-3D dropdowns drive drafting and the
   existing viewport; promote the 3D view cube into a shared on-canvas nav widget.
   Active level is implemented by the
   [2026-07-04 active-level drafting](../plans/2026-07-04-active-level-drafting.md)
   slice and same-level region picking by the
   [2026-07-05 level-filtered regions](../plans/2026-07-05-level-filtered-regions.md)
   slice.
4. **Polish** — marking/context actions (select/move/duplicate/delete), scale bar,
   hover/disabled states, command-strip density pass against the CAD workbench mock.

## Verification

- `cargo fmt --all -- --check` and `cargo test --workspace` per phase. Existing
  dimension/IO/workspace-mode tests are the regression net and must stay green.
- New backing logic (snap stepping, active level, theme persistence, cursor mapping) gets
  focused unit tests.
- Manual run + screenshot compare against the mockup.

## Constraints carried from the vision

- Keep `framer-core` and `framer-solver` free of desktop UI dependencies — all of this
  lands in `framer-app`.
- Do not change the intent/derived/presentation layering; this is presentation only.
