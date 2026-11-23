# Webox on Linux

Webox runs on Linux under **X11 and Wayland**. NixOS and Nix users: see [INSTALL-NIX.md](INSTALL-NIX.md).
For other distributions, build it with Cargo as described below.

## What works on Linux, and how

| Feature | Windows | Linux |
|---|---|---|
| Sign-in | Edge WebView2 inside the window | WebKitGTK 4.1 in a separate window (a GTK widget can't live inside the winit window); cookies stay in `~/.config/webox/EBWebView` |
| Tray icon + menu | Shell_NotifyIcon | StatusNotifierItem + dbusmenu over D-Bus: the panel draws the icon (red badge when unread) and the same menu. GNOME needs the *AppIndicator* extension. Without a tray host there is no icon, and "close to tray" keeps the window open |
| Toasts (messages, reminders) | own windows at the bottom right | **X11**: the same toast windows, placed in the EWMH work area. **Wayland**: desktop notifications (org.freedesktop.Notifications) with the same text and buttons, because a Wayland client can't position its own windows. `WEBOX_NATIVE_TOASTS=1/0` forces either path |
| Taskbar flash / badge | FlashWindowEx, ITaskbarList3 overlay | urgency hint (X11) / xdg-activation (Wayland); unread count via the Unity LauncherEntry API (KDE, Ubuntu Dock, Dash to Dock, Plank) |
| Global hotkeys | RegisterHotKey | **X11**: XGrabKey. **Wayland**: GlobalShortcuts portal (KDE Plasma 6, GNOME 48+, Hyprland); the desktop asks you to confirm once. Without the portal, shortcuts work only while the window has focus |
| Start with the computer | Run key | `~/.config/autostart/webox.desktop` |
| Browser meeting helper | registry | native-messaging manifests in `~/.config/{google-chrome,chromium,microsoft-edge}/NativeMessagingHosts` and `~/.mozilla/native-messaging-hosts` |
| Second start | (new instance) | hands its link to the running copy over D-Bus and exits (`WEBOX_MULTI_INSTANCE=1` turns this off) |
| File dialogs | IFileOpenDialog | xdg-desktop-portal (rfd) |
| Audio | WASAPI (cpal) | ALSA (cpal), which reaches PulseAudio / PipeWire through their ALSA plugins |
| Video playback (shared videos) | Media Foundation: MP4 / MOV / M4V / 3GP, ASF / WMV, AVI, MPEG-TS, MKV / WebM with the codecs Windows has (H.264, MPEG-4, MJPEG, WMV; HEVC, VP9, AV1 with their Store extensions); DXVA hardware decoding | GStreamer (`playbin`): whatever the installed plugins decode, see [Video formats](#video-formats); VA-API hardware decoding when the distribution's `va` plugin is installed. Sound goes to the speaker chosen in Settings > Audio on both |
| Screen sharing (capture + H.264) | Windows.Graphics.Capture (DXGI Desktop Duplication for monitors where it is missing); Media Foundation H.264 encoder, the GPU's when there is one | xdg-desktop-portal ScreenCast -> PipeWire (`pipewiresrc`), on X11 without the portal `ximagesrc`; GStreamer H.264 encoder, see [Screen sharing](#screen-sharing) |
| Font | Segoe UI from the system | bundled Selawik (OFL, Segoe-UI-compatible metrics), plus the system's DejaVu / Noto Sans as fallback |
| Settings | `%APPDATA%\WebOx` | `$XDG_CONFIG_HOME/webox` (`~/.config/webox`) |
| Links | `url.dll` | `xdg-open` |

## Build with Cargo

You need Rust 1.85 or newer (install it with [rustup](https://rustup.rs)), a C toolchain, `pkg-config` and these
development packages:

**Debian 12+ / Ubuntu 22.04+** (tested: Debian 13 "trixie")
```sh
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
  libxkbcommon-x11-0 libgl1 libegl1 fonts-dejavu-core
```

**Fedora 39+** (untested)
```sh
sudo dnf install gcc pkgconf-pkg-config webkit2gtk4.1-devel gtk3-devel alsa-lib-devel \
  gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-good gstreamer1-plugins-bad-free \
  libxkbcommon-x11 mesa-libGL mesa-libEGL dejavu-sans-fonts
```

**Arch Linux / Manjaro** (untested)
```sh
sudo pacman -S --needed base-devel pkgconf webkit2gtk-4.1 gtk3 alsa-lib gstreamer gst-plugins-base gst-plugins-good \
  gst-plugins-bad libxkbcommon-x11 mesa ttf-dejavu
```

Then:
```sh
cargo build --release -p webox --bin webox
./target/release/webox
```

### Install into ~/.local (tarball)

```sh
packaging/linux/make-tarball.sh          # -> dist/webox-<version>-linux-<arch>.tar.gz
tar xf dist/webox-*-linux-*.tar.gz -C /tmp
/tmp/webox-*/install.sh                  # bin, menu entry, icons into ~/.local (PREFIX=... to change)
/tmp/webox-*/install.sh --uninstall
```
The tarball contains the program only. The target system needs the run-time libraries: WebKitGTK 4.1, GTK 3, ALSA,
libxkbcommon-x11, OpenGL/EGL and GStreamer 1.x with its base plugins (e.g. `libwebkit2gtk-4.1-0 libgtk-3-0 libasound2
libxkbcommon-x11-0 libgl1 libegl1 libgstreamer1.0-0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good` on Debian/Ubuntu).

## Useful switches

| Variable | Effect |
|---|---|
| `WAYLAND_DISPLAY= webox` | run through XWayland (X11) instead of native Wayland |
| `WEBOX_NATIVE_TOASTS=1` / `0` | toasts as desktop notifications / as own windows |
| `WEBOX_NO_TRAY=1` | no tray icon |
| `WEBOX_MULTI_INSTANCE=1` | allow several running copies |
| `WEBKIT_DISABLE_DMABUF_RENDERER=1` | blank or flickering sign-in window (NVIDIA, some VMs) |
| `WEBOX_FALLBACK_FONT=/path/font.ttf` | glyph fallback font |
| `WEBOX_VIDEO_LOG=/path/file` | appends why a video does not play (the framework's message and the file name) |

## Video formats

Webox brings no decoder of its own (no FFmpeg is bundled): a shared video plays when GStreamer on the system can decode it.

| Plugins | Adds |
|---|---|
| `gstreamer1.0-plugins-base` (needed) | `playbin`, colour conversion and scaling, Ogg / Theora / Vorbis / Opus |
| `gstreamer1.0-plugins-good` | MP4 / MOV / 3GP, Matroska / WebM, AVI, VP8 / VP9 (libvpx), MJPEG, FLAC |
| `gstreamer1.0-plugins-bad` | AV1 (dav1d), MPEG-TS, H.264 through openh264 where the distribution builds it, VA-API hardware decoding (`va`) |
| `gstreamer1.0-libav` (optional) | H.264, HEVC, AAC, WMV and most other codecs through the distribution's own FFmpeg |

Phone and screen recordings are mostly H.264 + AAC in MP4; on Debian / Ubuntu they need `gstreamer1.0-libav` (or a VA-API
plugin for GPU decoding). A file that cannot be decoded shows "Video kann nicht abgespielt werden" with
"Erneut laden" on its card and can still be downloaded. The Nix package has -base / -good / -bad; `withLibav = true` adds
gst-libav (FFmpeg in the closure, therefore off by default).

## Screen sharing

Sharing a screen or window needs a way to capture it and an H.264 encoder from GStreamer; Webox bundles neither.

- **Capture.** Through the desktop portal's ScreenCast interface (GNOME, KDE, wlroots portals; Wayland and X11). The
  desktop's own dialog picks the screen or window, and the share dialog shows one "Bildschirm" and one "Anwendung" entry
  that open it. This needs `xdg-desktop-portal` with a backend for the desktop plus `gstreamer1.0-pipewire`
  (`pipewiresrc`). On X11 without the portal, Webox lists the screen and the windows itself, with thumbnails, and captures
  them with `ximagesrc` (gst-plugins-good). `WEBOX_SHARE_X11=1` prefers this on X11 even when the portal is there.
- **Encoder.** The first one installed, in this order: `x264enc` (`gstreamer1.0-plugins-ugly`), `openh264enc`
  (`gstreamer1.0-plugins-bad` where the distribution builds openh264), `vah264enc` / `vaapih264enc` (VA-API: `-plugins-bad`'s
  `va` / `gstreamer1.0-vaapi`). `WEBOX_SHARE_ENCODER=<element>` picks one. `h264parse` (`-plugins-bad`) is used when present.

Debian / Ubuntu: `sudo apt install gstreamer1.0-pipewire gstreamer1.0-plugins-ugly xdg-desktop-portal`
(plus the desktop's portal backend, e.g. `xdg-desktop-portal-gnome` or `-kde`).

## Headless tests (Docker)

```sh
docker build -t webox-debian -f packaging/docker/Dockerfile.debian packaging/docker
docker run --rm -v "$PWD:/src" -v webox-target:/target -e CARGO_TARGET_DIR=/target -w /src webox-debian \
  sh -c 'cargo build --bin webox && sh packaging/docker/headless-test.sh && sh packaging/docker/desktop-test.sh'
```
`headless-test.sh` renders the offline sample under Xvfb (X11) and weston (Wayland) into `shots/linux_sample*.png`.
`desktop-test.sh` runs a mock panel and notification server on a private D-Bus (`desktop-mock.py`) and checks the
tray item, its menu, notifications, the launcher badge, the X11 toast windows, XGrabKey hotkeys and the single-instance hand-off.
