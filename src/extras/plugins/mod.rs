//! Loads native Rust plugins (`.dll` / `.so` built against collabhost-plugin-api): loose files from the plugin folders and the
//! plugins installed from a repository (`plugin_repo::store`, verified against their pinned sha256 before every load), and lists the
//! built-in, in-tree plugins (`crate::extras::builtin_plugins`: first-party code compiled into the app, e.g. GIF-Extras) next to them.
//! A built-in entry has no library: its switch is saved like the others (plugins.json, default off) and the app asks
//! `is_enabled(key)` every frame. Built-in plugins can be switched off but not uninstalled.
use super::plugin_repo::{index::check_contract, store};
use super::plugin_settings as ps;
pub mod settings;
pub mod kept;
pub mod view;
pub mod confirm;
pub mod dialog;
pub mod i18n;
use settings::push_settings;
use collabhost_plugin_api::{cs, unpack_contract, Accents, Decor, HostApi, Palette, PluginVTable, ThemeInfo, ABI_VERSION, CONTRACT_SYMBOL, ENTRY_SYMBOL};
use serde_json::{Map, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// What plugins asked the host to do; the window drains it every frame. Plugins call from their own threads too, so every push
/// wakes the window (`ui::wake`): it has no periodic redraw, and a toast or `space.send` would wait for the next input otherwise.
#[derive(Default)]
pub struct Outbox {
    /// (plugin name, text).
    pub toasts: Vec<(String, String)>,
    pub sends: Vec<String>, pub logs: Vec<String>,
    /// `space.send`: (space id, text).
    pub sends_to: Vec<(String, String)>,
    /// `message.keep` changed what is kept: the loaded lists are drawn again (`Plugins::reapply_kept`).
    pub kept_changed: bool,
    /// `settings.set`: (plugin key, setting id, value), saved like a change on the plugin's page.
    pub settings_set: Vec<(String, String, Value)>,
    /// `clipboard.write`: the text to put on the clipboard.
    pub clipboard: Option<String>,
    /// `ui.dialog`: the dialog to show (replaces the one on screen).
    pub dialog: Option<dialog::Dialog> }

/// One message of a space as `space.history` hands it out (`date` / `time` UTC).
struct Recent { id: String, author: String, email: String, mine: bool, text: String, date: String, time: String, published_ms: i64 }
/// The newest messages per space, copied from the window's loaded lists while a plugin subscribes to `message.received`, so
/// plugin threads can read them (`space.history`) without touching the UI's data.
type Recents = Arc<Mutex<HashMap<String, Vec<Recent>>>>;
/// How many messages per space `space.history` can return.
const RECENT_MAX: usize = 200;
/// How many message ids `message_received` remembers to fire once per id.
const SEEN_MAX: usize = 4096;

/// `plugin` is the key (plugin-data.json, log lines), `name` the shown name (toasts).
struct Ctx { plugin: String, name: String, out: Arc<Mutex<Outbox>>, data: Arc<Mutex<Map<String, Value>>>, data_file: PathBuf, settings: Arc<Mutex<Map<String, Value>>>,
    recent: Recents, kept: Arc<Mutex<kept::Kept>>, view: Arc<Mutex<view::View>>, confirms: confirm::Confirms,
    /// The window's thread (`ui.confirm` refuses to block it); None in tests.
    ui_thread: Option<std::thread::ThreadId>,
    /// `reactions.show_avatars`: what this plugin last asked for (None = off).
    avatars: Mutex<Option<ReactionAvatars>>,
    /// The plugin's translations (`i18n.strings`), applied to its toasts and dialogs.
    strings: i18n::Strings }

/// `reactions.show_avatars`: small round avatars of the people who reacted, drawn after each reaction chip's count
/// (ui/messages/who_reacted.rs). Off until an enabled plugin turns it on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReactionAvatars {
    /// At most this many avatars per chip (1..=10), then "+K".
    pub max: usize,
    /// Avatar diameter in pt: `small` 14, `medium` 18.
    pub size: f32,
    /// Leave my own reaction out of the avatars.
    pub hide_self: bool,
}
/// The most avatars a chip shows.
pub const REACTION_AVATARS_MAX: usize = 10;
/// Parses `{"enabled","max","size","hide_self"}` (all optional: on, 3, "small", false). `Ok(None)` = switched off.
fn reaction_avatars(a: &Value) -> Result<Option<ReactionAvatars>, &'static str> {
    let Some(o) = a.as_object() else { return Err("object expected") };
    if !o.get("enabled").and_then(Value::as_bool).unwrap_or(true) { return Ok(None) }
    let max = o.get("max").and_then(Value::as_u64).unwrap_or(3).clamp(1, REACTION_AVATARS_MAX as u64) as usize;
    let size = match o.get("size").and_then(Value::as_str).unwrap_or("small") { "small" => 14.0, "medium" => 18.0, _ => return Err("size: small or medium") };
    Ok(Some(ReactionAvatars { max, size, hide_self: o.get("hide_self").and_then(Value::as_bool).unwrap_or(false) }))
}

type Ptr = *mut c_void;
/// The plugin's function table after checking that no entry is null.
#[derive(Clone, Copy)]
struct Calls {
    start: extern "C" fn(Ptr, *const HostApi), stop: extern "C" fn(Ptr),
    on_send: extern "C" fn(Ptr, *const c_char) -> *mut c_char,
    on_message: extern "C" fn(Ptr, *const c_char, *const c_char) -> *mut c_char,
    command: extern "C" fn(Ptr, *const c_char, *const c_char) -> *mut c_char,
    palette: extern "C" fn(Ptr, *mut Palette) -> i32, free_str: extern "C" fn(*mut c_char), destroy: extern "C" fn(Ptr),
    /// Contract 1.2 entries (optional; never read from an older plugin, whose table ends before them).
    schema: Option<extern "C" fn(Ptr) -> *mut c_char>, changed: Option<extern "C" fn(Ptr, *const c_char)>,
    action: Option<extern "C" fn(Ptr, *const c_char) -> *mut c_char>,
    /// Contract 1.3 (accent themes).
    accents: Option<extern "C" fn(Ptr, *const ThemeInfo, *mut Accents) -> i32>,
    /// Contract 1.4 (decorations).
    decor: Option<extern "C" fn(Ptr, *const ThemeInfo, *mut Decor) -> i32>,
    // 1.6
    /// Contract 1.6 (named events).
    on_event: Option<extern "C" fn(Ptr, *const c_char, *const c_char) -> *mut c_char>,
}
fn calls(v: &PluginVTable, minor: u16) -> Option<Calls> {
    let (schema, changed, action) = if minor >= 2 { (v.settings_schema, v.settings_changed, v.settings_action) } else { (None, None, None) };
    let accents = if minor >= 3 { v.accents } else { None };
    let decor = if minor >= 4 { v.decor } else { None };
    // 1.6
    let on_event = if minor >= 6 { v.on_event } else { None };
    Some(Calls { start: v.start?, stop: v.stop?, on_send: v.on_send?, on_message: v.on_message?, command: v.command?,
        palette: v.palette?, free_str: v.free_str?, destroy: v.destroy?, schema, changed, action, accents, decor, on_event })
}
/// The events a plugin subscribed to (`PluginVTable::events`, 1.6; older tables end before it, so it is not read there).
fn events(v: &PluginVTable, minor: u16) -> Vec<String> {
    if minor >= 6 && v.on_event.is_some() { s(v.events).split_whitespace().map(String::from).collect() } else { vec![] }
}
/// The plugin's decorations for `th`, if it has any.
fn decor_of(l: &Loaded, th: &ThemeInfo) -> Option<Decor> {
    let f = l.c.decor?;
    let mut d = Decor::default();
    (f(l.inst, th, &mut d) == 1).then_some(d)
}
/// The plugin's accents for `th`, if it is an accent theme.
fn accents_of(l: &Loaded, th: &ThemeInfo) -> Option<Accents> {
    let f = l.c.accents?;
    let mut a = Accents::default();
    (f(l.inst, th, &mut a) == 1).then_some(a)
}

// Field order is drop order: the library is unloaded last.
pub struct Loaded { inst: Ptr, c: Calls, events: Vec<String>, host: Box<HostApi>, _ctx: Box<Ctx>, _lib: libloading::Library }
impl Loaded {
    /// Takes a string the plugin returned (null = None) and hands it back to the plugin's allocator.
    fn take(&self, r: *mut c_char) -> Option<String> { if r.is_null() { None } else { let t = s(r); (self.c.free_str)(r); Some(t) } }
    /// Stops the plugin. Its open `ui.confirm` questions are dropped first (answered "no"), so threads it joins in `stop` are not
    /// left waiting for a dialog the window cannot show while it is inside `stop`.
    fn stop(&self) {
        lock(&self._ctx.confirms).retain(|q| q.plugin != self._ctx.plugin);
        (self.c.stop)(self.inst)
    }
    /// Sends event `name` (one of `EVENTS`) to this plugin if it subscribed to it: its answer, if any.
    fn event(&self, name: &str, json: &str) -> Option<String> {
        let f = self.c.on_event?;
        if !self.events.iter().any(|e| e == name) { return None; }
        let (n, j) = (cs(name), cs(json));
        self.take(f(self.inst, n.as_ptr(), j.as_ptr()))
    }
}

pub struct Entry {
    pub file: PathBuf, pub name: String, pub author: String, pub version: String,
    pub description: String, pub commands: Vec<String>, pub enabled: bool, pub error: Option<String>,
    /// Set for plugins installed from a repository (they can be updated and uninstalled).
    pub source: Option<store::Installed>,
    /// Compiled into the app (no library file; shown with a "Built-in" badge, cannot be uninstalled).
    pub builtin: bool,
    /// The key in plugins.json: the package id for repository installs, the built-in key, else the file name.
    pub key: String,
    /// Its settings: schema and current values (plugins/settings.rs).
    pub settings: ps::State,
    /// The English texts and the plugin's translations (plugins/i18n.rs).
    labels: i18n::Labels,
    loaded: Option<Loaded>,
}

pub struct Plugins {
    pub dirs: Vec<PathBuf>, pub list: Vec<Entry>, pub out: Arc<Mutex<Outbox>>,
    /// Where repository plugins are installed (`plugin_repo::store::install_root`).
    pub install_root: PathBuf,
    data: Arc<Mutex<Map<String, Value>>>, state_file: PathBuf, data_file: PathBuf,
    /// Saved setting values of every plugin (`<config>/plugin-settings/`, secrets in the OS keyring).
    pub(crate) settings_store: ps::Store,
    recent: Recents,
    /// Message ids `message.received` already fired for (the set, and the same ids oldest first to forget them).
    seen: (HashSet<String>, VecDeque<String>),
    /// Deleted messages and earlier texts plugins keep showing (`message.deleted` / `message.edited` / `message.keep`).
    kept: Arc<Mutex<kept::Kept>>,
    /// Keys `message.deleted` ("d" + id) / `message.edited` ("e" + id + new text) already fired for, like `seen`.
    fired: (HashSet<String>, VecDeque<String>),
    /// Spaces and people for plugin threads (`spaces.list`, `people.search`), filled by `feed` like `recent`.
    view: Arc<Mutex<view::View>>,
    /// Whether `feed` copied everything since a plugin subscribed to `message.received`.
    fed: bool,
    /// Open `ui.confirm` questions, oldest first (the window shows the first).
    confirms: confirm::Confirms,
    ui_thread: std::thread::ThreadId,
    /// The `ui.dialog` on screen (moved here from the outbox by the window).
    pub dialog: Option<dialog::Dialog>,
    /// `i18n::generation` the plugins' texts were last translated for.
    i18n_gen: u64,
    /// Bumped whenever a plugin is loaded, unloaded, switched or its settings change: what the plugins answer for the theme
    /// (palette, accents, decorations) is resolved again only then (`App::apply_theme`).
    pub(crate) generation: u64,
}

