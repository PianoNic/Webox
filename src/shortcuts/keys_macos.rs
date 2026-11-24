//! Shortcut texts the macOS way: modifier symbols in Apple's order (⌃ ⌥ ⇧ ⌘), no "+", and a binding's Ctrl shown as ⌘ (the
//! app matches Cmd for it, `Combo::matches_at`). `display` for the bindings, `hint` for the fixed texts in menus and tooltips
//! ("Strg+Umschalt+P" -> "⇧⌘P").
use super::Combo;
use eframe::egui::Key;

fn key_text(k: Key) -> String {
    match k {
        Key::Enter => "↩".into(), Key::Backspace => "⌫".into(), Key::Delete => "⌦".into(), Key::Escape => "⎋".into(), Key::Tab => "⇥".into(),
        Key::ArrowLeft => "←".into(), Key::ArrowRight => "→".into(), Key::ArrowUp => "↑".into(), Key::ArrowDown => "↓".into(),
        Key::PageUp => "⇞".into(), Key::PageDown => "⇟".into(), Key::Home => "↖".into(), Key::End => "↘".into(), Key::Space => "Leertaste".into(),
        k => super::key_name_de(k),
    }
}

fn symbols(ctrl: bool, alt: bool, shift: bool, cmd: bool) -> String {
    [(ctrl, "⌃"), (alt, "⌥"), (shift, "⇧"), (cmd, "⌘")].iter().filter(|(on, _)| *on).map(|(_, s)| *s).collect()
}

pub(crate) fn display(c: &Combo) -> String { format!("{}{}", symbols(false, c.alt, c.shift, c.ctrl), key_text(c.key)) }

/// Rewrites every "Strg+…" / "Ctrl+…" / "Alt+…" / "Umschalt+…" / "Shift+…" combination in `s`.
pub(crate) fn hint(s: &str) -> String {
    const MODS: [(&str, u8); 5] = [("Strg+", 0), ("Ctrl+", 0), ("Alt+", 1), ("Umschalt+", 2), ("Shift+", 2)];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    'outer: while !rest.is_empty() {
        // A combination starts at a modifier that begins a word.
        let at_word = out.is_empty() || out.ends_with([' ', '(', '"', '„']);
        if at_word {
            let (mut m, mut r) = ([false; 3], rest);
            while let Some((p, i)) = MODS.iter().find(|(p, _)| r.starts_with(p)) { m[*i as usize] = true; r = &r[p.len()..]; }
            if m.iter().any(|x| *x) && !r.is_empty() && !r.starts_with(' ') {
                out.push_str(&symbols(false, m[1], m[2], m[0]));
                // "Leertaste", "Q", "Pfeil nach oben" stay as they are; "+" is the key itself in "Strg++"
                rest = r;
                continue 'outer;
            }
        }
        let ch = rest.chars().next().unwrap_or_default();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_written_the_macos_way() {
        let c = |s: &str| Combo::parse(s).unwrap();
        assert_eq!(c("Ctrl+Shift+N").display(), "⇧⌘N");
        assert_eq!(c("Ctrl+Alt+Shift+D").display(), "⌥⇧⌘D");
        assert_eq!(c("Alt+LeftArrow").display(), "⌥←");
        assert_eq!(c("PgUp").display(), "⇞");
        assert_eq!(hint("Strg+Umschalt+P"), "⇧⌘P");
        assert_eq!(hint("Fett (Strg+B)"), "Fett (⌘B)");
        assert_eq!(hint("Emoticons und Symbole (Strg+Umschalt+Leertaste)"), "Emoticons und Symbole (⇧⌘Leertaste)");
        assert_eq!(hint("Alt+Umschalt+Q"), "⌥⇧Q");
        assert_eq!(hint("Umschalt+Pfeil nach oben"), "⇧Pfeil nach oben");
        assert_eq!(hint("Ctrl+F"), "⌘F");
        assert_eq!(hint("Mit der Tastenkombination Strg+Umschalt+X können Sie"), "Mit der Tastenkombination ⇧⌘X können Sie");
        assert_eq!(hint("Strg++"), "⌘+");
        assert_eq!(hint("Kein Kürzel, Alt+ allein"), "Kein Kürzel, Alt+ allein");
    }
}
