# Plugins and plugin repositories

Webox plugins are native libraries (`.dll` on Windows, `.so` on Linux) built against the `collabhost-plugin-api` crate in
`plugin-api/`. A plugin implements the `Plugin` trait and calls `export_plugin!`. Webox loads plugins from three places:

| Source | Where | Shown as |
|---|---|---|
| Repository install | `%APPDATA%\WebOx\plugins\<pkg>\<version>\<file>` (Linux: `$XDG_DATA_HOME/webox/plugins/...`, default `~/.local/share/webox/plugins`) | "Aus Repository: ...", can be updated and uninstalled |
| Plugin folder | `%APPDATA%\WebOx\plugins\*.dll` (Linux: `~/.config/webox/plugins/*.so`), `COLLABHOST_PLUGIN_PATH` | "Manuell installiert" |
| Program folder | `plugins/` next to `webox.exe`, `../lib/collabhost/plugins` (Nix), `collabhost_*` next to the exe (cargo output) | "Mitgeliefert" |

With the side-loading policy on (`enableDllSideLoadingProtection`, see `docs/architecture.md`) only the program folder loads,
and installing from repositories is switched off.

The example plugins live in two places: `plugins/slash-commands` and `plugins/pride-theme` in this repository stay as
workspace members, because Webox's own tests load them (`cargo build` then `cargo test`) and must build them against the
plugin API of the same commit. The published copies, the ones users install, live in the plugins monorepo (`WeboxPlugins`),
which builds them in CI against a pinned plugin-api.

## Plugin API contract

`plugin-api` carries an explicit contract version `CONTRACT_MAJOR.CONTRACT_MINOR` (currently **1.2**, `CONTRACT` as a string). History: 1.0 the first interface; 1.1 the contract symbol, nullable function pointers and panic-safe shims; 1.2 plugin settings; 1.3 accent themes.

- A plugin exports `collabhost_plugin_contract() -> u32` (`(major << 16) | minor`, added by `export_plugin!`). Plugins
  built before 1.1 have no such symbol and count as 1.0.
- Webox refuses a plugin built for another **major** version, or for a **newer minor** version than its own.
- The repository index repeats the value as `contract`, so incompatible plugins are greyed out before anything is downloaded.

Bump the minor version when you add something to the end of `HostApi` / `PluginVTable` that older hosts do not offer. Bump the
major version (and `ABI_VERSION` with it) when a field is removed, reordered or retyped.

What keeps the boundary stable:

- Only `#[repr(C)]` structs, integers, raw pointers, NUL-terminated UTF-8 strings and `extern "C"` function pointers cross it.
  No `String`, `Vec`, `Box`, `bool`, enums or trait objects.
- Function pointers are `Option<extern "C" fn ...>` (the same layout as a plain pointer; null is `None`). Webox refuses a
  table with a missing entry instead of calling a null pointer.
- Each side frees what it allocated: strings from a plugin go back through the plugin's `free_str`, strings from the host
  through the host's.
- Panics never unwind across the boundary. The plugin-side shims catch them (the hook then reports "no change"), and so do
  the host callbacks. A panic in an `extern "C"` function would otherwise abort Webox.
- A plugin must not keep threads running after `stop` returns. Webox may unload the library afterwards (uninstall, update,
  "Neu laden").

## Plugin settings

Since contract 1.2 a plugin can have settings. It declares them as a schema, and Webox draws the form on the plugin's own
page (Settings > Plugins > a plugin > "Einstellungen"). Webox validates and saves the values and passes them to the
plugin. No UI code crosses the boundary, only JSON strings. Built-in plugins (GIF-Extras) declare their settings in the
same format, so their pages look the same as those of installed plugins.

Plugin side (plugin-api):

