# Active Level Drafting — Implementation Plan (2026-07-04)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/design-system.md](../specs/design-system.md). This file records the slice that
> made the status/view-control bar's level chooser drive authoring.

## Goal

Deliver the active-level part of the Design System "Wire real state" phase: the `Level`
control in the status/view-control bar becomes real drafting state, and new level-owned
objects use it instead of silently falling back to the first level.

## Architecture / stack summary

- `crates/framer-app/src/app/mod.rs` owns presentation state, document reset/load paths,
  authored-object creation, and focused app tests.
- `crates/framer-app/src/app/panels.rs` owns the status bar, model browser level selection,
  and starter catalog placement actions.
- `crates/framer-app/src/app/viewport/mod.rs` owns active tool option-strip labels.
- No schema change: active level is transient app presentation state, not authored intent.

## Slice 1 — Active level as backed app state

- **Task 1.1** — Add active-level presentation state to `FramerApp`, clamp it during rebuild,
  reset it on new/open/demo resets, and expose helpers for active id/name.
  - Files: `crates/framer-app/src/app/mod.rs`
  - Verify: focused app tests for fallback when the active level disappears.
  - Commit: `feat(app): add active drafting level`
- **Task 1.2** — Route wall, room, roof, ceiling, floor, vault, furnishing, and MEP placement
  through the active level.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/panels.rs`
  - Verify: focused app tests for authored object level assignment and catalog placement.
  - Commit: `feat(app): use active level for authored placement`
- **Task 1.3** — Make status-level and model-browser level selection activate the drafting
  level, and make tool option strips display the active level.
  - Files: `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/mod.rs`
  - Verify: unit coverage for creation paths plus smoke-level manual UI review.
  - Commit: `feat(app): wire active level controls`

## Follow-up

Region hit-testing still resolves enclosed loops through the global room-boundary graph.
This slice assigns newly authored region-backed objects to the active level, but truly
level-filtered room/ceiling/floor/vault picking should be handled as a separate topology/UI
slice.

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
```
