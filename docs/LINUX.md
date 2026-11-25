# <img src="../assets/branding/platforms/linux.svg" width="28" height="28" alt="" /> Webox on Linux

Runs on X11 and Wayland, x86_64. Using Nix or NixOS? Go to [INSTALL-NIX.md](INSTALL-NIX.md) instead.

## Install

1. Install the libraries Webox needs (pick your distribution):

   ```sh
   # Debian / Ubuntu
   sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libasound2 libxkbcommon-x11-0 libgl1 libegl1 \
     libgstreamer1.0-0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good
   # Fedora
   sudo dnf install webkit2gtk4.1 gtk3 alsa-lib libxkbcommon-x11 mesa-libGL mesa-libEGL \
     gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good
   # Arch / Manjaro
   sudo pacman -S --needed webkit2gtk-4.1 gtk3 alsa-lib libxkbcommon-x11 mesa \
     gstreamer gst-plugins-base gst-plugins-good
   ```

2. Download `webox-<version>-linux-x86_64.tar.gz` from the [latest release](https://github.com/PianoNic/Webox/releases/latest).

3. Unpack it and run the installer:

   ```sh
   tar xf webox-*-linux-x86_64.tar.gz
   ./webox-*-linux-x86_64/install.sh
   ```

4. Start **Webox** from your application menu (or run `~/.local/bin/webox`).

That's it.

> [!NOTE]
> Webox tells you when a new version is out, but on Linux it doesn't update itself.
> To update, download the new tarball and repeat steps 3 and 4.

---

## More

<details>
<summary><b>Check the download (optional)</b></summary>

Each tarball has a `.sha256` file next to it in the release:

```sh
sha256sum -c webox-<version>-linux-x86_64.tar.gz.sha256
```

</details>

<details>
<summary><b>Install somewhere else / uninstall</b></summary>

`install.sh` copies the program, the menu entry and the icons into `~/.local`. Set `PREFIX` to change that:

```sh
PREFIX=/opt/webox ./webox-*-linux-x86_64/install.sh
```

Uninstall (use the same `PREFIX` if you set one):

```sh
./webox-*-linux-x86_64/install.sh --uninstall
```

Your settings and sign-in stay in `~/.config/webox`. Delete that folder to remove everything.

</details>

<details>
<summary><b>Videos don't play</b></summary>

Webox plays shared videos with the GStreamer plugins on your system. It brings no decoders of its own.

Most phone and screen recordings are H.264 in MP4. They need the `libav` plugin:

- Debian / Ubuntu: `sudo apt install gstreamer1.0-libav`
- Fedora: `sudo dnf install gstreamer1-plugin-libav`
- Arch: `sudo pacman -S gst-libav`

What each plugin set adds:

- **base** (required): Ogg, Theora, Vorbis, Opus.
- **good**: MP4 / MOV / 3GP, MKV / WebM, AVI, VP8 / VP9, MJPEG, FLAC.
- **bad**: AV1, MPEG-TS, VA-API hardware decoding.
- **libav**: H.264, HEVC, AAC, WMV and most other codecs.

A video that can't be decoded can still be downloaded from its card.
To see why it failed, start Webox with `WEBOX_VIDEO_LOG=/tmp/webox-video.log webox`.

</details>

<details>
<summary><b>Screen sharing doesn't work</b></summary>

Screen sharing needs the desktop portal, PipeWire for GStreamer and an H.264 encoder.

Debian / Ubuntu:

```sh
sudo apt install xdg-desktop-portal gstreamer1.0-pipewire gstreamer1.0-plugins-ugly
```

Also install your desktop's portal backend, e.g. `xdg-desktop-portal-gnome` or `xdg-desktop-portal-kde`.

- The desktop's own dialog picks the screen or window.
- On X11 without a portal, Webox lists screens and windows itself.
- Encoders, tried in this order: `x264enc` (plugins-ugly), `openh264enc` (plugins-bad), VA-API (`vah264enc` / `vaapih264enc`).
- Force one with `WEBOX_SHARE_ENCODER=<element>`. Prefer the X11 picker with `WEBOX_SHARE_X11=1`.

</details>

<details>
<summary><b>Other problems</b></summary>

**Sign-in window is blank or flickers** (NVIDIA, some VMs):

```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 webox
```

**No tray icon on GNOME:** install the
[AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/) (Ubuntu has it already).
Without a tray, closing the window quits Webox instead of hiding it.

**Global shortcuts don't work on Wayland:** they need the GlobalShortcuts portal (KDE Plasma 6, GNOME 48+, Hyprland).
The desktop asks you to confirm once. Without it, shortcuts only work while Webox has focus.

**Pasting an image from a Wayland app doesn't work (GNOME):** run Webox through XWayland:

```sh
WAYLAND_DISPLAY= webox
```

</details>

<details>
<summary><b>Environment variables</b></summary>

- `WAYLAND_DISPLAY= webox`: run under XWayland (X11) instead of native Wayland.
- `WEBOX_NATIVE_TOASTS=1` / `0`: show notifications as desktop notifications / as Webox's own windows.
- `WEBOX_NO_TRAY=1`: no tray icon.
- `WEBOX_MULTI_INSTANCE=1`: allow several running copies.
- `WEBKIT_DISABLE_DMABUF_RENDERER=1`: fixes a blank sign-in window.
- `WEBOX_FALLBACK_FONT=/path/font.ttf`: font for characters the bundled font lacks.
- `WEBOX_VIDEO_LOG=/path/file`: log why a video doesn't play.

</details>

<details>
<summary><b>Build from source</b></summary>

You need Rust ([rustup](https://rustup.rs)), a C toolchain, `pkg-config` and the development packages.

Debian 12+ / Ubuntu 22.04+ (tested on Debian 13):

```sh
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
  libxkbcommon-x11-0 libgl1 libegl1 fonts-dejavu-core
```

Fedora 39+ (untested):

```sh
sudo dnf install gcc pkgconf-pkg-config webkit2gtk4.1-devel gtk3-devel alsa-lib-devel \
  gstreamer1-devel gstreamer1-plugins-base-devel gstreamer1-plugins-good gstreamer1-plugins-bad-free \
  libxkbcommon-x11 mesa-libGL mesa-libEGL dejavu-sans-fonts
```

Arch / Manjaro (untested):

```sh
sudo pacman -S --needed base-devel pkgconf webkit2gtk-4.1 gtk3 alsa-lib gstreamer gst-plugins-base gst-plugins-good \
  gst-plugins-bad libxkbcommon-x11 mesa ttf-dejavu
```

Build and run:

```sh
cargo build --release -p webox --bin webox
./target/release/webox
```

Or pack your own tarball and install it as above:

```sh
packaging/linux/make-tarball.sh    # -> dist/webox-<version>-linux-<arch>.tar.gz
```

> [!IMPORTANT]
> Sign-in needs a `secrets.toml` at build time. Without it the app runs but can't sign in.
> See [dev-setup.md](dev-setup.md#secrets).

See [dev-setup.md](dev-setup.md) for tests and the rest. How each feature works on Linux is in
[platform-notes.md](platform-notes.md#linux).

</details>
