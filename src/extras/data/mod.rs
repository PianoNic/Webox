//! The data API (docs/data-api.md): one typed model (`webox-data/1`, `model.rs`) of what Webox already keeps in memory,
//! a store with change tracking (`store.rs`), built from app state (`build.rs`), the query language (`query.rs`), and
//! subscriptions with diffs (`subs.rs`). Plugins reach it through the host methods `data.model`, `data.get`, `data.query`,
//! `data.subscribe` / `data.unsubscribe` and the event `data.changed` (`api.rs`, registered in extras/plugins/mod.rs).
//!
//! Cost: nothing at all while no plugin uses the data API (the store is empty and `App::poll` checks one atomic flag). With
//! such a plugin, the store is updated only where app state changed: each `Resp` marks the kinds it touches, and a cheap
//! fingerprint of the window's own state (selection, flags, drafts, settings, call controls) catches changes made by the UI;
//! a frame without either does no store work. A rebuild of a kind compares field by field and records only real changes;
//! subscriptions run again only when a change touches a field they read, at most 10 times per second each.
pub mod build;
pub mod model;
pub mod query;
pub mod store;
pub mod subs;
pub mod api;
#[cfg(test)]
mod tests;

pub use build::Source;

/// The event a subscription's changes arrive as.
pub const EVENT: &str = "data.changed";
use model::Kind;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// At most this many subscriptions per plugin.
pub const SUBS_MAX: usize = 32;
/// A subscription runs at most every this long (10 per second).
pub const SUB_INTERVAL: Duration = Duration::from_millis(100);
/// Directory search results kept for the `people` collection.
const DIRECTORY_MAX: usize = 500;
/// Compiled `data.query` texts kept.
const CACHE_MAX: usize = 64;

/// Which parts of the store have to be rebuilt.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    /// Bit per `Kind`.
    pub kinds: u32,
    /// Spaces whose messages changed.
    pub spaces: HashSet<String>,
    /// Every space's messages.
    pub all_messages: bool,
}
impl Marks {
    pub fn all() -> Marks { Marks { kinds: u32::MAX, spaces: HashSet::new(), all_messages: true } }
    pub fn of(kinds: &[Kind]) -> Marks { Marks { kinds: kinds.iter().fold(0, |m, k| m | k.bit()), ..Default::default() } }
    pub fn space(mut self, id: &str) -> Marks { self.spaces.insert(id.to_string()); self }
    pub fn add(&mut self, o: Marks) { self.kinds |= o.kinds; self.all_messages |= o.all_messages; self.spaces.extend(o.spaces); }
    pub fn has(&self, k: Kind) -> bool { self.kinds & k.bit() != 0 }
    pub fn is_empty(&self) -> bool { self.kinds == 0 && self.spaces.is_empty() && !self.all_messages }
}

/// A cheap summary of the window's own state, compared every frame while the store is in use: what the UI changes
/// without a `Resp` (opening a space, flags, drafts, settings, the call controls).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fingerprint { pub app: u64, pub spaces: u64, pub drafts: u64, pub flags: u64, pub settings: u64, pub call: u64, pub notify: u64, pub plugins: u64 }
impl Fingerprint {
    /// What to rebuild because of the differences to `old`.
    pub fn marks_since(&self, old: &Fingerprint) -> Marks {
        let mut m = Marks::default();
        let mut on = |c: bool, k: &[Kind]| if c { m.add(Marks::of(k)) };
        on(self.app != old.app, &[Kind::App]);
        on(self.spaces != old.spaces, &[Kind::Space, Kind::Team, Kind::Person, Kind::App]);
        on(self.drafts != old.drafts, &[Kind::Space]);
        on(self.settings != old.settings, &[Kind::Me, Kind::Setting, Kind::Reminder, Kind::Plugin]);
        on(self.call != old.call, &[Kind::Call, Kind::App]);
        on(self.notify != old.notify, &[Kind::Notification]);
        on(self.plugins != old.plugins, &[Kind::Plugin]);
        if self.flags != old.flags { m.all_messages = true; }
        m
    }
}

/// Bumped by `system::save_settings`: settings the UI changed reach the store.
static SETTINGS_GEN: AtomicU64 = AtomicU64::new(0);
pub fn note_settings_saved() { SETTINGS_GEN.fetch_add(1, Ordering::Relaxed); }
pub fn settings_generation() -> u64 { SETTINGS_GEN.load(Ordering::Relaxed) }

/// The data store shared by the window (which fills it) and plugin threads (which read it through the host methods).
#[derive(Clone, Default)]
pub struct Hub(Arc<Inner>);

#[derive(Default)]
struct Inner { st: Mutex<State>, filled: Condvar, active: AtomicBool }

