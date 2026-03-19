#!/usr/bin/env bash
set -euo pipefail

echo "Building rgitui release binary..."
cargo build --release --package rgitui

VERSION="0.1.0"
APP_NAME="rgitui"
APP_DIR="target/release/${APP_NAME}.app"

echo "Creating macOS .app bundle..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy binary
cp target/release/rgitui "$APP_DIR/Contents/MacOS/"

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
echo "To create a DMG: hdiutil create -volname rgitui -srcfolder $APP_DIR -ov -format UDZO target/rgitui-${VERSION}.dmg"
