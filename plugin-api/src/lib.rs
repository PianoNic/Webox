//! Native Rust plugin interface for Webox (BetterDiscord/Vencord-style plugins,
//! compiled instead of scripted). A plugin is a `cdylib` that implements [`Plugin`]
//! and calls [`export_plugin!`]. The host loads `.dll`/`.so` files from its plugin
//! folder or installs them from a plugin repository (docs/plugins.md).
//!
//! ABI rules (what keeps host and plugin compatible across compiler versions):
//! - Everything crossing the boundary is `#[repr(C)]`: integers, raw pointers, NUL-terminated UTF-8 strings and
//!   `extern "C"` function pointers. No Rust types (`String`, `Vec`, `Box`, trait objects, `bool`, enums) cross it.
//! - Function pointers are `Option<extern "C" fn>` (same layout as a plain function pointer, null = `None`), so the
//!   host can refuse a table with a missing entry instead of calling a null pointer.
//! - Memory is freed by the side that allocated it: strings a plugin returns go back through the plugin's `free_str`,
//!   strings the host returns through the host's `free_str`.
//! - A panic never unwinds across the boundary: every shim catches it (the hook then reports "no change").
//! - The [`CONTRACT`] version is exported as its own symbol ([`CONTRACT_SYMBOL`]). The host refuses a plugin built for
//!   another major version or a newer minor version than its own.
//! - A plugin must not keep threads running after [`Plugin::stop`] returns: the host may unload the library afterwards.
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub mod color;
pub mod settings;
pub use settings::{Schema, Values};

/// Layout version of [`PluginVTable`] / [`HostApi`], stored in [`PluginVTable::abi`]. Equals [`CONTRACT_MAJOR`].
pub const ABI_VERSION: u32 = CONTRACT_MAJOR as u32;
/// Contract major version: changes when the tables change incompatibly (fields removed, reordered or retyped).
pub const CONTRACT_MAJOR: u16 = 1;
/// Contract minor version: changes when something is added that older hosts do not offer.
/// 1.0: the first interface (no contract symbol). 1.1: contract symbol, nullable function pointers, panic-safe shims.
/// 1.2: settings (settings_schema / settings_changed / settings_action at the end of [`PluginVTable`], settings_get at
/// the end of [`HostApi`]). The host reads the new vtable entries only from plugins that report 1.2 or newer.
/// 1.3: accent themes ([`Plugin::accents`], `accents` at the end of [`PluginVTable`]): accent-role colours and a gradient,
/// computed by the plugin for the active base theme ([`ThemeInfo`]).
/// 1.4: decorations ([`Plugin::decor`], `decor` at the end of [`PluginVTable`], `text_secondary` at the end of [`ThemeInfo`]):
/// background washes, host-drawn decorations and effects.
pub const CONTRACT_MINOR: u16 = 4;
/// "major.minor", as written into a plugin repository index (`contract`).
pub const CONTRACT: &str = "1.4";
/// `(major << 16) | minor`, returned by the [`CONTRACT_SYMBOL`] function.
pub const CONTRACT_PACKED: u32 = ((CONTRACT_MAJOR as u32) << 16) | CONTRACT_MINOR as u32;
pub const ENTRY_SYMBOL: &[u8] = b"collabhost_plugin\0";
/// `extern "C" fn() -> u32` returning [`CONTRACT_PACKED`]. Plugins without it were built for contract 1.0.
pub const CONTRACT_SYMBOL: &[u8] = b"collabhost_plugin_contract\0";

/// Splits a packed contract value into (major, minor).
pub const fn unpack_contract(v: u32) -> (u16, u16) { ((v >> 16) as u16, v as u16) }

/// Colours as 0xRRGGBBAA. A field of 0 keeps the host's colour.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Palette {
    pub bg_primary: u32,
    pub bg_secondary: u32,
    pub bg_tertiary: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub accent: u32,
    pub hyperlink: u32,
}

/// The base theme an accent theme is computed for (contract 1.3). Colours as 0xRRGGBBAA.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct ThemeInfo {
    /// `size_of::<ThemeInfo>()` of the host (newer hosts may append fields).
    pub size: u32,
    /// 1 for a light theme, 0 for a dark one.
    pub light: u32,
    /// The content cards' background (what text and accents are read on).
    pub background: u32,
    /// The window background around the cards.
    pub surface: u32,
    /// The primary text colour.
    pub text: u32,
    /// 1.4: the secondary text colour (labels, timestamps). Read it only when `size` covers it.
    pub text_secondary: u32,
}

