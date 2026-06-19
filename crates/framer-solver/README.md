# framer-solver

Deterministic framing generation and takeoffs. UI-agnostic: a **pure function of the
model** — the same `BuildingModel` always produces the same `ProjectFramePlan`.

Depends on: `framer-core`. Consumed by: `framer-app`.

## Module

One file, `src/lib.rs`. It generates per-wall framing, adds join members, builds the room
schedule and per-layer material takeoff, and provides SVG/CSV exporters.

## Key types & entry points

- **`generate_project_plan(&BuildingModel) -> Result<ProjectFramePlan, SolverError>`** — the
  single entry point the app calls.
- **`ProjectFramePlan`** → `wall_plans: Vec<WallFramePlan>`, `rooms: Vec<RoomSchedule>`,
  `diagnostics`; `bom() -> Vec<BomItem>` (member cut list) and `layer_bom() -> Vec<LayerBomItem>`
  (per-layer material takeoff from each wall's `ConstructionSystem`).
- **`FrameMember`** — a generated member with `kind: MemberKind`, `profile: BoardProfile`, and
  `provenance: RuleProvenance` (every member traces back to the rule that made it).
- Exports: `export_bom_csv`, `export_layer_bom_csv`, `export_room_schedule_csv`,
  `export_wall_elevation_svg`, `export_project_svg`.

Generated output is **derived state** — it is never written to the canonical `.framer` file.
See [`docs/code-map.md`](../../docs/code-map.md#framer-solver--deterministic-framing--takeoffs).

## Test

```sh
cargo test -p framer-solver
```

Covers framing determinism, BOM grouping/cut lengths, per-layer material takeoff
(area/volume), and the connected-shell whole-project plan.
