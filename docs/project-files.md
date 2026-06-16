# Framer Project Files

Framer project files are UTF-8 JSON documents with the `.framer` extension. The
format is intentionally text-first so humans, Git, and coding agents can inspect
and edit authored design intent without reverse-engineering an opaque binary
container.

The v4 format stores only the canonical intent model. Generated framing plans,
cached viewport data, drawings, BOM exports, and other disposable artifacts are
regenerated from the authored model and must not be written into the canonical
v4 file.

This matches Framer's Design Mode / Plan Mode split. Design Mode writes the
authored model saved here. Plan Mode regenerates framing layouts, diagnostics,
drawings, schedules, and exports from that authored model.

See
[../examples/projects/demo-shell.framer](../examples/projects/demo-shell.framer)
for a complete checked-in multi-wall alpha example with a connected shell,
corner joins, doors, windows, and a garage-door-style opening. The Phase 1
single-wall example remains checked in at
[../examples/projects/demo-wall.framer](../examples/projects/demo-wall.framer).

## V4 Shape

```json
{
  "format": "framer.project",
  "schema_version": 4,
  "authored": {
    "code": {},
    "levels": [],
    "walls": [],
    "wall_joins": []
  }
}
```

- `format` must be `framer.project`.
- `schema_version` must be `4` when saving from the current app.
- `authored` contains the user-authored semantic model.
- Unknown top-level keys are rejected. Do not add `generated`, `cache`,
  `exports`, or presentation data to v4 project files.
- Schema v1 single-wall files are accepted on load and migrated to the current
  placed-wall shape with a default `level-1` level and a straight wall segment.
- Schema v2 and v3 shell files are accepted on load; missing wall dimensions
  default to an empty list and missing dimension axes default to horizontal.

Lengths are exact integer ticks:

- `1 tick = 1/16 inch`.
- `16 ticks = 1 inch`.
- `192 ticks = 1 foot`.

Example:

```json
{
  "length": {
    "ticks": 3072
  }
}
```

`3072` ticks is a 16 foot wall.

## Authored Model

The current v4 authored model supports the completed Phase 1 single-wall
workflow and the first beyond-Phase-1 multi-wall shell workflow:

- `code`: starter framing defaults used by the current solver.
- `levels`: deterministic list of project levels.
- `walls`: deterministic list of placed rectilinear wall segments.
- `wall_joins`: deterministic list of authored wall joins/corners.
- `openings`: deterministic list of wall openings hosted by each wall segment.
- `dimensions`: deterministic wall-local dimension constraints hosted by a wall
  segment.

Each wall stores both a local framing length and a project placement:

- `level`: the level that owns the wall segment.
- `start` and `end`: rectilinear project coordinates in ticks.
- `length`: the wall's local framing length; it must match the axis-aligned
  distance between `start` and `end`.
- `dimensions`: optional driving or reference dimensions between wall-local
  anchors on either the horizontal or vertical wall-elevation axis.

Each dimension stores:

- `kind`: `Driving` or `Reference`.
- `axis`: `Horizontal` or `Vertical`. Missing `axis` defaults to `Horizontal`
  for projects created before schema v4.
- `start` and `end`: wall-local anchors. Legacy horizontal anchors such as
  `wall_start`, `wall_end`, `opening_left`, `opening_center`, and
  `opening_right` remain valid. New point anchors use `wall_point` or
  `opening_point` with horizontal `Left`/`Center`/`Right` and vertical
  `Bottom`/`Center`/`Top` references, allowing dimensions to snap to edges,
  vertices, and centers.
- `direction`: `Forward` or `Backward`, preserving the click order used to place
  the dimension.
- `line_offset`: optional wall-local annotation placement on the axis
  perpendicular to the dimension. Horizontal dimensions use this as the line's
  local Y position; vertical dimensions use it as the line's local X position.
  Missing offsets use the legacy outside-of-wall stacked layout.
- `value`: present only for `Driving` dimensions. Reference dimensions are
  measured from the current model and must not store a target value.

Opening anchors include the stable opening ID:

```json
{
  "id": "dimension-1",
  "name": "Dimension 1",
  "kind": "Driving",
  "axis": "Horizontal",
  "start": { "kind": "wall_start" },
  "end": {
    "kind": "opening_center",
    "opening": "opening-front-door"
  },
  "direction": "Forward",
  "value": { "ticks": 960 }
}
```

Vertical dimensions use the same shape. For example, this constrains a window's
rough opening height:

```json
{
  "id": "dimension-2",
  "name": "Dimension 2",
  "kind": "Driving",
  "axis": "Vertical",
  "start": {
    "kind": "opening_point",
    "opening": "opening-front-window",
    "horizontal": "Center",
    "vertical": "Bottom"
  },
  "end": {
    "kind": "opening_point",
    "opening": "opening-front-window",
    "horizontal": "Center",
    "vertical": "Top"
  },
  "direction": "Forward",
  "value": { "ticks": 768 }
}
```

