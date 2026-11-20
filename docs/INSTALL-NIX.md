# Webox mit Nix installieren · Installing Webox with Nix

[Deutsch](#deutsch) · [English](#english)

Markierungen · markers: ✅ = im Container getestet / tested in a container (nixos/nix, Nix 2.35, nixpkgs 2026-11-13) ·
⚠️ = nicht getestet / untested. `nixos-rebuild switch` und `home-manager switch` wurden nicht ausgeführt, die
Konfigurationen aber vollständig ausgewertet / were not run, but the configurations were fully evaluated.

---

## Deutsch

### Was ist Nix, und was bekomme ich?

Nix ist ein Paketmanager, der auf jeder Linux-Distribution (Ubuntu, Fedora, Arch, …) neben dem normalen Paketmanager
läuft und Programme mit **allen** ihren Bibliotheken in `/nix/store` ablegt. Er ändert nichts an deinem System.
NixOS ist eine Linux-Distribution, die komplett mit Nix gebaut ist.

Webox wird beim ersten Mal **aus dem Quellcode gebaut** (Rust). Das dauert je nach Rechner 5–20 Minuten und braucht
etwa 3 GB Platz in `/nix/store`. Danach startet es sofort.

### 1. Nix installieren

Wenn du NixOS benutzt, ist Nix schon da: weiter mit [Schritt 2](#2-flakes-einschalten).

**Variante A: offizieller Installer** ⚠️ (braucht systemd und `sudo`)
```sh
sh <(curl --proto '=https' --tlsv1.2 -L https://nixos.org/nix/install) --daemon
```
**Variante B: Determinate-Installer** ⚠️ (schaltet Flakes gleich mit ein, lässt sich sauber wieder entfernen)
```sh
curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install
```
Danach **ein neues Terminal öffnen** (oder ab- und wieder anmelden) und prüfen:
```sh
nix --version
```

### 2. Flakes einschalten

Webox wird als *Flake* verteilt. Mit Variante B oder unter NixOS mit Flakes ist das schon erledigt. Sonst ✅:
```sh
mkdir -p ~/.config/nix
echo "experimental-features = nix-command flakes" >> ~/.config/nix/nix.conf
```
Unter NixOS stattdessen in `/etc/nixos/configuration.nix`:
```nix
nix.settings.experimental-features = [ "nix-command" "flakes" ];
```
und `sudo nixos-rebuild switch`.

### 3. Einmal ausprobieren (ohne Installation)

```sh
nix run github:PianoNic/Webox
```
Mit einer lokalen Kopie des Repositorys ✅:
```sh
git clone https://github.com/PianoNic/Webox.git ~/webox
nix run path:$HOME/webox
# oder im Ordner selbst:
cd ~/webox && nix run .
```
`path:` nimmt den Ordner so, wie er ist. `.` in einem Git-Repository sieht nur Dateien, die Git kennt
(neue Dateien vorher mit `git add` hinzufügen).

### 4. Fest installieren (für deinen Benutzer)

```sh
nix profile add github:PianoNic/Webox
```
✅ (mit `path:`). Danach liegt `webox` im `PATH`, und nach dem nächsten Anmelden steht **Webox** im Startmenü.
Ältere Nix-Versionen (vor 2.30) kennen nur `nix profile install` (gleiche Bedeutung).
Der Eintrag im Profil heisst wie das Repository (`REPO`, bei `path:` wie der Ordner); `nix profile list` zeigt ihn.

Webox startet beim Anmelden automatisch, wenn du in den Einstellungen unter *Allgemein → Anmelden*
„Webex zusammen mit meinem Computer starten“ ankreuzt (oder mit der Option `autostart` der Module unten).

### 5. NixOS: in die Systemkonfiguration aufnehmen

Wenn deine Konfiguration schon eine `flake.nix` hat (z. B. `/etc/nixos/flake.nix`), ergänze sie so ✅ (ausgewertet):
```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    webox.url = "github:PianoNic/Webox";
    webox.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { nixpkgs, webox, ... }: {
    nixosConfigurations.MEINRECHNER = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        webox.nixosModules.default
        {
          programs.webox.enable = true;
          programs.webox.autostart = true; # optional: beim Anmelden minimiert im Tray starten
        }
      ];
    };
  };
}
```
`MEINRECHNER` ist der Name deines Rechners (`hostname`). Dann:
```sh
sudo nixos-rebuild switch --flake /etc/nixos#MEINRECHNER
```
Hast du noch keine `flake.nix`: `cd /etc/nixos && sudo nix flake init -t github:NixOS/templates#full` ⚠️ erzeugt
eine Vorlage; übernimm daraus nur die Struktur oben.

Optionen: `programs.webox.enable`, `programs.webox.package` (anderes Paket), `programs.webox.autostart`.

### 6. home-manager

Mit [home-manager](https://github.com/nix-community/home-manager) als Flake ✅ (ausgewertet):
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
    homeConfigurations.DEINNAME = home-manager.lib.homeManagerConfiguration {
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      modules = [
        webox.homeManagerModules.default
        {
          home.username = "DEINNAME";
          home.homeDirectory = "/home/DEINNAME";
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
home-manager switch --flake .#DEINNAME
```
(Als NixOS-Modul von home-manager: `home-manager.users.DEINNAME.imports = [ webox.homeManagerModules.default ];`.)
Solange home-manager die Autostart-Datei verwaltet, kann der Schalter in den Webox-Einstellungen sie nicht ändern.

### 7. Aktualisieren

| Installiert mit | Befehl |
|---|---|
| `nix profile add` | `nix profile upgrade REPO` ✅ (alles: `nix profile upgrade --all`) |
| NixOS-Flake | `cd /etc/nixos && sudo nix flake update webox && sudo nixos-rebuild switch --flake .#MEINRECHNER` (✅ `nix flake update webox`) |
| home-manager | `nix flake update webox && home-manager switch --flake .#DEINNAME` |
| `nix run` | `nix run --refresh github:PianoNic/Webox` ✅ |

### 8. Deinstallieren

- Profil: `nix profile remove REPO` ✅
- NixOS / home-manager: die Zeilen wieder entfernen und neu bauen (`nixos-rebuild switch` / `home-manager switch`).
- Danach Platz freigeben: `nix-collect-garbage -d` ⚠️
- Einstellungen, Anmeldung und Plugins liegen in `~/.config/webox`; lösche den Ordner, wenn du alles entfernen willst.
- Nix selbst entfernen: Variante B mit `/nix/nix-installer uninstall` ⚠️; für Variante A siehe
  <https://nix.dev/manual/nix/latest/installation/uninstall> ⚠️.

### 9. Fehlerbehebung

**Grafik / OpenGL (Ubuntu, Fedora, … – nicht NixOS)**
Programme aus Nix können die Grafiktreiber deiner Distribution nicht laden. Darum bringt das Webox-Paket eigene
Mesa-Treiber mit und nimmt sie automatisch, wenn es kein NixOS ist ✅ (getestet mit Software-Rendering unter X11 und
Wayland): Intel- und AMD-Grafik laufen beschleunigt, sonst rechnet die CPU. Mit **NVIDIA-Treiber** läuft Webox so ohne
Beschleunigung; schneller geht es mit [nixGL](https://github.com/nix-community/nixGL) ⚠️:
```sh
WEBOX_HOST_GL=1 nix run --impure github:nix-community/nixGL#nixGLNvidia -- webox
```
(`WEBOX_HOST_GL=1` schaltet die mitgebrachten Treiber ab.) Unter NixOS nimmt das Modul eine Variante ohne die
Zusatztreiber (`webox-nixos`, etwa 800 MB kleiner); dort muss `hardware.graphics.enable = true;` gesetzt sein (bei
jeder Desktop-Umgebung der Fall).

**Wayland oder X11?** Webox läuft unter Wayland nativ, wenn `WAYLAND_DISPLAY` gesetzt ist, sonst unter X11. Um unter
Wayland trotzdem X11 (XWayland) zu erzwingen:
```sh
WAYLAND_DISPLAY= webox
```
Unterschiede: Unter Wayland erscheinen die Benachrichtigungen als normale Desktop-Benachrichtigungen (Wayland lässt
Programme ihre Fenster nicht selbst platzieren), und globale Tastenkürzel brauchen das *GlobalShortcuts*-Portal
(KDE Plasma 6, GNOME 48+). Die Arbeitsfläche fragt beim ersten Mal nach.

**Anmeldefenster bleibt weiss oder flackert** ⚠️ (NVIDIA, virtuelle Maschinen):
```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 webox
```
Das Anmeldefenster ist unter Linux ein eigenes Fenster (WebKitGTK), nicht Teil des Hauptfensters. Es braucht
Internet; Cookies bleiben in `~/.config/webox/EBWebView`.

**Kein Tray-Symbol unter GNOME:** GNOME zeigt Tray-Symbole (StatusNotifierItem) nur mit der Erweiterung
*AppIndicator and KStatusNotifierItem Support*. Ubuntu hat sie schon eingebaut. Sonst installieren
(<https://extensions.gnome.org/extension/615/appindicator-support/>) oder unter NixOS:
```nix
environment.systemPackages = [ pkgs.gnomeExtensions.appindicator ];
services.udev.packages = [ pkgs.gnome-settings-daemon ];
```
und die Erweiterung in der App *Erweiterungen* einschalten. KDE, XFCE, Cinnamon, MATE und Waybar können es von sich aus.
Ohne Tray-Symbol schliesst das X das Fenster ganz, statt es in den Tray zu legen.

**Kein Ton bei Anrufen (nicht NixOS)** ⚠️: Das Nix-Paket spricht ALSA an; auf manchen Distributionen findet es das
PipeWire-/PulseAudio-Plugin des Systems nicht. Dann den normalen Build aus [LINUX.md](LINUX.md) verwenden.

**Fehlermeldung „experimental Nix feature 'nix-command' is disabled“:** Schritt 2 fehlt.

---

## English

### What is Nix, and what do I get?

Nix is a package manager that runs on any Linux distribution (Ubuntu, Fedora, Arch, …) next to the normal one and puts
programs with **all** their libraries into `/nix/store`. It doesn't change the rest of your system. NixOS is a Linux
distribution built entirely with Nix.

The first time, Webox is **built from source** (Rust). That takes 5–20 minutes depending on the machine and about 3 GB in
`/nix/store`. After that it starts right away.

### 1. Install Nix

On NixOS, Nix is already there: continue with [step 2](#2-enable-flakes).

**Option A: official installer** ⚠️ (needs systemd and `sudo`)
```sh
sh <(curl --proto '=https' --tlsv1.2 -L https://nixos.org/nix/install) --daemon
```
**Option B: Determinate installer** ⚠️ (enables flakes for you, can be uninstalled cleanly)
```sh
curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install
```
Then **open a new terminal** (or log out and back in) and check:
```sh
nix --version
```

### 2. Enable flakes

Webox ships as a *flake*. With option B, or on NixOS with flakes already on, skip this. Otherwise ✅:
```sh
mkdir -p ~/.config/nix
echo "experimental-features = nix-command flakes" >> ~/.config/nix/nix.conf
```
On NixOS put this in `/etc/nixos/configuration.nix` instead:
```nix
nix.settings.experimental-features = [ "nix-command" "flakes" ];
```
and run `sudo nixos-rebuild switch`.

### 3. Try it once (no installation)

```sh
nix run github:PianoNic/Webox
```
From a local copy of the repository ✅:
```sh
git clone https://github.com/PianoNic/Webox.git ~/webox
nix run path:$HOME/webox
# or inside the folder:
cd ~/webox && nix run .
```
`path:` takes the folder as it is. `.` inside a Git repository only sees files Git knows about (`git add` new files first).

### 4. Install it for your user

```sh
nix profile add github:PianoNic/Webox
```
✅ (with `path:`). `webox` is then on your `PATH`, and **Webox** shows up in the application menu after your next login.
Older Nix versions (before 2.30) only know `nix profile install` (same meaning).
The profile entry is named after the repository (`REPO`; with `path:` after the folder); `nix profile list` shows it.

To start Webox at login, tick "Webex zusammen mit meinem Computer starten" under *Allgemein → Anmelden* (General →
Sign in; the app's settings are in German), or use the modules' `autostart` option below.

### 5. NixOS: add it to the system configuration

If your configuration already has a `flake.nix` (e.g. `/etc/nixos/flake.nix`), extend it like this ✅ (evaluated):
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
          programs.webox.autostart = true; # optional: start minimized in the tray at login
        }
      ];
    };
  };
}
```
`MYHOST` is your machine's name (`hostname`). Then:
```sh
sudo nixos-rebuild switch --flake /etc/nixos#MYHOST
```
No `flake.nix` yet: `cd /etc/nixos && sudo nix flake init -t github:NixOS/templates#full` ⚠️ creates a template; only take
the structure shown above from it.

Options: `programs.webox.enable`, `programs.webox.package` (a different package), `programs.webox.autostart`.

### 6. home-manager

With [home-manager](https://github.com/nix-community/home-manager) as a flake ✅ (evaluated):
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
(As home-manager's NixOS module: `home-manager.users.YOURNAME.imports = [ webox.homeManagerModules.default ];`.)
While home-manager manages the autostart file, the switch in Webox's settings can't change it.

### 7. Update

| Installed with | Command |
|---|---|
| `nix profile add` | `nix profile upgrade REPO` ✅ (everything: `nix profile upgrade --all`) |
| NixOS flake | `cd /etc/nixos && sudo nix flake update webox && sudo nixos-rebuild switch --flake .#MYHOST` (✅ `nix flake update webox`) |
| home-manager | `nix flake update webox && home-manager switch --flake .#YOURNAME` |
| `nix run` | `nix run --refresh github:PianoNic/Webox` ✅ |

### 8. Uninstall

- Profile: `nix profile remove REPO` ✅
- NixOS / home-manager: remove the lines again and rebuild (`nixos-rebuild switch` / `home-manager switch`).
- Then free the space: `nix-collect-garbage -d` ⚠️
- Settings, sign-in and plugins live in `~/.config/webox`; delete that folder to remove everything.
- Removing Nix itself: option B with `/nix/nix-installer uninstall` ⚠️; for option A see
  <https://nix.dev/manual/nix/latest/installation/uninstall> ⚠️.

### 9. Troubleshooting

**Graphics / OpenGL (Ubuntu, Fedora, … – not NixOS)**
Programs from Nix can't load your distribution's graphics drivers. So the Webox package brings its own Mesa drivers and
uses them automatically when it isn't running on NixOS ✅ (tested with software rendering on X11 and Wayland): Intel and
AMD graphics are accelerated, otherwise the CPU renders. With the **NVIDIA driver** Webox runs unaccelerated this way;
for speed use [nixGL](https://github.com/nix-community/nixGL) ⚠️:
```sh
WEBOX_HOST_GL=1 nix run --impure github:nix-community/nixGL#nixGLNvidia -- webox
```
(`WEBOX_HOST_GL=1` turns the bundled drivers off.) On NixOS the module uses a variant without those extra drivers
(`webox-nixos`, about 800 MB smaller); `hardware.graphics.enable = true;` must be set there (every desktop environment
does that).

**Wayland or X11?** Webox runs natively on Wayland when `WAYLAND_DISPLAY` is set, otherwise on X11. To force X11
(XWayland) on a Wayland desktop:
```sh
WAYLAND_DISPLAY= webox
```
Differences: on Wayland, notifications appear as regular desktop notifications (Wayland doesn't let programs place
their own windows), and global shortcuts need the *GlobalShortcuts* portal (KDE Plasma 6, GNOME 48+). The desktop asks
you to confirm the first time.

**Sign-in window stays white or flickers** ⚠️ (NVIDIA, virtual machines):
```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 webox
```
On Linux the sign-in is a separate window (WebKitGTK), not part of the main window. It needs internet access; cookies
stay in `~/.config/webox/EBWebView`.

**No tray icon on GNOME:** GNOME only shows tray icons (StatusNotifierItem) with the *AppIndicator and KStatusNotifierItem
Support* extension. Ubuntu ships it. Otherwise install it (<https://extensions.gnome.org/extension/615/appindicator-support/>),
or on NixOS:
```nix
environment.systemPackages = [ pkgs.gnomeExtensions.appindicator ];
services.udev.packages = [ pkgs.gnome-settings-daemon ];
```
and enable it in the *Extensions* app. KDE, XFCE, Cinnamon, MATE and Waybar support it out of the box.
Without a tray icon, the X closes the window instead of hiding it in the tray.

**No sound in calls (not NixOS)** ⚠️: the Nix package talks to ALSA; on some distributions it can't find the system's
PipeWire / PulseAudio plugin. Use the regular build from [LINUX.md](LINUX.md) then.

**"experimental Nix feature 'nix-command' is disabled":** step 2 is missing.
