# Refactor plan: files under 400 lines

Rule: a `.rs` file stays under about 400 lines. Bigger files are split by responsibility into child modules, as plain moves with no behaviour change. Tests, build warnings and the test count stay the same after every file.

## How the split works

- A child module lives in its parent's folder: `src/ui/messages/mod.rs` gets `src/ui/messages/list.rs`. A file that gets its first child becomes a folder: `foo.rs` moves to `foo/mod.rs` (`git mv`), so no `foo.rs` ever sits next to a `foo/`.
- The child starts with `use super::*;`, so it sees everything the parent sees.
- Methods move out of `impl App { ... }` (or `impl Session`) into an `impl App { ... }` of their own in the child. Rust allows any number of impl blocks.
- A moved item that was private becomes `pub(super)`, which is exactly as visible as before: the parent module and everything below it. Items that already had a visibility keep it; `pub(super)` items become `pub(crate)` because they moved one level down.
- Free functions and constants are re-exported with `pub(crate) use child::*;`, so every existing path keeps working.
- Relative paths in moved code (`super::x`) gain one `super::`.
- A `mod tests { ... }` block becomes `tests.rs` (or its own name, e.g. `title_tests.rs`), unchanged apart from the indent.

## Inventory after the merge of all feature branches (main f68b509)

22 files were over 400 lines. All are split; the parent keeps its types, constants and small helpers. Paths and line counts are as they were then; the area folders below moved most of them.

| File (before) | Lines | Split into (lines) |
| --- | --- | --- |
| `src/ui/messages.rs` | 2028 | `messages.rs` 154, `list` 336, `rows` 318, `media` 316, `interact` 259, `helpers` 251, `text` 176, `tests` 247 |
| `src/i18n.rs` | 1560 | `i18n.rs` 360, `qm` 331, `format` 314, `time` 175, `select` 148, `tests` 252 |
| `src/cloud/activities.rs` | 1219 | `activities.rs` 218, `fold` 366, `send` 164, `history` 126, `tests` 359 |
| `src/cloud.rs` | 947 | `cloud.rs` 329, `session` 187, `start` 171, `lists` 117, `crypto` 82, `live` 54, `title_tests` 24 |
| `src/ui/composer.rs` | 863 | `composer.rs` 108, `draw` 515 (see below), `actions` 249 |
| `src/notify.rs` | 748 | `notify.rs` 159, `hub` 371, `card` 227 |
| `src/ui/gif_picker.rs` | 679 | `gif_picker.rs` 157, `draw` 326, `flow` 187, `tests` 19 |
| `src/cloud/locus.rs` | 558 | `locus.rs` 174, `session` 254, `tests` 133 |
| `src/ui/call_win.rs` | 553 | `call_win.rs` 78, `call` 272, `dock` 212 |
| `src/webview.rs` | 541 | `webview.rs` 207, `platform_win` 198, `platform_gtk` 112, `tests` 25 |
| `src/ui/new_message.rs` | 528 | `new_message.rs` 155, `pane` 313, `tests` 63 |
| `src/ui/space_tabs.rs` | 524 | `space_tabs.rs` 122, `tabs` 295, `find` 116 |
| `src/ui/calls_page.rs` | 472 | `calls_page.rs` 345, `voicemail` 134 |
| `src/ui/messages/html.rs` | 470 | `html.rs` 351, `tests` 118 |
| `src/ui/menu.rs` | 463 | `menu.rs` 335, `dialog` 108, `scroll_tests` 17, `combo_tests` 7 |
| `src/ui/attach.rs` | 457 | `attach.rs` 336, `receive` 110, `tests` 17 |
| `src/ui/spaces_ext.rs` | 439 | `spaces_ext.rs` 392, `shots` 53 |
| `src/cloud/files.rs` | 430 | `files.rs` 342, `tests` 87 |
| `src/cloud/spaces_ext.rs` | 421 | `spaces_ext.rs` 363, `tests` 57 |
| `src/tray.rs` | 418 | `tray.rs` 150, `imp` 267 (the Windows backend) |
| `src/shortcuts/mod.rs` | 411 | `mod.rs` 291, `bindings` 126 |
| `src/app.rs` | 403 | `app.rs` 266, `signin` 141 |

## Area folders (branch `restructure`, from main 6d55c0b)

