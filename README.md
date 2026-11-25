<p align="center">
  <img src="assets/branding/webox.svg" width="180" alt="Webox Logo" />
</p>
<p align="center">
  <strong>Webox</strong><br/>
  A fast, native Webex desktop client for Windows, macOS and Linux.
</p>
<p align="center">
  <a href="https://github.com/PianoNic/Webox"><img src="https://badgetrack.pianonic.ch/badge?tag=webox&label=visits&color=07C1F5&style=flat" alt="visits" /></a>
  <a href="https://github.com/PianoNic/Webox/releases"><img src="https://img.shields.io/github/v/release/PianoNic/Webox?include_prereleases&color=07C1F5&label=Latest%20Release" alt="Latest release" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-07C1F5.svg" alt="MIT license" /></a>
</p>

> [!IMPORTANT]
> Webox is **NOT** affiliated with, endorsed by, or connected to Cisco in any way. Webex and Cisco are trademarks of Cisco Systems, Inc.

---

## What is Webox?

The official Webex app is a big Qt and Chromium bundle. Webox is one small Rust binary that draws the same interface with [egui](https://github.com/emilk/egui) and talks to the same Webex cloud. Sign in with your normal Webex account: same spaces, messages and calls.

<p align="center">
  <img src="assets/branding/screenshot.png" width="800" alt="Webox showing a space with messages" />
</p>

## Why Webox?

- 🪶 **Light**: one small native app, quick to start and quiet in the background
- 🕵️ **No tracking**: no usage data or metrics, only what Webex needs to work
- 💻 **Windows, macOS and Linux**: including Wayland and Nix
- 🎨 **Make it yours**: hide, show and reorder buttons and menus, themes, compact mode
- 🧩 **Plugins**: message logger, auto-reply, video effects and more ([write your own](docs/plugins.md))

## Get started

- <img src="assets/branding/platforms/windows.svg" width="16" height="16" alt="Windows" />&nbsp;&nbsp;**Windows**: run the installer from the [latest release](https://github.com/PianoNic/Webox/releases).
- <img src="assets/branding/platforms/linux.svg" width="16" height="16" alt="Linux" />&nbsp;&nbsp;**Linux**: unpack and run `install.sh` ([details](docs/LINUX.md)).
- <img src="assets/branding/platforms/macos.svg" width="16" height="16" alt="macOS" />&nbsp;&nbsp;**macOS**: unzip and move `Webox.app` to Applications ([details](docs/MACOS.md)).
- <img src="assets/branding/platforms/nixos.svg" width="16" height="16" alt="Nix" />&nbsp;&nbsp;**Nix**: `nix run github:PianoNic/Webox` ([modules](docs/INSTALL-NIX.md)).
- <img src="assets/branding/platforms/source.svg" width="16" height="16" alt="Source" />&nbsp;&nbsp;**From source**: [developer setup](docs/dev-setup.md).

## License

MIT. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

---
<p align="center">Made with ❤️ by <a href="https://github.com/PianoNic">PianoNic</a></p>
