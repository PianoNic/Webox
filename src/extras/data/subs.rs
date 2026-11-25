//! Field selection, query runs over all parts, and subscriptions: a subscription keeps the ids of its result and the values
//! of its selected fields, and after a store change that touches what it reads it runs again and describes the difference
//! as `insert` / `remove` / `move` / `update` / `rekey` operations (docs/data-api.md section 4).
use super::model::Kind;
use super::query::{self, Env, Path, Query, Step};
use super::store::{ChangeSet, Id};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

/// At most this many operations in one `data.changed` event; more are sent as `{"op": "reset"}`.
pub const OPS_MAX: usize = 1000;

/// A selected field: the name as asked (`presence.state`) and its compiled path.
#[derive(Clone, Debug)]
pub struct Sel { pub name: String, pub path: Path, pub volatile: bool }

/// The fields of each part: those asked for (a name unknown to a part is absent there; unknown to all parts is an error), or
/// every stored field of the kind.
pub fn selection(q: &Query, fields: Option<&[String]>) -> Result<Vec<Vec<Sel>>, query::Error> {
    let mut out = vec![];
    let mut known = HashSet::new();
    for p in &q.parts {
        let k = p.kind;
        let sels = match fields {
            None => k.def().fields.iter().enumerate().filter(|(_, f)| !f.reserved && !f.volatile)
                .map(|(i, f)| Sel { name: f.name.into(), path: Path { steps: vec![Step::Field { kind: k, idx: i, rel: f.rel, list: f.ty.starts_with('[') }] }, volatile: false }).collect(),
            Some(names) => {
                let mut v = vec![];
                for n in names {
                    if n == "id" { known.insert(n.clone()); continue; } // always there
                    match query::compile_field(k, n) {
                        Ok((path, volatile)) => { known.insert(n.clone()); v.push(Sel { name: n.clone(), path, volatile }); }
                        Err(_) if q.parts.len() > 1 => {}
                        Err(e) => return Err(query::Error { col: 0, msg: format!("field \"{n}\": {}", e.msg) }),
                    }
                }
                v
            }
        };
        out.push(sels);
    }
    if let Some(names) = fields {
        if let Some(n) = names.iter().find(|n| !known.contains(*n)) {
            return Err(query::Error { col: 0, msg: format!("field \"{n}\" is in none of the query's collections") });
        }
    }
    Ok(out)
}

/// The fields `sel` reads per kind (for change matching).
pub fn reads_of(sel: &[Vec<Sel>]) -> [u64; 14] {
    let mut r = [0u64; 14];
    for s in sel.iter().flatten() {
        for st in &s.path.steps { if let Step::Field { kind, idx, rel, .. } = st { r[kind.idx()] |= 1u64 << idx; if let Some(t) = rel { r[t.idx()] |= 1; } } }
    }
    r
}

/// Puts `v` at the dotted `name` of `o` (`presence.state` -> `{"presence": {"state": v}}`).
fn put(o: &mut Map<String, Value>, name: &str, v: Value) {
    match name.split_once('.') {
        None => { o.insert(name.to_string(), v); }
        Some((a, rest)) => {
            let e = o.entry(a.to_string()).or_insert_with(|| Value::Object(Map::new()));
            if !e.is_object() { *e = Value::Object(Map::new()); }
            if let Value::Object(m) = e { put(m, rest, v); }
        }
    }
}

/// Reads a volatile field (device lists, statistics) for an answer; None = absent.
pub type Volatile<'a> = &'a dyn Fn(Kind, &str, Value) -> Value;

/// One item as JSON: its id, the selected fields (absent ones left out), and `entity` when the query has several parts.
pub fn project(env: &Env, kind: Kind, id: &str, sel: &[Sel], multi: bool, vol: Volatile) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(id));
    if multi { o.insert("entity".into(), json!(kind.name())); }
    for s in sel {
        let stored = if s.volatile { Value::Null } else { env.resolve(kind, id, &s.path).into_owned() };
        let v = if s.volatile || (kind == Kind::Me && s.name.starts_with("audio")) { vol(kind, &s.name, stored) } else { stored };
        if !v.is_null() { put(&mut o, &s.name, v); }
    }
    Value::Object(o)
}

/// Every part's result, concatenated: (part, id) and the total before each part's limit, plus group boundaries.
pub fn run_all(env: &Env, q: &Query) -> (Vec<(usize, Id)>, usize, Vec<Value>) {
    let mut items = vec![];
    let (mut total, mut groups) = (0, vec![]);
    for (i, p) in q.parts.iter().enumerate() {
        let (ids, t, g) = env.run(p);
        let start = items.len();
        let mut at = start;
        for (k, n) in g { groups.push(json!({ "key": k, "count": n, "start": at })); at += n; }
        total += t;
        items.extend(ids.into_iter().map(|id| (i, id)));
    }
    (items, total, groups)
}

