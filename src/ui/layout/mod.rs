#![allow(dead_code)] // the main window draws through it from theming phase 4, part 2; until then the tests use it
//! The layout engine (docs/theming-engine.md, "Layout layer"): a window is a declarative tree of regions (docks, splits,
//! stacks, switches, cards) whose leaves are components. The tree comes as data (`webox-ui-layout/1` JSON); the built-in
//! Webex arrangement is [`DEFAULT_LAYOUT`], in the same format a theme uses.
//!
//! The pipeline: [`parse`] (JSON with its path in every error) → [`validate`] (known components, required ones placed)
//! → [`compile`] (a flat node array in draw order, state keys and conditions resolved to ids) → [`Compiled::solve`] (rects
//! for the visible leaves). Parsing and compiling run once per layout change; a frame only solves, and only when the
//! window size, the state bits or a dynamic size changed, into a reused buffer. No strings, no JSON, no allocation per frame.
use eframe::egui::{pos2, Rect};
use serde_json::Value;

mod parse;
#[cfg(test)]
mod tests;
pub(crate) use parse::parse;

/// The built-in layout of the main window: header, rail, then per page the list and conversation cards, the settings
/// navigation and page, or a page.
pub(crate) const DEFAULT_LAYOUT: &str = include_str!("default.json");

/// A component a layout can place. Each keeps all of its behaviour; a layout only places it (theming engine §1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum Component { Header, Rail, SpaceList, Conversation, SettingsNav, SettingsPage, Page }

/// How much a layout must offer a component.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Need {
    /// Placed in every state (the page content).
    Required,
    /// Placed somewhere, or reachable through a toggle (the list, the rail, the settings navigation).
    RequiredReachable,
}

impl Component {
    pub(crate) const ALL: [Component; 7] = [Component::Header, Component::Rail, Component::SpaceList, Component::Conversation,
        Component::SettingsNav, Component::SettingsPage, Component::Page];
    pub(crate) fn id(self) -> &'static str {
        match self {
            Component::Header => "header", Component::Rail => "rail", Component::SpaceList => "space_list",
            Component::Conversation => "conversation", Component::SettingsNav => "settings_nav", Component::SettingsPage => "settings_page",
            Component::Page => "page",
        }
    }
    pub(crate) fn parse(s: &str) -> Option<Component> { Self::ALL.into_iter().find(|c| c.id() == s) }
    pub(crate) fn need(self) -> Need {
        match self {
            Component::Header | Component::Conversation | Component::SettingsPage | Component::Page => Need::Required,
            Component::Rail | Component::SpaceList | Component::SettingsNav => Need::RequiredReachable,
        }
    }
    /// The variants this component knows (contract version 1), with their options.
    pub(crate) fn variants(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            Component::Rail => &[("orientation", &["vertical"]), ("labels", &["never", "when_wide", "always"])],
            Component::SpaceList => &[("row", &["two_line", "one_line"])],
            _ => &[],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Side { Left, Right, Top, Bottom }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Axis { Row, Column }

/// A length: points, or a named value resolved by the caller per solve (`{metrics.header_h}`, `{rail.width}`).
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Len { Px(f32), Named(String) }

/// Size constraints of a region along its parent's axis.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Size { pub min: f32, pub pref: Option<Len>, pub max: f32, pub resizable: bool }
impl Default for Size { fn default() -> Self { Size { min: 0.0, pref: None, max: f32::INFINITY, resizable: false } } }

/// A condition on state keys.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Cond { Key(String, Option<String>), Not(Box<Cond>), All(Vec<Cond>), Any(Vec<Cond>) }

/// The parsed tree.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Node {
    /// `child` on one side of the area, `rest` in what remains.
    Dock { id: Option<String>, side: Side, size: Size, child: Box<Node>, rest: Box<Node> },
    /// Items side by side along `axis`; an item without a preferred size takes what is left.
    Split { id: Option<String>, axis: Axis, gap: f32, items: Vec<SplitItem> },
    /// Children one after another at their preferred size; `grow` children share the rest.
    Stack { id: Option<String>, axis: Axis, gap: f32, children: Vec<StackItem> },
    /// One case chosen by a state key's value; `"*"` is the default.
    Switch { on: String, cases: Vec<(String, Node)>, default: Option<Box<Node>> },
    /// A surface card around a child.
    Card { style: String, child: Box<Node> },
    /// A component.
    Slot { component: Component, variants: Vec<(String, String)> },
}
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct SplitItem { pub size: Size, pub when: Option<Cond>, pub node: Node }
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct StackItem { pub size: Size, pub grow: bool, pub node: Node }

