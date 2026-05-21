# Framer Project Files

Framer project files are UTF-8 JSON documents with the `.framer` extension. The
format is intentionally text-first so humans, Git, and coding agents can inspect
and edit authored design intent without reverse-engineering an opaque binary
container.

The v1 format stores only the canonical intent model. Generated framing plans,
cached viewport data, drawings, BOM exports, and other disposable artifacts are
regenerated from the authored model and must not be written into the canonical
v1 file.

See [../examples/projects/demo-wall.framer](../examples/projects/demo-wall.framer)
for a complete checked-in alpha example with a door, window, and
garage-door-style opening.

## V1 Shape

```json
{
  "format": "framer.project",
  "schema_version": 1,
  "authored": {
    "code": {},
    "walls": []
  }
}
```

- `format` must be `framer.project`.
- `schema_version` must be `1`.
- `authored` contains the user-authored semantic model.
- Unknown top-level keys are rejected. Do not add `generated`, `cache`,
  `exports`, or presentation data to v1 project files.

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

The current v1 authored model supports the Phase 1 single-wall workflow:

- `code`: starter framing defaults used by the current solver.
- `walls`: deterministic list of wall objects.
- `openings`: deterministic list of wall openings.

Wall and opening IDs are stable semantic identifiers. They must be non-empty and
contain only lowercase letters, digits, or hyphens. Examples:

- `wall-1`
- `opening-door-1`
- `opening-window-1`

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

- Walls are sorted by `id`.
- Openings within each wall are sorted by `id`.
- JSON is pretty-printed with a trailing newline.
- Generated framing is deterministic output and is not saved in v1 files.

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
9. Validate after edits.

Recommended validation:

```sh
cargo fmt --all -- --check
cargo test --workspace
```

If an agent only changes example project files, it should still run the workspace
tests because the fixture is checked against canonical serialization and solver
round-tripping.

## App Support

The desktop app opens with the demo wall model and a default project path of
`examples/projects/demo-wall.framer`.

Use:

- `New` to create a fresh single-wall project.
- `Open` and `Save` to load or persist the authored `.framer` file.
- the model tree and inspector to edit wall and opening intent.
- the catalog to add doors, windows, and garage doors.
- `Export` to write disposable sidecar artifacts next to the project path:
  `<project>.svg` for the current framing elevation and `<project>.csv` for the
  grouped BOM/cut list.

The SVG and CSV exports are regenerated outputs. Do not copy them back into the
canonical project document.
