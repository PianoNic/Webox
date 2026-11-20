//! Themes: the ten selectable Momentum themes (Settings > Erscheinungsbild > Themen) and the `Th` palette built from their
//! colour tokens.
//!
//! A Momentum theme names a parent (MomentumBronzeDark -> BronzeDark -> Dark -> Default); every token this crate uses is
//! resolved through that chain into the generated `tokens.rs`. The ids are the stored theme names
//! ("MomentumDark", ...); the display names come from the language pack ("Default Dark", "Bronze Dark", ...).
//!
//! Colours: `Th` fields are tokens (table below, one line per field); everything else asks `t.tk("token")` /
//! `t.tk("token:state")`. Alpha tokens (`#FFFFFF` at 7 %, ...) come back premultiplied, composited like Qt does (`crate::alpha`).
use eframe::egui::Color32;

pub(crate) mod accents;
mod tokens;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub(crate) enum ThemeId {
    #[default] Dark, Light, BronzeDark, BronzeLight, JadeDark, JadeLight, LavenderDark, LavenderLight, IndigoDark, IndigoLight,
    /// Not offered by the picker; selectable by id only.
    RoseDark, RoseLight,
    /// "OS High Contrast": used while Windows high contrast is on, and the picker is disabled then ("To choose this theme, first
    /// turn off high contrast in Windows settings."). Never stored.
    HighContrast,
}

impl ThemeId {
    /// Picker order: Standard Dunkel, Hell, Bronze dunkel, Bronze hell, Jade dunkel, Jade hell, Lavendel dunkel, Lavendel hell,
    /// Dunkles Indigoblau, Helles Indigoblau.
    pub(crate) const ALL: [ThemeId; 10] = [ThemeId::Dark, ThemeId::Light, ThemeId::BronzeDark, ThemeId::BronzeLight, ThemeId::JadeDark,
        ThemeId::JadeLight, ThemeId::LavenderDark, ThemeId::LavenderLight, ThemeId::IndigoDark, ThemeId::IndigoLight];
    /// Every theme of the token table (column order).
    pub(crate) const EVERY: [ThemeId; 13] = [ThemeId::Dark, ThemeId::Light, ThemeId::BronzeDark, ThemeId::BronzeLight, ThemeId::JadeDark,
        ThemeId::JadeLight, ThemeId::LavenderDark, ThemeId::LavenderLight, ThemeId::IndigoDark, ThemeId::IndigoLight, ThemeId::RoseDark,
        ThemeId::RoseLight, ThemeId::HighContrast];
    pub(crate) fn idx(self) -> usize { self as usize }
    /// The stored id (the theme name; settings key `theme`).
    pub(crate) fn key(self) -> &'static str {
        ["MomentumDark", "MomentumDefault", "MomentumBronzeDark", "MomentumBronzeLight", "MomentumJadeDark", "MomentumJadeLight",
         "MomentumLavenderDark", "MomentumLavenderLight", "MomentumIndigoDark", "MomentumIndigoLight", "MomentumRoseDark", "MomentumRoseLight",
         "MomentumSystemHighContrast"][self.idx()]
    }
    /// English source of the pack string (translated through the pack).
    pub(crate) fn english(self) -> &'static str {
        ["Default Dark", "Light", "Bronze Dark", "Bronze Light", "Jade Dark", "Jade Light", "Lavender Dark", "Lavender Light", "Indigo Dark",
         "Indigo Light", "Rose Dark", "Rose Light", "OS High Contrast"][self.idx()]
    }
    /// The German names, used when no pack is installed.
    pub(crate) fn german(self) -> &'static str {
        ["Standard Dunkel", "Hell", "Bronze dunkel", "Bronze hell", "Jade dunkel", "Jade hell", "Lavendel dunkel", "Lavendel hell",
         "Dunkles Indigoblau", "Helles Indigoblau", "Rose dunkel", "Rose hell", "Hoher Kontrast des Betriebssystems"][self.idx()]
    }
    pub(crate) fn label(self) -> String {
        match crate::i18n::lookup_any(self.english()) { Some(s) => s.to_string(), None => self.german().to_string() }
    }
    pub(crate) fn is_dark(self) -> bool {
        match self { ThemeId::HighContrast => hc::window_is_dark(), _ => self.idx() % 2 == 0 }
    }
    /// The dark theme of the same colour family (used by "Immer dunkles Thema verwenden" for calls).
    pub(crate) fn dark_variant(self) -> ThemeId { if self == ThemeId::HighContrast { self } else { ThemeId::EVERY[self.idx() & !1] } }
    /// Parses a stored value: the theme ids, and the older Webox values "dark" / "light" (kept readable; "system" = None,
    /// handled by the caller).
    pub(crate) fn from_key(s: &str) -> Option<ThemeId> {
        match s {
            "dark" => Some(ThemeId::Dark),
            "light" => Some(ThemeId::Light),
            _ => ThemeId::EVERY.into_iter().find(|t| t.key() == s && *t != ThemeId::HighContrast),
        }
    }
    /// The preview tile colours: bg top, bg bottom, card top, card bottom, outline.
    pub(crate) fn preview(self) -> [Color32; 5] { tokens::PREVIEW[self.idx().min(tokens::PREVIEW.len() - 1)].map(crate::hex) }
}

