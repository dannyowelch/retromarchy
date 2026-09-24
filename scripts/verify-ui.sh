#!/bin/bash
# UI Verification Script for Retromarchy
# Tests keyboard navigation, filter, and launch functionality under Xvfb

set -e

DISPLAY="${DISPLAY:-:99}"
SCREENSHOT_DIR="${1:-/tmp/retro-verify}"
APP_LOG="/tmp/retro-verify.log"

echo "=== Retromarchy UI Verification ==="
echo "Display: $DISPLAY"
echo "Screenshot dir: $SCREENSHOT_DIR"

# Setup
mkdir -p "$SCREENSHOT_DIR"
rm -f "$APP_LOG"

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
sleep 3

# Wait for window to appear
echo "Waiting for window..."
WINDOW=$(DISPLAY=$DISPLAY xdotool search --sync --onlyvisible --name "Retromarchy" | head -1)
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
    sleep 0.5
}

# Helper function to take screenshot
screenshot() {
    local name="$1"
    echo "  Screenshot: $name"
    DISPLAY=$DISPLAY scrot "$SCREENSHOT_DIR/${name}.png"
}

echo ""
echo "=== Test Sequence ==="

# 1. Initial state
echo "1. Initial state"
screenshot "01-initial"

# 2. Click sidebar to select console and load games
echo "2. Select console (click sidebar)"
DISPLAY=$DISPLAY xdotool mousemove --window "$WINDOW" 100 100 click 1
sleep 1
screenshot "02-console-selected"

# 3. Tab to game grid
echo "3. Tab to game grid"
send_keys Tab
screenshot "03-tab-to-grid"

# 4. Navigate with hjkl
echo "4. Navigate with h (left)"
send_keys h h
screenshot "04-hjkl-left"

echo "5. Navigate with l (right)"
send_keys l l l
screenshot "05-hjkl-right"

echo "6. Navigate with k (up)"
send_keys k
screenshot "06-hjkl-up"

echo "7. Navigate with j (down)"
send_keys j
screenshot "07-hjkl-down"

# 8. Navigate with arrow keys
echo "8. Navigate with arrow keys"
send_keys Right Right
screenshot "08-arrow-right"

send_keys Up
screenshot "09-arrow-up"

# 9. Open filter with /
echo "10. Open filter with /"
send_keys slash
screenshot "10-filter-open"

# 10. Type in filter
echo "11. Type in filter"
DISPLAY=$DISPLAY xdotool mousemove --window "$WINDOW" 640 360 click 1
sleep 0.2
DISPLAY=$DISPLAY xdotool type "5"
sleep 0.5
screenshot "11-filter-text"

# 11. Close filter with Escape
echo "12. Close filter with Escape"
send_keys Escape
screenshot "12-filter-closed"

# 12. Press Tab to refocus grid and navigate
echo "13. Tab back to grid"
send_keys Tab Left Left
screenshot "13-positioned-for-launch"

# 13. Press Enter to launch (check logs for launch command)
echo "14. Press Enter to launch game"
rm -f /tmp/retro-launch-test.txt
send_keys Return
sleep 2
screenshot "14-after-launch"

# Check if launch was successful by looking for the output file
if [ -f /tmp/retro-launch-test.txt ]; then
    echo "  ✓ Launch succeeded!"
    echo "  Launch output: $(cat /tmp/retro-launch-test.txt)"
else
    echo "  ✗ WARNING: Launch failed - no output file created"
    echo "  Check app log for errors"
fi

# 14. Test rescan with 'r' key
echo "15. Rescan console with 'r'"
send_keys r
sleep 2
screenshot "15-after-rescan"

echo ""
echo "=== Verification Complete ==="
echo "Screenshots saved to: $SCREENSHOT_DIR"
echo "App log: $APP_LOG"
echo ""
echo "To view screenshots:"
echo "  ls -lh $SCREENSHOT_DIR/"
echo ""
echo "To check app output:"
echo "  cat $APP_LOG"

# Keep app running for manual inspection
echo ""
echo "App is still running (PID $APP_PID). Press Ctrl+C to stop, or kill with:"
echo "  kill $APP_PID"
wait $APP_PID
