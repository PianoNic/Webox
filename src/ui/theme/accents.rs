//! Accent themes (plugin-api 1.3 `Accents`): which theme fields and tokens each accent role replaces. Backgrounds and body text
//! are never touched, so an accent theme works on every base theme. The plugin gets the base theme first (`info`) and picks
//! readable values itself; a 0 keeps Webox's colour.
use super::Th;
use collabhost_plugin_api::{Accents, ThemeInfo};
use eframe::egui::Color32;

fn pack(c: Color32) -> u32 { u32::from_be_bytes([c.r(), c.g(), c.b(), 0xFF]) }

/// What the plugin is told about the active base theme.
pub(crate) fn info(t: &Th) -> ThemeInfo {
    ThemeInfo { size: std::mem::size_of::<ThemeInfo>() as u32, light: t.light as u32, background: pack(t.card), surface: pack(t.win), text: pack(t.text1) }
}

/// What applying accents gives besides the changed fields of `t`.
pub(crate) struct Applied { pub overrides: Vec<(&'static str, Color32)>, pub gradient: Vec<Color32>, pub selection: Option<Color32> }

/// Puts `a` into `t` (fields) and returns the token overrides, the gradient and the selection colour.
pub(crate) fn apply(t: &mut Th, a: &Accents) -> Applied {
    let c = |v: u32| crate::rgba(v);
    let hover = |x: Color32, k: f32| x.lerp_to_gamma(if t.light { Color32::BLACK } else { Color32::WHITE }, k);
    let mut ov: Vec<(&'static str, Color32)> = vec![];
    let set = |f: &mut Color32, v: u32| if let Some(x) = c(v) { *f = x };
    set(&mut t.accent, a.accent); // toggles, the composer line, small accents
    set(&mut t.link, a.link);
    set(&mut t.focus, a.focus);
    set(&mut t.unread, a.unread);
    set(&mut t.rail_sel, a.rail_selected);
    set(&mut t.white_btn, a.button);
    set(&mut t.white_btn_text, a.button_text);
    set(&mut t.mention, a.mention);
    if let Some(x) = c(a.mention) { ov.push(("message-bodyText-mention-self-text", x)); }
    if let Some(x) = c(a.badge) { ov.push(("appNav-badge", x)); }
    if let Some(x) = c(a.button) {
        ov.extend([("button-primary-background:hovered", hover(x, 0.15)), ("button-primary-background:pressed", hover(x, 0.3))]);
    }
    if let Some(x) = c(a.button_text) { ov.push(("button-primary-text", x)); }
    if let Some(x) = c(a.control) {
        ov.extend([("checkbox-checked-background", x), ("checkbox-checked-background:hovered", hover(x, 0.15)),
            ("checkbox-checked-background:pressed", hover(x, 0.3)), ("slider-track-indicator-background", x)]);
    }
    if let Some(x) = c(a.control_icon) { ov.push(("checkbox-checked-icon", x)); }
    if let Some(x) = c(a.focus) { ov.extend([("input-focusRing", x), ("global-focus-outline", x)]); }
    if let Some(x) = c(a.scrollbar_hover) { ov.push(("scrollbar-thumb-background:hovered", x)); }
    let n = (a.gradient_len as usize).min(a.gradient.len());
    let gradient: Vec<Color32> = a.gradient[..n].iter().filter_map(|v| c(*v)).collect();
    Applied { overrides: ov, gradient, selection: c(a.selection) }
}

/// A 1 pt horizontal line in the accent gradient when there is one, else in `fallback`.
pub(crate) fn hline(p: &eframe::egui::Painter, x: eframe::egui::Rangef, y: f32, width: f32, fallback: Color32) {
    use eframe::egui::{epaint::{Mesh, Vertex, WHITE_UV}, pos2, Shape, Stroke};
    let Some(g) = super::accent_gradient() else { p.hline(x, y, Stroke::new(width, fallback)); return };
    let mut m = Mesh::default();
    let (t, b) = (y - width / 2.0, y + width / 2.0);
    for (i, col) in g.iter().enumerate() {
        let px = x.min + x.span() * i as f32 / (g.len() - 1) as f32;
        m.vertices.push(Vertex { pos: pos2(px, t), uv: WHITE_UV, color: *col });
        m.vertices.push(Vertex { pos: pos2(px, b), uv: WHITE_UV, color: *col });
        if i > 0 { let k = (i as u32) * 2; m.add_triangle(k - 2, k - 1, k); m.add_triangle(k - 1, k, k + 1); }
    }
    p.add(Shape::mesh(m));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::ThemeId;

    #[test]
    fn accents_touch_only_accent_roles() {
        let base = Th::of(ThemeId::Dark);
        let mut t = base;
        let a = Accents { accent: 0xE40303FF, link: 0x004DFFFF, button: 0x008026FF, button_text: 0xFFFFFFFF, control: 0x750787FF,
            badge: 0xFF8C00FF, gradient_len: 2, gradient: [0xE40303FF, 0x750787FF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], ..Default::default() };
        let r = apply(&mut t, &a);
        assert_eq!((t.accent, t.link, t.white_btn), (crate::hex(0xE40303), crate::hex(0x004DFF), crate::hex(0x008026)));
        assert_eq!((t.text1, t.text2, t.card, t.win, t.badge), (base.text1, base.text2, base.card, base.win, base.badge), "backgrounds, body text and the hang-up red stay");
        assert_eq!(t.focus, base.focus, "0 keeps the theme's colour");
        assert!(r.overrides.iter().any(|(n, c)| *n == "appNav-badge" && *c == crate::hex(0xFF8C00)));
        assert!(r.overrides.iter().any(|(n, _)| *n == "checkbox-checked-background"));
        assert_eq!(r.gradient.len(), 2);
        crate::ui::theme::set_accent_overrides(r.overrides, r.gradient);
        assert_eq!(base.tk("appNav-badge"), crate::hex(0xFF8C00));
        assert_eq!(Th::of(ThemeId::Dark).badge, base.badge, "Th::of never reads the overrides");
        assert!(crate::ui::theme::accent_gradient().is_some());
        crate::ui::theme::set_accent_overrides(vec![], vec![]);
        assert_ne!(base.tk("appNav-badge"), crate::hex(0xFF8C00));
    }
}
