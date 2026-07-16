# Framer Project Files

Framer project files are UTF-8 JSON documents with the `.framer` extension. The
format is intentionally text-first so humans, Git, and coding agents can inspect
and edit authored design intent without reverse-engineering an opaque binary
container.

The v14 format stores only the canonical intent model, including typed
project-authored cross-object assertions and explicit waiver records. Evaluated
outcomes, fact observations, diagnostics, generated framing plans, the
revision-bound project analysis graph and its query cache, typed room
schedule/boundary consequences, current library-lifecycle status, cached viewport
data, drawings, BOM exports, and other disposable artifacts are regenerated from
the authored model and must not be written into the canonical file.

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

## V14 Shape

```json
{
  "format": "framer.project",
  "schema_version": 14,
  "authored": {
    "site": { "jurisdiction": "" },
    "standards": [],
    "standards_packs": [],
    "libraries": [],
    "materials": [],
    "systems": [],
    "furnishings": [],
    "mep_objects": [],
    "levels": [],
    "walls": [],
    "wall_joins": [],
    "rooms": [],
    "furnishing_instances": [],
    "mep_instances": [],
    "roof_planes": [],
    "ceilings": [],
    "floor_decks": [],
    "braced_wall_lines": []
  }
}
```

- `format` must be `framer.project`.
- `schema_version` must be `14` when saving from the current app.
- `authored` contains the user-authored semantic model.
- Unknown top-level keys are rejected (`deny_unknown_fields`). Do not add
  `generated`, `analysis`, `graph`, `cache`, `exports`, or presentation data to
  project files.
- `site` stores jurisdiction and environmental inputs used by standards checks.
- `standards` is the ordered standards-pack stack. `standards_packs` embeds the
  self-contained pack definitions referenced by that stack. New projects seed
  both with the IRC 2021 starter pack.
- `libraries`, `materials`, `systems`, `furnishings`, `mep_objects`, `rooms`,
  `furnishing_instances`, `mep_instances`, `roof_planes`, `ceilings`,
  `floor_decks`, `braced_wall_lines`, `intents`, and `intent_overrides` are
  omitted when empty; `wall_joins`
  defaults to an empty list; `levels` defaults to a single `level-1`.
- `levels` carry an optional `height` (the top plane is `elevation + height`);
  a zero height is omitted. `systems` carry a `kind` of `Wall`, `Floor`, `Roof`,
  or `Ceiling`, each with exactly one framing layer. A framing layer's optional
  `member_family` (`Stud`, `Rafter`, `CeilingJoist`, `FloorJoist`, `Truss`) is
  omitted when it is the `Stud` default.
