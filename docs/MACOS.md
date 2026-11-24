# Webox on macOS

One app for Apple Silicon and Intel Macs (a universal binary), macOS 12.3 or newer.

## Install

1. Download `webox-<version>-macos-universal.zip` from the [latest release](https://github.com/PianoNic/Webox/releases) (the `.sha256`
   file next to it holds its checksum: `shasum -a 256 -c webox-<version>-macos-universal.zip.sha256`).
2. Unzip it and move `Webox.app` to `/Applications`.

Later versions install themselves: the update button downloads the new zip, checks its SHA-256 and signature, replaces
`Webox.app` and restarts it (not during a call). That needs write access to the folder the app is in.

## First launch

Webox has no Apple Developer certificate, so the app is signed ad hoc and not notarized. Gatekeeper therefore blocks the first
start with "Webox can't be opened because Apple cannot check it for malicious software". Either

- right-click (or Control-click) `Webox.app` → **Open** → **Open**, or, on macOS 15 and newer, open it once, then
  **System Settings → Privacy & Security → Open Anyway**; or
- remove the quarantine flag in Terminal: `xattr -dr com.apple.quarantine /Applications/Webox.app`

macOS then asks once for each permission when it is first needed: notifications (at start), the microphone (first call), the
camera (Settings > Video or the first video call) and Screen Recording (the first screen share; Webox must be restarted after
allowing it in **System Settings → Privacy & Security → Screen Recording**).

## Where things are

| What | Where |
| --- | --- |
| Settings, sign-in, plugins | `~/Library/Application Support/WebOx` |
| Downloads | `~/Downloads` |
| Secret plugin settings | the login Keychain (service "Webox"); the settings file when the Keychain refuses |
| Start at login | System Settings → General → Login Items (or `~/Library/LaunchAgents/io.github.pianonic.webox.plist` outside the app bundle / before macOS 13) |
| Browser meeting helper | `NativeMessagingHosts` in each browser's folder under `~/Library/Application Support` |
| Plugins | `libcollabhost_<name>.dylib`; plugin repositories use the platform keys `macos-aarch64` / `macos-x86_64` |

## How it fits in

- **Menu bar:** a status item with the unread dot and count, and the same menu as the tray elsewhere. Closing the window hides
  Webox there (Settings > "close to tray"); a click on the Dock icon brings it back. The Dock icon shows the unread count and
  bounces once for a new message while Webox is in the background.
- **App menu:** Webox (Einstellungen ⌘,, Ausblenden ⌘H, Beenden ⌘Q), Bearbeiten (undo, cut, copy, paste, select all), Fenster
  (Schließen ⌘W, Im Dock ablegen ⌘M).
- **Shortcuts** use ⌘ where Windows and Linux use Ctrl, and are written the macOS way (⇧⌘N). Global hotkeys are opt-in
  (Settings > Tastenkombinationen), as on Linux.
- **Notifications** come from Webox in Notification Center; a click opens the space, reminders keep their buttons and an incoming
  call has Annehmen / Ablehnen. Started outside `Webox.app` (`cargo run`) they fall back to plain banners from "Script Editor".
- **Links:** `webexteams://` and `collabhost://` links (from the browser, a mail, "Copy space link") open the space in Webox, at
  start-up as well as while it runs. `Webox.app` declares both schemes; when the official Webex app is installed too, macOS picks
  one of them for `webexteams://`.
- **Sign-in** is a WebKit window (WKWebView), the same flow as WebView2 on Windows and WebKitGTK on Linux.
- **Media:** call audio through Core Audio, cameras and video playback through AVFoundation, screen sharing through
  ScreenCaptureKit with the H.264 encoder of VideoToolbox.

## Not supported yet

| What | Why |
| --- | --- |
| Notarization, a Developer ID signature | needs a paid Apple Developer account; hence the first-launch step above |
| Playing MPEG-TS files inline | AVFoundation doesn't read `.ts` (MP4 / MOV / M4V play) |
| A larger Dock / Finder icon than 256 px | the icon source in `assets/icon` is 256 px |

## Building from source

Xcode's command line tools (`xcode-select --install`) and Rust are all it needs; WebKit, AVFoundation, ScreenCaptureKit and
the other frameworks are part of macOS.

```sh
cargo run -- --sample                   # offline, made-up data
sh packaging/macos/make-app.sh          # dist/Webox.app and dist/webox-<version>-macos-universal.zip (both architectures)
```

The camera, notifications and the login item only work from `Webox.app` (macOS needs the bundle's Info.plist and identity for
them); a bare `cargo run` binary is ended by macOS when it asks for the camera.

CI builds and tests on `macos-latest` and keeps a start-up screenshot (`--sample --screenshot`) as an artifact; the
`macOS package (dry run)` workflow builds the universal zip without publishing anything.
