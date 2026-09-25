#!/bin/bash
# Proves scraper settings, grid art, details slots, and a fixture scrape status.
set -euo pipefail

DISPLAY_NUM="${DISPLAY_NUM:-:97}"
export DISPLAY="$DISPLAY_NUM"
export GDK_BACKEND=x11
export GDK_CORE_DEVICE_EVENTS=1
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d /tmp/retro-art.XXXXXX)"
SHOTS="/opt/cursor/artifacts"
mkdir -p "$SHOTS"
rm -f "$SHOTS"/artwork-*.png

export HOME="$WORK/home"
export XDG_CONFIG_HOME="$WORK/config"
export XDG_DATA_HOME="$WORK/data"
export XDG_CACHE_HOME="$WORK/cache"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"

FIXTURES="$WORK/fixtures"
mkdir -p "$FIXTURES"
python3 - "$FIXTURES" << 'PY'
import struct, sys, zlib
from pathlib import Path

def png(path, w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    blob = b"\x89PNG\r\n\x1a\n"
    blob += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    blob += chunk(b"IDAT", zlib.compress(raw))
    blob += chunk(b"IEND", b"")
    Path(path).write_bytes(blob)

out = Path(sys.argv[1])
png(out / "box_art.png", 180, 180, (220, 40, 40))
png(out / "title_screen.png", 240, 120, (30, 180, 70))
png(out / "screenshot.png", 240, 120, (40, 90, 210))
PY
export RETROMARCHY_SCRAPER_FIXTURES="$FIXTURES"

eval "$(dbus-launch --sh-syntax)"

cleanup() {
  pkill -f "target/debug/retromarchy" 2>/dev/null || true
  if [ -n "${WM_PID:-}" ]; then kill "$WM_PID" 2>/dev/null || true; fi
  if [ -n "${XVFB_PID:-}" ]; then kill "$XVFB_PID" 2>/dev/null || true; fi
  if [ -n "${DBUS_SESSION_BUS_PID:-}" ]; then kill "$DBUS_SESSION_BUS_PID" 2>/dev/null || true; fi
}
trap cleanup EXIT

Xvfb "$DISPLAY" -screen 0 1280x800x24 >/tmp/xvfb-art.log 2>&1 &
XVFB_PID=$!
sleep 0.5
xfwm4 --daemon >/tmp/xfwm-art.log 2>&1 &
WM_PID=$!
sleep 0.5

ROMS="$HOME/ROMs/snes"
mkdir -p "$ROMS"
echo "dummy rom" > "$ROMS/Chrono Trigger.sfc"
ROM_BYTES=$(wc -c < "$ROMS/Chrono Trigger.sfc")

CFG="$XDG_CONFIG_HOME/retromarchy"
mkdir -p "$CFG"
cat > "$CFG/config.toml" << EOF
theme = "system"
details_visible = true

[[consoles]]
id = "snes"
name = "Super Nintendo"
rom_dirs = ["$ROMS"]
extensions = ["sfc"]
grid_art = "box_art"

[consoles.media]
box_art = true
screenshot = true
manual = false
video = false

[scraper]
box_art = true
title_screen = true
screenshot = true

[[scraper.providers]]
id = "screenscraper"
enabled = true

[[scraper.providers]]
id = "thegamesdb"
enabled = true

[scraper.credentials]
screenscraper_user = ""
screenscraper_password = ""
screenscraper_dev_id = ""
screenscraper_dev_password = ""
thegamesdb_api_key = ""
EOF

cd "$ROOT"
"$ROOT/target/debug/retromarchy" >/tmp/retro-art.log 2>&1 &
sleep 2

window_id() {
  xdotool search --onlyvisible --name "$1" 2>/dev/null | head -1 || true
}

shot() {
  local name="$1" wid="$2"
  sleep 0.4
  import -window "$wid" "$SHOTS/$name"
  echo "saved $name $(md5sum "$SHOTS/$name" | awk '{print $1}')"
}

MAIN="$(window_id Retromarchy)"
xdotool windowsize "$MAIN" 1200 720
xdotool windowmove "$MAIN" 40 40
sleep 0.3
xdotool key --window "$MAIN" r
sleep 0.8
xdotool key --window "$MAIN" Right
sleep 0.3

xdotool key --window "$MAIN" ctrl+g
sleep 0.8
SETTINGS="$(window_id "Scraper settings")"
echo "settings window: ${SETTINGS:-missing}"
xdotool windowsize "$SETTINGS" 560 640 || true
shot "artwork-scraper-settings.png" "$SETTINGS"

xdotool key --window "$SETTINGS" Escape
sleep 0.4
MAIN="$(window_id Retromarchy)"
xdotool key --window "$MAIN" s
sleep 1.2
shot "artwork-scrape-status.png" "$MAIN"

xdotool key --window "$MAIN" 2
sleep 0.6
shot "artwork-grid-title-screen.png" "$MAIN"
convert "$SHOTS/artwork-grid-title-screen.png" -crop 340x680+840+0 +repage "$SHOTS/artwork-details-title-screenshot.png"
echo "saved artwork-details-title-screenshot.png $(md5sum "$SHOTS/artwork-details-title-screenshot.png" | awk '{print $1}')"

echo "---- config grid_art ----"
rg "grid_art" "$CFG/config.toml"
echo "---- media rows ----"
sqlite3 "$XDG_DATA_HOME/retromarchy/library.db" "SELECT kind, source, path FROM media;"
echo "---- cache ----"
find "$XDG_DATA_HOME/retromarchy/media" -type f -printf '%p %s\n'
echo "---- rom unchanged ----"
test "$(wc -c < "$ROMS/Chrono Trigger.sfc")" = "$ROM_BYTES"
echo "rom bytes still $ROM_BYTES"
echo "---- md5 ----"
md5sum "$SHOTS"/artwork-*.png
