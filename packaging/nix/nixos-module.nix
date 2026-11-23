# NixOS module: `programs.webox.enable = true;` installs Webox system-wide (menu entry, icons, `webox` on PATH);
# `programs.webox.autostart = true;` starts it minimized in the tray at every graphical login.
self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.webox;
in
{
  options.programs.webox = {
    enable = lib.mkEnableOption "Webox, a desktop client for Webex messaging, meetings and calls";

    package = lib.mkOption {
      type = lib.types.package;
      # NixOS has its own OpenGL drivers (hardware.graphics): the variant without the Mesa fallback.
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.webox-nixos;
      defaultText = lib.literalExpression "webox.packages.\${pkgs.stdenv.hostPlatform.system}.webox-nixos";
      description = "The Webox package to use.";
    };

    autostart = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Start Webox minimized (in the tray) when a user logs in to a graphical session (XDG autostart).";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    hardware.graphics.enable = lib.mkDefault true;

    environment.etc."xdg/autostart/webox.desktop" = lib.mkIf cfg.autostart {
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

    # File dialogs and Wayland global shortcuts go through xdg-desktop-portal; desktop environment modules enable it (not forced here: it needs a backend choice).
    # Sign-in cookies, settings and plugins live in ~/.config/webox.
  };
}
