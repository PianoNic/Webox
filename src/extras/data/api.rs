//! The host methods of the data API (docs/data-api.md section 4, docs/plugins.md "Data API"): `data.model`, `data.get`,
//! `data.query`, `data.subscribe`, `data.unsubscribe`. Plugins have full access (native code): no scope refuses them, but
//! every read is recorded per plugin by scope (`Hub::scopes_used`), so the Plugins page can say what a plugin reads.
use super::model::{self, Kind};
use super::query::{self, LIMIT_MAX};
use super::subs::{self, Sub};
use super::{lock, Hub, State, SUBS_MAX};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Items one `data.query` answer holds when the call names no `limit`.
pub const PAGE_DEFAULT: usize = 100;
/// How long a call from a plugin thread waits for the store's first fill.
const FIRST_FILL_WAIT: Duration = Duration::from_secs(2);
/// Longest subscription id.
const SUB_ID_MAX: usize = 64;

fn error(msg: impl std::fmt::Display) -> Value { json!({ "error": msg.to_string() }) }

/// `data.model`: the generated model description.
pub fn model() -> Value { static M: OnceLock<Value> = OnceLock::new(); M.get_or_init(model::describe).clone() }

/// The `fields` argument: None = every stored field.
fn fields(a: &Value) -> Result<Option<Vec<String>>, Value> {
    match a.get("fields") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(l)) => l.iter().map(|x| x.as_str().map(String::from).ok_or_else(|| error("\"fields\" must be a list of field names"))).collect::<Result<_, _>>().map(Some),
        _ => Err(error("\"fields\" must be a list of field names")),
    }
}
fn params(a: &Value) -> Result<Map<String, Value>, Value> {
    match a.get("params") { None | Some(Value::Null) => Ok(Map::new()), Some(Value::Object(m)) => Ok(m.clone()), _ => Err(error("\"params\" must be an object")) }
}
fn limit(a: &Value, def: usize) -> Result<usize, Value> {
    match a.get("limit") {
        None | Some(Value::Null) => Ok(def),
        Some(v) => match v.as_u64() { Some(n) if (n as usize) <= LIMIT_MAX => Ok(n as usize), _ => Err(error(format!("\"limit\" must be a whole number up to {LIMIT_MAX}"))) },
    }
}

/// Records the scopes of the fields a plugin's read touched.
fn note_usage(s: &mut State, plugin: &str, reads: &[u64; 14]) {
    let u = s.usage.entry(plugin.to_string()).or_default();
    for k in Kind::ALL {
        let mask = reads[k.idx()];
        if mask == 0 { continue; }
        let def = k.def();
        for (i, f) in def.fields.iter().enumerate() { if mask & (1u64 << i) != 0 { u.insert(if f.scope.is_empty() { def.scope } else { f.scope }); } }
    }
}

/// The compiled query (compiled once per text and kept).
fn compiled(s: &mut State, q: &Value) -> Result<Arc<query::Query>, Value> {
    let key = q.to_string();
    if let Some(c) = s.cache.iter().find(|c| c.0 == key) { return Ok(c.1.clone()); }
    let c = Arc::new(query::compile_value(q).map_err(|e| error(format!("query: {e}")))?);
    if s.cache.len() >= super::CACHE_MAX { s.cache.remove(0); }
    s.cache.push((key, c.clone()));
    Ok(c)
}

