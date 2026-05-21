# Framer

Framer is an open source parametric CAD tool for wood framed structures.

The project goal is to model structures semantically, place openings into walls,
floors, ceilings, and roofs, then generate framing plans and bills of materials
from configurable code profiles such as IRC 2021.

## Status

Framer is at the first end-to-end alpha-slice stage. The repository currently
contains:

- `framer-core`: UI-agnostic building, opening, code-profile, and unit models.
- `framer-solver`: a deterministic straight-wall framing generator, grouped BOM,
  rule provenance, diagnostics, SVG elevation export, and CSV BOM export.
- `framer-app`: a native Rust desktop shell using `eframe`/`egui` with a model
  tree, object catalog, inspector, diagnostics, BOM, and elevation/3D workspace
  views.

The current solver is intentionally narrow: it frames one straight wall with
doors, windows, or garage-door-style openings. The code profile is a starter
data shape, not yet a complete code-compliance engine.

Project files use a schema-versioned, text-first `.framer` JSON format for
authored design intent. See [docs/project-files.md](docs/project-files.md) for
the file format and agent editing contract.

## Run

```sh
cargo run -p framer-app
```

The app opens `examples/projects/demo-wall.framer` by default. The alpha workflow
is:

1. Use `New` or `Open` to create or load a schema-versioned `.framer` project.
2. Select authored objects in the model tree and edit dimensions in the
   inspector.
3. Add doors, windows, or garage doors from the catalog.
4. Inspect regenerated framing, diagnostics, rule provenance, and the grouped
   BOM.
5. Use `Save` to persist authored intent only.
6. Use `Export` to write sidecar SVG and CSV artifacts next to the project path.

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
