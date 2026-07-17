# Framer Architecture

Framer is organized around a semantic model, a rule-driven framing solver, and a
desktop CAD surface.

> This document is the *conceptual* architecture (layering and product intent).
> For the *concrete* map — crates, modules, key types, data-flow, and "where to add
> X" — see [code-map.md](code-map.md).

## Product Boundary

Framer is not a general mesh modeler. It should preserve construction intent:
walls, floors, roofs, posts, beams, openings, loads, standards assumptions,
member families, and material assumptions. Geometry is an output of that intent,
not the primary source of truth.

The early application should make this concrete for structures such as sheds,
small buildings, garages, decks, and wood framed BBQ islands.

## Workspace

- `crates/framer-core`: shared domain types, units, structure model, openings,
  construction systems, the material/object libraries, standards-stack data,
  typed project-authored cross-object assertions/waivers, room topology, and
  validation.
- `crates/framer-library`: library resolution, exact content hashing,
  cache-first remote URL fetching, and vendor-on-use import/remap for reusable
  `.framerlib` content.
- `crates/framer-solver`: deterministic framing-plan, per-layer material takeoff,
  and BOM generation.
- `crates/framer-standards`: the UI-free shared `FactSnapshot` measurement and
  predicate-evaluation path used by standards checks and project-authored
  assertions, plus deterministic standards reporting and diagnostics lowering.
- `crates/framer-geometry`: UI-free physical-solid derivation over authored
  assemblies and generated framing, with stable semantic body identity and
  convex-piece lowering, spatial broad phase, and deterministic contact/overlap
  auditing.
- `crates/framer-analysis`: UI-free orchestration of the solver, standards,
  geometry, and library-lifecycle outputs; evaluation/lowering of persisted
  assertions and waivers through the shared standards facts; and a common
  non-persisted intent report plus deterministic, revision-bound project graph.
  It also owns typed placement patches and an explicitly requested, bounded,
  revision-cached placement-clearance candidate provider. Outcomes, diagnostics,
  reports, graph records, patches, options, and query caches are disposable; no
  second clearance or containment calculator lives here.
