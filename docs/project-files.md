# Framer Project Files

Framer project files are UTF-8 JSON documents with the `.framer` extension. The
format is intentionally text-first so humans, Git, and coding agents can inspect
and edit authored design intent without reverse-engineering an opaque binary
container.

The v8 format stores only the canonical intent model. Generated framing plans,
cached viewport data, drawings, BOM exports, and other disposable artifacts are
regenerated from the authored model and must not be written into the canonical
file.

This matches Framer's Design Mode / Plan Mode split. Design Mode writes the
authored model saved here. Plan Mode regenerates framing layouts, diagnostics,
drawings, schedules, and exports from that authored model.

See
[../examples/projects/demo-shell.framer](../examples/projects/demo-shell.framer)
for a complete checked-in multi-wall example with a connected shell, corner
joins, doors, windows, and a garage-door-style opening;
[../examples/projects/demo-two-bedroom.framer](../examples/projects/demo-two-bedroom.framer)
for interior partitions (tee joins) and rooms; and
[../examples/projects/demo-wall.framer](../examples/projects/demo-wall.framer)
for the single-wall example.

> For the in-memory types behind this format, see
> [code-map.md](code-map.md#framer-core--the-domain-model). The serialization code
> is `crates/framer-core/src/project.rs`; the companion `.framerlib` library
> format is implemented in `crates/framer-core/src/library.rs`.

## V8 Shape

```json
{
  "format": "framer.project",
  "schema_version": 8,
  "authored": {
    "code": {},
    "libraries": [],
    "materials": [],
    "systems": [],
    "levels": [],
    "walls": [],
    "wall_joins": [],
    "rooms": []
  }
}
```

- `format` must be `framer.project`.
- `schema_version` must be `8` when saving from the current app.
- `authored` contains the user-authored semantic model.
- Unknown top-level keys are rejected (`deny_unknown_fields`). Do not add
  `generated`, `cache`, `exports`, or presentation data to project files.
- `libraries`, `materials`, `systems`, and `rooms` are omitted when empty;
  `wall_joins` defaults to an empty list; `levels` defaults to a single
  `level-1`.

### Schema versioning is v8-only

The current build is **v8-only**. On load, Framer peeks the file header and
**rejects** any `schema_version` other than `8` with an explicit
`unsupported Framer project schema version N` error — older files are *not*
migrated in place. Convert old files with an older Framer build, or re-author
them.

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

## Library Files

Framer library files are UTF-8 JSON documents with the `.framerlib` extension.
They are versioned separately from project files and describe reusable content
that can be copied into a self-contained `.framer` project.

The initial format is schema 1:

```json
{
  "format": "framer.library",
  "schema_version": 1,
  "uid": "8f6ebee0-fbdc-4f29-9d90-0e3f3f0640a8",
  "version_id": "019e8b10-9b30-7c2b-8b4e-1db251cb8221",
  "version": "0.1.0",
  "coordinate": "framer-lib://framer/starter",
  "materials": [],
  "systems": []
}
```

- `format` must be `framer.library`.
- `schema_version` must be `1` for the current library loader.
- `uid` is the stable library identity; `version_id` identifies this published
  content version; `coordinate` is only a resolvable hint.
- `materials` and `systems` use the same typed definitions as project files.
- A library validates internally before save/load succeeds: IDs must be valid and
  unique, and every construction-system material reference must resolve to a
  material in the same file.

The checked-in starter catalog is
[`../libraries/framer-starter.framerlib`](../libraries/framer-starter.framerlib).
New projects and demos load that document and embed its material/system
definitions into the authored project model. Opening an existing `.framer` project
does not read any `.framerlib`; projects remain self-contained.

## Authored Model

The v8 authored model holds:

- `code`: the prescriptive code profile (starter framing defaults).
- `libraries`: optional descriptive stamps for library versions that supplied
  vendored definitions.
- `materials`: the project's material library (see below).
- `systems`: the project's construction systems (layered assemblies, see below).
- `levels`: deterministic list of project levels.
- `walls`: deterministic list of placed rectilinear wall segments.
- `wall_joins`: deterministic list of authored wall joins/corners.
- `rooms`: deterministic list of authored rooms (spaces).
- Per wall: `openings` (wall openings) and `dimensions` (wall-local dimension
  constraints).

Each wall stores a local framing length, a project placement, and a **reference to
a construction system** (it does not embed its assembly):

- `level`: the level that owns the wall segment.
- `start` and `end`: rectilinear project coordinates in ticks.
- `length`: the wall's local framing length; it must match the axis-aligned
  distance between `start` and `end`.
- `height`: the wall's framing height.
- `system`: the `id` of a `ConstructionSystem` in the project `systems` list. The
  generated framing uses that system's framing layer, so changing it re-sizes the
  wall's studs, plates, corner posts, and per-layer takeoff.
- `openings`, `dimensions`, and optional `tags` (a free-form string list, omitted
  when empty).

```json
{
  "id": "wall-1",
  "name": "Front wall",
  "level": "level-1",
  "start": { "x": { "ticks": 0 }, "y": { "ticks": 0 } },
  "end": { "x": { "ticks": 3072 }, "y": { "ticks": 0 } },
  "length": { "ticks": 3072 },
  "height": { "ticks": 1536 },
  "system": "system-wall-exterior-1",
  "openings": []
}
```

## Construction Systems & Materials

A **construction system** is a named, reusable assembly: an ordered stack of
material layers across the element's thickness, applied to walls by reference. New
projects seed a starter library (exterior + interior wall systems and their
materials). The durable intent behind this model is documented in the
[Construction Systems spec](specs/construction-systems.md).

Each material in `materials` stores:

- `id`: a stable semantic identifier (e.g. `mat-drywall`).
- `name`: a human-readable name.
- `source`: `Project` (default, omitted) or `Library(Provenance)` for vendored
  definitions copied from a `.framerlib`.
- `appearance`: an authored finish, currently `{ "SolidColor": [r, g, b] }`.
- `tags`: optional free-form string list, omitted when empty.
- `properties`: an extensible, **float-free** map of typed values
  (`Int` / `Length` / `Text` / `Flag`). Substance lives here rather than in the
  schema — e.g. `"r_per_inch_milli": { "Int": 900 }` (R-value × 1000 per inch).

```json
{
  "id": "mat-drywall",
  "name": "5/8\" Gypsum",
  "appearance": { "SolidColor": [228, 226, 220] },
  "tags": ["finish"],
  "properties": { "r_per_inch_milli": { "Int": 900 } }
}
```

Each system in `systems` stores:

- `id`, `name`.
- `kind`: `Wall`, `Floor`, or `Roof` (only `Wall` is wired today).
- `source`: optional `Provenance` for systems copied from a `.framerlib`.
- `layers`: an ordered list from **interior → exterior**. Layer order is semantic
  and is **never sorted**.

Each layer stores:

- `function`: one of `InteriorFinish`, `Framing`, `ContinuousInsulation`,
  `Sheathing`, `WeatherBarrier`, `AirGap`, `Cladding`, `Masonry`, `Structure`,
  `Other`.
- `material`: the `id` of a material in the library.
- `thickness`: the layer thickness in ticks.
- `framing`: present **if and only if** `function == Framing`. It stores `member`
  (a `BoardProfile` such as `TwoByFour`), `spacing`, `pattern`
  (`Single`/`Staggered`/`Double`), and an optional `cavity_material` (insulation
  between studs, which adds no extra through-wall depth).

A **wall system must have exactly one framing layer.** Validation rejects systems
with no layers, an unknown material reference, a framing/`function` mismatch,
non-positive thickness or spacing, or the wrong framing-layer count. A wall that
references an unknown `system` is also rejected.

```json
{
  "id": "system-wall-interior-1",
  "name": "Interior 2x4 partition",
  "kind": "Wall",
  "layers": [
    { "function": "InteriorFinish", "material": "mat-drywall", "thickness": { "ticks": 10 } },
    {
      "function": "Framing",
      "material": "mat-spf",
      "thickness": { "ticks": 56 },
      "framing": { "member": "TwoByFour", "spacing": { "ticks": 256 }, "pattern": "Single" }
    },
    { "function": "InteriorFinish", "material": "mat-drywall", "thickness": { "ticks": 10 } }
  ]
}
```

Total through-wall thickness, derived exposure (Exterior vs Interior), and the
clear-wall R-value are *derived* from the layer stack and materials — they are not
stored. The R-value is a clear-wall approximation; the framing-factor
(parallel-path) derate is not yet applied.

### Library provenance

Using a library item copies the full definition into the project. The project
stays self-contained: opening, validating, solving, and rendering never read a
`.framerlib` file.

When a copied item came from a library, the project records one `libraries` stamp
for that library version:

- `uid`: stable library identity.
- `version_id`: immutable published-version identity.
- `content_hash`: `blake3:<hex>` hash of the canonical `.framerlib` bytes.
- `coordinate`: human/resolver hint, not identity.
- `version`: human semver label.

Each vendored material/system may also carry `Provenance`:

- `library_uid` and `version_id`: point to the matching library stamp.
- `source_id`: the element id inside the library before project-local remapping.
- `content_hash`: `blake3:<hex>` hash of the source item canonical form.

This metadata is descriptive. Do not make wall/material/system references point
outside the project; every layer `material`, framing `cavity_material`, and wall
`system` must still resolve to a local definition in this file.

## Dimensions

Each wall may carry driving or reference dimensions between wall-local anchors on
either the horizontal or vertical wall-elevation axis. Each dimension stores:

- `id`, `name`.
- `kind`: `Driving` or `Reference`.
- `axis`: `Horizontal` or `Vertical`.
- `start` and `end`: wall-local anchors. Anchors use `wall_point` or
  `opening_point` with a horizontal `Left`/`Center`/`Right` and a vertical
  `Bottom`/`Center`/`Top` reference, allowing dimensions to snap to edges,
  vertices, and centers. (The earlier shorthand anchors `wall_start`, `wall_end`,
  `opening_left`, `opening_center`, `opening_right` remain accepted.)
- `direction`: `Forward` or `Backward`, preserving the click order used to place
  the dimension.
- `line_offset`: optional wall-local annotation placement on the axis
  perpendicular to the dimension. Horizontal dimensions use this as the line's
  local Y position; vertical dimensions use it as the line's local X position.
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

The app applies wall-local driving dimensions for wall length and height, opening
horizontal position and width, and opening vertical bottom and height. Reference
dimensions are non-driving annotations that display the current measured distance.
Dimension `line_offset` values only place annotation graphics; cross-wall
projections and alignment constraints are future schema extensions, not implicit
behavior.

Driving dimensions must be simultaneously satisfied by the authored wall and
opening geometry. If a new or edited driving dimension contradicts another driving
dimension on the same wall, the app rejects it during creation or validation
reports the dimension set as overconstrained instead of silently choosing one
target over another.

## Wall Joins

Each wall join stores:

- `kind`: `Corner`, `EndToEnd`, `Tee`, or `Cross`.
- `first_wall` and `second_wall`: the connected wall segment IDs.
- `point`: the project coordinate where the walls meet.

The join point connects the walls according to the kind:

- `Corner`/`EndToEnd`: the point is an endpoint of both walls.
- `Tee`: the point is an endpoint of one wall (the partition) and lies on the
  interior (mid-span) of the other (the through wall).
- `Cross`: the point lies in the interior of both walls.

The solver generates corner-post members for `Corner` and `EndToEnd` joins; for
`Tee` joins it generates a partition end stud plus a backing stud in the through
wall (no corner post); for `Cross` joins it generates backing studs in both walls
and notes that interrupting one wall for a true cross is not yet modelled.

## Rooms

`rooms` is a deterministic list of authored rooms (spaces). A room persists only
its identity and a seed point; its boundary, area, and perimeter are *derived*
from the surrounding wall loop each time the plan is regenerated and are never
stored. A room whose seed is no longer enclosed by a closed wall loop is reported
with a `room.boundary.open` warning rather than failing validation.

Each room stores:

- `id`: a stable semantic identifier.
- `name`: a human-readable name (e.g. `Bedroom 1`).
- `usage`: one of `Unspecified` (default), `Living`, `Bedroom`, `Bathroom`,
  `Kitchen`, `Dining`, `Office`, `Hallway`, `Closet`, `Utility`, `Garage`, or
  `Other`. Omitted/`Unspecified` when not set.
- `level`: the level that owns the room.
- `seed`: a project coordinate inside the room, used to locate its bounding loop.
- `tags`: optional free-form string list, omitted when empty.

```json
{
  "id": "room-bed-1",
  "name": "Bedroom 1",
  "usage": "Bedroom",
  "level": "level-1",
  "seed": { "x": { "ticks": 1152 }, "y": { "ticks": 768 } }
}
```

## Stable IDs

Material, system, level, wall, join, opening, dimension, and room IDs are stable
semantic identifiers. They must be non-empty and contain only lowercase letters,
digits, or hyphens. Examples:

- `mat-drywall`, `system-wall-exterior-1`
- `level-1`, `wall-1`, `join-front-right`
- `opening-door-1`, `opening-window-1`, `dimension-1`, `room-bed-1`

Do not rewrite existing IDs when changing properties or names. Add a new stable ID
only when adding a new authored object.

The checked-in IRC 2021 profile is named `IRC 2021 prescriptive starter profile`
because it is only a starter data shape for the early wall solver. It is not a
complete code-compliance claim.

Garage doors are stored as authored semantic openings, but the current solver
frames them as wide rough openings with starter king, jack, and header rules. It
emits an unsupported-condition diagnostic because garage-door-specific structural
design is not implemented.

## Determinism

Framer canonicalizes project files before saving:

- Materials are sorted by `id`.
- Systems are sorted by `id`; **layers within a system are not sorted** (layer
  order is semantic: interior → exterior).
- Levels, walls, wall joins, and rooms are sorted by `id`.
- Openings and dimensions within each wall are sorted by `id`.
- A material's `properties` map is ordered (a `BTreeMap`), so property insertion
  order does not affect output.
- JSON is pretty-printed with a trailing newline.
- Generated framing is deterministic output and is not saved.

This keeps `.framer` files stable for Git diffs, code review, and agent edits. The
three checked-in `examples/projects/*.framer` files are verified byte-for-byte
against canonical serialization by the round-trip tests in
`crates/framer-core/src/project.rs`.

## Agent Editing Contract

When Codex, Claude, or another coding agent edits a `.framer` file:

1. Read this document and the project file before editing.
2. Edit only `authored` design intent.
3. Preserve `format` and `schema_version` (`8`). The build is v8-only; do not
   hand-write a different version.
4. Preserve existing stable IDs.
5. Keep authored intent separate from generated framing, cached view data,
   drawings, BOM exports, and UI state.
6. Use exact tick values for dimensions and thicknesses.
7. Apply construction by *reference*: point a wall's `system` at a system `id`, and
   a layer's `material` at a material `id`. Do not dangle references — every
   referenced `system`/`material` must exist. Keep layer order interior → exterior.
8. Keep deterministic ordering by ID, or re-save through Framer to canonicalize.
9. Do not present the starter IRC 2021 profile as complete code compliance.
10. Represent plan adjustments as authored design changes or explicit override
    records if the schema supports them; do not add generated members directly.
11. Validate after edits.

Recommended validation:

```sh
cargo fmt --all -- --check
cargo test --workspace
```

If an agent only changes example project files, it should still run the workspace
tests because the fixtures are checked against canonical serialization and solver
round-tripping.

## App Support

The desktop app opens with the demo shell model and a default project path of
`examples/projects/demo-shell.framer`.

Use:

- `New` to create a fresh single-wall project (seeded with the starter material +
  system library).
- `Shell Demo` / `Wall Demo` to return to the connected multi-wall or single-wall
  examples.
- `Open` and `Save` to load or persist the authored `.framer` file.
- `Design` to edit authored levels, wall placement, openings, joins, construction
  systems, and materials through the model tree, inspector, catalog, and authored
  viewports.
- `Shell` in Design Mode for top-down wall selection and `Wall` in Design Mode for
  laying out authored openings on the selected wall.
- the Design-mode `Wall` (W) and `Room` (R) tools to draw walls and place rooms in
  the plan view, and the `Dimension` tool in the wall view to create driving or
  reference dimensions.
- `Plan` to inspect generated framing, diagnostics, BOM rows (including the
  per-layer material takeoff), read-only authored summaries, and selectable
  generated members.
- `Export` in Plan Mode to write disposable sidecar artifacts next to the project
  path: `<project>.svg` for the shell plan plus wall elevations and `<project>.csv`
  for the grouped whole-project BOM/cut list.

The SVG and CSV exports are regenerated outputs. Do not copy them back into the
canonical project document.
