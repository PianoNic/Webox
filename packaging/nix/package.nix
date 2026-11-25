# Webox for Linux / NixOS. Built from this repository with the pinned Cargo.lock (no network at build time).
# Built with crane (flake input): the ~760 dependencies are one derivation (cargoArtifacts) that only changes with Cargo.lock /
# Cargo.toml, so a source edit recompiles only Webox itself and CI reuses the dependencies from its Nix store cache.
{
  lib,
  craneLib,
  stdenvNoCC,
  pkg-config,
  wrapGAppsHook3,
  # linked at build time
  webkitgtk_4_1,
  gtk3,
  glib,
  cairo,
  pango,
  gdk-pixbuf,
  libsoup_3,
  alsa-lib,
  # run time: TLS for WebKit, GSettings schemas, media for the sign-in page
  glib-networking,
  gsettings-desktop-schemas,
  gst_all_1,
  # dlopen()ed by eframe / winit at run time (OpenGL, Vulkan, Wayland, X11, keyboard)
  libGL,
  vulkan-loader,
  libxkbcommon,
  wayland,
  pkgs,
  # glyph fallback behind the bundled Selawik font; xdg-open for links
  dejavu_fonts,
  xdg-utils,
  libsecret,
  pipewire,
  noto-fonts-cjk-sans,
  makeShellWrapper,
  mesa,
  # Outside NixOS, programs from Nix can't load the distribution's OpenGL drivers. With this on, the wrapper points libglvnd
  # at Nix's Mesa whenever the NixOS driver directory /run/opengl-driver is missing (Ubuntu, Fedora, Arch, ...): Intel/AMD
  # run accelerated, anything else falls back to software rendering. Costs ~800 MiB; the NixOS module turns it off.
  withMesaFallback ? true,
  # Video playback plays what the GStreamer plugins decode. -base / -good / -bad cover MP4, MOV, MKV, WebM, AVI with VP8, VP9,
  # AV1 (libaom's av1dec; nixpkgs has dav1d only in gst-plugins-rs), H.264 (openh264) and VA-API hardware decoding; gst-libav
  # adds HEVC and older codecs but brings FFmpeg into the closure, so it is off unless asked for (Webox bundles no FFmpeg of its own).
  withLibav ? false,
}:

let
  cargoToml = lib.importTOML ../../Cargo.toml;
  # New nixpkgs moved the X libraries out of xorg (old names warn); older ones only have xorg.*.
  x11 = new: old: pkgs.${new} or pkgs.xorg.${old};
  runtimeLibs = [
    libGL
    vulkan-loader
    libxkbcommon
    wayland
    (x11 "libx11" "libX11")
    (x11 "libxcursor" "libXcursor")
    (x11 "libxrandr" "libXrandr")
    (x11 "libxi" "libXi")
    (x11 "libxcb" "libxcb")
  ];
in
let
  version = cargoToml.package.version;

  # Only what the build needs (no target/, screenshots or tooling), so unrelated edits don't trigger rebuilds. build.rs reads
  # assets/ and secrets.toml: a file missing here silently drops a feature (without secrets.toml sign-in is "not configured").
  src = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [
      ../../Cargo.toml
      ../../Cargo.lock
      ../../build.rs
      ../../secrets.toml # sign-in and the GIF picker (build.rs embeds it)
      ../../src
      ../../assets
      ../../examples
      ../../plugin-api
      ../../plugins
      ../../packaging/linux
    ];
  };

  commonArgs = {
    pname = "webox-unwrapped";
    inherit version src;
    strictDeps = true;
    cargoExtraArgs = "--locked -p webox --bin webox";
    # Git dependencies need a fixed hash for the offline vendor step (Cargo.toml pins deep_filter to a commit).
    outputHashes."git+https://github.com/Rikorose/DeepFilterNet?rev=978576aa8400552a4ce9730838c635aa30db5e61#978576aa8400552a4ce9730838c635aa30db5e61" =
      "sha256-5bYbfO1kmduNm9YV5niaaPvRIDRmPt4QOX7eKpK+sWY=";
    # The unit tests run in the dev shell / CI; some need a display or network.
    doCheck = false;

    nativeBuildInputs = [ pkg-config ];
    buildInputs = [
      webkitgtk_4_1
      gtk3
      glib
      cairo
      pango
      gdk-pixbuf
      libsoup_3
      alsa-lib
      # video playback (src/media/video/gst.rs): gstreamer-rs links libgstreamer, -app, -video, -audio
      gst_all_1.gstreamer
      gst_all_1.gst-plugins-base
    ];
  };

  # Every dependency, compiled once against dummy sources of this workspace: changes only with Cargo.lock / Cargo.toml.
  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      # Only `cargo build`: the default extra `cargo check` pass would compile the dependencies' metadata a second time.
      buildPhaseCargoCommand = "cargoWithProfile build ${commonArgs.cargoExtraArgs}";
    }
  );

  # The Rust build (one compile of Webox on top of cargoArtifacts, shared by both variants of the wrapper below).
  unwrapped = craneLib.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      # Keep the target/ archive out of $out: the wrapper references this output, so it would land in the closure.
      doInstallCargoArtifacts = false;

      postInstall = ''
        install -Dm644 packaging/linux/webox.desktop $out/share/applications/webox.desktop
        for s in 16 20 24 32 40 48 60 64 256; do
          install -Dm644 assets/icon/webox-$s.png $out/share/icons/hicolor/''${s}x''${s}/apps/webox.png
        done
      '';

      passthru = { inherit cargoArtifacts; };
      meta.platforms = lib.platforms.linux;
    }
  );
