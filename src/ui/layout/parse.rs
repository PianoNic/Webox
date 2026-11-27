//! `webox-ui-layout/1` JSON → [`Layout`]. Every error names the JSON path it is about. Unknown keys are errors, so a
//! misspelt key never silently does nothing.
use super::*;

const SCHEMA: &str = "webox-ui-layout/1";

/// Parses a layout file's JSON.
pub(crate) fn parse(v: &Value) -> Result<Layout, Vec<Error>> {
    let mut p = P { errs: vec![] };
    let o = p.obj(v, "");
    match o.and_then(|o| o.get("schema")).and_then(Value::as_str) {
        Some(SCHEMA) => {}
        Some(s) => p.err("schema", format!("unknown schema \"{s}\" (this Webox reads {SCHEMA})")),
        None => p.err("schema", format!("missing (expected \"{SCHEMA}\")")),
    }
    let s = |k: &str| o.and_then(|o| o.get(k)).and_then(Value::as_str).unwrap_or_default().to_string();
    let (id, name) = (s("id"), s("name"));
    if id.is_empty() { p.err("id", "missing".into()); }
    if let Some(o) = o { p.known(o, "", &["schema", "id", "name", "surfaces", "panels", "data", "reach"]); }
    let main = match o.and_then(|o| o.get("surfaces")).and_then(|s| s.get("main")) {
        Some(m) => p.node(m, "surfaces.main"),
        None => { p.err("surfaces.main", "missing".into()); None }
    };
    match main {
        Some(main) if p.errs.is_empty() => Ok(Layout { id, name, main }),
        _ => Err(p.errs),
    }
}

struct P { errs: Vec<Error> }

