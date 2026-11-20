#!/bin/sh
# Opens the embedded sign-in (WebKitGTK window) for a placeholder address under Xvfb and photographs the screen once the
# Webex login page has loaded. Network: read-only (home-cluster lookup + the login page). Nothing is submitted.
#   packaging/docker/signin-test.sh [path/to/webox] [output dir]
set -u
BIN=${1:-/target/debug/webox}
OUT=${2:-/src/shots}
mkdir -p "$OUT"
export HOME=/tmp/signin-home; rm -rf "$HOME"; mkdir -p "$HOME"
export XDG_RUNTIME_DIR=/tmp/xdg-runtime; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
export LIBGL_ALWAYS_SOFTWARE=1 DISPLAY=:98
unset WAYLAND_DISPLAY
Xvfb :98 -screen 0 1600x1000x24 -nolisten tcp >/tmp/xvfb98.log 2>&1 &
sleep 1
export BIN OUT
dbus-run-session -- sh -c '
  WEBOX_NO_TRAY=1 "$BIN" --signin someone@example.com >"$OUT/signin.log" 2>&1 &
  APP=$!
  sleep ${SIGNIN_WAIT:-25}
  import -window root "$OUT/linux_signin.png"
  xdotool search --name "Webex" getwindowname %@ 2>/dev/null
  kill $APP 2>/dev/null; wait $APP 2>/dev/null
'
tail -5 "$OUT/signin.log"
ls -la "$HOME/.config/webox/EBWebView" 2>/dev/null
