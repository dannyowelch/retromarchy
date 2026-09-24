#!/bin/bash
# UI Verification Script for Retromarchy PR #2
# Tests details pane modes, play tracking, collapsible pane, theme, and keyboard shortcuts

set -e

DISPLAY="${DISPLAY:-:99}"
SCREENSHOT_DIR="/opt/cursor/artifacts"
APP_LOG="/tmp/retro-verify.log"
CONFIG_DIR="/tmp/retro-config"
DB_FILE="$CONFIG_DIR/library.db"

echo "=== Retromarchy PR #2 Verification ==="
echo "Display: $DISPLAY"
echo "Screenshot dir: $SCREENSHOT_DIR"

# Setup
mkdir -p "$SCREENSHOT_DIR"
mkdir -p "$CONFIG_DIR"
rm -f "$APP_LOG"
rm -f "$DB_FILE"

# Copy test config
cp /tmp/retro-test/config.toml "$CONFIG_DIR/config.toml"

# Set config path
export XDG_CONFIG_HOME="$(dirname $CONFIG_DIR)"
export XDG_DATA_HOME="$(dirname $CONFIG_DIR)"

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
echo "Starting retromarchy with old schema (no play columns)..."
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
echo "1. Select SNES console (click sidebar)"
DISPLAY=$DISPLAY xdotool mousemove --window "$WINDOW" 100 100 click 1
wait_ui 2

# Run initial scan to populate database
echo "2. Scan console with 'r' key"
send_keys r
wait_ui 2

# (a) Console mode with stats
echo "3. Console mode showing stats"
screenshot "a-console-mode-stats"

# (b) Select a game
echo "4. Tab to grid and select a game"
send_keys Tab Right Right
screenshot "b-game-selected"

# (c) Escape back to console mode
echo "5. Press Escape to return to console mode"
send_keys Escape
screenshot "c-escape-console-mode"

# (d) Type 'd' in filter (should not collapse pane, just filter)
echo "6. Open filter with / and type 'd'"
send_keys slash
wait_ui 0.5
DISPLAY=$DISPLAY xdotool type "d"
wait_ui 1
screenshot "d-filter-d-pane-visible"

# Close filter
send_keys Escape

# (e) Collapse pane with 'd' key
echo "7. Press 'd' to collapse details pane"
send_keys d
wait_ui 0.8
screenshot "e-pane-collapsed"

# Restart to verify pane stays collapsed
echo "8. Restart app to verify collapsed state persists"
kill $APP_PID
wait_ui 2
DISPLAY=$DISPLAY cargo run > "$APP_LOG" 2>&1 &
APP_PID=$!
sleep 4
WINDOW=$(DISPLAY=$DISPLAY xdotool search --sync --onlyvisible --name "Retromarchy" 2>/dev/null | head -1)
DISPLAY=$DISPLAY xdotool mousemove --window "$WINDOW" 100 100 click 1
wait_ui 1.5
screenshot "e2-pane-collapsed-after-restart"

# Re-open pane
send_keys d
wait_ui 0.8

# (f) Toggle to LaunchBox theme
echo "9. Press 't' to toggle to LaunchBox theme"
send_keys t
wait_ui 1
screenshot "f-launchbox-theme"

# (g) Launch game and wait for play tracking
echo "10. Select game and launch with Enter"
send_keys Tab Right
wait_ui 0.5
send_keys Return
echo "  Waiting for stub emulator to complete (3 seconds)..."
sleep 4
wait_ui 1
screenshot "g-play-count-incremented"

echo ""
echo "=== Verification Complete ==="
echo "Screenshots saved to: $SCREENSHOT_DIR"
echo "App log: $APP_LOG"
echo ""
echo "Generated screenshots:"
ls -1 $SCREENSHOT_DIR/*.png | tail -8

# Keep app running briefly for inspection
echo ""
echo "App is running (PID $APP_PID)."
sleep 2
kill $APP_PID 2>/dev/null || true

echo ""
echo "Verification script completed successfully!"
