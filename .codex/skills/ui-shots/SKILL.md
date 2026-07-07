---
name: ui-shots
description: Render and inspect the off-screen Framer UI screenshot deck for static UI review, before/after comparison, and UI-bug verification. Use this first for Framer layout, styling, theming, labels, panel content, menus captured by the deck, palette states, view states, and visual regression checks; fall back to installed-app/live interaction checks only for drags, hover behavior, snapping feel, camera motion, theme persistence across launches, and native window compositing.
---

# ui-shots

Use the repo-owned off-screen screenshot deck before judging static Framer UI changes. It renders the real `FramerApp` through scripted states with no app install, window, or screen-capture permissions.

```bash
scripts/ui-shots.sh
UI_SHOTS_DIR=/tmp/framer-ui-shots scripts/ui-shots.sh
```

The default output is `target/ui-shots/*.png`. Inspect the relevant PNGs directly, using `view_image` when available. The deck currently covers workflow tabs, the Plan workspace, wall/opening/corner selections, elevation/roof/3D/render views, the command palette, the Project menu, and dark-theme shots.

Deck definition lives in `crates/framer-app/src/app/ui_shots_tests.rs`, in the ignored test `capture_ui_shot_deck`. If a UI change is not represented by the deck, add or adjust a state there rather than relying on memory.

## Use The Right Check

- Use `scripts/ui-shots.sh` for layout, styling, theming, labels, panel content, static menu/palette states, and before/after visual comparisons.
- Use an installed-app/live check for interaction-only behavior: drags, hover states, snapping, camera feel, theme persistence across launches, and native popup/window compositing.
- When a live check is needed, install the current build with `scripts/install-app.sh` or `scripts/install-app.sh --debug`, then use whatever native app-control or manual verification path is available in the current environment. Do not rely on `cargo run` for screenshot tooling on macOS because installed-app capture may not see that binary.

## Gotchas

- The script needs a `wgpu` adapter. On macOS that is normally Metal; CI-like Linux environments may need lavapipe.
- Plain `cargo test --workspace` does not run the deck because the capture test is ignored; the script passes the right ignored-test flags.
- The default shots are 1360 x 860 at 1x. The render view gets only a few warm-up frames, so use it for layout/progress legibility, not converged rendering quality.
- Some native-window issues may not reproduce in the deck. For example, popup translucency can involve live window compositing even when the off-screen deck renders the same popup opaque.
