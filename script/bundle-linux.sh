#!/usr/bin/env bash
set -euo pipefail

echo "Building rgitui release binary..."
cargo build --release --package rgitui

VERSION="0.1.0"
ARCH="$(uname -m)"
APP_DIR="target/AppDir"

echo "Creating AppImage directory structure..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/usr/bin"
mkdir -p "$APP_DIR/usr/share/icons/hicolor/512x512/apps"
mkdir -p "$APP_DIR/usr/share/applications"
mkdir -p "$APP_DIR/usr/share/metainfo"

# Copy binary
cp target/release/rgitui "$APP_DIR/usr/bin/"
strip "$APP_DIR/usr/bin/rgitui" 2>/dev/null || true

# Copy desktop file
cp crates/rgitui/resources/linux/rgitui.desktop "$APP_DIR/usr/share/applications/"
cp crates/rgitui/resources/linux/rgitui.desktop "$APP_DIR/"

# Copy AppRun
cp crates/rgitui/resources/linux/AppRun "$APP_DIR/"
chmod +x "$APP_DIR/AppRun"

# Copy icon (use a placeholder if no PNG exists)
if [ -f "assets/icons/app-icon.png" ]; then
    cp assets/icons/app-icon.png "$APP_DIR/usr/share/icons/hicolor/512x512/apps/rgitui.png"
    cp assets/icons/app-icon.png "$APP_DIR/rgitui.png"
else
    echo "Warning: No app-icon.png found. AppImage will have no icon."
fi

# Copy appdata
cp crates/rgitui/resources/linux/rgitui.appdata.xml "$APP_DIR/usr/share/metainfo/"

# Create AppImage
if command -v appimagetool &> /dev/null; then
    echo "Creating AppImage..."
    ARCH="$ARCH" appimagetool "$APP_DIR" "target/rgitui-${VERSION}-${ARCH}.AppImage"
    echo "Done! AppImage created at target/rgitui-${VERSION}-${ARCH}.AppImage"
else
    echo "appimagetool not found. Creating tarball instead..."
    tar -czf "target/rgitui-${VERSION}-${ARCH}-linux.tar.gz" -C "$APP_DIR/usr/bin" rgitui
    echo "Done! Tarball created at target/rgitui-${VERSION}-${ARCH}-linux.tar.gz"
fi
