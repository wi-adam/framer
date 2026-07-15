# Specs

**Durable, feature-scoped statements of intent** — what each feature is, what it must do, and
why. Specs are living documents named by feature (no dates); they are kept current as the
feature evolves. Point-in-time task breakdowns live in [`../plans/`](../plans/).

New spec? Copy [`../templates/spec-template.md`](../templates/spec-template.md). Process:
[`../spec-driven-development.md`](../spec-driven-development.md).

| Spec | Linked goal | Status |
| --- | --- | --- |
| [Construction Systems & Material Library](construction-systems.md) | G-008 / G-001 | Implemented |
| [Libraries (Reusable, Distributable Content)](libraries.md) | G-013 | Phase 5 implemented |
| [Walls & Rooms (Floor-Plan Authoring)](walls-and-rooms.md) | G-007 | Implemented |
| [Wall Editing & Snapping](wall-editing-and-snapping.md) | G-003 | Implemented |
| [Wall Corner Laps](wall-corner-laps.md) | G-007 | Implemented |
| [Physical Geometry Overlap Audit](geometry-overlap-audit.md) | G-002 / G-014 | Implemented |
| [2D View Camera (Pan / Zoom)](2d-view-camera.md) | G-003 | Implemented |
| [View Layers (Wall Display Modes & Visibility)](view-layers.md) | G-003 | Implemented |
| [Render View Mode](render-view.md) | — | Implemented |
| [App Configuration](app-configuration.md) | — | Implemented |
| [Undo / Redo Infrastructure](undo-redo.md) | — | Implemented |
| [Design System](design-system.md) | G-011 | Implemented (evolving) |
| [Command Surfaces](command-surfaces.md) | G-011 | Implemented |
| [Component Visibility, Multi-Selection, and Isolation](component-visibility-and-isolation.md) | G-003 / G-011 | Implemented |
| [Tiled Viewport Workspaces](viewport-layouts.md) | G-003 / G-011 | Implemented |
| [Build & CI](build-and-ci.md) | — (infrastructure) | Implemented |

Related durable specs that live elsewhere because they are heavily cross-linked:

- [`../project-files.md`](../project-files.md) — the `.framer` file-format spec + agent editing
  contract.
- [`../vision.md`](../vision.md) — the product source of truth and goal backlog (G-001…G-012).
- [`../architecture.md`](../architecture.md) / [`../code-map.md`](../code-map.md) — system shape
  and concrete code navigation.
