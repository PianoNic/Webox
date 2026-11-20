#!/bin/sh
# Desktop-integration test on X11 (Xvfb) with a mock panel / notification server on a private session bus (desktop-mock.py):
#  1. tray: StatusNotifierItem registration, icon pixmaps, tooltip, dbusmenu rows, a menu click ("Info") that raises a toast,
#     which goes to the notification server (WEBOX_NATIVE_TOASTS=1, the Wayland path), whose default action comes back;
#     launcher badge signal.
#  2. the app's own toast windows on X11 (placement + capture through x11rb): shots/linux_toast.png
#  3. global hotkeys: an incoming sample call makes AnswerCall / DeclineCall global (XGrabKey); xdotool presses Ctrl+D.
#   packaging/docker/desktop-test.sh [path/to/webox] [output dir]
set -u
BIN=${1:-/target/debug/webox}
OUT=${2:-/src/shots}
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"
export HOME=${HOME:-/tmp/home}; mkdir -p "$HOME"
export XDG_RUNTIME_DIR=/tmp/xdg-runtime; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
export LIBGL_ALWAYS_SOFTWARE=1 DISPLAY=:99
unset WAYLAND_DISPLAY
Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
sleep 1
export BIN OUT HERE
dbus-run-session -- sh -c '
  python3 "$HERE/desktop-mock.py" >"$OUT/desktop_mock.log" 2>&1 &
  sleep 1
  echo "== 1. tray + notifications"
  WEBOX_NATIVE_TOASTS=1 timeout 15 "$BIN" --sample >"$OUT/desktop_app.log" 2>&1
  echo "== 2. X11 toast window"
  WEBOX_NATIVE_TOASTS=0 WEBOX_SAMPLE_TOAST=message,reminder WEBOX_TOAST_SHOT="$OUT/linux_toast.png" timeout 30 "$BIN" --sample >"$OUT/desktop_toast.log" 2>&1
  echo "exit $?"; cat "$OUT/linux_toast.png.txt" 2>/dev/null
  echo "== 2b. toast while the main window is hidden in the tray (unmapped)"
  WEBOX_NATIVE_TOASTS=0 WEBOX_TOAST_HIDE=1 WEBOX_SAMPLE_TOAST=mention WEBOX_TOAST_DELAY=3 WEBOX_TOAST_SHOT="$OUT/linux_toast_hidden.png" timeout 30 "$BIN" --sample >"$OUT/desktop_toast_hidden.log" 2>&1
  echo "exit $?"; cat "$OUT/linux_toast_hidden.png.txt" 2>/dev/null
  echo "== 3. global hotkeys (XGrabKey)"
  WEBOX_SAMPLE_CALL=incoming WEBOX_HOTKEY_LOG=1 "$BIN" --sample >"$OUT/desktop_hotkeys.log" 2>&1 &
  APP=$!
  sleep 6
  import -window root "$OUT/linux_call_x11.png"
  xdotool key ctrl+d
  sleep 2
  kill $APP 2>/dev/null; wait $APP 2>/dev/null
  grep hotkeys "$OUT/desktop_hotkeys.log"
  echo "== 4. single instance (second start hands over and quits)"
  WEBOX_NO_TRAY=1 "$BIN" >"$OUT/desktop_first.log" 2>&1 &
  FIRST=$!
  sleep 4
  START=$(date +%s)
  timeout 20 "$BIN" "collabhost://space/s2" >"$OUT/desktop_second.log" 2>&1
  echo "second start exit $? after $(( $(date +%s) - START ))s; first still running: $(kill -0 $FIRST 2>/dev/null && echo yes || echo no)"
  kill $FIRST 2>/dev/null; wait $FIRST 2>/dev/null
'
echo "== mock desktop log"
cat "$OUT/desktop_mock.log"
