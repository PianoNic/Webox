# <img src="../assets/branding/platforms/nixos.svg" width="28" height="28" alt="" /> Webox with Nix

Works on NixOS and on any Linux distribution with Nix. On NixOS, see [NixOS module](#nixos-module) below.

## Install

1. Install Nix (skip this if you have it). This installer also turns on flakes:

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install
   ```

2. Open a new terminal.

3. Install Webox:

   ```sh
   nix profile add github:PianoNic/Webox
   ```

4. Run `webox`. After your next login, **Webox** is also in your application menu.

> [!NOTE]
> The first install builds Webox from source. That takes 5–20 minutes and about 3 GB in `/nix/store`.

Just want to try it without installing? `nix run github:PianoNic/Webox`

---

## More

<details>
<summary><b>Got "experimental Nix feature 'nix-command' is disabled"?</b></summary>

Flakes are off. Turn them on:

```sh
mkdir -p ~/.config/nix
echo "experimental-features = nix-command flakes" >> ~/.config/nix/nix.conf
```

On NixOS, put this in `/etc/nixos/configuration.nix` and run `sudo nixos-rebuild switch`:

```nix
nix.settings.experimental-features = [ "nix-command" "flakes" ];
```

</details>

<details>
<summary><b>Other ways to install Nix</b></summary>

The official installer (needs systemd and `sudo`, doesn't turn on flakes):

```sh
sh <(curl --proto '=https' --tlsv1.2 -L https://nixos.org/nix/install) --daemon
```

Then turn on flakes as shown above. Check with `nix --version`.

</details>

<details>
<summary><b>Older Nix (before 2.30)</b></summary>

Use `nix profile install` instead of `nix profile add`. It does the same.

</details>

### NixOS module

Add Webox to your system flake (for example `/etc/nixos/flake.nix`):

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    webox.url = "github:PianoNic/Webox";
    webox.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { nixpkgs, webox, ... }: {
    nixosConfigurations.MYHOST = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        webox.nixosModules.default
        {
          programs.webox.enable = true;
          programs.webox.autostart = true; # optional
        }
      ];
    };
  };
}
```

Replace `MYHOST` with your machine's name (`hostname`), then:

```sh
sudo nixos-rebuild switch --flake /etc/nixos#MYHOST
```

<details>
<summary><b>Module options</b></summary>

- `programs.webox.enable`: install Webox for all users.
- `programs.webox.autostart`: start Webox minimised in the tray at every graphical login. Default `false`.
- `programs.webox.package`: the package to use. Default `webox.packages.<system>.webox-nixos`.

The module also sets `hardware.graphics.enable = true` (as a default you can override).

</details>

<details>
<summary><b>No <code>flake.nix</code> yet?</b></summary>

This creates a template (untested). Copy only the structure shown above from it:

```sh
cd /etc/nixos && sudo nix flake init -t github:NixOS/templates#full
```

</details>

### Home Manager

<details>
<summary><b>Standalone Home Manager flake</b></summary>

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
    webox.url = "github:PianoNic/Webox";
    webox.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { nixpkgs, home-manager, webox, ... }: {
    homeConfigurations.YOURNAME = home-manager.lib.homeManagerConfiguration {
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      modules = [
        webox.homeManagerModules.default
        {
          home.username = "YOURNAME";
          home.homeDirectory = "/home/YOURNAME";
          home.stateVersion = "25.05";
          programs.webox.enable = true;
          programs.webox.autostart = true; # optional
        }
      ];
    };
  };
}
```

```sh
home-manager switch --flake .#YOURNAME
```

</details>

<details>
<summary><b>Home Manager as a NixOS module</b></summary>

```nix
home-manager.users.YOURNAME.imports = [ webox.homeManagerModules.default ];
```

Then set `programs.webox.enable = true;` for that user.

</details>

<details>
<summary><b>Home Manager options</b></summary>

Same names as the NixOS module: `programs.webox.enable`, `programs.webox.autostart`, `programs.webox.package`.

- `autostart` writes `~/.config/autostart/webox.desktop`. While Home Manager owns that file, the
  "Start Webox when my computer starts" switch in Webox's settings can't change it.
- `package` defaults to `webox-nixos` when Home Manager runs as a NixOS module, otherwise to `default`.

</details>

### Start at login

Tick **Start Webox when my computer starts** in Webox's settings (General), or use the `autostart` option above.

### Update

- `nix profile`: `nix profile upgrade Webox` (or `nix profile upgrade --all`)
- NixOS: `cd /etc/nixos && sudo nix flake update webox && sudo nixos-rebuild switch --flake .#MYHOST`
- Home Manager: `nix flake update webox && home-manager switch --flake .#YOURNAME`
- `nix run`: `nix run --refresh github:PianoNic/Webox`

<details>
<summary><b>The profile entry isn't called "Webox"?</b></summary>

The entry is named after the repository, or after the folder when you installed from a local path. `nix profile list`
shows the name.

</details>

### Uninstall

- `nix profile`: `nix profile remove Webox`
- NixOS / Home Manager: remove the lines again and rebuild.
- Free the space afterwards: `nix-collect-garbage -d`
- Settings, sign-in and plugins live in `~/.config/webox`. Delete that folder to remove everything.

<details>
<summary><b>Remove Nix itself</b></summary>

- Determinate installer: `/nix/nix-installer uninstall`
- Official installer: see <https://nix.dev/manual/nix/latest/installation/uninstall>

</details>

### Troubleshooting

<details>
<summary><b>Slow graphics with an NVIDIA driver (not NixOS)</b></summary>

Off NixOS, the package brings its own Mesa drivers: Intel and AMD are accelerated, NVIDIA falls back to the CPU.
For NVIDIA acceleration use [nixGL](https://github.com/nix-community/nixGL) (untested):

```sh
WEBOX_HOST_GL=1 nix run --impure github:nix-community/nixGL#nixGLNvidia -- webox
```

`WEBOX_HOST_GL=1` turns the bundled drivers off. On NixOS this isn't needed; the module uses `webox-nixos`, which
leaves them out (about 800 MB smaller).

</details>

<details>
<summary><b>Sign-in window is blank or flickers</b></summary>

Happens with NVIDIA and in some VMs:

```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 webox
```

</details>

<details>
<summary><b>No tray icon on GNOME</b></summary>

GNOME needs the [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/) (Ubuntu has it
already). On NixOS:

```nix
environment.systemPackages = [ pkgs.gnomeExtensions.appindicator ];
services.udev.packages = [ pkgs.gnome-settings-daemon ];
```

Then enable it in the *Extensions* app. KDE, XFCE, Cinnamon, MATE and Waybar work out of the box.
Without a tray icon, closing the window quits Webox.

</details>

<details>
<summary><b>Force X11 on a Wayland desktop</b></summary>

```sh
WAYLAND_DISPLAY= webox
```

On Wayland, notifications are normal desktop notifications, and global shortcuts need the GlobalShortcuts portal
(KDE Plasma 6, GNOME 48+). The desktop asks you to confirm once.

</details>

<details>
<summary><b>No sound in calls (not NixOS)</b></summary>

On some distributions the Nix package can't find the system's PipeWire / PulseAudio plugin for ALSA (untested).
Use the regular install from [LINUX.md](LINUX.md) instead.

</details>

<details>
<summary><b>Run from a local clone</b></summary>

```sh
git clone https://github.com/PianoNic/Webox.git ~/webox
nix run path:$HOME/webox
```

`path:` takes the folder as it is. `nix run .` inside the folder only sees files Git knows about (`git add` new files first).

</details>
