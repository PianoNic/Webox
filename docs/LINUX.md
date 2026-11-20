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
| Font | Segoe UI from the system | bundled Selawik (OFL, Segoe-UI-compatible metrics), plus the system's DejaVu / Noto Sans as fallback |
| Settings | `%APPDATA%\WebOx` | `$XDG_CONFIG_HOME/webox` (`~/.config/webox`) |
| Links | `url.dll` | `xdg-open` |

## Build with Cargo

You need Rust 1.85 or newer (install it with [rustup](https://rustup.rs)), a C toolchain, `pkg-config` and these
development packages:

**Debian 12+ / Ubuntu 22.04+** (tested: Debian 13 "trixie")
```sh
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev \
  libxkbcommon-x11-0 libgl1 libegl1 fonts-dejavu-core
```

**Fedora 39+** (untested)
```sh
sudo dnf install gcc pkgconf-pkg-config webkit2gtk4.1-devel gtk3-devel alsa-lib-devel \
  libxkbcommon-x11 mesa-libGL mesa-libEGL dejavu-sans-fonts
```

**Arch Linux / Manjaro** (untested)
```sh
sudo pacman -S --needed base-devel pkgconf webkit2gtk-4.1 gtk3 alsa-lib libxkbcommon-x11 mesa ttf-dejavu
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
libxkbcommon-x11, OpenGL/EGL (e.g. `libwebkit2gtk-4.1-0 libgtk-3-0 libasound2 libxkbcommon-x11-0 libgl1 libegl1` on Debian/Ubuntu).

## Useful switches

| Variable | Effect |
|---|---|
| `WAYLAND_DISPLAY= webox` | run through XWayland (X11) instead of native Wayland |
| `WEBOX_NATIVE_TOASTS=1` / `0` | toasts as desktop notifications / as own windows |
| `WEBOX_NO_TRAY=1` | no tray icon |
| `WEBOX_MULTI_INSTANCE=1` | allow several running copies |
| `WEBKIT_DISABLE_DMABUF_RENDERER=1` | blank or flickering sign-in window (NVIDIA, some VMs) |
| `WEBOX_FALLBACK_FONT=/path/font.ttf` | glyph fallback font |

## Headless tests (Docker)

```sh
docker build -t webox-debian -f packaging/docker/Dockerfile.debian packaging/docker
docker run --rm -v "$PWD:/src" -v webox-target:/target -e CARGO_TARGET_DIR=/target -w /src webox-debian \
  sh -c 'cargo build --bin webox && sh packaging/docker/headless-test.sh && sh packaging/docker/desktop-test.sh'
```
`headless-test.sh` renders the offline sample under Xvfb (X11) and weston (Wayland) into `shots/linux_sample*.png`.
`desktop-test.sh` runs a mock panel and notification server on a private D-Bus (`desktop-mock.py`) and checks the
tray item, its menu, notifications, the launcher badge, the X11 toast windows, XGrabKey hotkeys and the single-instance hand-off.