The current app applies wall-local driving dimensions for wall length and
height, opening horizontal position and width, and opening vertical bottom and
height. Reference dimensions are non-driving annotations that display the
current measured distance. Dimension `line_offset` values only place annotation
graphics; cross-wall projections and alignment constraints are future schema
extensions, not implicit behavior in v4.

Driving dimensions must be simultaneously satisfied by the authored wall and
opening geometry. If a new or edited driving dimension contradicts another
driving dimension on the same wall, the app rejects it during creation or
validation reports the dimension set as overconstrained instead of silently
choosing one target over another.

Each wall join stores:

- `kind`: currently `Corner`, `EndToEnd`, `Tee`, or `Cross`.
- `first_wall` and `second_wall`: the connected wall segment IDs.
- `point`: the project coordinate where the walls meet.

The solver currently generates corner-post members for `Corner` and `EndToEnd`
joins. `Tee` and `Cross` joins are stored as authored intent but reported as
unsupported for framing generation.

Level, wall, join, opening, and dimension IDs are stable semantic identifiers.
They must be non-empty and contain only lowercase letters, digits, or hyphens.
Examples:

- `level-1`
- `wall-1`
- `join-front-right`
- `opening-door-1`
- `opening-window-1`
- `dimension-1`

Do not rewrite existing IDs when changing dimensions or names. Add a new stable
ID only when adding a new authored object.

The checked-in IRC 2021 profile is named `IRC 2021 prescriptive starter profile`
because it is only a starter data shape for the early wall solver. It is not a
complete code-compliance claim.

Garage doors are stored as authored semantic openings, but the current solver
frames them as wide rough openings with starter king, jack, and header rules. It
emits an unsupported-condition diagnostic because garage-door-specific
structural design is not implemented.

## Determinism

Framer canonicalizes project files before saving:

- Levels are sorted by `id`.
- Walls are sorted by `id`.
- Openings within each wall are sorted by `id`.
- Dimensions within each wall are sorted by `id`.
- Wall joins are sorted by `id`.
- JSON is pretty-printed with a trailing newline.
- Generated framing is deterministic output and is not saved in v4 files.

This keeps `.framer` files stable for Git diffs, code review, and agent edits.

## Agent Editing Contract

When Codex, Claude, or another coding agent edits a `.framer` file:

1. Read this document and the project file before editing.
2. Edit only `authored` design intent.
3. Preserve `format` and `schema_version` unless performing an explicit schema
   migration.
4. Preserve existing stable IDs.
5. Keep authored intent separate from generated framing, cached view data,
   drawings, BOM exports, and UI state.
6. Use exact tick values for dimensions.
7. Keep deterministic ordering by ID, or re-save through Framer to canonicalize.
8. Do not present the starter IRC 2021 profile as complete code compliance.
9. Represent plan adjustments as authored design changes or explicit override
   records if the schema supports them; do not add generated members directly.
10. Validate after edits.

Recommended validation:

```sh
cargo fmt --all -- --check
cargo test --workspace
```

If an agent only changes example project files, it should still run the workspace
tests because the fixture is checked against canonical serialization and solver
round-tripping.

## App Support

The desktop app opens with the demo shell model and a default project path of
`examples/projects/demo-shell.framer`.

Use:

- `New` to create a fresh single-wall project.
- `Shell Demo` to return to the connected multi-wall alpha example.
- `Wall Demo` to return to the completed Phase 1 straight-wall example.
- `Open` and `Save` to load or persist the authored `.framer` file.
- `Design` to edit authored levels, wall placement, openings, and joins through
  the model tree, inspector, catalog, and authored-object viewports.
- `Shell` in Design Mode for top-down wall selection and `Wall` in Design Mode
  for laying out authored openings on the selected wall. Selecting a wall or
  opening in the Shell view opens the selected wall in Wall view.
- `Dimension` in Design Mode to pick two wall/opening anchors in Wall view and
  create either a driving dimension or a non-driving reference dimension. Driving
  dimension values can be edited in the inspector.
- `Plan` to inspect generated framing, diagnostics, BOM rows, read-only authored
  summaries, and selectable generated members.
- the catalog in Design Mode to add doors, windows, and garage doors to the
  selected wall.
- `Export` in Plan Mode to write disposable sidecar artifacts next to the
  project path:
  `<project>.svg` for the whole-project shell plan plus wall elevations and
  `<project>.csv` for the grouped whole-project BOM/cut list.

The SVG and CSV exports are regenerated outputs. Do not copy them back into the
canonical project document.
