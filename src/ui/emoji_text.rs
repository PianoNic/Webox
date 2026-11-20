//! Emoji inside message text. egui can only draw a character that one of its fonts has, and for everything else its built-in
//! fallback fonts show an unrelated glyph (an emoji newer than they are - 🫠, 🥹, 🫨 - comes out as a different picture).
//! So messages are split into emoji clusters here and each one is drawn as
//!   1. the colour picture of the emoji set (assets/emoji, looked up by exact code points: with / without U+FE0F, and for skin
//!      tones the neutral base), or
//!   2. when the set doesn't have it (newer emoji): the system emoji font - Segoe UI Emoji on Windows, text-style (egui can't
//!      draw colour fonts) - or
//!   3. nothing (hidden) when neither has it - never a box, never somebody else's glyph.
//! Text outside emoji clusters loses its invisible format characters (variation selectors, ZWJ/ZWNJ/ZWSP, bidi marks, word
//! joiner, BOM, tags, ...: Unicode Default_Ignorable_Code_Point), which Qt's text engine never draws; egui's fonts lack them and
//! would show a box (e.g. a stray U+FE0F after ":D"). A lone skin-tone modifier is an emoji of its own (Twemoji has them).
use crate::*;
use egui::{FontFamily, Galley};

/// The context, for glyph-coverage checks of the system emoji font from the text builders (set by [`ensure_font`]).
static CTX: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

/// Unicode Default_Ignorable_Code_Point (DerivedCoreProperties.txt): never drawn.
pub(crate) fn is_invisible(c: char) -> bool {
    matches!(c as u32, 0xAD | 0x34F | 0x61C | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180B..=0x180F | 0x200B..=0x200F | 0x202A..=0x202E
        | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFF8 | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0000..=0xE0FFF)
}

/// `s` without its invisible format characters.
pub(crate) fn strip_invisible(s: &str) -> std::borrow::Cow<'_, str> {
    if s.chars().any(is_invisible) { s.chars().filter(|c| !is_invisible(*c)).collect::<String>().into() } else { s.into() }
}

/// What the system emoji font has to draw for a cluster (its visible code points, the skin tone dropped), or None when the font
/// is missing or lacks one of them - then the cluster is hidden.
pub(crate) fn sys_text(cluster: &str) -> Option<String> {
    let ctx = CTX.get()?;
    let vis: String = cluster.chars().filter(|c| !is_invisible(*c) && !is_tone(*c)).collect();
    let fid = egui::FontId::new(16.0, FontFamily::Name(FAMILY.into()));
    (!vis.is_empty() && ctx.fonts(|f| f.families().contains(&fid.family) && f.has_glyphs(&fid, &vis))).then_some(vis)
}

/// The egui font family that holds only the system emoji font (empty when there is none).
pub(crate) const FAMILY: &str = "emoji-sys";

/// Whether the system emoji font family exists (it is registered by `set_fonts` in main.rs; Windows only).
pub(crate) fn ensure_font(ctx: &egui::Context) -> bool {
    let _ = CTX.set(ctx.clone());
    ctx.fonts(|f| f.families().contains(&FontFamily::Name(FAMILY.into())))
}

fn is_tone(c: char) -> bool { ('\u{1F3FB}'..='\u{1F3FF}').contains(&c) }
fn is_regional(c: char) -> bool { ('\u{1F1E6}'..='\u{1F1FF}').contains(&c) }
fn is_tag(c: char) -> bool { ('\u{E0020}'..='\u{E007F}').contains(&c) }

/// Code points of the BMP that are emoji by default (Emoji_Presentation); the other symbols there (©, ™, ✔, ☎, ...) only count as
/// emoji when followed by U+FE0F.
fn bmp_default_emoji(c: char) -> bool {
    matches!(c as u32, 0x231A | 0x231B | 0x23E9..=0x23EC | 0x23F0 | 0x23F3 | 0x25FD | 0x25FE | 0x2614 | 0x2615 | 0x2648..=0x2653 | 0x267F | 0x2693
        | 0x26A1 | 0x26AA | 0x26AB | 0x26BD | 0x26BE | 0x26C4 | 0x26C5 | 0x26CE | 0x26D4 | 0x26EA | 0x26F2 | 0x26F3 | 0x26F5 | 0x26FA | 0x26FD
        | 0x2705 | 0x270A | 0x270B | 0x2728 | 0x274C | 0x274E | 0x2753..=0x2755 | 0x2757 | 0x2764 | 0x2795..=0x2797 | 0x27B0 | 0x27BF
        | 0x2B1B | 0x2B1C | 0x2B50 | 0x2B55)
}

