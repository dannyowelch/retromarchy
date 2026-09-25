#!/bin/bash
# Proves first-run config, empty-state body actions, ES-DE import grid, and restart.
set -euo pipefail

DISPLAY_NUM="${DISPLAY_NUM:-:98}"
export DISPLAY="$DISPLAY_NUM"
export GDK_BACKEND=x11
export GDK_CORE_DEVICE_EVENTS=1
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d /tmp/retro-verify.XXXXXX)"
SHOTS="/opt/cursor/artifacts"
mkdir -p "$SHOTS"
rm -f "$SHOTS"/pr3-*.png

export HOME="$WORK/home"
export XDG_CONFIG_HOME="$WORK/config"
export XDG_DATA_HOME="$WORK/data"
export XDG_CACHE_HOME="$WORK/cache"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"

eval "$(dbus-launch --sh-syntax)"

cleanup() {
  pkill -f "target/debug/retromarchy" 2>/dev/null || true
  if [ -n "${WM_PID:-}" ]; then kill "$WM_PID" 2>/dev/null || true; fi
  if [ -n "${XVFB_PID:-}" ]; then kill "$XVFB_PID" 2>/dev/null || true; fi
  if [ -n "${DBUS_SESSION_BUS_PID:-}" ]; then kill "$DBUS_SESSION_BUS_PID" 2>/dev/null || true; fi
}
trap cleanup EXIT

Xvfb "$DISPLAY" -screen 0 1280x800x24 >/tmp/xvfb-pr3.log 2>&1 &
XVFB_PID=$!
sleep 0.5
xfwm4 --daemon >/tmp/xfwm-pr3.log 2>&1 &
WM_PID=$!
sleep 0.5

cd "$ROOT"
cargo build --offline >/tmp/retro-build.log

"$ROOT/target/debug/retromarchy" >/tmp/retro-app.log 2>&1 &
APP_PID=$!
sleep 2

window_id() {
  xdotool search --onlyvisible --name "$1" 2>/dev/null | head -1 || true
}

shot() {
  local name="$1"
  local wid
  wid="$(window_id Retromarchy)"
  sleep 0.3
  import -window "$wid" "$SHOTS/$name"
  echo "saved $name $(wc -c < "$SHOTS/$name") $(md5sum "$SHOTS/$name" | awk '{print $1}')"
}

click_pct() {
  local wid="$1" xp="$2" yp="$3"
  local geo
  geo="$(xdotool getwindowgeometry --shell "$wid")"
  eval "$geo"
  local x=$((X + WIDTH * xp / 100))
  local y=$((Y + HEIGHT * yp / 100))
  echo "click $wid abs $x,$y window ${X},${Y} ${WIDTH}x${HEIGHT}"
  xdotool mousemove "$x" "$y"
  sleep 0.1
  xdotool click 1
  sleep 0.6
}

MAIN="$(window_id Retromarchy)"
xdotool windowsize "$MAIN" 1200 720
xdotool windowmove "$MAIN" 40 40
sleep 0.4
shot "pr3-empty-state.png"

test -f "$XDG_CONFIG_HOME/retromarchy/config.toml"
echo "config after first launch:"
cat "$XDG_CONFIG_HOME/retromarchy/config.toml"

ROMS="$HOME/ROMs"
mkdir -p "$ROMS/nes" "$ROMS/snes" "$ROMS/unknownsys"
echo dummy > "$ROMS/nes/Super Mario Bros.nes"
echo dummy > "$ROMS/nes/Zelda.nes"
echo dummy > "$ROMS/snes/Super Metroid.sfc"
echo dummy > "$ROMS/snes/Chrono Trigger.sfc"
echo dummy > "$ROMS/unknownsys/x.rom"

xdotool key --window "$MAIN" ctrl+i
sleep 0.8
IMP="$(window_id "Import ROMs")"
import -window "$IMP" /tmp/pr3-import-chooser.png
xdotool key --window "$IMP" Return
sleep 0.6
echo "step esde: $(rg -c 'esde clicked' /tmp/retro-app.log || true)"
xdotool key --window "$IMP" Return
sleep 0.6
echo "step scan: $(rg -c 'scan clicked' /tmp/retro-app.log || true)"
import -window "$IMP" /tmp/pr3-import-list.png
xdotool key --window "$IMP" Return
sleep 1.0
echo "step confirm: $(rg -c 'confirm clicked' /tmp/retro-app.log || true)"

shot "pr3-grid-after-import.png"

STUB="$WORK/stub.sh"
cat > "$STUB" << EOF
#!/bin/sh
echo "launched \$1" > "$WORK/launched.txt"
EOF
chmod +x "$STUB"

MAIN="$(window_id Retromarchy)"
echo "main before emu: ${MAIN:-missing}"
xdotool getwindowname "$MAIN" || true
xdotool mousemove 400 400
xdotool click 1
sleep 0.2
xdotool key ctrl+m
sleep 0.3
echo "after xtest ctrl+m"
sleep 0.8
sleep 0.4
EMU="$(window_id "Manage Emulators")"
echo "emu window: ${EMU:-missing}"
xwininfo -id "$EMU" || true
xdotool windowmove "$MAIN" 900 20 || true
xdotool windowsize "$MAIN" 300 200 || true
sleep 0.2
import -window "$EMU" /tmp/pr3-emu.png || echo "emu shot failed"
# The id entry grabs focus when the dialog opens.
xdotool type --delay 20 "stub"
xdotool key Tab
xdotool type --delay 12 "$STUB {rom}"
xdotool key Tab Return
sleep 0.4
import -window "$EMU" /tmp/pr3-emu-filled.png || true
sleep 0.5
import -window "$EMU" /tmp/pr3-emu-filled.png || true
xdotool key Down || true
sleep 0.3
xdotool key Escape || true
sleep 0.4

xdotool windowmove "$MAIN" 40 40 || true
xdotool windowsize "$MAIN" 1200 720 || true
sleep 0.3
xdotool mousemove 400 280 click 1
sleep 0.2
xdotool key Return
sleep 1
if [ -f "$WORK/launched.txt" ]; then
  echo "LAUNCHED: $(cat "$WORK/launched.txt")"
else
  echo "LAUNCH MISSING"
  echo "---- config so far ----"
  cat "$XDG_CONFIG_HOME/retromarchy/config.toml"
fi

kill "$APP_PID" || true
sleep 1
"$ROOT/target/debug/retromarchy" >/tmp/retro-app2.log 2>&1 &
APP_PID=$!
sleep 2
sleep 1
MAIN="$(window_id Retromarchy)"
echo "restart window: $MAIN $(xdotool getwindowname "$MAIN" 2>/dev/null || true)"
xdotool windowsize "$MAIN" 1200 720 || true
xdotool windowmove "$MAIN" 40 40 || true
sleep 0.6
xdotool mousemove 90 150 click 1
sleep 0.5
shot "pr3-grid-after-restart.png"

echo "---- config ----"
cat "$XDG_CONFIG_HOME/retromarchy/config.toml"
echo "---- db ----"
sqlite3 "$XDG_DATA_HOME/retromarchy/library.db" "SELECT title, console FROM games ORDER BY console, title;"
echo "---- shots ----"
md5sum "$SHOTS"/pr3-*.png
wc -c "$SHOTS"/pr3-*.png
