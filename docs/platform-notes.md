# Platform notes

How Webox's features map onto each desktop. For developers; users want [LINUX.md](LINUX.md) or [MACOS.md](MACOS.md).

## Linux

Windows mechanism first, Linux after the arrow.

- **Sign-in:** Edge WebView2 inside the window → WebKitGTK 4.1 in a separate window (a GTK widget can't live inside the
  winit window). Cookies stay in `~/.config/webox/EBWebView`.
- **Tray icon and menu:** Shell_NotifyIcon → StatusNotifierItem + dbusmenu over D-Bus. The panel draws the icon (red badge
  when unread) and the same menu. GNOME needs the AppIndicator extension. Without a tray host there is no icon, and "close
  to tray" doesn't hide the window.
- **Toasts:** own windows at the bottom right → **X11**: the same windows, placed in the EWMH work area. **Wayland**: desktop
  notifications (org.freedesktop.Notifications) with the same text and buttons, because a Wayland client can't position
  its own windows. `WEBOX_NATIVE_TOASTS=1/0` forces either path.
- **Taskbar flash / badge:** FlashWindowEx, ITaskbarList3 overlay → urgency hint (X11) / xdg-activation (Wayland); unread
  count via the Unity LauncherEntry API (KDE, Ubuntu Dock, Dash to Dock, Plank).
- **Global hotkeys:** RegisterHotKey → **X11**: XGrabKey. **Wayland**: GlobalShortcuts portal (KDE Plasma 6, GNOME 48+,
  Hyprland); the desktop asks once. Without the portal, shortcuts work only while the window has focus.
- **Start with the computer:** Run key → `~/.config/autostart/webox.desktop`.
- **Browser meeting helper:** registry → native-messaging manifests in
  `~/.config/{google-chrome,chromium,microsoft-edge}/NativeMessagingHosts` and `~/.mozilla/native-messaging-hosts`.
- **Second start:** hands its link to the running copy over D-Bus and exits (`WEBOX_MULTI_INSTANCE=1` turns this off).
- **File dialogs:** IFileOpenDialog → xdg-desktop-portal (rfd).
- **Composer screenshot:** GDI capture + own area picker → xdg-desktop-portal Screenshot (the desktop's own dialog picks the
  area). **X11** without a portal: the whole screen and Webox's area picker. Wayland without a portal: Webox says so in a toast.
- **Pasting an image:** CF_DIBV5 / PNG → **X11**: the PNG on the clipboard. **Wayland**: read natively through the
  data-control protocol (KDE Plasma, wlroots compositors such as Sway and Hyprland); PNG, WebP, JPEG and GIF. GNOME has no
  data-control protocol, so there Webox reads the clipboard through XWayland; if an image from a Wayland app doesn't arrive,
  run Webox under XWayland (`WAYLAND_DISPLAY= webox`).
- **Audio:** WASAPI (cpal) → ALSA (cpal), which reaches PulseAudio / PipeWire through their ALSA plugins.
- **Video playback:** Media Foundation (DXVA hardware decoding) → GStreamer `playbin`: whatever the installed plugins decode;
  VA-API hardware decoding with the distribution's `va` plugin. Sound goes to the speaker chosen in Settings > Audio.
- **Screen sharing:** Windows.Graphics.Capture (DXGI Desktop Duplication where it is missing) + Media Foundation H.264 →
  xdg-desktop-portal ScreenCast → PipeWire (`pipewiresrc`); on X11 without the portal `ximagesrc`; a GStreamer H.264
  encoder (`x264enc`, `openh264enc`, `vah264enc` / `vaapih264enc`, in that order; `h264parse` when present).
- **Font:** Segoe UI → bundled Selawik (OFL, Segoe-UI-compatible metrics), with the system's DejaVu / Noto Sans as fallback.
- **Settings:** `%APPDATA%\WebOx` → `$XDG_CONFIG_HOME/webox` (`~/.config/webox`).
- **Links:** `url.dll` → `xdg-open`.
- **Updates:** Linux only reports a new version; packages update through Nix, the distribution or a new tarball.

The Nix package ships gst-plugins-base / -good / -bad; `withLibav = true` adds gst-libav (FFmpeg in the closure, so off
by default). Off NixOS it brings its own Mesa drivers (`WEBOX_HOST_GL=1` turns them off); the `webox-nixos` package
leaves them out.

### Headless tests (Docker)

```sh
docker build -t webox-debian -f packaging/docker/Dockerfile.debian packaging/docker
docker run --rm -v "$PWD:/src" -v webox-target:/target -e CARGO_TARGET_DIR=/target -w /src webox-debian \
  sh -c 'cargo build --bin webox && sh packaging/docker/headless-test.sh && sh packaging/docker/desktop-test.sh'
```

- `headless-test.sh` renders the offline sample under Xvfb (X11) and weston (Wayland) into `shots/linux_sample*.png`.
- `desktop-test.sh` runs a mock panel and notification server on a private D-Bus (`desktop-mock.py`) and checks the tray
  item, its menu, notifications, the launcher badge, the X11 toast windows, XGrabKey hotkeys and the single-instance hand-off.

## macOS

- **Menu bar:** a status item with the unread dot and count, and the same menu as the tray elsewhere. Closing the window
  hides Webox there (Settings > "close to tray"); a click on the Dock icon brings it back. The Dock icon shows the unread
  count and bounces once for a new message while Webox is in the background.
- **App menu:** Webox (Settings ⌘,, Hide ⌘H, Quit ⌘Q), Edit (undo, cut, copy, paste, select all),
  Window (Close ⌘W, Minimize ⌘M).
- **Shortcuts** use ⌘ where Windows and Linux use Ctrl, written the macOS way (⇧⌘N). Global hotkeys are opt-in
  (Settings > Keyboard shortcuts), as on Linux.
- **Notifications** come from Webox in Notification Center; a click opens the space, reminders keep their buttons and an
  incoming call has Annehmen / Ablehnen. Started outside `Webox.app` (`cargo run`) they fall back to plain banners from
  "Script Editor".
- **Links:** `webexteams://` and `collabhost://` links (from the browser, a mail, "Copy space link") open the space in
  Webox, at start-up and while it runs. `Webox.app` declares both schemes; when the official Webex app is installed too,
  macOS picks one of them for `webexteams://`.
- **Sign-in** is a WebKit window (WKWebView), the same flow as WebView2 on Windows and WebKitGTK on Linux.
- **Media:** call audio through Core Audio, cameras and video playback through AVFoundation, screen sharing through
  ScreenCaptureKit with VideoToolbox's H.264 encoder.
- **Start at login:** a login item; `~/Library/LaunchAgents/io.github.pianonic.webox.plist` outside the app bundle or
  before macOS 13.
- **Browser meeting helper:** `NativeMessagingHosts` in each browser's folder under `~/Library/Application Support`.
- **Plugins:** `libcollabhost_<name>.dylib`; plugin repositories use the platform keys `macos-aarch64` / `macos-x86_64`.
- **Secret plugin settings:** the login Keychain (service "Webox"); the settings file when the Keychain refuses.
- **Icon:** the source in `assets/icon` is 256 px, so the Dock / Finder icon is no larger.
- **Packaging:** `make-app.sh` builds both architectures, joins them with `lipo`, signs ad hoc and zips with `ditto`.
  CI builds and tests on `macos-latest` and keeps a start-up screenshot (`--sample --screenshot`) as an artifact; the
  `macOS package (dry run)` workflow builds the universal zip without publishing anything.