fn s(p: *const c_char) -> String { if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() } }
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(|e| e.into_inner()) }
/// Host callbacks never unwind into plugin code (a panic in an `extern "C"` fn would abort the app) and ignore a null context.
fn guard<R>(c: Ptr, fallback: R, f: impl FnOnce(&Ctx) -> R) -> R {
    if c.is_null() { return fallback; }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(unsafe { &*(c as *const Ctx) }))).unwrap_or(fallback)
}
/// Puts something in the outbox and wakes the window to handle it.
fn post(c: &Ctx, f: impl FnOnce(&mut Outbox)) { f(&mut lock(&c.out)); crate::ui::wake(); }
fn toast(c: &Ctx, t: String) { post(c, |o| o.toasts.push((i18n::tr(&c.strings, &c.name), i18n::tr(&c.strings, &t)))) }
fn log(c: &Ctx, t: &str) { post(c, |o| o.logs.push(format!("[{}] {t}", c.plugin))) }
extern "C" fn h_toast(c: Ptr, t: *const c_char) { guard(c, (), |c| toast(c, s(t))) }
extern "C" fn h_send(c: Ptr, t: *const c_char) { guard(c, (), |c| post(c, |o| o.sends.push(s(t)))) }
extern "C" fn h_log(c: Ptr, t: *const c_char) { guard(c, (), |c| log(c, &s(t))) }
extern "C" fn h_get(c: Ptr, k: *const c_char) -> *mut c_char {
    guard(c, std::ptr::null_mut(), |c| match lock(&c.data).get(&c.plugin).and_then(|m| m.get(s(k))).and_then(|v| v.as_str()) {
        Some(v) => cs(v).into_raw(), None => std::ptr::null_mut(),
    })
}
extern "C" fn h_set(c: Ptr, k: *const c_char, v: *const c_char) {
    guard(c, (), |c| {
        let mut d = lock(&c.data);
        let m = d.entry(c.plugin.clone()).or_insert_with(|| Value::Object(Map::new()));
        if !m.is_object() { *m = Value::Object(Map::new()); }
        m[s(k)] = Value::String(s(v));
        let _ = std::fs::write(&c.data_file, serde_json::to_string_pretty(&*d).unwrap_or_default());
    })
}
extern "C" fn h_settings(c: Ptr) -> *mut c_char { guard(c, std::ptr::null_mut(), |c| cs(&Value::Object(lock(&c.settings).clone()).to_string()).into_raw()) }
extern "C" fn h_free(p: *mut c_char) { if !p.is_null() { drop(unsafe { CString::from_raw(p) }) } }

// Contract 1.6: named methods and events (docs/plugins.md, "Events and methods"). New capabilities are added here, not to the
// tables. Names: `<area>.<verb>` for methods, `<area>.<past participle>` for events. The number is the payload version: it
// goes up when a payload gains an optional field; anything incompatible gets a new name.

/// A host method: the parsed JSON argument (Null when missing or not JSON) -> its JSON result.
type Method = fn(&Ctx, &Value) -> Value;
/// What `HostApi::call` answers: (name, version, handler).
const METHODS: &[(&str, u32, Method)] = &[
    ("host.capabilities", 1, |_, _| capabilities()),
    ("ui.toast", 1, |c, a| with_text(a, |t| toast(c, t.to_string()))),
    ("log.write", 1, |c, a| with_text(a, |t| log(c, t))),
    ("settings.get", 1, |c, _| Value::Object(lock(&c.settings).clone())),
    // `{"space","text"}`: posted as a new plain-text message on the next frame (through the plugins' send hooks; dropped for a
    // space I cannot write in). The composer is not touched.
    ("space.send", 1, |c, a| match (a.get("space").and_then(Value::as_str), a.get("text").and_then(Value::as_str)) {
        (Some(sp), Some(t)) => { post(c, |o| o.sends_to.push((sp.to_string(), t.to_string()))); Value::Object(Map::new()) }
        _ => serde_json::json!({ "error": "space and text required" }),
    }),
    // `{"space","limit"}`: the last `limit` loaded messages, oldest first, without system lines. No network request.
    // 2: each message also has "id", "email", "date", "time" (UTC) and "published_ms".
    ("space.history", 2, |c, a| {
        let Some(sp) = a.get("space").and_then(Value::as_str) else { return serde_json::json!({ "error": "space missing" }) };
        let n = a.get("limit").and_then(Value::as_u64).unwrap_or(RECENT_MAX as u64) as usize;
        let r = lock(&c.recent);
        let l = r.get(sp).map_or(&[][..], |l| &l[l.len().saturating_sub(n)..]);
        serde_json::json!({ "messages": l.iter().map(view::message_json).collect::<Vec<_>>() })
    }),
    // `{"limit","query"}`: the spaces of the list (hidden ones left out), latest activity first: `{"spaces": [{"id","title",
    // "direct","unread","last","writable"}]}`. Same feed as `space.history`.
    ("spaces.list", 1, view::spaces_list),
    // `{"query","space","limit"}`: loaded messages containing `query`, newest first, with "space" / "space_title".
    ("messages.search", 1, view::messages_search),
    // `{"query","limit"}`: known people by name or e-mail, with their presence: `{"people": [{"id","name","email","status"}]}`.
    ("people.search", 1, view::people_search),
    // `{"text","title","ok","cancel","timeout"}`: asks the person in a dialog; blocks the calling (plugin) thread until the
    // answer `{"ok": bool}`. Not on the UI thread.
    ("ui.confirm", 1, confirm::ui_confirm),
    // `{"id","value"}`: saves one of the plugin's own settings as if changed on its page (validated; passwords to the keyring),
    // on the next frame; the plugin then gets `settings.changed`.
    ("settings.set", 1, |c, a| match (a.get("id").and_then(Value::as_str), a.get("value")) {
        (Some(id), Some(v)) => { post(c, |o| o.settings_set.push((c.plugin.clone(), id.to_string(), v.clone()))); Value::Object(Map::new()) }
        _ => serde_json::json!({ "error": "id and value required" }),
    }),
    // `{"text"}`: puts the text on the clipboard (next frame).
    ("clipboard.write", 1, |c, a| with_text(a, |t| post(c, |o| o.clipboard = Some(t.to_string())))),
    // `{"deleted": [...], "edited": [...], "replace": bool}`: deleted messages to keep showing and earlier texts to attach
    // (records as the events deliver them, plus "style" / "history"); used to restore them after a restart. Local only.
    // 2: deleted records take "style": "plain", "collapsed" and "attachments"; edited records spans, "current" and "inline".
    ("message.keep", 2, |c, a| {
        let r = lock(&c.kept).keep(a);
        post(c, |o| o.kept_changed = true);
        r
    }),
    // `{"enabled","max","size","hide_self"}`: avatars of the people who reacted after each reaction chip's count. Safe from any thread.
    ("reactions.show_avatars", 1, |c, a| match reaction_avatars(a) {
        Ok(v) => { *lock(&c.avatars) = v; crate::ui::wake(); Value::Object(Map::new()) }
        Err(e) => serde_json::json!({ "error": e }),
    }),
    // Camera pictures (media/camera/hook.rs). Pictures never travel as JSON: the plugin passes addresses as numbers.
    // `{"fn": <FrameFn address>, "ctx": <address>}`: the plugin's frame filter (one per plugin), called once per camera frame on
    // a thread of its own under a time budget. `{"fn": 0, "ctx"?}` removes it and waits for a call still running.
    ("camera.filter", 1, |c, a| {
        let (f, ctx) = (a.get("fn").and_then(Value::as_u64).unwrap_or(0), a.get("ctx").and_then(Value::as_u64).unwrap_or(0));
        if f == 0 { crate::media::camera::hook::remove_filter(&c.plugin, a.get("ctx").and_then(Value::as_u64).map(|x| x as usize)); return Value::Object(Map::new()); }
        // SAFETY: the plugin hands in the address of an `extern "C" fn(*mut c_void, *mut VideoFrame) -> i32` (plugin-api's
        // `Host::camera_filter`); it stays valid until the filter is removed, which `stop` / unloading does first.
        let f: collabhost_plugin_api::FrameFn = unsafe { std::mem::transmute::<usize, collabhost_plugin_api::FrameFn>(f as usize) };
        ok_or_error(crate::media::camera::hook::add_filter(&c.plugin, f, ctx as usize))
    }),
    // `{"path": "..."}`: a video file (looping) or an image replaces the camera; `""` brings the camera back.
    ("camera.source", 1, |c, a| {
        let p = a.get("path").and_then(Value::as_str).unwrap_or_default();
        let r = crate::media::camera::hook::set_source(&c.plugin, (!p.is_empty()).then(|| PathBuf::from(p)));
        crate::ui::wake(); // the Settings > Video preview reopens with it
        ok_or_error(r)
    }),
    // `{"path", "width", "height"}` -> `{"id", "still"}`: a video (looping) or an image as pictures of that size, for `media.read`.
    ("media.open", 1, |c, a| {
        let n = |k: &str| a.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
        let Some(p) = a.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()) else { return serde_json::json!({ "error": "path missing" }) };
        match crate::media::camera::hook::media_open(&c.plugin, Path::new(p), n("width"), n("height")) {
            Ok((id, still)) => serde_json::json!({ "id": id, "still": still }),
            Err(e) => serde_json::json!({ "error": e }),
        }
    }),
    // `{"id", "data": <address>, "width", "height", "stride"}` -> `{"pts"}`: copies the next picture (RGBA) into the buffer.
    ("media.read", 1, |c, a| {
        let n = |k: &str| a.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
        let (ptr, w, h, stride) = (n("data"), n("width"), n("height"), n("stride"));
        if ptr == 0 || w == 0 || h == 0 || stride < w * 4 { return serde_json::json!({ "error": "data, width, height, stride required" }); }
        // SAFETY: the plugin owns `stride * height` writable bytes at `data` for the duration of the call.
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, stride * h) };
        match crate::media::camera::hook::media_read(&c.plugin, n("id") as u64, dst, w, h, stride) {
            Ok(pts) => serde_json::json!({ "pts": pts }),
            Err(e) => serde_json::json!({ "error": e }),
        }
    }),
    // `{"id"}` closes one clip, `{}` all of this plugin's.
    ("media.close", 1, |c, a| { crate::media::camera::hook::media_close(&c.plugin, a.get("id").and_then(Value::as_u64)); Value::Object(Map::new()) }),
    // `{"title", "tabs": [{"label"|"time_ms", "text", "alt"?, "disabled"?, "hint"?}], "selected"?, "switch"?: {"label","on"}}`: a
    // read-only dialog of rich-text tabs (spans as in `message.keep`), optionally with a switch to each tab's `alt` text. Does
    // not block (fine on the UI thread); a new one replaces the one shown.
    ("ui.dialog", 1, dialog::ui_dialog),
    // `{}` -> `{"lang", "fallbacks"}`: the active language, as the `i18n.strings` event sends it (plugins/i18n.rs).
    (i18n::METHOD, 1, |_, _| i18n::language()),
];
/// An entry a plugin adds to a message's or a space's menu (`message.menu` / `space.menu`).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    /// The key of the plugin that offered it (`message.menu_action` / `space.menu_action` goes back to it).
    pub plugin: String,
    pub id: String, pub label: String,
    /// A Momentum icon name ("delete-regular"); empty = none.
    pub icon: String,
    pub danger: bool,
}
/// At most this many entries per plugin, labels at most this many characters.
const MENU_MAX: usize = 8;
const MENU_LABEL_MAX: usize = 80;
/// The entries in a `message.menu` answer: `{"items": [...]}` or the bare list; entries without an id or a label are skipped.
fn menu_items(plugin: &str, answer: &str) -> Vec<MenuItem> {
    let v: Value = serde_json::from_str(answer).unwrap_or(Value::Null);
    let list = v.get("items").unwrap_or(&v).as_array().cloned().unwrap_or_default();
    let st = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    list.iter().filter_map(|x| {
        let (id, label) = (st(x, "id"), st(x, "label"));
        (!id.is_empty() && !label.is_empty()).then(|| MenuItem { plugin: plugin.to_string(), id, label: label.chars().take(MENU_LABEL_MAX).collect(),
            icon: st(x, "icon"), danger: x.get("danger").and_then(Value::as_bool).unwrap_or(false) })
    }).take(MENU_MAX).collect()
}
fn ok_or_error(r: Result<(), String>) -> Value { match r { Ok(()) => Value::Object(Map::new()), Err(e) => serde_json::json!({ "error": e }) } }
/// The events Webox fires: (name, version). A plugin receives those it lists in `Plugin::EVENTS`.
const EVENTS: &[(&str, u32)] = &[
    // The plugin's setting values changed (and once before every start). Payload: all values, as `settings.get` returns them.
    ("settings.changed", 1),
    // A message arrived live, once per message id, also my own. Payload: `{"space","id","author","text","mine","direct",
    // "mentioned"}` (`direct` = 1:1 space, `mentioned` = it @-mentions me). Fired on the UI thread: return at once.
    ("message.received", 1),
    // A message was deleted (it became a tombstone or vanished), once per id. Payload: `{"space","space_name","id","author",
    // "email","text","mine","parent","published_ms","date","time"}` (the last known text; date / time UTC). Answer
    // `{"keep": true, "style": "text"|"overlay"}` to keep it on screen, drawn as deleted. Fired on the UI thread.
    // 2: "team" / "team_name" (the space's team; empty outside one).
    ("message.deleted", 2),
    // A message's text changed, once per id and new text. Payload: `{"space","space_name","id","author","mine","old_text",
    // "new_text"}`. Answer `{"history": [...]}` (earlier texts, oldest first) to show them above the text. UI thread.
    // 2: "email", "team" / "team_name", "published_ms", "old_edited" (the old text was itself an edit). The answer may also
    // give spans instead of strings, "current" (the current text as spans) and "inline": false (only "(edited)" shows).
    ("message.edited", 2),
    // A message's menu opens (right-click on the message or its "···" button). Payload: `{"space","id","deleted","edited",
    // "mine"}` (`deleted`: a kept deleted message; `edited`: edited per the service or with earlier texts shown). Answer
    // `{"items": [{"id","label","icon"?,"danger"?}]}` (or the bare list): entries appended to the menu after a separator.
    // UI thread, once per opening.
    ("message.menu", 1),
    // One of the plugin's own entries was picked. Payload: `{"space","id","item"}`; sent only to the plugin that offered it.
    ("message.menu_action", 1),
    // The menu of a space in the list opens (right-click on its row or the row's "···"). Payload: `{"space","title","direct",
    // "team"}`. Answer as `message.menu`. UI thread, once per opening.
    ("space.menu", 1),
    // One of the plugin's own space-menu entries was picked. Payload: `{"space","item"}`; sent only to that plugin.
    ("space.menu_action", 1),
    // "(edited)" (below a message a plugin logs edits of) or the header's "Edited" was clicked. Payload: `{"space","id",
    // "text","mine","published_ms"}` (`text`: the current plain text). Answer ignored; the plugin may open `ui.dialog`. UI thread.
    ("message.edit_tag_clicked", 1),
    // The plugin's translations are wanted: at start and after every language change. Payload: `{"lang","fallbacks"}`. Answer
    // `{"strings": {"English source": "translation"}}`, applied to everything Webox shows from the plugin (plugins/i18n.rs).
    (i18n::EVENT, 1),
];
/// `host.capabilities`: every method and event with its version, as one flat object.
fn capabilities() -> Value {
    Value::Object(METHODS.iter().map(|m| (m.0, m.1)).chain(EVENTS.iter().copied()).map(|(n, v)| (n.to_string(), v.into())).collect())
}
/// Runs `f` with the argument's `text`: `{}` on success, `{"error": ...}` without one.
fn with_text(a: &Value, f: impl FnOnce(&str)) -> Value {
    match a.get("text").and_then(Value::as_str) { Some(t) => { f(t); Value::Object(Map::new()) } None => serde_json::json!({ "error": "text missing" }) }
}
/// Runs `method` from `table`; null for an unknown method.
fn dispatch(table: &[(&str, u32, Method)], c: &Ctx, method: &str, json: &str) -> *mut c_char {
    let Some(m) = table.iter().find(|m| m.0 == method) else { return std::ptr::null_mut() };
    cs(&(m.2)(c, &serde_json::from_str(json).unwrap_or(Value::Null)).to_string()).into_raw()
}
extern "C" fn h_call(c: Ptr, m: *const c_char, j: *const c_char) -> *mut c_char { guard(c, std::ptr::null_mut(), |c| dispatch(METHODS, c, &s(m), &s(j))) }

