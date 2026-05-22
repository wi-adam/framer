# Framer Vision

Framer is an open source parametric CAD tool for wood framed structures. It
should help a builder, designer, or maker describe what they want to build, then
turn that intent into framing plans, diagnostics, and a useful bill of materials.

## North Star

The core workflow is:

1. Define the structure shape.
2. Configure code, material, and project assumptions.
3. Place openings in walls, floors, ceilings, and roofs.
4. Generate framing plans and a BOM.
5. Inspect assumptions, unsupported conditions, and code-rule diagnostics.
6. Export drawings, schedules, and machine-readable project data.

The product succeeds when someone can model a small building or framed object,
understand how Framer framed it, and take the output into a real planning or
construction workflow.

## Target Users

- Owner-builders planning sheds, studios, garages, ADUs, decks, or outdoor
  kitchens.
- Contractors and framers who want quick framing takeoffs and repeatable
  alternatives.
- Designers who need wood-framing-aware early plans before permit drawings.
- Open source contributors who want a practical CAD system with a focused domain.

## Product Principles

- **Model construction intent, not meshes.** Walls, openings, levels, roofs,
  joists, rafters, posts, beams, and code profiles are first-class objects.
- **Generated output must be explainable.** Every derived member should trace
  back to the input geometry, code profile, and rule assumptions that created it.
- **Be deterministic.** The same project file and code profile should regenerate
  the same framing plan and BOM.
- **Make project files agent-accessible.** A design file should be easy for
  Codex, Claude, and other tools to inspect, diff, explain, and modify without
  reverse-engineering an opaque binary blob.
- **Code compliance is explicit, never implied.** Starter profiles can exist, but
  Framer must clearly label incomplete rule coverage and unsupported conditions.
- **Keep the core independent of the UI.** The solver must stay testable,
  scriptable, and exportable without the desktop app.
- **Prefer open formats.** Project files, drawings, BOMs, and geometry exports
  should be inspectable and automation-friendly.
- **Be a focused CAD tool.** Framer should become excellent at wood framing
  before attempting general-purpose CAD.

## CAD Experience

The long-term UI should feel closer to Fusion, Inventor, or SolidWorks than a
form-driven estimating tool. The current `egui` wall demo is only a temporary
working slice.

Framer should become a real 3D parametric CAD workspace with:

- A full 3D model viewport with orbit, pan, zoom, selection, snapping, and object
  manipulation.
- Plan, elevation, section, and cross-section views generated from the same model.
- A model tree for levels, walls, floors, roofs, openings, generated framing, and
  diagnostics.
- A properties inspector for selected objects, constraints, dimensions, code
  assumptions, and rule diagnostics.
- A command/tool palette for common modeling actions.
- A catalog of framing-aware objects such as walls, windows, doors, garage doors,
  skylights, stair openings, posts, beams, joists, rafters, and blocking.
- Drag-and-drop placement for catalog objects, followed by parametric editing of
  size, location, host, offsets, sill height, header assumptions, and constraints.
- Section planes and visibility controls that let users inspect framing inside
  assemblies without destroying the model.

The UI should not expose arbitrary solid modeling operations as the main product
primitive. Users should place and edit construction objects; Framer should derive
the framing geometry, drawings, and BOM from those objects.

## Agent-Accessible Project Files

Framer project files are part of the product surface. They should be designed so
humans and coding agents can help translate a user's intent into a working design.

The durable project format should:

- Be text-first or text-indexed, with stable IDs and deterministic ordering.
- Preserve semantic construction objects rather than only generated geometry.
- Separate authored intent from generated framing, cached view data, and exports.
- Include schema versions, migrations, and compatibility checks.
- Be friendly to Git diffs, code review, and scripted edits.
- Include enough names, notes, assumptions, and provenance for an agent to explain
  or modify the design safely.
- Allow future bundled assets, but keep the authoritative model inspectable
  without proprietary tooling.

Binary caches are acceptable for performance, but they must be disposable. The
canonical design state should remain inspectable and recoverable from open,
documented data.

## Scope

### In Scope

- Wood framed structures and framed objects.
- Walls, wall openings, floors, floor openings, ceilings, roofs, skylights,
  stairs, decks, posts, beams, blocking, and sheathing zones.
- Parametric editing of dimensions, spacing, code assumptions, and member
  families.
- 3D modeling, sectioning, object placement, and parametric object editing for
  wood-framing-aware design objects.
- Framing plans, elevation/plan drawings, schedules, BOMs, cut lists, and exports.
- Code profiles such as IRC 2021, including explicit local amendment hooks over
  time.

### Out Of Scope For Early Releases

- Stamped structural engineering.
- Permit-ready construction documents without professional review.
- Full mechanical, electrical, and plumbing design.
- Arbitrary freeform surface modeling.
- Opaque project files that only Framer can understand.
- Hidden or unverifiable code-compliance claims.

## System Shape

Framer should preserve three separate layers:

- **Intent model:** user-authored building objects and assumptions.
- **Derived framing model:** generated members, callouts, diagnostics, and BOM
  entries.
- **Presentation model:** viewport geometry, drawings, annotations, exports, and
  schedules.

Users edit intent. Framer regenerates derived framing and presentation artifacts.
Manual overrides may come later, but they should be explicit override records,
not silent mutations of generated output.

## Milestones

### M0: Repo Foundation

Status: started.

- Rust workspace with core, solver, and desktop app crates.
- Initial straight-wall demo.
- Architecture and Phase 1 planning docs.

### M1: Single-Wall Framing Loop

Goal: make one wall useful enough to trust as the product seed.

