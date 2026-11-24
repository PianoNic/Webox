# Developer setup

## Prerequisites

- Rust 1.85 or newer ([rustup](https://rustup.rs))
- Windows: the MSVC build tools and the Edge WebView2 runtime (Windows 10 and 11 have it)
- Linux: a C toolchain, `pkg-config`, WebKitGTK 4.1, GTK 3 and ALSA development packages, listed per distribution in [LINUX.md](LINUX.md). With Nix, `nix develop` gives a shell with all of them.

## Secrets

Sign-in needs the OAuth client of the Webex desktop app and the GIF picker a Giphy API key. They are in `secrets.toml` at the repo root (the layout is in `secrets.example.toml`).

`build.rs` embeds the file at compile time. The variables `WEBOX_CLIENT_ID`, `WEBOX_CLIENT_SECRET` and `WEBOX_GIPHY_API_KEY` override it, both when building and when running. After creating or deleting `secrets.toml`, run `cargo clean -p webox` once so the build script runs again; edits to an existing file are picked up on their own.

Without the file the app builds and runs: the sign-in screen says sign-in is not configured, and the GIF picker shows its error state.

## Files from the official app

Translations, sounds, some icons and pictures belong to Cisco. They are in `assets/cisco/`, and `cargo build` embeds them. To refresh them:

```powershell
pwsh scripts/restore-cisco-assets.ps1 -FromInstall        # the sounds from an installed official app (Windows)
pwsh scripts/restore-cisco-assets.ps1 -FromFolder <dir>   # everything, from another assets/cisco
```

Without the folder Webox runs in English and silent, and draws Momentum icons where the app has its own. Details: [cisco-assets](cisco-assets.md).

## Run it

```bash
cargo run --release -- --sample    # offline sample data, no account, no network
cargo run --release                # the real app: sign in with your Webex account
cargo run -- --sample              # debug build, for stepping through code
```

| Flag | Does |
| --- | --- |
| `--sample` | made-up spaces, people and messages, nothing leaves the machine |
| `--screenshot <file.png>` | renders, saves the window as PNG and exits |
| `--signin` | starts the sign-in right away with the saved address |
| `--minimized` | starts in the tray |

Screenshots of specific states use `WEBOX_SHOT_*` variables together with `--sample --screenshot`, for example `WEBOX_SHOT_OPEN=s2` (a direct chat), `WEBOX_SHOT_SETTINGS=notifications`, `WEBOX_SHOT_GIF=1` (the GIF picker; with `WEBOX_SHOT_GIF_EXTRAS=1` the GIF-Extras plugin's popup, e.g. `WEBOX_SHOT_GIF=stickers`, and `WEBOX_SHOT_GIF_COMPACT=1` for its compact size) or `WEBOX_SHOT_THEME=light`. `WEBOX_LANG=en` picks the language. Each variable is documented where it is read; `scripts/shots.ps1` renders a set into `shots/`.

## Build speed

- While editing, `cargo check` (or rust-analyzer in the editor) reports errors in a few seconds without building the exe.
- `.cargo/config.toml` links with `rust-lld` on Windows (shipped with every toolchain), much faster than MSVC's `link.exe`.
- Debug builds compile Webox's own code unoptimised and with line tables only; the dependencies stay optimised, so the UI is
  still smooth. A one-file change rebuilds in about 19 s (was 22 s); nearly all of it is compiling the `webox` crate.

## Tests

```bash
cargo test
```

The tests are offline and never write to an account. A few network tests are `#[ignore]`: `cargo test -- --ignored` runs them, and they only read. The one live call test needs two explicit variables and is run by hand only (see `src/cloud/locus/`).

Linux in a container (Debian 13, X11 and Wayland, headless):

```bash
docker build -t webox-debian -f packaging/docker/Dockerfile.debian packaging/docker
docker run --rm -v "$PWD:/src" -v webox-target:/target -e CARGO_TARGET_DIR=/target -w /src webox-debian cargo test
```

## Layout

| Path | Holds |
| --- | --- |
| `src/` | the app, see [architecture](architecture.md) |
| `plugin-api/`, `plugins/` | the plugin interface and two example plugins |
| `assets/` | icons, emoji, branding; see [cisco-assets](cisco-assets.md) for what stays local |
| `scripts/` | screenshot helpers, i18n and emoji generators, inspection tools, the private-data check |
| `packaging/` | Linux tarball, Nix package and modules, Docker test images |
| `docs/` | this documentation |

## Versioning

`Cargo.toml` holds the version. Publishing a release draft tags it, and the release workflow bumps `Cargo.toml` to the tag and attaches the Windows zip and the Linux tarball.

## Before publishing

```powershell
pwsh scripts/check-company-data.ps1 -DenyList <path to your private deny-list>
```

See [publishing](publishing.md).
