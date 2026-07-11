# framer-geometry

UI-free physical-solid derivation for Framer.

The crate consumes authored `framer-core::BuildingModel` intent plus a
deterministic `framer-solver::ProjectFramePlan` and produces a disposable
`PhysicalScene`. Each body retains a canonical `BodyRef`, an indexed surface
(exact exterior geometry for generated members), and a union of convex pieces
for maintained collision-query libraries.

Current coverage includes:

- every generated wall, floor, ceiling, and roof member;
- spatial hip, valley, jack, ridge, blocking, and rake members;
- common-rafter plumb cuts, matched-bearing birdsmouths, ridge-face setbacks,
  and face-butted hip/valley junctions;
- lapped wall envelopes with wall-opening cavities and gable infill;
- floor, flat/sloped ceiling, and overhung roof assembly envelopes, including
  roof-opening cavities.

Geometry is derived only. It is never serialized into schema v13, and the crate
has no UI, renderer, material, or GPU dependency. Use
`build_physical_scene(&model, &plan)` as the whole-project entry point.

`audit_project(&model, &plan)` checks assembly bodies against assemblies and
generated framing against framing. Cross-detail host/member pairs are not
compared. Face, edge, and point contact are valid; positive-volume penetration,
unbuildable bodies, and unsupported convex queries produce deterministic
structured violations.

For a headless repository or project check:

```sh
cargo run -p framer-geometry --bin geometry-audit -- examples/projects/demo-shell.framer
```

The command exits `0` for a clean project, `1` for geometry violations, and `2`
for usage, load, or solve failures. Human-readable output uses stable semantic
body paths; library callers should consume `GeometryAudit` rather than parse the
text output.