fn is_base(c: char, next: Option<char>) -> bool {
    let u = c as u32;
    if (0x1F000..=0x1FAFF).contains(&u) { return !is_regional(c) && !is_tone(c); }
    if bmp_default_emoji(c) { return true; }
    let symbol = matches!(u, 0xA9 | 0xAE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA | 0x231A..=0x23FF | 0x24C2 | 0x25AA..=0x25FE
        | 0x2600..=0x27BF | 0x2934 | 0x2935 | 0x2B05..=0x2B07 | 0x2B1B..=0x2B55 | 0x3030 | 0x303D | 0x3297 | 0x3299);
    symbol && next == Some('\u{FE0F}')
}

/// Byte ranges of the emoji clusters in `s`: keycaps (1️⃣), flags (🇨🇭 and tag flags), and base + FE0F + skin tone, joined by ZWJ.
pub(crate) fn clusters(s: &str) -> Vec<(usize, usize)> {
    let cs: Vec<(usize, char)> = s.char_indices().collect();
    let end_of = |i: usize| cs.get(i).map_or(s.len(), |x| x.0);
    let (mut out, mut i) = (Vec::new(), 0);
    while i < cs.len() {
        let c = cs[i].1;
        let at = |k: usize| cs.get(k).map(|x| x.1);
        let start = i;
        // Keycap: [0-9#*] FE0F? 20E3.
        if (c.is_ascii_digit() || c == '#' || c == '*') && (at(i + 1) == Some('\u{20E3}') || (at(i + 1) == Some('\u{FE0F}') && at(i + 2) == Some('\u{20E3}'))) {
            i += if at(i + 1) == Some('\u{20E3}') { 2 } else { 3 };
            out.push((cs[start].0, end_of(i)));
            continue;
        }
        if is_regional(c) {
            i += if at(i + 1).is_some_and(is_regional) { 2 } else { 1 };
            if i - start == 2 { out.push((cs[start].0, end_of(i))); }
            continue;
        }
        if is_tone(c) { i += 1; out.push((cs[start].0, end_of(i))); continue; } // a lone skin tone (one after a base was taken below)
        if !is_base(c, at(i + 1)) { i += 1; continue; }
        i += 1;
        loop {
            while at(i).is_some_and(|x| x == '\u{FE0F}' || is_tone(x)) { i += 1; }
            if c == '\u{1F3F4}' { while at(i).is_some_and(is_tag) { i += 1; } } // subdivision flags (🏴󠁧󠁢󠁥󠁮󠁧󠁿)
            if at(i) == Some('\u{200D}') && at(i + 1).is_some_and(|x| is_base(x, at(i + 2))) { i += 2; } else { break; }
        }
        out.push((cs[start].0, end_of(i)));
    }
    out
}

/// Most emoji a message may have to still count as "only emoji" (`emoji_only`).
pub(crate) const BIG_EMOJI_MAX: usize = 3;

/// Webox addition: `s` is nothing but 1..=`BIG_EMOJI_MAX` emoji clusters (ZWJ sequences, skin tones, flags, keycaps), whitespace
/// between / around them allowed, no other character. Returns the clusters joined by single spaces where the text had whitespace.
pub(crate) fn emoji_only(s: &str) -> Option<String> {
    let cl = clusters(s);
    if cl.is_empty() || cl.len() > BIG_EMOJI_MAX { return None; }
    let mut out = String::new();
    let mut at = 0;
    for (a, b) in cl {
        let gap = &s[at..a];
        if !gap.chars().all(char::is_whitespace) { return None; }
        if !gap.is_empty() && !out.is_empty() { out.push(' '); }
        out.push_str(&s[a..b]);
        at = b;
    }
    s[at..].chars().all(char::is_whitespace).then_some(out)
}

/// What to draw for a cluster: the set's picture under this name (a code point sequence `icons::Icons::emoji` accepts), or the system font.
pub(crate) fn resolve(cluster: &str) -> Option<String> {
    if icons::emoji_exists(cluster) { return Some(cluster.to_string()); }
    let neutral: String = cluster.chars().filter(|c| !is_tone(*c)).collect(); // skin tone not in the set: the neutral picture
    (neutral != cluster && icons::emoji_exists(&neutral)).then_some(neutral)
}

/// The text reserved for an emoji picture in a layout (an em space and a thin space: ~1.2 em wide, invisible).
pub(crate) const PLACEHOLDER: &str = "\u{2003}\u{2009}";

