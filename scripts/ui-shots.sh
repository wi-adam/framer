#!/usr/bin/env bash
# Render the off-screen UI screenshot deck (no app install, no window, no
# screen-capture permissions). See crates/framer-app/src/app/ui_shots_tests.rs
# for the deck's states; PNGs land in target/ui-shots/ (or $UI_SHOTS_DIR).
#
# Needs a wgpu adapter: Metal locally, lavapipe on CI (same as gpu_parity).
set -euo pipefail
cd "$(dirname "$0")/.."

# Absolute path: cargo runs the test binary with cwd = crates/framer-app, so a
# relative default would land the PNGs in crates/framer-app/target/ instead of
# the workspace-root target/ that the docs point at.
: "${UI_SHOTS_DIR:=$PWD/target/ui-shots}"
export UI_SHOTS_DIR

cargo test -p framer-app ui_shots -- --ignored --nocapture "$@"

echo
echo "ui-shots: deck at $UI_SHOTS_DIR/"
