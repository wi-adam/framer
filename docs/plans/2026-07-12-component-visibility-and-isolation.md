# Component Visibility, Multi-Selection, and Isolation — Implementation Plan (2026-07-12)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/component-visibility-and-isolation.md](../specs/component-visibility-and-isolation.md).
> This file is an archival record of how the work was sequenced; the spec is the
> durable source of truth.

## Goal

Deliver ordered multi-selection, model-browser visibility controls, and
hide/dim isolation for authored assemblies, semantic rough-opening/corner
groups, and generated members in the interactive 3-D authoring/Plan views.

## Architecture / stack summary

`FramerApp` owns session selection and presentation state. The model browser and
viewport return selection intent to that owner. `Scene3d::from_project_with_geometry`
already rebuilds app-side geometry and picks every frame, so it can classify
components before emission without changing `BuildingModel`, solver output, the
physical scene, or path-traced Render.

## Risk ledger

| Contract | Boundary | Required proof | Likely PR failure if missed |
| --- | --- | --- | --- |
| Ordered replace/toggle/clear selection, stable wall ids, primary item | app state → browser/viewport/inspector | Unit tests for two walls, two members, toggle removal, empty canvas, Escape, and undo restoration | UI says multi-select but only one path or wall index actually participates |
| Generated leaf identity differs from semantic provenance | solver plan → app selection/isolation | Exact-member host/id regression plus opening-source group test | Opening isolation selects the wall or cannot find physical member bodies |
| Stale presentation keys are harmless after rebuild | authored/derived regeneration → app session state | Delete/regenerate prune tests for authored and generated keys | Hidden or selected ghosts survive deletion and break later actions |
| Hidden components emit no geometry or picks | presentation state → scene builder/picking | Member, wall/opening, roof/ceiling/floor scene tests | Invisible objects still win clicks or child framing leaks through hidden hosts |
| Dimmed geometry uses real alpha | scene builder → GPU opaque/transparent partition | Index/alpha assertions and UI shots in both isolation modes | Alpha stays in the opaque pass and looks fully solid |
| Outline walls match filled geometry behavior | scene builder → egui painter overlay | Hidden/dimmed outline edge tests | Outline mode ignores isolation while Width/Full works |
| Rough-opening group is precise | authored opening → `FrameMember.source` | King/jack/header/cripple included; common studs excluded | Isolating a door shows all wall framing or misses header pieces |
| Browser eyes stay reachable/accessibly named | panels → session visibility | UI harness for Show/Hide authored and generated rows; narrow UI shot | State works in code but cannot be discovered or reversed |
| Selection lifecycle commands use the documented home | action metadata → context toolbar/search | Metadata and UI harness reachability tests | Commands leak into permanent workflow chrome or search dispatch is dead |
| Render remains unaffected and commands unavailable there | app workspace → separate path tracer | App action-context test and diff review | Interactive filters accidentally imply filtered final output |
| No schema/solver state changes | app presentation → core/solver | Diff review, project round trips, full workspace tests | Disposable view state becomes a second persisted source of truth |

## Slices / phases

### Slice 1 — Stable ordered component selection

- **Task 1.1** — Add stable wall ids to selection, ordered multi-selection state,
  centralized replace/toggle/clear operations, stale-key reconciliation, and
  complete snapshot restore.
  - Files: `crates/framer-app/src/app/mod.rs`,
    `crates/framer-app/src/app/history_integration_tests.rs`
  - Verify: focused selection and undo tests
  - Commit: `feat(app): support ordered component multi-selection`
- **Task 1.2** — Route browser and 2-D/3-D clicks through the common selection
  operation; Command/Ctrl toggles, additive selection suppresses automatic
  single-wall navigation, and empty 3-D canvas clears.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/viewport/axonometric.rs`
  - Verify: app unit tests plus UI harness multi-select coverage
  - Commit: included with Task 1.1

### Slice 2 — Visibility resolution and 3-D emission

- **Task 2.1** — Add session hidden/isolation state and one leaf appearance
  resolver for authored hosts, exact generated members, and semantic
  `FrameMember.source` groups.
  - Files: `crates/framer-app/src/app/mod.rs` or a focused app child module
  - Verify: resolver unit tests including rough-opening membership and stale keys
  - Commit: `feat(viewport): resolve component visibility and isolation`
- **Task 2.2** — Route normal/dimmed/hidden leaves through scene building, merge
  dimmed geometry into the transparent pass, and apply the same state to outline
  edges and picks.
  - Files: `crates/framer-app/src/app/viewport/scene_build/*.rs`,
    `crates/framer-app/src/app/viewport/axonometric.rs`
  - Verify: focused scene tests for every component family, opacity partition,
    rough-opening group, and pick behavior
  - Commit: included with Task 2.1

### Slice 3 — Browser and contextual command surfaces

- **Task 3.1** — Add accessible eye controls to renderable authored/generated
  component rows; hidden rows remain available to show.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/design/icons.rs`
  - Verify: UI harness authored/generated visibility tests
  - Commit: `feat(browser): add component visibility controls`
- **Task 3.2** — Register isolate dim/hide, exit, hide-selection, and show-all
  presentation commands; expose them from the compact selection context surface
  and command search with truthful enabled state.
  - Files: `crates/framer-app/src/app/actions.rs`,
    `crates/framer-app/src/app/mod.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: action metadata tests and UI harness reachability/dispatch
  - Commit: included with Task 3.1
- **Task 3.3** — Show all selected rows and a read-only multi-selection inspector
  summary while keeping edit/delete/duplicate single-selection only.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: UI harness summary and action gating tests
  - Commit: included with Task 3.1

### Slice 4 — Visual QA and durable navigation

- **Task 4.1** — Add screenshot-deck states for multi-selection, one hidden row,
  isolate-dim, isolate-hide, and dark theme. Replace any screenshot-only plan
  mutation used to fake isolation with real presentation state where practical.
  - Files: `crates/framer-app/src/app/ui_shots_tests.rs`
  - Verify: `scripts/ui-shots.sh` and direct PNG inspection
  - Commit: `test(ui): cover component isolation states`
- **Task 4.2** — Mark the spec implemented and update concrete app/viewport
  ownership navigation.
  - Files: `docs/specs/component-visibility-and-isolation.md`,
    `docs/specs/README.md`, `docs/specs/command-surfaces.md`,
    `docs/specs/design-system.md`, `docs/code-map.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: included with Task 4.1

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
scripts/ui-shots.sh
```

`cargo test -p framer-app --test gpu_parity --locked -- --nocapture` is not a
feature gate for this app-only rasterized 3-D presentation change; run it if the
diff unexpectedly reaches path-traced Render or shared render material/math.
