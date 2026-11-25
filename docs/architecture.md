# Architecture

Webox is one Rust binary built on [egui/eframe](https://github.com/emilk/egui). It talks to the same Webex cloud services the Webex desktop client uses and follows the Webex desktop layout.

## Source layout

Only `src/main.rs` sits at the root of `src/`; everything else lives in an area folder. Every folder is a module with a `mod.rs`, and no `foo.rs` sits next to a `foo/` anywhere in the tree.

| Path | Holds |
| --- | --- |
| `src/main.rs` | the window, the `App` struct, fonts and `main` |
| `src/ui/theme/` | the themes: `ThemeId` (the Momentum themes), `Th` built from colour tokens, `t.tk("token:state")`; `tokens.rs` is a generated table of the resolved colour tokens (do not edit by hand) |
| `src/app/` | app state and the frame loop (`mod.rs`), actions (`actions.rs`), sign-in and tokens (`signin.rs`), the live space list (`live_list.rs`), upload / download state shared with the cloud thread (`files.rs`) |
| `src/cloud/` | the Webex protocol: U2C catalog, device registration, Mercury websocket, conversations, KMS encryption, people and presence, locus (calls); OAuth sign-in on idbroker and token refresh (`auth.rs`), the build-time client credentials (`secrets.rs`), JOSE/JWE for KMS (`jose.rs`), markdown to message HTML (`richtext.rs`) |
| `src/ui/` | every screen and widget: rail, space list, messages, composer, pickers, settings, call windows, the presence line |
| `src/media/` | call audio devices and pipeline (`audio.rs`), the receive-side jitter buffer of call audio (`jitter.rs`), notification and ringtone sounds (`sounds.rs`) |
| `src/platform/` | OS integration: settings folder, arguments, links, host log (`system.rs`), tray and taskbar (`tray/`), toast windows (`notify/`), the embedded sign-in webview (WebView2 on Windows, WebKitGTK on Linux, `webview/`), the browser's native messaging host (`nmhost.rs`), app updates from the GitHub releases (`updater/`), the Linux D-Bus / X11 pieces (`linux/`) |
| `src/assets/` | icons and emoji (`icons.rs`), the locally kept Webex client files (`cisco_assets.rs`), decoded profile pictures and message images (`photos/`), translations from the embedded language packs (`i18n/`) |
| `src/extras/` | the Giphy client (`giphy.rs`), the built-in plugin "GIF-Extras" (`gif_extras/`: extended GIF picker, stickers, favourites, star chips; reached only through the hook in `src/ui/gif_picker/ext.rs`, off by default), the native plugin host and plugin list (`plugins/`), plugin repositories: index, download, install store (`plugin_repo/`, see `docs/plugins.md`) |
| `src/shortcuts/` | keyboard shortcuts and global hotkeys |
| `plugin-api/`, `plugins/*` | the plugin ABI and two example plugins (workspace members) |
| `build.rs` | embeds the icon and emoji tables and the optional `secrets.toml` |

`system`, `icons`, `i18n`, `jose` and `richtext` are used all over the crate, so `main.rs` re-exports them at the root (`crate::system`, `crate::i18n`, ...; the `tr!` macros expand to `$crate::i18n::...`). Every other module is reached by its full path, e.g. `crate::cloud::auth`, `crate::platform::tray`, `crate::media::audio`.

## Launcher behaviour (`src/platform/system.rs`)

- **Host log:** every start writes `webox-host-log.txt` in the settings folder: the command line (without `/skip-de-elevation`), the start mode and, for every loaded plugin library, a `<file> version: <version>` line. The log is UTF-16LE without a byte-order mark, capped at 32768 UTF-16 units; every non-empty line starts with a UTC time stamp `yyyy-MM-dd HH:mm:ss.mmm ` and ends with CRLF. The local data folder and the user name are masked with `*`.
- **Autostart:** `/minimized /autostartedWithWindows=true` (the Run key on Windows, an XDG autostart entry on Linux).
- **Side-loading protection:** the policy `enableDllSideLoadingProtection` (1 = on) is read from `HKLM\SOFTWARE\Policies\WebOx` on Windows and `/etc/webox/policy.json` on Linux; unset means off. While it is on, plugins outside the program folder are not loaded (`src/extras/plugins/`).

## Sign-in (`src/cloud/auth.rs`, `src/platform/webview/`)

- **Client credentials:** the desktop OAuth client's `client_id` and `client_secret` are not in this repository: put them in `secrets.toml` (see [dev-setup](dev-setup.md)). `redirect_uri` is `urn:ietf:wg:oauth:2.0:oob`.
- **IDBroker endpoints** (on the account's home cluster, found with the limited U2C catalog):
  - Authorize: `/idb/oauth2/v1/authorize?cisKeepMeSignedInOption=1&isHideAuthzCode=true` (PKCE S256, only the hashes of the e-mail and its domain are sent)
  - Token: `/idb/oauth2/v1/access_token?self_contained_token=true&token_version=2`
  - Revoke: `/idb/oauth2/v1/revoke`
- **Code exchange:** `Authorization: Basic base64(client_id + ":" + client_secret)`; the form body carries `grant_type=authorization_code`, `code`, `redirect_uri`, `code_verifier` and `device_id`, never the client credentials.
- **Flow:** the embedded webview (`SCRAPER`, `doExchange`, `ExchangeCfg`) captures the one-time code; `exchange`, `refresh` and `revoke` in `auth.rs` talk to the token endpoint. Without a webview the browser + paste path is used.
