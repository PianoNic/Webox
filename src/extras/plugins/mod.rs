//! Loads native Rust plugins (`.dll` / `.so` built against collabhost-plugin-api): loose files from the plugin folders and the
//! plugins installed from a repository (`plugin_repo::store`, verified against their pinned sha256 before every load), and lists the
//! built-in, in-tree plugins (`crate::extras::builtin_plugins`: first-party code compiled into the app, e.g. GIF-Extras) next to them.
//! A built-in entry has no library: its switch is saved like the others (plugins.json, default off) and the app asks
//! `is_enabled(key)` every frame. Built-in plugins can be switched off but not uninstalled.
use super::plugin_repo::{index::check_contract, store};
use super::plugin_settings as ps;
pub mod settings;
use settings::push_settings;
use collabhost_plugin_api::{cs, unpack_contract, Accents, Decor, HostApi, Palette, PluginVTable, ThemeInfo, ABI_VERSION, CONTRACT_SYMBOL, ENTRY_SYMBOL};
use serde_json::{Map, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// What plugins asked the host to do; the window drains it every frame.
#[derive(Default)]
pub struct Outbox { pub toasts: Vec<String>, pub sends: Vec<String>, pub logs: Vec<String>,
    /// `space.send`: (space id, text).
    pub sends_to: Vec<(String, String)> }

/// One message of a space as `space.history` hands it out.
struct Recent { author: String, mine: bool, text: String }
/// The newest messages per space, copied from the window's loaded lists while a plugin subscribes to `message.received`, so
/// plugin threads can read them (`space.history`) without touching the UI's data.
type Recents = Arc<Mutex<HashMap<String, Vec<Recent>>>>;
/// How many messages per space `space.history` can return.
const RECENT_MAX: usize = 200;
/// How many message ids `message_received` remembers to fire once per id.
const SEEN_MAX: usize = 4096;

struct Ctx { plugin: String, out: Arc<Mutex<Outbox>>, data: Arc<Mutex<Map<String, Value>>>, data_file: PathBuf, settings: Arc<Mutex<Map<String, Value>>>,
    recent: Recents }

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
}

fn s(p: *const c_char) -> String { if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() } }
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(|e| e.into_inner()) }
/// Host callbacks never unwind into plugin code (a panic in an `extern "C"` fn would abort the app) and ignore a null context.
fn guard<R>(c: Ptr, fallback: R, f: impl FnOnce(&Ctx) -> R) -> R {
    if c.is_null() { return fallback; }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(unsafe { &*(c as *const Ctx) }))).unwrap_or(fallback)
}
extern "C" fn h_toast(c: Ptr, t: *const c_char) { guard(c, (), |c| lock(&c.out).toasts.push(format!("{}: {}", c.plugin, s(t)))) }
extern "C" fn h_send(c: Ptr, t: *const c_char) { guard(c, (), |c| lock(&c.out).sends.push(s(t))) }
extern "C" fn h_log(c: Ptr, t: *const c_char) { guard(c, (), |c| lock(&c.out).logs.push(format!("[{}] {}", c.plugin, s(t)))) }
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
    ("ui.toast", 1, |c, a| with_text(a, |t| lock(&c.out).toasts.push(format!("{}: {t}", c.plugin)))),
    ("log.write", 1, |c, a| with_text(a, |t| lock(&c.out).logs.push(format!("[{}] {t}", c.plugin)))),
    ("settings.get", 1, |c, _| Value::Object(lock(&c.settings).clone())),
    // `{"space","text"}`: posted as a new plain-text message on the next frame (through the plugins' send hooks; dropped for a
    // space I cannot write in). The composer is not touched.
    ("space.send", 1, |c, a| match (a.get("space").and_then(Value::as_str), a.get("text").and_then(Value::as_str)) {
        (Some(sp), Some(t)) => { lock(&c.out).sends_to.push((sp.to_string(), t.to_string())); Value::Object(Map::new()) }
        _ => serde_json::json!({ "error": "space und text erforderlich" }),
    }),
    // `{"space","limit"}`: the last `limit` loaded messages, oldest first, without system lines. No network request.
    ("space.history", 1, |c, a| {
        let Some(sp) = a.get("space").and_then(Value::as_str) else { return serde_json::json!({ "error": "space fehlt" }) };
        let n = a.get("limit").and_then(Value::as_u64).unwrap_or(RECENT_MAX as u64) as usize;
        let r = lock(&c.recent);
        let l = r.get(sp).map_or(&[][..], |l| &l[l.len().saturating_sub(n)..]);
        serde_json::json!({ "messages": l.iter().map(|m| serde_json::json!({ "author": m.author, "mine": m.mine, "text": m.text })).collect::<Vec<_>>() })
    }),
];
/// The events Webox fires: (name, version). A plugin receives those it lists in `Plugin::EVENTS`.
const EVENTS: &[(&str, u32)] = &[
    // The plugin's setting values changed (and once before every start). Payload: all values, as `settings.get` returns them.
    ("settings.changed", 1),
    // A message arrived live, once per message id, also my own. Payload: `{"space","id","author","text","mine","direct",
    // "mentioned"}` (`direct` = 1:1 space, `mentioned` = it @-mentions me). Fired on the UI thread: return at once.
    ("message.received", 1),
];
/// `host.capabilities`: every method and event with its version, as one flat object.
fn capabilities() -> Value {
    Value::Object(METHODS.iter().map(|m| (m.0, m.1)).chain(EVENTS.iter().copied()).map(|(n, v)| (n.to_string(), v.into())).collect())
}
/// Runs `f` with the argument's `text`: `{}` on success, `{"error": ...}` without one.
fn with_text(a: &Value, f: impl FnOnce(&str)) -> Value {
    match a.get("text").and_then(Value::as_str) { Some(t) => { f(t); Value::Object(Map::new()) } None => serde_json::json!({ "error": "text fehlt" }) }
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
    format!("blockiert: Schutz vor Side-Loading ist aktiv (Richtlinie {}); nur Plugins neben Webox werden geladen", crate::system::SIDE_LOADING_POLICY)
}