/// A gradient the host paints over one surface (contract 1.4): `stops_len` colours spread evenly along `angle` (degrees,
/// 0 = left to right, 90 = top to bottom), laid over the base colour at `opacity` (0..1). `border` > 0 also strokes the
/// surface's outline with the same gradient at full strength, that many points wide. `stops_len` 0 = nothing.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Wash { pub stops_len: u32, pub stops: [u32; GRADIENT_MAX], pub angle: f32, pub opacity: f32, pub border: f32 }

/// Host-drawn decorations and effects a design plugin can switch on ([`Decor::flags`]). Webox draws them in the
/// [`Decor::stripes`] colours (falling back to the accent gradient).
pub mod deco {
    /// A vertical flag band along the rail's left edge.
    pub const RAIL_STRIPE: u64 = 1 << 0;
    /// A thin gradient line under the header.
    pub const HEADER_STRIPE: u64 = 1 << 1;
    /// A ring in the stripe colours around your own avatar.
    pub const AVATAR_RING_SELF: u64 = 1 << 2;
    /// ... and around everybody else's.
    pub const AVATAR_RING_ALL: u64 = 1 << 3;
    /// A gradient outline on the selected list row.
    pub const SELECTED_ROW: u64 = 1 << 4;
    /// A gradient outline on the selected text tab pill ("Alle", "Nachrichten", ...).
    pub const TAB_PILLS: u64 = 1 << 5;
    /// A gradient underline under the selected settings navigation item.
    pub const NAV_UNDERLINE: u64 = 1 << 6;
    /// Scrollbar thumbs in a vertical gradient.
    pub const SCROLLBAR: u64 = 1 << 7;
    /// Progress bars in a horizontal gradient.
    pub const PROGRESS: u64 = 1 << 8;
    /// A gradient ring around presence dots.
    pub const PRESENCE_RING: u64 = 1 << 9;
    /// The send button with a gradient outline and a soft gradient fill.
    pub const SEND_BUTTON: u64 = 1 << 10;
    /// A small flag (drawn as stripes) next to your name in the profile menu.
    pub const NAME_FLAG: u64 = 1 << 11;
    /// Switched-on toggle tracks in a horizontal gradient.
    pub const TOGGLE_TRACK: u64 = 1 << 12;
    /// Unread dots cycle through the stripe colours from row to row.
    pub const BADGE_CYCLE: u64 = 1 << 13;
    /// Effect: a short burst of confetti and hearts from the send button when you send (at most 1 s).
    pub const SEND_BURST: u64 = 1 << 32;
    /// Effect: a gradient ripple on a reaction you add.
    pub const REACTION_RIPPLE: u64 = 1 << 33;
    /// Effect: a "Happy Pride!" sparkle once, on the first start in June.
    pub const JUNE_SPARKLE: u64 = 1 << 34;
}

/// What a design plugin asks Webox to draw (contract 1.4). Effects are skipped while the OS asks for reduced motion.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Decor {
    /// `size_of::<Decor>()` of the plugin (set by the shim).
    pub size: u32,
    /// Behind everything: the window, rail and header.
    pub window: Wash,
    /// The list card (spaces, settings navigation).
    pub list_card: Wash,
    /// The content card (conversation, settings page). Keep `opacity` low: message text sits on it.
    pub content_card: Wash,
    /// The colours of the decorations, in order (e.g. a flag's stripes).
    pub stripes_len: u32,
    pub stripes: [u32; GRADIENT_MAX],
    /// [`deco`] bits.
    pub flags: u64,
}

/// Maximum stops of [`Accents::gradient`].
pub const GRADIENT_MAX: usize = 12;

