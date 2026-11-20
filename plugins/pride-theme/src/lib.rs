//! Pride Theme: the colours of a Pride flag on Webox's accents (links, focus, selection, unread dots, badges, the selected rail
//! item, primary buttons, toggles / checkboxes / sliders, mentions, the scrollbar and the composer's focus line, there as a
//! gradient). Backgrounds and body text stay the base theme's, so it works on every theme, dark and light. Every colour is moved
//! in lightness just enough for WCAG AA on the active theme: 4.5:1 for text and for text on a fill, 3:1 for dots, rings and
//! control fills.
use collabhost_plugin_api::color::{contrast, fill_for_text, is_grey, mix, readable, rgb, text_on, AA_GRAPHIC, AA_TEXT};
use collabhost_plugin_api::{export_plugin, Accents, Plugin, Schema, ThemeInfo, Values, GRADIENT_MAX};

/// (setting value, label, stripes top to bottom as 0xRRGGBB): the flags' standard colours.
pub const FLAGS: [(&str, &str, &[u32]); 8] = [
    ("rainbow", "Regenbogen", &[0xE40303, 0xFF8C00, 0xFFED00, 0x008026, 0x004DFF, 0x750787]),
    // Progress Pride: the rainbow plus the chevron (white, pink, light blue, brown, black)
    ("progress", "Progress Pride", &[0xE40303, 0xFF8C00, 0xFFED00, 0x008026, 0x004DFF, 0x750787, 0xFFFFFF, 0xF5A9B8, 0x5BCEFA, 0x613915, 0x000000]),
    ("trans", "Trans", &[0x5BCEFA, 0xF5A9B8, 0xFFFFFF, 0xF5A9B8, 0x5BCEFA]),
    ("bi", "Bi", &[0xD60270, 0xD60270, 0x9B4F96, 0x0038A8, 0x0038A8]),
    ("lesbian", "Lesbisch", &[0xD52D00, 0xEF7627, 0xFF9A56, 0xFFFFFF, 0xD162A4, 0xB55690, 0xA30262]),
    ("pan", "Pan", &[0xFF218C, 0xFFD800, 0x21B1FF]),
    ("nonbinary", "Nicht-binär", &[0xFCF434, 0xFFFFFF, 0x9C59D1, 0x2C2C2C]),
    ("ace", "Asexuell", &[0x000000, 0xA3A3A3, 0xFFFFFF, 0x800080]),
];

struct Pride { flag: usize, gradient: bool }
impl Default for Pride { fn default() -> Self { Pride { flag: 0, gradient: true } } }

/// The distinct coloured (not grey) stripes, in flag order.
fn hues(stripes: &[u32]) -> Vec<u32> {
    let mut v: Vec<u32> = vec![];
    for s in stripes.iter().map(|s| rgb(*s)).filter(|s| !is_grey(*s)) { if !v.contains(&s) { v.push(s) } }
    if v.is_empty() { v.push(rgb(stripes[0])) }
    v
}

/// The accents for one flag on one base theme.
pub fn accents_for(stripes: &[u32], th: &ThemeInfo, gradient: bool) -> Accents {
    let bg = th.background;
    let h = hues(stripes);
    // Six roles spread over the flag's colours: on the rainbow red, orange, yellow, green, blue, violet.
    let pick = |k: usize| h[(k * (h.len() - 1) + 2) / 5];
    let (unread, rail, mention, button, link, focus) = (pick(0), pick(1), pick(2), pick(3), pick(4), pick(5));
    let (badge, badge_text) = fill_for_text(readable(unread, bg, AA_GRAPHIC), AA_TEXT);
    let (button, button_text) = fill_for_text(button, AA_TEXT);
    let control = readable(focus, bg, AA_GRAPHIC);
    // the tick / dot on a checked control: black or white, the control moved until it reads (3:1 for a graphic)
    let (control, control_icon) = { let (f, t) = fill_for_text(control, AA_GRAPHIC); if contrast(f, bg) >= AA_GRAPHIC { (f, t) } else { (control, text_on(control)) } };
    // The selected rail circle carries the rail's icon in the primary text colour: tint it only as far as that stays readable.
    let rail_selected = (1..=12).rev().map(|i| mix(bg, rail, i as f64 * 0.05)).find(|c| contrast(*c, th.text) >= AA_TEXT).unwrap_or(bg);
    let mut a = Accents {
        accent: control, link: readable(link, bg, AA_TEXT), focus: readable(focus, bg, AA_GRAPHIC), selection: readable(link, bg, AA_GRAPHIC),
        unread: readable(unread, bg, AA_GRAPHIC), badge, badge_text, rail_selected, button, button_text, control, control_icon,
        mention: readable(mention, bg, AA_TEXT), scrollbar_hover: readable(rail, bg, AA_GRAPHIC), ..Default::default()
    };
    if gradient {
        let stops: Vec<u32> = stripes.iter().map(|s| readable(rgb(*s), bg, AA_GRAPHIC)).take(GRADIENT_MAX).collect();
        a.gradient_len = stops.len() as u32;
        a.gradient[..stops.len()].copy_from_slice(&stops);
    }
    a
}

