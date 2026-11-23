#!/bin/sh
# Builds a release binary and packs a relocatable tarball (no AppImage): bin/webox, the desktop entry, hicolor icons and an
# install.sh that copies them to ~/.local (or PREFIX). The target system needs the run-time libraries listed in docs/LINUX.md
# (WebKitGTK 4.1, GTK 3, ALSA, libxkbcommon-x11, OpenGL/EGL, GStreamer with its base plugins).
#   packaging/linux/make-tarball.sh            -> dist/webox-<version>-linux-<arch>.tar.gz
set -eu
cd "$(dirname "$0")/../.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
NAME="webox-$VERSION-linux-$ARCH"
TARGET=${CARGO_TARGET_DIR:-target}
cargo build --release --locked -p webox --bin webox
STAGE="dist/$NAME"
rm -rf "$STAGE"; mkdir -p "$STAGE/bin" "$STAGE/share/applications"
install -m755 "$TARGET/release/webox" "$STAGE/bin/webox"
strip "$STAGE/bin/webox" 2>/dev/null || true
install -m644 packaging/linux/webox.desktop "$STAGE/share/applications/webox.desktop"
for s in 16 20 24 32 40 48 60 64 256; do
  install -Dm644 "assets/icon/webox-$s.png" "$STAGE/share/icons/hicolor/${s}x${s}/apps/webox.png"
done
install -m644 assets/fonts/Selawik-OFL.txt "$STAGE/Selawik-OFL.txt"
cat > "$STAGE/install.sh" <<'EOF'
#!/bin/sh
# Installs Webox into PREFIX (default ~/.local): bin/webox, menu entry, icons.  Uninstall: ./install.sh --uninstall
set -eu
PREFIX=${PREFIX:-$HOME/.local}
HERE=$(cd "$(dirname "$0")" && pwd)
FILES="bin/webox share/applications/webox.desktop $(cd "$HERE" && find share/icons -name webox.png)"
if [ "${1:-}" = "--uninstall" ]; then
  for f in $FILES; do rm -f "$PREFIX/$f"; done
  echo "removed Webox from $PREFIX (settings stay in ~/.config/webox)"; exit 0
fi
for f in $FILES; do install -D -m "$( [ "$f" = bin/webox ] && echo 755 || echo 644 )" "$HERE/$f" "$PREFIX/$f"; done
sed -i "s|^Exec=webox|Exec=$PREFIX/bin/webox|" "$PREFIX/share/applications/webox.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" 2>/dev/null || true
echo "installed Webox into $PREFIX - start it from the menu or with $PREFIX/bin/webox"
EOF
chmod 755 "$STAGE/install.sh"
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
echo "dist/$NAME.tar.gz"
