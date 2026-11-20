#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod cloud;
mod shortcuts;
mod assets; // icons, images, translations, Webex client assets
mod extras; // Giphy search + favourites, plugins
mod media; // call audio + sounds
mod platform; // OS integration: system, tray, toasts, webview, native messaging host, Linux
mod ui;

// Used all over the crate as `crate::system` / `system::`; re-exported at the root so those paths stay valid.
pub(crate) use platform::system;
// Likewise `icons::` / `i18n::` (the `tr!` macros expand to `$crate::i18n::...`).
pub(crate) use assets::{i18n, icons};
// And `jose::` / `richtext::`, used by both the cloud layer and the views.
pub(crate) use cloud::{jose, richtext};

use cloud::{Meeting, Message, Person, Req, Resp, Space, Team};
use eframe::egui::{self, pos2, Color32, FontId, Key, Modifiers, Rect, RichText, Sense, Stroke, Vec2};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use ui::widgets::*;
use app::actions::Compose;

// Colours: the Momentum theme tokens (ui/theme, resolved through their parent chains). `Th` is the active theme's
// palette; `t.tk("token")` reads any other token.
pub(crate) use ui::theme::{Th, ThemeId};
const fn hex(v: u32) -> Color32 { Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8) }
/// `c` at opacity `a`, composited like Qt does (premultiplied in gamma space). egui's
/// `from_rgba_unmultiplied` premultiplies in linear light, which makes "white 7 %" about 34 % bright.
fn alpha(c: Color32, a: f32) -> Color32 {
    let p = |v: u8| (v as f32 * a).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_premultiplied(p(c.r()), p(c.g()), p(c.b()), (a * 255.0).round().clamp(0.0, 255.0) as u8)
}
/// MomentumDark ("Standard Dunkel").
#[allow(dead_code)]
fn dark() -> Th { Th::of(ThemeId::Dark) }
/// MomentumDefault ("Hell").
#[allow(dead_code)]
fn light() -> Th { Th::of(ThemeId::Light) }
fn rgba(v: u32) -> Option<Color32> { (v != 0).then(|| alpha(Color32::from_rgb((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8), (v as u8) as f32 / 255.0)) }

fn presence_text(s: &str) -> &'static str {
    match s {
        "active" => "Verfügbar", "inactive" => "Abwesend", "DoNotDisturb" => "Bitte nicht stören", "meeting" => "In einem Meeting",
        "call" => "In einem Anruf", "presenting" => "Präsentation", "OutOfOffice" => "Abwesend", _ => "",
    }
}

// Team colours: (avatar background, subtitle text) per team. The text is the theme's
// wx-leftpane-shortcut-app-icon-<colour> token and the avatar the avatar-color-avatar-background-<colour> token
// (ui/theme TEAM_COLOURS). Teams without a service colour get one from a stable FNV-1a hash of the team id.
fn team_slot(id: &str) -> usize {
    let h = id.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    (h % ui::theme::TEAM_COLOURS.len() as u64) as usize
}
fn team_colors(t: &Th, id: &str) -> (Color32, Color32) {
    let (_, bg, text) = ui::theme::TEAM_COLOURS[team_slot(id)];
    (t.tk(bg), t.tk(text))
}
/// The subtitle colour of a team name in the list.
fn team_color(t: &Th, id: &str) -> Color32 { team_colors(t, id).1 }
/// (avatar background, subtitle text) of a team. The service delivers `teamColor` (`Team.color`), the dark-theme text colour: in
/// dark themes the name is drawn in it and the avatar in a darker tone of the same hue (x0.65 brightness). Light themes draw the
/// light token of the nearest team colour (the light variants are tokens, not a formula); a colour far from all of them is
/// darkened like the avatar.
fn team_tones(t: &Th, color: Option<(u8, u8, u8)>, id: &str) -> (Color32, Color32) {
    let Some((r, g, b)) = color else { return team_colors(t, id) };
    let dim = Color32::from_rgb((r as f32 * 0.65) as u8, (g as f32 * 0.65) as u8, (b as f32 * 0.65) as u8);
    if !t.light { return (dim, Color32::from_rgb(r, g, b)); }
    let dark = Th::of(t.id.dark_variant());
    let d = |c: Color32| (c.r() as i32 - r as i32).pow(2) + (c.g() as i32 - g as i32).pow(2) + (c.b() as i32 - b as i32).pow(2);
    let best = ui::theme::TEAM_COLOURS.iter().min_by_key(|(_, _, tx)| d(dark.tk(tx))).filter(|(_, _, tx)| d(dark.tk(tx)) < 60 * 60);
    (dim, best.map_or(dim, |(_, _, tx)| t.tk(tx)))
}

