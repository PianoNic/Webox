# home-manager module: `programs.webox.enable = true;` installs Webox for this user;
# `programs.webox.autostart = true;` adds ~/.config/autostart/webox.desktop (starts minimized in the tray).
# (While home-manager owns that file, the in-app "start with the computer" switch can't change it.)
self:
{ config, lib, pkgs, osConfig ? null, ... }:

let
  cfg = config.programs.webox;
in
{
  options.programs.webox = {
    enable = lib.mkEnableOption "Webox, a desktop client for Webex messaging, meetings and calls";

    package = lib.mkOption {
      type = lib.types.package;
      # As a NixOS module (osConfig set) the system has its own GL drivers: skip the ~800 MiB Mesa fallback.
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.${if osConfig != null then "webox-nixos" else "default"};
      defaultText = lib.literalExpression "webox.packages.\${pkgs.stdenv.hostPlatform.system}.default";
      description = "The Webox package to use.";
    };

    autostart = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Start Webox minimized (in the tray) when you log in (XDG autostart).";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile."autostart/webox.desktop" = lib.mkIf cfg.autostart {
      text = ''
        [Desktop Entry]
        Type=Application
        Name=Webox
        Icon=webox
        Terminal=false
        Exec=${lib.getExe cfg.package} --minimized --autostarted
        X-GNOME-Autostart-enabled=true
      '';
    };
  };
}