/// Accent colours of a design plugin (contract 1.3), 0xRRGGBBAA; a field of 0 keeps the host's colour. Backgrounds and body
/// text stay the base theme's, so an accent theme works on every theme, dark and light. Pick readable values for the
/// [`ThemeInfo`] you are given ([`color`] has WCAG contrast helpers).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Accents {
    /// `size_of::<Accents>()` of the plugin (set by the shim).
    pub size: u32,
    /// Toggles and the composer's focus line (when there is no gradient), and other small accents.
    pub accent: u32,
    pub link: u32,
    /// Keyboard focus ring and the outline of a focused text field.
    pub focus: u32,
    /// Text selection (drawn translucent).
    pub selection: u32,
    /// Unread dot in the lists.
    pub unread: u32,
    /// Badges on the rail, and their text.
    pub badge: u32,
    pub badge_text: u32,
    /// The circle behind the selected rail item (the rail's icon is drawn on it in the primary text colour).
    pub rail_selected: u32,
    /// Primary (filled) buttons and their text.
    pub button: u32,
    pub button_text: u32,
    /// Checked checkboxes / radio buttons, the filled part of sliders; `control_icon` the tick / dot on them.
    pub control: u32,
    pub control_icon: u32,
    /// Mentions of you in messages.
    pub mention: u32,
    /// Scrollbar thumb under the pointer.
    pub scrollbar_hover: u32,
    /// The composer's focus line as a horizontal gradient: the first `gradient_len` stops (0 = use `accent`).
    pub gradient_len: u32,
    pub gradient: [u32; GRADIENT_MAX],
}

pub type StrFn = Option<extern "C" fn(*mut c_void, *const c_char)>;

/// What the host offers a plugin. Strings returned by `data_get` are freed with `free_str`.
#[repr(C)]
pub struct HostApi {
    pub ctx: *mut c_void,
    pub toast: StrFn,
    pub send: StrFn,
    pub log: StrFn,
    pub data_get: Option<extern "C" fn(*mut c_void, *const c_char) -> *mut c_char>,
    pub data_set: Option<extern "C" fn(*mut c_void, *const c_char, *const c_char)>,
    pub free_str: Option<extern "C" fn(*mut c_char)>,
    /// 1.2: the plugin's current setting values as a flat JSON object (freed with `free_str`).
    pub settings_get: Option<extern "C" fn(*mut c_void) -> *mut c_char>,
}

/// What a plugin offers the host. Returned strings are freed with the plugin's `free_str`;
/// a null return means "no change".
#[repr(C)]
pub struct PluginVTable {
    pub abi: u32,
    pub instance: *mut c_void,
    pub name: *const c_char,
    pub author: *const c_char,
    pub version: *const c_char,
    pub description: *const c_char,
    /// Space-separated slash commands this plugin answers, without the slash.
    pub commands: *const c_char,
    pub start: Option<extern "C" fn(*mut c_void, *const HostApi)>,
    pub stop: Option<extern "C" fn(*mut c_void)>,
    /// Outgoing text: null = unchanged, empty = block sending, else the replacement.
    pub on_send: Option<extern "C" fn(*mut c_void, *const c_char) -> *mut c_char>,
    /// Incoming message (author, text): null = show as is, else the text to show.
    pub on_message: Option<extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *mut c_char>,
    /// Slash command (name, args): the text to send, or null to send nothing.
    pub command: Option<extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *mut c_char>,
    /// Returns 1 and fills the palette when the plugin is a theme.
    pub palette: Option<extern "C" fn(*mut c_void, *mut Palette) -> i32>,
    pub free_str: Option<extern "C" fn(*mut c_char)>,
    pub destroy: Option<extern "C" fn(*mut c_void)>,
    /// 1.2: the settings schema as JSON (freed with `free_str`), null = no settings.
    pub settings_schema: Option<extern "C" fn(*mut c_void) -> *mut c_char>,
    /// 1.2: the setting values changed (flat JSON object of all values). Also called once before every start.
    pub settings_changed: Option<extern "C" fn(*mut c_void, *const c_char)>,
    /// 1.2: a settings button was pressed (its id): a message to show, or null.
    pub settings_action: Option<extern "C" fn(*mut c_void, *const c_char) -> *mut c_char>,
    /// 1.3: fills the accents for the given base theme and returns 1 when the plugin is an accent theme.
    pub accents: Option<extern "C" fn(*mut c_void, *const ThemeInfo, *mut Accents) -> i32>,
    /// 1.4: fills the decorations (backgrounds, host-drawn decorations, effects) for the given base theme; 1 = it has some.
    pub decor: Option<extern "C" fn(*mut c_void, *const ThemeInfo, *mut Decor) -> i32>,
}

/// Safe handle to the host, given to [`Plugin::start`]. Valid until [`Plugin::stop`] returns.
#[derive(Clone, Copy)]
pub struct Host(*const HostApi);
unsafe impl Send for Host {}