/// Where the placeholders starting at the given char indices ended up in a laid-out text: (x, row top, width, row height),
/// relative to the galley.
pub(crate) fn placeholder_rects(g: &Galley, idx: &[usize]) -> Vec<Option<Rect>> {
    let mut out = vec![None; idx.len()];
    let mut off = 0usize;
    for row in &g.rows {
        let n = row.glyphs.len();
        for (k, &want) in idx.iter().enumerate() {
            if want >= off && want < off + n {
                let (g0, w) = (&row.glyphs[want - off], row.glyphs.get(want - off + 1).map_or(0.0, |x| x.advance_width));
                out[k] = Some(Rect::from_min_size(pos2(g0.pos.x, row.rect.top()), Vec2::new(g0.advance_width + w, row.rect.height())));
            }
        }
        off += n + usize::from(row.ends_with_newline);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cl(s: &str) -> Vec<&str> { clusters(s).into_iter().map(|(a, b)| &s[a..b]).collect() }

    #[test]
    fn emoji_only_messages() {
        let yes = |s: &str| emoji_only(s).unwrap_or_else(|| panic!("{s:?} should be emoji-only"));
        assert_eq!(yes("😆"), "😆");
        assert_eq!(yes("😆😆😆"), "😆😆😆");
        assert_eq!(yes("👨‍👩‍👧"), "👨‍👩‍👧", "a ZWJ family is one emoji");
        assert_eq!(yes("🇨🇭"), "🇨🇭");
        assert_eq!(yes("1️⃣"), "1️⃣");
        assert_eq!(yes("❤️"), "❤️");
        assert_eq!(yes("👍🏽"), "👍🏽", "skin tone");
        assert_eq!(yes("  😆 \n 👍🏽  "), "😆 👍🏽", "whitespace allowed, collapsed to one space");
        assert_eq!(yes("👨‍👩‍👧👨‍👩‍👧🇨🇭"), "👨‍👩‍👧👨‍👩‍👧🇨🇭", "3 sequences");
        for no in ["😆 hi", ":)", "ツ", "😆😆😆😆", "😆 😆 😆 😆", "", "   ", "1", "#", "a😆", "😆.", "©", "@Mara 😆"] {
            assert_eq!(emoji_only(no), None, "{no:?}");
        }
    }

    #[test]
    fn clusters_follow_unicode_sequences() {
        assert_eq!(cl("a🫠 b"), ["🫠"]);
        assert_eq!(cl("👍🏽x❤️y❤"), ["👍🏽", "❤️", "❤"]);
        assert_eq!(cl("🇨🇭 und 🇩"), ["🇨🇭"]); // a lone regional indicator is no flag
        assert_eq!(cl("1️⃣ 2⃣ 3"), ["1️⃣", "2⃣"]);
        assert_eq!(cl("👨‍💻!"), ["👨‍💻"]); // ZWJ sequence stays one cluster
        assert_eq!(cl("© ™ ✔ x"), Vec::<&str>::new()); // text-style symbols without FE0F are plain text
        assert_eq!(cl("©️✔️"), ["©️", "✔️"]);
        assert_eq!(cl("ok ⭐ 🎉"), ["⭐", "🎉"]);
        assert_eq!(cl("a 🏽 b"), ["🏽"]); // a lone skin tone is an emoji
        assert_eq!(cl(":D\u{FE0F}"), Vec::<&str>::new()); // a stray VS16 after text is no emoji
    }

    #[test]
    fn invisible_format_characters_are_never_drawn() {
        for s in [":D\u{FE0F}", ":D\u{200D}", ":D\u{200B}", "\u{FEFF}:D", ":\u{2060}D", ":D\u{E0067}", ":D\u{200E}"] {
            assert_eq!(strip_invisible(s), ":D", "{s:?}");
        }
        assert!(matches!(strip_invisible("plain äöü 😀"), std::borrow::Cow::Borrowed(_)));
        // Without a registered context there is no system font to fall back to: hide.
        assert_eq!(sys_text("\u{1FAEB}"), None);
    }

    #[test]
    fn lookup_is_exact_and_never_guesses() {
        for present in ["👍", "😀", "🙌", "👨‍💻", "🇨🇭", "❤", "❤️", "👍🏽"] { assert!(resolve(present).is_some(), "{present} should be in the set"); }
        assert_eq!(resolve("🙌🏿").as_deref(), Some("🙌🏿")); // tone variants exist for 🙌
        // Not in the set at all (newer than Twemoji) or not an emoji at all: no picture, so the caller uses the system font / a box.
        for missing in ["\u{1FAEB}", "\u{1F6D9}", "\u{1FA8B}", "\u{E000}", "x", ""] { assert_eq!(resolve(missing), None, "{missing:?}"); }
        // A tone the set lacks falls back to the neutral picture of the same emoji, never to another emoji.
        assert_eq!(resolve("🫠🏽").as_deref(), Some("🫠"));
    }
}
