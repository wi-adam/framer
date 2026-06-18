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
- `framer-render`: a UI-agnostic, physically based path tracer. It extracts a
  renderable scene from the building model (auto-derived materials: cladding,
  drywall, glass, doors, ground, plus a procedural sky and sun), builds a BVH,
  and path-traces it with diffuse / metal / dielectric-glass materials, soft sun
  shadows, and ACES tone mapping. Includes a headless PNG render CLI.
- `framer-app`: a native Rust desktop shell using `eframe`/`egui` with a model
  tree, object catalog, inspector, diagnostics, BOM, whole-shell plan view,
  selected-wall elevation view, a WGPU-backed 3D workspace with depth-tested
  wall and framing member solids, and a path-traced **Render** view mode. The
  Render view runs a real-time WGSL compute path tracer that mirrors
  `framer-render`'s math (validated against the CPU reference by a headless
  GPU↔CPU parity test), falling back to the CPU renderer when the GPU lacks
  compute support.

The completed Phase 1 workflow still frames one straight wall with doors,
windows, or garage-door-style openings. The current beyond-Phase-1 alpha also
loads, edits, regenerates, exports, saves, and reopens a connected multi-wall
shell. It adds interactive floor-plan authoring: draw and delete interior and
exterior walls with ortho/grid/endpoint/mid-span snapping and automatic
corner/tee joins, and place rooms that derive their area and perimeter from the
enclosing wall loop. Interior partitions are framed with partition end studs and
backing. The code profile remains a starter data shape, not yet a complete
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
4. Use the Design-mode `Wall` tool (W) to draw walls in the plan view and the
   `Room` tool (R) to place a room inside an enclosed area.
5. Use the Design-mode `Dimension` tool in the wall view to click wall/opening
   anchors and create driving or reference dimensions.
6. Inspect regenerated whole-project framing, diagnostics, rule provenance, and
   the grouped BOM.
7. Use `Save` to persist authored intent only.
8. Use `Export` to write sidecar SVG and CSV artifacts next to the project path.
9. Switch to the **Render** view (toolbar) for a path-traced architectural
   rendering of the design — real raytraced lighting, glass, and soft shadows.
   It runs on the GPU (WGSL compute) in real time, drag to orbit and scroll to
   zoom; the image refines progressively while the camera is still.

## Render (headless)

Path-trace a project straight to a PNG without opening the app:

```sh
cargo run -p framer-render --features cli --release --bin render -- \
    examples/projects/demo-shell.framer out.png --width 1280 --height 720 --spp 256
```

Optional flags: `--yaw DEG --pitch DEG --zoom Z --exposure E --seed S`. The
renderer is deterministic (a pure function of the seed) and parallelized across
cores. The in-app Render view's WGSL compute path tracer mirrors this exact math
— same PCG RNG, BVH traversal, BSDFs, and ACES — verified by a headless GPU↔CPU
parity test (`crates/framer-app/tests/gpu_parity.rs`).

## Test

```sh
cargo fmt --all -- --check
cargo test --workspace
```

## Architecture

See [docs/vision.md](docs/vision.md),
[docs/architecture.md](docs/architecture.md),
[docs/project-files.md](docs/project-files.md),
[docs/plans/2026-05-21-phase-1.md](docs/plans/2026-05-21-phase-1.md), and
[docs/plans/2026-06-15-render-view-mode-design.md](docs/plans/2026-06-15-render-view-mode-design.md).
