# Physical Geometry Overlap Audit

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goals:** G-002 (Solver Correctness) / G-014 (Ceilings & Roofs) ·
> **Plan:** [2026-07-10-geometry-overlap-audit.md](../plans/2026-07-10-geometry-overlap-audit.md) ·
> **Last reviewed:** 2026-07-11

## Intent / Purpose

Framer derives physical construction from compact authored intent. A model can therefore be
valid as authored data and still regenerate two wall bodies, framing members, or roof pieces that
occupy the same physical volume. Those defects are easy to miss in a particular camera view and
can survive focused geometry tests when the missing assertion is a relationship between two
otherwise plausible bodies.

The physical geometry overlap audit makes volume-disjointness an explicit, reusable invariant. It
builds an identity-preserving, UI-free physical scene from the authored `BuildingModel` and derived
`ProjectFramePlan`, reports every unintended penetration between comparable bodies, gates checked-in
examples against regressions, and surfaces actionable diagnostics in Plan mode. It supports the
geometry-correctness and diagnostic direction in the [goal backlog](../vision.md#goal-backlog)
without turning Framer into a general solid modeler.

## Requirements & behavior

### Physical scene contract

- The audit consumes the same derived physical boundaries used by presentation, not authored
  centerlines, BOM cut lengths alone, pick proxies, or an already-flattened render mesh.
- Every auditable body has a stable `BodyRef` that identifies its collision domain, authored owner,
  semantic kind, and generated member id where applicable. A body may contain multiple convex
  pieces, but diagnostics refer to the semantic body rather than its internal pieces.
- The physical scene includes every currently modeled solid structural member in wall, floor,
  ceiling, and roof frame plans, including cut-profile common rafters.
- The physical scene also includes finished assembly envelopes for walls, floor decks, ceilings,
  and roof planes. Openings and other modeled cavities do not count as occupied volume.
- A body that should exist but cannot produce valid physical geometry is reported as a geometry
  build violation. It must not silently disappear from the audit.
- Empty projects produce an empty, successful audit. Invalid authored models still fail through the
  existing model/solver validation path before geometry auditing.

### Overlap semantics

- Two bodies overlap when their interiors penetrate by more than the implementation's numerical
  epsilon. Face, edge, and point contact are valid and do not produce an overlap.
- The numerical epsilon exists only to absorb floating-point query noise, remains substantially
  smaller than one project tick, and is covered by explicit touching and shallow-penetration tests.
  It is not a user-configurable construction tolerance.
- Each overlap reports the two canonical `BodyRef`s, maximum detected penetration depth, a witness
  location, and a stable diagnostic code. Penetration depth is an explanatory metric; exact
  intersection volume is not required.
- Pair enumeration and diagnostic ordering are deterministic and independent of model vector order,
  spatial-index traversal order, or which body is presented first to the narrow-phase query.
- Broad-phase candidate filtering must not decide whether bodies overlap. Every candidate passes
  through a solid-level narrow-phase query that distinguishes contact from penetration.
- An unsupported narrow-phase shape pair is an audit failure, not a clean result. New physical
  primitives must add query coverage before they can participate in a clean audit.

### Collision domains and policy

- **Structural framing ↔ structural framing** pairs are audited, including members owned by
  different walls, decks, ceilings, or roof planes.
- **Finished assembly envelope ↔ finished assembly envelope** pairs are audited across authored
  owners.
- Structural framing is not compared with its containing finished assembly envelope in v1. The two
  domains intentionally represent alternative physical detail levels and would otherwise report the
  framing that belongs inside each wall, floor, ceiling, or roof.
- Ground planes, selection geometry, pick solids, outlines, annotations, and other presentation-only
  objects never enter the physical scene.
- Expected construction joints are represented by correct touching/cut geometry. V1 has no
  persisted exception list, pair-id allowlist, or blanket exemption for connected bodies.
- Any future intentional volumetric connection must add a semantic collision policy and focused
  tests; it must not be hidden by raising the global epsilon.

### Diagnostics, regression gates, and product behavior

- The audit returns structured geometry diagnostics rather than encoding the second body only in
  human-readable text.
- Overlaps and unbuildable/unsupported geometry are `Violation`-equivalent results. They fail the
  geometry regression gate and appear with error styling in the app diagnostics surfaces.
- A headless audit entry point can validate a `.framer` project and print stable body ids,
  penetration depth, and witness coordinates without launching the desktop UI.
- Every checked-in `examples/projects/*.framer` project is audited in tests. A clean example must
  have no geometry violations.
- Regression fixtures cover the historical wall-corner overlap/duplicate-post failure and the
  common-rafter/ridge-board penetration that existed before the ridge-face setback.
- Activating an overlap diagnostic in Plan mode focuses and danger-highlights both bodies and shows
  the witness location. This is disposable session state and does not alter authored intent.
- The app may continue presenting other plan and compliance diagnostics when geometry violations
  exist; a geometry violation does not prevent inspection of the generated scene.

## Decisions (locked)

- **Audit final physical solids, not semantic centerlines.** Solver endpoints intentionally retain
  meanings such as a ridge centerline even when the buildable board stops at a bearing face.
- **Share UI-free solid derivation, not renderer-specific vertices.** Presentations keep their own
  vertex, material, and acceleration formats while consuming the same physical boundary decisions.
- **Represent bodies as unions of convex pieces.** Framer's boxes and extruded 2-D/2.5-D profiles
  can be queried robustly without a general B-rep or arbitrary Boolean modeling kernel.
- **Use maintained spatial-query libraries.** Broad- and narrow-phase collision algorithms are not
  hand-rolled when maintained Rust crates cover the required queries.
- **Contact is success; penetration is failure.** Buildable joints meet at faces rather than relying
  on a tolerance or connection-specific overlap exemption.
- **Separate physical detail domains.** Assembly envelopes and generated framing are each audited
  internally but are not compared with one another.
- **Fail closed on missing query coverage.** An unsupported or unbuildable body cannot make a project
  appear clean.
- **Report depth, not exact overlap volume.** Penetration depth plus body identities and a witness is
  enough to diagnose and gate this invariant; exact intersection volume would add a solid-Boolean
  subsystem without improving the pass/fail decision.
- **No persisted audit state.** Bodies, overlap results, active highlights, and any future semantic
  policy are derived from current intent and plan data; none is serialized in current schema v14.

## Architecture (grounded in the codebase)

- The UI-free `crates/framer-geometry` crate owns `PhysicalScene`, `PhysicalBody`, `BodyRef`,
  collision domains, convex-piece lowering, geometry build diagnostics, and overlap auditing. It
  depends on `framer-core` and `framer-solver`; neither core nor solver depends on it.
- `framer-core/src/model.rs` remains the source of shared assembly boundaries such as
  `BuildingModel::wall_envelope_span`, roof overhang outlines, `RoofPlaneFrame`, ceiling frames,
  level elevations, and construction-system thickness.
- `framer-solver/src/lib.rs` remains the source of semantic generated members and their integer-tick
  placement. `ProjectFramePlan` stays deterministic, `Eq`, and free of transient collision shapes.
- `framer-geometry/src/build/` lowers model assemblies and every `FrameMember` into identity-bearing
  bodies. Wall-local cuboids, arbitrary spatial board prisms, rake plates, and common-rafter cut
  profiles are geometry-owned primitives.
- `crates/framer-app/src/app/viewport/scene_build/members.rs` consumes geometry-owned member solids
  for interactive triangles and picking. The common-rafter birdsmouth and ridge-face setback no
  longer exist only inside the app.
- `crates/framer-app/src/app/viewport/scene_build/walls.rs` and
  `crates/framer-render/src/build.rs` continue consuming shared core assembly derivations; focused
  parity tests prove that audit envelopes use the same occupied boundaries.
- `framer-geometry/src/audit.rs` performs spatial broad-phase candidate generation, collision-domain
  filtering, convex-piece contact queries, canonical pair sorting, and geometry diagnostics.
- `crates/framer-app/src/app/mod.rs` caches the audit beside the regenerated project plan.
  `panels.rs` lowers geometry violations into the existing diagnostics presentation while retaining
  both `BodyRef`s for focus behavior. `viewport/scene_build/` consumes the cached physical scene,
  applies the active two-body danger highlight, and `axonometric.rs` draws the witness marker.
- `crates/framer-geometry/src/bin/geometry-audit.rs` is the headless developer entry point. Library
  APIs, not CLI output parsing, remain the machine-readable contract.

## Constraints & invariants

- Authored intent remains the only persisted source of truth. Physical scenes and audits are
  regenerated, disposable data.
- `BuildingModel` and `ProjectFramePlan` remain integer-tick, deterministic, and independent of
  floating-point collision-library types.
- Floating-point conversion is confined to `framer-geometry`; canonical ids and pass/fail behavior
  must be stable across supported platforms.
- `framer-core`, `framer-solver`, `framer-standards`, `framer-render`, and `framer-geometry` remain
  free of UI dependencies.
- Adding `framer-geometry` must not create a dependency cycle or move rendering/material policy into
  the geometry crate.
- The audit runs fast enough for plan regeneration and interactive diagnostics on checked-in example
  projects. Broad-phase indexing prevents an unconditional all-pairs narrow phase.
- The checked-in examples and historical regression fixtures define the initial supported shape and
  collision-policy surface. Adding a new solid element family requires positive, contact, overlap,
  and unsupported/fallback coverage in the same change.
- This feature introduced no project-schema change or fixture migration; its derived audit state
  remains absent from current schema v14.

## Out of scope (YAGNI)

- Automatic repair, body movement, joint cutting, or solver rule selection based on audit results.
- General CSG, NURBS, B-rep editing, arbitrary imported-mesh repair, or exact intersection-volume
  calculation.
- Clearance, minimum-gap, clash-envelope, code-required separation, or near-miss analysis; v1 checks
  occupied-volume penetration only.
- Detecting missing members, gaps, inverted normals, non-buildable but non-overlapping placement, or
  an incorrect exposed cut that intersects nothing.
- Persisted user suppressions, project-specific overlap tolerances, or id-based exception lists.
- Auditing furnishings, MEP, fasteners, membranes, annotations, or other non-solid/presentation-only
  content until those families have an explicit physical-body contract.
