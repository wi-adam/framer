#!/usr/bin/env bash
#
# Build framer-app and install it as ~/Applications/Framer.app, ad-hoc signed.
#
# Why: macOS GUI tooling that filters screenshots by app — notably the
# computer-use MCP — only renders *installed* applications, never a bare
# `cargo run` / `target/debug` binary. To screenshot or drive the real build you
# must install it into a .app bundle. This script does that (and re-signs so
# Gatekeeper will launch it), giving GUI verification a one-command setup.
#
# Usage:
#   scripts/install-app.sh              # release build, install + launch
#   scripts/install-app.sh --debug      # faster debug build (e.g. quick checks)
#   scripts/install-app.sh --no-launch  # install but don't open the app
#
# Notes:
#   - Overwrites the binary in ~/Applications/Framer.app (bundle id industries.winstanley.framer).
#     The first run on this machine backs up any pre-existing binary to
#     Framer.orig.bak; restore with: scripts/install-app.sh --restore
#   - Ad-hoc signature only — fine for local dev, not for distribution.
#   - computer-use can drive orbit (left-drag) and scroll controls including
#     modifiers (Cmd+scroll), but cannot hold a modifier during a drag or do a
#     middle-drag, so Shift/middle-drag panning must be checked by hand.
set -euo pipefail

PROFILE="release"
LAUNCH=1
RESTORE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --debug) PROFILE="debug" ;;
    --release) PROFILE="release" ;;
    --no-launch) LAUNCH=0 ;;
    --restore) RESTORE=1 ;;
    -h | --help)
      sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown argument: $1 (try --help)" >&2
      exit 2
      ;;
  esac
  shift
done

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$HOME/Applications/Framer.app"
EXE="$APP/Contents/MacOS/Framer"
BACKUP="$EXE.orig.bak"
BUNDLE_ID="industries.winstanley.framer"

lsregister() {
  local lsr=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
  [ -x "$lsr" ] && "$lsr" -f "$APP" 2>/dev/null || true
}

if [ "$RESTORE" = "1" ]; then
  if [ -f "$BACKUP" ]; then
    pkill -f "$EXE" 2>/dev/null || true
    sleep 1
    cp -f "$BACKUP" "$EXE"
    codesign --force --sign - "$EXE" >/dev/null 2>&1 || true
    lsregister
    echo "restored original binary from $(basename "$BACKUP")"
  else
    echo "no backup found at $BACKUP — nothing to restore" >&2
    exit 1
  fi
  exit 0
fi

echo "==> building framer-app ($PROFILE)"
if [ "$PROFILE" = "release" ]; then
  cargo build --release -p framer-app
  BIN="$REPO/target/release/framer-app"
else
  cargo build -p framer-app
  BIN="$REPO/target/debug/framer-app"
fi

# Create a minimal bundle if one isn't present yet.
if [ ! -d "$APP" ]; then
  echo "==> creating $APP"
  mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
  cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>Framer</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>Framer</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
fi

# Stop any running instance so the executable isn't busy during the copy.
pkill -f "$EXE" 2>/dev/null || true
sleep 1

# Back up a pre-existing (non-dev) binary once, so --restore can recover it.
if [ -f "$EXE" ] && [ ! -f "$BACKUP" ]; then
  cp -p "$EXE" "$BACKUP"
  echo "==> backed up existing binary to $(basename "$BACKUP")"
fi

echo "==> installing $PROFILE binary"
cp -f "$BIN" "$EXE"

echo "==> ad-hoc signing"
codesign --force --sign - "$EXE"

lsregister

echo "==> installed: $APP ($(/usr/bin/stat -f '%z bytes' "$EXE"))"
if [ "$LAUNCH" = "1" ]; then
  open "$APP"
  echo "==> launched — grant access to \"Framer\" in computer-use, then screenshot"
fi
