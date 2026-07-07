#!/usr/bin/env bash
# Render the off-screen UI screenshot deck (no app install, no window, no
# screen-capture permissions). See crates/framer-app/src/app/ui_shots_tests.rs
# for the deck's states; PNGs land in target/ui-shots/ (or $UI_SHOTS_DIR).
#
# Needs a wgpu adapter: Metal locally, lavapipe on CI (same as gpu_parity).
set -euo pipefail
cd "$(dirname "$0")/.."

: "${UI_SHOTS_DIR:=target/ui-shots}"
export UI_SHOTS_DIR

cargo test -p framer-app ui_shots -- --ignored --nocapture "$@"

echo
echo "ui-shots: deck at $UI_SHOTS_DIR/"
