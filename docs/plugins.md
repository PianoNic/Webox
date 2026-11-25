# Plugins

Plugins add features to Webox: a message logger, auto-reply, video effects, themes and more.
They are native libraries (`.dll` on Windows, `.so` on Linux) that Webox loads at runtime.

> [!WARNING]
> Plugins are native code. They run with the same rights as Webox and have full access to your account and signed-in
> session: messages, spaces, contacts and credentials. Only install plugins from repositories you trust.

## Install a plugin

1. Open **Einstellungen > Plugins**.
2. Switch to the **Verfügbar** tab.
3. Click **Installieren** next to a plugin and confirm. It runs at once, no restart.
4. Back on **Installiert**, use the switch to turn it on or off. **Einstellungen** (or **Details**) opens its page.
5. To remove it, click **Deinstallieren**.

Updates show as **Aktualisieren auf vX**. Webox checks once a day ("Automatisch nach Updates suchen") and never installs
updates unasked.

### Add a plugin repository

1. Open **Einstellungen > Plugins > Verfügbar** and scroll to **Repositories**.
2. Paste the index URL (`https://…/index.min.json`) and click **Hinzufügen**.
3. Confirm the warning. The repository's plugins appear under **Verfügbare Plugins**.

The official repository is always there. Other repositories have **Entfernen**.

> [!TIP]
> A loose `.dll` / `.so` also works: **Plugin-Ordner öffnen**, copy the file in, click **Neu laden**.

---

## Write a plugin

