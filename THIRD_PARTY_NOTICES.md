# Third-party notices

Webox itself is under the [MIT licence](LICENSE). It ships or builds on the following work by others.

## Assets

| What | Where | Licence |
| --- | --- | --- |
| Emoji graphics: [Twemoji](https://github.com/jdecked/twemoji), (c) Twitter, Inc. and other contributors | `assets/emoji/` | [CC-BY 4.0](https://creativecommons.org/licenses/by/4.0/). The app also credits it under Settings > About |
| Icons: [Momentum Design](https://github.com/momentum-design/momentum-design), (c) 2022 Momentum Design | `assets/icons/` | [MIT](https://github.com/momentum-design/momentum-design/blob/main/LICENSE) |
| Icons: [Momentum UI](https://github.com/momentum-design/momentum-ui), (c) 2014-2020 Cisco Systems, Inc. and/or its affiliated entities | `assets/icons/` | [MIT](https://github.com/momentum-design/momentum-ui/blob/main/LICENSE) |
| Emoji names and keywords: [Unicode CLDR](https://github.com/unicode-org/cldr) annotations, (c) Unicode, Inc. | `assets/emoji_data.tsv`, `assets/emoji_shortcodes.tsv` | [Unicode License v3](https://www.unicode.org/license.txt) |
| Font: Selawik, (c) Microsoft Corporation (Linux builds) | `assets/fonts/` | [SIL Open Font License 1.1](assets/fonts/Selawik-OFL.txt) |
| Noise-removal model: DeepFilterNet 3 (`DeepFilterNet3_onnx.tar.gz`, unmodified, from [Rikorose/DeepFilterNet](https://github.com/Rikorose/DeepFilterNet) v0.5.6), (c) 2021 Hendrik Schröter | `assets/models/deepfilternet3/`, embedded in the binary | MIT or Apache-2.0, at your option ([LICENSE-MIT](assets/models/deepfilternet3/LICENSE-MIT), [LICENSE-APACHE](assets/models/deepfilternet3/LICENSE-APACHE)) |

Translations, sounds and some pictures of the official Webex app are not part of this repository. A local build can use them from your own Webex installation, see [docs/cisco-assets.md](docs/cisco-assets.md). Webex and Cisco are trademarks of Cisco Systems, Inc.; Webox is not affiliated with Cisco. GIPHY is a trademark of Giphy, Inc.

## Rust crates

`cargo metadata` lists about 720 third-party crates across all platforms. Nearly all are under MIT, Apache-2.0 or both (with a few Zlib, BSD, ISC, BSL-1.0, Unlicense and CC0 options). The ones with other terms:

| Crates | Licence | Note |
| --- | --- | --- |
| `symphonia` and its `symphonia-*` codec and format crates | MPL-2.0 | used unmodified; source at [pdeljanov/Symphonia](https://github.com/pdeljanov/Symphonia) |
| `icu_*`, `litemap`, `tinystr`, `writeable`, `yoke`, `zerofrom`, `zerotrie`, `zerovec`, `potential_utf` (and their `-derive` crates), `unicode-ident` | Unicode-3.0 | Unicode data and ICU4X |
| `webpki-roots` | CDLA-Permissive-2.0 | Mozilla's root certificate list |
| `epaint_default_fonts` | MIT OR Apache-2.0, plus OFL-1.1 and the Ubuntu Font Licence for the bundled fonts | egui's default fonts |
| `r-efi` | MIT OR Apache-2.0 OR LGPL-2.1-or-later | used under MIT/Apache-2.0 |

Microphone noise removal uses `deep_filter` (libDF of [DeepFilterNet](https://github.com/Rikorose/DeepFilterNet), MIT OR Apache-2.0, from git at the v0.5.6 release) with the `tract-*` ONNX runtime (MIT OR Apache-2.0), and `nnnoiseless` (BSD-3-Clause, a port of Xiph's RNNoise) as the lightweight fallback.

Regenerate the list with `cargo metadata --format-version 1` or [`cargo about`](https://github.com/EmbarkStudios/cargo-about) before a release if dependencies change.
