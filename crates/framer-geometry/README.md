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
- common-rafter plumb cuts, matched-bearing birdsmouths, and ridge-face setbacks;
- lapped wall envelopes with wall-opening cavities and gable infill;
- floor, flat/sloped ceiling, and overhung roof assembly envelopes, including
  roof-opening cavities.

Geometry is derived only. It is never serialized into schema v13, and the crate
has no UI, renderer, material, or GPU dependency. Use
`build_physical_scene(&model, &plan)` as the whole-project entry point.
