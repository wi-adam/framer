# Framer Architecture

Framer is organized around a semantic model, a rule-driven framing solver, and a
desktop CAD surface.

## Product Boundary

Framer is not a general mesh modeler. It should preserve construction intent:
walls, floors, roofs, posts, beams, openings, loads, code profile, member
families, and material assumptions. Geometry is an output of that intent, not the
primary source of truth.

The early application should make this concrete for structures such as sheds,
small buildings, garages, decks, and wood framed BBQ islands.

## Workspace

- `crates/framer-core`: shared domain types, units, structure model, openings,
  code profiles, and validation.
- `crates/framer-solver`: deterministic framing-plan and BOM generation.
- `crates/framer-app`: native desktop UI and viewport.

The app may become visually rich over time, but core modeling and solving must
stay independent of the GUI so it can be tested, exported, scripted, and reused
by future CLIs or plugins.

The desktop UI should evolve toward a professional parametric CAD workspace:
full 3D viewport, model tree, properties inspector, object catalog, command
palette, generated plan/elevation/section views, and section-plane inspection.
The initial `egui` shell is a scaffold for product learning, not the intended
end-state interaction model.

## Modeling Layers

1. **Intent model**: user-authored objects such as wall segments, openings,
   levels, roof planes, floor systems, and code/material profiles.
2. **Derived framing model**: studs, plates, headers, joists, rafters,
   sheathing zones, connector callouts, and blocking.
3. **Presentation model**: drawings, annotations, schedules, BOM tables, exports,
   and viewport geometry.

Only the first layer should be directly editable by users. The second and third
layers are regenerated unless the product later adds explicit override records.

## Code Profiles

Code profiles should be versioned data plus executable rules. A profile such as
`IRC 2021` should eventually include:

- Prescriptive framing defaults, spacing limits, and member families.
- Opening/header lookup rules by span, load path, and story count.
- Snow, wind, seismic, and local amendment inputs.
- Explicit assumptions and unsupported-condition diagnostics.

The current `IRC 2021 prescriptive starter profile` only stores defaults needed
for the first straight-wall solver. It must not be represented as complete code
compliance.

## Geometry Strategy

Framer should start with robust rectilinear 2D/2.5D primitives rather than a full
B-rep CAD kernel. Wood framing work is dominated by planes, spans, levels,
offsets, openings, and repeated members. That makes a semantic solver more
valuable than early NURBS or freeform modeling.

A future 3D viewport can be generated from the same derived framing model using
`wgpu`, while the first application shell uses `eframe`/`egui` for native panels,
editing, and immediate-mode drawing.

Framer should not make arbitrary solid operations the primary modeling surface.
The viewport should let users place, select, drag, snap, and parametrically edit
construction objects such as walls, openings, floors, roofs, posts, beams,
joists, rafters, skylights, and stairs. Cross sections and drawing views should
be projections of the same semantic model.

## Current Alpha Slice

The completed Phase 1 slice is still supported: a single straight wall can be
opened, edited, framed, exported, saved, reopened, and regenerated
deterministically. The current checked-in alpha now extends that loop to a first
multi-wall CAD shell:

- `.framer` JSON stores schema-versioned authored intent only.
- The default demo project models a connected rectilinear wall shell with one
  level, four placed wall segments, four corner joins, doors, windows, and a
  garage-door-style opening on different walls.
- `framer-core` represents levels, wall segment placement, wall joins/corners,
  wall openings, and deterministic project ordering. Schema v1 single-wall files
  migrate into the current placed-wall shape on load.
- `framer-solver` deterministically generates per-wall plates, common studs,
  king studs, jack studs, headers, rough sills, cripples, join corner posts,
  grouped whole-project BOM rows, diagnostics, and per-member rule provenance.
- `framer-app` exposes a CAD-oriented shell with a model tree for levels, wall
  segments, openings, joins, and generated framing; an inspector for selectable
  objects; catalog placement for doors, windows, and garage doors; diagnostics;
  a BOM table; a whole-shell plan viewport; selected-wall elevation view; and a
  first selectable 2.5D shell workspace.
- Whole-project SVG and CSV BOM exports are sidecar artifacts regenerated from
  the authored model and generated framing plan.

Unsupported conditions are shown explicitly. The starter profile does not claim
complete IRC compliance, and garage doors are currently framed as wide rough
openings with a diagnostic noting that garage-door-specific structural design is
unsupported.

## Data and Export

The model should remain serializable and agent-accessible. Initial project files
can be JSON while the schema is young. Before a public alpha, add a stable
`.framer` container format with schema versioning, migration tests, and room for
drawings and generated artifacts.

The canonical design state should be text-first or text-indexed with stable IDs,
deterministic ordering, and clear separation between authored intent, generated
framing, cached view data, and exports. Coding agents should be able to inspect a
project, explain it, propose edits, and validate the result without needing to
reverse-engineer an opaque binary format.

The current v2 `.framer` format is documented in
[project-files.md](project-files.md). It stores the authored intent model only;
derived framing plans, cached view state, and exports remain disposable outputs
that are regenerated from the project.

Binary caches are acceptable only as disposable acceleration data. They must not
be the only source of truth for a design.

Current exports:

- Whole-project SVG with a shell plan and wall elevation strips.
- Grouped whole-project BOM and cut list as CSV.

Expected future exports:

- Richer plan/elevation/section drawing sheets as SVG/PDF.
- Framing plans as PDF.
- 3D/interop geometry as GLTF or a similar open format.
- Machine-readable project summaries for downstream automation.