#[derive(Default)]
struct State {
    store: store::Store,
    /// Subscriptions per plugin key.
    subs: HashMap<String, Vec<subs::Sub>>,
    /// Plugins that use the data API (called a data method or listen to `data.changed`).
    users: HashSet<String>,
    /// Rebuilds waiting for the next sync (everything after the store was switched on).
    pending: Marks,
    fp: Option<Fingerprint>,
    directory: Vec<crate::cloud::Person>,
    cache: Vec<(String, Arc<query::Query>)>,
    /// Scopes each plugin's reads used (docs/data-api.md 6.3: Settings > Plugins shows them).
    usage: HashMap<String, BTreeSet<&'static str>>,
    /// `data.changed` payloads to deliver: (plugin key, JSON).
    events: Vec<(String, String)>,
    /// Builds and subscription runs done (the idle-frame test counts them).
    work: u64,
    /// A wake-up for subscriptions held back by the rate limit is planned.
    wake_planned: bool,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> { m.lock().unwrap_or_else(|e| e.into_inner()) }

impl Hub {
    /// Whether any plugin uses the store (one atomic load; the window checks it every frame).
    pub fn active(&self) -> bool { self.0.active.load(Ordering::Relaxed) }

    /// A plugin uses the data API: the store is switched on (built on the next frame) if it was off.
    pub fn use_by(&self, plugin: &str) {
        let mut s = lock(&self.0.st);
        if s.users.insert(plugin.to_string()) && s.users.len() == 1 {
            s.pending = Marks::all();
            s.fp = None;
            self.0.active.store(true, Ordering::Relaxed);
            drop(s);
            crate::ui::wake();
        }
    }

    /// A plugin was switched off or unloaded: its subscriptions go; with no user left the store is emptied.
    pub fn forget(&self, plugin: &str) {
        let mut s = lock(&self.0.st);
        s.subs.remove(plugin);
        s.events.retain(|e| e.0 != plugin);
        if s.users.remove(plugin) && s.users.is_empty() {
            self.0.active.store(false, Ordering::Relaxed);
            let usage = std::mem::take(&mut s.usage);
            *s = State { usage, ..Default::default() };
        }
    }

    /// Directory search results (kept for `people`, at most `DIRECTORY_MAX`, newest last).
    pub fn note_directory(&self, people: &[crate::cloud::Person]) {
        if !self.active() { return; }
        let mut s = lock(&self.0.st);
        for p in people {
            s.directory.retain(|x| x.id != p.id);
            s.directory.push(p.clone());
        }
        let n = s.directory.len();
        if n > DIRECTORY_MAX { s.directory.drain(..n - DIRECTORY_MAX); }
    }
    pub fn directory(&self) -> Vec<crate::cloud::Person> { lock(&self.0.st).directory.clone() }

    /// What this frame has to rebuild: the marks of the arrived answers, the window's changes since the last frame, what is
    /// pending. None = nothing to do (also no subscription waiting for its turn).
    pub fn plan(&self, marks: Marks, fp: Fingerprint) -> Option<Marks> {
        let mut s = lock(&self.0.st);
        let mut m = std::mem::take(&mut s.pending);
        m.add(marks);
        match s.fp { Some(old) => m.add(fp.marks_since(&old)), None => m = Marks::all() }
        s.fp = Some(fp);
        let now = Instant::now();
        let due = s.subs.values().flatten().any(|x| x.dirty && x.last_run.is_none_or(|t| now >= t + SUB_INTERVAL));
        (!m.is_empty() || due).then_some(m)
    }

