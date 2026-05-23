# Framer

Framer is an open source parametric CAD tool for wood framed structures.

The project goal is to model structures semantically, place openings into walls,
floors, ceilings, and roofs, then generate framing plans and bills of materials
from configurable code profiles such as IRC 2021.

## Status

Framer has moved beyond the first straight-wall alpha slice into an initial
multi-system CAD alpha. The repository currently contains:

- `framer-core`: UI-agnostic levels, placed wall segments, wall joins/corners,
  openings, code-profile, and unit models.
- `framer-solver`: deterministic wall and whole-project framing generation,
  join corner posts, grouped BOM, rule provenance, diagnostics, SVG project
  export, and CSV BOM export.
- `framer-app`: a native Rust desktop shell using `eframe`/`egui` with a model
  tree, object catalog, inspector, diagnostics, BOM, whole-shell plan view,
  selected-wall elevation view, and a WGPU-backed 3D workspace with
  depth-tested wall and framing member solids.

The completed Phase 1 workflow still frames one straight wall with doors,
windows, or garage-door-style openings. The current beyond-Phase-1 alpha also
loads, edits, regenerates, exports, saves, and reopens a connected multi-wall
shell. The code profile remains a starter data shape, not yet a complete
code-compliance engine.

Project files use a schema-versioned, text-first `.framer` JSON format for
authored design intent. See [docs/project-files.md](docs/project-files.md) for
the file format and agent editing contract.

## Run

```sh
cargo run -p framer-app
```

The app opens `examples/projects/demo-shell.framer` by default. The alpha
workflow is:

1. Use `New` or `Open` to create or load a schema-versioned `.framer` project.
2. Select authored objects in the model tree and edit dimensions in the
   inspector, including wall placement, openings, levels, joins, and dimension
   constraints.
3. Add doors, windows, or garage doors from the catalog.
4. Use the Design-mode `Dimension` tool in the wall view to click wall/opening
   anchors and create driving or reference dimensions.
5. Inspect regenerated whole-project framing, diagnostics, rule provenance, and
   the grouped BOM.
6. Use `Save` to persist authored intent only.
7. Use `Export` to write sidecar SVG and CSV artifacts next to the project path.

## Test

```sh
cargo fmt --all -- --check
cargo test --workspace
```

## Architecture

See [docs/vision.md](docs/vision.md),
[docs/architecture.md](docs/architecture.md),
[docs/project-files.md](docs/project-files.md), and
[docs/plans/2026-05-21-phase-1.md](docs/plans/2026-05-21-phase-1.md).