/// How a list of ids turns into another: the operations in the order a plugin applies them.
#[derive(Clone, Debug, PartialEq)]
pub enum ListOp { Remove(Id), Insert(usize, Id), Move(Id, usize) }

/// The operations that turn `old` into `new` (both without duplicates): removals first, then insertions and moves at their
/// final index, as few moves as a longest increasing subsequence leaves (items that kept their relative order stay put).
pub fn diff_lists(old: &[Id], new: &[Id]) -> Vec<ListOp> {
    let new_set: HashSet<&Id> = new.iter().collect();
    let mut ops: Vec<ListOp> = old.iter().filter(|id| !new_set.contains(id)).map(|id| ListOp::Remove(id.clone())).collect();
    let mut cur: Vec<Id> = old.iter().filter(|id| new_set.contains(id)).cloned().collect();
    let old_pos: HashMap<&Id, usize> = cur.iter().enumerate().map(|(i, id)| (id, i)).collect();
    // kept items in new order -> their old positions; the longest increasing run stays where it is
    let kept: Vec<(usize, usize)> = new.iter().enumerate().filter_map(|(i, id)| old_pos.get(id).map(|&p| (i, p))).collect();
    let stable: HashSet<usize> = lis(&kept.iter().map(|k| k.1).collect::<Vec<_>>()).into_iter().map(|j| kept[j].0).collect();
    // back to front: each insertion or move goes right before its successor in `new`
    for i in (0..new.len()).rev() {
        let id = &new[i];
        if stable.contains(&i) { continue; }
        let before = new.get(i + 1).and_then(|n| cur.iter().position(|x| x == n)).unwrap_or(cur.len());
        match cur.iter().position(|x| x == id) {
            Some(from) => {
                cur.remove(from);
                let to = if from < before { before - 1 } else { before };
                cur.insert(to, id.clone());
                ops.push(ListOp::Move(id.clone(), to));
            }
            None => { cur.insert(before, id.clone()); ops.push(ListOp::Insert(before, id.clone())); }
        }
    }
    ops
}

/// Indices of a longest strictly increasing subsequence of `a`.
fn lis(a: &[usize]) -> Vec<usize> {
    let (mut tails, mut prev): (Vec<usize>, Vec<Option<usize>>) = (vec![], vec![None; a.len()]);
    for i in 0..a.len() {
        let k = tails.partition_point(|&t| a[t] < a[i]);
        if k > 0 { prev[i] = Some(tails[k - 1]); }
        if k == tails.len() { tails.push(i) } else { tails[k] = i }
    }
    let mut out = vec![];
    let mut cur = tails.last().copied();
    while let Some(i) = cur { out.push(i); cur = prev[i]; }
    out.reverse();
    out
}

/// One subscription of one plugin.
pub struct Sub {
    pub id: String,
    pub query: Query,
    pub sel: Vec<Vec<Sel>>,
    pub params: Map<String, Value>,
    /// Most items it follows.
    pub window: usize,
    /// What it reads per kind (query and selected fields).
    pub reads: [u64; 14],
    items: Vec<(usize, Id)>,
    values: HashMap<Id, Vec<Value>>,
    pub rev: u64,
    /// A change touched it since the last run.
    pub dirty: bool,
    /// Rekeys seen while it waited for its turn.
    pub rekeys: Vec<(Id, Id)>,
    pub last_run: Option<std::time::Instant>,
}

