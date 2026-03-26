#!/usr/bin/env bash
# Headless smoke test for rgitui GPUI app.
# Uses Xvfb (virtual framebuffer) to run the GUI app without a physical display.
# This verifies the app starts, opens a window, and doesn't crash on startup.
#
# Requirements (already installed on this VPS):
#   - xvfb-run   (virtual framebuffer X server wrapper)
#   - xdpyinfo   (optional, for display verification)
#
# Usage:
#   ./scripts/headless-smoke-test.sh
#
# Exit codes:
#   0 = app started and ran for 5s without crash
#   1 = app crashed or failed to start
#   2 = missing dependencies

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
TARGET_DIR="$REPO_DIR/target/debug"

# Check dependencies
check_deps() {
    local missing=()
    for cmd in xvfb-run; do
        if ! command -v "$cmd" &>/dev/null; then
            missing+=("$cmd")
        fi
    done
    if ((${#missing[@]} > 0)); then
        echo "ERROR: missing dependencies: ${missing[*]}" >&2
        echo "Install with: sudo apt-get install -y xvfb" >&2
        exit 2
    fi
}

# Create a temporary git repository for the test
setup_test_repo() {
    local tmp_dir
    tmp_dir=$(mktemp -t rgitui-smoke-test-XXXXXX -d)
    local repo_path="$tmp_dir/test-repo"

    mkdir -p "$repo_path"
    git init -q "$repo_path"
    git -C "$repo_path" config user.email "smoke-test@rgitui.dev"
    git -C "$repo_path" config user.name "rgitui Smoke Test"

    echo "# Test Repository" > "$repo_path/README.md"
    git -C "$repo_path" add README.md
    git -C "$repo_path" commit -q -m "Initial commit"

    echo "$repo_path"
}

# Main test
main() {
    check_deps

    echo "=== rgitui Headless Smoke Test ==="
    echo "Build status:"
    if [[ -f "$TARGET_DIR/rgitui" ]]; then
        echo "  binary exists: $TARGET_DIR/rgitui"
    else
        echo "  binary not found — building first..."
        cargo build --manifest-path "$REPO_DIR/Cargo.toml" 2>&1 | tail -3
    fi

    local test_repo
    test_repo=$(setup_test_repo)
    echo "Test repo: $test_repo"

    local exit_code=0
    local app_pid

    # Launch with xvfb-run: Xvfb creates virtual display,
    # -a = auto-select free server number
    # -s "-screen 0 WxHxD" = screen geometry and depth
    # gpui_platform now compiles with both x11 and wayland features, so GPUI
    # can detect X11 under xvfb and initialize properly.
    echo "Starting rgitui on virtual display ..."
    xvfb-run -a --server-args="-screen 0 1280x720x24" \
        "$TARGET_DIR/rgitui" "$test_repo" &
    app_pid=$!
    app_pid=$!

    echo "App PID: $app_pid"
    echo "Waiting 5s for startup..."
    sleep 5

    if kill -0 "$app_pid" 2>/dev/null; then
        echo "✓ App is running (PID $app_pid)"
        echo "Sending SIGTERM for clean shutdown..."
        kill -TERM "$app_pid" 2>/dev/null || true
        sleep 1
    else
        echo "✗ App exited unexpectedly (crashed?)" >&2
        exit_code=1
    fi

    # Clean up test repo
    rm -rf "$(dirname "$test_repo")"
    echo "Test repo cleaned up."

    if ((exit_code == 0)); then
        echo ""
        echo "=== PASS: rgitui started and ran for 5s without crash ==="
    else
        echo ""
        echo "=== FAIL ===" >&2
    fi

    exit $exit_code
}

main "$@"
