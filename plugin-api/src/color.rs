//! Colour helpers for design plugins (0xRRGGBBAA values, as in [`crate::Palette`] and [`crate::Accents`]): WCAG 2 relative
//! luminance and contrast, and adjusting a colour's lightness until it reaches a contrast ratio against a background. Text
//! needs 4.5:1 (WCAG AA), other things you have to see (dots, focus rings, control fills) 3:1.

pub const AA_TEXT: f64 = 4.5;
pub const AA_GRAPHIC: f64 = 3.0;

pub const fn rgb(v: u32) -> u32 { (v << 8) | 0xFF }
fn parts(c: u32) -> (f64, f64, f64) { (((c >> 24) & 0xFF) as f64 / 255.0, ((c >> 16) & 0xFF) as f64 / 255.0, ((c >> 8) & 0xFF) as f64 / 255.0) }
fn pack(r: f64, g: f64, b: f64) -> u32 {
    let q = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
    (q(r) << 24) | (q(g) << 16) | (q(b) << 8) | 0xFF
}

/// WCAG relative luminance (alpha ignored).
pub fn luminance(c: u32) -> f64 {
    let lin = |x: f64| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
    let (r, g, b) = parts(c);
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG contrast ratio, 1..21.
pub fn contrast(a: u32, b: u32) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn to_hsl(c: u32) -> (f64, f64, f64) {
    let (r, g, b) = parts(c);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn { return (0.0, 0.0, l); }
    let d = mx - mn;
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == r { (g - b) / d + if g < b { 6.0 } else { 0.0 } } else if mx == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    (h / 6.0, s, l)
}
fn from_hsl(h: f64, s: f64, l: f64) -> u32 {
    if s == 0.0 { return pack(l, l, l); }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 { t += 1.0 } if t > 1.0 { t -= 1.0 }
        if t < 1.0 / 6.0 { p + (q - p) * 6.0 * t } else if t < 0.5 { q } else if t < 2.0 / 3.0 { p + (q - p) * (2.0 / 3.0 - t) * 6.0 } else { p }
    };
    pack(f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// The colour with its HSL lightness moved by `d` (-1..1).
pub fn shift(c: u32, d: f64) -> u32 { let (h, s, l) = to_hsl(c); from_hsl(h, s, (l + d).clamp(0.0, 1.0)) }
/// Whether a colour is (nearly) grey: white, black and grey flag stripes are left out where a hue is needed.
pub fn is_grey(c: u32) -> bool { to_hsl(c).1 < 0.12 || { let l = to_hsl(c).2; !(0.04..=0.97).contains(&l) } }

/// `c`, made lighter (on a dark background) or darker (on a light one) in small steps until it has `min` contrast against `bg`.
pub fn readable(c: u32, bg: u32, min: f64) -> u32 {
    let dir = if luminance(bg) < 0.18 { 0.02 } else { -0.02 };
    let mut x = c;
    for _ in 0..60 { if contrast(x, bg) >= min { return x; } x = shift(x, dir); }
    x
}

/// Black or white, whichever reads better on `fill`.
pub fn text_on(fill: u32) -> u32 { if contrast(fill, 0x000000FF) >= contrast(fill, 0xFFFFFFFF) { 0x000000FF } else { 0xFFFFFFFF } }

/// A fill for text: (fill, text) where the text (black or white) has `min` contrast on the fill, the fill moved away from it.
pub fn fill_for_text(c: u32, min: f64) -> (u32, u32) {
    let t = text_on(c);
    let dir = if t == 0xFFFFFFFF { -0.02 } else { 0.02 };
    let mut x = c;
    for _ in 0..60 { if contrast(x, t) >= min { break; } x = shift(x, dir); }
    (x, t)
}

/// `a` mixed towards `b` by `k` (0 = a, 1 = b).
pub fn mix(a: u32, b: u32, k: f64) -> u32 {
    let ((r1, g1, b1), (r2, g2, b2)) = (parts(a), parts(b));
    pack(r1 + (r2 - r1) * k, g1 + (g2 - g1) * k, b1 + (b2 - b1) * k)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contrast_and_adjust() {
        assert!((contrast(0x000000FF, 0xFFFFFFFF) - 21.0).abs() < 1e-9);
        let dark_bg = rgb(0x1A1A1A);
        let v = readable(rgb(0x750787), dark_bg, AA_TEXT);
        assert!(contrast(v, dark_bg) >= AA_TEXT);
        let light_bg = rgb(0xFFFFFF);
        let y = readable(rgb(0xFFED00), light_bg, AA_TEXT);
        assert!(contrast(y, light_bg) >= AA_TEXT);
        let (f, t) = fill_for_text(rgb(0xFF8C00), AA_TEXT);
        assert!(contrast(f, t) >= AA_TEXT);
        assert!(is_grey(0xFFFFFFFF) && is_grey(rgb(0xA3A3A3)) && !is_grey(rgb(0xE40303)));
        assert_eq!(shift(rgb(0x808080), 0.0), rgb(0x808080));
    }
}
