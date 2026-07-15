# Component Visibility, Multi-Selection, and Isolation

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** G-003 (Viewport Interaction) / G-011
> (CAD Workspace UX) · **Plan:**
> [2026-07-12 component visibility and isolation](../plans/2026-07-12-component-visibility-and-isolation.md),
> [2026-07-13 viewport context menus](../plans/2026-07-13-viewport-context-menus.md) ·
> **Last reviewed:** 2026-07-14

## Intent / Purpose

Framing inspection needs a way to reduce a dense whole-building scene to the
parts involved in one construction question. A user must be able to select more
than one authored or generated component, hide individual components, and
isolate the selected set while the rest of the interactive 3-D scene is either
ghosted or removed.

The interaction follows Framer's compact CAD command grammar: stable selection
in the model browser and canvas, eye controls beside component rows, and
isolate/show actions in the selection context surface rather than permanent
workflow-strip chrome.

## Requirements & behavior

- Plain click replaces the current component selection. Command/Ctrl-click
  toggles one component without dropping the rest; the most recently added item
  is the primary selection. Empty-canvas click or Escape clears the whole set
  after Escape has first dismissed any open popup or cancelled an active tool.
- Every selected component is highlighted in the model browser and in each
  interactive 3-D scene that renders that component. Opening, corner, and
  generated-member highlights appear in generated Plan 3-D, where their framing
  geometry exists. The existing single-object inspector is shown only for one
  selected item; multiple items produce a read-only selection summary. Edit handles,
  Duplicate, and Delete remain single-selection actions until a separate batch
  editing contract is specified.
- Authored wall identity is the wall's stable `ElementId`, not its current vector
  index. The active wall index may remain as an elevation/editing context, but it
  does not define membership in a multi-selection.
- A generated member is identified by its owning plan host plus member id. The
  member's `FrameMember.source` remains derivation provenance and is used for
  semantic groups, not leaf identity.
- Selecting an authored opening represents its rough-opening framing group in
  generated Plan 3-D: every member whose `FrameMember.source` equals the opening
  id (king/jack studs, header plies, sill, and cripples) participates in
  selection highlighting and isolation. Common wall framing is not part of that
  group.
- Isolate has two explicit modes:
  - **Dim others** keeps non-isolated components visible as low-opacity ghosts
    and pickable.
  - **Hide others** removes non-isolated geometry and pick targets.
- Isolation captures the selected set when the command is invoked. Later
  selection changes do not silently redefine that isolated set. Exit Isolation
  restores ordinary per-component visibility.
- In interactive 3-D, secondary-clicking a component opens its canvas selection
  menu. A component outside the current selection replaces the selection before
  the menu opens; secondary-clicking a member of the current multi-selection
  preserves the whole selected set. Secondary-clicking empty canvas or the
  ViewCube does not open a component menu or clear selection.
- Wall, roof, ceiling, and floor hosts can be isolated in either Design or Plan
  3-D. Opening rough frames, corners, and exact generated members require Plan
  3-D because Design intentionally omits generated framing. Entering Design with
  one of those Plan-only isolation snapshots exits isolation and reports that
  transition instead of rendering a blank scene.
- A component eye control toggles a session visibility override. A hidden
  component stays present in the browser so it can be shown again. Hidden
  components emit neither interactive 3-D geometry nor pick targets. An explicit
  eye toggle or Hide Selected command exits active isolation first so the manual
  visibility change has an immediate, truthful visual result. Eyes are enabled
  only when the active interactive 3-D workspace renders that component;
  opening and corner eyes therefore become available in generated Plan. The
  browser keeps a stable Show All Components recovery control, enabled whenever
  one or more explicit hidden overrides exist.
- Visibility follows semantic ownership:
  - hiding a wall or roof/ceiling/floor host hides its authored 3-D assembly and
    all generated members owned by that host;
  - hiding an authored opening or corner hides generated members whose
    `FrameMember.source` is that authored id;
  - hiding one generated member affects only that leaf.
- Hidden and isolated state is pruned when model edits or regeneration remove
  referenced components. New, open, and sample-load operations reset it.
- If hide-mode isolation leaves a small set, camera framing is derived from the
  visible set. Dim-mode isolation retains full-scene framing because the ghosted
  context remains part of the scene.
- The first implementation applies to the interactive 3-D authoring and
  generated Plan views. The path-traced Render workspace is not silently
  filtered and exposes no isolation commands in this slice.
- These controls are presentation state. They do not mutate authored intent,
  change solver output, or change `.framer` schema v13. Explicit component
  visibility and isolation actions do produce session-only undo entries. Undo
  snapshots restore the complete visibility/isolation state and component
  selection alongside authored intent without persisting either presentation
  state.

## Command homes

