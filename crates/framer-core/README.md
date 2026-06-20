# framer-core

The domain model and single source of truth for Framer. UI-agnostic: **no GUI
dependency** — it is testable, scriptable, and exportable on its own. Every other crate
derives from a `BuildingModel`.

Depends on: nothing in the workspace. Consumed by: `framer-solver`, `framer-render`,
`framer-app`.

## Modules

| File | Purpose |
| --- | --- |
| `src/lib.rs` | Module wiring + public API (the `pub use` list is the public surface). |
| `src/model.rs` | All domain types: `BuildingModel`, construction systems, materials, walls, openings, joins, rooms, dimensions, code profiles, `ModelError`. |
| `src/project.rs` | `.framer` serialization: `ProjectDocument`, `load_project`/`save_project`, schema versioning + canonicalization. |
| `src/topology.rs` | Derives room boundaries/areas from the wall graph; `wall_interior_sides`. |
| `src/units.rs` | `Length` (integer **ticks**, 16 = 1 inch) and `Point2` — the basis of determinism. |
| `src/constraints.rs` | Generic linear-constraint layer for driving dimensions / overconstraint checks. |

## Key types & entry points

- **`BuildingModel`** — root authored container (`code`, `materials`, `systems`, `levels`,
  `walls`, `wall_joins`, `rooms`). The only thing persisted.
- **`Wall`** references a **`ConstructionSystem` by id** (`system: ElementId`); systems hold an
  ordered (interior→exterior) stack of **`ConstructionLayer`**s; **`Material`**s are
  open/extensible (`properties: BTreeMap<String, PropertyValue>`).
- Construct: `BuildingModel::new(code)`, `demo_wall()`, `demo_shell()`, `demo_two_bedroom()`.
- Validate: `BuildingModel::validate()`.
- Serialize: `load_project(&str)` / `save_project(&BuildingModel)`;
  `PROJECT_SCHEMA_VERSION` (currently **8**, v8-only).
- Topology: `room_boundaries(model)`, `room_boundary(model, seed)`.

See [`docs/code-map.md`](../../docs/code-map.md#framer-core--the-domain-model) for full detail
and [`docs/project-files.md`](../../docs/project-files.md) for the `.framer` format + agent
editing contract.

## Test

```sh
cargo test -p framer-core
```

Covers model validation, unit/tick arithmetic, room topology, and `.framer`
round-trip/canonicalization + unsupported-schema rejection (`src/project.rs` tests).
