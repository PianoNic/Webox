# <img src="../assets/branding/platforms/source.svg" width="28" height="28" alt="" /> Developer setup

## Quick start

1. Install Rust 1.85 or newer with [rustup](https://rustup.rs), plus your platform's build tools (see below).
2. Clone the repository:

   ```sh
   git clone https://github.com/PianoNic/Webox.git
   cd Webox
   ```

3. Run it with made-up data (no account, no network):

   ```sh
   cargo run --release -- --sample
   ```

4. To sign in with a real account, add `secrets.toml` (see [Secrets](#secrets)) and run `cargo run --release`.

---

## Build tools

- **Windows:** the MSVC build tools. The Edge WebView2 runtime comes with Windows 10 and 11.
- **Linux:** a C toolchain, `pkg-config` and the development packages listed per distribution in
  [LINUX.md](LINUX.md) (under *Build from source*). With Nix, `nix develop` gives a shell with all of them.
- **macOS:** Xcode's command line tools (`xcode-select --install`). See [MACOS.md](MACOS.md).

## Secrets

Sign-in needs the OAuth client of the Webex desktop app; the GIF picker needs a Giphy API key.

- Put them in `secrets.toml` at the repo root. The layout is in `secrets.example.toml`.
- `build.rs` embeds the file at compile time. The next build picks up changes.
- `WEBOX_CLIENT_ID`, `WEBOX_CLIENT_SECRET` and `WEBOX_GIPHY_API_KEY` override it, both when building and when running.

Without the file the app still builds and runs. The sign-in screen says sign-in isn't configured, and the GIF picker
shows its error state.

## Files from the official app

Translations, sounds, some icons and pictures belong to Cisco. They live in `assets/cisco/`, and `cargo build` embeds them.
Without the folder Webox runs in English and silent, and draws Momentum icons instead.

To refresh them:

```powershell
pwsh scripts/restore-cisco-assets.ps1 -FromInstall        # sounds from an installed official app (Windows)
pwsh scripts/restore-cisco-assets.ps1 -FromFolder <dir>   # everything, from another assets/cisco
```

Details: [cisco-assets](cisco-assets.md).

## Run it

```sh
cargo run --release -- --sample    # offline sample data
cargo run --release                # the real app: sign in with your Webex account
cargo run -- --sample              # debug build, for stepping through code
```

Flags:

- `--sample`: made-up spaces, people and messages. Nothing leaves the machine.
- `--screenshot <file.png>`: renders, saves the window as PNG and exits.
- `--signin`: starts the sign-in right away with the saved address.
- `--minimized`: starts in the tray.

<details>
<summary><b>Screenshots of specific states</b></summary>

Use `WEBOX_SHOT_*` variables together with `--sample --screenshot`. Examples:

- `WEBOX_SHOT_OPEN=s2`: a direct chat.
- `WEBOX_SHOT_SETTINGS=notifications`: a settings page.
- `WEBOX_SHOT_THEME=light`: the light theme.
- `WEBOX_SHOT_GIF=1`: the GIF picker. With the GIF-Extras plugin: `WEBOX_SHOT_GIF_EXTRAS=1`, e.g. `WEBOX_SHOT_GIF=stickers`,
  and `WEBOX_SHOT_GIF_COMPACT=1` for its compact size.
- Message logging hooks: `WEBOX_SHOT_KEPT=<n>[,<n>]`, `WEBOX_SHOT_EDITED=<n>:<text>[;...]`, `WEBOX_SHOT_CTX=<n>`
  (the right-click menu), `WEBOX_SHOT_EDIT_DIALOG=<n>` with `WEBOX_SHOT_DIALOG_TAB=<i>`.
- `WEBOX_LANG=en`: the language.

Each variable is documented where it is read. `scripts/shots.ps1` renders a set into `shots/`.

</details>

<details>
<summary><b>Build speed</b></summary>

- While editing, `cargo check` (or rust-analyzer) reports errors in a few seconds without building the exe.
- `.cargo/config.toml` links with `rust-lld` on Windows (shipped with every toolchain), much faster than MSVC's `link.exe`.
- Debug builds compile Webox's own code unoptimised with line tables only; dependencies stay optimised, so the UI is
  still smooth. A one-file change rebuilds in about 19 s, nearly all of it compiling the `webox` crate.

</details>

## Tests

```sh
cargo test
```

The tests are offline and never write to an account.

- A few network tests are `#[ignore]`. `cargo test -- --ignored` runs them; they only read.
- The one live call test needs two explicit variables and is run by hand only (see `src/cloud/locus/`).

<details>
<summary><b>Linux in a container (Debian 13, X11 and Wayland, headless)</b></summary>

```sh
docker build -t webox-debian -f packaging/docker/Dockerfile.debian packaging/docker
docker run --rm -v "$PWD:/src" -v webox-target:/target -e CARGO_TARGET_DIR=/target -w /src webox-debian cargo test
```

The desktop integration tests (tray, notifications, hotkeys) are in [platform-notes.md](platform-notes.md#headless-tests-docker).

</details>

## Layout

- `src/`: the app, see [architecture](architecture.md).
- `plugin-api/`, `plugins/`: the plugin interface and two example plugins.
- `assets/`: icons, emoji, branding; see [cisco-assets](cisco-assets.md) for what stays local.
- `scripts/`: screenshot helpers, i18n and emoji generators, inspection tools, the private-data check.
- `packaging/`: Linux tarball, macOS app, Windows installer, Nix package and modules, Docker test images.
- `docs/`: this documentation. How features map onto each OS: [platform-notes](platform-notes.md).

## Versioning

`Cargo.toml` holds the version. Publishing a release draft tags it. The release workflow then bumps `Cargo.toml` to the
tag and attaches the Windows zip and installer, the Linux tarball and the macOS zip, each with a `.sha256` file.

## Before publishing

```powershell
pwsh scripts/check-company-data.ps1 -DenyList <path to your private deny-list>
```

See [publishing](publishing.md).
