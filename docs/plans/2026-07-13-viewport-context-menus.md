# Viewport Context Menus — Implementation Plan (2026-07-13)

> **Implementation plan** (point-in-time). **Specs:**
> [Command Surfaces](../specs/command-surfaces.md) and
> [Component Visibility, Multi-Selection, and Isolation](../specs/component-visibility-and-isolation.md).
> This file is an archival record of how the work was sequenced; the specs are
> the durable source of truth.

## Goal

Add the first right-click selection menu to interactive 3-D, backed by a shared
context/model/renderer contract that a future Model Browser menu can reuse
without sharing viewport-only command composition. Keep menu actions routed
through the existing `ActionId` enablement and dispatch paths, and leave a clean
migration seam for a contribution registry if independently owned menus later
need one.

## Architecture / stack summary

`FramerApp::workspace` already owns the active `ViewportMode`, selection
dispatch, and canvas context toolbar. `draw_project_axonometric` already resolves
3-D picks to `ViewClick`. A focused app-only `context_menu` module will own the
surface context, menu model, surface-specific builders, and one renderer. The
3-D viewport will return primary and secondary pick intent to `FramerApp`, which
will preserve an existing multi-selection when its member is right-clicked and
replace selection when a different component is right-clicked. The renderer
will read `ActionMetadata` and the existing `action_enabled` /
`action_disabled_reason` contract; it will not become a second command bus.

## Risk ledger

| Contract | Boundary | Required proof | Likely PR failure if missed |
| --- | --- | --- | --- |
| Right-click on an unselected 3-D component replaces selection before commands run | 3-D picking → app selection → menu | App test for authored component replacement plus UI reachability | Menu actions operate on the previous selection |
| Right-click on a member of a multi-selection preserves the frozen selected set | 3-D picking → ordered component selection | App test with two authored components | CAD-standard multi-selection collapses unexpectedly |
| Generated members use the same context path as authored hosts | scene pick identity → `ComponentKey` → menu target | Generated-member context-selection regression | Menu works for walls but not generated Plan geometry |
| Empty canvas and the ViewCube do not expose a selection menu | 3-D hit testing → popup trigger | Axonometric hit-test regression and UI smoke coverage | Stale selection menu opens away from geometry |
| Context-menu commands keep one enablement/dispatch truth | menu renderer → `actions.rs` → `FramerApp` | Model/renderer tests plus UI action dispatch | Labels work but disabled state or action behavior diverges from search/toolbar |
| Canvas and Model Browser menus remain separate surfaces | menu context → surface-specific builder | Builder test and durable spec language | Browser inherits 3-D-only isolation commands |
| A future registry can replace builder internals without replacing UI or actions | surface builder → shared model/renderer | Typed context/model API and unit tests for section/order composition | Premature registry or later rewrite of every menu call site |
| Existing 2-D right-click tool cancellation is unchanged | Plan tool input → `ViewClick::DrawWallCancel` | Existing workspace tests plus diff review | Opening a menu interrupts wall-run cancellation |
| Popup remains accessible and Escape dismisses it before selection/tool state | egui popup → keyboard routing/access tree | UI harness labels and Escape regression | Closing the menu also clears selection or active isolation |
| Change remains disposable app presentation state | `framer-app` → core/solver/render/schema | Diff review and full workspace tests | Context state leaks into authored intent or `.framer` |

## Slices / phases

### Slice 1 — Durable surface contract

- **Task 1.1** — Update command-surface, component-visibility, and design-system
  specs with right-click selection semantics, separate canvas/browser builders,
  shared model/renderer ownership, and the staged registry migration.
  - Files: `docs/specs/command-surfaces.md`,
    `docs/specs/component-visibility-and-isolation.md`,
    `docs/specs/design-system.md`, `docs/code-map.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: `docs(ui): specify viewport context menus`

### Slice 2 — Shared menu foundation and 3-D interaction

- **Task 2.1** — Add typed menu surface/target context, sections/items, a 3-D
  surface builder, and one renderer that consumes `ActionId` availability.
  - Files: `crates/framer-app/src/app/context_menu.rs`,
    `crates/framer-app/src/app/actions.rs`, `crates/framer-app/src/app/mod.rs`
  - Verify: focused context-menu model and action-routing tests
  - Commit: `feat(ui): add shared context menu model`
- **Task 2.2** — Return secondary-pick intent from axonometric drawing, apply
  right-click selection rules in `FramerApp`, and show the 3-D presentation menu
  without affecting Plan tool right-click behavior.
  - Files: `crates/framer-app/src/app/viewport/axonometric.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: focused app and UI harness tests
  - Commit: `feat(viewport): open selection menu on right click`

### Slice 3 — Visual and regression proof

- **Task 3.1** — Cover the real right-click menu, Escape behavior, authored and
  generated context selection, and surface-specific composition.
  - Files: `crates/framer-app/src/app/ui_harness_tests.rs`, app unit tests
  - Verify: `cargo test -p framer-app --bin framer-app --all-features --locked`
  - Commit: included with Slice 2
- **Task 3.2** — Add a 3-D right-click menu state to the screenshot deck and
  inspect the rendered PNG for density, opacity, labels, and placement.
  - Files: `crates/framer-app/src/app/ui_shots_tests.rs`
  - Verify: `scripts/ui-shots.sh` plus direct PNG inspection
  - Commit: `test(ui): cover viewport context menu`

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
scripts/ui-shots.sh
```

GPU parity is not a feature gate for this app-only menu/input slice; run it only
if implementation unexpectedly changes scene construction, render material/math,
or the path-traced Render workspace.
