# Level-Filtered Regions — Implementation Plan (2026-07-05)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/design-system.md](../specs/design-system.md). This file records the slice that
> made active-level drafting use same-level wall topology for region picking and
> room-backed surface resolution.

## Goal

Complete the follow-up from
[2026-07-04 active-level drafting](2026-07-04-active-level-drafting.md): room,
ceiling, floor, and vault placement should only see enclosed loops on the active
drafting level. Room-backed surfaces and room schedules should resolve through the
room's own level so stacked floors can share plan coordinates without bleeding
topology across levels.

## Architecture / stack summary

- `crates/framer-core/src/topology.rs` owns wall-graph room boundaries. This slice
  adds level-scoped helpers beside the existing global helpers.
- `crates/framer-solver/src/lib.rs` resolves `SurfaceRegion::Room` and room schedules
  through same-level topology.
- `crates/framer-app/src/app/mod.rs` owns active-level placement and draw-wall
  enclosure detection.
- `crates/framer-app/src/app/viewport/plan.rs`,
  `crates/framer-app/src/app/viewport/scene_build.rs`, and
  `crates/framer-app/src/app/panels.rs` consume same-level room outlines for
  display, picking, meshing, and inspector conversion.
- No schema change: levels and regions are already authored intent; this changes
  derived/presentation lookup behavior only.

## Slice 1 — Level-scoped room topology

- **Task 1.1** — Add `room_boundary_on_level`, `room_boundaries_on_level`, and
  `enclosed_room_count_on_level` while preserving the global topology helpers.
  - Files: `crates/framer-core/src/topology.rs`, `crates/framer-core/src/lib.rs`
  - Verify: core tests where a lower-level loop must not resolve on an upper level.
  - Commit: `feat(core): add level-scoped room topology`
- **Task 1.2** — Resolve room schedules and `SurfaceRegion::Room` outlines through
  the room's own level in core, solver, app meshing, inspector conversion, and plan
  room fills.
  - Files: `crates/framer-core/src/model.rs`, `crates/framer-solver/src/lib.rs`,
    `crates/framer-app/src/app/panels.rs`,
    `crates/framer-app/src/app/viewport/plan.rs`,
    `crates/framer-app/src/app/viewport/scene_build.rs`
  - Verify: solver regressions for an upper-level room region sitting over only a
    lower-level enclosure.
  - Commit: `fix(solver): resolve room regions by level`
- **Task 1.3** — Route active region tools and draw-wall enclosure completion through
  active-level topology; keep auto-joins same-level for stacked walls.
  - Files: `crates/framer-app/src/app/mod.rs`, `crates/framer-app/src/app/draw_wall.rs`
  - Verify: app tests for inactive-level region clicks, stacked same-footprint room
    attachment, active-level draw-wall closure, and no cross-level wall joins.
  - Commit: `fix(app): filter region drafting by active level`

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
```