fn read_json(p: &Path) -> Map<String, Value> {
    std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// The reason nothing outside the program folder may load while the side-loading policy is on.
pub fn policy_block_reason() -> String {
    crate::trq!("blocked: side-loading protection is on (policy {{0}}); only plugins next to Webox are loaded", crate::system::SIDE_LOADING_POLICY)
}

/// The plugin folder of the Linux package layout, relative to the program's folder (`<prefix>/bin`).
const PACKAGE_PLUGINS: &str = "../lib/collabhost/plugins";

/// Whether `dir` is one of the program's own plugin folders ("shipped": loaded even under the side-loading policy): inside the
/// program's folder, or (not on Windows) the package's `<prefix>/lib/collabhost/plugins`. Both sides are resolved first (`..`,
/// links); a folder that can't be resolved is not shipped.
fn is_shipped(dir: &Path, exe_dir: &Path) -> bool {
    let (Ok(d), Ok(x)) = (dir.canonicalize(), exe_dir.canonicalize()) else { return false };
    d.starts_with(&x) || (cfg!(not(windows)) && exe_dir.join(PACKAGE_PLUGINS).canonicalize().is_ok_and(|l| l == d))
}

impl Plugins {
    pub fn new(config: &Path) -> Self { Self::with_root(config, store::install_root(config)) }

    pub fn with_root(config: &Path, install_root: PathBuf) -> Self {
        let mut dirs = vec![config.join("plugins")];
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            dirs.push(exe_dir.join("plugins"));
            // Nix / Linux package layout (<prefix>/bin/webox). Not on Windows: next to a portable C:\<dir>\webox.exe that would be
            // C:\lib\collabhost\plugins, a folder any local user can create.
            if cfg!(not(windows)) { dirs.push(exe_dir.join(PACKAGE_PLUGINS)); }
        }
        if let Some(extra) = std::env::var_os("COLLABHOST_PLUGIN_PATH") { dirs.extend(std::env::split_paths(&extra)); }
        let _ = std::fs::create_dir_all(&dirs[0]);
        let data_file = config.join("plugin-data.json");
        let mut p = Plugins { dirs, list: vec![], out: Default::default(), install_root, data: Arc::new(Mutex::new(read_json(&data_file))),
            state_file: config.join("plugins.json"), data_file, settings_store: ps::Store::new(config), recent: Default::default(), seen: Default::default(),
            kept: Default::default(), fired: Default::default(), view: Default::default(), fed: false, confirms: Default::default(),
            ui_thread: std::thread::current().id(), dialog: None, i18n_gen: 0, generation: 0 };
        store::clean(&p.install_root);
        p.rescan();
        p
    }

    fn entry(file: PathBuf, key: &str, state: &Map<String, Value>, source: Option<store::Installed>) -> Entry {
        Entry { file, name: source.as_ref().map_or_else(|| key.to_string(), |i| i.name.clone()), author: String::new(), version: String::new(),
            description: String::new(), commands: vec![], enabled: state.get(key).and_then(Value::as_bool).unwrap_or(source.is_some()),
            error: None, source, builtin: false, key: key.to_string(), settings: Default::default(), labels: Default::default(), loaded: None }
    }

    pub fn rescan(&mut self) {
        self.generation += 1;
        for e in &mut self.list { unload(e) }
        self.list.clear();
        let state = read_json(&self.state_file);
        let ext = std::env::consts::DLL_EXTENSION;
        let mut seen = std::collections::HashSet::new();
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
        let protect = crate::system::side_loading_protection();
        let mut versions = vec![];
        for b in crate::extras::builtin_plugins() {
            let mut e = Entry { file: PathBuf::new(), name: b.name.into(), author: "Webox".into(), version: b.version.into(), description: b.description.into(),
                commands: vec![], enabled: state.get(b.key).and_then(Value::as_bool).unwrap_or(false), error: None, source: None, builtin: true,
                key: b.key.into(), settings: Default::default(), labels: i18n::Labels::builtin(b.name, b.description, b.settings), loaded: None };
            // Built-in plugins declare their settings in the same JSON format as native ones.
            e.settings.load(&self.settings_store, b.key, b.settings.map(|f| f()).as_deref());
            i18n::refresh(&mut e, false);
            self.list.push(e);
        }
        // Repository installs first: their file name wins over a loose copy of the same plugin.
        for inst in store::read(&self.install_root) {
            let mut e = Self::entry(store::path_of(&self.install_root, &inst), &inst.pkg, &state, Some(inst.clone()));
            seen.insert(stem_of(&e.file));
            // Loading runs the library's code: a switched-off plugin is listed from its install record and loaded when switched on.
            if protect { e.error = Some(policy_block_reason()); e.enabled = false; }
            else if let Err(x) = store::verify(&self.install_root, &inst) { e.error = Some(x); e.enabled = false; }
            else if e.enabled { self.load(&mut e, &inst.pkg) }
            versions.push(format!("{} version: {}", e.file.display(), inst.version));
            self.list.push(e);
        }
        for dir in self.dirs.clone().iter() {
            let shipped = exe_dir.as_ref().is_some_and(|x| is_shipped(dir, x));
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == ext)).collect();
            files.sort();
            for f in files {
                let stem = stem_of(&f);
                if !seen.insert(stem.clone()) { continue; }
                let mut e = Self::entry(f, &stem, &state, None);
                if protect && !shipped {
                    // With the side-loading policy on, nothing side-loaded runs.
                    e.error = Some(policy_block_reason());
                    e.enabled = false;
                } else if e.enabled {
                    // Switched off: listed by its file name only, loaded when switched on (loading runs its code).
                    self.load(&mut e, &stem);
                }
                versions.push(format!("{} version: {}", e.file.display(), if e.version.is_empty() { "unknown" } else { &e.version }));
                self.list.push(e);
            }
        }
        // "<file> version: <version>" for every library.
        if let Some(cfg) = self.state_file.parent() { crate::system::host_log_append(cfg, &versions); }
    }

    fn load(&self, e: &mut Entry, key: &str) {
        let res = (|| -> Result<Loaded, String> {
            // Loading a library runs its code: only files the person put in a plugin folder, or verified repository installs, get here.
            let lib = unsafe { libloading::Library::new(&e.file) }.map_err(|x| x.to_string())?;
            let entry: libloading::Symbol<extern "C" fn() -> *const PluginVTable> =
                unsafe { lib.get(ENTRY_SYMBOL) }.map_err(|_| crate::trq!("not a Webox plugin (collabhost_plugin missing)").into_owned())?;
            // Plugins from before the contract symbol were built for 1.0.
            let contract = unsafe { lib.get::<extern "C" fn() -> u32>(CONTRACT_SYMBOL) }.map(|f| f()).unwrap_or(1 << 16);
            let (major, minor) = unpack_contract(contract);
            check_contract(major, minor)?;
            let vt = entry();
            self.attach(e, key, vt, minor, lib)
        })();
        match res {
            Ok(l) => {
                if e.name.is_empty() { e.name = key.to_string(); }
                let schema = l.c.schema.and_then(|f| l.take(f(l.inst)));
                e.settings.load(&self.settings_store, key, schema.as_deref());
                i18n::capture(e);
                e.loaded = Some(l);
                let on = e.enabled;
                i18n::refresh(e, on); // before the start: what it shows at start is translated already
                if let Some(l) = e.loaded.as_ref().filter(|_| on) { push_settings(l, &e.settings); (l.c.start)(l.inst, &*l.host) }
            }
            Err(x) => { e.error = Some(x); e.enabled = false; }
        }
    }

    /// Checks a plugin's table (read only as far as contract `minor` has it) and gives the plugin its host context.
    fn attach(&self, e: &mut Entry, key: &str, vt: *const PluginVTable, minor: u16, lib: libloading::Library) -> Result<Loaded, String> {
        if vt.is_null() || unsafe { (*vt).abi } != ABI_VERSION { return Err(crate::trq!("built for a different plugin interface").into_owned()); }
        let v = unsafe { &*vt };
        let c = calls(v, minor).ok_or_else(|| crate::trq!("incomplete function table").into_owned())?;
        if v.instance.is_null() { return Err(crate::trq!("the plugin created no instance").into_owned()); }
        e.name = s(v.name); e.author = s(v.author); e.version = s(v.version); e.description = s(v.description);
        e.commands = s(v.commands).split_whitespace().map(String::from).collect();
        i18n::capture(e);
        let name = if e.name.trim().is_empty() { key.to_string() } else { e.name.clone() };
        let ctx = Box::new(Ctx { plugin: key.to_string(), name, out: self.out.clone(), data: self.data.clone(), data_file: self.data_file.clone(),
            settings: e.settings.values.clone(), recent: self.recent.clone(), kept: self.kept.clone(), view: self.view.clone(), confirms: self.confirms.clone(),
            ui_thread: Some(self.ui_thread), avatars: Mutex::new(None), strings: e.labels.strings.clone() });
        let host = Box::new(HostApi { ctx: &*ctx as *const Ctx as Ptr, toast: Some(h_toast), send: Some(h_send), log: Some(h_log),
            data_get: Some(h_get), data_set: Some(h_set), free_str: Some(h_free), settings_get: Some(h_settings),
            // 1.6
            call: Some(h_call) });
        Ok(Loaded { inst: v.instance, c, events: events(v, minor), host, _ctx: ctx, _lib: lib })
    }

    pub fn set_enabled(&mut self, i: usize, on: bool) {
        self.generation += 1;
        // Not loaded yet (switched off at start): load it now, which starts it. A blocked or broken one (error set) stays off;
        // a repository install is hashed again first.
        let e = &self.list[i];
        if on && !e.enabled && e.loaded.is_none() && !e.builtin && e.error.is_none() {
            let mut e = self.list.remove(i);
            e.enabled = true;
            let key = e.key.clone();
            match e.source.as_ref().map(|s| store::verify(&self.install_root, s)) {
                Some(Err(x)) => { e.error = Some(x); e.enabled = false; }
                _ => self.load(&mut e, &key),
            }
            self.list.insert(i, e);
            self.save_state();
            return;
        }
        let e = &mut self.list[i];
        if on && !e.enabled && e.loaded.is_some() { i18n::refresh(e, true); } // before the start, as in `load`
        if let Some(l) = &e.loaded {
            if on && !e.enabled { push_settings(l, &e.settings); (l.c.start)(l.inst, &*l.host) } else if !on && e.enabled { crate::media::camera::hook::release(&e.key); l.stop() }
        }
        e.enabled = on && (e.loaded.is_some() || e.builtin);
        self.save_state();
        self.drop_kept_unless_logging();
    }

    /// With no plugin listening to `message.deleted` / `message.edited` any more (switched off, uninstalled), nothing stays kept.
    fn drop_kept_unless_logging(&self) {
        if self.logs_messages() { return; }
        let mut k = lock(&self.kept);
        if !k.is_empty() { k.clear(); drop(k); lock(&self.out).kept_changed = true; }
    }

    /// Whether an enabled plugin listens to `message.deleted` or `message.edited`.
    pub fn logs_messages(&self) -> bool { self.subscribed("message.deleted") || self.subscribed("message.edited") }

    /// Whether the plugin with key is listed and switched on (the built-in plugins ask this every frame).
    pub fn is_enabled(&self, key: &str) -> bool { self.list.iter().any(|e| e.key == key && e.enabled) }

    fn save_state(&self) {
        let mut state = read_json(&self.state_file);
        for e in &self.list { state.insert(e.key.clone(), Value::Bool(e.enabled)); }
        let _ = std::fs::write(&self.state_file, serde_json::to_string_pretty(&state).unwrap_or_default());
    }

    /// The installed entry of a repository package.
    pub fn installed(&self, pkg: &str) -> Option<&store::Installed> { self.list.iter().filter_map(|e| e.source.as_ref()).find(|s| s.pkg == pkg) }

    /// Moves a verified download (`staged`, inside a `store::stage_dir`) into place and loads it, replacing an installed version
    /// of the same package. If the new version does not load, the old one is put back and loaded again.
    pub fn install(&mut self, inst: store::Installed, staged: &Path) -> Result<(), String> {
        self.generation += 1;
        let discard = || { if let Some(d) = staged.parent() { let _ = std::fs::remove_dir_all(d); } };
        if crate::system::side_loading_protection() { discard(); return Err(policy_block_reason()); }
        // Hashed again on this thread right before it can run: what is loaded is exactly what was pinned.
        if !store::sha256_file(staged).is_ok_and(|h| h.eq_ignore_ascii_case(&inst.sha256)) {
            discard();
            return Err(crate::trq!("Checksum (SHA-256) does not match; not installed").into_owned());
        }
        let old_i = self.list.iter().position(|e| e.source.as_ref().is_some_and(|s| s.pkg == inst.pkg));
        let enabled = old_i.map_or(true, |i| self.list[i].enabled || self.list[i].error.is_some());
        let old = old_i.map(|i| { let mut e = self.list.remove(i); unload(&mut e); e.source.unwrap() });
        let reload_old = |me: &mut Self| if let Some(o) = &old {
            let mut e = Self::entry(store::path_of(&me.install_root, o), &o.pkg, &Map::new(), Some(o.clone()));
            e.enabled = enabled;
            match store::verify(&me.install_root, o) { Ok(_) => me.load(&mut e, &o.pkg), Err(x) => e.error = Some(x) }
            me.list.push(e);
        };
        let c = match store::commit(&self.install_root, staged, &inst) {
            Ok(c) => c,
            Err(x) => { discard(); reload_old(self); return Err(x); }
        };
        let mut e = Self::entry(c.path.clone(), &inst.pkg, &Map::new(), Some(inst.clone()));
        e.enabled = enabled;
        self.load(&mut e, &inst.pkg);
        if let Some(x) = e.error.take() {
            store::rollback(&self.install_root, &inst, c);
            reload_old(self);
            return Err(crate::trq!("the new version cannot be loaded: {{0}}", x));
        }
        if let Err(x) = store::finish(&self.install_root, &inst, old.as_ref(), c) { unload(&mut e); return Err(x); }
        self.list.push(e);
        self.save_state();
        Ok(())
    }

    /// Unloads a repository plugin and deletes its files.
    pub fn uninstall(&mut self, pkg: &str) -> Result<(), String> {
        self.generation += 1;
        let i = self.list.iter().position(|e| e.source.as_ref().is_some_and(|s| s.pkg == pkg)).ok_or_else(|| crate::trq!("not installed").into_owned())?;
        let mut e = self.list.remove(i);
        unload(&mut e);
        self.drop_kept_unless_logging();
        store::remove(&self.install_root, pkg)?;
        let mut state = read_json(&self.state_file);
        state.remove(pkg);
        let _ = std::fs::write(&self.state_file, serde_json::to_string_pretty(&state).unwrap_or_default());
        Ok(())
    }

    fn active(&self) -> impl Iterator<Item = &Loaded> {
        self.list.iter().filter(|e| e.enabled).filter_map(|e| e.loaded.as_ref())
    }

    /// Runs every enabled plugin's send hook. None means a plugin blocked the message.
    pub fn on_send(&self, text: &str) -> Option<String> {
        let mut t = text.to_string();
        for l in self.active() {
            let c = cs(&t);
            if let Some(r) = l.take((l.c.on_send)(l.inst, c.as_ptr())) { t = r; if t.is_empty() { return None; } }
        }
        Some(t)
    }

    /// Fires event `name` (one of `EVENTS`) at every enabled plugin that subscribed to it: their answers.
    pub fn emit(&self, name: &str, json: &str) -> Vec<String> { self.active().filter_map(|l| l.event(name, json)).collect() }

    /// `message.menu`: the entries the enabled plugins add to the menu of a message (`payload` as the event documents it), in
    /// plugin order, each tagged with its plugin's key for `message_menu_action`. Without a subscribed plugin: none, and no
    /// plugin is called.
    pub fn message_menu(&self, payload: &Value) -> Vec<MenuItem> { self.menu("message.menu", payload) }

    /// `space.menu`: the same for the menu of a space in the list.
    pub fn space_menu(&self, payload: &Value) -> Vec<MenuItem> { self.menu("space.menu", payload) }

    fn menu(&self, event: &str, payload: &Value) -> Vec<MenuItem> {
        if !self.subscribed(event) { return vec![]; }
        let json = payload.to_string();
        self.list.iter().filter(|e| e.enabled).filter_map(|e| e.loaded.as_ref().map(|l| (e, l)))
            .filter_map(|(e, l)| l.event(event, &json).map(|a| (e, a)))
            .flat_map(|(e, a)| menu_items(&e.key, &a).into_iter().map(|mut m| { m.label = i18n::tr(&e.labels.strings, &m.label); m }).collect::<Vec<_>>()).collect()
    }

    /// `space.menu_action`: the entry `item` of the plugin `plugin` was picked in the menu of `space`. Sent to that plugin only.
    pub fn space_menu_action(&self, plugin: &str, space: &str, item: &str) {
        let Some(l) = self.list.iter().filter(|e| e.enabled && e.key == plugin).find_map(|e| e.loaded.as_ref()) else { return };
        l.event("space.menu_action", &serde_json::json!({ "space": space, "item": item }).to_string());
    }

    /// The `ui.dialog` to show now: None once its plugin is switched off or gone.
    pub fn dialog_shown(&self) -> Option<&dialog::Dialog> { self.dialog.as_ref().filter(|d| self.is_enabled(&d.plugin)) }

    /// `message.menu_action`: the entry `item` of the plugin `plugin` was picked on message `id` of `space`. Sent to that plugin only.
    pub fn message_menu_action(&self, plugin: &str, space: &str, id: &str, item: &str) {
        let Some(l) = self.list.iter().filter(|e| e.enabled && e.key == plugin).find_map(|e| e.loaded.as_ref()) else { return };
        l.event("message.menu_action", &serde_json::json!({ "space": space, "id": id, "item": item }).to_string());
    }

    /// Whether an enabled plugin subscribed to event `name`.
    pub fn subscribed(&self, name: &str) -> bool { self.active().any(|l| l.c.on_event.is_some() && l.events.iter().any(|e| e == name)) }

    /// Copies the newest messages of `space` (system lines left out) for `space.history`. The window calls it only while a
    /// plugin subscribes to `message.received`.
    pub fn note_space(&self, space: &str, list: &[crate::cloud::Message]) {
        let l = list.iter().filter(|m| m.notice.is_empty() && m.deleted == 0).rev().take(RECENT_MAX).collect::<Vec<_>>();
        let l = l.into_iter().rev().map(view::recent_of).collect();
        lock(&self.recent).insert(space.to_string(), l);
    }

    /// Fires `message.received` once per message id (a message can arrive twice, as a live event and in a reloaded list).
    #[allow(clippy::too_many_arguments)]
    pub fn message_received(&mut self, space: &str, id: &str, author: &str, text: &str, mine: bool, direct: bool, mentioned: bool) {
        if !self.seen.0.insert(id.to_string()) { return; }
        self.seen.1.push_back(id.to_string());
        if self.seen.1.len() > SEEN_MAX { if let Some(old) = self.seen.1.pop_front() { self.seen.0.remove(&old); } }
        let p = serde_json::json!({ "space": space, "id": id, "author": author, "text": text, "mine": mine, "direct": direct, "mentioned": mentioned });
        self.emit("message.received", &p.to_string());
    }

    /// A new list of `space` arrived and replaces `old` (None: the space had none loaded): fires `message.deleted` /
    /// `message.edited` for what changed (once per id / new text), keeps what the answers ask for, and draws everything kept
    /// into `new`. Without a listening plugin and nothing kept it does nothing, so `new` stays exactly as received.
    pub fn list_arrived(&mut self, at: kept::Where, old: Option<&[crate::cloud::Message]>, new: &mut Vec<crate::cloud::Message>) {
        let space = at.space;
        let (del, edit) = (self.subscribed("message.deleted"), self.subscribed("message.edited"));
        if let Some(old) = old.filter(|_| del || edit) {
            for ch in kept::changes(old, new) {
                let (key, name, payload) = match ch {
                    kept::Change::Deleted(m) if del => (format!("d{}", m.id), "message.deleted", kept::deleted_payload(at, m)),
                    kept::Change::Edited(m, t) if edit => (format!("e{}
{t}", m.id), "message.edited", kept::edited_payload(at, m, t)),
                    _ => continue,
                };
                if !self.fired.0.insert(key.clone()) { continue; }
                self.fired.1.push_back(key);
                if self.fired.1.len() > SEEN_MAX { if let Some(o) = self.fired.1.pop_front() { self.fired.0.remove(&o); } }
                let answers = self.emit(name, &payload.to_string());
                let answers: Vec<Value> = answers.iter().filter_map(|a| serde_json::from_str(a).ok()).collect();
                let mut k = lock(&self.kept);
                match ch {
                    kept::Change::Deleted(m) => if let Some(a) = answers.iter().find(|a| a["keep"] == true) {
                        k.keep_deleted(space, m.clone(), kept::DeletedLook::of(a));
                    },
                    kept::Change::Edited(m, _) => if let Some(h) = answers.iter().find(|a| a["history"].is_array()) { k.set_edit(&m.id, h) },
                }
            }
        }
        let mut k = lock(&self.kept);
        if !k.is_empty() { k.apply(space, new); }
    }

    /// After `message.keep` (or when nothing is kept any more): takes the old state out of a loaded list and draws the current one.
    pub fn reapply_kept(&self, space: &str, list: &mut Vec<crate::cloud::Message>) {
        let mut k = lock(&self.kept);
        k.strip(list);
        k.apply(space, list);
    }

    /// `reactions.show_avatars` of the last enabled plugin that turned it on; None (the default) draws no avatars.
    pub fn reaction_avatars(&self) -> Option<ReactionAvatars> { self.active().filter_map(|l| *lock(&l._ctx.avatars)).last() }

    /// Whether any loaded, enabled plugin can rewrite messages (`on_message` is the identity otherwise).
    pub fn rewrites_messages(&self) -> bool { self.active().next().is_some() }

    pub fn on_message(&self, author: &str, text: &str) -> String {
        let mut t = text.to_string();
        for l in self.active() {
            let (a, c) = (cs(author), cs(&t));
            if let Some(r) = l.take((l.c.on_message)(l.inst, a.as_ptr(), c.as_ptr())) { t = r; }
        }
        t
    }

    /// `/name args` -> Some(text to send, possibly empty) when a plugin owns the command.
    pub fn command(&self, line: &str) -> Option<String> {
        let rest = line.strip_prefix('/')?;
        let (name, args) = rest.split_once(' ').unwrap_or((rest, ""));
        let e = self.list.iter().find(|e| e.enabled && e.commands.iter().any(|c| c == name))?;
        let l = e.loaded.as_ref()?;
        let (n, a) = (cs(name), cs(args));
        Some(l.take((l.c.command)(l.inst, n.as_ptr(), a.as_ptr())).unwrap_or_default())
    }

    pub fn commands(&self) -> Vec<(String, String)> {
        self.list.iter().filter(|e| e.enabled).flat_map(|e| e.commands.iter().map(move |c| (c.clone(), e.name.clone()))).collect()
    }

    /// Whether the plugin at `i` is a design plugin (it supplies a colour palette), enabled or not.
    pub fn is_design(&self, i: usize) -> bool {
        self.list.get(i).and_then(|e| e.loaded.as_ref()).is_some_and(|l| {
            let mut p = Palette::default();
            (l.c.palette)(l.inst, &mut p) == 1 || accents_of(l, &ThemeInfo { size: std::mem::size_of::<ThemeInfo>() as u32, ..Default::default() }).is_some()
        })
    }

    /// Decorations of the last enabled design plugin that has any, for the base theme `th`.
    pub fn decor(&self, th: &ThemeInfo) -> Option<Decor> {
        let mut out = None;
        for l in self.active() { if let Some(d) = decor_of(l, th) { out = Some(d) } }
        out
    }

    /// Accent colours of the last enabled accent theme, computed for the base theme `th`.
    pub fn accents(&self, th: &ThemeInfo) -> Option<Accents> {
        let mut out = None;
        for l in self.active() { if let Some(a) = accents_of(l, th) { out = Some(a) } }
        out
    }

    /// The last enabled theme plugin wins, like the last theme in a theme list.
    pub fn palette(&self) -> Option<Palette> {
        let mut out = None;
        for l in self.active() { let mut p = Palette::default(); if (l.c.palette)(l.inst, &mut p) == 1 { out = Some(p) } }
        out
    }
}

fn stem_of(f: &Path) -> String { f.file_stem().unwrap_or_default().to_string_lossy().trim_start_matches("lib").to_string() }

fn unload(e: &mut Entry) {
    if let Some(l) = e.loaded.take() {
        // the camera hooks first: no frame filter may run while (or after) the plugin stops
        crate::media::camera::hook::release(&e.key);
        if e.enabled { l.stop() }
        (l.c.destroy)(l.inst);
    }
}

impl Drop for Plugins { fn drop(&mut self) { for e in &mut self.list { unload(e) } } }

#[cfg(test)]
mod e2e;
#[cfg(test)]
mod method_tests;
#[cfg(test)]
mod logger_tests;

#[cfg(test)]
pub(super) mod tests {
    #[derive(Default)]
    struct P;
    impl collabhost_plugin_api::Plugin for P {
        const NAME: &'static str = "P"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
    }

    #[test]
    fn settings_entries_are_read_only_from_contract_1_2() {
        let v = unsafe { &*collabhost_plugin_api::vtable::<P>() };
        // A 1.1 plugin's table ends before the settings entries: they must not be read at all.
        let c = super::calls(v, 1).unwrap();
        assert!(c.schema.is_none() && c.changed.is_none() && c.action.is_none());
        let c = super::calls(v, 2).unwrap();
        assert!(c.schema.is_some() && c.changed.is_some() && c.action.is_some());
        (c.destroy)(v.instance);
    }

    /// A 1.6 plugin that subscribes to two events, answers them with what it saw, panics on `test.boom` and calls the host on start.
    #[derive(Default)]
    struct Ev { seen: Vec<String>, caps: Option<String>, toast: Option<String>, nope: Option<String> }
    impl collabhost_plugin_api::Plugin for Ev {
        const NAME: &'static str = "Ev"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
        const EVENTS: &'static str = "settings.changed test.boom";
        fn start(&mut self, h: collabhost_plugin_api::Host) {
            self.caps = h.call("host.capabilities", "{}");
            self.toast = h.call("ui.toast", r#"{"text":"hi"}"#);
            self.nope = h.call("no.such", "{}");
            assert!(h.has("ui.toast") && h.version("settings.changed") == Some(1) && !h.has("no.such"));
        }
        fn on_event(&mut self, name: &str, payload: &str) -> Option<String> {
            if name == "test.boom" { panic!("boom") }
            self.seen.push(name.into());
            Some(format!("{}:{payload}", self.seen.len()))
        }
    }

    /// Loads plugin `T` from this test binary as if it were a library reporting contract 1.`minor`.
    pub(super) fn load_in_process<T: collabhost_plugin_api::Plugin>(minor: u16) -> (super::Plugins, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("collabhost-ev-{minor}-{}-{}", T::NAME, std::process::id()));
        let mut p = super::Plugins::with_root(&dir, dir.join("installed"));
        #[cfg(windows)] let lib: libloading::Library = libloading::os::windows::Library::this().unwrap().into();
        #[cfg(unix)] let lib: libloading::Library = libloading::os::unix::Library::this().into();
        // one key per plugin type: keyed state (the camera hooks) of tests running side by side stays apart
        let key = format!("ev-{}", T::NAME.to_lowercase());
        let mut e = super::Plugins::entry(dir.join("ev"), &key, &Default::default(), None);
        let l = p.attach(&mut e, &key, collabhost_plugin_api::vtable::<T>(), minor, lib).unwrap();
        e.loaded = Some(l);
        p.list.push(e);
        let i = p.list.len() - 1;
        p.set_enabled(i, true);
        (p, dir)
    }

    #[test]
    fn a_1_4_plugin_still_loads_and_gets_no_events() {
        let v = unsafe { &*collabhost_plugin_api::vtable::<Ev>() };
        // A 1.4 table ends before `events` / `on_event`: neither is read.
        assert!(super::calls(v, 4).unwrap().on_event.is_none() && super::events(v, 4).is_empty());
        (v.destroy.unwrap())(v.instance);
        let (p, dir) = load_in_process::<Ev>(4);
        assert!(p.list.last().unwrap().error.is_none());
        assert!(p.emit("settings.changed", "{}").is_empty());
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn events_reach_subscribers_only_and_panics_stay_inside() {
        let (p, dir) = load_in_process::<Ev>(6);
        assert_eq!(p.emit("settings.changed", r#"{"a":1}"#), vec![r#"1:{"a":1}"#.to_string()]);
        // Not subscribed: never called (the counter in the answer stays at 2 below).
        assert!(p.emit("test.other", "{}").is_empty());
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        assert!(p.emit("test.boom", "{}").is_empty());
        std::panic::set_hook(prev);
        assert_eq!(p.emit("settings.changed", "{}"), vec!["2:{}".to_string()]);
        // What the plugin got from `call` on start.
        let l = p.list.last().unwrap().loaded.as_ref().unwrap();
        let ev = unsafe { &*(l.inst as *const Ev) };
        let caps: serde_json::Value = serde_json::from_str(ev.caps.as_deref().unwrap()).unwrap();
        for (n, v) in super::METHODS.iter().map(|m| (m.0, m.1)).chain(super::EVENTS.iter().copied()) { assert_eq!(caps[n], v); }
        assert_eq!(caps.as_object().unwrap().len(), super::METHODS.len() + super::EVENTS.len());
        assert_eq!(ev.toast.as_deref(), Some("{}"));
        assert_eq!(ev.nope, None);
        // Toasts carry the plugin's shown name, not its key.
        assert_eq!(p.out.lock().unwrap().toasts, vec![("Ev".to_string(), "hi".to_string())]);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn host_methods_answer_and_never_unwind() {
        use std::ffi::CStr;
        let ctx = super::Ctx { plugin: "k".into(), name: "K".into(), out: Default::default(), data: Default::default(), data_file: Default::default(),
            settings: std::sync::Arc::new(std::sync::Mutex::new(serde_json::json!({ "x": true }).as_object().unwrap().clone())), recent: Default::default(), kept: Default::default(),
            view: Default::default(), confirms: Default::default(), ui_thread: None, avatars: Default::default(), strings: Default::default() };
        let c = &ctx as *const super::Ctx as super::Ptr;
        let call = |m: &str, j: &str| {
            let (m, j) = (collabhost_plugin_api::cs(m), collabhost_plugin_api::cs(j));
            let r = super::h_call(c, m.as_ptr(), j.as_ptr());
            if r.is_null() { return None; }
            let t = unsafe { CStr::from_ptr(r) }.to_string_lossy().into_owned();
            super::h_free(r);
            Some(t)
        };
        assert_eq!(call("nope", "{}"), None);
        assert_eq!(call("settings.get", "").as_deref(), Some(r#"{"x":true}"#));
        assert_eq!(call("ui.toast", "not json").as_deref(), Some(r#"{"error":"text missing"}"#));
        assert_eq!(call("log.write", r#"{"text":"t"}"#).as_deref(), Some("{}"));
        assert_eq!(ctx.out.lock().unwrap().logs, vec!["[k] t".to_string()]);
        // A null context and a panicking handler both answer null instead of unwinding into the plugin.
        assert!(super::h_call(std::ptr::null_mut(), std::ptr::null(), std::ptr::null()).is_null());
        let boom: &[(&str, u32, super::Method)] = &[("boom", 1, |_, _| panic!("boom"))];
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        assert!(super::guard(c, std::ptr::null_mut(), |c| super::dispatch(boom, c, "boom", "{}")).is_null());
        std::panic::set_hook(prev);
    }

    /// Records every `message.received` payload.
    static RECEIVED: std::sync::Mutex<Vec<serde_json::Value>> = std::sync::Mutex::new(vec![]);
    #[derive(Default)]
    struct Listener;
    impl collabhost_plugin_api::Plugin for Listener {
        const NAME: &'static str = "L"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
        const EVENTS: &'static str = "message.received";
        fn on_event(&mut self, _: &str, payload: &str) -> Option<String> { RECEIVED.lock().unwrap().push(serde_json::from_str(payload).unwrap()); None }
    }

    #[test]
    fn message_received_fires_once_per_id_and_only_to_subscribers() {
        // Ev subscribes to other events: nothing collects messages for it and it gets none (its counter stays at 0).
        let (mut p, dir) = load_in_process::<Ev>(6);
        assert!(!p.subscribed("message.received"));
        p.message_received("s", "x", "A", "t", false, true, false);
        assert_eq!(p.emit("settings.changed", "{}"), vec!["1:{}".to_string()]);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);

        let (mut p, dir) = load_in_process::<Listener>(6);
        assert!(p.subscribed("message.received"));
        p.message_received("s1", "m1", "Anna", "Hello", false, true, false);
        // The same activity again (live event + reloaded list): not a second time.
        p.message_received("s1", "m1", "Anna", "Hello", false, true, false);
        p.message_received("s2", "m2", "Me", "ok", true, false, true);
        // Events it did not subscribe to never reach it.
        assert!(p.emit("test.other", "{}").is_empty());
        assert_eq!(*RECEIVED.lock().unwrap(), [
            serde_json::json!({ "space": "s1", "id": "m1", "author": "Anna", "text": "Hello", "mine": false, "direct": true, "mentioned": false }),
            serde_json::json!({ "space": "s2", "id": "m2", "author": "Me", "text": "ok", "mine": true, "direct": false, "mentioned": true }),
        ]);
        // Switched off: not subscribed any more, nothing fires.
        let i = p.list.len() - 1;
        p.set_enabled(i, false);
        assert!(!p.subscribed("message.received"));
        p.message_received("s1", "m3", "Anna", "x", false, true, false);
        assert_eq!(RECEIVED.lock().unwrap().len(), 2);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn space_history_and_space_send_through_call() {
        use std::ffi::CStr;
        let dir = std::env::temp_dir().join(format!("collabhost-space-{}", std::process::id()));
        let p = super::Plugins::with_root(&dir, dir.join("installed"));
        let msg = |author: &str, mine: bool, text: &str| crate::cloud::Message { author: author.into(), mine, text: text.into(), ..Default::default() };
        let notice = crate::cloud::Message { notice: "Anna joined".into(), ..Default::default() };
        p.note_space("s1", &[msg("Anna", false, "one"), notice, msg("Me", true, "two"), msg("Anna", false, "three")]);
        let ctx = super::Ctx { plugin: "k".into(), name: "K".into(), out: p.out.clone(), data: Default::default(), data_file: Default::default(), settings: Default::default(), recent: p.recent.clone(), kept: Default::default(),
            view: p.view.clone(), confirms: Default::default(), ui_thread: None, avatars: Default::default(), strings: Default::default() };
        let c = &ctx as *const super::Ctx as super::Ptr;
        let call = |m: &str, j: serde_json::Value| {
            let (m, j) = (collabhost_plugin_api::cs(m), collabhost_plugin_api::cs(&j.to_string()));
            let r = super::h_call(c, m.as_ptr(), j.as_ptr());
            let t: serde_json::Value = serde_json::from_str(&unsafe { CStr::from_ptr(r) }.to_string_lossy()).unwrap();
            super::h_free(r);
            t
        };
        // The last `limit`, oldest first, system lines left out; an unknown space has none.
        assert_eq!(call("space.history", serde_json::json!({ "space": "s1", "limit": 2 })),
            serde_json::json!({ "messages": [
                { "id": "", "author": "Me", "email": "", "mine": true, "text": "two", "date": "", "time": "", "published_ms": 0 },
                { "id": "", "author": "Anna", "email": "", "mine": false, "text": "three", "date": "", "time": "", "published_ms": 0 }] }));
        assert_eq!(call("space.history", serde_json::json!({ "space": "s1" }))["messages"].as_array().unwrap().len(), 3);
        assert_eq!(call("space.history", serde_json::json!({ "space": "nope", "limit": 5 })), serde_json::json!({ "messages": [] }));
        assert!(call("space.history", serde_json::json!({})).get("error").is_some());
        assert_eq!(call("space.send", serde_json::json!({ "space": "s1", "text": "See you soon!" })), serde_json::json!({}));
        assert!(call("space.send", serde_json::json!({ "text": "where to?" })).get("error").is_some());
        let out = std::mem::take(&mut *p.out.lock().unwrap());
        assert_eq!(out.sends_to, [("s1".to_string(), "See you soon!".to_string())]);
        assert!(out.sends.is_empty(), "never the open space");
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Records `message.deleted` / `message.edited` and asks to keep everything: deleted ones as overlay, edits with the old text.
    static LOGGED: std::sync::Mutex<Vec<(String, serde_json::Value)>> = std::sync::Mutex::new(vec![]);
    #[derive(Default)]
    struct Logger;
    impl collabhost_plugin_api::Plugin for Logger {
        const NAME: &'static str = "Logger"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
        const EVENTS: &'static str = "message.deleted message.edited";
        fn on_event(&mut self, name: &str, payload: &str) -> Option<String> {
            let v: serde_json::Value = serde_json::from_str(payload).unwrap();
            LOGGED.lock().unwrap().push((name.into(), v.clone()));
            Some(match name {
                "message.deleted" => serde_json::json!({ "keep": true, "style": "overlay" }),
                _ => serde_json::json!({ "history": [v["old_text"]] }),
            }.to_string())
        }
    }

    fn msg(id: &str, text: &str, ms: i64) -> crate::cloud::Message {
        crate::cloud::Message { id: id.into(), author: "Anna".into(), text: text.into(), published_ms: ms, date: "2026-11-24".into(), time: "10:00".into(), ..Default::default() }
    }
    fn tomb(id: &str, ms: i64) -> crate::cloud::Message { crate::cloud::Message { id: id.into(), notice: "Anna deleted their own message.".into(), published_ms: ms, ..Default::default() } }
    type Row = (String, String, String, u8, Vec<String>);
    /// (id, text, notice, deleted, history as plain texts) of every row.
    fn rows(l: &[crate::cloud::Message]) -> Vec<Row> {
        l.iter().map(|m| (m.id.clone(), m.text.clone(), m.notice.clone(), m.deleted, m.history.iter().map(|h| h.iter().map(|s| s.0.as_str()).collect()).collect())).collect()
    }
    fn at<'a>(space: &'a str, name: &'a str) -> super::kept::Where<'a> { super::kept::Where { space, name, ..Default::default() } }
    fn row(id: &str, text: &str, notice: &str, deleted: u8, history: &[&str]) -> Row {
        (id.into(), text.into(), notice.into(), deleted, history.iter().map(|h| h.to_string()).collect())
    }
    const GONE: &str = "Anna deleted their own message.";

    #[test]
    fn deleted_and_edited_fire_once_and_what_is_kept_stays() {
        let (mut p, dir) = load_in_process::<Logger>(6);
        assert!(p.logs_messages());
        let old = vec![msg("m1", "one", 1000), msg("m2", "two", 2000), msg("m3", "three", 3000)];
        // m2 became a tombstone (the service puts it at the deletion time, its row has no `published_ms`), m3 was edited.
        let mut new = vec![msg("m1", "one", 1000), msg("m3", "three!", 3000), tomb("m2", 0)];
        p.list_arrived(at("s1", "Team"), Some(&old), &mut new);
        assert_eq!(rows(&new), [row("m1", "one", "", 0, &[]), row("m2", "two", "", super::kept::STYLE_OVERLAY, &[]), row("m3", "three!", "", 0, &["three"])]);
        {
            let l = LOGGED.lock().unwrap();
            assert_eq!(l.len(), 2);
            assert_eq!(l[0], ("message.deleted".to_string(), serde_json::json!({ "space": "s1", "space_name": "Team", "team": "", "team_name": "", "id": "m2", "author": "Anna",
                "email": "", "text": "two", "mine": false, "parent": "", "published_ms": 2000, "date": "2026-11-24", "time": "10:00" })));
            assert_eq!(l[1], ("message.edited".to_string(), serde_json::json!({ "space": "s1", "space_name": "Team", "team": "", "team_name": "", "id": "m3", "author": "Anna",
                "email": "", "mine": false, "published_ms": 3000, "old_edited": false, "old_text": "three", "new_text": "three!" })));
        }
        // The next load of the space (the tombstone again): no second event, the kept rows are drawn again.
        let mut again = vec![msg("m1", "one", 1000), msg("m3", "three!", 3000), tomb("m2", 0)];
        p.list_arrived(at("s1", "Team"), Some(&new), &mut again);
        assert_eq!(rows(&again), rows(&new));
        assert_eq!(LOGGED.lock().unwrap().len(), 2);
        // m1 vanished (a delete without a tombstone) inside the range the new list covers: kept where it stood.
        let mut gone = vec![msg("m0", "zero", 500), msg("m3", "three!", 3000), tomb("m2", 0)];
        p.list_arrived(at("s1", "Team"), Some(&again), &mut gone);
        assert_eq!(gone.iter().map(|m| (m.id.as_str(), m.deleted)).collect::<Vec<_>>(), [("m0", 0), ("m1", 2), ("m2", 2), ("m3", 0)]);
        assert_eq!(LOGGED.lock().unwrap().len(), 3);
        // A newer window that does not reach back to a message is not a delete of it.
        let mut newer = vec![msg("m3", "three!", 3000), msg("m4", "four", 4000)];
        p.list_arrived(at("s1", "Team"), Some(&gone), &mut newer);
        assert_eq!(LOGGED.lock().unwrap().len(), 3);
        // The plugin gives its log back with another style (a settings change): the full copy stays, only the style changes.
        p.kept.lock().unwrap().keep(&serde_json::json!({ "replace": true, "deleted": [{ "space": "s1", "id": "m2", "text": "from the log", "style": "text" }] }));
        let mut l = again.clone();
        p.reapply_kept("s1", &mut l);
        assert_eq!(rows(&l), [row("m1", "one", "", 0, &[]), row("m2", "two", "", super::kept::STYLE_TEXT, &[]), row("m3", "three!", "", 0, &[])]);
        // Switched off: nothing stays kept, the window draws its lists again (the tombstone comes back).
        let i = p.list.len() - 1;
        p.set_enabled(i, false);
        assert!(std::mem::take(&mut p.out.lock().unwrap().kept_changed));
        let mut l = again.clone();
        p.reapply_kept("s1", &mut l);
        assert_eq!(rows(&l), [row("m1", "one", "", 0, &[]), row("m2", "", GONE, 0, &[]), row("m3", "three!", "", 0, &[])]);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Adds two entries to the menu of deleted messages (one without a label, skipped) and records the picks.
    static MENU_SEEN: std::sync::Mutex<Vec<(String, serde_json::Value)>> = std::sync::Mutex::new(vec![]);
    #[derive(Default)]
    struct MenuPlugin;
    impl collabhost_plugin_api::Plugin for MenuPlugin {
        const NAME: &'static str = "MenuPlugin"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
        const EVENTS: &'static str = "message.menu message.menu_action";
        fn on_event(&mut self, name: &str, payload: &str) -> Option<String> {
            let v: serde_json::Value = serde_json::from_str(payload).unwrap();
            MENU_SEEN.lock().unwrap().push((name.into(), v.clone()));
            (name == "message.menu" && v["deleted"] == true).then(|| serde_json::json!({ "items": [
                { "id": "forget", "label": "Remove deleted message", "icon": "delete-regular", "danger": true },
                { "id": "nolabel" },
                { "id": "clear", "label": "Clear this space's log" }] }).to_string())
        }
    }

    #[test]
    fn plugin_menu_entries_round_trip() {
        let (p, dir) = load_in_process::<MenuPlugin>(6);
        let key = p.list.last().unwrap().key.clone();
        // A normal message: the plugin is asked, adds nothing.
        assert!(p.message_menu(&serde_json::json!({ "space": "s1", "id": "m1", "deleted": false, "edited": false, "mine": false })).is_empty());
        let items = p.message_menu(&serde_json::json!({ "space": "s1", "id": "m2", "deleted": true, "edited": false, "mine": true }));
        assert_eq!(items, [
            super::MenuItem { plugin: key.clone(), id: "forget".into(), label: "Remove deleted message".into(), icon: "delete-regular".into(), danger: true },
            super::MenuItem { plugin: key.clone(), id: "clear".into(), label: "Clear this space's log".into(), icon: String::new(), danger: false },
        ]);
        p.message_menu_action(&key, "s1", "m2", "forget");
        p.message_menu_action("someone-else", "s1", "m2", "forget"); // not this plugin's entry: never delivered
        {
            let seen = MENU_SEEN.lock().unwrap();
            assert_eq!(seen.len(), 3);
            assert_eq!(seen[1].1, serde_json::json!({ "space": "s1", "id": "m2", "deleted": true, "edited": false, "mine": true }));
            assert_eq!(seen[2], ("message.menu_action".to_string(), serde_json::json!({ "space": "s1", "id": "m2", "item": "forget" })));
        }
        // The bare list is an answer too; more than MENU_MAX entries are cut.
        let many: Vec<serde_json::Value> = (0..20).map(|i| serde_json::json!({ "id": format!("i{i}"), "label": "x" })).collect();
        assert_eq!(super::menu_items("k", &serde_json::Value::Array(many).to_string()).len(), super::MENU_MAX);
        assert!(super::menu_items("k", "no json").is_empty());
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_menu_plugin_the_menu_gets_nothing() {
        let dir = std::env::temp_dir().join(format!("collabhost-nomenu-{}", std::process::id()));
        let p = super::Plugins::with_root(&dir, dir.join("installed"));
        assert!(p.message_menu(&serde_json::json!({ "space": "s1", "id": "m1", "deleted": true, "edited": true, "mine": true })).is_empty());
        p.message_menu_action("x", "s1", "m1", "forget"); // nothing to deliver to, no panic
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_logging_plugin_lists_stay_as_received() {
        let dir = std::env::temp_dir().join(format!("collabhost-nolog-{}", std::process::id()));
        let mut p = super::Plugins::with_root(&dir, dir.join("installed"));
        assert!(!p.logs_messages());
        let old = vec![msg("m1", "one", 1000), msg("m2", "two", 2000)];
        let mut new = vec![tomb("m1", 1000), msg("m2", "two?", 2000)];
        p.list_arrived(at("s1", ""), Some(&old), &mut new);
        assert_eq!(rows(&new), [row("m1", "", GONE, 0, &[]), row("m2", "two?", "", 0, &[])]);
        let mut vanished = vec![msg("m2", "two?", 2000)];
        p.list_arrived(at("s1", ""), Some(&new), &mut vanished);
        assert_eq!(rows(&vanished), [row("m2", "two?", "", 0, &[])]);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn message_keep_restores_and_replaces() {
        use std::ffi::CStr;
        let dir = std::env::temp_dir().join(format!("collabhost-keep-{}", std::process::id()));
        let p = super::Plugins::with_root(&dir, dir.join("installed"));
        let ctx = super::Ctx { plugin: "k".into(), name: "K".into(), out: p.out.clone(), data: Default::default(), data_file: Default::default(), settings: Default::default(),
            recent: p.recent.clone(), kept: p.kept.clone(), view: Default::default(), confirms: Default::default(), ui_thread: None, avatars: Default::default(), strings: Default::default() };
        let c = &ctx as *const super::Ctx as super::Ptr;
        let call = |j: serde_json::Value| {
            let (m, j) = (collabhost_plugin_api::cs("message.keep"), collabhost_plugin_api::cs(&j.to_string()));
            let r = super::h_call(c, m.as_ptr(), j.as_ptr());
            let t = unsafe { CStr::from_ptr(r) }.to_string_lossy().into_owned();
            super::h_free(r);
            t
        };
        // After a restart: a deleted message (as `message.deleted` delivered it, plus a style) and an edit history.
        assert_eq!(call(serde_json::json!({
            "deleted": [{ "space": "s1", "id": "m2", "author": "Anna", "text": "two", "mine": false, "published_ms": 2000, "date": "2026-11-24", "time": "09:00", "style": "text" },
                        { "id": "no-space" }],
            "edited": [{ "id": "m3", "history": ["three", "three?"] }] })), "{}");
        assert!(std::mem::take(&mut p.out.lock().unwrap().kept_changed));
        let mut l = vec![msg("m1", "one", 1000), msg("m3", "three!", 3000)];
        p.reapply_kept("s1", &mut l);
        assert_eq!(rows(&l), [row("m1", "one", "", 0, &[]), row("m2", "two", "", super::kept::STYLE_TEXT, &[]), row("m3", "three!", "", 0, &["three", "three?"])]);
        assert_eq!((l[1].date.as_str(), l[1].time.as_str(), l[1].published_ms), ("2026-11-24", "09:00", 2000));
        // Another space is not touched.
        let mut other = vec![msg("x", "y", 2500)];
        p.reapply_kept("s2", &mut other);
        assert_eq!(rows(&other), [row("x", "y", "", 0, &[])]);
        // `replace` drops what was kept before: the rows go again.
        assert_eq!(call(serde_json::json!({ "replace": true })), "{}");
        p.reapply_kept("s1", &mut l);
        assert_eq!(rows(&l), [row("m1", "one", "", 0, &[]), row("m3", "three!", "", 0, &[])]);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Turns reaction avatars on in `start`, like the WhoReacted plugin.
    #[derive(Default)]
    struct Who;
    impl collabhost_plugin_api::Plugin for Who {
        const NAME: &'static str = "Who"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
        fn start(&mut self, h: collabhost_plugin_api::Host) {
            assert!(h.has("reactions.show_avatars"));
            assert_eq!(h.call("reactions.show_avatars", r#"{"max":5,"size":"medium"}"#).as_deref(), Some("{}"));
        }
    }

    #[test]
    fn reaction_avatars_are_off_by_default_and_follow_the_plugin() {
        use super::{reaction_avatars as parse, ReactionAvatars as A};
        use serde_json::json;
        // Defaults, clamping, switched off, refused values.
        assert_eq!(parse(&json!({})), Ok(Some(A { max: 3, size: 14.0, hide_self: false })));
        assert_eq!(parse(&json!({ "max": 0, "size": "small", "hide_self": true })), Ok(Some(A { max: 1, size: 14.0, hide_self: true })));
        assert_eq!(parse(&json!({ "max": 99, "size": "medium" })), Ok(Some(A { max: 10, size: 18.0, hide_self: false })));
        assert_eq!(parse(&json!({ "enabled": false, "max": 4 })), Ok(None));
        assert!(parse(&json!({ "size": "huge" })).is_err() && parse(&serde_json::Value::Null).is_err());

        // No plugin: Webox draws no avatars.
        let none = std::env::temp_dir().join(format!("collabhost-who-none-{}", std::process::id()));
        assert_eq!(super::Plugins::with_root(&none, none.join("installed")).reaction_avatars(), None);
        // A plugin that calls the method in `start`: on with its values; switched off: gone; on again: back.
        let (mut p, dir) = load_in_process::<Who>(6);
        assert_eq!(p.reaction_avatars(), Some(A { max: 5, size: 18.0, hide_self: false }));
        let i = p.list.len() - 1;
        p.set_enabled(i, false);
        assert_eq!(p.reaction_avatars(), None);
        p.set_enabled(i, true);
        assert_eq!(p.reaction_avatars().map(|a| a.max), Some(5));
        // `{"enabled": false}` (the plugin's toggle) turns them off; a bad call keeps the last state and answers an error.
        let c = &*p.list[i].loaded.as_ref().unwrap()._ctx as *const super::Ctx as super::Ptr;
        let call = |j: &str| {
            let (m, j) = (collabhost_plugin_api::cs("reactions.show_avatars"), collabhost_plugin_api::cs(j));
            let r = super::h_call(c, m.as_ptr(), j.as_ptr());
            let t = unsafe { std::ffi::CStr::from_ptr(r) }.to_string_lossy().into_owned();
            super::h_free(r);
            t
        };
        assert!(call(r#"{"size":"huge"}"#).contains("error"));
        assert_eq!(p.reaction_avatars().map(|a| a.size), Some(18.0));
        assert_eq!(call(r#"{"enabled":false}"#), "{}");
        assert_eq!(p.reaction_avatars(), None);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Registers a frame filter (red channel to 7) and a picture as the camera in `start`, like the Video plugin; drops the
    /// filter in `stop`.
    #[derive(Default)]
    struct Cam { filter: Option<collabhost_plugin_api::CameraFilter>, bad: Option<String> }
    static CAM_PNG: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);
    impl collabhost_plugin_api::Plugin for Cam {
        const NAME: &'static str = "Cam"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
        fn start(&mut self, h: collabhost_plugin_api::Host) {
            assert!(h.has("camera.filter") && h.has("camera.source") && h.has("media.read"));
            self.filter = h.camera_filter(|f| for y in 0..f.height { for x in 0..f.width { f.data[y * f.stride + x * 4] = 7; } });
            self.bad = h.call("camera.source", r#"{"path":"/no/such/file.mp4"}"#);
            let p = CAM_PNG.lock().unwrap().clone().unwrap();
            assert_eq!(h.call("camera.source", &serde_json::json!({ "path": p }).to_string()).as_deref(), Some("{}"));
        }
        fn stop(&mut self) { self.filter = None; }
    }

    #[test]
    fn camera_filter_and_source_follow_the_plugin() {
        use crate::media::camera::{hook, Pix, Raw};
        let png = std::env::temp_dir().join(format!("collabhost-cam-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255])).save(&png).unwrap();
        *CAM_PNG.lock().unwrap() = Some(png.clone());
        let raw = || Raw { pix: Pix::Rgb24, w: 4, h: 2, stride: 12, uv_off: 0, data: vec![100; 24] };
        let (mut p, dir) = load_in_process::<Cam>(6);
        let i = p.list.len() - 1;
        // the filter runs on the camera picture (a frame may miss the budget while its thread starts on a busy test machine:
        // then it is black, never the unfiltered picture), the source replaces the camera
        let filtered = || (1..=50).map(|n| hook::apply(raw(), n, 0)).inspect(|o| assert!(o.pix == Pix::Rgba && (o.data[..3] == [0, 0, 0] || o.data[..3] == [7, 100, 100]), "{:?}", &o.data[..4]))
            .any(|o| o.data[..4] == [7, 100, 100, 255]);
        assert!(filtered());
        assert!(hook::filter_stats("ev-cam").is_some());
        assert_eq!(hook::source().as_deref(), Some(png.as_path()));
        let cam = unsafe { &*(p.list[i].loaded.as_ref().unwrap().inst as *const Cam) };
        assert!(cam.bad.as_deref().is_some_and(|b| b.contains("error")), "{:?}", cam.bad);
        // switched off: both are gone before `stop` frees the filter; the camera picture passes untouched
        p.set_enabled(i, false);
        assert!(hook::filter_stats("ev-cam").is_none() && hook::source().is_none());
        assert_eq!(hook::apply(raw(), 2, 0).pix, Pix::Rgb24);
        // on again: back
        p.set_enabled(i, true);
        assert!(filtered());
        drop(p);
        assert!(hook::filter_stats("ev-cam").is_none() && hook::source().is_none());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_file(png);
    }

    #[test]
    fn example_plugin_runs() {
        let dir = std::env::temp_dir().join(format!("collabhost-test-{}", std::process::id()));
        let plug = dir.join("plugins");
        std::fs::create_dir_all(&plug).unwrap();
        let name = format!("{}collabhost_slash_commands.{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_EXTENSION);
        let Some(built) = crate::extras::plugin_repo::tests::built(&name) else { eprintln!("build the workspace first; skipping"); return };
        std::fs::copy(&built, plug.join(&name)).unwrap();
        let mut p = super::Plugins::with_root(&dir, dir.join("installed"));
        // Switched off (the default for a loose file): listed by its file name, not loaded.
        let i = p.list.iter().position(|e| e.key == "collabhost_slash_commands").expect("plugin listed");
        assert!(p.list[i].loaded.is_none() && !p.list[i].enabled && p.list[i].name == "collabhost_slash_commands");
        p.set_enabled(i, true);
        assert_eq!(p.list[i].name, "Slash Commands", "loaded when switched on");
        assert_eq!(p.command("/shrug hi").as_deref(), Some("hi \u{af}\\_(\u{30c4})_/\u{af}"));
        assert_eq!(p.on_send("ok :)").as_deref(), Some("ok \u{1f642}"));
        assert_eq!(p.command("/nope"), None);
        // /count is the plugin's one toast; starting shows none.
        assert!(p.out.lock().unwrap().toasts.is_empty());
        let _ = p.command("/count");
        assert!(!p.out.lock().unwrap().toasts.is_empty());
    }

    #[test]
    fn switched_off_plugins_are_not_loaded() {
        if crate::system::side_loading_protection() { eprintln!("side-loading policy on; skipping"); return; }
        let dir = std::env::temp_dir().join(format!("collabhost-test-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plug = dir.join("plugins");
        std::fs::create_dir_all(&plug).unwrap();
        // Not a library at all: any attempt to load it reports an error.
        std::fs::write(plug.join(format!("broken.{}", std::env::consts::DLL_EXTENSION)), b"not a library").unwrap();
        let mut p = super::Plugins::with_root(&dir, dir.join("installed"));
        let i = p.list.iter().position(|e| e.key == "broken").expect("listed");
        assert!(p.list[i].error.is_none() && p.list[i].loaded.is_none() && !p.list[i].enabled, "off: never opened");
        assert_eq!(p.list[i].name, "broken");
        p.set_enabled(i, true);
        let e = &p.list[i];
        assert!(e.error.is_some() && !e.enabled && e.key == "broken", "on: loaded (and refused) in place");
        // Switched on in plugins.json: loaded at start.
        std::fs::write(dir.join("plugins.json"), r#"{"broken": true}"#).unwrap();
        p.rescan();
        assert!(p.list.iter().find(|e| e.key == "broken").unwrap().error.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_programs_own_folders_are_shipped() {
        let root = std::env::temp_dir().join(format!("collabhost-test-shipped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (bin, lib, other) = (root.join("prefix/bin"), root.join("prefix/lib/collabhost/plugins"), root.join("lib/collabhost/plugins"));
        for d in [&bin.join("plugins"), &lib, &other] { std::fs::create_dir_all(d).unwrap(); }
        assert!(super::is_shipped(&bin.join("plugins"), &bin));
        assert!(super::is_shipped(&bin.join("plugins/../plugins"), &bin));
        // `..` is resolved: a folder above the program's folder is not inside it.
        assert!(!super::is_shipped(&bin.join("../../lib/collabhost/plugins"), &bin));
        assert!(!super::is_shipped(&root.join("prefix/bin/../.."), &bin));
        // The package layout <prefix>/bin + <prefix>/lib/collabhost/plugins (Linux only; on Windows that folder is not even searched).
        assert_eq!(super::is_shipped(&bin.join(super::PACKAGE_PLUGINS), &bin), cfg!(not(windows)));
        assert!(!super::is_shipped(&bin.join("missing"), &bin), "unresolvable: not shipped");
        let _ = std::fs::remove_dir_all(&root);
    }
}
