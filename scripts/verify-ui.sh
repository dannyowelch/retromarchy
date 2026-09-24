#!/bin/bash
# UI Verification Script for Retromarchy PR #2
# Tests details pane modes, play tracking, collapsible pane, theme, and keyboard shortcuts

set -e

DISPLAY="${DISPLAY:-:99}"
SCREENSHOT_DIR="/opt/cursor/artifacts"
APP_LOG="/tmp/retro-verify.log"

echo "=== Retromarchy PR #2 Verification ==="
echo "Display: $DISPLAY"
echo "Screenshot dir: $SCREENSHOT_DIR"

# Setup
mkdir -p "$SCREENSHOT_DIR"
rm -f "$APP_LOG"

# Set XDG paths and copy config
export XDG_CONFIG_HOME="/tmp"
export XDG_DATA_HOME="/tmp"
export RETROMARCHY_DEBUG=1
mkdir -p /tmp/retromarchy
cp /tmp/retro-test/config.toml /tmp/retromarchy/config.toml
rm -f /tmp/retromarchy/library.db

echo "Config path: /tmp/retromarchy/config.toml"
echo "Database path: /tmp/retromarchy/library.db"
echo "Initial config:"
grep details_visible /tmp/retromarchy/config.toml

# Helper to wait for UI to settle
wait_ui() {
    sleep "${1:-1.5}"
}

# Ensure window manager is running
if ! pgrep -x xfwm4 > /dev/null; then
    echo "Starting xfwm4 window manager..."
    DISPLAY=$DISPLAY xfwm4 --daemon 2>&1 &
    sleep 2
fi

# Kill any existing instances
pkill -9 retromarchy || true
sleep 1

# Start the app
echo "Starting retromarchy..."
cd /workspace
DISPLAY=$DISPLAY cargo run > "$APP_LOG" 2>&1 &
APP_PID=$!
sleep 4

# Wait for window to appear
echo "Waiting for window..."
WINDOW=$(DISPLAY=$DISPLAY xdotool search --sync --onlyvisible --name "Retromarchy" 2>/dev/null | head -1)
if [ -z "$WINDOW" ]; then
    echo "ERROR: Window not found"
    cat "$APP_LOG"
    exit 1
fi
echo "Found window: $WINDOW"

# Helper function to send keys with focus
send_keys() {
    DISPLAY=$DISPLAY xdotool mousemove --window "$WINDOW" 640 360 click 1
    sleep 0.2
    DISPLAY=$DISPLAY xdotool key $@
    wait_ui 0.8
}

# Helper function to take screenshot
screenshot() {
    local name="$1"
    echo "  Screenshot: $name"
    DISPLAY=$DISPLAY scrot "$SCREENSHOT_DIR/${name}.png"
}

echo ""
echo "=== Test Sequence ==="

# 1. Click sidebar to select console and trigger scan
echo "1. Select SNES console and scan"
DISPLAY=$DISPLAY xdotool mousemove --window "$WINDOW" 55 52 click 1
wait_ui 1.5
send_keys r
wait_ui 2

# (a) Console mode with stats - no game selected
echo "2. Capture console mode with stats"
screenshot "a-console-mode-stats"

# (b) Select a game
echo "3. Tab to grid and select a game"
send_keys Tab Right Right
wait_ui 0.5
screenshot "b-game-selected"

# (c) Escape back to console mode
echo "4. Press Escape to return to console mode"
send_keys Escape
wait_ui 0.5
screenshot "c-escape-console-mode"

# (d) Test filter without collapsing pane
# First make sure a game is selected so pane shows game details
echo "5. Select game, then filter with 'd'"
send_keys Tab Right
wait_ui 0.5
# Open search and type
send_keys slash
wait_ui 0.3
DISPLAY=$DISPLAY xdotool type "d"
wait_ui 1
screenshot "d-filter-d-pane-visible"

# Close search
send_keys Escape Escape  # First Escape closes search, might need second to clear
wait_ui 0.8

# (e) Collapse the pane - make sure it's visible first and grid has focus
echo "6. Collapse details pane with 'd'"
# Click in the grid area to ensure focus is NOT on search
DISPLAY=$DISPLAY xdotool mousemove --window "$WINDOW" 300 100 click 1
wait_ui 0.3
# Select a game to ensure pane is visible
send_keys Tab Right
wait_ui 0.5
# Now collapse - press 'd' with grid focused
send_keys d
wait_ui 0.8
echo "Config after collapse:"
grep details_visible /tmp/retromarchy/config.toml
screenshot "e-pane-collapsed"

# (f) Restart and verify pane stays collapsed
echo "7. Restart to verify collapsed state persists"
kill $APP_PID
wait_ui 2
echo "Config before restart:"
grep details_visible /tmp/retromarchy/config.toml
DISPLAY=$DISPLAY cargo run > "$APP_LOG" 2>&1 &
APP_PID=$!
sleep 4
WINDOW=$(DISPLAY=$DISPLAY xdotool search --sync --onlyvisible --name "Retromarchy" 2>/dev/null | head -1)
DISPLAY=$DISPLAY xdotool mousemove --window "$WINDOW" 55 52 click 1
wait_ui 1.5
screenshot "e2-pane-collapsed-after-restart"
echo "Config after restart:"
grep details_visible /tmp/retromarchy/config.toml

# Re-open pane for next tests
send_keys d
wait_ui 0.8

# (g) Toggle to LaunchBox theme
echo "8. Toggle to LaunchBox theme"
send_keys t
wait_ui 1
screenshot "f-launchbox-theme"

# (h) Launch game and verify play tracking
echo "9. Select and launch game for play tracking"
send_keys Tab Right
wait_ui 0.5
# Expand pane to see stats
send_keys d
wait_ui 0.5
send_keys Return
echo "  Waiting for stub emulator (3 seconds)..."
sleep 5
wait_ui 1
screenshot "g-play-count-incremented"

echo ""
echo "=== Verification Complete ==="
echo "Screenshots saved to: $SCREENSHOT_DIR"
echo ""
echo "Generated screenshots:"
ls -1 $SCREENSHOT_DIR/*.png 2>/dev/null || echo "No screenshots found"

# Cleanup
kill $APP_PID 2>/dev/null || true

echo ""
echo "Check app log at: $APP_LOG"
echo "Verification script completed!"
