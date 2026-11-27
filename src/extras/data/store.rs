//! The data store: per entity kind a table of rows (one `Value` per model field, in the model's field order, `Null` =
//! absent), each with the store revision of its last change. `replace` takes the freshly built rows of one kind (or of one
//! group, e.g. one space's messages), compares them field by field with what is kept and records what changed: inserted,
//! removed and updated entities with a mask of the changed fields, and rekeys (a person first known by e-mail gets its id).
//! Nothing is compared or copied for a kind that was not rebuilt.
use super::model::Kind;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub type Id = Arc<str>;
pub type Row = Vec<Value>;

pub struct Item {
    pub row: Row,
    /// The store revision of its last change.
    pub rev: u64,
    /// Its position in the source (the space list's order, messages oldest first): the order of an unsorted query.
    pub order: u32,
    /// The group it was replaced with (a message's space), if any.
    group: Option<Id>,
}

#[derive(Default)]
pub struct Table {
    pub items: HashMap<Id, Item>,
    /// Ids per group (messages per space).
    groups: HashMap<Id, HashSet<Id>>,
}

/// What happened to one entity in a sync.
#[derive(Clone, Debug, PartialEq)]
pub enum Op { Insert, Remove, Update(u64) }

#[derive(Clone, Debug, PartialEq)]
pub struct Change { pub kind: Kind, pub id: Id, pub op: Op }

/// Per kind: the union of changed field masks and whether entities came or went.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KindChange { pub fields: u64, pub structural: bool }

/// Everything one sync changed.
#[derive(Clone, Debug, Default)]
pub struct ChangeSet {
    pub kinds: [KindChange; 14],
    pub changes: Vec<Change>,
    /// (old id, new id) of people first known by e-mail.
    pub rekeys: Vec<(Id, Id)>,
}
impl ChangeSet {
    pub fn is_empty(&self) -> bool { self.changes.is_empty() && self.rekeys.is_empty() }
    /// Whether anything of `kind` changed that a reader of `mask` (fields read; structural changes always count) sees.
    pub fn touches(&self, kind: Kind, mask: u64) -> bool {
        let k = self.kinds[kind.idx()];
        k.structural || k.fields & mask != 0
    }
    fn note(&mut self, kind: Kind, id: Id, op: Op) {
        let k = &mut self.kinds[kind.idx()];
        match op { Op::Update(m) => k.fields |= m, _ => k.structural = true }
        self.changes.push(Change { kind, id, op });
    }
}

#[derive(Default)]
pub struct Store {
    pub tables: [Table; 14],
    /// Increases once per sync that changed something.
    pub rev: u64,
    /// At least one full build happened (the store has data).
    pub ready: bool,
}

/// The e-mail part of a provisional person id.
pub fn email_key(id: &str) -> Option<&str> { id.strip_prefix("email:") }

impl Store {
    pub fn table(&self, k: Kind) -> &Table { &self.tables[k.idx()] }
    pub fn get(&self, k: Kind, id: &str) -> Option<&Item> { self.tables[k.idx()].items.get(id) }
    /// The value of field `f` of entity `id`.
    pub fn value(&self, k: Kind, id: &str, f: usize) -> Option<&Value> { self.get(k, id).and_then(|i| i.row.get(f)) }
    /// The ids of a group (one space's messages).
    pub fn group(&self, k: Kind, g: &str) -> impl Iterator<Item = &Id> { self.tables[k.idx()].groups.get(g).into_iter().flatten() }

    /// Starts a sync: changes are collected into `cs` and stamped with the next revision.
    pub fn begin(&self) -> ChangeSet { ChangeSet::default() }

    /// Replaces all entities of `kind` (or, with `group`, those of that group) with `rows` (id, row, in source order).
    pub fn replace(&mut self, kind: Kind, group: Option<&str>, rows: Vec<(String, Row)>, cs: &mut ChangeSet) {
        let rev = self.rev + 1;
        let t = &mut self.tables[kind.idx()];
        let old: HashSet<Id> = match group {
            Some(g) => t.groups.get(g).cloned().unwrap_or_default(),
            None => t.items.keys().cloned().collect(),
        };
        let g: Option<Id> = group.map(Id::from);
        let mut seen: HashSet<Id> = HashSet::with_capacity(rows.len());
        let mut inserted = vec![];
        for (order, (id, row)) in rows.into_iter().enumerate() {
            let order = order as u32;
            match t.items.get_key_value(id.as_str()).map(|(k, _)| k.clone()) {
                Some(id) => {
                    let Some(it) = t.items.get_mut(&id) else { continue };
                    let mask = diff_mask(&it.row, &row);
                    if mask != 0 { it.row = row; it.rev = rev; cs.note(kind, id.clone(), Op::Update(mask)); }
                    it.order = order;
                    if it.group != g { let from = it.group.take(); it.group = g.clone(); move_group(&mut t.groups, from, g.clone(), &id); }
                    seen.insert(id);
                }
                None => {
                    let id: Id = Id::from(id);
                    if let Some(g) = &g { t.groups.entry(g.clone()).or_default().insert(id.clone()); }
                    t.items.insert(id.clone(), Item { row, rev, order, group: g.clone() });
                    seen.insert(id.clone());
                    inserted.push(id);
                }
            }
        }
        let removed: Vec<Id> = old.into_iter().filter(|id| !seen.contains(id)).collect();
        // A person known by e-mail that now has its real id: a rekey (the removal and the insertion are recorded too).
        if kind == Kind::Person {
            let email_f = Kind::Person.f("email");
            for old_id in &removed {
                let Some(mail) = email_key(old_id) else { continue };
                let to = inserted.iter().find(|n| t.items[*n].row.get(email_f).and_then(Value::as_str).is_some_and(|e| e.eq_ignore_ascii_case(mail)));
                if let Some(to) = to { cs.rekeys.push((old_id.clone(), to.clone())); }
            }
        }
        for id in removed {
            if let Some(it) = t.items.remove(&id) { move_group(&mut t.groups, it.group, None, &id); }
            cs.note(kind, id, Op::Remove);
        }
        for id in inserted { cs.note(kind, id, Op::Insert); }
    }

