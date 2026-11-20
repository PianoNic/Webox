#!/bin/sh
# Headless smoke test of a Linux build: renders the offline sample (--sample --screenshot) once under X11 (Xvfb) and once under
# Wayland (weston's headless backend), both with a private D-Bus session bus. Software GL (Mesa llvmpipe).
#   packaging/docker/headless-test.sh [path/to/webox] [output dir]
set -u
BIN=${1:-/target/debug/webox}
OUT=${2:-/src/shots}
mkdir -p "$OUT"
export HOME=${HOME:-/tmp/home}; mkdir -p "$HOME"
export XDG_RUNTIME_DIR=/tmp/xdg-runtime; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
export LIBGL_ALWAYS_SOFTWARE=1
rc=0

echo "== X11 (Xvfb)"
Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
XPID=$!
sleep 1
env -u WAYLAND_DISPLAY DISPLAY=:99 dbus-run-session -- timeout 60 "$BIN" --sample --screenshot "$OUT/linux_sample.png" >"$OUT/linux_x11.log" 2>&1
echo "exit $?"; tail -5 "$OUT/linux_x11.log"
[ -s "$OUT/linux_sample.png" ] && echo "ok: $OUT/linux_sample.png" || { echo "FAIL: no X11 screenshot"; rc=1; }
kill $XPID 2>/dev/null

if command -v weston >/dev/null; then
  echo "== Wayland (weston headless)"
  weston --backend=headless --width=1600 --height=1000 --socket=wayland-1 --idle-time=0 >/tmp/weston.log 2>&1 &
  WPID=$!
  sleep 2
  env -u DISPLAY WAYLAND_DISPLAY=wayland-1 dbus-run-session -- timeout 60 "$BIN" --sample --screenshot "$OUT/linux_sample_wayland.png" >"$OUT/linux_wayland.log" 2>&1
  echo "exit $?"; tail -5 "$OUT/linux_wayland.log"
  [ -s "$OUT/linux_sample_wayland.png" ] && echo "ok: $OUT/linux_sample_wayland.png" || { echo "FAIL: no Wayland screenshot"; rc=1; }
  kill $WPID 2>/dev/null
fi
exit $rc
