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
    <key>CFBundleIconFile</key>
    <string>rgitui</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

# Generate .icns from PNG sources using iconutil (macOS-only)
ICONSET_DIR="target/rgitui.iconset"
rm -rf "$ICONSET_DIR"
mkdir -p "$ICONSET_DIR"
if [ -f "assets/icons/app-icon-16.png" ]; then
    sips -z 16 16     "assets/icons/app-icon-16.png"  --out "$ICONSET_DIR/icon_16x16.png"      > /dev/null 2>&1
    sips -z 32 32     "assets/icons/app-icon-32.png"  --out "$ICONSET_DIR/icon_16x16@2x.png"   > /dev/null 2>&1
    sips -z 32 32     "assets/icons/app-icon-32.png"  --out "$ICONSET_DIR/icon_32x32.png"      > /dev/null 2>&1
    sips -z 64 64     "assets/icons/app-icon-48.png"  --out "$ICONSET_DIR/icon_32x32@2x.png"   > /dev/null 2>&1
    sips -z 128 128   "assets/icons/app-icon-256.png" --out "$ICONSET_DIR/icon_128x128.png"    > /dev/null 2>&1
    sips -z 256 256   "assets/icons/app-icon-256.png" --out "$ICONSET_DIR/icon_128x128@2x.png" > /dev/null 2>&1
    sips -z 256 256   "assets/icons/app-icon-256.png" --out "$ICONSET_DIR/icon_256x256.png"    > /dev/null 2>&1
    sips -z 512 512   "assets/icons/app-icon-512.png" --out "$ICONSET_DIR/icon_256x256@2x.png" > /dev/null 2>&1
    sips -z 512 512   "assets/icons/app-icon-512.png" --out "$ICONSET_DIR/icon_512x512.png"    > /dev/null 2>&1
    sips -z 1024 1024 "assets/icons/app-icon-512.png" --out "$ICONSET_DIR/icon_512x512@2x.png" > /dev/null 2>&1
    iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/rgitui.icns"
    echo "Generated app icon (rgitui.icns)"
elif [ -f "assets/icons/app-icon.icns" ]; then
    cp assets/icons/app-icon.icns "$APP_DIR/Contents/Resources/rgitui.icns"
fi
rm -rf "$ICONSET_DIR"

# Ad-hoc code sign the app bundle so macOS doesn't report it as "damaged"
echo "Ad-hoc signing app bundle..."
codesign --force --deep --sign - "$APP_DIR"

echo "Done! App bundle created at $APP_DIR"

if [ "${RGITUI_MAC_CREATE_DMG:-0}" = "1" ]; then
    case "$TARGET_TRIPLE" in
        aarch64-apple-darwin) ARCH_SUFFIX="aarch64" ;;
        x86_64-apple-darwin)  ARCH_SUFFIX="x86_64" ;;
        "")                   ARCH_SUFFIX="$(uname -m)" ;;
        *)                    ARCH_SUFFIX="$TARGET_TRIPLE" ;;
    esac
    DMG_PATH="target/rgitui-${VERSION}-${ARCH_SUFFIX}-macos.dmg"
    DMG_STAGING="target/dmg-staging"
    echo "Creating DMG at $DMG_PATH..."
    rm -f "$DMG_PATH"
    rm -rf "$DMG_STAGING"
    mkdir -p "$DMG_STAGING"
    cp -R "$APP_DIR" "$DMG_STAGING/"
    ln -s /Applications "$DMG_STAGING/Applications"
    hdiutil create -volname rgitui -srcfolder "$DMG_STAGING" -ov -format UDZO "$DMG_PATH"
    rm -rf "$DMG_STAGING"
    echo "Done! DMG created at $DMG_PATH"
else
    echo "To create a DMG: RGITUI_MAC_CREATE_DMG=1 $0"
fi
