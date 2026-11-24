#!/bin/sh
# Builds release binaries for Apple Silicon and Intel, joins them into one universal program (lipo) and packs Webox.app
# (Info.plist, .icns made from assets/icon, German + English permission texts), signed ad hoc (no Apple Developer certificate:
# Gatekeeper asks once, see docs/MACOS.md), zipped with ditto so the signature survives.
#   packaging/macos/make-app.sh               -> dist/Webox.app, dist/webox-<version>-macos-universal.zip
#   WEBOX_SKIP_BUILD=1 packaging/macos/make-app.sh   packs existing target/{aarch64,x86_64}-apple-darwin/release/webox
set -eu
cd "$(dirname "$0")/../.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="webox-$VERSION-macos-universal"
TARGET=${CARGO_TARGET_DIR:-target}
ARCHS="aarch64-apple-darwin x86_64-apple-darwin"
if [ -z "${WEBOX_SKIP_BUILD:-}" ]; then
  command -v rustup >/dev/null && rustup target add $ARCHS
  for t in $ARCHS; do cargo build --release --locked -p webox --bin webox --target "$t"; done
fi
mkdir -p dist
lipo -create -output dist/webox-universal $(for t in $ARCHS; do echo "$TARGET/$t/release/webox"; done)
lipo -info dist/webox-universal

APP=dist/Webox.app
rm -rf "$APP" "dist/$NAME.zip"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/en.lproj" "$APP/Contents/Resources/de.lproj"
install -m755 dist/webox-universal "$APP/Contents/MacOS/webox"
rm -f dist/webox-universal
strip -x "$APP/Contents/MacOS/webox" 2>/dev/null || true
sed "s/@VERSION@/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Permission prompts in the user's language.
cat > "$APP/Contents/Resources/en.lproj/InfoPlist.strings" <<'EOF'
"NSMicrophoneUsageDescription" = "Webox uses the microphone for calls and meetings.";
"NSCameraUsageDescription" = "Webox uses the camera for video calls and meetings.";
EOF
cat > "$APP/Contents/Resources/de.lproj/InfoPlist.strings" <<'EOF'
"NSMicrophoneUsageDescription" = "Webox verwendet das Mikrofon für Anrufe und Meetings.";
"NSCameraUsageDescription" = "Webox verwendet die Kamera für Videoanrufe und Meetings.";
EOF

# The icon: the PNG frames of assets/icon (largest 256 px) as an iconset; 128 px is scaled down from 256 with sips.
SET=$(mktemp -d)/webox.iconset
mkdir -p "$SET"
cp assets/icon/webox-16.png "$SET/icon_16x16.png"
cp assets/icon/webox-32.png "$SET/icon_16x16@2x.png"
cp assets/icon/webox-32.png "$SET/icon_32x32.png"
cp assets/icon/webox-64.png "$SET/icon_32x32@2x.png"
sips -z 128 128 assets/icon/webox-256.png --out "$SET/icon_128x128.png" >/dev/null
cp assets/icon/webox-256.png "$SET/icon_128x128@2x.png"
cp assets/icon/webox-256.png "$SET/icon_256x256.png"
iconutil -c icns "$SET" -o "$APP/Contents/Resources/webox.icns"
rm -rf "$(dirname "$SET")"

install -m644 LICENSE THIRD_PARTY_NOTICES.md assets/fonts/Selawik-OFL.txt "$APP/Contents/Resources/"

# Ad-hoc signature: Apple Silicon runs no unsigned code, and the bundle seal covers Info.plist and the resources.
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

ditto -c -k --sequesterRsrc --keepParent "$APP" "dist/$NAME.zip"
echo "dist/$NAME.zip"