    /// Rebuilds what `m` names from `src`, records the changes, and runs the subscriptions they touch.
    pub fn apply(&self, m: &Marks, src: &Source) {
        let mut guard = lock(&self.0.st);
        let s = &mut *guard;
        let st = &mut s.store;
        let mut cs = st.begin();
        if !m.is_empty() {
            s.work += 1;
            let people = (m.has(Kind::Person) || m.has(Kind::Presence) || m.has(Kind::Space) || m.has(Kind::Message) || m.has(Kind::Call)
                || m.has(Kind::CallRecord) || m.has(Kind::Me) || m.all_messages || !m.spaces.is_empty()).then(|| build::People::of(src));
            let p = people.as_ref();
            let mut put = |k: Kind, rows: Vec<(String, store::Row)>| st.replace(k, None, rows, &mut cs);
            if m.has(Kind::App) { put(Kind::App, build::app_rows(src)); }
            if let Some(p) = p {
                if m.has(Kind::Me) { put(Kind::Me, build::me_rows(src, p)); }
                if m.has(Kind::Person) { put(Kind::Person, build::person_rows(src, p)); }
                if m.has(Kind::Presence) || m.has(Kind::Person) { put(Kind::Presence, build::presence_rows(src, p)); }
                if m.has(Kind::Space) { put(Kind::Space, build::space_rows(src, p)); }
                if m.has(Kind::Call) { put(Kind::Call, build::call_rows(src, p)); }
                if m.has(Kind::CallRecord) { put(Kind::CallRecord, build::call_record_rows(src, p)); }
            }
            if m.has(Kind::Team) { put(Kind::Team, build::team_rows(src)); }
            if m.has(Kind::Meeting) { put(Kind::Meeting, build::meeting_rows(src)); }
            if m.has(Kind::Notification) { put(Kind::Notification, build::notification_rows(src)); }
            if m.has(Kind::Setting) { put(Kind::Setting, build::setting_rows(src)); }
            if m.has(Kind::Reminder) { put(Kind::Reminder, build::reminder_rows(src)); }
            if m.has(Kind::Plugin) { put(Kind::Plugin, build::plugin_rows(src)); }
            if let Some(p) = p {
                let spaces: Vec<String> = if m.all_messages || m.has(Kind::Message) {
                    // every loaded space, and the spaces whose list is gone
                    let mut v: Vec<String> = src.messages.keys().cloned().collect();
                    let known: Vec<String> = st.table(Kind::Message).items.values().filter_map(|i| i.row.get(Kind::Message.f("space")).and_then(|v| v.as_str()).map(String::from)).collect::<HashSet<_>>().into_iter().collect();
                    v.extend(known.into_iter().filter(|k| !src.messages.contains_key(k)));
                    v
                } else { m.spaces.iter().cloned().collect() };
                for sp in spaces {
                    let rows = build::message_rows(src, p, &sp);
                    st.replace(Kind::Message, Some(&sp), rows, &mut cs);
                }
            }
            st.commit(&cs);
            for x in s.subs.values_mut().flatten() { if x.touched_by(&cs) { x.dirty = true; } }
        }
        self.0.filled.notify_all();
        self.run_subs(s, &cs.rekeys);
    }

    /// Runs the dirty subscriptions whose turn it is; plans a wake-up for the others.
    fn run_subs(&self, s: &mut State, rekeys: &[(store::Id, store::Id)]) {
        let now = Instant::now();
        let params = serde_json::Map::new();
        let mut later: Option<Duration> = None;
        let State { store, subs, events, work, wake_planned, .. } = s;
        for (plugin, list) in subs.iter_mut() {
            for x in list.iter_mut() {
                // rekeys of items it holds wait with it for its turn
                let mine: Vec<_> = rekeys.iter().filter(|(from, _)| x.holds(from)).cloned().collect();
                if !mine.is_empty() { x.rekeys.extend(mine); x.dirty = true; }
                if !x.dirty { continue; }
                if let Some(t) = x.last_run.filter(|t| now < *t + SUB_INTERVAL) {
                    let wait = t + SUB_INTERVAL - now;
                    later = Some(later.map_or(wait, |l| l.min(wait)));
                    continue;
                }
                let p = if x.params.is_empty() { params.clone() } else { x.params.clone() };
                let env = query::Env { store, params: &p, now_ms: crate::i18n::now_ms() };
                let rk = std::mem::take(&mut x.rekeys);
                let ops = x.refresh(&env, &rk, &|k, n, v| api::volatile(k, n, v, true));
                x.dirty = false;
                *work += 1;
                if ops.is_empty() { continue; }
                x.last_run = Some(now);
                x.rev += 1;
                events.push((plugin.clone(), serde_json::json!({ "sub": x.id, "rev": x.rev, "ops": ops }).to_string()));
            }
        }
        if let Some(d) = later.filter(|_| !*wake_planned) {
            *wake_planned = true;
            let hub = self.clone();
            std::thread::spawn(move || { std::thread::sleep(d); lock(&hub.0.st).wake_planned = false; crate::ui::wake(); });
        }
    }

    /// The `data.changed` payloads to deliver now (after the frame's work, outside the store's lock).
    pub fn take_events(&self) -> Vec<(String, String)> { std::mem::take(&mut lock(&self.0.st).events) }

    /// Builds and subscription runs so far.
    #[cfg(test)]
    pub fn work(&self) -> u64 { lock(&self.0.st).work }

    /// The scopes a plugin's data reads used, for Settings > Plugins ("reads message content").
    #[allow(dead_code)] // shown by the Plugins page (follow-up: src/ui is outside this change)
    pub fn scopes_used(&self, plugin: &str) -> Vec<&'static str> { lock(&self.0.st).usage.get(plugin).map(|s| s.iter().copied().collect()).unwrap_or_default() }

    /// Waits (plugin threads only) until the store was filled once, at most `max`.
    fn wait_filled<'a>(&'a self, mut s: MutexGuard<'a, State>, max: Duration) -> MutexGuard<'a, State> {
        let until = Instant::now() + max;
        while !s.store.ready {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() { break; }
            s = self.0.filled.wait_timeout(s, left).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
        }
        s
    }
}