impl Sub {
    pub fn new(id: String, query: Query, sel: Vec<Vec<Sel>>, params: Map<String, Value>, window: usize) -> Sub {
        let mut reads = reads_of(&sel);
        for (r, q) in reads.iter_mut().zip(query.reads) { *r |= q; }
        Sub { id, query, sel, params, window, reads, items: vec![], values: HashMap::new(), rev: 0, dirty: false, rekeys: vec![], last_run: None }
    }
    /// Whether it holds the item `id`.
    pub fn holds(&self, id: &str) -> bool { self.values.contains_key(id) }
    /// Whether a change set can change this subscription's result.
    pub fn touched_by(&self, cs: &ChangeSet) -> bool {
        Kind::ALL.iter().any(|k| self.reads[k.idx()] != 0 && cs.touches(*k, self.reads[k.idx()]))
            || cs.rekeys.iter().any(|(from, _)| self.values.contains_key(from))
    }
    fn stored(&self, env: &Env, part: usize, id: &str) -> Vec<Value> {
        let kind = self.query.parts[part].kind;
        self.sel[part].iter().filter(|s| !s.volatile).map(|s| env.resolve(kind, id, &s.path).into_owned()).collect()
    }
    /// Runs the query: the first page (what `data.subscribe` answers) and the state the next diff starts from.
    pub fn first(&mut self, env: &Env, vol: Volatile) -> (Vec<Value>, usize) {
        let (mut items, total, _) = run_all(env, &self.query);
        items.truncate(self.window);
        let multi = self.query.parts.len() > 1;
        let page = items.iter().map(|(p, id)| project(env, self.query.parts[*p].kind, id, &self.sel[*p], multi, vol)).collect();
        self.values = items.iter().map(|(p, id)| (id.clone(), self.stored(env, *p, id))).collect();
        self.items = items;
        (page, total)
    }
    /// Runs again after a change: the operations (empty = nothing visible changed).
    pub fn refresh(&mut self, env: &Env, rekeys: &[(Id, Id)], vol: Volatile) -> Vec<Value> {
        let mut ops = vec![];
        for (from, to) in rekeys {
            if let Some(i) = self.items.iter().position(|x| &x.1 == from) {
                self.items[i].1 = to.clone();
                if let Some(v) = self.values.remove(from) { self.values.insert(to.clone(), v); }
                ops.push(json!({ "op": "rekey", "from": &**from, "to": &**to }));
            }
        }
        let (mut items, _, _) = run_all(env, &self.query);
        items.truncate(self.window);
        let old_ids: Vec<Id> = self.items.iter().map(|x| x.1.clone()).collect();
        let new_ids: Vec<Id> = items.iter().map(|x| x.1.clone()).collect();
        let part_of: HashMap<&Id, usize> = items.iter().map(|(p, id)| (id, *p)).collect();
        let multi = self.query.parts.len() > 1;
        let mut inserted = HashSet::new();
        for op in diff_lists(&old_ids, &new_ids) {
            ops.push(match op {
                ListOp::Remove(id) => { self.values.remove(&id); json!({ "op": "remove", "id": &*id }) }
                ListOp::Move(id, to) => json!({ "op": "move", "id": &*id, "to": to }),
                ListOp::Insert(at, id) => {
                    let p = part_of[&id];
                    inserted.insert(id.clone());
                    self.values.insert(id.clone(), self.stored(env, p, &id));
                    json!({ "op": "insert", "at": at, "item": project(env, self.query.parts[p].kind, &id, &self.sel[p], multi, vol) })
                }
            });
        }
        // updates: only the selected fields that changed
        for (p, id) in &items {
            if inserted.contains(id) { continue; }
            let now = self.stored(env, *p, id);
            let Some(was) = self.values.get_mut(id) else { continue };
            if *was == now { continue; }
            let mut set = Map::new();
            for ((s, a), b) in self.sel[*p].iter().filter(|s| !s.volatile).zip(was.iter()).zip(&now) { if a != b { set.insert(s.name.clone(), b.clone()); } }
            *was = now;
            ops.push(json!({ "op": "update", "id": &**id, "set": set }));
        }
        self.items = items;
        if ops.len() > OPS_MAX { ops = vec![json!({ "op": "reset" })]; }
        ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(s: &str) -> Vec<Id> { s.chars().map(|c| Id::from(c.to_string())).collect() }
    fn apply(old: &[Id], ops: &[ListOp]) -> Vec<Id> {
        let mut l = old.to_vec();
        for op in ops {
            match op {
                ListOp::Remove(id) => l.retain(|x| x != id),
                ListOp::Insert(at, id) => l.insert(*at, id.clone()),
                ListOp::Move(id, to) => { l.retain(|x| x != id); l.insert(*to, id.clone()); }
            }
        }
        l
    }

    #[test]
    fn list_diffs_apply_and_stay_small() {
        for (a, b, moves) in [("abc", "abc", 0), ("abc", "cab", 1), ("abc", "bca", 1), ("abcd", "dcba", 3), ("", "abc", 0), ("abc", "", 0),
            ("abcde", "aXcYe", 0), ("abcdef", "fabcde", 1), ("abcdef", "bcdefa", 1)] {
            let (o, n) = (ids(a), ids(b));
            let ops = diff_lists(&o, &n);
            assert_eq!(apply(&o, &ops), n, "{a} -> {b}: {ops:?}");
            assert_eq!(ops.iter().filter(|x| matches!(x, ListOp::Move(..))).count(), moves, "{a} -> {b}: {ops:?}");
        }
        // random permutations with insertions and removals
        let mut seed = 7u64;
        let mut rnd = |n: u64| { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (seed >> 33) % n };
        for _ in 0..300 {
            let pool: Vec<Id> = (0..12).map(|i| Id::from(format!("x{i}"))).collect();
            let pick = |r: &mut dyn FnMut(u64) -> u64| { let mut v: Vec<Id> = pool.iter().filter(|_| r(3) > 0).cloned().collect(); for i in (1..v.len()).rev() { v.swap(i, r(i as u64 + 1) as usize); } v };
            let (o, n) = (pick(&mut rnd), pick(&mut rnd));
            assert_eq!(apply(&o, &diff_lists(&o, &n)), n);
        }
    }
}
