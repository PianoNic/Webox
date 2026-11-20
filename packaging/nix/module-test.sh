#!/bin/sh
# Evaluates the NixOS and home-manager modules in throw-away configurations (as in docs/INSTALL-NIX.md) and prints what they
# produce. Needs Nix with flakes; network for nixpkgs / home-manager.   packaging/nix/module-test.sh [/path/to/repo]
set -eu
REPO=$(cd "${1:-$(dirname "$0")/../..}" && pwd)
T=$(mktemp -d)
cat > "$T/flake.nix" <<EOF
{
  inputs = {
    webox.url = "path:$REPO";
    nixpkgs.follows = "webox/nixpkgs";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
  };
  outputs = { nixpkgs, home-manager, webox, ... }: {
    nixosConfigurations.test = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        webox.nixosModules.default
        {
          programs.webox.enable = true;
          programs.webox.autostart = true;
          boot.loader.grub.enable = false;
          fileSystems."/" = { device = "/dev/null"; fsType = "ext4"; };
          system.stateVersion = "25.05";
        }
      ];
    };
    homeConfigurations.test = home-manager.lib.homeManagerConfiguration {
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      modules = [
        webox.homeManagerModules.default
        {
          home.username = "test";
          home.homeDirectory = "/home/test";
          home.stateVersion = "25.05";
          programs.webox.enable = true;
          programs.webox.autostart = true;
        }
      ];
    };
  };
}
EOF
cd "$T"
echo "== NixOS module"
nix eval --raw '.#nixosConfigurations.test.config.environment.etc."xdg/autostart/webox.desktop".text'
nix eval '.#nixosConfigurations.test.config.environment.systemPackages' --apply 'ps: builtins.filter (n: n == "webox") (map (p: p.pname or "") ps)'
nix eval --raw '.#nixosConfigurations.test.config.system.build.toplevel.drvPath'; echo
echo "== home-manager module"
nix eval --raw '.#homeConfigurations.test.config.xdg.configFile."autostart/webox.desktop".text'
nix eval '.#homeConfigurations.test.config.home.packages' --apply 'ps: builtins.filter (n: n == "webox") (map (p: p.pname or "") ps)'
nix eval --raw '.#homeConfigurations.test.activationPackage.drvPath'; echo