impl Hub {
    /// The store's lock, after waiting for its first fill on a plugin thread (never on the UI thread, which fills it).
    fn ready_state(&self, plugin: &str, on_ui: bool) -> std::sync::MutexGuard<'_, State> {
        self.use_by(plugin);
        let s = lock(&self.0.st);
        if on_ui || s.store.ready { s } else { self.wait_filled(s, FIRST_FILL_WAIT) }
    }

    /// `data.get {"entity", "id", "fields"?}` -> `{"item": {...}, "rev"}`.
    pub fn get(&self, plugin: &str, a: &Value, on_ui: bool) -> Value {
        let Some(name) = a.get("entity").and_then(Value::as_str) else { return error("\"entity\" missing (person, space, message, ...)") };
        let Some(kind) = Kind::by_name(name).or_else(|| model::collection(name).map(|c| c.kind)) else {
            return error(format!("unknown entity \"{name}\"; entities: {}", model::ENTITIES.iter().map(|e| e.name).collect::<Vec<_>>().join(", ")));
        };
        let id = match a.get("id").and_then(Value::as_str) { Some(id) => id, None if matches!(kind, Kind::Me | Kind::App) => kind.name(), None => return error("\"id\" missing") };
        let f = match fields(a) { Ok(f) => f, Err(e) => return e };
        let q = query::Query { parts: vec![query::Part { kind, collection: "", arg: None, filter: None, implicit: None, search: None, group: None, sort: vec![], limit: 1, offset: 0 }], reads: [0; 14] };
        let sel = match subs::selection(&q, f.as_deref()) { Ok(s) => s, Err(e) => return error(e) };
        let mut s = self.ready_state(plugin, on_ui);
        note_usage(&mut s, plugin, &subs::reads_of(&sel));
        let Some(item) = s.store.get(kind, id) else {
            let mut e = error(format!("no {} with id \"{id}\"", kind.name()));
            if !s.store.ready { e["ready"] = json!(false); }
            return e;
        };
        let rev = item.rev;
        let params = Map::new();
        let env = query::Env { store: &s.store, params: &params, now_ms: crate::i18n::now_ms() };
        json!({ "item": subs::project(&env, kind, id, &sel[0], false, &|k, n, v| volatile(k, n, v, on_ui)), "rev": rev })
    }

    /// `data.query {"query", "params"?, "fields"?, "limit"?, "cursor"?}` -> `{"items", "total", "rev", "cursor"?, "groups"?}`.
    pub fn query(&self, plugin: &str, a: &Value, on_ui: bool) -> Value {
        let Some(qv) = a.get("query") else { return error("\"query\" missing") };
        let (f, p, page) = match (fields(a), params(a), limit(a, PAGE_DEFAULT)) { (Ok(f), Ok(p), Ok(l)) => (f, p, l), (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => return e };
        let from = match a.get("cursor") {
            None | Some(Value::Null) => 0,
            Some(c) => match c.as_str().and_then(|c| c.parse::<usize>().ok()) { Some(n) => n, None => return error("\"cursor\" is not one this host gave") },
        };
        let mut s = self.ready_state(plugin, on_ui);
        let q = match compiled(&mut s, qv) { Ok(q) => q, Err(e) => return e };
        let sel = match subs::selection(&q, f.as_deref()) { Ok(x) => x, Err(e) => return error(e) };
        let mut reads = subs::reads_of(&sel);
        for (r, x) in reads.iter_mut().zip(q.reads) { *r |= x; }
        note_usage(&mut s, plugin, &reads);
        let env = query::Env { store: &s.store, params: &p, now_ms: crate::i18n::now_ms() };
        let (items, total, groups) = subs::run_all(&env, &q);
        let multi = q.parts.len() > 1;
        let vol = |k, n: &str, v| volatile(k, n, v, on_ui);
        let out: Vec<Value> = items.iter().skip(from).take(page).map(|(pi, id)| subs::project(&env, q.parts[*pi].kind, id, &sel[*pi], multi, &vol)).collect();
        let mut r = json!({ "items": out, "total": total, "rev": s.store.rev });
        if from + page < items.len() { r["cursor"] = json!((from + page).to_string()); }
        if !groups.is_empty() { r["groups"] = json!(groups); }
        if !s.store.ready { r["ready"] = json!(false); }
        r
    }

    /// `data.subscribe {"id", "query", "params"?, "fields"?, "limit"?}` -> the first page and `"sub"`; later changes arrive as
    /// `data.changed` events (the plugin must list that event).
    pub fn subscribe(&self, plugin: &str, a: &Value, on_ui: bool, listens: bool) -> Value {
        let Some(id) = a.get("id").and_then(Value::as_str).filter(|i| !i.is_empty() && i.len() <= SUB_ID_MAX) else { return error(format!("\"id\" missing (a name of up to {SUB_ID_MAX} characters)")) };
        if !listens { return error("list \"data.changed\" in the plugin's EVENTS to receive the changes"); }
        let Some(qv) = a.get("query") else { return error("\"query\" missing") };
        let (f, p, window) = match (fields(a), params(a), limit(a, query::LIMIT_DEFAULT)) { (Ok(f), Ok(p), Ok(l)) => (f, p, l), (Err(e), ..) | (_, Err(e), _) | (.., Err(e)) => return e };
        let mut s = self.ready_state(plugin, on_ui);
        let q = match compiled(&mut s, qv) { Ok(q) => q, Err(e) => return e };
        let sel = match subs::selection(&q, f.as_deref()) { Ok(x) => x, Err(e) => return error(e) };
        let list = s.subs.get(plugin);
        if list.is_some_and(|l| l.len() >= SUBS_MAX && !l.iter().any(|x| x.id == id)) { return error(format!("at most {SUBS_MAX} subscriptions per plugin")); }
        let mut sub = Sub::new(id.to_string(), (*q).clone(), sel, p, window);
        note_usage(&mut s, plugin, &sub.reads);
        let State { store, subs: all, .. } = &mut *s;
        let sp = sub.params.clone();
        let env = query::Env { store, params: &sp, now_ms: crate::i18n::now_ms() };
        let (items, total) = sub.first(&env, &|k, n, v| volatile(k, n, v, on_ui));
        let rev = store.rev;
        let ready = store.ready;
        let l = all.entry(plugin.to_string()).or_default();
        l.retain(|x| x.id != id);
        l.push(sub);
        let mut r = json!({ "sub": id, "items": items, "total": total, "rev": rev });
        if !ready { r["ready"] = json!(false); }
        r
    }

    /// `data.unsubscribe {"id"}` -> `{}`.
    pub fn unsubscribe(&self, plugin: &str, a: &Value) -> Value {
        let Some(id) = a.get("id").and_then(Value::as_str) else { return error("\"id\" missing") };
        let mut s = lock(&self.0.st);
        let Some(l) = s.subs.get_mut(plugin) else { return error(format!("no subscription \"{id}\"")) };
        let n = l.len();
        l.retain(|x| x.id != id);
        if l.len() == n { return error(format!("no subscription \"{id}\"")); }
        let plugin_key = plugin.to_string();
        s.events.retain(|e| !(e.0 == plugin_key && e.1.contains(&format!("\"sub\":{}", json!(id)))));
        json!({})
    }
}