Plugins are Rust `cdylib` crates built against [`collabhost-plugin-api`](../plugin-api) (in this repository).
Published plugins live in [PianoNic/WeboxPlugins](https://github.com/PianoNic/WeboxPlugins).

### 1. The smallest plugin

`Cargo.toml`:

```toml
[package]
name = "collabhost_hello"          # keep the collabhost_ prefix: the file becomes collabhost_hello.dll
version = "0.1.0"
description = "Says hello."
edition = "2021"

[package.metadata.webox]           # read by the WeboxPlugins index generator
name = "Hello"
pkg = "dev.webox.plugins.hello"    # reverse-DNS id, never changes once published

[lib]
crate-type = ["cdylib"]

[dependencies]
collabhost-plugin-api = { git = "https://github.com/PianoNic/Webox" }  # inside WeboxPlugins: .workspace = true
```

`src/lib.rs`:

```rust
use collabhost_plugin_api::{export_plugin, Host, Plugin};

#[derive(Default)]
struct Hello;

impl Plugin for Hello {
    const NAME: &'static str = "Hello";
    const AUTHOR: &'static str = "you";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = env!("CARGO_PKG_DESCRIPTION");
    const COMMANDS: &'static str = "hello";                 // answers /hello

    fn start(&mut self, host: Host) {
        if host.has("ui.toast") { host.call("ui.toast", r#"{"text":"Hallo"}"#); }
    }
    fn command(&mut self, _name: &str, args: &str) -> Option<String> {
        Some(format!("Hallo {args}"))                       // the text to send
    }
}

export_plugin!(Hello);
```

Every hook has a default that does nothing. The full example is
[slash-commands](https://github.com/PianoNic/WeboxPlugins/tree/main/plugins/slash-commands); for a theme see
[pride-theme](https://github.com/PianoNic/WeboxPlugins/tree/main/plugins/pride-theme).

<details>
<summary>All <code>Plugin</code> hooks</summary>

| Hook | Called when |
|---|---|
| `start(host)` / `stop()` | switched on / off (also on install, update, uninstall) |
| `on_send(text)` | before sending: `None` = unchanged, `Some("")` = do not send, `Some(t)` = send `t` |
| `on_message(author, text)` | a row is drawn (display only, see [Events and methods](#reference)) |
| `command(name, args)` | `/name args` for a name in `COMMANDS`: the text to send, or `None` |
| `palette()` / `accents(theme)` / `decor(theme)` | design plugins |
| `settings()` / `settings_changed(values)` / `settings_action(id)` | plugin settings |
| `EVENTS` / `on_event(name, payload)` | named events (1.6) |

`Host` offers `toast`, `send`, `log`, `get` / `set` (small strings kept between runs), `settings()`, and since 1.6
`call`, `has`, `version` and `camera_filter`.

</details>

### 2. Build and try it

```sh
cargo build --release -p collabhost_hello
```

Copy `target/release/collabhost_hello.dll` (Linux: `libcollabhost_hello.so`) into the plugin folder
(**Einstellungen > Plugins > Plugin-Ordner öffnen**) and click **Neu laden**.

> [!NOTE]
> To build WeboxPlugins against a local Webox checkout instead of the git dependency, run
> `pwsh scripts/use-local-api.ps1 ../Webox` (or `scripts/use-local-api.sh`) in WeboxPlugins. `-Off` switches back.
> During Webox development, `COLLABHOST_PLUGIN_PATH` adds extra plugin folders.

### 3. Publish it

1. Fork [WeboxPlugins](https://github.com/PianoNic/WeboxPlugins) and put the crate in `plugins/<name>/`.
2. Run `cargo run -p xtask -- check` (unique `pkg`, valid version).
3. Open a pull request. CI builds it for every platform and publishes it to the `repo` branch.
4. For an update, **bump `version`**. Without a bump, installed copies never update.

Details are in WeboxPlugins' [`CONTRIBUTING.md`](https://github.com/PianoNic/WeboxPlugins/blob/main/CONTRIBUTING.md).
To run your own repository, publish an index file (see [Repository index](#reference)).

---

## Reference

The authoritative description of the plugin API. Code comments link to the section names below.

<details>
<summary><b>Where plugins load from</b></summary>

| Source | Folder | Shown as |
|---|---|---|
| Repository install | `%APPDATA%\WebOx\plugins\<pkg>\<version>\<file>` (Linux: `$XDG_DATA_HOME/webox/plugins/...`, default `~/.local/share/webox/plugins`) | "Aus Repository: ...", can be updated and uninstalled |
| Plugin folder | `%APPDATA%\WebOx\plugins\*.dll` (Linux: `~/.config/webox/plugins/*.so`), `COLLABHOST_PLUGIN_PATH` | "Manuell installiert" |
| Program folder | `plugins/` next to `webox.exe`, `../lib/collabhost/plugins` (Linux and Nix only) | "Mitgeliefert" |

- With the side-loading policy on (`enableDllSideLoadingProtection`, see `docs/architecture.md`) only the program folder
  loads, and installing from repositories is off.
- `plugins/slash-commands` and `plugins/pride-theme` in this repository stay workspace members: Webox's own tests load
  them and must build them against the plugin API of the same commit. The copies users install live in `WeboxPlugins`,
  built in CI against a pinned plugin-api.

</details>

<details>
<summary><b>Plugin API contract</b> (currently 1.6)</summary>

`plugin-api` has a contract version `CONTRACT_MAJOR.CONTRACT_MINOR` (`CONTRACT` as a string, crate version 1.6.1).

History:
- 1.0 — the first interface.
- 1.1 — the contract symbol, nullable function pointers, panic-safe shims.
- 1.2 — plugin settings.
- 1.3 — accent themes.
- 1.4 — decorations (washes, host-drawn decorations, effects).
- 1.6 — named events and methods. After 1.6, new capabilities are names, not table entries. (There is no 1.5.)

Checks:
- A plugin exports `collabhost_plugin_contract() -> u32` = `(major << 16) | minor` (added by `export_plugin!`). Plugins
  built before 1.1 have no such symbol and count as 1.0.
- Webox refuses a plugin built for another **major** version, or a **newer minor** version than its own.
- The repository index repeats it as `contract`, so incompatible plugins are greyed out before any download.

Bumping:
- **Minor**: something added at the end of `HostApi` / `PluginVTable` that older hosts lack.
- **Major** (and `ABI_VERSION` with it): a field removed, reordered or retyped.

What keeps the boundary stable:
- Only `#[repr(C)]` structs, integers, raw pointers, NUL-terminated UTF-8 strings and `extern "C"` function pointers
  cross it. No `String`, `Vec`, `Box`, `bool`, enums or trait objects.
- Function pointers are `Option<extern "C" fn ...>` (same layout as a plain pointer; null = `None`). Webox refuses a
  table with a missing entry instead of calling a null pointer.
- Each side frees what it allocated: plugin strings go back through the plugin's `free_str`, host strings through the
  host's.
- Panics never unwind across the boundary. Plugin-side shims catch them (the hook then reports "no change"), and so do
  the host callbacks. A panic in an `extern "C"` function would otherwise abort Webox.
- A plugin must not keep threads running after `stop` returns. Webox may unload the library afterwards (uninstall,
  update, "Neu laden").

</details>

<details>
<summary><b>Events and methods</b> (contract 1.6): the mechanism and naming rules</summary>

Since 1.6 the tables no longer grow per feature. A plugin receives **events** and calls host **methods** by name, with
JSON payloads, through one entry on each side:

- `PluginVTable.events: *const c_char` — space-separated events the plugin subscribes to (`Plugin::EVENTS`). Webox sends
  only these.
- `PluginVTable.on_event(inst, name, json) -> *mut c_char` — an event fired: its JSON answer (freed with the plugin's
  `free_str`), or null = not handled / no answer.
- `HostApi.call(ctx, method, json) -> *mut c_char` — a host method: its JSON result (freed with the host's `free_str`),
  or null = unknown method.

Webox reads `events` / `on_event` only from plugins that report 1.6. A panic in `on_event` or in a host method answers
null.

```rust
const EVENTS: &'static str = "settings.changed";
fn on_event(&mut self, name: &str, payload: &str) -> Option<String> { /* ... */ None }
// in start(host) or later:
if host.has("ui.toast") { host.call("ui.toast", r#"{"text":"Hallo"}"#); }
```

`host.capabilities` returns every method and event with its version as one flat object
(`{"host.capabilities":1,"ui.toast":1,...,"settings.changed":1}`). `Host::version(name)` / `Host::has(name)` read it, so
a plugin can detect something newer than its own contract and degrade when the host lacks it.

Naming and versioning:
- Names are `<area>.<name>`, lower case. Methods are verbs (`ui.toast`, `space.send`); events say what happened
  (`settings.changed`, `message.received`). Methods and events share one namespace.
- Payloads are JSON objects. They only gain **optional** fields; each addition bumps that name's version in
  `host.capabilities`. Readers ignore unknown fields.
- Anything incompatible (field removed, retyped, made required, new semantics) gets a **new name** (`space.history2`);
  the old one stays as long as it can.
- A method that cannot do its work answers `{"error": "..."}`. Null means only "no such method".
- Adding a method or event needs no contract bump: register it in `METHODS` / `EVENTS` in `src/extras/plugins/mod.rs`
  and list it here. The contract minor still goes up when the tables themselves change.

The fixed hooks of 1.0–1.4 stay; older plugins keep using them. `on_message(author, text)` is a display hook (called for
the rows on screen on every repaint, without a space or message id), and the `send` hook posts into whichever space is
open. A plugin that reacts to messages uses `message.received` and `space.send`. [Auto-Antwort](https://github.com/PianoNic/WeboxPlugins/tree/main/plugins/auto-reply)
uses all three.

**The read-only copy.** Host methods run on the plugin's thread, so they never touch the window's data. Instead the
window copies what they may read: the space list, the people and the newest 200 messages of every loaded space. It does
so only while an enabled plugin subscribes to `message.received` (a plugin that only reads subscribes and ignores the
event). The copy is made when such a plugin appears (with every list loaded so far), kept current, and dropped when the
last such plugin goes. `space.history`, `spaces.list`, `messages.search` and `people.search` read it, make no network
request and are safe from any thread. [Webex MCP](https://github.com/PianoNic/WeboxPlugins/tree/main/plugins/mcp) uses
them (with `ui.confirm` before every `space.send`) to let a local AI client read Webex through Webox.

</details>

<details>
<summary><b>Events</b> (what a plugin receives)</summary>

All events fire on the **UI thread**: answer at once and do slow work on a thread of your own. The number is the
payload version.

- **`settings.changed`** (1) — the plugin's setting values changed; also once before every start. Payload: all values
  (as `settings.get`). Answer ignored.
- **`message.received`** (1) — a message arrived live, once per message id, also your own (from any client). Answer
  ignored.
  ```json
  {"space": "…", "id": "…", "author": "…", "text": "…", "mine": false, "direct": true, "mentioned": false}
  ```
  `direct` = a 1:1 space, `mentioned` = it @-mentions you.
- **`message.deleted`** (2) — a message Webox had loaded was deleted (from any client), once per id. `text` is the last
  known plain text, `date` / `time` are UTC, `team` / `team_name` the space's team (`""` outside one; added in 2).
  ```text
  {"space", "space_name", "team", "team_name", "id", "author", "email", "text", "mine", "parent", "published_ms", "date", "time"}
  ```
  Answer `{"keep": true, "style": "text"|"overlay"|"plain", "collapsed"?: bool, "attachments"?: bool}` to keep it on
  screen, drawn as deleted (see [Deleted and edited messages](#reference)). Anything else: it goes as before.
- **`message.edited`** (2) — a loaded message's text changed, once per id and new text. `old_edited`: the old text was
  itself an edit (its original was never seen). Version 2 added `team` / `team_name`, `email`, `published_ms`,
  `old_edited`.
  ```text
  {"space", "space_name", "team", "team_name", "id", "author", "email", "mine", "published_ms", "old_edited", "old_text", "new_text"}
  ```
  Answer `{"history": [...], "current"?: spans, "inline"?: bool}`: earlier texts, oldest first, each a string or spans,
  shown above the text. Anything else: nothing shown.
- **`message.menu`** (1) — a message's menu opens (right click anywhere on it, or its "···"), once per opening.
  `deleted`: a deleted message a plugin keeps; `edited`: edited per the service, or showing earlier texts.
  ```text
  {"space", "id", "deleted", "edited", "mine"}
  ```
  Answer `{"items": [{"id", "label", "icon"?, "danger"?}]}` (or the bare list): entries appended after a separator, at
  most 8 per plugin, labels at most 80 characters (entries without `id` or `label` are skipped). `icon` is a Momentum
  icon name (`delete-regular`; unknown = none), `danger` draws it red. Anything else: nothing added. Without a
  subscribed plugin the menu is unchanged.
- **`message.menu_action`** (1) — one of the plugin's own entries was picked (`item` = its `id`). Sent only to the
  plugin that offered it. Payload `{"space", "id", "item"}`. Answer ignored.
- **`space.menu`** (1) — a space's menu in the list opens (right click on its row or its "···"), once per opening.
  Payload `{"space", "title", "direct", "team"}`. Answer as `message.menu`; entries go after a separator at the end.
  Without a subscribed plugin the menu is unchanged.
- **`space.menu_action`** (1) — one of the plugin's space-menu entries was picked. Sent only to that plugin. Payload
  `{"space", "item"}`. Answer ignored.
- **`message.edit_tag_clicked`** (1) — "(bearbeitet)" below a message, or "Bearbeitet" in its header, was clicked. Both
  are buttons (pointer cursor) only while a plugin subscribes. `text` is the current plain text. Payload
  `{"space", "id", "text", "mine", "published_ms"}`. Answer ignored; the plugin may open `ui.dialog`.

</details>

<details>
<summary><b>Host methods</b> (what a plugin calls)</summary>

Call with `host.call(name, json)`. Results are JSON; `{}` means done. The number is the payload version.

- **`host.capabilities`** (1) — no argument. Returns `{name: version, ...}` for every method and event.
- **`ui.toast`** (1) — `{"text": "..."}`. Shown as "<plugin name>: text" (same as the `toast` hook): in the window while
  it is in front, otherwise as a desktop toast. Safe from any thread (it wakes the window).
- **`log.write`** (1) — `{"text": "..."}`. Written to the plugin log (same as the `log` hook).
- **`settings.get`** (1) — no argument. Returns the plugin's setting values (same as `settings_get`).
- **`settings.set`** (1) — `{"id": "...", "value": ...}`. Saves one of the plugin's own settings on the next frame,
  exactly like a change on its page: validated against the schema (a refused value goes to the plugin log), passwords
  into the OS keyring, then `settings.changed`. For values the plugin makes itself, e.g. a generated token.
- **`space.send`** (1) — `{"space": "...", "text": "..."}`. Posted into that space as a new plain-text message on the
  next frame, through the plugins' send hooks like a typed one. Dropped for a space you cannot write in. The composer is
  not touched.
- **`space.history`** (2) — `{"space": "...", "limit": n}` (default and at most 200). The last `limit` messages Webox has
  loaded, oldest first, system lines left out. `date` / `time` UTC as in `message.deleted`. Version 2 added everything
  but `author` / `mine` / `text`. No network request: a space never opened holds only what arrived live. From the
  read-only copy.
  ```text
  {"messages": [{"id", "author", "email", "mine", "text", "date", "time", "published_ms"}]}
  ```
- **`spaces.list`** (1) — `{"limit": n, "query": "..."}`, both optional (limit 50, at most 500). The space list in its
  order (latest activity first), hidden spaces left out, `query` matched in the title. `unread` is a flag (Webox keeps no
  count), `last` the list's display text of the last activity ("17:05", "Mo"), `writable` false for closed and
  announcement spaces (`space.send` drops those). From the read-only copy.
  ```text
  {"spaces": [{"id", "title", "direct", "unread", "last", "writable"}]}
  ```
- **`messages.search`** (1) — `{"query": "...", "space": "...", "limit": n}` (`space` optional; limit 20, at most 200).
  Returns `{"messages": [...]}` as `space.history` plus `"space"` / `"space_title"`, newest first: the loaded messages
  whose text contains `query`, case-insensitive. Webex's server-side search is not used; Webox's own global search
  works the same way. From the read-only copy.
- **`people.search`** (1) — `{"query": "...", "limit": n}` (limit 20, at most 200). Contacts, then the people of the
  loaded spaces, each once, `query` matched in name or e-mail. `status` is the presence Webox follows, as the service
  names it ("active", "inactive", "dnd"; `""` = unknown). From the read-only copy.
  ```text
  {"people": [{"id", "name", "email", "status"}]}
  ```
- **`ui.confirm`** (1) — `{"text", "title"?, "ok"?, "cancel"?, "timeout"?}` (timeout 120 s, at most 600). Shows a dialog
  ("<plugin name>: text", two buttons) and **blocks** until the person answers. Returns `{"ok": bool}`: `true` only for
  the `ok` button. The X, Esc, the timeout (`"timeout": true`) and switching the plugin off or uninstalling it all answer
  `false`, so a thread joined in `stop` never waits for it. Questions show one at a time. Refused on the UI thread
  (`on_event`, `start`): call it from a thread of your own.
- **`ui.dialog`** (1) — a read-only dialog. Returns `{}`, or `{"error"}` with no title or no tabs.
  ```text
  {"title": "…", "tabs": [{"label"?, "time_ms"?, "text", "alt"?, "disabled"?, "hint"?}], "selected"?: 0, "switch"?: {"label": "…", "on"?: false}}
  ```
  An optional switch, a row of tabs (`label`, or `time_ms` shown as local "25.11.2026, 14:05"; a `disabled` tab is
  greyed out with `hint` as tooltip), and the selected tab's `text` (spans, see
  [Deleted and edited messages](#reference)), or its `alt` while the switch is on. `selected` defaults to the last tab.
  At most 50 tabs. Does not block, so it may be called from `on_event`. A new one replaces the one shown. It closes with
  "Schließen", the X or Esc, and when its plugin is switched off.
- **`clipboard.write`** (1) — `{"text": "..."}`. Puts the text on the clipboard on the next frame.
- **`message.keep`** (2) — `{"deleted": [...], "edited": [...], "replace": bool}`. Gives again what the answers to
  `message.deleted` / `message.edited` keep, e.g. after a restart. `deleted` entries are `message.deleted` payloads plus
  the answer's fields (`"style"`, `"collapsed"`, `"attachments"`); `edited` entries are
  `{"id", "history": [...], "current"?, "inline"?}` as the answer (version 2). `replace` first drops everything kept so
  far (`{"replace": true}` forgets it all). Lists already on screen are drawn again.
- **`reactions.show_avatars`** (1) — `{"enabled": true, "max": 3, "size": "small", "hide_self": false}`, every field
  optional (these are the defaults). Webox then draws small round avatars of the people who reacted inside each
  reaction chip, after the count: the first `max` (1–10, clamped) in the service's order, overlapping, then "+K" for
  the rest. `size`: `small` (14 pt) or `medium` (18 pt); anything else answers `{"error"}` and changes nothing.
  `hide_self` leaves your own reaction out. Hovering an avatar shows the name. Avatars come from the people and photo
  caches, no network request. `{"enabled": false}` turns them off. Remembered per plugin; a plugin switched off or
  unloaded stops counting, and with several the last enabled one wins. Without any, chips look as before. Safe from any
  thread. [Wer hat reagiert](https://github.com/PianoNic/WeboxPlugins/tree/main/plugins/who-reacted) calls it in `start`
  and on every settings change. It is a method, not a decoration bit: decorations belong to design plugins (the last
  enabled one wins, and a bit cannot carry `max` or `size`), while a method needs no table change, is versioned by name
  and can be checked with `host.has`.
- **`camera.filter`**, **`camera.source`**, **`media.open`**, **`media.read`**, **`media.close`** (1) — see
  [Camera pictures](#reference). Safe from any thread.

</details>

<details>
<summary><b>Deleted and edited messages</b> (message logger hooks, diff spans)</summary>

**Detection.** Webox compares every list of a space it receives with the one it replaces. A message that turned into the
"… hat … gelöscht" row (same id), or that is missing although the new list reaches back to it, was deleted. A message
whose text changed was edited. Only messages Webox had loaded count: what changed while Webox was closed, or in a space
not opened since start, fires nothing. In a loaded space a live delete or edit folds the list again at once (the
service's `tombstone`, cloud/activities), so the events come right away.

**Deleted message styles** (the answer to `message.deleted`):
- `"style": "text"` — kept where it was written (the "gelöscht" row, which stands at the deletion time, goes). Text in
  danger red with "(gelöscht)" below, pictures grey (in colour under the pointer).
- `"style": "overlay"` — text keeps its colour, a red wash lies over the message.
- `"style": "plain"` — drawn like any message (no red, no "(gelöscht)", pictures in colour), e.g. after the person
  switched the highlight off.
- `"collapsed": true` — a run of such messages of one day without replies folds into one row "N gelöschte Nachrichten"
  (link colour, arrow) that unfolds on a click.
- `"attachments": false` — drawn without its pictures, files ("📎" lines) and link cards.

A kept deleted message has no toolbar. Its menu (right click) has only "Nachricht kopieren" (plus the text or picture
actions of what was clicked) and the plugins' `message.menu` entries. A menu never starts or ends with a separator.

**Edit history** (the answer to `message.edited`): `{"history": [...]}` shows the earlier texts above the text, with
"(bearbeitet)" below it.
- An entry is a string (struck through, secondary grey) or **spans**: `[{"text", "kind"?}]` (or `{"spans": [...]}`).
  Kind `"added"` (green), `"removed"` (red, struck through) or none (secondary grey), e.g. a diff against the next
  version. At most 400 spans and 20,000 characters per line.
- `"current"`: the current text as spans. Its `added` parts are drawn green in the message when the spans spell exactly
  the text Webox shows (rich text or emoji that change it leave it as is).
- `"inline": false`: no earlier text in the message, only "(bearbeitet)".

Without such a plugin Webox draws every list exactly as received.

**Keeping and forgetting.** Kept messages are local: Webox holds them in memory while a plugin that listens to either
event is on, and never sends them anywhere. To keep them across restarts the plugin stores the payloads itself
(`host.set`) and gives them back with `message.keep` on start. To forget one message, an edit history or a space's log,
the plugin drops it from its records and gives the rest again with `message.keep` and `"replace": true`. A deleted
message no longer kept goes as Webox deletes any message (its "… gelöscht" row comes back); an edit history no longer
given disappears.

The [Nachrichtenprotokoll](https://github.com/PianoNic/WeboxPlugins/tree/main/plugins/message-logger) plugin does all of
this: menu entries from `message.menu`, clearing a space's log from `space.menu`, and its edit history dialog is
`ui.dialog` opened on `message.edit_tag_clicked`.

</details>

<details>
<summary><b>Camera pictures</b> (frame filters, video source, media)</summary>

Pictures are too big for JSON (720p at 30 fps is 110 MB/s of RGBA), so they never travel as JSON. The plugin passes
addresses as numbers. All in `src/media/camera/hook.rs`.

- **`camera.filter`** — `{"fn": <address>, "ctx": <address>}`; `{"fn": 0, "ctx"?}` removes it. Returns `{}` or
  `{"error"}`. Registers the plugin's frame filter, an `extern "C" fn(ctx, *mut VideoFrame) -> i32` (plugin-api
  `FrameFn`). One per plugin; a second call replaces the first. Removing it waits until a running call has returned.
  Use `Host::camera_filter(closure)`, which does all of this.
- **`camera.source`** — `{"path": "..."}`, `""` = the camera. Returns `{}` or `{"error"}` (no such file). A video file
  (looping, paced at its own frame times) or a picture replaces the camera, at the camera's maximum resolution up to
  1280 x 720. The camera stays closed. Decoded with the platform's framework (`src/media/video/`, no codec of our own); a
  file it cannot decode shows the camera's error panel. Only the plugin that set the file can clear it; the last call
  wins.
- **`media.open`** — `{"path", "width", "height"}`. Returns `{"id", "still"}` or `{"error"}`: a video (looping) or a
  picture as RGBA pictures of exactly that size (scaled to fill, overhang cut off evenly). A video decodes on a host
  thread, a picture or two ahead. At most 4 per plugin.
- **`media.read`** — `{"id", "data": <address>, "width", "height", "stride"}`. Returns `{"pts"}` or `{"error"}`: copies
  the next picture (the current one again while the next is not decoded yet) into the plugin's buffer of
  `stride * height` bytes. The size must be the one given to `media.open`.
- **`media.close`** — `{"id"}`, or `{}` for all of the plugin's. Returns `{}`.

**The frame filter.** `hook::apply` is the one place every camera picture passes: the capture worker calls it right
after the device's picture is decoded and before anything uses it. That covers the Settings > Video preview and the
call's camera (`src/media/camera/send.rs`), so the far end sees the filtered picture too. Without a filter `apply` costs
one atomic load and the picture stays in the device's format. With one, the picture is converted to full-size RGBA
(`VideoFrame`: `width`, `height`, `stride`, `data`, `seq`, `time_us`) and handed to the filter's own thread, which edits
it in place.

**Time budget.** The camera thread waits at most 20 ms per frame (`hook::BUDGET`, of the 33 ms a 30 fps frame has) for
all filters together. A filter that misses it is skipped for that frame: Webox shows that filter's previous picture
again (a replaced background never flashes the real room), or black before it has one. While its thread is busy with an
earlier frame, the next frames skip it at once; its late result becomes the picture shown again. A slow plugin lowers
its own frame rate, never the camera's.

**Why a method, not a table entry.** The filter is an address passed as a number in JSON. `HostApi` and `PluginVTable`
stay as they are, so the contract stays 1.6 and no plugin needs rebuilding. An older host answers null for
`camera.filter`, and `host.has("camera.filter")` tells the plugin. The only new boundary type is `#[repr(C)] VideoFrame`,
which starts with its `size` like `ThemeInfo` so it can grow. plugin-api 1.6.1 adds `VideoFrame`, `FrameFn`,
`Host::camera_filter` (a closure behind a panic-catching shim; dropping the returned `CameraFilter` removes it and frees
the closure) and the `file` settings field. Picture pointers (`media.read`'s `data`) work the same way: the plugin owns
the buffer, the host only writes into it during the call.

**Lifetime.** Before a plugin's `stop` (switched off, updated, uninstalled, Webox closing) Webox removes its filter
(waiting for a running call), its camera source and its open media. A filter never runs while or after its plugin
stops, and the library is never unloaded under it.

Everything stays on the machine: pictures go from the camera through the filters to the preview and the call, nowhere
else. The [Video](https://github.com/PianoNic/WeboxPlugins/tree/main/plugins/video) plugin uses all of them (video or
picture as the camera, background blur / picture / video with person segmentation, mirror, brightness / contrast, soft
focus, a name overlay, a low-light boost).

</details>

<details>
<summary><b>Plugin settings</b> (contract 1.2)</summary>

A plugin declares its settings as a schema. Webox draws the form on the plugin's page (**Einstellungen > Plugins > a
plugin > Einstellungen**), validates and saves the values, and passes them to the plugin. No UI code crosses the
boundary, only JSON. Built-in plugins (GIF-Extras) use the same format, so their pages look the same.

```rust
use collabhost_plugin_api::{Schema, Values};

fn settings(&self) -> Option<Schema> {
    Some(Schema::new()
        .section("Ersetzungen", "Was beim Senden ersetzt wird.")
        .toggle("smiley", "„:)“ ersetzen", "", true)
        .text("shrug_text", "Text für /shrug", "", "¯\\_(ツ)_/¯")
        .number("delay", "Verzögerung (s)", "", 2.0, 0.0, 10.0, 0.5)   // default, min, max, step
        .choice("mode", "Modus", "", "a", &[("a", "Einfach"), ("b", "Genau")])
        .multiline("notes", "Notizen", "", "")
        .password("token", "API-Schlüssel", "Wird im Schlüsselbund gespeichert.")
        .file("bg", "Hintergrundbild", "", "Bilder", &["png", "jpg"])
        .button("reset", "Zähler zurücksetzen", ""))
}
fn settings_changed(&mut self, v: &Values) { self.smiley = v.bool("smiley").unwrap_or(true); }
fn settings_action(&mut self, id: &str) -> Option<String> { (id == "reset").then(|| "Zurückgesetzt".into()) }
```

`host.settings()` returns the same values at any time.

**ABI** (appended in 1.2, so older tables stay valid; read only from plugins that report 1.2 or newer):
- `PluginVTable.settings_schema(inst) -> *mut c_char` — schema JSON (freed with the plugin's `free_str`); null = no
  settings.
- `PluginVTable.settings_changed(inst, json)` — all current values as a flat JSON object, after every change and once
  before every `start`.
- `PluginVTable.settings_action(inst, id) -> *mut c_char` — a button was pressed; an optional message to show, or null.
- `HostApi.settings_get(ctx) -> *mut c_char` — the current values (freed with the host's `free_str`).

Tables of older plugins end before these entries. A plugin without a schema has no settings.

**Schema JSON** (`version` 1):

```json
{ "version": 1, "sections": [
  { "title": "Ersetzungen", "description": "...", "fields": [
    { "type": "bool", "id": "smiley", "label": "...", "description": "...", "default": true },
    { "type": "string", "id": "shrug_text", "label": "...", "default": "..." },
    { "type": "multiline", "id": "notes", "label": "...", "default": "" },
    { "type": "password", "id": "token", "label": "..." },
    { "type": "number", "id": "delay", "label": "...", "default": 2, "min": 0, "max": 10, "step": 0.5 },
    { "type": "choice", "id": "mode", "label": "...", "default": "a", "options": [ { "value": "a", "label": "Einfach" } ] },
    { "type": "file", "id": "bg", "label": "...", "default": "", "filter": "Bilder", "extensions": ["png", "jpg"] },
    { "type": "button", "id": "reset", "label": "..." }
  ] } ] }
```

A top-level `"fields": [...]` without sections works too.

Rules:
- `id`: letters, digits, `_`, `.` and `-`, at most 64 characters, unique. `label` is required. At most 64 fields.
- A field with an unknown type or bad values is left out; the rest still shows.
- Number defaults are clamped into `min..max`. A choice default that is not an option becomes the first option.
- Validation before saving:
  - bool: a bool.
  - string: no line breaks, at most 1000 characters.
  - multiline: at most 20,000 characters. password: at most 4096.
  - number: within `min..max` and on the `step` grid.
  - choice: one of the options.
  - file: a path without line breaks, at most 4096 characters (`""` = none).
  - A refused value is not saved; the reason appears under the field.
- `file` (plugin-api 1.6.1): a path field with "Durchsuchen…", which opens the system's open-file dialog (on its own
  thread) filtered to `extensions` (letters and digits, no dot) under the name `filter`, plus "Alle Dateien".
  "Entfernen" clears it. The path can also be typed. Older hosts leave the field out, like any unknown type.
- The plugin always receives every field (buttons excluded). A stored value that no longer validates (e.g. after a
  schema change) is replaced by the default. An unset password is `""`.

**Storage.** `<config>/plugin-settings/<plugin key>.json` (`{"values": {...}}`). Passwords go to the OS keyring: the
Windows Credential Manager (generic credential `Webox/plugin/<key>/<id>`) or, on Linux, the Secret Service through
`secret-tool`. The file records only that a keyring entry exists. Without a keyring the value is stored in the file under
`fileSecrets`, and the page says it is not encrypted. `--sample` runs keep passwords in memory only.

**Search.** The settings search (top of the Settings navigation) also finds plugin settings by label, description,
section and plugin name. A hit opens that plugin's page.

</details>

<details>
<summary><b>Design plugins</b> (palettes, accent themes, decorations)</summary>

A design plugin either replaces the base theme's main colours (`palette()`, contract 1.0: backgrounds, text, accent,
links) or, since 1.3, only its accents (`accents(&ThemeInfo)`). With accents, backgrounds and body text stay the base
theme's, so an accent theme works on all ten themes, dark and light. Example: `plugins/pride-theme`. The last enabled
design plugin wins.

**Accents (1.3).** Webox calls `accents` with the active base theme (`ThemeInfo`: `light`, `background` = the content
cards, `surface` = the window, `text` = primary text, as 0xRRGGBBAA) and uses what comes back. A field left at 0 keeps
Webox's colour. The plugin picks readable values itself: `collabhost_plugin_api::color` has WCAG helpers (`contrast`,
`readable(c, bg, min)`, `fill_for_text(c, min)`, `text_on(fill)`, `mix`). Aim for 4.5:1 for text and text on fills, 3:1
for dots, rings and control fills.

`Accents` fields:
- `accent` — toggles, the composer's focus line when there is no gradient, small accents.
- `link` — links (`Th::link`).
- `focus` — keyboard focus ring, the outline of a focused text field.
- `selection` — text selection (drawn at 40 %).
- `unread` — unread dots in the lists.
- `badge` (`badge_text`) — the badge dot on the rail.
- `rail_selected` — the circle behind the selected rail item (its icon is drawn in the primary text colour).
- `button` / `button_text` — primary (filled) buttons; hover and pressed shades are derived.
- `control` / `control_icon` — checked checkboxes and radio buttons, the filled part of sliders / the tick and dot.
- `mention` — mentions of you (and the message list's accent text such as "Zur Originalnachricht springen").
- `scrollbar_hover` — the message list's scrollbar thumb under the pointer.
- `gradient` (`gradient_len` stops, at most 12) — the composer's focus line as a horizontal gradient.

ABI: `PluginVTable.accents(inst, *const ThemeInfo, *mut Accents) -> i32` (1 = it is an accent theme), appended in 1.3
and read only from plugins that report 1.3. Both structs start with `size` (the writer's `size_of`), so later versions
can append fields. Accent overrides apply only through `Th::tk()`, never to theme fields that share a token: the red
hang-up buttons stay red.

**Decorations (1.4).** `decor(&ThemeInfo) -> Option<Decor>` declares more than colours. Webox draws everything; no UI
code crosses the boundary. `ThemeInfo` gained `text_secondary`, so the plugin can check both text colours.

- **Washes** (`Wash`: up to 12 `stops`, `angle` in degrees (0 = left to right, 90 = top to bottom), `opacity` over the
  base colour, optional gradient `border` in pt) for the `window` (behind rail and header), the `list_card` and the
  `content_card`. The plugin picks the opacity. Keep text readable: the Pride Theme lowers each wash until primary and
  secondary text keep 4.5:1 over every stop. Message text sits on the content card, so keep that wash faint.
- **Decorations**: bits in `flags` (`collabhost_plugin_api::deco`), drawn in the `stripes` colours:
  - `RAIL_STRIPE` — a vertical band of the stripes along the rail's left edge.
  - `HEADER_STRIPE` — a 2 pt gradient under the header.
  - `AVATAR_RING_SELF` / `AVATAR_RING_ALL` — a ring in the stripe colours around your own avatar / everybody's.
  - `PRESENCE_RING` — a ring around presence dots.
  - `SELECTED_ROW` — a gradient outline on the selected list row.
  - `TAB_PILLS` — a gradient outline on the selected text tab ("Alle", "Nachrichten", ...).
  - `NAV_UNDERLINE` — a gradient underline on the selected settings navigation item.
  - `TOGGLE_TRACK` — switched-on toggles with a gradient track.
  - `BADGE_CYCLE` — unread dots (and the rail badge) cycling through the stripes from row to row.
  - `SCROLLBAR` — scrollbar thumbs in a vertical gradient.
  - `PROGRESS` — progress bars in a horizontal gradient.
  - `SEND_BUTTON` — the send button with a gradient outline and a soft gradient fill under its glyph.
  - `NAME_FLAG` — a small flag (the stripes, stacked) after your name in the profile menu.
  - `SEND_BURST` — effect: confetti and hearts from the send button for 0.9 s when you send.
  - `REACTION_RIPPLE` — effect: a gradient ring spreading from a reaction you add (0.6 s).
  - `JUNE_SPARKLE` — effect: "Happy Pride!" with twinkles for 2.6 s, once a year on the first start in June
    (`pride_sparkle_year` in the settings).

Effects are painted on a layer that takes no input. The window repaints only while one runs (`request_repaint_after`,
16 ms); otherwise nothing animates. None starts while Windows asks for fewer animations ("Animationen in Windows
anzeigen" off; `WEBOX_REDUCE_MOTION` elsewhere). The `decor` entry is read only from plugins that report 1.4. Older
design plugins keep working with their palette or accents. Screenshots: `WEBOX_SHOT_PRIDE_SPARKLE=1` forces the sparkle.

</details>

<details>
<summary><b>Repository index</b> (the format)</summary>

A repository is a static file, usually `index.min.json` on the `repo` branch of a plugins monorepo (the same model as
Aniyomi extension repositories). Two shapes are accepted.

A plain array (the shape SchulyPlugins uses):

```json
[
  {
    "name": "Slash Commands",
    "pkg": "dev.webox.plugins.slash-commands",
    "version": "1.4.0",
    "contract": "1.4",
    "description": "Adds /shrug, /tableflip and /count, and turns :) into a smiley.",
    "authors": ["PianoNic"],
    "icon": "icons/dev.webox.plugins.slash-commands.png",
    "homepage": "https://github.com/PianoNic/WeboxPlugins/tree/main/plugins/slash-commands",
    "minWebox": "46.10.0",
    "files": {
      "windows-x86_64": { "file": "files/dev.webox.plugins.slash-commands/1.4.0/windows-x86_64/collabhost_slash_commands.dll", "sha256": "<64 hex>", "size": 181248 },
      "linux-x86_64":   { "file": "files/dev.webox.plugins.slash-commands/1.4.0/linux-x86_64/libcollabhost_slash_commands.so", "sha256": "<64 hex>", "size": 412312 }
    }
  }
]
```

Or an object that also names the repository:

```json
{ "formatVersion": 1, "name": "Webox Plugins", "description": "Official Webox plugins", "homepage": "https://github.com/PianoNic/WeboxPlugins",
  "plugins": [ { "...": "entries as above" } ] }
```

Fields (required ones marked):
- `pkg` (**required**) — reverse-DNS id (`[a-z0-9_-]` labels, at least two, none empty). The install folder name and the
  key of the on/off state.
- `version` (**required**) — semver. An update is offered when a repository has a newer compatible version.
- `contract` (**required**) — plugin-api contract `major.minor` the binaries were built for.
- `files` (**required**) — platform key -> `{ file, sha256, size }`. Keys: `windows-x86_64`, `linux-x86_64`,
  `linux-aarch64`, `macos-aarch64`, `macos-x86_64` (`<os>-<arch>` from Rust's `std::env::consts`).
  - `file` (**required**) — path relative to the index URL (`/`-separated names of letters, digits, `.`, `_`, `-`, none
    starting with `.`, so no `..`), or an absolute `https://` URL. The last segment is the installed file name: keep
    the library's own name (`collabhost_<name>.dll`, `libcollabhost_<name>.so`) and put the version in the folders.
  - `sha256` (**required**) — lowercase hex SHA-256 of the binary; pinned at install time.
  - `size` — bytes. When present, the download must match exactly.
- `name` — display name (defaults to `pkg`).
- `description`, `homepage`, `icon` — shown on the page. `icon` is a URL or a path relative to the index.
- `authors` — array of names (a single string works too).
- `minWebox` — lowest Webox version (semver) that may install it.

Entries that do not validate (bad id, version, contract, hash or file name, or no files) are skipped, and the page shows
how many. The rest still loads. Duplicate `pkg` entries keep the first one.

</details>

<details>
<summary><b>What the app does</b> (fetch, install, update, uninstall)</summary>

- **Repositories** (**Einstellungen > Plugins > Verfügbar**): the official repository
  (`https://raw.githubusercontent.com/PianoNic/WeboxPlugins/repo/index.min.json`; override with `official` in
  `plugin-repos.json` or the `WEBOX_PLUGIN_REPO` environment variable), plus the ones the user adds or removes. Stored in
  `plugin-repos.json` in the config folder.
- **Fetching**: when the page opens and on "Aktualisieren", on worker threads. Responses are cached in `plugin-repos/`
  with `ETag` / `Last-Modified` and revalidated with `If-None-Match` / `If-Modified-Since`. The cached list shows at once
  and stays when a fetch fails. "Automatisch nach Updates suchen" (on by default) checks once a day, only while
  repository plugins are installed. It reports updates and never installs them unasked. No other background polling.
- **Install**: the platform's file is downloaded off the UI thread with a progress bar.
  - HTTPS only, capped at the index's `size` or 64 MB, 10 s to connect and 5 min overall.
  - Redirects are followed by hand, only to the same host or to GitHub's raw / release CDN (`raw.githubusercontent.com`,
    `github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`,
    `github-releases.githubusercontent.com`, `codeload.github.com`).
  - Hashed while it streams into `<pkg>/.staging-*`. On a hash or size mismatch it is deleted and nothing loads.
  - On the UI thread: hashed again, renamed to `<pkg>/<version>/`, recorded in `installed.json` (temp file, then rename),
    and loaded at once without a restart.
- **Update**: the old version is stopped and unloaded, the new one moved into place and loaded. If it does not load, it is
  removed and the old one comes back. The old folder is deleted only after success.
- **Uninstall**: stops and unloads the plugin, then deletes `<pkg>/` and its `installed.json` entry.
- **Every start**: leftover `.staging-*` / `.old-*` folders are removed. Each installed plugin is hashed against the
  pinned `sha256` before it loads; a file changed on disk is refused ("Datei wurde nach der Installation verändert").
- **Warnings**: before the first install at all, and before every install from a non-official repository, a dialog says
  that plugins are native code with full access to the account and the signed-in session, and to install only from
  trusted sources. Adding a non-official repository shows the same warning. `installed.json` records which repository
  each plugin came from.

</details>

<details>
<summary><b>Tests</b></summary>

Run `cargo build` (it builds the example plugins), then `cargo test`.

- `src/extras/plugin_repo/tests.rs` — index parsing (array and object form, invalid entries), contract and platform
  checks, transport rules, persistence of the repository list, the ETag cache, refusal of a SHA-256 or size mismatch and
  of foreign redirects, and a full install, update, tamper check and uninstall of the example plugin. These run against
  an in-process HTTP server on 127.0.0.1, which a test-only policy allows. The app itself never allows plain HTTP.
- `src/extras/plugin_settings/tests.rs` — schema parsing (what plugin-api writes, broken and odd schemas), validation,
  defaults, persistence with keyring and file fallback, a native plugin notified on every change and before start,
  buttons, the search, the built-in GIF-Extras schema, and the `file` field. `os_keyring_roundtrip` (`--ignored`) checks
  the real OS keyring.
- `src/extras/plugins` —
  - table gating (a 1.4 plugin loads and gets no events), events reaching subscribers only, `call` (unknown methods
    answer null, `host.capabilities` lists the registry), panics in events and host methods;
  - `message.received` firing once per message id and only to subscribers, `space.send` / `space.history`;
  - the read-only copy (made only while a plugin listens, dropped after it), `spaces.list`, `messages.search`,
    `people.search`;
  - `ui.confirm` (answered, refused, timed out, refused on the UI thread, answered "no" when the plugin is switched off so
    `stop` does not hang), `settings.set` / `clipboard.write`;
  - `message.deleted` / `message.edited` firing once, a kept deleted message staying (with its style, also when it
    vanished without a tombstone), edit history attaching, `message.keep` restoring and replacing, everything kept going
    when the plugin is switched off, lists staying as received without such a plugin;
  - `logger_tests.rs`: `space.menu` / `space.menu_action` reaching only the offering plugin and nothing without one,
    `message.edit_tag_clicked` opening a `ui.dialog` that closes with its plugin, the version 2 payloads, spans,
    `current`, `"inline": false`, `"plain"` / `"collapsed"` / `"attachments": false`;
  - `kept.rs`, `dialog.rs` and `ui/messages`: parsing, diff colours, folded runs, menus without dangling separators;
  - `reactions.show_avatars` (off without a plugin, follows the plugin's switch, `max` / `size` clamped or refused);
  - `camera_filter_and_source_follow_the_plugin`: a plugin registering both through `Host::camera_filter` /
    `camera.source` and losing them when switched off.
- `src/ui/messages/who_reacted.rs` — which reactors a chip shows, "+K", hiding your own reaction, the width.
- `src/media/camera/hook.rs` — frame filters (called once per frame with the picture edited in place, a filter over the
  budget skipped at once with its last picture shown instead of the raw one, removal waiting for a running call, owner
  and ctx), the RGBA conversions, scaling, a picture as camera source and `media.read` clip.
- `src/media/video/tests.rs` (`file_source_loops`) — a video as camera source starting over at the end, paced at its own
  rate.

</details>
