---
name: install-app
description: Build framer-app and install it as ~/Applications/Framer.app (ad-hoc signed) so GUI tools like computer-use — which only render *installed* apps, never a `cargo run`/`target/debug` binary — can screenshot and drive the real build. Use when verifying the app visually, taking screenshots, or driving it with computer-use.
---

# Installing Framer for GUI / computer-use verification

`cargo run -p framer-app` launches a bare binary. macOS screenshot filtering
(including the computer-use MCP) only renders **installed** apps, so that window
is invisible to screenshots — you'll see a stale `~/Applications/Framer.app`
instead, or nothing. To verify the *real* build in the GUI, install it:

```bash
scripts/install-app.sh              # release build, install + launch
scripts/install-app.sh --debug      # faster build for a quick check
scripts/install-app.sh --no-launch  # install without opening
scripts/install-app.sh --restore    # put back the pre-dev binary (from .orig.bak)
```

Then in computer-use: `request_access(["Framer"])` (bundle id `windustries.framer.app`),
`open_application("windustries.framer.app")`, and `screenshot`.

## Gotchas (learned the hard way)

- **Two instances confusion.** If a `cargo run` build *and* the installed bundle
  are both running, they share bundle id `windustries.framer.app` and the compositor
  shows whichever is frontmost — usually the installed one. Kill stray
  `target/debug/framer-app` processes (`pkill -f target/debug/framer-app`) and
  drive only the installed bundle.
- **`open_application` launches the installed copy**, not your `target/debug`
  binary — another reason to install rather than `cargo run` for GUI checks.
- **What computer-use can and can't drive:**
  - ✅ orbit (left-drag), and scroll controls *including* modifiers — e.g.
    `computer_batch` scroll with `text:"cmd"` for Cmd+scroll telephoto zoom.
  - ❌ a modifier held *during a drag* (Shift+left-drag) and ❌ middle-button
    drag — the drag tools don't carry a modifier and there's no middle-drag.
    So **Shift/middle-drag panning must be verified by hand**, not via
    computer-use.

## Reaching the Render view

Toolbar → VIEW group → **Render** (the path-traced view). Camera controls there:
left-drag orbit, scroll dolly, Cmd+scroll/pinch telephoto, middle- or
Shift+left-drag pan.
