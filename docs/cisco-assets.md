# Assets from the official Webex app

Webox looks and sounds like the official Webex desktop app because it can use that app's own icons, sounds, pictures and translations. Some of them are published under an open licence and are in this repository. The rest belong to Cisco: they live only on your machine, in the git-ignored `assets/cisco/`, and Webox falls back when they are missing.

## What is where

| Files | Count | Source | Licence | In the repository |
| --- | --- | --- | --- | --- |
| `assets/icons/*.svg` | 3,724 | [Momentum Design](https://github.com/momentum-design/momentum-design) (`packages/assets/icons`, `illustrations`) and [Momentum UI](https://github.com/momentum-design/momentum-ui) (`icons`, `icons-rebrand`, `illustrations`); the five `*_16_em` toolbar glyphs are the `*_16` ones re-framed by `scripts/emoji/gen_em_icons.py` | MIT | yes |
| `assets/emoji/*.svg` | 3,205 | [Twemoji](https://github.com/jdecked/twemoji) | CC-BY 4.0 | yes, with credit |
| `assets/emoji_data.tsv` | 1 | emoji order and groups, German names and keywords from [Unicode CLDR](https://github.com/unicode-org/cldr) (`annotations/de.xml`) | Unicode-3.0 | yes |
| `assets/emoji_shortcodes.tsv` | 1 | English names from CLDR (`annotations/en.xml`, `annotationsDerived/en.xml`), written by `scripts/emoji/gen_emoji_shortcodes.py` | Unicode-3.0 | yes |
| `assets/keyboard-shortcuts.json` | 1 | command ids, default keys and flags of the official shortcuts, without their descriptions | MIT | yes |
| `assets/sounds/manifest.json`, `assets/test_sounds/tone*` | 5 | written and generated for Webox | MIT | yes |
| `assets/icon/`, `assets/logo-*.png`, `assets/branding/` | | Webox's own artwork | MIT | yes |
| `assets/cisco/icons/*.svg` | 619 | the app's own icons: animation frames, reaction pictures, presence, recording and layout art, illustrations, the AI assistant mark | Cisco | no |
| `assets/cisco/i18n/` | 35 | the 34 translation packs made from the app's `.qm` files, and `index_en_de.tsv` | Cisco | no |
| `assets/cisco/sounds/*.wav` | 14 | ringtones, message sounds and call tones | Cisco | no |
| `assets/cisco/images/` | 8 | the splash spinner and wallpapers, the login background, the Giphy marks shipped with the app, the message list's loading animation (`Loading.gif`; fallback: three drawn dots) | Cisco / Giphy | no |
| `assets/cisco/keyboardShortcuts.json` | 1 | the app's own shortcut table, with descriptions | Cisco | no |
| `assets/cisco/emoji_shortcodes.tsv` | 1 | the app's English emoji names (`EmojiLocalisation`) | Cisco | no |
| `assets/cisco/test_sounds/dtmf_orig.mp3` | 1 | the app's DTMF tone, re-encoded, for one test | Cisco | no |

The full list of the local files is `scripts/cisco-assets.txt` (names only).

The icons count as Momentum when their name is in one of the two repositories. A spot check of 20 of them shows the same drawing as upstream, minified like the published `@momentum-design/icons` build, so they are the MIT icons. Brand logos are never embedded (`build.rs` filters them by name).

Webex and Cisco are trademarks of Cisco Systems, Inc.; Webox is not affiliated with Cisco.

## How the app finds them

1. **Build time.** `build.rs` adds `assets/cisco/icons` to the icon table and embeds every other file under `assets/cisco/` into a table that `src/assets/cisco_assets.rs` reads (`cisco_assets::get("sounds/Delight.wav")`). It watches `assets/`, so adding or removing the folder triggers a rebuild.
2. **Run time.** A file that is not embedded comes from the installed official app where it is a loose file: the sounds in `%LOCALAPPDATA%\CiscoSparkLauncher\dynamic_dependencies\{ringtones,momentum_ringtones,sounds}` (Windows). Translations of the user's language come from `...\dynamic_dependencies\translations\*.qm` through `i18n::pack_dirs`, as before.
3. **Fallback.**

| Missing | Webox then |
| --- | --- |
| translation packs | shows English (or the installed app's `.qm` for that language) |
| sounds | plays nothing; custom ringtone and message sound files still work |
| splash spinner, wallpapers, login background | draws the plain background and egui's spinner |
| app-only icons | draws the closest Momentum icon instead (`icons::FALLBACKS`: the empty-state illustrations of calls, voicemail, contacts, search and the emoji picker, the invite picture, the AI assistant mark and the highlights avatar); a test checks every fallback exists |
| Giphy marks | writes "Powered by GIPHY" |
| `keyboardShortcuts.json` | uses `assets/keyboard-shortcuts.json` |
| `emoji_shortcodes.tsv` | uses the CLDR names |

Tests that compare against these files (German pack strings, sound durations) return early when they are missing, so `cargo test` passes in CI.

## Getting the files

```powershell
pwsh scripts/restore-cisco-assets.ps1 -FromCommit <rev>   # from a private checkout's history, where they were under assets/
pwsh scripts/restore-cisco-assets.ps1 -FromFolder <dir>   # from another assets/cisco
pwsh scripts/restore-cisco-assets.ps1 -FromInstall        # the sounds from the installed official app (Windows)
```

The pictures and icons are not loose files of an installed app; they only come from a folder or a commit that already has them. `scripts/i18n/gen_i18n.py` rebuilds the packs from the app's `.qm` files.
