# Roof Overhangs, Visible Framing & Gable Walls — Implementation Plan (2026-07-09)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/ceilings-and-roofs.md](../specs/ceilings-and-roofs.md). This file is an archival
> record of how the work was sequenced; the spec is the durable source of truth.

## Goal

Make the existing eave/rake inputs affect roof geometry in both visual pipelines, expose the
already-derived roof framing in Plan-mode 3-D, and close/frame simple gable ends. Preserve the
stored `RoofPlane` plus derived-plan architecture and keep schema v13 unchanged.

## Architecture / stack summary

- `framer-core/src/model.rs` owns `RoofPlane`, `RoofPlaneFrame`, wall/system intent, and the shared
  derived geometry consumed across crates.
- `framer-solver/src/lib.rs` already emits `RoofFramePlan`; its `SlopedPlacement` lacks plan
  endpoints and `WallFramePlan` currently stops at the authored rectangular wall height.
- `framer-render/src/build.rs` and `framer-app/src/app/viewport/scene_build.rs` independently mesh
  authored surfaces, so both must consume the same core overhang/gable derivations.
- The app 3-D path currently traverses generated wall members only and makes roof skins opaque in
  Plan mode; the Generated tree and member selection are likewise wall-specific.

## Slices / phases

### Slice 1 — Shared derived roof and gable geometry

- **Task 1.1** — Add a model-aware derived roof outline: offset the eave, offset exposed rakes,
  preserve shared ridge/hip/valley edges, and project through the original bearing frame.
  - Files: `crates/framer-core/src/model.rs`, `crates/framer-core/src/lib.rs`
  - Verify: core tests for zero/gable/reverse-winding/hip/valley outlines and seam closure
  - Commit: `fix(core): derive roof overhang outlines without breaking shared seams`
- **Task 1.2** — Derive a narrow `GableWallProfile` only from two matched rake edges spanning an
  exterior wall; reject hip, shed, interior, incomplete, and mismatched cases.
  - Files: `crates/framer-core/src/model.rs`, `crates/framer-core/src/lib.rs`
  - Verify: positive and negative core fixtures for every detection branch
  - Commit: `feat(core): derive simple gable wall profiles from roof intent`

### Slice 2 — Spatial roof and gable framing

- **Task 2.1** — Add exact plan endpoints to `SlopedPlacement` and populate them for sloped
  ceiling members and every roof member, including hip and valley edges.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: exact common/ridge/hip/valley endpoint assertions plus existing cut-length tests
  - Commit: `fix(solver): make sloped framing placement spatially complete`
- **Task 2.2** — Append gable studs/rake plates and triangular layer takeoff to the hosting wall
  plan using its resolved wall system; update roof layer area to the shared overhung outline.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: gable member kinds/counts/cuts/BOM, hip negative case, overhang-area test
  - Commit: `feat(solver): generate simple gable end framing and overhang takeoff`

### Slice 3 — Both meshers and Plan presentation

- **Task 3.1** — Route the shared overhang outline and gable profile through CPU Render and app
  3-D, including bounds, picking, wall display modes, and the roof-plan overlay.
  - Files: `crates/framer-render/src/build.rs`,
    `crates/framer-app/src/app/viewport/scene_build.rs`,
    `crates/framer-app/src/app/viewport/plan.rs`
  - Verify: render/app geometry bounds, lowered eave tail, gable closure, overhang-only pick
  - Commit: `fix(render): show roof overhangs and derived gable walls`
- **Task 3.2** — Mesh roof-plan members in Plan-mode 3-D from their exact endpoints, make roof
  skins translucent there, and generalize the Generated tree/member selection from wall ids to
  owning plan host ids. Keep `FrameMember::source` as rule provenance (for example, an opening
  that caused a header), not UI ownership. Add a roofed Plan-mode screenshot state.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/scene_build.rs`,
    `crates/framer-app/src/app/ui_shots_tests.rs`
  - Verify: app tests for Design-vs-Plan members, roof-member picking/tree count, screenshot deck
  - Commit: `feat(app): inspect generated roof framing in plan 3d`

### Slice 4 — Durable documentation and visual proof

- **Task 4.1** — Update the spec/code map, add overhang/gable coverage to shared render fixtures,
  inspect before/after 3-D and Render screenshots, and run parity/full gates.
  - Files: `docs/specs/ceilings-and-roofs.md`, `docs/code-map.md`, render/app fixtures
  - Verify: markdown links, render tests/golden as needed, GPU parity, screenshot deck, full gate
  - Commit: `docs(roofs): record overhang framing and gable wall completion`

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
cargo test -p framer-app --test gpu_parity --locked -- --nocapture
scripts/ui-shots.sh
```

Inspect the roofed Design 3-D, Plan 3-D, and Render screenshots for eave/rake projection, closed
gable faces, coincident multi-plane seams, and legible generated roof members.
