{
  # Webox on Linux / NixOS.
  #
  #   nix run .                    start it (or: nix run github:PianoNic/Webox)
  #   nix build .                  ./result/bin/webox
  #   nix profile install .        install for this user
  #   nix develop                  dev shell: cargo build / cargo run / cargo test
  #
  # NixOS:        imports = [ webox.nixosModules.default ];        programs.webox.enable = true;
  # home-manager: imports = [ webox.homeManagerModules.default ];  programs.webox.enable = true;
  # Overlay:      nixpkgs.overlays = [ webox.overlays.default ];   -> pkgs.webox
  # See docs/INSTALL-NIX.md.
  description = "Webox - desktop client for Webex messaging, meetings and calls (Rust/egui, WebKitGTK sign-in on Linux)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ self.overlays.default ];
            }
          )
        );
    in
    {
      overlays.default = final: _prev: { webox = final.callPackage ./packaging/nix/package.nix { }; };

      packages = forAll (pkgs: {
        default = pkgs.webox;
        webox = pkgs.webox;
        # Without the Mesa fallback for non-NixOS systems (NixOS provides the drivers; ~800 MiB smaller). The NixOS module uses it.
        webox-nixos = pkgs.webox.override { withMesaFallback = false; };
      });

      apps = forAll (pkgs: {
        default = {
          type = "app";
          program = "${pkgs.webox}/bin/webox";
          meta.description = "Start Webox";
        };
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [
            pkgs.webox.unwrapped
            pkgs.webox
          ];
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
          ];
          # What the wrapper of the package sets, for `cargo run`.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath pkgs.webox.runtimeLibs;
          GIO_EXTRA_MODULES = "${pkgs.glib-networking}/lib/gio/modules";
          WEBOX_FALLBACK_FONT = "${pkgs.dejavu_fonts}/share/fonts/truetype/DejaVuSans.ttf";
          shellHook = ''
            export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:''${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
          '';
        };
      });

      nixosModules.default = import ./packaging/nix/nixos-module.nix self;
      homeManagerModules.default = import ./packaging/nix/hm-module.nix self;

      checks = forAll (pkgs: {
        package = pkgs.webox;
      });

      formatter = forAll (pkgs: pkgs.nixfmt);
    };
}