impl Host {
    fn api(&self) -> Option<&HostApi> { unsafe { self.0.as_ref() } }
    fn call(&self, f: impl Fn(&HostApi) -> StrFn, text: &str) {
        if let Some(a) = self.api() { if let Some(f) = f(a) { let c = cs(text); f(a.ctx, c.as_ptr()) } }
    }
    pub fn toast(&self, text: &str) { self.call(|a| a.toast, text) }
    pub fn send(&self, text: &str) { self.call(|a| a.send, text) }
    pub fn log(&self, text: &str) { self.call(|a| a.log, text) }
    pub fn get(&self, key: &str) -> Option<String> {
        let a = self.api()?;
        let (get, free) = (a.data_get?, a.free_str?);
        let k = cs(key);
        let p = get(a.ctx, k.as_ptr());
        if p.is_null() { return None; }
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        free(p);
        Some(s)
    }
    /// The current setting values (the same the plugin got in [`Plugin::settings_changed`]).
    pub fn settings(&self) -> Values {
        let Some(a) = self.api() else { return Values::default() };
        let (Some(get), Some(free)) = (a.settings_get, a.free_str) else { return Values::default() };
        let p = get(a.ctx);
        if p.is_null() { return Values::default(); }
        let v = Values::parse(&unsafe { CStr::from_ptr(p) }.to_string_lossy());
        free(p);
        v
    }
    pub fn set(&self, key: &str, value: &str) {
        if let Some(a) = self.api() { if let Some(f) = a.data_set { let (k, v) = (cs(key), cs(value)); f(a.ctx, k.as_ptr(), v.as_ptr()) } }
    }
}

/// Implement this in your plugin crate. Every hook has a default that does nothing.
pub trait Plugin: Default + 'static {
    const NAME: &'static str;
    const AUTHOR: &'static str;
    const VERSION: &'static str;
    const DESCRIPTION: &'static str;
    const COMMANDS: &'static str = "";
    fn start(&mut self, _host: Host) {}
    fn stop(&mut self) {}
    fn on_send(&mut self, _text: &str) -> Option<String> { None }
    fn on_message(&mut self, _author: &str, _text: &str) -> Option<String> { None }
    fn command(&mut self, _name: &str, _args: &str) -> Option<String> { None }
    fn palette(&mut self) -> Option<Palette> { None }
    /// The settings this plugin has (drawn by Webox on the plugin's page). None = no settings.
    fn settings(&self) -> Option<Schema> { None }
    /// The values changed (and once before every start), all of them, defaults filled in.
    fn settings_changed(&mut self, _values: &Values) {}
    /// A settings button was pressed: an optional message for the user.
    fn settings_action(&mut self, _id: &str) -> Option<String> { None }
    /// For accent themes: the accent colours for this base theme (asked again whenever the theme or a setting changes;
    /// keep it cheap). Leave fields 0 to keep the host's colour.
    fn accents(&mut self, _theme: &ThemeInfo) -> Option<Accents> { None }
    /// For design plugins: backgrounds, decorations and effects Webox draws for this base theme (contract 1.4).
    fn decor(&mut self, _theme: &ThemeInfo) -> Option<Decor> { None }
}

/// A C string from any `&str` (interior NULs are dropped).
pub fn cs(s: &str) -> CString { CString::new(s.replace('\0', "")).unwrap_or_default() }
fn out(s: Option<String>) -> *mut c_char { s.map_or(std::ptr::null_mut(), |s| cs(&s).into_raw()) }
/// A borrowed string from a C pointer; null reads as "".
unsafe fn arg<'a>(p: *const c_char) -> std::borrow::Cow<'a, str> { if p.is_null() { "".into() } else { CStr::from_ptr(p).to_string_lossy() } }
/// Runs `f` on the plugin instance; a null instance or a panic yields `fallback`.
fn with<T, R>(p: *mut c_void, fallback: R, f: impl FnOnce(&mut T) -> R) -> R {
    if p.is_null() { return fallback; }
    catch_unwind(AssertUnwindSafe(|| f(unsafe { &mut *(p as *mut T) }))).unwrap_or(fallback)
}