impl Plugin for Pride {
    const NAME: &'static str = "Pride Theme";
    const AUTHOR: &'static str = "Webox examples";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = env!("CARGO_PKG_DESCRIPTION");

    fn settings(&self) -> Option<Schema> {
        let opts: Vec<(&str, &str)> = FLAGS.iter().map(|(v, l, _)| (*v, *l)).collect();
        Some(Schema::new()
            .section("Farben", "Hintergründe und Text bleiben die des gewählten Themas; nur Akzente werden eingefärbt.")
            .choice("flag", "Flagge", "Deren Farben auf Links, Markierungen, Schaltflächen und Schaltern.", "rainbow", &opts)
            .toggle("gradient", "Verlauf als Akzent", "Die Fokuslinie über dem Eingabefeld als Verlauf in allen Farben der Flagge.", true))
    }
    fn settings_changed(&mut self, v: &Values) {
        self.flag = v.text("flag").and_then(|f| FLAGS.iter().position(|x| x.0 == f)).unwrap_or(0);
        self.gradient = v.bool("gradient").unwrap_or(true);
    }
    fn accents(&mut self, th: &ThemeInfo) -> Option<Accents> {
        // a host that sent no colours (e.g. a capability probe): assume the default dark card
        let th = if th.background == 0 { ThemeInfo { background: rgb(0x262626), text: rgb(0xF7F7F7), ..*th } } else { *th };
        Some(accents_for(FLAGS[self.flag].2, &th, self.gradient))
    }
}

export_plugin!(Pride);

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(light: bool) -> ThemeInfo {
        if light { ThemeInfo { light: 1, background: rgb(0xFAFAFA), surface: rgb(0xF0F0F0), text: rgb(0x121212), ..Default::default() } }
        else { ThemeInfo { light: 0, background: rgb(0x262626), surface: rgb(0x1A1A1A), text: rgb(0xF7F7F7), ..Default::default() } }
    }

    #[test]
    fn every_flag_is_readable_on_dark_and_light() {
        for (name, _, stripes) in FLAGS {
            for light in [false, true] {
                let th = theme(light);
                let a = accents_for(stripes, &th, true);
                let bg = th.background;
                let ctx = format!("{name} light={light}");
                assert!(contrast(a.link, bg) >= AA_TEXT, "link {ctx}");
                assert!(contrast(a.mention, bg) >= AA_TEXT, "mention {ctx}");
                assert!(contrast(a.button, a.button_text) >= AA_TEXT, "button {ctx}");
                assert!(contrast(a.badge, a.badge_text) >= AA_TEXT, "badge {ctx}");
                for (role, c) in [("focus", a.focus), ("unread", a.unread), ("control", a.control), ("scrollbar", a.scrollbar_hover)] {
                    assert!(contrast(c, bg) >= AA_GRAPHIC, "{role} {ctx}");
                }
                assert!(contrast(a.rail_selected, th.text) >= AA_TEXT, "rail icon {ctx}");
                assert!(a.gradient_len >= 3 && a.gradient_len as usize <= GRADIENT_MAX, "gradient {ctx}");
            }
        }
    }

    #[test]
    fn rainbow_spreads_its_six_colours() {
        let a = accents_for(FLAGS[0].2, &theme(false), false);
        assert_eq!(a.gradient_len, 0, "no gradient when switched off");
        let roles = [a.unread, a.scrollbar_hover, a.mention, a.button, a.link, a.focus];
        let distinct: std::collections::HashSet<u32> = roles.iter().copied().collect();
        assert_eq!(distinct.len(), 6);
    }

    #[test]
    fn settings_pick_the_flag() {
        let mut p = Pride::default();
        assert!(p.settings().unwrap().to_json().contains("\"value\":\"ace\""));
        p.settings_changed(&Values::parse(r#"{"flag":"trans","gradient":false}"#));
        assert_eq!((FLAGS[p.flag].0, p.gradient), ("trans", false));
        p.settings_changed(&Values::parse(r#"{"flag":"nope"}"#));
        assert_eq!(p.flag, 0);
        assert!(p.accents(&ThemeInfo::default()).is_some());
    }
}
