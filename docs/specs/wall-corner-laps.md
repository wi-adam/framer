# Wall Corner Laps

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented · **Linked goal:** G-007 (Walls & Rooms) ·
> **Plan:** [2026-07-10-wall-corner-laps.md](../plans/2026-07-10-wall-corner-laps.md) ·
> **Last reviewed:** 2026-07-10

## Intent / Purpose

Authored walls describe construction intent as centerline segments that meet at one logical
corner. The generated building must translate that abstract junction into buildable physical
geometry: one wall runs through the corner and the adjoining wall butts into it. The result is
a closed corner with neither a cavity nor two wall bodies occupying the same volume.

The structural framing follows the same intent. The primary stud-and-plate layer uses the
through/butt joint, while a second top plate counter-laps the joint to tie the two walls
together. This keeps authored editing simple while making the regenerated framing, takeoff,
Plan/3D views, and path-traced render describe one physical corner.

## Requirements & behavior

- A `Corner` join derives one **primary through wall** and one **primary butting wall**. The
  role is regenerated from topology; it is not authored or serialized.
- On the primary lap, the through wall extends from the shared centerline point to the other
  wall's outside face. The butting wall retracts to the through wall's inside face. Unequal
  wall thicknesses remain gap-free and volume-disjoint.
- Around an enclosed exterior loop, primary roles follow the counterclockwise room boundary:
  the incoming wall runs through and the outgoing wall butts. Each perimeter wall therefore
  runs through at one end and butts at the other, independent of its authored start/end
  direction.
- When room topology cannot orient a corner (for example, two otherwise free wall segments),
  the lower wall id is the primary through wall. This fallback is deterministic and is
  unchanged by model vector order or `WallJoin.first_wall`/`second_wall` order.
- `EndToEnd`, `Tee`, and `Cross` joins retain their existing centerline-bounded envelope
  behavior. Corner lapping must not change their geometry.
- Finished wall envelopes use full construction-system thickness. Generated studs and plates
  use the framing layer's depth, so structural pieces meet at structural faces rather than at
  finish/cladding faces.
- Bottom plates, studs, and the lower top plate use the primary framing lap. When double top
  plates are enabled, the upper top plate uses the counter-lap (the opposite wall runs
  through) so the plate seams are staggered across the corner.
- Per-layer wall takeoff uses the finished envelope's derived physical length, so unequal wall
  thicknesses add material to the through wall and remove it from the butting wall.
- Existing physical end studs become the generated corner posts. Corner generation must not
  place a second member in the same volume as an already-generated end stud.
- Authored wall length, endpoints, opening offsets, snapping, dimensions, and `.framer`
  serialization stay centerline-based and unchanged. Derived physical spans may extend before
  local `x = 0` or after `x = wall.length`.
- Plan Full/Width display, app 3D wall bodies and pick envelopes, and `framer-render` all use
  the same derived lapped envelope span.
- A derived retraction is clamped so malformed/extremely short geometry never produces an
  inverted span. Normal validated building walls retain a positive physical span.

## Decisions (locked)

- **Butt/lap, not overlapping boxes or a 45-degree miter.** Platform-framed walls are
  represented as buildable rectangular assemblies; a miter would imply cutting every layer
  and framing member diagonally.
- **Topology chooses the primary lap.** A room has a construction sequence even when wall ids
  or authored directions differ. The id fallback exists only where topology is ambiguous.
- **Counter-lap the upper plate.** Reversing the seam on the second top plate ties intersecting
  wall segments without adding persisted corner-detail controls.
- **Reuse the two physical end studs.** The starter detail is an open/two-stud corner rather
  than duplicate coincident posts. Specialized three-stud/California-corner backing remains a
  future construction-detail choice.
- **No schema field for lap direction in this slice.** The derived rule gives deterministic,
  editable intent now; an authored override can be added later only if construction documents
  need control over wall erection order.

## Architecture (grounded in the codebase)

- `framer-core/src/model.rs`: `BuildingModel::wall_envelope_span` derives the primary
  full-assembly lap. Structural span helpers use the resolved framing-layer depth for the
  primary and counter laps. `wall_interior_sides` supplies room-boundary orientation.
- `framer-solver/src/lib.rs`: project wall generation receives the derived structural spans;
  plates and stud layout use those spans, and join generation reclassifies the relevant end
  studs as `MemberKind::CornerPost`.
- `framer-app/src/app/viewport/plan.rs`: Width and Full wall bodies use the derived envelope
  endpoints while authored centerlines remain the interaction surface.
- `framer-app/src/app/viewport/scene_build.rs` and `framer-render/src/build.rs`: both already
  consume `wall_envelope_span`, so the core contract keeps interactive and path-traced
  geometry aligned.

## Constraints & invariants

- Authored intent remains the only persisted source of truth; all corner spans and roles are
  disposable derived data.
- All role selection and span math are deterministic, integer-tick, and independent of vector
  order.
- `framer-core`, `framer-solver`, and `framer-render` remain UI-free.
- No `.framer` schema change or fixture migration is required.
- CPU/GPU path-tracer math is unchanged; both consume the same CPU-built lapped scene geometry.

## Out of scope (YAGNI)

- User-authored through/butt overrides or wall erection sequencing.
- Layer-by-layer corner wrapping for membranes, sheathing, cladding, siding, or drywall.
- Traditional three-stud/California-corner nailers, drywall clips, hold-downs, and engineered
  shear-transfer details.
- Non-corner (`Tee`, `Cross`, `EndToEnd`) physical connection detailing.
- Full prescriptive-code validation of corner fasteners and top-plate ties.
