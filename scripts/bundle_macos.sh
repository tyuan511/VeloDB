#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "Building VeloDB (release)..."
cargo build --release --bin velodb

APP_DIR="$ROOT_DIR/target/release/VeloDB.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$ROOT_DIR/target/release/velodb" "$APP_DIR/Contents/MacOS/velodb"
cp "$ROOT_DIR/macos/Info.plist" "$APP_DIR/Contents/Info.plist"
cp "$ROOT_DIR/assets/AppIcon.icns" "$APP_DIR/Contents/Resources/AppIcon.icns"

# Ad-hoc sign so macOS Gatekeeper / LaunchServices recognizes the bundle cleanly
codesign --force --deep --sign - "$APP_DIR" >/dev/null 2>&1 || true

echo "Created $APP_DIR"