/// Devices, read off the UI thread and kept for a while (listing them blocks).
struct Devices { at: Instant, inputs: Value, outputs: Value, cameras: Value }
static DEVICES: Mutex<Option<Devices>> = Mutex::new(None);
const DEVICES_TTL: Duration = Duration::from_secs(10);

fn devices() -> (Value, Value, Value) {
    let mut d = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
    if d.as_ref().is_none_or(|x| x.at.elapsed() > DEVICES_TTL) {
        let (ins, outs) = crate::media::audio::devices();
        let dev = |l: Vec<crate::media::audio::Dev>, kind: &str| json!(l.into_iter().map(|x| json!({ "id": x.name, "name": x.name, "kind": kind, "default": x.default })).collect::<Vec<_>>());
        let cams = json!(crate::media::camera::list().into_iter().enumerate().map(|(i, c)| json!({ "id": c.id, "name": c.name, "kind": "camera", "default": i == 0 })).collect::<Vec<_>>());
        *d = Some(Devices { at: Instant::now(), inputs: dev(ins, "audio_in"), outputs: dev(outs, "audio_out"), cameras: cams });
    }
    let x = d.as_ref().map(|x| (x.inputs.clone(), x.outputs.clone(), x.cameras.clone()));
    x.unwrap_or_default()
}

fn flow(f: &crate::cloud::media_stats::Flow) -> Value {
    let mut v = json!({ "bytes": f.bytes, "packets": f.packets, "expected": f.expected, "lost": f.lost, "frames": f.frames });
    if let Some(x) = f.fraction_lost { v["fraction_lost"] = json!(x); }
    if let Some(x) = f.jitter_ms { v["jitter_ms"] = json!(x); }
    if let Some((w, h)) = f.size { v["width"] = json!(w); v["height"] = json!(h); }
    v
}

/// Fills a volatile field (`stored` is the stored value, Null for fields never stored). Device lists are not read on the UI
/// thread (it would block the window); there they stay absent.
pub fn volatile(kind: Kind, name: &str, stored: Value, on_ui: bool) -> Value {
    let (head, rest) = name.split_once('.').map_or((name, None), |(a, b)| (a, Some(b)));
    let pick = |v: Value| match rest { Some(k) => v.get(k).cloned().unwrap_or(Value::Null), None => v };
    match (kind, head) {
        (Kind::Me, "cameras") if !on_ui => devices().2,
        (Kind::Me, "audio") if !on_ui && (rest.is_none() || matches!(rest, Some("inputs" | "outputs"))) => {
            let (ins, outs, _) = devices();
            match rest {
                Some("inputs") => ins,
                Some(_) => outs,
                None => { let mut v = if stored.is_object() { stored } else { json!({}) }; v["inputs"] = ins; v["outputs"] = outs; v }
            }
        }
        (Kind::Me, "features") => pick(Value::Object(crate::cloud::features::all().into_iter().collect())),
        (Kind::Call, "stats") => crate::cloud::media_stats::latest().map_or(Value::Null, |s| {
            let mut v = json!({ "audio": { "tx": flow(&s.tx[0]), "rx": flow(&s.rx[0]) }, "video": { "tx": flow(&s.tx[1]), "rx": flow(&s.rx[1]) },
                "share": { "tx": flow(&s.tx[2]), "rx": flow(&s.rx[2]) } });
            if let Some(r) = s.rtt_ms { v["rtt_ms"] = json!(r); }
            pick(v)
        }),
        _ => stored,
    }
}