```rust
use collabhost_plugin_api::{Schema, Values};

fn settings(&self) -> Option<Schema> {
    Some(Schema::new()
        .section("Ersetzungen", "Was beim Senden ersetzt wird.")
        .toggle("smiley", "„:)“ ersetzen", "", true)
        .text("shrug_text", "Text für /shrug", "", "¯\\_(ツ)_/¯")
        .number("delay", "Verzögerung (s)", "", 2.0, 0.0, 10.0, 0.5)
        .choice("mode", "Modus", "", "a", &[("a", "Einfach"), ("b", "Genau")])
        .multiline("notes", "Notizen", "", "")
        .password("token", "API-Schlüssel", "Wird im Schlüsselbund gespeichert.")
        .button("reset", "Zähler zurücksetzen", ""))
}
fn settings_changed(&mut self, v: &Values) { self.smiley = v.bool("smiley").unwrap_or(true); }
fn settings_action(&mut self, id: &str) -> Option<String> { (id == "reset").then(|| "Zurückgesetzt".into()) }
```

`host.settings()` returns the same values at any time.

ABI (contract 1.2, appended so older tables stay valid):

| Where | Entry | Meaning |
|---|---|---|
| `PluginVTable` | `settings_schema(inst) -> *mut c_char` | schema JSON (freed with the plugin's `free_str`); null = no settings |
| `PluginVTable` | `settings_changed(inst, json)` | all current values as a flat JSON object. Called after every change and once before every `start`. |
| `PluginVTable` | `settings_action(inst, id) -> *mut c_char` | a button was pressed; an optional message to show, or null |
| `HostApi` | `settings_get(ctx) -> *mut c_char` | the current values (freed with the host's `free_str`) |

Webox reads the three vtable entries only from plugins that report contract 1.2 or newer. Tables of older plugins end
before them. A plugin without a schema simply has no settings.

Schema JSON (`version` 1):

```json
{ "version": 1, "sections": [
  { "title": "Ersetzungen", "description": "...", "fields": [
    { "type": "bool", "id": "smiley", "label": "...", "description": "...", "default": true },
    { "type": "string", "id": "shrug_text", "label": "...", "default": "..." },
    { "type": "multiline", "id": "notes", "label": "...", "default": "" },
    { "type": "password", "id": "token", "label": "..." },
    { "type": "number", "id": "delay", "label": "...", "default": 2, "min": 0, "max": 10, "step": 0.5 },
    { "type": "choice", "id": "mode", "label": "...", "default": "a", "options": [ { "value": "a", "label": "Einfach" } ] },
    { "type": "button", "id": "reset", "label": "..." }
  ] } ] }
```

A top-level `"fields": [...]` without sections is accepted too. Rules:

- `id`: letters, digits, `_`, `.` and `-`, at most 64 characters, unique. `label` is required. At most 64 fields.
- A field with an unknown type or bad values is left out (the rest still shows).
- Number defaults are clamped into `min..max`. A choice default that is not one of the options becomes the first option.
- Validation before saving: bool must be a bool. string has no line breaks and at most 1000 characters. multiline allows
  20,000 characters, password 4096. number must lie within `min..max` and on the `step` grid. choice must be one of the
  options. A refused value is not saved, and the reason appears under the field.
- The plugin always receives every field (buttons excluded). A stored value that no longer validates, for example after a
  schema change, is replaced by the default. An unset password is `""`.

Storage: `<config>/plugin-settings/<plugin key>.json` (`{"values": {...}}`). Passwords go to the OS keyring: the Windows
Credential Manager (generic credential `Webox/plugin/<key>/<id>`) or, on Linux, the Secret Service through `secret-tool`.
The file records only that a keyring entry exists. Without a keyring, the value is stored in the file under
`fileSecrets`, and the page says it is not encrypted. `--sample` runs keep passwords in memory only.

The settings search (top of the Settings navigation) also finds plugin settings by label, description, section and
plugin name. A hit opens that plugin's page.

## Design plugins: palettes and accent themes

A design plugin either replaces the base theme's main colours (`palette()`, contract 1.0: backgrounds, text, accent,
links) or, since contract 1.3, only its accents (`accents(&ThemeInfo)`). Backgrounds and body text then stay the base
theme's, so an accent theme works on all ten themes, dark and light. The example is `plugins/pride-theme`.

Webox calls `accents` with the active base theme (`ThemeInfo`: `light`, `background` = the content cards, `surface` = the
window, `text` = primary text, as 0xRRGGBBAA) and uses what comes back. A field left at 0 keeps Webox's colour. The plugin
picks readable values itself: `collabhost_plugin_api::color` has WCAG helpers (`contrast`, `readable(c, bg, min)`,
`fill_for_text(c, min)`, `text_on(fill)`, `mix`). Aim for 4.5:1 for text and text on fills, 3:1 for dots, rings and control
fills.

| `Accents` field | Where Webox uses it |
|---|---|
| `accent` | toggles, the composer's focus line when there is no gradient, small accents |
| `link` | links (`Th::link`) |
| `focus` | keyboard focus ring, the outline of a focused text field |
| `selection` | text selection (drawn at 40 %) |
| `unread` | unread dots in the lists |
| `badge` (`badge_text`) | the badge dot on the rail |
| `rail_selected` | the circle behind the selected rail item (the icon on it is drawn in the primary text colour) |
| `button` / `button_text` | primary (filled) buttons, with hover and pressed shades derived from them |
| `control` / `control_icon` | checked checkboxes and radio buttons, the filled part of sliders / the tick and dot on them |
| `mention` | mentions of you in messages (and the message list's accent text such as "Zur Originalnachricht springen") |
| `scrollbar_hover` | the message list's scrollbar thumb under the pointer |
| `gradient` (`gradient_len` stops, at most 12) | the composer's focus line as a horizontal gradient |

ABI: `PluginVTable.accents(inst, *const ThemeInfo, *mut Accents) -> i32` (1 = it is an accent theme), appended in 1.3 and
read only from plugins that report 1.3. Both structs start with `size` (the writer's `size_of`), so later versions can
append fields. The last enabled design plugin wins. Accent overrides only apply through `Th::tk()`, never to the theme
fields that share a token: the red hang-up buttons stay red.

## Repository index

A repository is a static file, usually `index.min.json` on the `repo` branch of a plugins monorepo (the same model as
Aniyomi extension repositories). Webox accepts two shapes.

A plain array (the shape SchulyPlugins uses):

```json
[
  {
    "name": "Slash Commands",
    "pkg": "dev.webox.plugins.slash-commands",
    "version": "1.3.0",
    "contract": "1.3",
    "description": "Adds /shrug, /tableflip and /count, and turns :) into a smiley.",
    "authors": ["PianoNic"],
    "icon": "icons/dev.webox.plugins.slash-commands.png",
    "homepage": "https://github.com/PianoNic/WeboxPlugins/tree/main/plugins/slash-commands",
    "minWebox": "46.10.0",
    "files": {
      "windows-x86_64": { "file": "files/dev.webox.plugins.slash-commands/1.3.0/windows-x86_64/collabhost_slash_commands.dll", "sha256": "<64 hex>", "size": 181248 },
      "linux-x86_64":   { "file": "files/dev.webox.plugins.slash-commands/1.3.0/linux-x86_64/libcollabhost_slash_commands.so", "sha256": "<64 hex>", "size": 412312 }
    }
  }
]
```

Or an object that also names the repository:

```json
{ "formatVersion": 1, "name": "Webox Plugins", "description": "Official Webox plugins", "homepage": "https://github.com/PianoNic/WeboxPlugins",
  "plugins": [ { "...": "entries as above" } ] }
```

| Field | Required | Meaning |
|---|---|---|
| `name` | no (defaults to `pkg`) | display name |
| `pkg` | yes | reverse-DNS id (`[a-z0-9_-]` labels, at least two, no empty label). It is the install folder name and the key of the on/off state. |
| `version` | yes | semver. An update is offered when a repository has a newer compatible version. |
| `contract` | yes | plugin-api contract `major.minor` the binaries were built for |
| `description`, `homepage`, `icon` | no | shown on the page. `icon` is a URL or a path relative to the index. |
| `authors` | no | array of names (a single string is accepted too) |
| `minWebox` | no | lowest Webox version (semver) that may install it |
| `files` | yes | platform key -> `{ file, sha256, size }`. Keys: `windows-x86_64`, `linux-x86_64`, `linux-aarch64`, `macos-aarch64`, `macos-x86_64` (`<os>-<arch>` from Rust's `std::env::consts`). |
| `files.*.file` | yes | path relative to the index URL (`/`-separated names of letters, digits, `.`, `_`, `-`, none starting with `.`, so no `..`), or an absolute `https://` URL. The last segment is the installed file name, so keep the library's own name (`collabhost_<name>.dll`, `libcollabhost_<name>.so`) and put the version in the folders. |
| `files.*.sha256` | yes | lowercase hex SHA-256 of the binary; pinned at install time |
| `files.*.size` | no | size in bytes. When present, the download must match it exactly. |

Entries that do not validate (bad id, version, contract, hash or file name, or no files) are skipped, and the page shows
how many. The rest of the repository still loads. Duplicate `pkg` entries keep the first one.

## What the app does

- **Repositories** (Settings > Plugins > Verfügbar): the official repository
  (`https://raw.githubusercontent.com/PianoNic/WeboxPlugins/repo/index.min.json`; override with `official` in
  `plugin-repos.json` or the `WEBOX_PLUGIN_REPO` environment variable), plus repositories the user adds or removes. They are
  stored in `plugin-repos.json` in the config folder.
- **Fetching**: when the page opens and on "Aktualisieren", on worker threads. Responses are cached in `plugin-repos/` with
  `ETag` / `Last-Modified` and revalidated with `If-None-Match` / `If-Modified-Since`. The cached list shows at once and
  stays visible when a fetch fails. "Automatisch nach Updates suchen" (on by default) checks once a day, and only while
  repository plugins are installed. It reports updates and never installs them unasked. There is no other background polling.
- **Install**: the platform's file is downloaded off the UI thread with a progress bar. The download uses HTTPS only, is
  capped at the index's `size` or 64 MB, and times out after 10 s to connect and 5 min overall. Redirects are followed by
  hand and only to the same host or to GitHub's raw / release CDN (`raw.githubusercontent.com`, `github.com`,
  `objects.githubusercontent.com`, `release-assets.githubusercontent.com`, `github-releases.githubusercontent.com`,
  `codeload.github.com`). The file is hashed while it streams into `<pkg>/.staging-*`. On a hash or size mismatch it is
  deleted and nothing is loaded. On the UI thread it is hashed again, renamed to `<pkg>/<version>/`, recorded in
  `installed.json` (written to a temp file, then renamed), and loaded at once without a restart.
- **Update**: the old version is stopped and unloaded, the new one moved into place and loaded. If the new version does not
  load, it is removed and the old one comes back. The old folder is deleted only after success.
- **Uninstall**: stops and unloads the plugin, then deletes `<pkg>/` and its `installed.json` entry.
- **Every start**: leftover `.staging-*` / `.old-*` folders are removed. Each installed plugin is hashed against the pinned
  `sha256` before it loads, and a file changed on disk is refused ("Datei wurde nach der Installation verändert").
- **Warnings**: before the first install at all, and before every install from a repository that is not the official one,
  a dialog says that plugins are native code with full access to the account and the signed-in session, and to install
  only from trusted sources. `installed.json` records which repository each plugin came from.

## Writing a plugin

See the plugins monorepo's `CONTRIBUTING.md`: copy an example crate, give it a new `pkg` and a version, and open a pull
request. CI builds it for every platform and publishes it to the `repo` branch.

## Tests

`cargo build` (it builds the example plugins), then `cargo test`. `src/extras/plugin_repo/tests.rs` covers index parsing
(array and object form, invalid entries), the contract and platform checks, the transport rules, the persistence of the
repository list, the ETag cache, refusal of a SHA-256 or size mismatch and of foreign redirects, and a full install,
update, tamper check and uninstall of the example plugin. `src/extras/plugin_settings/tests.rs` covers the settings: schema parsing (what plugin-api writes, broken and odd schemas), validation, defaults, persistence with keyring and file fallback, a native plugin notified on every change and before start, buttons, the search, and the built-in GIF-Extras schema. `os_keyring_roundtrip` (`--ignored`) checks the real OS keyring. The repository tests run against an in-process HTTP server on 127.0.0.1, which a
test-only policy allows. The app itself never allows plain HTTP.