    /// Ends a sync: with changes, the revision goes up.
    pub fn commit(&mut self, cs: &ChangeSet) { if !cs.is_empty() { self.rev += 1; } self.ready = true; }
}

fn move_group(groups: &mut HashMap<Id, HashSet<Id>>, from: Option<Id>, to: Option<Id>, id: &Id) {
    if let Some(f) = from { if let Some(s) = groups.get_mut(&f) { s.remove(id); if s.is_empty() { groups.remove(&f); } } }
    if let Some(t) = to { groups.entry(t).or_default().insert(id.clone()); }
}
/// Bit i set when field i differs.
pub fn diff_mask(a: &Row, b: &Row) -> u64 {
    let n = a.len().max(b.len());
    (0..n).filter(|&i| a.get(i) != b.get(i)).fold(0u64, |m, i| m | (1u64 << i.min(63)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn person(id: &str, name: &str, email: &str) -> (String, Row) {
        let mut r = vec![Value::Null; Kind::Person.def().fields.len()];
        r[0] = json!(id);
        r[Kind::Person.f("name")] = json!(name);
        r[Kind::Person.f("email")] = json!(email);
        (id.to_string(), r)
    }

    #[test]
    fn replace_records_inserts_updates_removes_with_field_masks() {
        let mut s = Store::default();
        let mut cs = s.begin();
        s.replace(Kind::Person, None, vec![person("a", "Anna", "a@x"), person("b", "Ben", "b@x")], &mut cs);
        s.commit(&cs);
        assert_eq!(s.rev, 1);
        assert_eq!(cs.changes.iter().filter(|c| c.op == Op::Insert).count(), 2);

        // the same rows again: nothing changed, the revision stays
        let mut cs = s.begin();
        s.replace(Kind::Person, None, vec![person("a", "Anna", "a@x"), person("b", "Ben", "b@x")], &mut cs);
        s.commit(&cs);
        assert!(cs.is_empty());
        assert_eq!(s.rev, 1);

        // Anna renamed, Ben gone, Cleo new
        let mut cs = s.begin();
        s.replace(Kind::Person, None, vec![person("a", "Anna B", "a@x"), person("c", "Cleo", "c@x")], &mut cs);
        s.commit(&cs);
        assert_eq!(s.rev, 2);
        assert!(cs.changes.contains(&Change { kind: Kind::Person, id: "a".into(), op: Op::Update(1 << Kind::Person.f("name")) }));
        assert!(cs.changes.contains(&Change { kind: Kind::Person, id: "b".into(), op: Op::Remove }));
        assert!(cs.changes.contains(&Change { kind: Kind::Person, id: "c".into(), op: Op::Insert }));
        assert!(cs.touches(Kind::Person, 1 << Kind::Person.f("name")));
        assert!(!cs.touches(Kind::Space, u64::MAX));
        assert_eq!(s.get(Kind::Person, "a").unwrap().rev, 2);
        assert_eq!(s.get(Kind::Person, "c").unwrap().rev, 2);
    }

    #[test]
    fn groups_replace_only_their_own_items() {
        let mut s = Store::default();
        let msg = |id: &str| { let mut r = vec![Value::Null; Kind::Message.def().fields.len()]; r[0] = json!(id); (id.to_string(), r) };
        let mut cs = s.begin();
        s.replace(Kind::Message, Some("s1"), vec![msg("m1"), msg("m2")], &mut cs);
        s.replace(Kind::Message, Some("s2"), vec![msg("m3")], &mut cs);
        s.commit(&cs);
        let mut cs = s.begin();
        s.replace(Kind::Message, Some("s1"), vec![msg("m2"), msg("m4")], &mut cs);
        s.commit(&cs);
        assert!(s.get(Kind::Message, "m3").is_some(), "another space's messages stay");
        assert!(s.get(Kind::Message, "m1").is_none());
        let mut g: Vec<&str> = s.group(Kind::Message, "s1").map(|x| &**x).collect();
        g.sort();
        assert_eq!(g, ["m2", "m4"]);
        assert_eq!(s.get(Kind::Message, "m2").unwrap().order, 0, "source order follows the new list");
    }

    #[test]
    fn a_person_known_by_email_is_rekeyed() {
        let mut s = Store::default();
        let mut cs = s.begin();
        s.replace(Kind::Person, None, vec![person("email:a@x", "Anna", "a@x")], &mut cs);
        s.commit(&cs);
        let mut cs = s.begin();
        s.replace(Kind::Person, None, vec![person("8f0c", "Anna", "A@x")], &mut cs);
        s.commit(&cs);
        assert_eq!(cs.rekeys, vec![(Id::from("email:a@x"), Id::from("8f0c"))]);
    }
}