- `crates/framer-render`: UI-agnostic CPU path tracer (scene extraction, BVH, the
  rendering reference math mirrored by the app's GPU compute shader).
- `crates/framer-app`: native desktop UI and tiled viewport workspace.

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
   levels, roof planes, floor systems, and standards/material assumptions, plus
   explicit typed cross-object `IntentAssertion` and targeted
   `IntentOverride::Waive` records.
2. **Derived framing model**: studs, plates, headers, joists, rafters,
   sheathing zones, connector callouts, and blocking.
3. **Presentation model**: drawings, annotations, schedules, BOM tables, exports,
   viewport geometry, split layouts, and per-pane camera/render runtimes.

Only the first layer should be directly editable by users. The second and third
layers are regenerated. A persisted intent waiver is an explicit exception to an
authored assertion; it does not mutate generated framing or create a second design
model.
Named viewport-layout presets are an app-local preference over presentation state,
not project data; they never enter `.framer`.

`framer-analysis::IntentReport` and `ProjectGraph` are derived indexes across
these layers, not a fourth source of truth. Each report/graph and every
generated-member, body, compliance, diagnostic, or derived-assertion reference
it contains is bound to a deterministic `GraphRevision` over the graph contract
version, a length-delimited deterministic starter-library source input
(availability plus content hash when available), and the canonical
post-propagation project bytes. Reports, graph nodes/edges, and cached query
closures are discarded and rebuilt after any of those inputs change; they are
never persisted.

Schema v14's placed-object containment and directional-clearance assertions
exercise this boundary end to end. `framer-core` owns their exact furnishing/MEP
instance and room references, supported domains and modes, validation, and
canonical serialization. `framer-standards::FactSnapshot` alone resolves the
placed-object footprint, room binding, finished wall faces, and other-object
obstacles. `framer-analysis` maps that observation to the common outcome,
waiver, evidence, graph, and existing diagnostics protocols. `framer-app`
continues to own all interactive mutation and undo/redo. Assertion
author/edit/delete/waive, multi-participant focus, and explicitly accepted
placement options route through that existing app ownership. Candidate
generation stays outside `analyze_project()` and app rebuild/paint paths; every
option is bound to both its deterministic `GraphRevision` and the app's monotonic
document revision. Preview is presentation-only, while acceptance stages a
sorted, validated typed patch as one ordinary undoable authored edit. Structural
alternatives remain unavailable until their support/load-path, capacity, and
engineered-member prerequisites exist.

`GraphRevision` is the option provider's evaluation and cache identity;
`document_revision` is a separate process-local authorization guard. A displayed
set becomes stale on every rebuild. If an explicit regeneration request observes
the same graph bytes, analysis may reauthorize its immutable cached evaluation for
the new document generation; a graph change always discards it. Bounded search
metadata reports both fact-measurement and full-candidate analysis caps and whether
pose measurement or candidate ranking was truncated.

Regenerated room schedules and topology boundaries are typed consequence nodes
in that graph. They depend on the authored room and the walls that bound it; an
open boundary, unmatched boundary edge, or absent schedule produces explicit
unknown evidence. Explanation traversal follows dependency/evidence direction
toward support rather than wandering into downstream consequences. The project
ownership node may be returned as a useful endpoint, but it is never traversed as
a bridge between otherwise unrelated project-owned entities. Generated member
hosts and sources are kind-checked, and site context is an explicit dependency of
solver provenance and compliance evaluation so impact queries reach those
consequences without traversing through project ownership.

## Mode Contract

The desktop app should expose the modeling layers as two conceptual modes:

1. **Design Mode** edits the intent model. It is the default workspace for
   placing and sizing walls, doors, windows, floors, roofs, posts, beams,
   openings, levels, joins, constraints, and project assumptions. The solver may
   run in the background for validation or preview, but Design Mode operations
   should persist only authored intent.
2. **Plan Mode** regenerates and inspects derived output. It displays framing
   members, sheathing zones, blocking, diagnostics, drawings, BOM rows, and rule
   explanations produced from the current design plus standards/material
   assumptions.
   Generated objects can be selected and explained, but they should not become
   the canonical source of truth.

This mode split is a UI and workflow contract, not a second storage model. The
canonical `.framer` file remains authored design intent. Plan output is
deterministic generated state, equivalent to a slicer output derived from a 3D
printing model. If Framer later allows manual plan adjustments, they should be
stored as explicit intent constraints or override records that the solver can
validate and explain.

## Viewport Workspace Contract

The desktop workspace presents the same authored model and regenerated plan through
one to sixteen viewport panes. Docked panes form a resizable horizontal/vertical
split tree. Every leaf has a monotonic session `PaneId`, a selected view type, and
its own 2D/3D cameras plus CPU/GPU progressive-render runtime. Repeating Plan, 3D,
or Render therefore creates another presentation of the project rather than an
alias of one global camera or accumulator.

One pane is active for global view commands, workflow soft defaults, diagnostic
focus, tool routing, and status readouts. Authored intent, generated framing,
selection, component visibility/isolation, active level, view layers, and render
lighting remain shared. The split topology and live runtimes are disposable;
explicitly named layout presets persist only their validated presentation subset
through app-local eframe storage. See the durable
[Tiled Viewport Workspaces spec](specs/viewport-layouts.md).

A pane may be shown in an egui deferred native viewport. The child callback owns
only an immutable document snapshot and the pane's shared presentation-runtime
handle. Selection and view-scoped actions return as typed, pane-tagged events to
the root `FramerApp`, which remains the sole owner of model/history mutation. A
native close request docks the pane; deleting it remains an explicit root-window
operation. Interactive 3D and path-trace callback caches are keyed by pane identity
and released when a pane or layout is retired.

Authored driving dimensions are checked through a generic linear constraint
layer in `framer-core`. Walls currently adapt their local length and opening
edge anchors into that layer, but the rank check itself is independent of wall
geometry. Future height, roof, floor, rafter, pitch, and offset constraints
should add variables and anchor expressions for their own authored objects
rather than adding one-off overconstraint checks.

## Standards Engine

Standards packs should be versioned data plus executable rules. An authoritative
pack from a user, jurisdiction, or licensed source should include:

- Prescriptive framing defaults, spacing limits, and member families.
- Opening/header lookup rules by span, load path, and story count.
- Snow, wind, seismic, and local amendment inputs.
- Explicit assumptions and unsupported-condition diagnostics.

The current `Framer Illustrative Starter` pack stores defaults and a small
starter table set needed by the first wall solver. It must not be represented as
complete code compliance.

The shared fact vocabulary also includes placed furnishing/MEP containment and
parameterized directional clearances. Model `+X` is right and `+Y` is up;
`QuarterTurn` is counterclockwise, with `Deg0` front at local `+Y`. Clearance
requests distinguish left/right/front/back/around and centerline versus
footprint-face datum. Missing or ambiguous room, family, or assembly input stays
unknown rather than passing.

## Geometry Strategy

Framer should start with robust rectilinear 2D/2.5D primitives rather than a full
B-rep CAD kernel. Wood framing work is dominated by planes, spans, levels,
offsets, openings, and repeated members. That makes a semantic solver more
valuable than early NURBS or freeform modeling.

The interactive 3D viewport is generated from authored model surfaces plus the
same derived framing plan using `wgpu` inside the `eframe`/`egui` shell.
`framer-geometry` first derives identity-bearing physical solids for every
generated member and finished assembly envelope. The viewport lowers member
surface meshes into depth-tested vertices and uses the same indexed triangles for
picking, while retaining presentation-only material, translucency, and outline
policy. The separate path-traced Render view consumes the same UI-free core
assembly derivations through `framer-render`; each presentation owns its vertex
and material representation while native panels, inspectors, and drawing views
remain ordinary `egui` surfaces.

Each successful framing regeneration also caches the identity-bearing physical
scene and its overlap audit beside the `ProjectFramePlan`. Geometry violations
remain structured through diagnostics presentation so Plan 3-D can frame and
danger-highlight both bodies and draw the reported witness without changing
authored intent or ordinary selection. Scene, audit, and focus state are all
disposable and clear or reconcile on regeneration.

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
  wall openings, rooms, a reusable material library, layered construction systems
  (applied to walls by reference), standards packs and project site assumptions,
  furnishing/MEP object families and placed instances, and deterministic project
  ordering. The `.framer` format is schema **v14** and v14-only: older files are
  rejected with an explicit
  unsupported-schema error rather than migrated. See the
  [Construction Systems spec](specs/construction-systems.md) and
  [Libraries spec](specs/libraries.md).
- `framer-solver` deterministically generates per-wall plates, common studs,
  king studs, jack studs, headers, rough sills, cripples, join corner posts,
  grouped whole-project BOM rows, diagnostics, and per-member rule provenance.
- `framer-app` exposes a CAD-oriented shell with explicit Design, Plan, and Render
  command contexts; a model tree for levels, wall segments, openings, joins, and
  generated framing; an inspector for selectable objects; catalog placement for
  doors, windows, garage doors, furnishings, and MEP objects; diagnostics; a BOM
  table; and a tiled viewport workspace that can mix or repeat Plan, Roof,
  Elevation, WGPU-backed 3D, and path-traced Render panes. Panes have independent
  cameras/render runtimes and may be deferred into native windows while sharing
  project selection and presentation context. The Intent inspector authors,
  edits, deletes, and waives supported containment/clearance assertions through
  ordinary validated history and can focus the exact instance-plus-room
  participant set; candidate movement/rotation options are not part of Slice 3.
  Violated authored placed-object clearances can explicitly request deterministic
  movement/rotation options, preview a non-pickable Plan ghost, inspect outcome
  tradeoffs, and accept exactly one validated authored edit through undo/redo.
- `framer-analysis` generates the coherent plan, detailed standards evaluation,
  physical scene, geometry audit, starter-library lifecycle status, common
  non-persisted intent report, and canonically ordered project graph for each
  successful app rebuild. Current driving dimensions, construction selections,
  site premises, persisted project assertions/waivers, standards checks,
  diagnostics, and geometry findings lower into one typed outcome/evidence
  protocol; shared standards/project-intent fact measurements remain owned by
  `framer-standards::FactSnapshot`.
  `StandardsEvaluation::diagnostics()` lowers detailed standards rows; analysis
  appends those and lifecycle rows once before intent and graph compilation, so
  every returned surface describes the same generation. For authored selections,
  the inspector shows domain-grouped current status, filtered potential impact,
  and "Depends on" evidence. Generated-member selections state that current
  status applies to authored selections and show directional "Why generated"
  evidence. Missing evaluator input is an explicit unknown outcome rather than
  silently omitted evidence. An intent-report failure also makes the graph
  unavailable; graph endpoint failure can occur after a valid report and leaves
  current status plus valid framing, standards, geometry, and lifecycle output
  available while relationship queries are reported as unavailable. Graph
  finalization validates that every edge endpoint exists; an internal missed
  endpoint returns a typed `GraphBuildError` through `AnalysisError` rather than
  panicking the rebuild.
- Placement candidate synthesis is a separate lazy analysis entry point. It
  searches a fixed bounded room lattice, measures containment and clearance only
  through `FactSnapshot`, rejects any required-intent regression before ranking
  preference tiers and named direction-aware objective observations, and returns
  typed expected-value patches with boolean, objective, and assumption evidence
  plus lexicographic costs. It does not add walls, routes, framing, or graph nodes.
- Whole-project SVG and CSV BOM exports are sidecar artifacts regenerated from
  the authored model and generated framing plan.

The current app makes Design Mode and Plan Mode explicit UI states. Design Mode
keeps the catalog, authored model tree, editable inspector, Shell view, Wall
view, and envelope-oriented 3D view focused on authored objects. Selecting a wall
or opening from the Design Shell view opens the Wall view for layout on that wall.
Plan Mode exposes generated framing in the model tree, read-only authored
summaries, selectable generated members, diagnostics, BOM review, and export.

Unsupported conditions are shown explicitly. The starter standards pack is
illustrative and not for construction, and garage doors are currently framed as
wide rough openings with a diagnostic noting that garage-door-specific structural
design is unsupported.

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

The current v14 `.framer` format is documented in
[project-files.md](project-files.md). It stores the authored intent model only
(including site context, standards packs, the material library, construction
systems, furnishing/MEP families and placed instances, typed cross-object
assertions, and explicit project assertion waivers); derived outcomes,
`FactSnapshot` observations, framing plans, analysis graphs and query caches,
room consequences, library-lifecycle status, live viewport
layouts/cameras/render accumulators, and exports remain disposable outputs that
are regenerated from the project. Named viewport-layout presets are persisted
separately as app-local preferences and contain no project selection, 2D camera,
or authored-model state.

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