/// A parsed layout file.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Layout { pub id: String, pub name: String, pub main: Node }

/// A finding with the JSON path it is about ("surfaces.main.rest.child").
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Error { pub path: String, pub msg: String }
impl std::fmt::Display for Error { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{}: {}", self.path, self.msg) } }

/// Checks what parsing cannot: every required component is placed, every component's variants exist.
pub(crate) fn validate(l: &Layout) -> Result<(), Vec<Error>> {
    let mut found = vec![];
    let mut errs = vec![];
    walk(&l.main, "surfaces.main", &mut |n, path| {
        if let Node::Slot { component, variants } = n {
            found.push(*component);
            for (k, v) in variants {
                match component.variants().iter().find(|(id, _)| id == k) {
                    None => errs.push(Error { path: path.into(), msg: format!("{} has no variant \"{k}\" (contract {}@1)", component.id(), component.id()) }),
                    Some((_, opts)) if !opts.contains(&v.as_str()) => errs.push(Error { path: path.into(), msg: format!("{}.{k} has no option \"{v}\"", component.id()) }),
                    _ => {}
                }
            }
        }
    });
    for c in Component::ALL {
        if !found.contains(&c) { errs.push(Error { path: "surfaces.main".into(), msg: format!("{} is not placed ({:?})", c.id(), c.need()) }) }
    }
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn walk(n: &Node, path: &str, f: &mut impl FnMut(&Node, &str)) {
    f(n, path);
    match n {
        Node::Dock { child, rest, .. } => { walk(child, &format!("{path}.child"), f); walk(rest, &format!("{path}.rest"), f); }
        Node::Split { items, .. } => for (i, it) in items.iter().enumerate() { walk(&it.node, &format!("{path}.items[{i}].node"), f) },
        Node::Stack { children, .. } => for (i, it) in children.iter().enumerate() { walk(&it.node, &format!("{path}.children[{i}]"), f) },
        Node::Switch { cases, default, .. } => {
            for (k, c) in cases { walk(c, &format!("{path}.cases.{k}"), f) }
            if let Some(d) = default { walk(d, &format!("{path}.cases.*"), f) }
        }
        Node::Card { child, .. } => walk(child, &format!("{path}.child"), f),
        Node::Slot { .. } => {}
    }
}

// ---- compiled form ----

/// A state key's value as the layout compares it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum KeyVal<'a> { None, Bool(bool), Str(&'a str) }

/// What a solve reads: state keys by their compiled index, and named lengths.
pub(crate) trait Env {
    fn key(&self, k: usize) -> KeyVal<'_>;
    fn len(&self, n: usize) -> f32;
}

#[derive(Clone, Debug)]
enum CLen { Px(f32), Named(usize) }
#[derive(Clone, Debug)]
struct CSize { min: f32, pref: Option<CLen>, max: f32 }
#[derive(Clone, Debug)]
enum CCond { Key(usize, Option<String>), Not(Box<CCond>), All(Vec<CCond>), Any(Vec<CCond>) }

#[derive(Clone, Debug)]
enum CNode {
    Dock { side: Side, size: CSize, child: usize, rest: usize },
    Split { axis: Axis, gap: f32, items: Vec<(CSize, Option<CCond>, usize)> },
    Stack { axis: Axis, gap: f32, children: Vec<(CSize, bool, usize)> },
    Switch { on: usize, cases: Vec<(String, usize)>, default: Option<usize> },
    Card { child: usize },
    Slot { component: Component },
}

/// A compiled layout: nodes by index (0 is the root), the state keys and named lengths it reads.
pub(crate) struct Compiled {
    nodes: Vec<CNode>,
    /// State key names, in the order `Env::key` is asked.
    pub keys: Vec<String>,
    /// Named lengths, in the order `Env::len` is asked.
    pub lens: Vec<String>,
}

/// A visible leaf of a solve: a component in its rect, or a card's rect (painted before what it holds).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Placed { Component(Component, Rect), Card(Rect) }

pub(crate) fn compile(l: &Layout) -> Compiled {
    let mut c = Compiled { nodes: vec![], keys: vec![], lens: vec![] };
    c.add(&l.main);
    c
}

impl Compiled {
    fn intern(list: &mut Vec<String>, s: &str) -> usize {
        list.iter().position(|x| x == s).unwrap_or_else(|| { list.push(s.to_string()); list.len() - 1 })
    }
    fn size(&mut self, s: &Size) -> CSize {
        let pref = s.pref.as_ref().map(|l| match l { Len::Px(v) => CLen::Px(*v), Len::Named(n) => CLen::Named(Self::intern(&mut self.lens, n)) });
        CSize { min: s.min, pref, max: s.max }
    }
    fn cond(&mut self, c: &Cond) -> CCond {
        match c {
            Cond::Key(k, v) => CCond::Key(Self::intern(&mut self.keys, k), v.clone()),
            Cond::Not(x) => CCond::Not(Box::new(self.cond(x))),
            Cond::All(v) => CCond::All(v.iter().map(|x| self.cond(x)).collect()),
            Cond::Any(v) => CCond::Any(v.iter().map(|x| self.cond(x)).collect()),
        }
    }
    /// Adds `n` and its subtree; returns its index. Parents come before children (draw order).
    fn add(&mut self, n: &Node) -> usize {
        let at = self.nodes.len();
        self.nodes.push(CNode::Card { child: 0 }); // placeholder
        let c = match n {
            Node::Dock { side, size, child, rest, .. } => { let size = self.size(size); let child = self.add(child); let rest = self.add(rest); CNode::Dock { side: *side, size, child, rest } }
            Node::Split { axis, gap, items, .. } => {
                let items = items.iter().map(|it| { let s = self.size(&it.size); let w = it.when.as_ref().map(|c| self.cond(c)); (s, w, self.add(&it.node)) }).collect();
                CNode::Split { axis: *axis, gap: *gap, items }
            }
            Node::Stack { axis, gap, children, .. } => {
                let children = children.iter().map(|it| { let s = self.size(&it.size); (s, it.grow, self.add(&it.node)) }).collect();
                CNode::Stack { axis: *axis, gap: *gap, children }
            }
            Node::Switch { on, cases, default } => {
                let on = Self::intern(&mut self.keys, on);
                let cases = cases.iter().map(|(k, c)| (k.clone(), self.add(c))).collect();
                let default = default.as_ref().map(|d| self.add(d));
                CNode::Switch { on, cases, default }
            }
            Node::Card { child, .. } => CNode::Card { child: self.add(child) },
            Node::Slot { component, .. } => CNode::Slot { component: *component },
        };
        self.nodes[at] = c;
        at
    }

    /// The rects of the visible leaves in draw order, written into `out` (cleared first; its capacity is reused).
    pub(crate) fn solve(&self, area: Rect, env: &dyn Env, out: &mut Vec<Placed>) {
        out.clear();
        if !self.nodes.is_empty() { self.place(0, area, env, out); }
    }

    fn pref(&self, s: &CSize, env: &dyn Env) -> Option<f32> {
        s.pref.as_ref().map(|p| match p { CLen::Px(v) => *v, CLen::Named(i) => env.len(*i) }.clamp(s.min, s.max.max(s.min)))
    }

    fn holds(&self, c: &CCond, env: &dyn Env) -> bool {
        match c {
            CCond::Key(k, None) => matches!(env.key(*k), KeyVal::Bool(true) | KeyVal::Str(_)),
            CCond::Key(k, Some(v)) => matches!(env.key(*k), KeyVal::Str(s) if s == v),
            CCond::Not(x) => !self.holds(x, env),
            CCond::All(v) => v.iter().all(|x| self.holds(x, env)),
            CCond::Any(v) => v.iter().any(|x| self.holds(x, env)),
        }
    }

    fn place(&self, i: usize, r: Rect, env: &dyn Env, out: &mut Vec<Placed>) {
        match &self.nodes[i] {
            CNode::Slot { component } => out.push(Placed::Component(*component, r)),
            CNode::Card { child } => { out.push(Placed::Card(r)); self.place(*child, r, env, out); }
            CNode::Switch { on, cases, default } => {
                let v = env.key(*on);
                let hit = cases.iter().find(|(k, _)| match v {
                    KeyVal::Str(s) => s == k,
                    KeyVal::Bool(b) => (k == "true") == b,
                    KeyVal::None => k == "none",
                });
                if let Some(n) = hit.map(|h| h.1).or(*default) { self.place(n, r, env, out) }
            }
            CNode::Dock { side, size, child, rest } => {
                let s = self.pref(size, env).unwrap_or(0.0);
                let (a, b) = match side {
                    Side::Left => { let x = (r.left() + s).min(r.right()); (Rect::from_min_max(r.min, pos2(x, r.bottom())), Rect::from_min_max(pos2(x, r.top()), r.max)) }
                    Side::Right => { let x = (r.right() - s).max(r.left()); (Rect::from_min_max(pos2(x, r.top()), r.max), Rect::from_min_max(r.min, pos2(x, r.bottom()))) }
                    Side::Top => { let y = (r.top() + s).min(r.bottom()); (Rect::from_min_max(r.min, pos2(r.right(), y)), Rect::from_min_max(pos2(r.left(), y), r.max)) }
                    Side::Bottom => { let y = (r.bottom() - s).max(r.top()); (Rect::from_min_max(pos2(r.left(), y), r.max), Rect::from_min_max(r.min, pos2(r.right(), y))) }
                };
                self.place(*child, a, env, out);
                self.place(*rest, b, env, out);
            }
            CNode::Split { axis, gap, items } => {
                // Visible items; those with a preferred size get it, the others share what is left (at least their minimum).
                let total = match axis { Axis::Row => r.width(), Axis::Column => r.height() };
                let mut n = 0; let mut fixed = 0.0; let mut flex = 0;
                for (s, w, _) in items {
                    if w.as_ref().is_some_and(|c| !self.holds(c, env)) { continue }
                    n += 1;
                    match self.pref(s, env) { Some(p) => fixed += p, None => flex += 1 }
                }
                if n == 0 { return }
                let free = (total - fixed - gap * (n - 1) as f32).max(0.0);
                let mut at = match axis { Axis::Row => r.left(), Axis::Column => r.top() };
                for (s, w, child) in items {
                    if w.as_ref().is_some_and(|c| !self.holds(c, env)) { continue }
                    let len = self.pref(s, env).unwrap_or_else(|| (free / flex.max(1) as f32).clamp(s.min, s.max.max(s.min)));
                    let cr = match axis {
                        Axis::Row => Rect::from_min_max(pos2(at, r.top()), pos2(at + len, r.bottom())),
                        Axis::Column => Rect::from_min_max(pos2(r.left(), at), pos2(r.right(), at + len)),
                    };
                    self.place(*child, cr, env, out);
                    at += len + gap;
                }
            }
            CNode::Stack { axis, gap, children } => {
                let total = match axis { Axis::Row => r.width(), Axis::Column => r.height() };
                let fixed: f32 = children.iter().filter(|c| !c.1).map(|c| self.pref(&c.0, env).unwrap_or(0.0)).sum();
                let grow = children.iter().filter(|c| c.1).count();
                let free = (total - fixed - gap * children.len().saturating_sub(1) as f32).max(0.0);
                let mut at = match axis { Axis::Row => r.left(), Axis::Column => r.top() };
                for (s, g, child) in children {
                    let len = if *g { free / grow as f32 } else { self.pref(s, env).unwrap_or(0.0) };
                    let cr = match axis {
                        Axis::Row => Rect::from_min_max(pos2(at, r.top()), pos2(at + len, r.bottom())),
                        Axis::Column => Rect::from_min_max(pos2(r.left(), at), pos2(r.right(), at + len)),
                    };
                    self.place(*child, cr, env, out);
                    at += len + gap;
                }
            }
        }
    }
}

/// Parses, validates and compiles a layout file; every finding with its JSON path.
pub(crate) fn load(json: &str) -> Result<(Layout, Compiled), Vec<Error>> {
    let v: Value = serde_json::from_str(json).map_err(|e| vec![Error { path: format!("line {} column {}", e.line(), e.column()), msg: e.to_string() }])?;
    let l = parse(&v)?;
    validate(&l)?;
    let c = compile(&l);
    Ok((l, c))
}