extern "C" fn start_shim<T: Plugin>(p: *mut c_void, h: *const HostApi) { with::<T, _>(p, (), |me| me.start(Host(h))) }
extern "C" fn stop_shim<T: Plugin>(p: *mut c_void) { with::<T, _>(p, (), |me| me.stop()) }
extern "C" fn send_shim<T: Plugin>(p: *mut c_void, t: *const c_char) -> *mut c_char {
    with::<T, _>(p, std::ptr::null_mut(), |me| out(me.on_send(&unsafe { arg(t) })))
}
extern "C" fn msg_shim<T: Plugin>(p: *mut c_void, a: *const c_char, t: *const c_char) -> *mut c_char {
    with::<T, _>(p, std::ptr::null_mut(), |me| out(me.on_message(&unsafe { arg(a) }, &unsafe { arg(t) })))
}
extern "C" fn cmd_shim<T: Plugin>(p: *mut c_void, n: *const c_char, a: *const c_char) -> *mut c_char {
    with::<T, _>(p, std::ptr::null_mut(), |me| out(me.command(&unsafe { arg(n) }, &unsafe { arg(a) })))
}
extern "C" fn pal_shim<T: Plugin>(p: *mut c_void, o: *mut Palette) -> i32 {
    if o.is_null() { return 0; }
    with::<T, _>(p, 0, |me| match me.palette() { Some(pal) => { unsafe { *o = pal }; 1 } None => 0 })
}
extern "C" fn schema_shim<T: Plugin>(p: *mut c_void) -> *mut c_char {
    with::<T, _>(p, std::ptr::null_mut(), |me| out(me.settings().map(|s| s.to_json())))
}
extern "C" fn changed_shim<T: Plugin>(p: *mut c_void, v: *const c_char) {
    with::<T, _>(p, (), |me| me.settings_changed(&Values::parse(&unsafe { arg(v) })))
}
extern "C" fn action_shim<T: Plugin>(p: *mut c_void, id: *const c_char) -> *mut c_char {
    with::<T, _>(p, std::ptr::null_mut(), |me| out(me.settings_action(&unsafe { arg(id) })))
}
/// The host's ThemeInfo, reading only the fields both sides know (a newer host may pass a longer one, an older a shorter one).
fn theme_info(th: *const ThemeInfo) -> ThemeInfo {
    let mut info = ThemeInfo::default();
    let n = (unsafe { (*th).size } as usize).min(std::mem::size_of::<ThemeInfo>());
    unsafe { std::ptr::copy_nonoverlapping(th as *const u8, &mut info as *mut ThemeInfo as *mut u8, n) };
    info.size = std::mem::size_of::<ThemeInfo>() as u32;
    info
}
extern "C" fn decor_shim<T: Plugin>(p: *mut c_void, th: *const ThemeInfo, o: *mut Decor) -> i32 {
    if th.is_null() || o.is_null() { return 0; }
    let info = theme_info(th);
    with::<T, _>(p, 0, |me| match me.decor(&info) {
        Some(mut d) => {
            d.size = std::mem::size_of::<Decor>() as u32;
            d.stripes_len = d.stripes_len.min(GRADIENT_MAX as u32);
            for w in [&mut d.window, &mut d.list_card, &mut d.content_card] { w.stops_len = w.stops_len.min(GRADIENT_MAX as u32); }
            unsafe { *o = d };
            1
        }
        None => 0,
    })
}
extern "C" fn accents_shim<T: Plugin>(p: *mut c_void, th: *const ThemeInfo, o: *mut Accents) -> i32 {
    if th.is_null() || o.is_null() { return 0; }
    let info = theme_info(th);
    with::<T, _>(p, 0, |me| match me.accents(&info) {
        Some(mut a) => { a.size = std::mem::size_of::<Accents>() as u32; a.gradient_len = a.gradient_len.min(GRADIENT_MAX as u32); unsafe { *o = a }; 1 }
        None => 0,
    })
}
extern "C" fn free_shim(s: *mut c_char) { if !s.is_null() { drop(unsafe { CString::from_raw(s) }) } }
extern "C" fn destroy_shim<T: Plugin>(p: *mut c_void) {
    if !p.is_null() { let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(p as *mut T) }))); }
}

