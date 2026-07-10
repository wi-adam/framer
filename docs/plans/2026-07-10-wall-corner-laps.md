# Wall Corner Laps — Implementation Plan (2026-07-10)

> **Implementation plan** (point-in-time). **Spec:**
> [docs/specs/wall-corner-laps.md](../specs/wall-corner-laps.md). This file is an archival
> record of how the work was sequenced; the spec is the durable source of truth.

## Goal

Replace overlapping corner wall bodies with one deterministic through/butt lap, carry that
physical span into generated wall framing, and counter-lap the upper member of a double top
plate. Preserve centerline-authored editing and the v13 project schema.

## Architecture / stack summary

`BuildingModel::wall_envelope_span` in `framer-core/src/model.rs` is the shared derived-geometry
seam already used by app 3D and `framer-render`. The solver currently generates each wall over
`0..wall.length` and adds coincident corner posts after the per-wall pass. Plan Full/Width still
draw authored endpoint spans directly. This plan makes core own deterministic lap roles, then
threads full-assembly and framing-layer spans into those existing consumers.

## Slices / phases

### Slice 1 — Deterministic derived lap geometry

- **Task 1.1** — Derive primary through/butt roles from room-boundary orientation with a stable
  id fallback, then compute primary full-envelope and primary/counter structural spans.
  - Files: `crates/framer-core/src/model.rs`
  - Verify: core tests for closed loops, reversed authored directions, unordered joins, unequal
    thicknesses, open-corner fallback, non-corner joins, and short-span clamping
  - Commit: `feat(core): derive deterministic wall corner laps`

### Slice 2 — Physical wall framing

- **Task 2.1** — Generate bottom plates, studs, and lower top plates over the primary framing
  span; use the counter span for the upper plate when double top plates are enabled.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: solver tests assert exact plate endpoints/cut lengths and opposite seams per layer
  - Commit: `feat(solver): lap wall framing at corners`
- **Task 2.2** — Reclassify the existing end studs as corner posts instead of adding
  coincident duplicate members.
  - Files: `crates/framer-solver/src/lib.rs`
  - Verify: spatial member footprints at every demo-shell corner are pairwise disjoint and BOM
    quantities remain deterministic
  - Commit: `fix(solver): reuse end studs for corner posts`

### Slice 3 — Shared presentation geometry

- **Task 3.1** — Draw Plan Full/Width bodies over the lapped physical span while leaving the
  authored centerline as the selection/editing surface.
  - Files: `crates/framer-app/src/app/viewport/plan.rs`
  - Verify: targeted Plan geometry tests and the off-screen UI-shot deck
  - Commit: `fix(plan): draw lapped wall corner bodies`
- **Task 3.2** — Update app 3D and CPU-render assertions from overlapping outside quadrants to
  one closed, volume-disjoint butt/lap joint.
  - Files: `crates/framer-app/src/app/viewport/mod.rs`, `crates/framer-render/src/build.rs`
  - Verify: targeted app/render scene tests plus GPU parity
  - Commit: `test(render): lock physical wall corner laps`

### Slice 4 — Durable documentation and review

- **Task 4.1** — Add this spec/plan and update construction, view, render, and code-map
  contracts.
  - Files: `docs/specs/wall-corner-laps.md`, `docs/specs/README.md`,
    `docs/specs/construction-systems.md`, `docs/specs/view-layers.md`,
    `docs/specs/render-view.md`, `docs/code-map.md`
  - Verify: `python3 scripts/check-markdown-links.py`
  - Commit: `docs: specify physical wall corner laps`

## Final verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
python3 scripts/check-markdown-links.py
cargo test -p framer-app --test gpu_parity --locked -- --nocapture
scripts/ui-shots.sh
```

Inspect the selected-wall 3D shot and Plan Full/Width states for one clean seam at every
corner. When complete, set the spec status to **Implemented**.