// Navigation in the Webex rail order; shortcuts from the keyboard-shortcut registry.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Tab { Messaging, Meetings, Calling, Whiteboards, Contacts, Teams, Voicemail, Plugins, Settings }
// Rail order (Vidcast is left out).
const RAIL: [(Tab, &str, &str, &str); 6] = [
    // The rail uses filled glyphs in both states.
    (Tab::Messaging, "chat-filled", "chat-filled", "Messaging"), (Tab::Meetings, "calendar-month-filled", "calendar-month-filled", "Meetings"),
    (Tab::Calling, "handset-filled", "handset-filled", "Calling"), (Tab::Whiteboards, "whiteboard-content-filled", "whiteboard-content-filled", "Whiteboards"),
    (Tab::Contacts, "contacts-filled", "contacts-filled", "Contacts"),
    (Tab::Teams, "webex-teams-filled", "webex-teams-filled", "Teams"),
];

#[derive(Clone, Copy, PartialEq)]
enum ListTab { All, Direct, Spaces, Public }
#[derive(Clone, Copy, PartialEq)]
enum ConvTab { Messages, Profile, People, Content, Meetings, Apps }
#[derive(Clone, Copy, PartialEq)]
enum Filter { None, Notifications, Drafts, Unread, Favorites, MentionMe, MentionAll, Flags }
const FILTERS: [(Filter, &str); 8] = [
    (Filter::None, "No filter"), (Filter::Notifications, "Notifications"), (Filter::Drafts, "Drafts"), (Filter::Unread, "Unread"),
    (Filter::Favorites, "Favorites"), (Filter::MentionMe, "@Mentions to me"), (Filter::MentionAll, "@Mentions to all"), (Filter::Flags, "Flags"),
];