/// Builds the table the host reads. Used by [`export_plugin!`]; the host calls the entry symbol once per load.
pub fn vtable<T: Plugin>() -> *const PluginVTable {
    catch_unwind(|| {
        // The table and its name strings live as long as the library is loaded (a few bytes per load).
        let leak = |s: &str| cs(s).into_raw() as *const c_char;
        Box::into_raw(Box::new(PluginVTable {
            abi: ABI_VERSION,
            instance: Box::into_raw(Box::new(T::default())) as *mut c_void,
            name: leak(T::NAME), author: leak(T::AUTHOR), version: leak(T::VERSION),
            description: leak(T::DESCRIPTION), commands: leak(T::COMMANDS),
            start: Some(start_shim::<T>), stop: Some(stop_shim::<T>), on_send: Some(send_shim::<T>), on_message: Some(msg_shim::<T>),
            command: Some(cmd_shim::<T>), palette: Some(pal_shim::<T>), free_str: Some(free_shim), destroy: Some(destroy_shim::<T>),
            settings_schema: Some(schema_shim::<T>), settings_changed: Some(changed_shim::<T>), settings_action: Some(action_shim::<T>),
            accents: Some(accents_shim::<T>), decor: Some(decor_shim::<T>),
        })) as *const PluginVTable
    }).unwrap_or(std::ptr::null())
}

/// `export_plugin!(MyPlugin);` makes the crate loadable by the host.
#[macro_export]
macro_rules! export_plugin {
    ($t:ty) => {
        #[no_mangle]
        pub extern "C" fn collabhost_plugin() -> *const $crate::PluginVTable { $crate::vtable::<$t>() }
        #[no_mangle]
        pub extern "C" fn collabhost_plugin_contract() -> u32 { $crate::CONTRACT_PACKED }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Boom;
    impl Plugin for Boom {
        const NAME: &'static str = "Boom";
        const AUTHOR: &'static str = "test";
        const VERSION: &'static str = "0.0.0";
        const DESCRIPTION: &'static str = "";
        fn on_send(&mut self, _t: &str) -> Option<String> { panic!("boom") }
        fn command(&mut self, n: &str, a: &str) -> Option<String> { Some(format!("{n}|{a}")) }
    }

    #[test]
    fn contract_string_matches_the_numbers() {
        assert_eq!(CONTRACT, format!("{CONTRACT_MAJOR}.{CONTRACT_MINOR}"));
        assert_eq!(unpack_contract(CONTRACT_PACKED), (CONTRACT_MAJOR, CONTRACT_MINOR));
    }

    #[test]
    fn option_fn_pointers_keep_the_c_layout() {
        assert_eq!(std::mem::size_of::<Option<extern "C" fn(*mut c_void)>>(), std::mem::size_of::<*const c_void>());
    }

    #[test]
    fn panics_and_nulls_do_not_cross_the_boundary() {
        let vt = unsafe { &*vtable::<Boom>() };
        let t = cs("hi");
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        assert!((vt.on_send.unwrap())(vt.instance, t.as_ptr()).is_null());
        std::panic::set_hook(prev);
        let r = (vt.command.unwrap())(vt.instance, std::ptr::null(), t.as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(r) }.to_str().unwrap(), "|hi");
        (vt.free_str.unwrap())(r);
        assert_eq!((vt.palette.unwrap())(vt.instance, std::ptr::null_mut()), 0);
        // No settings: a null schema.
        assert!((vt.settings_schema.unwrap())(vt.instance).is_null());
        (vt.destroy.unwrap())(vt.instance);
    }

    #[derive(Default)]
    struct Cfg { on: bool }
    impl Plugin for Cfg {
        const NAME: &'static str = "Cfg";
        const AUTHOR: &'static str = "test";
        const VERSION: &'static str = "0.0.0";
        const DESCRIPTION: &'static str = "";
        fn settings(&self) -> Option<Schema> { Some(Schema::new().toggle("on", "On", "", false).button("b", "B", "")) }
        fn settings_changed(&mut self, v: &Values) { self.on = v.bool("on").unwrap_or(false) }
        fn settings_action(&mut self, id: &str) -> Option<String> { Some(format!("{id}:{}", self.on)) }
    }

    #[test]
    fn settings_cross_the_boundary_as_json() {
        let vt = unsafe { &*vtable::<Cfg>() };
        let s = (vt.settings_schema.unwrap())(vt.instance);
        assert!(unsafe { CStr::from_ptr(s) }.to_str().unwrap().contains("\"id\":\"on\""));
        (vt.free_str.unwrap())(s);
        let v = cs("{\"on\":true}");
        (vt.settings_changed.unwrap())(vt.instance, v.as_ptr());
        let id = cs("b");
        let r = (vt.settings_action.unwrap())(vt.instance, id.as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(r) }.to_str().unwrap(), "b:true");
        (vt.free_str.unwrap())(r);
        (vt.destroy.unwrap())(vt.instance);
    }
}