- `roof_planes` are sloped/flat structural faces (outline, pitch `slope`, eave
  edge, overhangs, and nested plane-local `openings`). A roof outline is an
  implicit ring: do not repeat the first point or insert duplicate/collinear
  boundary vertices. Overhangs are nonnegative, and same-level planes sharing an
  exact edge use one matching eave/rake pair so their derived seams stay closed;
  `ceilings` and
  `floor_decks` carry a `region` (an enclosed room id or an explicit polygon). A
  `ceiling` may carry an optional `slope` (`{ pitch, low_edge }` — a scissor/vault
  surface that springs from its polygon's `low_edge`); it is omitted for a flat
  ceiling, which keeps the region's two forms. A sloped ceiling requires an
  explicit polygon region.

### Schema versioning is v14-only

The current build is **v14-only**. On load, Framer peeks the file header and
**rejects** any `schema_version` other than `14` with an explicit
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

The current format is schema 3:

```json
{
  "format": "framer.library",
  "schema_version": 3,
  "uid": "8f6ebee0-fbdc-4f29-9d90-0e3f3f0640a8",
  "version_id": "019e8b10-9b30-7c2b-8b4e-1db251cb8221",
  "version": "0.1.0",
  "coordinate": "framer-lib://framer/starter",
  "materials": [],
  "systems": [],
  "furnishings": [],
  "mep_objects": [],
  "standards": []
}
```

- `format` must be `framer.library`.
- `schema_version` must be `3` for the current library loader.
- `uid` is the stable library identity; `version_id` identifies this published
  content version; `coordinate` is only a resolvable hint.
- `materials`, `systems`, `furnishings`, `mep_objects`, and `standards` use the
  same typed definitions as project files. Library `standards` entries become
  project `standards_packs` when vendored.
- A library validates internally before save/load succeeds: IDs must be valid and
  unique, and every construction-system material reference must resolve to a
  material in the same file. Furnishing and MEP family sizes must be positive,
  and every standards pack must pass `StandardsPack::validate()`.

The checked-in starter catalog is
[`../libraries/framer-starter.framerlib`](../libraries/framer-starter.framerlib).
New projects and demos load that document for material/system definitions and
seed the same IRC 2021 starter standards pack into the authored project model;
the starter catalog also distributes that pack for vendor workflows. Furnishing
and MEP object families are copied when placed from the starter catalog. Opening
an existing `.framer` project does not read any `.framerlib`; projects remain
self-contained.

## Project Packages

Portable project packages use the `.framerpkg` extension. A package is a
deterministic ZIP with stored entries, sorted paths, and zeroed timestamps:

- `project.framer`: the canonical v14 project JSON.
- `manifest.json`: `{ "format": "framer.package", "schema_version": 1, ... }`.
- `assets/blake3-<hex>`: optional content-addressed binary assets referenced by
  material appearances.

The bare `.framer` file remains the primary, inspectable project format. Asset
bytes are disposable caches: if a texture/depth asset is missing, rendering falls
back to the material's authored color and the project still opens.

## Authored Model

The v14 authored model holds:

- `site`: project jurisdiction and environmental assumptions.
- `standards`: ordered standards-pack stack. Later packs override earlier packs.
- `standards_packs`: embedded standards-pack definitions, including framing
  defaults, prescriptive tables, compliance checks, and overlays.
- `libraries`: optional descriptive stamps for library versions that supplied
  vendored definitions.
- `materials`: the project's material library (see below).
- `systems`: the project's construction systems (layered assemblies, see below).
- `furnishings`: reusable furnishing family definitions.
- `mep_objects`: reusable MEP object family definitions.
- `levels`: deterministic list of project levels.
- `walls`: deterministic list of placed rectilinear wall segments.
- `wall_joins`: deterministic list of authored wall joins/corners.
- `rooms`: deterministic list of authored rooms (spaces).
- `furnishing_instances`: level-owned placed furnishing instances.
- `mep_instances`: level-owned placed MEP object instances.
- `roof_planes`: level-owned sloped/flat structural roof faces (minimal implicit-ring outline,
  pitch `slope`, eave edge, nonnegative overhangs, nested plane-local `openings`). Same-level
  exact-edge-connected planes must share their eave/rake values.
- `ceilings`: level-owned finished ceiling surfaces over a `region` (room or
  polygon) at an authored `height`, with an optional `slope`
  (`{ pitch, low_edge }`) for a scissor/vault surface (flat when omitted; a slope
  requires an explicit polygon region).
- `floor_decks`: level-owned structural floor decks over a `region` with a joist
  `span` direction.
- `braced_wall_lines`: optional authored braced wall lines, used by standards
  checks.
- `intents`: optional, id-sorted project-authored requirement/preference
  assertions over exact furnishing/MEP instance and room participants.
- `intent_overrides`: optional, id-sorted explicit waivers targeting authored
  assertion ids; every waiver carries a non-empty reason.
- Per wall: `openings` (wall openings), `dimensions` (wall-local dimension
  constraints), and `bracing` (optional braced panels).

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
- `openings`, `dimensions`, `bracing`, and optional `tags` (free-form string
  lists are omitted when empty).

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

Braced panels are wall-local spans. They are omitted when a wall has no authored
bracing:

```json
{
  "id": "panel-front-1",
  "offset": { "ticks": 384 },
  "length": { "ticks": 768 },
  "method": "Wsp"
}
```

Braced wall lines are level-owned project-coordinate segments:

```json
{
  "id": "bwl-front",
  "name": "Front braced wall line",
  "level": "level-1",
  "start": { "x": { "ticks": 0 }, "y": { "ticks": 0 } },
  "end": { "x": { "ticks": 3072 }, "y": { "ticks": 0 } }
}
```

## Construction Systems & Materials

A **construction system** is a named, reusable assembly: an ordered stack of
material layers across the element's thickness, applied by reference to walls,
roof planes, ceilings, and floor decks (each system carries a `kind` matching the
element it clads). New projects seed a starter library (exterior + interior wall
systems, a roof, a floor, and a ceiling system, plus their materials). The durable
intent behind this model is documented in the
[Construction Systems spec](specs/construction-systems.md).

Each material in `materials` stores:

- `id`: a stable semantic identifier (e.g. `mat-drywall`).
- `name`: a human-readable name.
- `source`: `Project` (default, omitted) or `Library(Provenance)` for vendored
  definitions copied from a `.framerlib`.
- `appearance`: an authored finish. `SolidColor` stores only a fallback color;
  `Textured` and `DepthMapped` store the same color plus an `AssetRef` and
  scale. Asset bytes are never embedded in `.framer`.
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

Asset-backed appearances use content hashes:

```json
{
  "appearance": {
    "Textured": {
      "color": [170, 110, 70],
      "texture": {
        "hash": "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "media_type": "image/png",
        "role": "Texture"
      },
      "scale": { "ticks": 384 }
    }
  }
}
```

`DepthMapped` uses the same shape with `height` and `role: "Height"`. `scale` is
a positive `Length` in ticks.

Each system in `systems` stores:

- `id`, `name`.
- `kind`: `Wall`, `Floor`, `Roof`, or `Ceiling` (all four are wired — authoring, solver, and
  render; see [specs/ceilings-and-roofs.md](specs/ceilings-and-roofs.md)).
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

## Furnishings and MEP Objects

Furnishing and MEP object definitions are reusable **families**. Instances place
those families in the project by reference. This mirrors walls referencing
construction systems: the family definition owns the reusable size/properties,
and each instance owns level, position, rotation, name, and tags.

Each furnishing in `furnishings` stores:

- `id`, `name`.
- `source`: optional `Provenance` for families copied from a `.framerlib`.
- `size`: exact `width`, `depth`, and `height` lengths. All three must be
  positive.
- `tags`: optional free-form string list, omitted when empty.
- `properties`: an extensible float-free typed map (`Int` / `Length` / `Text` /
  `Flag`).

```json
{
  "id": "furnishing-workbench",
  "name": "Workbench",
  "size": {
    "width": { "ticks": 1152 },
    "depth": { "ticks": 480 },
    "height": { "ticks": 576 }
  },
  "tags": ["shop"]
}
```

Each MEP object in `mep_objects` uses the same shape plus `kind`, one of
`Electrical`, `Lighting`, `Plumbing`, `Mechanical`, or `Other`.

```json
{
  "id": "mep-load-center",
  "name": "Load center",
  "kind": "Electrical",
  "size": {
    "width": { "ticks": 224 },
    "depth": { "ticks": 64 },
    "height": { "ticks": 384 }
  },
  "tags": ["electrical"],
  "properties": { "amperage": { "Int": 200 } }
}
```

Placed instances store a local family reference and a project coordinate. The
family and level must both exist in this file.

```json
{
  "id": "furnishing-instance-1",
  "name": "Workbench 1",
  "family": "furnishing-workbench",
  "level": "level-1",
  "position": { "x": { "ticks": 384 }, "y": { "ticks": 576 } },
  "rotation": "Deg90"
}
```

`rotation` is a quarter-turn enum: `Deg0`, `Deg90`, `Deg180`, or `Deg270`.
`Deg0` is omitted. MEP instances use the same shape in `mep_instances` and point
their `family` at an id in `mep_objects`.

Model plan coordinates are right-handed: `+X` points right and `+Y` points up.
Family `width` is local left-to-right, `depth` is local back-to-front, and the
instance position is the footprint center. `Deg0` front is local `+Y`, and
positive `QuarterTurn` rotation is counterclockwise: `Deg90` maps front to
model `-X`, `Deg180` to `-Y`, and `Deg270` to `+X`. Screen coordinates do not
change this model convention.

## Cross-Object Intent and Waivers

Schema v14 adds two skip-empty authored collections. `intents` stores typed
cross-object assertions; `intent_overrides` stores explicit exceptions to those
assertions. Outcomes, measured facts, report rows, graph edges, and diagnostics
are derived and never belong in either collection.

The first persisted vertical slice intentionally accepts only:

- `domain`: `SpatialProgram`, `Mep`, `Compliance`, or
  `OperationalMaintenance`. The Rust enum contains the complete product domain
  vocabulary, but the other domains are rejected until an end-to-end evaluator
  exists.
- `mode`: `Requirement` or `Preference { priority }`. Preference priority is a
  nonzero `u16`; larger values are stronger. Objectives and assumptions remain
  derived-protocol types and are not valid authored modes in v14.
- `scope`: one exact furnishing or MEP instance as `subject` and exactly one
  same-level room in `participants`. The references are kind-checked; strings
  that exist under a different entity kind are rejected rather than coerced.
- `expression`: a shared `FactPredicate` over
  `PlacedObjectContainedInRoom` or parameterized `PlacedObjectClearance` facts.
  `All`, `Any`, `Not`, and type-correct `Compare` trees are accepted; empty
  groups, non-placed-object facts, invalid flag operators, type mismatches, and
  negative clearance thresholds are rejected.
- `source`: `User`. `rationale` is optional.

A canonical example containing both fact forms and a project waiver is:

```json
{
  "intents": [
    {
      "id": "intent-toilet-contained",
      "domain": "SpatialProgram",
      "mode": "Requirement",
      "scope": {
        "Exact": {
          "subject": { "MepInstance": "mep-toilet-1" },
          "participants": [{ "Room": "room-bath-1" }]
        }
      },
      "expression": {
        "FactPredicate": {
          "Compare": {
            "fact": "PlacedObjectContainedInRoom",
            "op": "Eq",
            "value": { "FlagLiteral": true }
          }
        }
      },
      "source": "User",
      "rationale": "Keep the toilet footprint inside the bathroom"
    },
    {
      "id": "intent-toilet-front-clearance",
      "domain": "OperationalMaintenance",
      "mode": { "Preference": { "priority": 200 } },
      "scope": {
        "Exact": {
          "subject": { "MepInstance": "mep-toilet-1" },
          "participants": [{ "Room": "room-bath-1" }]
        }
      },
      "expression": {
        "FactPredicate": {
          "Compare": {
            "fact": {
              "PlacedObjectClearance": {
                "direction": "Front",
                "datum": "FootprintFace"
              }
            },
            "op": "Ge",
            "value": { "LengthLiteral": { "ticks": 480 } }
          }
        }
      },
      "source": "User",
      "rationale": "Prefer 30 inches clear in front"
    }
  ],
  "intent_overrides": [
    {
      "Waive": {
        "id": "override-toilet-front-existing",
        "target": "intent-toilet-front-clearance",
        "reason": "Owner accepted the existing-condition approach",
        "source": "User"
      }
    }
  ]
}
```

`PlacedObjectContainedInRoom` observes whether the complete rotated rectangular
family footprint lies inside the exact derived room boundary.
`PlacedObjectClearance` carries a `direction` (`Left`, `Right`, `Front`, `Back`,
or `Around`) and `datum` (`Centerline` or `FootprintFace`). It measures in the
rotated local direction across the target footprint's perpendicular span to the
nearest finished room-wall face or other same-level furnishing/MEP footprint;
`Around` is the minimum of all four cardinal results. A geometric containment
miss is a known `false` and yields zero clearance. An open room boundary,
unresolved family geometry, or missing wall-system input yields a derived
unknown result instead of a pass. These observations come only from
`framer-standards::FactSnapshot`, whether the predicate originated in a standards
check or a project assertion.

Reusable standards packs use selector scope rather than project ids. A
`CheckScope::PlacedObjects` rule resolves an instance to an exact room only when
its center lies in exactly one closed same-level authored room; zero closed
matches are unresolved and multiple matches are ambiguous. Both cases remain explicit
unknown fact subjects and are not silently skipped.

An `IntentOverride::Waive` targets one known authored intent id and requires a
non-whitespace reason. At most one project override may target an assertion.
The override changes the derived outcome to `Waived`; it is not a second
assertion and does not authorize editing generated geometry.

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

Each vendored material, system, furnishing, MEP object, or standards pack may also carry
`Provenance`:

- `library_uid` and `version_id`: point to the matching library stamp.
- `source_id`: the element id inside the library before project-local remapping.
- `content_hash`: `blake3:<hex>` hash of the source item canonical form.

This metadata is descriptive. Do not make wall/material/system/family/standards-stack
references point outside the project; every layer `material`, framing `cavity_material`,
wall `system`, object-instance `family`, and standards stack entry must still resolve to a
local definition in this file.

Library lifecycle state is derived from the embedded definitions plus any
currently available source library. A vendored item whose project-local content
no longer matches its stamped source-item hash is reported as locally modified;
an item whose available source library now has a different source-item hash is
reported as out of date. Re-sync overwrites the embedded vendored definition from
the source library while keeping project-local ids stable; systems also refresh
their material closure. Standards packs re-sync as single definitions because
their rules do not reference material/system ids. Detach clears the selected
item's provenance so it becomes ordinary project-owned content. None of these
checks run during `load_project`, and a missing `.framerlib` never blocks
opening this file.

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

Standards pack, material, system, furnishing, MEP object, placed object
instance, level, wall, join, opening, dimension, bracing panel, braced wall line,
room, authored intent, and intent override IDs are stable semantic identifiers.
They share one global id pool, must be non-empty, and contain only lowercase
letters, digits, or hyphens. Examples:

- `mat-drywall`, `system-wall-exterior-1`
- `furnishing-workbench`, `mep-load-center`, `furnishing-instance-1`
- `level-1`, `wall-1`, `join-front-right`
- `opening-door-1`, `opening-window-1`, `dimension-1`, `panel-front-1`,
  `bwl-front`, `room-bed-1`
- `intent-toilet-front-clearance`, `override-toilet-front-existing`

Do not rewrite existing IDs when changing properties or names. Add a new stable ID
only when adding a new authored object.

The checked-in IRC 2021 standards pack is named `IRC 2021 Prescriptive (starter)`
because it is only a starter data shape for the early wall solver. It is not a
complete code-compliance claim.

Garage doors are stored as authored semantic openings, but the current solver
frames them as wide rough openings with starter king, jack, and header rules. It
emits an unsupported-condition diagnostic because garage-door-specific structural
design is not implemented.

## Determinism

Framer canonicalizes project files before saving:

- Materials are sorted by `id`.
- Standards packs, systems, furnishings, MEP objects, furnishing instances, MEP
  instances, braced wall lines, intents, and intent overrides are sorted by
  `id`; **layers within a system
  are not sorted** (layer order is semantic: interior → exterior), and the
  `standards` stack order is semantic.
- Levels, walls, wall joins, and rooms are sorted by `id`.
- Openings, dimensions, and bracing panels within each wall are sorted by `id`.
- A material's `properties` map is ordered (a `BTreeMap`), so property insertion
  order does not affect output.
- Exact-scope participant order and `All`/`Any` predicate-child order are
  semantic and are not silently resorted. Schema v14 currently requires exactly
  one room participant.
- JSON is pretty-printed with a trailing newline.
- Measured fact observations, intent outcomes and compiled waiver records,
  generated diagnostics and framing, room schedule/boundary consequence nodes,
  library-lifecycle status,
  and the derived project graph are deterministic output and are not saved.
  `GraphRevision` fingerprints the analysis contract version, a
  length-delimited deterministic starter-library source input (availability plus
  content hash when available), and canonical project bytes; it is a
  cache/evidence boundary, not project data.

This keeps `.framer` files stable for Git diffs, code review, and agent edits. The
three checked-in `examples/projects/*.framer` files are verified byte-for-byte
against canonical serialization by the round-trip tests in
`crates/framer-core/src/project.rs`.

## Agent Editing Contract

When Codex, Claude, or another coding agent edits a `.framer` file:

1. Read this document and the project file before editing.
2. Edit only `authored` design intent.
3. Preserve `format` and `schema_version` (`14`). The build is v14-only; do not
   hand-write a different version.
4. Preserve existing stable IDs.
5. Keep authored intent separate from generated framing, room consequences,
   library-lifecycle status, analysis graphs and query caches, cached view data,
   drawings, BOM exports, and UI state.
6. Use exact tick values for dimensions and thicknesses.
7. Apply construction and object families by *reference*: point a wall's `system`
   at a system `id`, a layer's `material` at a material `id`, and a placed
   object instance's `family` at a matching furnishing or MEP family `id`, and a
   standards stack entry at a matching `standards_packs` `id`. Do not dangle
   references — every referenced `system`/`material`/`family`/standards pack must
   exist. Keep layer order interior → exterior and keep standards stack order
   semantic.
8. Give every `IntentAssertion` and `IntentOverride` a new globally unique stable
   id. Use an exact furnishing/MEP instance subject and one same-level room
   participant; do not substitute labels or vector indices for typed references.
   Keep the v14 allowlist, use only nonzero preference priorities, and keep fact
   operands type-correct. A waiver must target a known authored assertion, have a
   non-whitespace reason, and be the only project override for that target.
9. Keep roof outlines as minimal implicit rings (no repeated closing point,
   consecutive duplicate, or redundant collinear vertex), keep overhangs
   nonnegative, and use matching eave/rake values across every same-level
   exact-edge-connected roof assembly.
10. Keep deterministic ordering by ID, or re-save through Framer to canonicalize.
    Preserve semantic order inside construction layers, standards stacks, exact
    participants, and predicate children.
11. Do not present the starter IRC 2021 standards pack as complete code
   compliance.
12. Represent plan adjustments as authored design changes. Use
    `IntentOverride::Waive` only as an explicit exception to an authored
    assertion; it does not permit adding or changing generated members directly.
13. Validate after edits.

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
  system library; starter object families are available from the catalog when placed).
