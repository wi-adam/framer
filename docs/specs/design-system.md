# Design System

> **Feature spec** — durable intent, requirements, and locked decisions for this feature.
> Kept current as the feature evolves; point-in-time task breakdowns live in
> [`docs/plans/`](../plans/). See [spec-driven-development.md](../spec-driven-development.md).
>
> **Status:** Implemented (evolving) · **Linked goal:** G-011 (CAD Workspace UX)

## Goal

Framer's UI grew organically and has no logically organized design system: `theme.rs`
is ~30 loose, dark-only color functions, spacing/typography/sizing are inline magic
numbers, and components are ad-hoc helpers. Bring the app in line with the approved
mockup (a light "studio" CAD interface with a dark app header, icon-driven toolbars,
a sectioned inspector, a workspace tab bar, status-bar toggles, an on-canvas nav cube
and axis gizmo) by building a real design system and reskinning the app onto it.

## Decisions

- **Theme:** Support **both** a light palette (matching the mockup) and a refreshed
  dark palette, switchable at runtime. Tokens are theme-agnostic; palettes are data.
- **Scope:** **Full functional parity** with the mockup's interactive chrome — Grid /
  Ortho / Snap become real backed state that drives drafting; Snap / Level / 2D-3D
  dropdowns work; the view cube (already present in 3D) becomes a shared on-canvas nav
  widget; the floating canvas toolbar wires to existing ops.
- **Icons:** **Bundle an icon font** (Lucide, MIT, thin-line) registered into egui's
  font stack, referenced through a typed `Icon` enum.
- **Title bar:** Keep native window decorations (real macOS traffic lights) and render
  the dark app header directly beneath — visually equivalent to the mock without going
  frameless and reimplementing window drag/resize.

## Architecture

Replace flat `theme.rs` with a `design/` module in `framer-app`:

- `design/tokens.rs` — `Theme`, a cheap `Copy` struct of semantic tokens + metrics.
- `design/palette.rs` — `Theme::studio_light()` and `Theme::studio_dark()`.
- `design/icons.rs` — bundled font registration + `Icon` enum (`Icon::Save.glyph()`).
- `design/widgets.rs` — reusable component builders.
- `design/mod.rs` — re-exports + the active-theme accessor.

### Token model (`Theme`)

Grouped by role so call sites read intent, not hex:

- **Surfaces:** `title_bar`, `toolbar`, `panel`, `panel_header`, `canvas`, `field`,
  `control`, `control_hover`, `overlay`.
- **Text:** `text`, `text_secondary`, `text_muted`, `text_on_accent`.
- **Accent + semantic:** `accent`, `accent_soft`, `success`, `warning`, `danger`.
- **Lines:** `divider`, `divider_soft`, `border` (+ `Stroke` helpers).
- **Drawing/canvas:** `paper`, `grid_minor`, `grid_major`, `ruler`, `framing`,
  `framing_dark`, `selection`, `dimension`.
- **Metrics:** spacing scale (`xs=2, sm=4, md=8, lg=12, xl=16`), `radius` (sm/md),
  type scale (`title/heading/body/label/mono`), control sizes (`tool_btn`, `icon_btn`,
  `row_h`), icon sizes.

### Theme storage & switching

The active `Theme` lives in egui memory (`ctx.data_mut`), fetched anywhere via
`design::theme(ui)`. Existing free functions migrate with a one-line swap
(`theme::panel_bg()` → `t.panel`) instead of threading `&Theme` through every signature.
Switching theme calls `design::configure_style(ctx, &theme)` once to rebuild egui's
`Style.visuals`, and persists the choice via egui storage. Toggle lives in the app
header.

## Component library (`design/widgets.rs`)

Each reads `theme(ui)` so it restyles for free:

- `tool_button(Icon, label, active, enabled)` — icon-over-label toolbar button.
- `icon_button(Icon, tooltip)` — bare icon (tree footer, status bar, help).
- `toggle_switch(&mut bool, label)` — sliding Grid/Ortho/Snap switch.
- `segmented(&mut T, options)` — Design/Plan + view segments.
- `combo(Icon?, label, …)` — themed dropdown.
- `section(title, default_open, body)` — collapsible inspector group.
- `property_row(label, value_widget)` — left label / right-aligned value field.
- `swatch_field(...)` — wall-type visual stripe + dropdown.
- `chip(text, tone)`, `status_item(...)`, `tab_bar(...)`, `panel_header(...)`.
- Canvas overlays: `floating_toolbar(actions)`, `view_cube`, `axis_gizmo`,
  `scale_bar`, `nav_widget`.

## Screen-by-screen reskin

- **App header (new):** dark strip — wordmark · `project ▾` · "✓ Saved" · right:
  `IRC 2021 profile ▾` · `?` · theme toggle.
- **Toolbar:** text buttons → grouped `tool_button`s with icons
  (PROJECT/WORKSPACE/VIEW/BUILD/DIMENSION/TOOLS), active-blue states.
- **Workspace tab bar (new):** "Design Workspace" · `Shell` · `Level 1` · `+`.
- **Model Browser:** search + filter icon, restyled indented tree, Wall Joins /
  Catalog sections, bottom icon footer.
- **Inspector:** collapsible `section`s (Dimensions, Placement, Wall Type, Materials,
  Tags) with `property_row`s + wall-type swatch.
- **Canvas:** floating contextual toolbar over selection; axis gizmo (bottom-left),
  nav cube (bottom-right), scale bar, `2D/3D ▾`; light "paper" palette.
- **Status bar:** ✓ Ready · `Level ▾` · X/Y/Z live readout · `Snap ▾` · Grid/Ortho
  toggles · errors/warnings · view-layout icons · zoom · fullscreen.

## Phasing (each phase compiles & runs; existing tests stay green)

1. **Foundation** — `design/` module, tokens, light+dark palettes, icon font,
   `configure_style`, theme accessor + toggle. App reskins via the accessor swap.
2. **Chrome** — app header, tool_button toolbar, tab bar, sectioned inspector, status
   bar layout, tree footer.
3. **Wire real state** — add `grid`, `ortho`, `snap_step`, live cursor X/Y/Z, active
   level; make Grid/Ortho/Snap + Snap/Level/2D-3D dropdowns drive drafting and the
   existing viewport; promote the 3D view cube into a shared on-canvas nav widget.
4. **Polish** — floating canvas toolbar actions (select/move/duplicate/delete), scale
   bar, hover/disabled states, spacing pass against the mock.

## Verification

- `cargo fmt --all -- --check` and `cargo test --workspace` per phase. Existing
  dimension/IO/workspace-mode tests are the regression net and must stay green.
- New backing logic (snap stepping, theme persistence, cursor mapping) gets focused
  unit tests.
- Manual run + screenshot compare against the mockup.

## Constraints carried from the vision

- Keep `framer-core` and `framer-solver` free of desktop UI dependencies — all of this
  lands in `framer-app`.
- Do not change the intent/derived/presentation layering; this is presentation only.