impl Plugins {
    pub fn new(config: &Path) -> Self { Self::with_root(config, store::install_root(config)) }

    pub fn with_root(config: &Path, install_root: PathBuf) -> Self {
        let mut dirs = vec![config.join("plugins")];
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            dirs.push(exe_dir.join("plugins"));
            dirs.push(exe_dir.join("../lib/collabhost/plugins")); // Nix / Linux package layout
        }
        if let Some(extra) = std::env::var_os("COLLABHOST_PLUGIN_PATH") { dirs.extend(std::env::split_paths(&extra)); }
        let _ = std::fs::create_dir_all(&dirs[0]);
        let data_file = config.join("plugin-data.json");
        let mut p = Plugins { dirs, list: vec![], out: Default::default(), install_root, data: Arc::new(Mutex::new(read_json(&data_file))),
            state_file: config.join("plugins.json"), data_file, settings_store: ps::Store::new(config), recent: Default::default(), seen: Default::default() };
        store::clean(&p.install_root);
        p.rescan();
        p
    }

    fn entry(file: PathBuf, key: &str, state: &Map<String, Value>, source: Option<store::Installed>) -> Entry {
        Entry { file, name: source.as_ref().map_or_else(|| key.to_string(), |i| i.name.clone()), author: String::new(), version: String::new(),
            description: String::new(), commands: vec![], enabled: state.get(key).and_then(Value::as_bool).unwrap_or(source.is_some()),
            error: None, source, builtin: false, key: key.to_string(), settings: Default::default(), loaded: None }
    }

    pub fn rescan(&mut self) {
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
                key: b.key.into(), settings: Default::default(), loaded: None };
            // Built-in plugins declare their settings in the same JSON format as native ones.
            e.settings.load(&self.settings_store, b.key, b.settings.map(|f| f()).as_deref());
            self.list.push(e);
        }
        // Repository installs first: their file name wins over a loose copy of the same plugin.
        for inst in store::read(&self.install_root) {
            let mut e = Self::entry(store::path_of(&self.install_root, &inst), &inst.pkg, &state, Some(inst.clone()));
            seen.insert(stem_of(&e.file));
            if protect { e.error = Some(policy_block_reason()); e.enabled = false; }
            else {
                match store::verify(&self.install_root, &inst) { Ok(_) => self.load(&mut e, &inst.pkg), Err(x) => { e.error = Some(x); e.enabled = false; } }
            }
            versions.push(format!("{} version: {}", e.file.display(), inst.version));
            self.list.push(e);
        }
        for dir in self.dirs.clone().iter() {
            let shipped = exe_dir.as_ref().is_some_and(|x| dir.starts_with(x));
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
                } else {
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
                unsafe { lib.get(ENTRY_SYMBOL) }.map_err(|_| "kein Webox-Plugin (collabhost_plugin fehlt)".to_string())?;
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
                if e.enabled { push_settings(&l, &e.settings); (l.c.start)(l.inst, &*l.host) }
                e.loaded = Some(l);
            }
            Err(x) => { e.error = Some(x); e.enabled = false; }
        }
    }

    /// Checks a plugin's table (read only as far as contract `minor` has it) and gives the plugin its host context.
    fn attach(&self, e: &mut Entry, key: &str, vt: *const PluginVTable, minor: u16, lib: libloading::Library) -> Result<Loaded, String> {
        if vt.is_null() || unsafe { (*vt).abi } != ABI_VERSION { return Err("für eine andere Plugin-Schnittstelle gebaut".into()); }
        let v = unsafe { &*vt };
        let c = calls(v, minor).ok_or("unvollständige Funktionstabelle")?;
        if v.instance.is_null() { return Err("Plugin hat keine Instanz angelegt".into()); }
        e.name = s(v.name); e.author = s(v.author); e.version = s(v.version); e.description = s(v.description);
        e.commands = s(v.commands).split_whitespace().map(String::from).collect();
        let ctx = Box::new(Ctx { plugin: key.to_string(), out: self.out.clone(), data: self.data.clone(), data_file: self.data_file.clone(),
            settings: e.settings.values.clone(), recent: self.recent.clone() });
        let host = Box::new(HostApi { ctx: &*ctx as *const Ctx as Ptr, toast: Some(h_toast), send: Some(h_send), log: Some(h_log),
            data_get: Some(h_get), data_set: Some(h_set), free_str: Some(h_free), settings_get: Some(h_settings),
            // 1.6
            call: Some(h_call) });
        Ok(Loaded { inst: v.instance, c, events: events(v, minor), host, _ctx: ctx, _lib: lib })
    }

    pub fn set_enabled(&mut self, i: usize, on: bool) {
        let e = &mut self.list[i];
        if let Some(l) = &e.loaded {
            if on && !e.enabled { push_settings(l, &e.settings); (l.c.start)(l.inst, &*l.host) } else if !on && e.enabled { (l.c.stop)(l.inst) }
        }
        e.enabled = on && (e.loaded.is_some() || e.builtin);
        self.save_state();
    }

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
        let discard = || { if let Some(d) = staged.parent() { let _ = std::fs::remove_dir_all(d); } };
        if crate::system::side_loading_protection() { discard(); return Err(policy_block_reason()); }
        // Hashed again on this thread right before it can run: what is loaded is exactly what was pinned.
        if !store::sha256_file(staged).is_ok_and(|h| h.eq_ignore_ascii_case(&inst.sha256)) {
            discard();
            return Err("Prüfsumme (SHA-256) stimmt nicht; nicht installiert".into());
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
            return Err(format!("neue Version lässt sich nicht laden: {x}"));
        }
        if let Err(x) = store::finish(&self.install_root, &inst, old.as_ref(), c) { unload(&mut e); return Err(x); }
        self.list.push(e);
        self.save_state();
        Ok(())
    }

    /// Unloads a repository plugin and deletes its files.
    pub fn uninstall(&mut self, pkg: &str) -> Result<(), String> {
        let i = self.list.iter().position(|e| e.source.as_ref().is_some_and(|s| s.pkg == pkg)).ok_or("nicht installiert")?;
        let mut e = self.list.remove(i);
        unload(&mut e);
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

    /// Whether an enabled plugin subscribed to event `name`.
    pub fn subscribed(&self, name: &str) -> bool { self.active().any(|l| l.c.on_event.is_some() && l.events.iter().any(|e| e == name)) }

    /// Copies the newest messages of `space` (system lines left out) for `space.history`. The window calls it only while a
    /// plugin subscribes to `message.received`.
    pub fn note_space(&self, space: &str, list: &[crate::cloud::Message]) {
        let l = list.iter().filter(|m| m.notice.is_empty()).rev().take(RECENT_MAX).collect::<Vec<_>>();
        let l = l.into_iter().rev().map(|m| Recent { author: m.author.clone(), mine: m.mine, text: m.text.clone() }).collect();
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
        if e.enabled { (l.c.stop)(l.inst) }
        (l.c.destroy)(l.inst);
    }
}

