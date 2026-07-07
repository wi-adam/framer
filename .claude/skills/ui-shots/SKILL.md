---
name: ui-shots
description: Render an off-screen screenshot deck of the real Framer UI (every workflow tab, view, selection state, menus, palette, both themes) in ~15s with no app install, window, or screen-capture permissions. Use this FIRST for any static UI review, before/after comparison, or UI-bug verification; fall back to install-app + computer-use only for interactive behavior (drags, hover, snapping feel).
---

# ui-shots — off-screen UI screenshot deck

One command renders the full `FramerApp` UI through a scripted deck of states
and writes PNGs — no install, no window, no permission prompts, debug build:

```bash
scripts/ui-shots.sh          # → target/ui-shots/NN-<state>.png (~15s warm)
UI_SHOTS_DIR=/tmp/shots scripts/ui-shots.sh   # custom output dir
```

Then just Read the PNGs (e.g. `target/ui-shots/01-frame-shell.png`). The deck
covers: each workflow tab, the Plan workspace, wall/opening/corner selections,
elevation/roof/3D/render views, the command palette, the Project menu, and two
dark-palette shots. Deck definition:
`crates/framer-app/src/app/ui_shots_tests.rs` (`#[ignore]`d test
`capture_ui_shot_deck`, driven by `egui_kittest` + wgpu) — add states there
when reviewing something the deck doesn't cover.

## When to use what

- **ui-shots**: layout, styling, theming, labels, panel content, regressions —
  anything visible in a static frame. Also the fastest repro for rendering
  bugs: the deck faithfully reproduces app-side theming bugs (e.g. the
  half-dark theme state renders identically to the live app).
- **install-app + computer-use**: only for interactive feel — drags, hover
  states, snapping, camera motion, native window compositing. Note the live
  app's popup translucency involves window compositing and does NOT reproduce
  in ui-shots (popups render opaque here).

## Gotchas

- Needs a wgpu adapter (Metal locally; CI would use lavapipe like
  `gpu_parity`). No adapter → the test panics with a clear message.
- The test is `#[ignore]`d so plain `cargo test --workspace` never pays the
  GPU cost; the script passes `--ignored` for you.
- Shots are 1360×860 @1x (the spec's default desktop viewport). The render
  view gets a few warm-up frames only — expect a noisy first-pass image, not a
  converged render.