/// A token of `theme`, `name` = "token" (normal state) or "token:state". Unknown names (not generated) are magenta.
pub(crate) fn tok(theme: ThemeId, name: &str) -> Color32 {
    let Ok(i) = tokens::TOKENS.binary_search_by(|(n, _)| (*n).cmp(name)) else {
        debug_assert!(false, "theme token {name} missing from tokens.rs");
        return Color32::from_rgb(255, 0, 255);
    };
    if theme == ThemeId::HighContrast {
        if let Ok(j) = tokens::SYS.binary_search_by(|(n, _)| (*n).cmp(name)) {
            let s = tokens::SYS[j].1;
            return if s == 255 { rgba32(tokens::TOKENS[i].1[ThemeId::Light.idx()]) } else { hc::sys_colour(tokens::SYS_NAMES[s as usize]) };
        }
    }
    rgba32(tokens::TOKENS[i].1[theme.idx()])
}

// Accent-theme plugins (plugin-api 1.3 `Accents`) replace a few tokens and may give a gradient for the composer's focus line.
// Only `Th::tk` reads the overrides, never `Th::of`: fields resolved from the same token (e.g. `badge`, which also paints the
// red hang-up buttons) keep the theme's colour unless apply_theme sets them on purpose.
thread_local! {
    static OVERRIDES: std::cell::RefCell<Vec<(&'static str, Color32)>> = const { std::cell::RefCell::new(Vec::new()) };
    static GRADIENT: std::cell::RefCell<Vec<Color32>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// Replaces the token overrides (empty = none) and the accent gradient (fewer than 2 stops = none). Set every frame by apply_theme.
pub(crate) fn set_accent_overrides(map: Vec<(&'static str, Color32)>, gradient: Vec<Color32>) {
    OVERRIDES.with(|o| *o.borrow_mut() = map);
    GRADIENT.with(|g| *g.borrow_mut() = gradient);
}
/// The override of a token, if an accent theme set one.
pub(crate) fn over(name: &str) -> Option<Color32> { OVERRIDES.with(|o| o.borrow().iter().find(|(n, _)| *n == name).map(|x| x.1)) }
/// The accent gradient's stops (at least 2) while an accent theme gives one.
pub(crate) fn accent_gradient() -> Option<Vec<Color32>> { GRADIENT.with(|g| (g.borrow().len() >= 2).then(|| g.borrow().clone())) }

/// `hc::active()`, asked at most every 2 s (the OS setting is followed while running).
pub(crate) fn hc_cached(ctx: &eframe::egui::Context) -> bool {
    let id = eframe::egui::Id::new("theme-hc");
    let now = ctx.input(|i| i.time);
    if let Some((at, on)) = ctx.data(|d| d.get_temp::<(f64, bool)>(id)) { if now - at < 2.0 { return on; } }
    let on = hc::active();
    ctx.data_mut(|d| d.insert_temp(id, (now, on)));
    on
}

/// Windows high contrast: whether it is on, and the system colours the high-contrast theme refers to.
pub(crate) mod hc {
    use eframe::egui::Color32;
    #[cfg(windows)]
    pub(crate) fn active() -> bool {
        use windows::Win32::UI::Accessibility::{HIGHCONTRASTW, HCF_HIGHCONTRASTON};
        use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
        let mut h = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
        let ok = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, h.cbSize, Some(&mut h as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
        ok.is_ok() && h.dwFlags.contains(HCF_HIGHCONTRASTON)
    }
    #[cfg(not(windows))]
    pub(crate) fn active() -> bool { false }
    /// A Windows system colour by the name the theme uses (Qt's QPalette roles: WindowColor = COLOR_WINDOW, ...).
    #[cfg(windows)]
    pub(crate) fn sys_colour(name: &str) -> Color32 {
        use windows::Win32::Graphics::Gdi::{GetSysColor, SYS_COLOR_INDEX};
        let i = match name {
            "WindowColor" => 5, "WindowTextColor" => 8, "HighlightColor" => 13, "HighlightTextColor" => 14, "ButtonFaceColor" => 15,
            "ButtonTextColor" => 18, "GrayTextColor" | "PlaceholderColor" => 17, "HotlightColor" => 26, _ => 8,
        };
        let c = unsafe { GetSysColor(SYS_COLOR_INDEX(i)) };
        Color32::from_rgb(c as u8, (c >> 8) as u8, (c >> 16) as u8)
    }
    #[cfg(not(windows))]
    pub(crate) fn sys_colour(name: &str) -> Color32 { if name.contains("Text") { Color32::WHITE } else { Color32::BLACK } }
    pub(crate) fn window_is_dark() -> bool { let c = sys_colour("WindowColor"); (c.r() as u32 + c.g() as u32 + c.b() as u32) < 384 }
}
/// 0xRRGGBBAA -> premultiplied colour, composited like Qt (see `crate::alpha`).
fn rgba32(v: u32) -> Color32 {
    let c = Color32::from_rgb((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8);
    let a = v as u8;
    if a == 255 { c } else { crate::alpha(c, a as f32 / 255.0) }
}

macro_rules! th {
    ($($(#[$m:meta])* $f:ident = $tok:literal),* $(,)?) => {
        /// The palette of the active theme. Every colour field is the token named in `th!` below; `light` and `id` tell
        /// code that has to choose between shapes (not colours) which kind of theme is active.
        #[derive(Clone, Copy, PartialEq, Debug)]
        pub(crate) struct Th { pub(crate) id: ThemeId, pub(crate) light: bool, $($(#[$m])* pub(crate) $f: Color32,)* }
        impl Th {
            pub(crate) fn of(id: ThemeId) -> Th { Th { id, light: !id.is_dark(), $($f: tok(id, $tok),)* } }
        }
        /// (field, token) of every colour field, for the tests.
        #[cfg(test)]
        pub(crate) const FIELDS: &[(&str, &str)] = &[$((stringify!($f), $tok),)*];
        #[cfg(test)]
        impl Th { pub(crate) fn field(&self, f: &str) -> Color32 { match f { $(stringify!($f) => self.$f,)* _ => panic!("no field {f}") } } }
    };
}

th! {
    /// Window background: theme-gradient-primary, top (dark #1A1A1A -> #0F0F0F, light #F0F0F0 -> #FFFFFF).
    win = "theme-gradient-primary-0-background",
    win_b = "theme-gradient-primary-1-background",
    /// Header row: on the window gradient, its top colour.
    top = "theme-gradient-primary-0-background",
    /// List card and every other content card: theme-gradient-secondary (dark #262626 -> #0F0F0F; light #FAFAFA -> #FFFFFF).
    card = "theme-gradient-secondary-0-background",
    card_b = "theme-gradient-secondary-1-background",
    conv = "theme-gradient-secondary-0-background",
    conv_b = "theme-gradient-secondary-1-background",
    /// Composer: the conversation card's gradient shows through; its bottom colour.
    composer = "theme-gradient-secondary-1-background",
    /// Selected list row (dark #3B3B3B = listItem-primary checked; light #DEDEDE).
    selected = "listItem-primary:checked",
    hover = "listItem-primary:hovered",
    pill = "listitem-background:hovered",
    /// Selected rail circle: white 20 % on the window gradient (#474747 dark, #C2C2C2 light).
    rail_sel = "navigationtab-background:checked",
    /// Header search pill (#3B3B3B dark, #DEDEDE light).
    search = "globalHeader-SearchEditField-background",
    avatar = "avatar-color-avatar-background-default",
    avatar_text = "avatar-color-avatar-text-default",
    text1 = "text-primary",
    text2 = "text-secondary",
    /// Accent (unread dot, composer focus line #07C1F5 dark): mainList-indicator, the themes' accent colour.
    accent = "mainList-indicator",
    link = "button-hyperlink-text",
    focus = "global-focus-outline",
    sep = "navigation-bar-divider-1",
    unread = "mainList-indicator",
    badge = "appNav-badge",
    mention = "wx-mentionsHighlight-text",
    green = "wx-statusReadyIcon",
    blue = "legacy-blue-color-set:hovered",
    /// Favourite star (light #7D4705).
    star = "interactiveicon-tertiary-filled-favorite-icon-active",
    outline = "wx-default-border",
    banner_a = "theme-gradient-dialog-secondary-0-background",
    banner_b = "theme-gradient-dialog-secondary-0-background",
    /// Selected text tab ("Alle": light #C9C9C9 = black 20 % on #FBFBFB).
    tab_on = "tab-active-background",
    toast_bg = "popover-primary-background",
    toast_text = "label-primary-text",
    /// Primary (filled) buttons: white 95 % in dark themes, black 95 % in light ones (the dark "+" of the list card).
    white_btn = "button-primary-background",
    white_btn_text = "button-primary-text",
    tab_off = "tab-inactive-text",
}

impl Th {
    /// Any token of the active theme ("token" or "token:state").
    pub(crate) fn tk(&self, name: &str) -> Color32 { over(name).unwrap_or_else(|| tok(self.id, name)) }
    /// Black in light themes, white in dark ones: the base of the many "white / black at n %" tokens.
    pub(crate) fn ink(&self) -> Color32 { if self.light { Color32::BLACK } else { Color32::WHITE } }
    /// `ink()` at opacity `a` (the `white-100-alpha-N` / `black-100-alpha-N` token pairs).
    pub(crate) fn ink_a(&self, a: f32) -> Color32 { crate::alpha(self.ink(), a) }
}

/// The team colours: (name, avatar background token, team-name text token). Team text = wx-leftpane-shortcut-app-icon-<c>
/// (e.g. orange #FF9D52 dark, #914017 light); avatar = avatar-color-avatar-background-<c> (the same in both kinds of theme).
pub(crate) const TEAM_COLOURS: [(&str, &str, &str); 8] = [
    ("orange", "avatar-color-avatar-background-orange", "wx-leftpane-shortcut-app-icon-orange"),
    ("cobalt", "avatar-color-avatar-background-cobalt", "wx-leftpane-shortcut-app-icon-cobalt"),
    ("mint", "avatar-color-avatar-background-mint", "wx-leftpane-shortcut-app-icon-mint"),
    ("violet", "avatar-color-avatar-background-violet", "wx-leftpane-shortcut-app-icon-violet"),
    ("pink", "avatar-color-avatar-background-pink", "wx-leftpane-shortcut-app-icon-pink"),
    ("lime", "avatar-color-avatar-background-lime", "wx-leftpane-shortcut-app-icon-lime"),
    ("gold", "avatar-color-avatar-background-gold", "wx-leftpane-shortcut-app-icon-gold"),
    ("cyan", "avatar-color-avatar-background-cyan", "wx-leftpane-shortcut-app-icon-cyan"),
];