impl Drop for Plugins { fn drop(&mut self) { for e in &mut self.list { unload(e) } } }

#[cfg(test)]
mod tests {
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
    fn load_in_process<T: collabhost_plugin_api::Plugin>(minor: u16) -> (super::Plugins, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("collabhost-ev-{minor}-{}-{}", T::NAME, std::process::id()));
        let mut p = super::Plugins::with_root(&dir, dir.join("installed"));
        #[cfg(windows)] let lib: libloading::Library = libloading::os::windows::Library::this().unwrap().into();
        #[cfg(unix)] let lib: libloading::Library = libloading::os::unix::Library::this().into();
        let mut e = super::Plugins::entry(dir.join("ev"), "ev", &Default::default(), None);
        let l = p.attach(&mut e, "ev", collabhost_plugin_api::vtable::<T>(), minor, lib).unwrap();
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
        assert_eq!(p.out.lock().unwrap().toasts, vec!["ev: hi".to_string()]);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn host_methods_answer_and_never_unwind() {
        use std::ffi::CStr;
        let ctx = super::Ctx { plugin: "k".into(), out: Default::default(), data: Default::default(), data_file: Default::default(),
            settings: std::sync::Arc::new(std::sync::Mutex::new(serde_json::json!({ "x": true }).as_object().unwrap().clone())), recent: Default::default() };
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
        assert_eq!(call("ui.toast", "not json").as_deref(), Some(r#"{"error":"text fehlt"}"#));
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
        p.message_received("s1", "m1", "Anna", "Hallo", false, true, false);
        // The same activity again (live event + reloaded list): not a second time.
        p.message_received("s1", "m1", "Anna", "Hallo", false, true, false);
        p.message_received("s2", "m2", "Ich", "ok", true, false, true);
        // Events it did not subscribe to never reach it.
        assert!(p.emit("test.other", "{}").is_empty());
        assert_eq!(*RECEIVED.lock().unwrap(), [
            serde_json::json!({ "space": "s1", "id": "m1", "author": "Anna", "text": "Hallo", "mine": false, "direct": true, "mentioned": false }),
            serde_json::json!({ "space": "s2", "id": "m2", "author": "Ich", "text": "ok", "mine": true, "direct": false, "mentioned": true }),
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
        let notice = crate::cloud::Message { notice: "Anna ist beigetreten".into(), ..Default::default() };
        p.note_space("s1", &[msg("Anna", false, "eins"), notice, msg("Ich", true, "zwei"), msg("Anna", false, "drei")]);
        let ctx = super::Ctx { plugin: "k".into(), out: p.out.clone(), data: Default::default(), data_file: Default::default(), settings: Default::default(), recent: p.recent.clone() };
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
            serde_json::json!({ "messages": [{ "author": "Ich", "mine": true, "text": "zwei" }, { "author": "Anna", "mine": false, "text": "drei" }] }));
        assert_eq!(call("space.history", serde_json::json!({ "space": "s1" }))["messages"].as_array().unwrap().len(), 3);
        assert_eq!(call("space.history", serde_json::json!({ "space": "nope", "limit": 5 })), serde_json::json!({ "messages": [] }));
        assert!(call("space.history", serde_json::json!({})).get("error").is_some());
        assert_eq!(call("space.send", serde_json::json!({ "space": "s1", "text": "Bis gleich!" })), serde_json::json!({}));
        assert!(call("space.send", serde_json::json!({ "text": "wohin?" })).get("error").is_some());
        let out = std::mem::take(&mut *p.out.lock().unwrap());
        assert_eq!(out.sends_to, [("s1".to_string(), "Bis gleich!".to_string())]);
        assert!(out.sends.is_empty(), "never the open space");
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
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
        let i = p.list.iter().position(|e| e.name == "Slash Commands").expect("plugin listed");
        p.set_enabled(i, true);
        assert_eq!(p.command("/shrug hi").as_deref(), Some("hi \u{af}\\_(\u{30c4})_/\u{af}"));
        assert_eq!(p.on_send("ok :)").as_deref(), Some("ok \u{1f642}"));
        assert_eq!(p.command("/nope"), None);
        // /count is the plugin's one toast; starting shows none.
        assert!(p.out.lock().unwrap().toasts.is_empty());
        let _ = p.command("/count");
        assert!(!p.out.lock().unwrap().toasts.is_empty());
    }
}