`src/` had 23 `.rs` files at its root and several names twice (`foo.rs` next to `foo/`). Now only `main.rs` is at the root and every other file sits in an area folder; every folder is a module with a `mod.rs`. Moves only (`git mv`), one commit per area; build warnings (42), test-build warnings (25) and the tests (309 passed, 3 ignored) stayed the same after each.

| Area | Moved there |
| --- | --- |
| `src/app/` | `app.rs` (now `app/mod.rs`), `actions.rs`, `files.rs` (the upload / download state shared by UI and cloud thread: `cloud/files/` is the protocol side, so it isn't in `cloud/`) |
| `src/cloud/` | `cloud.rs` (now `cloud/mod.rs`), `auth.rs`, `jose.rs`, `secrets.rs`, `richtext.rs` |
| `src/ui/` | `presence.rs` (the presence line under a person's name: UI text from the translations, an `App` method) |
| `src/media/` | `audio.rs`, `sounds.rs` |
| `src/platform/` | `system.rs`, `nmhost.rs`, `tray.rs` + `tray/`, `notify.rs` + `notify/`, `webview.rs` + `webview/`, `linux/` |
| `src/assets/` | `icons.rs`, `cisco_assets.rs`, `photos.rs`, `i18n.rs` + `i18n/` |
| `src/extras/` | `giphy.rs`, `gif_extras/` (built-in plugin: favourites, uploaded-GIF favourites, extended picker, stars), `plugins.rs` |

Inside `ui/` and `cloud/` every `foo.rs` with a `foo/` became `foo/mod.rs` (`ui/`: attach, call_win, calls_page, composer, gif_picker, menu, messages, messages/html, new_message, settings, space_tabs, spaces_ext; `cloud/`: activities, files, live_list, locus, spaces_ext).

Paths: `system`, `icons`, `i18n`, `jose` and `richtext` are used all over the crate (most files start with `use crate::*;` and call `system::...`, `richtext::...`), so `main.rs` re-exports them (`pub(crate) use platform::system;` ...); the `tr!` macros need `$crate::i18n` anyway. Every other moved module is named by its full path (`crate::cloud::auth`, `crate::platform::tray`, `crate::media::audio`, `crate::app::files`, ...).

The move showed five files over 400 lines; each was split in its own commit:

| File | Lines | Split into (lines) |
| --- | --- | --- |
| `src/ui/composer/draw.rs` | 514 | `draw.rs` 368, `restore.rs` 46 (first frame / space switch), `bar.rs` 131 (toolbar row with Cancel / Update, send pill) |
| `src/cloud/people.rs` | 498 | `people/mod.rs` 366, `people/tests.rs` 131 |
| `src/ui/call.rs` | 460 | `call/mod.rs` 323, `call/devices.rs` 144 (device choice, lists, level meter, pickers, device menu) |
| `src/assets/photos.rs` | 434 | `photos/mod.rs` 387, `photos/tests.rs` 46 |
| `src/ui/chrome.rs` | 416 | `chrome/mod.rs` 327, `chrome/rail.rs` 96 |

The composer was the one exception of the first round: `App::composer` is a single function over shared local state. Its three self-contained sections became methods (`composer_restore`, `composer_toolbar`, `composer_send_buttons`) that take the state they touch (`&mut St`, the space id, the rects) and return what the rest of the function reads (the emoji / GIF anchors, Update / Send clicked). The code inside is unchanged apart from `&mut st` -> `st` and `get(&id)` -> `get(id)`. Screenshots of main and of the branch (`--sample --screenshot`: every `WEBOX_SHOT_COMPOSER` preset, light theme, Settings > Audio, the main window with its rail) are pixel-identical. The call windows are viewports of their own and not in those screenshots.

## Next candidates

Files between 350 and 400 lines that will cross the line with the next feature: `src/ui/messages/rows.rs` (401), `src/ui/spaces_ext/mod.rs` (392), `src/assets/photos/mod.rs` (387), `src/platform/notify/hub.rs` (380), `src/ui/messages/helpers.rs` (375), `src/cloud/activities/fold.rs` (372), `src/ui/composer/draw.rs` (368), `src/cloud/people/mod.rs` (366), `src/cloud/spaces_ext/mod.rs` (365), `src/assets/i18n/mod.rs` (362).