#[derive(Clone, Copy)]
enum Act {
    AddPeople, LeaveSpace, ToggleRail, Tab(Tab), Search, Prefs, Shortcuts, Prev, Next, Back, Forward, CreateSpace, Contact, Quit, ZoomIn, ZoomOut, List(ListTab), Filter(Filter), Favorite, Reload, CopyInfo,
    // Composer formatting commands (registry ids "ToggleBold", "ToggleItalic", "ToggleUnderline", "AddLink", "ToggleMarkdown",
    // "HeadingOne".."HeadingThree", "ToggleNumberList", "ToggleBulletList", "ToggleCodeBlock", "ToggleStrikeThrough", "ToggleSubscript",
    // "ToggleSuperscript", "ToggleInlineCode", "PasteMatchStyle", "InsertPMR", "EditLastMessage"): `App::act` pushes the id onto
    // `app.sc.composer_events`; the composer drains that Vec each frame and applies the formatting. The shortcut registry itself implements no formatting.
    Composer(&'static str),
}

struct App {
    cfg: PathBuf, settings: serde_json::Value, client: Option<cloud::Client>, status: String,
    me: Person, spaces: Vec<Space>, teams: Vec<Team>, people: Vec<Person>, meetings: Vec<Meeting>,
    messages: HashMap<String, Vec<Message>>, tab: Tab, list_tab: ListTab, filter: Filter, conv_tab: ConvTab,
    selected: Option<String>, team_sel: Option<String>, search: String, focus_search: bool, drafts: HashMap<String, String>,
    flags: HashSet<(String, String)>, collapsed: HashSet<&'static str>, list_hidden: bool, history: Vec<(Tab, Option<String>)>, hpos: usize,
    plugins: extras::plugins::Plugins, icons: icons::Icons,
    /// Plugin repositories: indexes, downloads, the repository list (extras/plugin_repo/).
    repos: extras::plugin_repo::Repos, toasts: Vec<(String, f64)>, plugin_log: Vec<String>, code_input: String,
    new_space: Option<String>, dial: String, show_shortcuts: bool, pending_space: Option<String>,
    shot: Option<PathBuf>, frames: u32, applied: Option<Th>, minimize: bool,
    need_login: bool, login_error: String, email_input: String, pkce_verifier: String,
    pending: Option<std::sync::mpsc::Receiver<Result<cloud::auth::Token, String>>>, refreshed: bool,
    login_page: LoginPage, config_open: bool, meeting_input: String, login_sized: bool,
    /// Embedded sign-in webview (Windows: WebView2 child of our HWND; Linux: WebKitGTK own window),
    /// and the app window's native handle used to parent it. `None` when not signing in in-window.
    webview: Option<platform::webview::WebView>, hwnd: Option<isize>,
    /// `--signin [email]`: start the native sign-in automatically once the window exists. Taken on
    /// the first frame where the HWND is available (the webview needs it as its parent).
    autosignin: Option<String>,
    cover: Option<egui::TextureHandle>, cover_bytes: Option<Vec<u8>>,
    /// Profile pictures + last-active times from the cloud thread.
    photos: assets::photos::Photos,
    /// Reply / edit mode of the message box, and the space-settings dialogs.
    compose: Compose, rename_input: Option<String>, add_people: bool, people_filter: String, open_threads: HashSet<String>, confirm_leave: Option<String>, forward_text: Option<String>, rail_wide: bool,
    fonts_set: bool, row_salt: String, row_draft: bool,
    /// Keyboard shortcuts: user overrides, resolved bindings, popup / settings-page state (src/shortcuts/).
    sc: shortcuts::State,
    /// The current call card / incoming-call toast and the session's call history (`ui/call/`).
    call: ui::call::CallUi,
    // OS toast windows, system tray, taskbar flash / badge (platform/notify).
    notify: platform::notify::Hub,
    // Message actions (flags, quote, mark unread) and scheduled sending (ui/msg_actions.rs, ui/schedule_send.rs).
    msgx: ui::msg_actions::MsgX,
}

/// The typeface on Windows is Segoe UI (regular + semibold), loaded from the system's fonts and never bundled.
/// Falls back to egui's built-in font when it isn't installed (e.g. Linux).
/// Windows 11 rounds the corners (and draws the shadow) of framed windows itself; a borderless window
/// (we draw our own title bar) gets square corners unless DWM is asked to round it, so this sets the documented
/// corner preference (DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND). No effect on Windows 10, which never rounds windows.
#[cfg(windows)]
fn round_corners(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{Foundation::HWND, Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE}};
    if let Ok(h) = cc.window_handle() {
        if let RawWindowHandle::Win32(w) = h.as_raw() {
            let pref: i32 = 2; // DWMWCP_ROUND
            unsafe { let _ = DwmSetWindowAttribute(HWND(w.hwnd.get() as *mut _), DWMWINDOWATTRIBUTE(33), &pref as *const i32 as *const _, 4); }
        }
    }
}
#[cfg(not(windows))]
fn round_corners(_cc: &eframe::CreationContext<'_>) {}

/// Points scrolled per mouse-wheel notch (egui's default is 40).
const WHEEL_PT_PER_NOTCH: f32 = 60.0;

fn set_fonts(ctx: &egui::Context) {
    let dir = std::env::var("WINDIR").map(|w| PathBuf::from(w).join("Fonts")).unwrap_or_default();
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(bytes) = std::fs::read(dir.join("segoeui.ttf")) {
        fonts.font_data.insert("segoe".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, "segoe".into());
    }
    // Semibold: the weight for selected labels, titles and tabs.
    let mut sb: Vec<String> = Vec::new();
    if let Ok(bytes) = std::fs::read(dir.join("seguisb.ttf")) {
        fonts.font_data.insert("segoesb".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        sb.push("segoesb".into());
    }
    sb.extend(fonts.families[&egui::FontFamily::Proportional].clone());
    fonts.families.insert(egui::FontFamily::Name("semibold".into()), sb);
    // Emoji the bundled picture set doesn't have (newer ones) are drawn from the system emoji font, in a family of its own that
    // contains nothing else - so a missing glyph is a box, never another emoji (ui/emoji_text.rs).
    if let Ok(bytes) = std::fs::read(dir.join("seguiemj.ttf")) {
        fonts.font_data.insert("seguiemj".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        fonts.families.insert(egui::FontFamily::Name(ui::emoji_text::FAMILY.into()), vec!["seguiemj".into()]);
    }
    #[cfg(windows)]
    windows_fallback_fonts(&mut fonts, &dir);
    #[cfg(not(windows))]
    linux_fonts(&mut fonts);
    ctx.set_fonts(fonts);
}

/// Windows: Qt falls back to the system's fonts for characters Segoe UI lacks; egui doesn't, so they would be boxes
/// (e.g. the table flip "(╯°□°)╯︵ ┻━┻"). Appended after the main fonts so ordinary text is unchanged: Segoe UI Symbol (box
/// drawing, arrows, ✓ ♥), Meiryo (kana, CJK compatibility forms such as ︵, ʘ), Nirmala UI (Indic scripts, e.g. ಠ).
/// A missing file is simply skipped.
#[cfg(windows)]
fn windows_fallback_fonts(fonts: &mut egui::FontDefinitions, dir: &std::path::Path) {
    let semibold = egui::FontFamily::Name("semibold".into());
    for (key, file) in [("seguisym", "seguisym.ttf"), ("meiryo", "meiryo.ttc"), ("nirmala", "Nirmala.ttc")] {
        let Ok(bytes) = std::fs::read(dir.join(file)) else { continue };
        fonts.font_data.insert(key.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        for fam in [egui::FontFamily::Proportional, semibold.clone()] {
            if let Some(list) = fonts.families.get_mut(&fam) { list.push(key.into()); }
        }
    }
}

/// Linux / other: Segoe UI isn't there (and can't be shipped), so the bundled Selawik - Microsoft's open-source (OFL) metric-compatible
/// stand-in for Segoe UI - takes its place, regular + semibold; egui's own fonts stay behind it for the glyphs Selawik lacks.
/// Emoji outside the bundled picture set come from egui's built-in monochrome Noto Emoji (colour bitmap fonts can't be drawn).
#[cfg(not(windows))]
fn linux_fonts(fonts: &mut egui::FontDefinitions) {
    use std::sync::Arc;
    let prop = egui::FontFamily::Proportional;
    if !fonts.families[&prop].iter().any(|f| f == "segoe") {
        fonts.font_data.insert("selawik".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/selawk.ttf"))));
        fonts.families.get_mut(&prop).unwrap().insert(0, "selawik".into());
    }
    let sb = egui::FontFamily::Name("semibold".into());
    if !fonts.families[&sb].iter().any(|f| f == "segoesb") {
        fonts.font_data.insert("selawiksb".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/selawksb.ttf"))));
        let list = fonts.families.get_mut(&sb).unwrap();
        list.retain(|f| f != "selawik");
        list.insert(0, "selawiksb".into());
        list.insert(1, "selawik".into());
    }
    let emoji = egui::FontFamily::Name(ui::emoji_text::FAMILY.into());
    if !fonts.families.contains_key(&emoji) && fonts.font_data.contains_key("NotoEmoji-Regular") {
        fonts.families.insert(emoji, vec!["NotoEmoji-Regular".into()]);
    }
    // Last resort for what neither Selawik nor egui's fonts have (typographic spaces such as the quote indent U+2002, more scripts):
    // the system's DejaVu / Noto / Liberation Sans. `WEBOX_FALLBACK_FONT` (set by the Nix wrapper) names the file directly.
    if let Some(bytes) = system_fallback_font() {
        fonts.font_data.insert("sysfallback".into(), Arc::new(egui::FontData::from_owned(bytes)));
        for fam in [prop, sb] { fonts.families.get_mut(&fam).unwrap().push("sysfallback".into()); }
    }
}

#[cfg(not(windows))]
fn system_fallback_font() -> Option<Vec<u8>> {
    if let Some(b) = std::env::var_os("WEBOX_FALLBACK_FONT").and_then(|p| std::fs::read(p).ok()) { return Some(b); }
    const NAMES: [&str; 4] = ["DejaVuSans.ttf", "NotoSans-Regular.ttf", "LiberationSans-Regular.ttf", "FreeSans.ttf"];
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::var_os("XDG_DATA_DIRS").map(|v| std::env::split_paths(&v).map(|p| p.join("fonts")).collect()).unwrap_or_default();
    dirs.extend([PathBuf::from("/usr/share/fonts"), "/usr/local/share/fonts".into(), home.join(".local/share/fonts"), home.join(".nix-profile/share/fonts"),
        "/run/current-system/sw/share/X11/fonts".into()]);
    fn walk(d: &std::path::Path, name: &str, depth: u32) -> Option<PathBuf> {
        let mut sub = vec![];
        for e in std::fs::read_dir(d).ok()?.flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == name) { return Some(p); }
            if depth > 0 && p.is_dir() { sub.push(p); }
        }
        sub.iter().find_map(|s| walk(s, name, depth - 1))
    }
    NAMES.iter().find_map(|n| dirs.iter().find_map(|d| walk(d, n, 4))).and_then(|p| std::fs::read(p).ok())
}

#[derive(Clone, Copy, PartialEq)]
enum LoginPage { Start, Email, Code, Join, Web }
const SIGN_UP_URL: &str = "https://signup.webex.com/";
// "Terms of Service" link of the sign-in footer.
const TERMS_URL: &str = "https://www.cisco.com/c/dam/en_us/about/doing_business/legal/eula/cisco_end_user_license_agreement-eng.pdf";
const PRIVACY_URL: &str = "https://www.cisco.com/c/en/us/about/legal/privacy-full.html"; // "Privacy Statement" link of the sign-in footer
const LEARN_MORE_URL: &str = "https://help.webex.com";
const LOGIN_SIZE: [f32; 2] = [788.0, 563.0]; // sign-in window size in points (985x744 px at 125 % incl. the 40 px title)
const MAIN_SIZE: [f32; 2] = [1280.0, 800.0];
/// Rail width with labels, in points.
const RAIL_WIDE: f32 = 196.0;

fn main() -> eframe::Result<()> {
    let raw: Vec<String> = std::env::args().collect();
    if platform::nmhost::is_host_call(&raw) { platform::nmhost::run(&system::config_dir()); return Ok(()); }
    // The updater runs a downloaded program with --version before it replaces this one.
    if raw.iter().skip(1).any(|a| a == "--version") { println!("webox {}", env!("CARGO_PKG_VERSION")); return Ok(()); }
    let _ = rustls::crypto::ring::default_provider().install_default();
    icons::prewarm(); // rasterise the first screens' icons on a worker while the window is created
    let args = system::args();
    #[cfg(windows)]
    if args.screenshot.is_none() { system::hold_app_mutex(); } // the installer waits for Webox to close
    // Linux: a second start hands its link to the running copy (D-Bus) and quits; test runs (--sample / --screenshot) never do.
    #[cfg(target_os = "linux")]
    if args.screenshot.is_none() && !args.sample && std::env::var_os("WEBOX_MULTI_INSTANCE").is_none() && platform::linux::instance::hand_off(args.protocol_uri.as_deref()) {
        return Ok(());
    }
    let mut vp = egui::ViewportBuilder::default().with_title("Webox").with_inner_size([1280.0, 800.0]).with_min_inner_size([760.0, 540.0])
        .with_decorations(false) // Webox draws its own window controls in its header
        .with_app_id("webox"); // X11 WM_CLASS / Wayland app_id = webox.desktop (icon + grouping in the Linux panels; unused on Windows)
    if let Ok(img) = image::load_from_memory(icons::APP_ICON_256) { // app icon (window + taskbar button)
        let img = img.to_rgba8();
        vp = vp.with_icon(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() });
    }
    if args.screenshot.is_some() {
        vp = vp.with_active(false).with_inner_size([1200.0, 720.0]); // a fixed size so screenshots compare 1:1
        #[cfg(windows)] { vp = vp.with_window_level(egui::WindowLevel::AlwaysOnBottom); }
    }
    // X11: a transparent root viewport makes eframe pick an ARGB GL config, which every window shares, so the toast / tray-menu /
    // incoming-call popups keep `with_transparent(true)` (x11::argb_visual). The main window stays opaque (opaque clear colour).
    // Windows needs no such config (tray::see_through); Wayland's default config has alpha already.
    #[cfg(target_os = "linux")]
    if platform::linux::x11::argb_visual() { vp = vp.with_transparent(true); }
    let opts = eframe::NativeOptions { viewport: vp, ..Default::default() };
    eframe::run_native("Webox", opts, Box::new(move |cc| {
        set_fonts(&cc.egui_ctx);
        // Mouse wheel: 60 pt per notch instead of egui's 40.
        cc.egui_ctx.options_mut(|o| o.line_scroll_speed = WHEEL_PT_PER_NOTCH);
        round_corners(cc);
        Ok(Box::new(App::new(args)))
    }))
}