Status: first end-to-end alpha slice is implemented for a single straight wall.
It supports schema-versioned project files, authored wall/opening edits,
deterministic framing generation, diagnostics, per-member starter-rule
provenance, grouped BOM, SVG elevation export, and CSV BOM export. It remains a
starter framing model, not complete IRC compliance.

- Edit wall dimensions and openings.
- Generate plates, common studs, king studs, jack studs, headers, sills, and
  cripples.
- Show validation errors inline.
- Show grouped BOM and member list.
- Save and load project files.
- Establish the text-first, agent-accessible project format direction.
- Export a simple drawing and CSV BOM.

### M2: Multi-Wall Shells

Goal: model a simple rectilinear framed shell.

Status: first multi-wall CAD alpha is implemented. Framer now has a
project-level model with levels, placed wall segments, wall joins/corners, and
multiple openings across different walls. The app defaults to a connected
multi-wall shell example and can open, inspect, edit, regenerate, export, save,
and reopen it.

- Add levels and connected wall segments. **Implemented for rectilinear wall
  shells.**
- Handle corners, intersections, and wall joins. **Implemented for authored
  corner/end-to-end joins with generated corner-post members; tee/cross framing
  remains unsupported and diagnosed.**
- Place doors, windows, and garage doors across multiple walls. **Implemented
  through the model tree/catalog plus per-wall inspector selection.**
- Introduce the 3D workspace direction with selectable construction objects.
  **Implemented as a whole-shell plan view, selected-wall elevation view, and
  selectable WGPU 3D viewport with wall-envelope and generated-framing cuboids.**
- Generate plan and elevation views. **Implemented as app views and a
  whole-project SVG export with a shell plan and wall elevations.**
- Produce a whole-shell BOM. **Implemented by aggregating generated members
  across all wall segments and join members.**

Remaining M2 work: richer join rules, intersections beyond simple corners,
snapping/drag handles, dimension annotations, and stronger plan/elevation
drawing output.

### M3: Floors And Roofs

Goal: cover the main framing systems of small wood structures.

- Add floor framing with joists, rim boards, blocking, and stair openings.
- Add roof planes, rafters/trusses as data, overhangs, skylights, and roof
  openings.
- Generate section/elevation callouts for floor and roof assemblies.

### M4: Code Profiles And Diagnostics

Goal: move from defaults to auditable prescriptive rules.

- Version code profiles.
- Store rule inputs, assumptions, unsupported cases, and local amendments.
- Add header/span lookup rules with explicit limitations.
- Explain why each member was generated.

### M5: Public Alpha

Goal: make Framer useful to early open source users.

- Stable `.framer` project format with migration tests.
- Agent-readable project internals suitable for Git review and assisted editing.
- Native desktop packaging for macOS, Windows, and Linux.
- SVG/PDF drawings, CSV BOM, and an open geometry export.
- Example projects: shed, garage wall, deck frame, and BBQ island.
- Contributor docs and issue templates.

## Goal Backlog

Use these IDs when creating `/goal` work. Keep work scoped to one goal unless the
dependency is unavoidable.

- **G-001 Project Files:** add a schema-versioned, text-first project format,
  save/load UI, round-trip tests, and deterministic serialization.
- **G-002 Wall Solver Correctness:** improve wall framing rules for corners,
  stock-length plate breaks, opening edge cases, member IDs, and diagnostics.
- **G-003 Viewport Interaction:** add pan, zoom, selection, hover labels, and
  dimension handles to the elevation viewport.
- **G-004 Drawing Export:** export the current wall elevation as SVG, then PDF.
- **G-005 BOM Export:** export grouped BOM and cut list as CSV.
- **G-006 Rule Explanations:** attach rule provenance and assumption text to
  generated members.
- **G-007 Multi-Wall Model:** add connected wall segments, levels, corners, and a
  whole-project framing plan.
- **G-008 Code Profile Data:** expand the IRC 2021 starter profile into explicit
  rule tables and unsupported-condition warnings.
- **G-009 Example Projects:** add checked-in sample projects for a wall, shed,
  framed BBQ island, and small garage.
- **G-010 Packaging:** add release packaging for the native desktop app.
- **G-011 CAD Workspace UX:** define and prototype the Fusion/Inventor/SolidWorks
  style workspace: 3D viewport, model tree, inspector, command palette, object
  catalog, and generated drawing views.
- **G-012 Agent Editing Contract:** document how agents should inspect and edit
  `.framer` projects, including stable IDs, authored-vs-generated data,
  provenance, and validation commands.

## Definition Of Done

A Framer goal is done when:

- The behavior is implemented in the appropriate layer.
- Core and solver changes have focused tests.
- Generated output is deterministic.
- Project-file changes are reviewable by humans and agents when the goal touches
  persisted design data.
- User-facing assumptions and limitations are visible.
- Documentation is updated when the product surface or architecture changes.
- `cargo fmt --all -- --check` and `cargo test --workspace` pass.

## `/goal` Operating Rules

When using `/goal` with this vision:

- Treat this document as the product source of truth.
- Preserve the semantic intent model as the editable source of truth.
- Keep persisted design files text-first or text-indexed, deterministic, and
  agent-readable.
- Keep `framer-core` and `framer-solver` free of desktop UI dependencies.
- Treat the long-term UI as a professional 3D parametric CAD workspace, even when
  early slices use simpler controls.
- Do not represent starter defaults as complete IRC compliance.
- Prefer thin vertical slices that update model, solver, UI, docs, and tests
  together when that creates a usable product loop.
- If a proposed implementation conflicts with this document, update the vision
  intentionally before implementing the conflicting behavior.
