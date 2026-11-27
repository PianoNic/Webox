//! Theme values beyond the Momentum colour tokens (docs/theming-engine.md, "Theme layer"): font roles, component colours,
//! lengths (radii, widths) and motion timings. Every value has a stable name (`font.body`, `chat_header.call_button.background`,
//! `card.radius`, `rail.expand`) and a default equal to the literal the UI used before, so the default theme looks exactly as
//! it did. A theme overrides values by name ([`set`]); the overrides are resolved once per theme change, never per frame.
//!
//! Reading is a thread-local array index: no strings, no locks, no allocation. The UI (main window, call windows) runs on one
//! thread, which is the thread that resolves the theme. Font sizes and lengths are stored as
//! `f32` bits; a component colour holds its override (premultiplied RGBA, 0 = none; a fully transparent override is
//! stored with alpha 1) and otherwise falls back to its default, which may refer to a core token of the active theme.
use super::{Th, TokenId};
use eframe::egui::{Color32, FontFamily, FontId};
use std::cell::Cell;

/// Declares one registry: an enum of ids, their names and defaults, the override storage, and the name lookup.
macro_rules! registry {
    ($(#[$m:meta])* $ty:ident, $store:ident, $default_ty:ty, $none:expr; $($(#[$vm:meta])* $id:ident = $name:literal => $def:expr),* $(,)?) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(u16)]
        pub(crate) enum $ty { $($(#[$vm])* $id,)* }
        #[allow(dead_code)] // names and lookup serve the theme package loader
        impl $ty {
            pub(crate) const ALL: &'static [$ty] = &[$($ty::$id,)*];
            /// The value's name in theme files.
            pub(crate) const fn name(self) -> &'static str { [$($name,)*][self as usize] }
            /// The built-in default (the look of the Webex themes).
            pub(crate) const fn default_value(self) -> $default_ty { [$($def,)*][self as usize] }
            pub(crate) fn by_name(n: &str) -> Option<$ty> { Self::ALL.iter().copied().find(|v| v.name() == n) }
        }
        thread_local! { static $store: [Cell<u32>; $ty::ALL.len()] = const { [const { Cell::new($none) }; $ty::ALL.len()] }; }
    };
}

registry! {
    /// Text sizes by role, in points. The roles are a type scale: most of the UI uses `Body` (14) and `Secondary` (13).
    F, FONTS, f32, u32::MAX;
    Caption = "font.caption" => 12.0,
    Small = "font.small" => 12.5,
    Secondary = "font.secondary" => 13.0,
    Label = "font.label" => 13.5,
    Body = "font.body" => 14.0,
    BodyLarge = "font.body_large" => 14.5,
    Subtitle = "font.subtitle" => 15.0,
    Title = "font.title" => 16.0,
    Heading = "font.heading" => 18.0,
    Banner = "font.banner" => 20.0,
    Display = "font.display" => 22.0,
    Hero = "font.hero" => 26.0,
}

impl F {
    /// The size of this role in the active theme.
    #[inline]
    pub(crate) fn px(self) -> f32 {
        let v = FONTS.with(|a| a[self as usize].get());
        if v == u32::MAX { self.default_value() } else { f32::from_bits(v) }
    }
    /// The regular font of this role.
    #[inline]
    pub(crate) fn font(self) -> FontId { FontId::proportional(self.px()) }
    /// The semibold font of this role.
    #[inline]
    pub(crate) fn semibold(self) -> FontId { FontId::new(self.px(), FontFamily::Name("semibold".into())) }
}

registry! {
    /// Lengths in points: corner radii, bar widths. Spacing that density controls stays in `density::Metrics`.
    L, LENGTHS, f32, u32::MAX;
    /// The rounded cards: the space list, the conversation, settings and pages.
    CardRadius = "card.radius" => 10.0,
}

impl L {
    #[inline]
    pub(crate) fn get(self) -> f32 {
        let v = LENGTHS.with(|a| a[self as usize].get());
        if v == u32::MAX { self.default_value() } else { f32::from_bits(v) }
    }
}

registry! {
    /// Animation durations in seconds.
    M, MOTION, f32, u32::MAX;
    /// The rail opening to its labelled width and closing again.
    RailExpand = "motion.rail.expand" => 0.18,
}

impl M {
    #[inline]
    pub(crate) fn secs(self) -> f32 {
        let v = MOTION.with(|a| a[self as usize].get());
        if v == u32::MAX { self.default_value() } else { f32::from_bits(v) }
    }
}

/// The default of a component colour.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Def {
    /// The same in every theme (0xRRGGBB).
    Rgb(u32),
    /// Light themes, dark themes (0xRRGGBB each).
    LightDark(u32, u32),
    /// A core token of the active theme.
    Tok(TokenId),
}

registry! {
    /// Colours of single components that are not a Momentum token.
    C, COLORS, Def, 0;
    /// The ring around the profile picture in the header.
    HeaderAvatarRing = "header.avatar.ring" => Def::LightDark(0x454545, 0xCFCFCF),
}

impl Th {
    /// A component colour of this theme: the theme's override, or the default.
    #[inline]
    pub(crate) fn c(&self, c: C) -> Color32 {
        let v = COLORS.with(|a| a[c as usize].get());
        if v != 0 { return unpack(v); }
        match c.default_value() {
            Def::Rgb(v) => crate::hex(v),
            Def::LightDark(l, d) => crate::hex(if self.light { l } else { d }),
            Def::Tok(id) => self.at(id),
        }
    }
}

#[allow(dead_code)]
fn pack(c: Color32) -> u32 { u32::from_be_bytes(c.to_array()).max(1) }
fn unpack(v: u32) -> Color32 { let [r, g, b, a] = v.to_be_bytes(); Color32::from_rgba_premultiplied(r, g, b, a) }

/// A value a theme sets.
#[allow(dead_code)] // set by the theme package loader
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Value { Px(f32), Color(Color32) }

#[allow(dead_code)]
/// Sets one value by name; false for an unknown name or a value of the wrong kind. Called while a theme is resolved.
pub(crate) fn set(name: &str, v: Value) -> bool {
    match v {
        Value::Px(x) if x.is_finite() && x >= 0.0 => {
            if let Some(f) = F::by_name(name) { FONTS.with(|a| a[f as usize].set(x.to_bits())); return true; }
            if let Some(l) = L::by_name(name) { LENGTHS.with(|a| a[l as usize].set(x.to_bits())); return true; }
            if let Some(m) = M::by_name(name) { MOTION.with(|a| a[m as usize].set(x.to_bits())); return true; }
            false
        }
        Value::Color(c) => C::by_name(name).map(|i| COLORS.with(|a| a[i as usize].set(pack(c)))).is_some(),
        _ => false,
    }
}

#[allow(dead_code)]
/// Back to the built-in defaults (no theme package active).
pub(crate) fn reset() {
    FONTS.with(|a| a.iter().for_each(|c| c.set(u32::MAX)));
    LENGTHS.with(|a| a.iter().for_each(|c| c.set(u32::MAX)));
    MOTION.with(|a| a.iter().for_each(|c| c.set(u32::MAX)));
    COLORS.with(|a| a.iter().for_each(|c| c.set(0)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_namespaced() {
        let mut all: Vec<&str> = F::ALL.iter().map(|v| v.name()).chain(L::ALL.iter().map(|v| v.name()))
            .chain(M::ALL.iter().map(|v| v.name())).chain(C::ALL.iter().map(|v| v.name())).collect();
        assert!(all.iter().all(|n| n.contains('.') && !n.contains(' ')));
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "duplicate value names");
    }

    #[test]
    fn overrides_apply_and_reset() {
        // The store is per thread, and every test runs on its own thread.
        let t = Th::of(crate::ThemeId::Dark);
        assert_eq!(F::Body.px(), 14.0);
        assert!(set("font.body", Value::Px(15.0)));
        assert_eq!(F::Body.px(), 15.0);
        assert!(set("card.radius", Value::Px(14.0)));
        assert_eq!(L::CardRadius.get(), 14.0);
        assert_eq!(t.c(C::HeaderAvatarRing), crate::hex(0xCFCFCF));
        assert!(set("header.avatar.ring", Value::Color(Color32::TRANSPARENT)));
        assert!(t.c(C::HeaderAvatarRing).a() <= 1);
        assert!(!set("font.nope", Value::Px(1.0)));
        assert!(!set("font.body", Value::Px(f32::NAN)));
        assert!(!set("font.body", Value::Color(Color32::RED)));
        reset();
        assert_eq!(F::Body.px(), 14.0);
        assert_eq!(L::CardRadius.get(), 10.0);
        assert_eq!(t.c(C::HeaderAvatarRing), crate::hex(0xCFCFCF));
    }
}
