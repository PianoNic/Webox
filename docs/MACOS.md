# <img src="../assets/branding/platforms/macos.svg" width="28" height="28" alt="" /> Webox on macOS

One app for Apple Silicon and Intel Macs. Needs macOS 12.3 or newer.

## Install

1. Download `webox-<version>-macos-universal.zip` from the [latest release](https://github.com/PianoNic/Webox/releases/latest).
2. Unzip it and drag `Webox.app` into **Applications**.
3. **Right-click** `Webox.app` → **Open** → **Open**.

> [!WARNING]
> The first start shows *"Webox can't be opened because Apple cannot check it for malicious software"*.
> That's expected: Webox isn't signed with a paid Apple certificate. Step 3 gets past it, and you only do it once.
>
> **macOS 15 or newer:** open the app once, then go to **System Settings → Privacy & Security** and click **Open Anyway**.

That's it. Later versions update themselves from inside the app.

---

## More

<details>
<summary><b>Still blocked? Use Terminal</b></summary>

Remove the quarantine flag, then open the app normally:

```sh
xattr -dr com.apple.quarantine /Applications/Webox.app
```

</details>

<details>
<summary><b>Permissions macOS asks for</b></summary>

macOS asks once, the first time each one is needed:

- **Notifications**: at the first start.
- **Microphone**: at the first call.
- **Camera**: at the first video call, or in Settings > Video.
- **Screen Recording**: at the first screen share. Restart Webox after allowing it in
  **System Settings → Privacy & Security → Screen Recording**.

</details>

<details>
<summary><b>Updates</b></summary>

The update button downloads the new version, checks it, replaces `Webox.app` and restarts. It never does this during a call.

Webox needs write access to the folder it's in (for example `/Applications`).

</details>

<details>
<summary><b>Check the download (optional)</b></summary>

Each zip has a `.sha256` file next to it in the release:

```sh
shasum -a 256 -c webox-<version>-macos-universal.zip.sha256
```

</details>

<details>
<summary><b>Where Webox keeps its files</b></summary>

- Settings, sign-in and plugins: `~/Library/Application Support/WebOx`
- Downloads: `~/Downloads`
- Secret plugin settings: the login Keychain (service "Webox")
- Start at login: **System Settings → General → Login Items**

To remove Webox, move `Webox.app` to the Trash. Delete `~/Library/Application Support/WebOx` to remove your settings too.

</details>

<details>
<summary><b>Known limits</b></summary>

- No notarisation or Developer ID signature yet (needs a paid Apple Developer account). Hence the first-start step.
- MPEG-TS (`.ts`) videos don't play inline. MP4, MOV and M4V do.
- If the official Webex app is also installed, macOS picks one of the two apps for `webexteams://` links.

</details>

<details>
<summary><b>Build from source</b></summary>

You need Xcode's command line tools (`xcode-select --install`) and Rust ([rustup](https://rustup.rs)).

```sh
cargo run -- --sample               # quick test with made-up data, offline
sh packaging/macos/make-app.sh      # -> dist/Webox.app and dist/webox-<version>-macos-universal.zip
```

> [!NOTE]
> The camera, notifications and the login item only work from `Webox.app`.
> A bare `cargo run` binary is stopped by macOS when it asks for the camera.

See [dev-setup.md](dev-setup.md) for secrets and tests. How Webox fits into macOS (menu bar, Dock, shortcuts, links)
is in [platform-notes.md](platform-notes.md#macos).

</details>