impl P {
    fn err(&mut self, path: &str, msg: String) { self.errs.push(Error { path: path.trim_start_matches('.').to_string(), msg }) }
    fn obj<'a>(&mut self, v: &'a Value, path: &str) -> Option<&'a serde_json::Map<String, Value>> {
        let o = v.as_object();
        if o.is_none() { self.err(path, "expected an object".into()) }
        o
    }
    fn known(&mut self, o: &serde_json::Map<String, Value>, path: &str, keys: &[&str]) {
        for k in o.keys() { if !keys.contains(&k.as_str()) { self.err(&format!("{path}.{k}"), "unknown key".into()) } }
    }
    fn num(&mut self, v: &Value, path: &str) -> Option<f32> {
        let n = v.as_f64().map(|x| x as f32).filter(|x| x.is_finite() && *x >= 0.0);
        if n.is_none() { self.err(path, "expected a number of points (0 or more)".into()) }
        n
    }
    fn len(&mut self, v: &Value, path: &str) -> Option<Len> {
        if let Some(s) = v.as_str() {
            // "{metrics.header_h}", "{tok:night.rail.width}": named, resolved per solve.
            match s.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c)) => return Some(Len::Named(n.to_string())),
                _ => { self.err(path, format!("expected a number or a {{name}}, not \"{s}\"")); return None }
            }
        }
        self.num(v, path).map(Len::Px)
    }
    fn size(&mut self, v: Option<&Value>, path: &str) -> Size {
        let mut s = Size::default();
        let Some(v) = v else { return s };
        let Some(o) = self.obj(v, path) else { return s };
        self.known(o, path, &["min", "pref", "max", "resizable"]);
        if let Some(x) = o.get("min") { s.min = self.num(x, &format!("{path}.min")).unwrap_or(0.0) }
        if let Some(x) = o.get("max") { s.max = self.num(x, &format!("{path}.max")).unwrap_or(f32::INFINITY) }
        if let Some(x) = o.get("pref") { s.pref = self.len(x, &format!("{path}.pref")) }
        if let Some(x) = o.get("resizable") { s.resizable = x.as_bool().unwrap_or_else(|| { self.err(&format!("{path}.resizable"), "expected true or false".into()); false }) }
        if s.min > s.max { self.err(path, format!("min {} is larger than max {}", s.min, s.max)) }
        s
    }
    fn axis(&mut self, v: &Value, path: &str) -> Axis {
        match v.as_str() { Some("row") => Axis::Row, Some("column") => Axis::Column, _ => { self.err(path, "expected \"row\" or \"column\"".into()); Axis::Row } }
    }
    fn cond(&mut self, v: &Value, path: &str) -> Option<Cond> {
        let o = self.obj(v, path)?;
        if let Some(k) = o.get("key") {
            self.known(o, path, &["key", "eq"]);
            let key = k.as_str().map(str::to_string);
            if key.is_none() { self.err(&format!("{path}.key"), "expected a state key".into()) }
            let eq = o.get("eq").map(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string()));
            return key.map(|k| Cond::Key(k, eq));
        }
        if let Some(x) = o.get("not") { self.known(o, path, &["not"]); return self.cond(x, &format!("{path}.not")).map(|c| Cond::Not(Box::new(c))); }
        for (k, f) in [("all", Cond::All as fn(Vec<Cond>) -> Cond), ("any", Cond::Any)] {
            if let Some(x) = o.get(k) {
                self.known(o, path, &[k]);
                let Some(a) = x.as_array() else { self.err(&format!("{path}.{k}"), "expected a list".into()); return None };
                let v: Vec<Cond> = a.iter().enumerate().filter_map(|(i, c)| self.cond(c, &format!("{path}.{k}[{i}]"))).collect();
                return Some(f(v));
            }
        }
        self.err(path, "expected key, not, all or any".into());
        None
    }
    fn id(o: &serde_json::Map<String, Value>) -> Option<String> { o.get("id").and_then(Value::as_str).map(str::to_string) }

    fn node(&mut self, v: &Value, path: &str) -> Option<Node> {
        let o = self.obj(v, path)?;
        if let Some(c) = o.get("slot") {
            self.known(o, path, &["slot", "variants", "grow", "size"]);
            let Some(component) = c.as_str().and_then(Component::parse) else {
                self.err(&format!("{path}.slot"), format!("unknown component {c} (known: {})", Component::ALL.map(|c| c.id()).join(", ")));
                return None;
            };
            let mut variants = vec![];
            if let Some(vs) = o.get("variants") {
                match vs.as_object() {
                    Some(m) => for (k, v) in m {
                        match v.as_str() { Some(s) => variants.push((k.clone(), s.to_string())), None => self.err(&format!("{path}.variants.{k}"), "expected a text".into()) }
                    },
                    None => self.err(&format!("{path}.variants"), "expected an object".into()),
                }
            }
            return Some(Node::Slot { component, variants });
        }
        if let Some(side) = o.get("dock") {
            self.known(o, path, &["dock", "id", "size", "child", "rest"]);
            let side = match side.as_str() {
                Some("left") => Side::Left, Some("right") => Side::Right, Some("top") => Side::Top, Some("bottom") => Side::Bottom,
                _ => { self.err(&format!("{path}.dock"), "expected left, right, top or bottom".into()); Side::Left }
            };
            let size = self.size(o.get("size"), &format!("{path}.size"));
            let child = self.need(o, "child", path);
            let rest = self.need(o, "rest", path);
            return Some(Node::Dock { id: Self::id(o), side, size, child: Box::new(child?), rest: Box::new(rest?) });
        }
        if let Some(a) = o.get("split") {
            self.known(o, path, &["split", "id", "gap", "items"]);
            let axis = self.axis(a, &format!("{path}.split"));
            let gap = o.get("gap").and_then(|g| self.num(g, &format!("{path}.gap"))).unwrap_or(0.0);
            let Some(items) = o.get("items").and_then(Value::as_array) else { self.err(&format!("{path}.items"), "expected a list".into()); return None };
            let mut out = vec![];
            for (i, it) in items.iter().enumerate() {
                let ip = format!("{path}.items[{i}]");
                let Some(io) = self.obj(it, &ip) else { continue };
                self.known(io, &ip, &["size", "when", "node"]);
                let size = self.size(io.get("size"), &format!("{ip}.size"));
                let when = io.get("when").and_then(|w| self.cond(w, &format!("{ip}.when")));
                if let Some(node) = self.need(io, "node", &ip) { out.push(SplitItem { size, when, node }) }
            }
            return Some(Node::Split { id: Self::id(o), axis, gap, items: out });
        }
        if let Some(a) = o.get("stack") {
            self.known(o, path, &["stack", "id", "gap", "children"]);
            let axis = self.axis(a, &format!("{path}.stack"));
            let gap = o.get("gap").and_then(|g| self.num(g, &format!("{path}.gap"))).unwrap_or(0.0);
            let Some(ch) = o.get("children").and_then(Value::as_array) else { self.err(&format!("{path}.children"), "expected a list".into()); return None };
            let mut out = vec![];
            for (i, c) in ch.iter().enumerate() {
                let cp = format!("{path}.children[{i}]");
                let grow = c.get("grow").and_then(Value::as_bool).unwrap_or(false);
                let size = self.size(c.get("size"), &format!("{cp}.size"));
                if let Some(node) = self.node(c, &cp) { out.push(StackItem { size, grow, node }) }
            }
            return Some(Node::Stack { id: Self::id(o), axis, gap, children: out });
        }
        if let Some(on) = o.get("switch") {
            self.known(o, path, &["switch", "id", "cases", "default"]);
            let Some(on) = on.as_str() else { self.err(&format!("{path}.switch"), "expected a state key".into()); return None };
            let Some(cases) = o.get("cases").and_then(Value::as_object) else { self.err(&format!("{path}.cases"), "expected an object".into()); return None };
            let mut out = vec![];
            let mut default = o.get("default").and_then(|d| self.node(d, &format!("{path}.default"))).map(Box::new);
            for (k, c) in cases {
                let n = self.node(c, &format!("{path}.cases.{k}"));
                match (k.as_str(), n) { ("*", Some(n)) => default = Some(Box::new(n)), (_, Some(n)) => out.push((k.clone(), n)), _ => {} }
            }
            return Some(Node::Switch { on: on.to_string(), cases: out, default });
        }
        if let Some(st) = o.get("card") {
            self.known(o, path, &["card", "id", "child"]);
            let style = st.as_str().unwrap_or_default().to_string();
            return self.need(o, "child", path).map(|c| Node::Card { style, child: Box::new(c) });
        }
        self.err(path, "expected slot, dock, split, stack, switch or card".into());
        None
    }
    fn need(&mut self, o: &serde_json::Map<String, Value>, k: &str, path: &str) -> Option<Node> {
        match o.get(k) { Some(v) => self.node(v, &format!("{path}.{k}")), None => { self.err(&format!("{path}.{k}"), "missing".into()); None } }
    }
}
