#!/usr/bin/env bash
#
# Build a macOS .app bundle (and optionally a DMG).
#
# Version resolution order:
#   1. $RGITUI_VERSION environment variable
#   2. First positional argument
#   3. version field parsed from crates/rgitui/Cargo.toml
#
# Set $RGITUI_MAC_TARGET (e.g. aarch64-apple-darwin, x86_64-apple-darwin) to
# build for a specific target triple.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="${RGITUI_VERSION:-${1:-}}"
if [ -z "$VERSION" ]; then
    VERSION="$(awk -F '"' '/^version *=/ {print $2; exit}' crates/rgitui/Cargo.toml)"
fi
if [ -z "$VERSION" ]; then
    echo "error: could not determine version from crates/rgitui/Cargo.toml" >&2
    exit 1
fi

APP_NAME="rgitui"
APP_DIR="target/release/${APP_NAME}.app"
TARGET_TRIPLE="${RGITUI_MAC_TARGET:-}"

echo "Building rgitui $VERSION release binary..."
if [ -n "$TARGET_TRIPLE" ]; then
    cargo build --release --package rgitui --target "$TARGET_TRIPLE"
    BIN_PATH="target/$TARGET_TRIPLE/release/rgitui"
else
    cargo build --release --package rgitui
    BIN_PATH="target/release/rgitui"
fi

echo "Creating macOS .app bundle..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy binary (supports cross-compiled targets via $RGITUI_MAC_TARGET)
cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/rgitui"

# Create Info.plist
cat > "$APP_DIR/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>rgitui</string>
    <key>CFBundleDisplayName</key>
    <string>rgitui</string>
    <key>CFBundleIdentifier</key>
    <string>com.rgitui.app</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleExecutable</key>
    <string>rgitui</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.15</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

# Copy icon if available
if [ -f "assets/icons/app-icon.icns" ]; then
    cp assets/icons/app-icon.icns "$APP_DIR/Contents/Resources/rgitui.icns"
fi

echo "Done! App bundle created at $APP_DIR"

if [ "${RGITUI_MAC_CREATE_DMG:-0}" = "1" ]; then
    case "$TARGET_TRIPLE" in
        aarch64-apple-darwin) ARCH_SUFFIX="aarch64" ;;
        x86_64-apple-darwin)  ARCH_SUFFIX="x86_64" ;;
        "")                   ARCH_SUFFIX="$(uname -m)" ;;
        *)                    ARCH_SUFFIX="$TARGET_TRIPLE" ;;
    esac
    DMG_PATH="target/rgitui-${VERSION}-${ARCH_SUFFIX}-macos.dmg"
    echo "Creating DMG at $DMG_PATH..."
    rm -f "$DMG_PATH"
    hdiutil create -volname rgitui -srcfolder "$APP_DIR" -ov -format UDZO "$DMG_PATH"
    echo "Done! DMG created at $DMG_PATH"
else
    echo "To create a DMG: RGITUI_MAC_CREATE_DMG=1 $0"
fi
