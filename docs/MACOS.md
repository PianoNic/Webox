# Webox on macOS

macOS support is a preview: messaging, sign-in, notifications and call audio work; several desktop integrations are still missing
(list below). Builds are for Apple Silicon (arm64), macOS 11 or newer.

## Install

1. Download `webox-<version>-macos-arm64.zip` from the [latest release](https://github.com/PianoNic/Webox/releases) (the `.sha256` file
   next to it holds its checksum: `shasum -a 256 -c webox-<version>-macos-arm64.zip.sha256`).
2. Unzip it and move `Webox.app` to `/Applications`.

## First launch

Webox has no Apple Developer certificate, so the app is signed ad hoc and not notarized. Gatekeeper therefore blocks the first
start with "Webox can't be opened because Apple cannot check it for malicious software". Either

- right-click (or Control-click) `Webox.app` → **Open** → **Open**, or, on macOS 15 and newer, open it once, then
  **System Settings → Privacy & Security → Open Anyway**; or
- remove the quarantine flag in Terminal: `xattr -dr com.apple.quarantine /Applications/Webox.app`

After that it starts like any other app. macOS asks for the microphone the first time a call starts.

## Where things are

| What | Where |
| --- | --- |
| Settings, sign-in, plugins | `~/Library/Application Support/WebOx` |
| Downloads | `~/Downloads` |
| Sign-in cookies (the WebKit sign-in window) | WebKit's data store for the app, `~/Library/WebKit` |
| Plugins | `libcollabhost_<name>.dylib`; plugin repositories use the platform key `macos-aarch64` |

## What works

- Sign-in in a WebKit window (WKWebView), the same flow as WebView2 on Windows and WebKitGTK on Linux.
- Messaging, files, emoji, GIFs, plugins, the 34 languages (the system language is taken from the macOS language list).
- Notifications in Notification Center (they show as coming from "Script Editor"; a click does not open the space yet).
- Call audio through Core Audio (microphone, speakers, Smart Audio noise removal, echo cancellation).
- Opening links and files with their default app, "Show in Finder" for downloads.
- Update check: a new release is reported with a link to the download (no self-updating of the app bundle).

## Not supported yet

| Feature | What it would take |
| --- | --- |
| Menu-bar icon (tray), closing to it | `NSStatusItem`; the window's close button quits for now |
| Unread badge on the Dock icon | `NSDockTile` badge label |
| Global hotkeys | `RegisterEventHotKey` / `CGEventTap` |
| Screen sharing | ScreenCaptureKit + VideoToolbox (H.264) |
| Camera | AVFoundation (`AVCaptureSession`) |
| Playing shared videos inline | AVFoundation (`AVAssetReader`); download and open them in QuickTime meanwhile |
| Start at login | `SMAppService.mainApp` |
| Secret plugin settings in the Keychain | Security framework; they fall back to the settings file |
| Browser meeting helper (native messaging) | manifests in `~/Library/Application Support/<browser>/NativeMessagingHosts` |
| Intel Macs | an `x86_64-apple-darwin` build (the code has nothing arm64-specific) |

## Building from source

Xcode's command line tools (`xcode-select --install`) and Rust are all it needs; WebKit, Core Audio and AppKit are part of macOS.

```sh
cargo run -- --sample                   # offline, made-up data
sh packaging/macos/make-app.sh          # dist/Webox.app and dist/webox-<version>-macos-arm64.zip
```

CI builds and tests on `macos-latest` and keeps a start-up screenshot (`--sample --screenshot`) as an artifact; the
`macOS package (dry run)` workflow builds the zip without publishing anything.