- `Shell Demo` / `Wall Demo` to return to the connected multi-wall or single-wall
  examples.
- `Open` and `Save` to load or persist the authored `.framer` file.
- `Design` to edit authored levels, wall placement, openings, joins, construction
  systems, materials, furnishing/MEP families, and placed furnishing/MEP instances
  through the model tree, inspector, catalog, and authored viewports. The
  inspector's **Intent** section authors, edits, deletes, and waives schema-v14
  project assertions through the ordinary validated edit/history path; it requires
  a rationale while authoring even though the file schema keeps `rationale`
  optional for agent-authored data.
- `Shell` in Design Mode for top-down wall selection and `Wall` in Design Mode for
  laying out authored openings on the selected wall.
- the Design-mode `Wall` (W) and `Room` (R) tools to draw walls and place rooms in
  the plan view, starter-catalog `Place` actions to place furnishing/MEP instances,
  and the `Dimension` tool in the wall view to create driving or reference dimensions.
- `Plan` to inspect generated framing, diagnostics, BOM rows (including the
  fastening and per-layer material takeoffs), read-only authored summaries, and
  selectable generated members. The inspector's **Intent** section combines
  read-only regenerated current status, dependencies, possible impact, and
  generated provenance; it does not persist those derived views. Authored intent
  mutation controls are disabled in Plan, while the read-only **Focus all** action
  remains available to select every exact participant. Explicit placement
  resolution requests, ranked options, outcome evidence, and Plan ghost previews
  are disposable derived state and are never written to `.framer`; only an
  accepted typed placement change updates the ordinary authored instance fields. Plan can inspect
  and preview those options, but acceptance is enabled only in the Design workspace and commits
  one ordinary validated, undoable authored edit.
- `Export` in Plan Mode to write disposable sidecar artifacts next to the project
  path: `<project>.svg` for the shell plan plus wall elevations, `<project>.csv`
  for the grouped whole-project BOM/cut list plus fastener takeoff section, and
  `<project>.compliance.csv` for the derived standards compliance report.

The SVG and CSV exports are regenerated outputs. Do not copy them back into the
canonical project document.
