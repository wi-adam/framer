# Sustainable Viewport Architecture — Implementation Plan (2026-07-10)

> **Implementation plan** (point-in-time). This is a behavior-preserving refactor,
> so it does not introduce a new feature spec. The durable behavior remains in
> [View Layers](../specs/view-layers.md),
> [Ceilings & Roofs](../specs/ceilings-and-roofs.md),
> [Wall Corner Laps](../specs/wall-corner-laps.md), and
> [Render View Mode](../specs/render-view.md).

## Goal

Keep viewport work sustainable as the authored model and generated framing gain
new element families. The immediate slice replaces the 3,382-line
`viewport/scene_build.rs` catch-all with a scene-building package whose stable
facade orchestrates focused wall, member, surface, picking, and style modules.

The later slices apply the same rule to the remaining viewport hotspots without
inventing a viewport framework: keep view inputs explicit, keep model mutation in
`FramerApp`, and keep each renderer's interaction result typed and local.

No slice changes product behavior, `.framer` data, solver output, or CPU/GPU
path-tracing math.

## Architecture / stack summary

The viewport is disposable presentation over authored `BuildingModel` intent and
derived `ProjectFramePlan` output. `framer-core` owns shared semantic geometry
derivations such as wall envelope spans and roof frames; each presentation path
lowers those facts into the representation it needs:

- `framer-app/src/app/viewport/scene_build/` builds the interactive axonometric
  mesh, outline overlay, and world-space pick geometry with projected hit testing.
- `framer-render/src/build.rs` builds finished-surface triangles and materials for
  the CPU/GPU path tracer.

Those two builders should share UI-free semantic derivations, not a lowest-common-
denominator mesh. Axonometric picking, generated framing, translucent layers, and
outline presentation are app-specific contracts.

The current explicit input bundles (`PlanView`, `AxonometricView`, and
`DesignElevationView`) plus typed return events are the preferred seam. This plan
does not add a `ViewportState` god struct, a renderer trait hierarchy, or a generic
command bus.

## Risk ledger

| Contract | Boundary | Required proof | Likely failure if missed |
| --- | --- | --- | --- |
| Scene emission order and `finish_opaque()` partition | `scene_build` → axonometric GPU callback | Existing scene tests, app tests, GPU parity | Transparent roofs or wall layers draw in the wrong pass or write depth incorrectly |
| Pick shape, priority, and depth tie-breaking | `scene_build` → `ViewClick` | Existing wall/opening/member/surface pick tests | Hidden or rear geometry wins selection |
| Cut-rafter render and pick meshes stay identical | member lowering → picking | Rafter cut/birdsmouth scene tests | Visible cut profile selects as an uncut cuboid |
| `Scene3d` and shared style-helper visibility | scene package → axonometric/view cube/elevation | `cargo test -p framer-app --locked` | Module split compiles locally but breaks sibling consumers |
| Shared wall/roof/surface facts remain aligned | `framer-core` → app and render builders | Existing app scene, render, and GPU parity tests | Interactive 3D and Render views disagree geometrically |
| No authored or derived state moves into presentation | core/solver → app viewport | Diff review; no schema or solver changes | View state becomes a second source of truth |
| Plan interaction ordering in a future split | plan renderer → wall/region tools | Existing plan tests plus UI screenshot deck | Hover, drag, snap, or placement precedence changes |
| Workspace dispatch in a future split | `FramerApp` → per-view functions | App tests plus UI screenshot deck | View switching or tool state resets drift |

## Decisions

1. **Split by reason to change.** Element-family lowering belongs in `walls`,
   `members`, and `surfaces`; reusable mesh accumulation, picking, and style policy
   are supporting leaves. New element families should gain a focused emitter
   instead of another branch in one omnibus file.
2. **Keep one small facade.** `scene_build/mod.rs` owns `Scene3d::from_project`,
   emission ordering, and the package's narrow sibling-facing surface. It is the
   place to see the full scene recipe without reading geometry details.
3. **Preserve explicit data flow.** Views take read-only bundles plus disjoint
   mutable camera/output parameters and return typed interaction results.
   `FramerApp` remains the model/history mutation boundary.
4. **Keep tests with their owner.** Focused behavior tests live inside the viewport
   package they exercise. Only genuinely cross-package camera/scene and dispatch
   tests remain at the viewport facade; private implementation details do not
   widen merely to support a distant test.
5. **Share semantics, not presentation meshes.** A later cross-crate extraction is
   justified only when both builders duplicate a UI-free construction fact. GPU
   vertices, pick solids, render triangles, and material policy stay owned by their
   presentation path.