in
stdenvNoCC.mkDerivation {
  pname = "webox";
  inherit version;
  dontUnpack = true;
  dontConfigure = true;
  dontBuild = true;

  # wrapGAppsHook3 collects GIO modules (TLS for WebKit), GSettings schemas and GStreamer plugins into gappsWrapperArgs.
  nativeBuildInputs = [
    wrapGAppsHook3
    makeShellWrapper
  ];
  buildInputs = [
    gtk3
    webkitgtk_4_1
    glib-networking
    gsettings-desktop-schemas
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
    gst_all_1.gst-plugins-bad
    pipewire # libgstpipewire.so (pipewiresrc) for screen share
  ] ++ lib.optional withLibav gst_all_1.gst-libav;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    cp -r ${unwrapped}/share $out/share
    chmod -R u+w $out/share
    runHook postInstall
  '';

  # A shell wrapper (for the driver check) around the unwrapped binary, plus the dlopen()ed libraries.
  dontWrapGApps = true;
  postFixup = ''
    makeShellWrapper ${unwrapped}/bin/webox $out/bin/webox "''${gappsWrapperArgs[@]}" \
      --run 'export WEBOX_ORIG_LD_LIBRARY_PATH="''${LD_LIBRARY_PATH-}"' \
      --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibs}" \
      --set-default WEBOX_FALLBACK_FONT "${dejavu_fonts}/share/fonts/truetype/DejaVuSans.ttf" \
      --set-default WEBOX_FALLBACK_FONTS "${noto-fonts-cjk-sans}/share/fonts/opentype/noto-cjk/NotoSansCJK-VF.otf.ttc" \
      --set WEBOX_WRAPPER "$out/bin/webox" \
      --suffix PATH : "${lib.makeBinPath [ xdg-utils libsecret glib.bin ]}" ${lib.optionalString withMesaFallback ''\
      --run 'if [ ! -e /run/opengl-driver/lib ] && [ -z "''${WEBOX_HOST_GL:-}" ]; then
        export __EGL_VENDOR_LIBRARY_DIRS="''${__EGL_VENDOR_LIBRARY_DIRS:-${mesa}/share/glvnd/egl_vendor.d}"
        export __GLX_VENDOR_LIBRARY_NAME="''${__GLX_VENDOR_LIBRARY_NAME:-mesa}"
        export LIBGL_DRIVERS_PATH="''${LIBGL_DRIVERS_PATH:-${mesa}/lib/dri}"
        export GBM_BACKENDS_PATH="''${GBM_BACKENDS_PATH:-${mesa}/lib/gbm}"
        export LD_LIBRARY_PATH="${mesa}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
      fi'
    ''}
  '';

  passthru = {
    inherit runtimeLibs withMesaFallback unwrapped;
    inherit (unwrapped) cargoArtifacts;
  };

  meta = {
    description = "Desktop client for Webex messaging, meetings and calls (Rust/egui)";
    mainProgram = "webox";
    platforms = lib.platforms.linux;
  };
}
