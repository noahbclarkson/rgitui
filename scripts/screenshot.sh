#!/usr/bin/env bash
set -euo pipefail

# Load .env if present
if [ -f "$(dirname "$0")/../.env" ]; then
    source "$(dirname "$0")/../.env"
fi

REPO_PATH="${RGITUI_TEST_REPO:-/home/noah/src/krypto}"
OUTPUT_DIR="$(dirname "$0")/../test_output"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
OUTPUT_FILE="$OUTPUT_DIR/screenshot_$TIMESTAMP.png"

mkdir -p "$OUTPUT_DIR"

echo "Starting rgitui with repo: $REPO_PATH"

# Build if needed
cargo build --manifest-path "$(dirname "$0")/../Cargo.toml" 2>&1

# Start the app in background
"$(dirname "$0")/../target/debug/rgitui" "$REPO_PATH" &
APP_PID=$!

echo "App PID: $APP_PID"
echo "Waiting for window to open..."
sleep 3

# Take screenshot using grim (Wayland screenshot tool)
if command -v grim &>/dev/null; then
    grim "$OUTPUT_FILE"
    echo "Screenshot saved to: $OUTPUT_FILE"
elif command -v scrot &>/dev/null; then
    scrot "$OUTPUT_FILE"
    echo "Screenshot saved to: $OUTPUT_FILE"
else
    echo "No screenshot tool found (need grim or scrot)"
fi

# Kill the app
kill $APP_PID 2>/dev/null || true

echo "Done. Screenshot: $OUTPUT_FILE"
