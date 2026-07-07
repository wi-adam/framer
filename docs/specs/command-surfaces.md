# Command Surfaces

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** G-011 (CAD Workspace UX) ·
> **Plan:** [2026-07-03 command surfaces](../plans/2026-07-03-command-surfaces.md),
> [2026-07-07 UI/UX hardening](../plans/2026-07-07-ui-ux-hardening.md) ·
> **Last reviewed:** 2026-07-07

## Intent / Purpose

Framer needs a command system that scales beyond the current full-width button row
without drifting into a soft SaaS dashboard. The target is a **compact parametric
CAD workbench**: dense, technical, flat, and optimized for repeated modeling work.
The command row below the dark app header is implemented today as
`FramerApp::toolbar` in `crates/framer-app/src/app/panels.rs`, but the product
target is a **workflow command strip**: tabbed by workflow, grouped into compact
panels, and able to switch into contextual tool tabs.

This spec defines Framer's **command surfaces**: the app/quick-access bar,
workflow command strip, workspace/view controls, browser/catalog panels,
PropertyManager-style inspector, canvas marking/context menus, command search,
status/view-control bar, and shortcuts. The goal is a predictable CAD workspace
where commands are discoverable, dense enough for expert use, contextual enough
to avoid permanent chrome sprawl, and routed by process before they add UI.

## Reference CAD pattern

Framer should borrow the interaction grammar of mainstream parametric CAD tools
without cloning any one product:

- Autodesk Fusion separates the application bar, workflow toolbar, browser,
  canvas, ViewCube, marking menu, navigation bar, and timeline; its toolbar varies
  by workspace and contextual environment.
- Onshape uses workflow-dependent top toolbars, vertical group dividers, flyouts,
  responsive collapse, tool search, a feature list, dialogs, and context menus.
- SOLIDWORKS centers command access around the CommandManager, FeatureManager,
  PropertyManager, Task Pane, context toolbars, shortcut menus, and status bar.
- Autodesk Inventor uses a ribbon / quick-access / model-browser / property-panel
  pattern with customizable command density, icon sizes, text display, navigation
  bar, ViewCube, and marking menus.