6. **No arbitrary line-count target.** File size is a signal, not the design. A
   module is split when it has multiple change axes or its tests cannot name a
   single owned contract.

## Slices / phases

### Slice 1 — Scene-building package

- **Task 1.1 — Establish the scene facade and supporting leaves**
  - Convert `viewport/scene_build.rs` to `viewport/scene_build/mod.rs` without
    changing the external Rust module path.
  - Keep `Scene3d::from_project`, raw mesh accumulation, and the opaque/transparent
    ordering in the facade/builder layer.
  - Extract element-family lowering into `walls.rs`, `members.rs`, and
    `surfaces.rs`; extract pick and style policy into `picking.rs` and `style.rs`.
  - Files: `crates/framer-app/src/app/viewport/scene_build/*.rs`
  - Verify: `cargo test -p framer-app --locked`
  - Commit: `refactor(viewport): split scene building by element family`
- **Task 1.2 — Localize scene-building tests**
  - Move the in-file scene fixture and regression suite into
    `viewport/scene_build/tests.rs` with no assertion changes.
  - Keep only genuinely cross-module projection/scene tests in
    `viewport/mod.rs`.
  - Files: `crates/framer-app/src/app/viewport/scene_build/tests.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: `cargo test -p framer-app --locked`
  - Commit: included with Task 1.1 so the module conversion is one green move
- **Task 1.3 — Refresh concrete navigation**
  - Update `docs/code-map.md` and current durable spec path references to name the
    scene-building package and its ownership boundaries.
  - Files: `docs/code-map.md`, affected files under `docs/specs/`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: included with Task 1.1

### Slice 2 — Plan-view pipeline

- **Task 2.1 — Split plan rendering from interaction state machines**
  - Keep `plan/mod.rs` as the draw-order facade.
  - Extract wall body drawing, authored object footprints, snapping/draw-wall,
    wall dragging, and region placement into focused children with local tests.
  - Preserve the current hit and tool precedence exactly; do not introduce one
    catch-all viewport event enum merely to normalize unlike interactions.
  - Files: `crates/framer-app/src/app/viewport/plan/*.rs`
  - Verify: `cargo test -p framer-app --locked`, `scripts/ui-shots.sh`
  - Commit: `refactor(viewport): split plan rendering and interactions`

### Slice 3 — Workspace shell and test ownership

- **Task 3.1 — Separate workspace dispatch from chrome**
  - Keep the `FramerApp::workspace` match readable as the mode-level coordinator.
  - Move the view strip, tool options, navigation cube, and selection context
    toolbar into a `workspace`/`chrome` child module with narrow methods.
  - Group cohesive session state only when it has one lifecycle (for example,
    navigation cameras or one placement interaction); do not wrap all viewport
    state in a single mutable struct.
  - Files: `crates/framer-app/src/app/viewport/mod.rs`,
    `crates/framer-app/src/app/viewport/workspace/*.rs`
  - Verify: `cargo test -p framer-app --locked`, `scripts/ui-shots.sh`
  - Commit: `refactor(viewport): separate workspace dispatch and chrome`
- **Task 3.2 — Move tests to stable owners**
  - Relocate camera, view-cube, elevation, scene, and render-resolution unit tests
    from the central facade to their owning modules where privacy permits.
  - Leave integration tests that intentionally cross view boundaries in one
    facade-level test module.
  - Verify: `cargo test -p framer-app --locked`
  - Commit: `test(viewport): localize module regression coverage`

### Slice 4 — Shared semantic geometry audit

- **Task 4.1 — Inventory duplicated construction facts**
  - Compare app scene lowering with `framer-render/src/build.rs` for roof surface
    outlines, room-region resolution, wall envelope spans, and assembly-face
    selection.
  - Move only representation-independent derivations to the UI-free owning crate,
    with positive and fallback tests there. Keep app mesh/pick and render triangle
    emission separate.
  - Files: determined by the audit; likely `framer-core` plus both consumers
  - Verify: core tests, app scene tests, render tests/goldens, and GPU parity
  - Commit: one focused commit per shared derivation

## Final verification

For Slice 1:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo test -p framer-app --test gpu_parity --locked -- --nocapture
python3 scripts/check-markdown-links.py
```

Run `scripts/ui-shots.sh` for Slices 2 or 3, where drawing or workspace ordering
is mechanically disturbed. The scene package split has no intended visual delta;
its proof is unchanged scene assertions, app coverage, and GPU parity.
