<p align="center">
  <img src="assets/branding/webox.svg" width="180" alt="Webox Logo" />
</p>
<p align="center">
  <strong>Webox</strong><br/>
  A fast, native Webex desktop client for Windows and Linux, written in Rust. 
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

## Features

- 💬 **Messaging**: spaces, threads, reactions, mentions, edits and rich text, end-to-end encrypted like the official app
- 📎 **Files, emoji and GIFs**: inline images, link previews, emoji and GIF pickers
- 📞 **Calls and meetings**: 1:1 calls, screen share, upcoming meetings
- 🔔 **Notifications**: toasts, tray badge, do-not-disturb, the official keyboard shortcuts
- 🌍 **34 languages**, from the official app's translations
- 🧩 **Plugins**: a native plugin API with installable plugin repositories ([docs](docs/plugins.md))

## Get started

- 🪟 **Windows**: run the installer from the [latest release](https://github.com/PianoNic/Webox/releases).
- 🐧 **Linux**: unpack `webox-<version>-linux-x86_64.tar.gz` and run `install.sh` ([libraries](docs/LINUX.md)).
- ❄️ **Nix**: `nix run github:PianoNic/Webox` ([modules](docs/INSTALL-NIX.md)).
- 🛠️ **From source**: [developer setup](docs/dev-setup.md).

## License

MIT. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

---
<p align="center">Made with ❤️ by <a href="https://github.com/PianoNic">PianoNic</a></p>