| Command | Primary surface | Secondary surface | Enabled context | Undo |
| --- | --- | --- | --- | --- |
| Toggle component visibility | Model Browser eye | — | Component rendered in active interactive 3-D workspace | Yes |
| Isolate — Dim others | 3-D selection context menu | Context toolbar / command search | Interactive 3-D + component selection | Yes |
| Isolate — Hide others | 3-D selection context menu | Context toolbar / command search | Interactive 3-D + component selection | Yes |
| Exit Isolation | 3-D selection context menu | Context toolbar / command search | Active isolation | Yes |
| Hide selected | 3-D selection context menu | Context toolbar / command search | Component selection | Yes |
| Show all components | Model Browser / selection context menu | Command search | Any hidden override | Yes |

## Decisions (locked)

- **Stable leaf identity, semantic group expansion.** Generated leaves use plan
  host + member id; rough openings and corners expand through
  `FrameMember.source`. Conflating those identities would break physical-body
  lookup and cross-host semantic grouping.
- **Ordered multi-selection with one primary.** A small interaction-ordered set
  preserves the familiar single-object inspector while allowing every selected
  item to drive highlight/isolation.
- **Dimmed context remains pickable.** Ghosting is an inspection aid, not a lock;
  a user can select a ghosted component and deliberately start a new isolation.
- **Hidden means absent from picking.** Invisible geometry cannot win hit tests.
- **Isolation is a frozen snapshot.** Selection remains useful inside an
  isolated scene and does not make the scene composition jump after every click.
- **Presentation-only but undoable state.** Visibility is not construction intent
  and does not belong in `BuildingModel`, `ProjectFramePlan`, or `.framer`.
  Recording explicit visibility actions in ephemeral app history does not make
  the state persistent document data.
- **Interactive 3-D first.** Path-traced Render needs separate component-aware
  scene/BVH and CPU/GPU material work; it is not approximated here.
- **Selection-preserving secondary click.** Opening a menu on an already-selected
  component keeps the ordered selection intact; a different target becomes the
  new selection. This matches the menu's command target without making
  right-click an additive-selection gesture.

## Architecture (grounded in the codebase)

- `crates/framer-app/src/app/mod.rs` owns the ordered component selection and
  session visibility/isolation state. Selection mutation is centralized so
  viewport, browser, diagnostic, creation, and workspace-transition paths use
  the same replace/toggle/clear rules. Explicit visibility mutations share one
  history wrapper and the app snapshot includes the complete visibility state.
- `crates/framer-app/src/app/panels.rs` renders multi-selection state and the
  model-browser component rows. Shared row helpers expose separate selection and
  accessible `Show …` / `Hide …` eye responses.
- `crates/framer-app/src/app/viewport/axonometric.rs` forwards the complete
  selection plus visibility state into the scene builder and reports
  primary and secondary pick intent. Empty primary click returns the same
  clear-selection event as the 2-D Plan view; empty secondary click does not.
- `crates/framer-app/src/app/context_menu.rs` owns the typed menu context/model,
  the 3-D canvas builder, and the shared menu renderer. Surface builders compose
  `ActionId`s while `FramerApp` remains the single source of command enablement
  and dispatch. A future Model Browser builder remains separate from viewport
  composition and can later consume registered contributions without changing
  the model or renderer.
- `crates/framer-app/src/app/viewport/scene_build/` resolves each authored
  assembly/member leaf to normal, dimmed, or hidden before emission. Hidden
  leaves omit geometry and picks. Dimmed leaves are routed after the opaque
  partition through the existing alpha-blended pass; outline walls carry a
  matching painter opacity.
- `FrameMember.source` in `crates/framer-solver/src/lib.rs` supplies rough-opening
  and corner provenance. The solver contract is consumed read-only and does not
  change.

## Constraints & invariants

- `framer-core`, `framer-solver`, `framer-geometry`, and `framer-render` remain
  UI-free and unchanged by presentation state.
- Authored intent remains the only persisted/editable source of truth; generated
  framing and visibility remain disposable.
- Scene filtering must preserve the existing opaque/transparent partition,
  physical member mesh/pick equivalence, pick priorities, and danger highlights.
- Selection and visibility keys are deterministic and tolerate regenerated
  members disappearing.

## Out of scope (YAGNI)

- Persisting visibility in `.framer` or app configuration.
- Batch delete, duplicate, or property editing for heterogeneous selections.
- Range selection based on browser row order, marquee/window selection, and
  cycling coincident 3-D hits.
- Tri-state visibility controls on level/generated-host headers.
- Component-aware filtering or transmissive ghosting in path-traced Render.
- Authoring a temporary suppression as construction intent.
- A Model Browser right-click menu and a runtime/plugin contribution registry.
