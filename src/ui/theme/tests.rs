use super::*;

fn raw(theme: ThemeId, name: &str) -> u32 {
    tokens::TOKENS.iter().find(|(n, _)| *n == name).map(|(_, v)| v[theme.idx()]).unwrap_or_else(|| panic!("{name} not generated"))
}

#[test]
fn table_is_sorted_for_binary_search() {
    assert!(tokens::TOKENS.windows(2).all(|w| w[0].0 < w[1].0));
}

/// No dark leftovers: wherever the light theme differs from its dark one, the field differs too - for every
/// dark / light pair of themes.
#[test]
fn every_field_follows_its_token_in_all_themes() {
    for pair in ThemeId::ALL.chunks(2) {
        let (d, l) = (Th::of(pair[0]), Th::of(pair[1]));
        for (f, tok) in FIELDS {
            assert_eq!(d.field(f), super::tok(pair[0], tok), "{f}");
            assert_eq!(l.field(f), super::tok(pair[1], tok), "{f}");
            if raw(pair[0], tok) != raw(pair[1], tok) { assert_ne!(d.field(f), l.field(f), "{f} ({tok}) has the dark value in {:?}", pair[1]); }
        }
    }
}

/// The light surfaces of the "Light" theme.
#[test]
fn light_matches_the_official_light_screenshot() {
    let t = Th::of(ThemeId::Light);
    let over = |c: Color32, bg: u8| { let a = c.a() as f32 / 255.0; (c.r() as f32 + bg as f32 * (1.0 - a)).round() as u8 };
    assert_eq!((t.win, t.win_b), (crate::hex(0xF0F0F0), crate::hex(0xFFFFFF))); // rail column
    assert_eq!((t.card, t.card_b), (crate::hex(0xFAFAFA), crate::hex(0xFFFFFF))); // list card at the top
    assert_eq!(t.search, crate::hex(0xDEDEDE)); // header search pill
    assert_eq!(over(t.rail_sel, 0xF2), 0xC2); // selected rail circle on #F2F2F2
    assert_eq!(over(t.tab_on, 0xFB), 0xC9); // "All" pill on #FBFBFB
    assert_eq!(t.avatar, crate::hex(0x545454)); // letter avatars
    assert_eq!(t.star, crate::hex(0x7D4705)); // favourites star
    assert_eq!(over(t.white_btn, 0xFB), 0x0D); // dark "+" of the list card
    assert_eq!(t.tk("modal-decorativeTitle-slate"), crate::hex(0x8C91BD)); // "Public"
    assert_eq!(t.tk("wx-leftpane-shortcut-app-icon-orange"), crate::hex(0x914017)); // team text
}

#[test]
fn dark_matches_the_official_dark_screenshots() {
    let t = Th::of(ThemeId::Dark);
    assert_eq!((t.win, t.win_b), (crate::hex(0x1A1A1A), crate::hex(0x0F0F0F)));
    assert_eq!((t.card, t.card_b), (crate::hex(0x262626), crate::hex(0x0F0F0F)));
    assert_eq!(t.selected, crate::hex(0x3B3B3B));
    assert_eq!((t.text1, t.text2, t.accent, t.link), (crate::hex(0xF7F7F7), crate::hex(0xB2B2B2), crate::hex(0x07C1F5), crate::hex(0x64B4FA)));
}

#[test]
fn ids_round_trip_and_old_values_are_read() {
    for id in ThemeId::ALL { assert_eq!(ThemeId::from_key(id.key()), Some(id)); }
    assert_eq!(ThemeId::from_key("light"), Some(ThemeId::Light));
    assert_eq!(ThemeId::from_key("dark"), Some(ThemeId::Dark));
    assert_eq!(ThemeId::JadeLight.dark_variant(), ThemeId::JadeDark);
    assert!(ThemeId::ALL.iter().filter(|t| t.is_dark()).count() == 5);
}

/// Every `tk("...")` in the sources is in the generated table (else it would draw magenta).
#[test]
fn every_used_token_is_generated() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root];
    let re_start = "tk(\"";
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p); continue; }
            if p.extension().is_none_or(|x| x != "rs") { continue; }
            let s = std::fs::read_to_string(&p).unwrap();
            let mut rest = s.as_str();
            while let Some(i) = rest.find(re_start) {
                let after = &rest[i + re_start.len()..];
                let end = after.find('"').unwrap();
                let name = &after[..end];
                let ok_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':';
                if !name.is_empty() && name.chars().all(ok_char) && name.contains('-') {
                    assert!(tokens::TOKENS.iter().any(|(n, _)| *n == name), "{} uses {name}, not generated", p.display());
                }
                rest = &after[end..];
            }
        }
    }
}

/// The theme preview tiles: dark themes have a dark tile and a white outline, light ones the reverse.
#[test]
fn previews_follow_the_theme_kind() {
    for id in ThemeId::ALL {
        let p = id.preview();
        assert_eq!(p[0].r() < 128, id.is_dark(), "{id:?}");
        assert_eq!(p[4] == Color32::WHITE, id.is_dark(), "{id:?}");
    }
}