Reference sources: Autodesk Fusion's
[desktop interface overview](https://help.autodesk.com/cloudhelp/ENU/Fusion-GetStarted/files/GS-THE-FUSION-INTERFACE.htm),
Onshape's [user interface basics](https://cad.onshape.com/help/Content/Home/user_interface_basics.htm),
SOLIDWORKS' [user interface overview](https://help.solidworks.com/2026/english/swtutorialonline/c_tut_user_interface_overview_start.htm?id=3.0),
and Inventor [UI refresh](https://help.autodesk.com/cloudhelp/2022/ENU/Inventor-WhatsNew/files/GUID-333B7827-5CD7-4E79-810A-5BD1274254E6.htm)
and [customization](https://resources.ascented.com/ascent-blog/customizing-the-inventor-interface)
notes.

Framer's command surfaces should therefore feel more like Fusion/Inventor/
SOLIDWORKS density with Onshape's workflow-aware web responsiveness: small
technical controls, restrained color, sparse shadows, low-radius chrome, compact
tool groups, and model-first canvas space.

## Art direction

Framer should not become a generic gray CAD clone. The visual target is
**CAD density plus Framer craft**: precise and compact like a professional CAD
workbench, but purpose-built for wood-framed structures.

- **Palette:** graphite app chrome, cool gray command surfaces, warm off-white
  drawing paper, sparse blue selection, construction amber for authored framing
  intent, green for valid completion, and red only for conflicts/destructive
  actions.
- **Icons:** utility commands can use the bundled Lucide-style line icon set, but
  framing/domain commands need custom mechanical icons: stud wall, rough opening,
  roof pitch, joist bay, layer stack, section cut, generated frame, and BOM.
- **Command strip:** panels should feel machined and technical: tight group
  spacing, crisp dividers, subtle active states, compact flyout arrows, and no
  pill/card proportions.
- **Browser:** use small domain glyphs, disclosure triangles, visibility toggles,
  lock/status dots, and authored/generated badges so the model tree reads like a
  feature tree instead of a file explorer.
- **Canvas:** invest visual polish in the drawing surface: rulers, strong major
  grid rhythm, precise snap/hover highlights, quiet labels, clear handles, and a
  ViewCube/nav widget with real faces and edges.
- **Property Manager:** keep fields dense, but use section headers, validation
  badges, accept/cancel affordances, and wall-type/material swatches to make the
  inspector feel like a CAD tool rather than a web form.
- **Brand presence:** Framer-specific style should come from framing semantics and
  drawing craft, not oversized branding, decorative gradients, or marketing-like
  illustration.

## Requirements & behavior

- Framer uses **workflow command strip** as the product/docs name for the current
  top command surface. "Primary toolbar" can describe the current implementation,
  but specs and future plans should use "workflow command strip." Avoid "control
  bar" because it is too vague; avoid "ribbon" unless Framer later adopts a full
  Office-style ribbon.
- Every product-visible command has a documented command home before it is added to
  UI: primary surface, secondary surface, enabled context, icon, label, tooltip,
  shortcut if any, undo label if model-mutating, and owning module.
- The workflow command strip is tabbed by workflow (`Design`, `Frame`, `Openings`,
  `Roofs`, `Annotate`, `Inspect`, `Plan`) and each tab contains compact command
  panels with small icons, short labels, dividers, and flyout arrows where variants
  belong. A workflow tab that currently has no commands ships **hidden** (today:
  `Inspect`) rather than rendering an empty strip. The strip keeps a constant
  height across tabs so switching never reflows the panels below.
- The workflow tab strip is also the **workspace control**: selecting `Plan`
  enters the Plan workspace, selecting any authoring tab returns to the Design
  workspace, and the `Plan` tab is visually separated as an output tab. There is
  no standalone workspace switcher row.
- Disabled commands state their enabling context in a tooltip ("Available in the
  Plan workspace"), on every surface (menus, strip, palette).
- Command search entries display a human-readable category and the command's
  shortcut — never internal surface names ("App header"). Escape closes the
  palette (topmost transient surface dismisses first).
- Contextual tool tabs or tool option strips replace broad permanent buttons when
  a tool is active. For example, activating Wall can show wall type, baseline,
  height, level, and placement options in a dense strip.
- The workflow command strip is not the default home for project file operations,
  sample loaders, selection lifecycle actions, deep property edits, export
  variants, generated reports, diagnostics, or rarely used object variants.
- A command can appear on more than one surface only when each surface has a clear
  job. For example, Delete can be a shortcut and context-menu action; it does not
  also need permanent command-strip space.
- Contextual commands appear near context: selection lifecycle actions belong in
  marking menus / shortcut menus / compact context toolbars; selected-object edits
  belong in the inspector; insertion variants belong in a catalog, flyout, or
  host-aware Insert chooser.
- View and drafting state belongs where it explains the current view: viewport
  tabs/header for major view changes, status/view-control bar for snap/grid/ortho/
  layer visibility, navigation bar / ViewCube for camera controls, and inspector
  only for selected-object state.
- The command strip must remain visually scannable at the default desktop width.
  If adding a command would force another broad group or oversized button, the
  feature must use a flyout, contextual tab, catalog, property panel, command
  search, or marking menu instead.
- The command-strip budget is intentionally small: at most **five top-level
  command-strip actions per workflow tab**. The default desktop viewport is
  `1360 x 860`, and the narrow/minimum supported viewport is `1040 x 680`.
  At the narrow width command panels may wrap, but tabs, flyouts, contextual
  actions, and command search must stay reachable. Adding commands beyond this
  budget requires moving variants to flyouts/context/search instead of widening
  permanent chrome.
- The visual language is compact and technical: small square icon buttons,
  1-pixel dividers, subdued gray panels, low-radius controls, minimal shadows,
  sparse accent color, and no card-like toolbar buttons.
- The style layer adds warmth and domain specificity without reducing density:
  drawing-paper canvas, framing amber, custom construction icons, precise grid/
  ruler treatment, and visible authored-vs-generated semantics.
- Every command that mutates authored intent goes through existing undo/redo
  transaction semantics and exposes a human-readable history label.
- Hidden or overflowed commands remain reachable through a discoverable alternate
  route: command search, menu, context menu, inspector action, catalog action, or
  keyboard shortcut.

## Command routing matrix

| Command kind | Primary surface | Secondary surface | Does not belong |
| --- | --- | --- | --- |
| Project/document actions (`New`, `Open`, `Save`, profile, help, theme) | App/quick-access bar or project menu | Command search / shortcuts | Workflow command strip |
| Sample/demo loaders | Project menu or examples picker | Command search | Workflow command strip |
| Workspace mode (`Design`, `Plan`) | Workflow tab strip (`Plan` tab = Plan workspace) | Command search | A second standalone switcher row |
| View mode (`Shell`, `Wall/Elevation`, `Roof`, `3D`, `Render`) | Workspace/view tabs or view bar | Command search / shortcuts | Mixed into modeling panels; duplicate floating canvas dropdowns |
| Diagnostics (errors, warnings, unsupported, info) | Status-bar counters → diagnostics popover | Plan workspace inspector / command search | Visible-but-dead counters; buried in a single workspace |
| Drafting/view state (`Grid`, `Snap`, `Ortho`, layers) | Status/view-control bar | Command search | Inspector property rows |
| Modal authoring tools (`Wall`, `Room`, `Ceiling`, `Floor`, `Dimension`) | Workflow command strip tab/panel | Shortcuts / command search | Inspector |
| Tool settings (`Driving` vs `Reference`, placement mode, wall baseline) | Contextual tool tab or options strip | Inspector when selection-backed | Permanent global buttons |
| Object insertion variants (`Door`, `Window`, `Garage`, roof forms) | Command-strip flyout, catalog, or host-aware Insert | Command search | Permanent top-level buttons |
| Selection lifecycle (`Delete`, `Duplicate`, `isolate/show`) | Marking menu / shortcut menu / compact context toolbar | Shortcuts / command search | Permanent command-strip buttons |
| Selected-object properties | PropertyManager-style inspector | Canvas handles where direct manipulation helps | App header |
| Exports, generated reports, plan artifacts | Plan workspace tab or project menu | Command search | Always-visible Design command strip |

## Process for adding a command

1. Classify the command using the routing matrix before editing `panels.rs`.
2. Record the command's metadata in the relevant spec or plan: id, label, icon,
   tooltip, shortcut, owner, enabled context, primary surface, secondary route,
   undo behavior, and verification.
3. If the command would be command-strip visible, name its workflow tab, compact
   panel, and whether it is a top-level button or flyout variant. If it cannot fit
   into a compact grouped panel, use a contextual, catalog, menu, or search route.
4. If the command mutates authored intent, name the undo transaction label and the
   test or manual check proving the action is undoable.
5. If the command introduces a new surface or changes toolbar layout, update this
   spec, [design-system.md](design-system.md), and [code-map.md](../code-map.md).

## Decisions (locked)

- **Name:** the visible top command surface is the **workflow command strip**.
  "Command surfaces" names the broader system.
- **CAD workbench over SaaS dashboard:** the visual target is compact, dense,
  technical, low-radius, flat, and model-first.
- **Framer craft over generic CAD clone:** visual personality comes from
  wood-framing semantics, custom domain icons, warm drawing-paper canvas, and
  precise construction feedback.
- **Default route:** new commands do not default to the workflow command strip.
  They default to the narrowest surface matching their context.
- **Tabs and panels before loose groups:** command-strip commands live in workflow
  tabs and compact panels. Variants use flyouts, contextual tabs, tool option
  strips, catalogs, or placement previews.
- **Context before permanence:** selection and host-specific actions appear near
  selected or hovered geometry instead of occupying permanent chrome.
- **Workflow tabs are the workspace control:** the standalone "Design Workspace /
  Plan Workspace" switcher row is removed (decided 2026-07-07 after the UI/UX
  review found the three stacked nav rows with coupled state unlearnable). The
  `Plan` workflow tab *is* the Plan workspace; view tabs remain the view-mode
  control; the floating on-canvas 2D/3D dropdown is removed (nav cube + view tabs
  cover it).
- **Search as universal backstop:** once implemented, command search is the route
  for commands that are useful but not worth permanent chrome.
- **No core command dependency:** command metadata and rendering live in
  `framer-app`; `framer-core`, `framer-solver`, and `framer-render` remain UI-free.

## Architecture (grounded in the codebase)

- `crates/framer-app/src/app/panels.rs` currently owns `app_header`, `toolbar`,
  inspector bodies, and `status_bar`. `app_header` owns quick-access project/edit
  controls plus Project and Examples menus; `toolbar()` renders the workflow tab
  row (`Design`, `Frame`, `Openings`, `Roofs`, `Annotate`, `Inspect`, `Plan`)
  plus compact modeling/generated command panels, insertion flyouts for opening
  and roof-form variants, and the app-header / Cmd/Ctrl+K command-search modal.
  View/workspace switching and active tool options live in workspace-adjacent
  chrome.
- `crates/framer-app/src/app/design/widgets.rs` owns reusable controls such as
  `tool_button`, `tool_group`, `icon_button`, `toggle_switch`, and inspector
  sections. It should evolve toward compact CAD primitives (small command buttons,
  split buttons, flyouts, panel headers, option rows), while command-routing policy
  stays in this spec.
- `crates/framer-app/src/app/viewport/mod.rs` owns the workspace/view bar,
  contextual tool options strip, workspace header, and selection context toolbar;
  it is the right home for workspace/view switching, active placement settings,
  context actions, marking menu hooks, navigation controls, and tool feedback
  that depend on canvas selection or placement state.
- `crates/framer-app/src/app/actions.rs` holds lightweight command metadata
  (`ActionId`, workflow tab, panel, surfaces, labels, icons, flyout membership,
  and intent-mutation flags) without becoming a generic command bus. Actual
  mutations continue routing through existing `FramerApp` methods such as
  `toggle_draw_wall_tool`, `add_opening`, `add_roof`, `delete_selected`, `undo`,
  and `redo`; command search reads this metadata and dispatches back through
  those same app action paths.
- Headless UI tests in `crates/framer-app/src/app/ui_harness_tests.rs` cover
  smoke-level reachability for core command surfaces, including app-header
  quick access, workflow-strip tabs/panels, flyouts, contextual selection
  actions, command search, active tool options, and minimum-window reachability.
  Pure command metadata tests cover duplicate ids, missing labels/tooltips,
  flyout routes, non-strip routing, and the top-level command-strip budget.

## Constraints & invariants

- This is presentation and interaction structure only; it does not change the
  `.framer` schema or any authored model semantics.
- Core, solver, library, and render crates stay UI-free.
- Authored intent mutations still flow through model-edit helpers and undo/redo
  transactions. Derived framing/render/export state remains regenerated.
- Existing keyboard shortcuts remain valid unless a plan explicitly migrates them.
- Accessibility metadata remains explicit for icon-only or glyph-only triggers.
- Dense UI does not mean hidden UI: every icon-only command needs a tooltip,
  accessible label, and searchable command name.

## Out of scope (YAGNI)

- A plugin/extension command API.
- A fully generic command-dispatch framework or serialized command log.
- A full Office-style ribbon with large gallery panels.
- User-customizable toolbars.
- Command telemetry.

## Open questions

- Should object insertion use a single catalog surface for doors/windows/roof forms,
  or separate host-aware Add menus in the canvas and inspector?
