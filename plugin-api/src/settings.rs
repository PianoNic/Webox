//! Plugin settings (contract 1.2). A plugin declares its settings as a schema; Webox draws the form on the plugin's own page
//! under Settings > Plugins, validates and saves the values, and hands them to the plugin as a flat JSON object
//! (`{"id": value, ...}`) through [`crate::Plugin::settings_changed`] (also once before every start). Nothing but JSON strings
//! crosses the boundary, so no UI code runs in the plugin.
//!
//! ```ignore
//! fn settings(&self) -> Option<Schema> {
//!     Some(Schema::new()
//!         .section("Replacements", "")
//!         .toggle("smiley", "Replace ':)' with an emoji", "", true)
//!         .text("shrug", "Text for /shrug", "", "¯\\_(ツ)_/¯")
//!         .button("reset", "Reset counter", ""))
//! }
//! ```
//! The schema format is documented in Webox's docs/plugins.md ("Plugin settings").

/// Schema format version written by [`Schema::to_json`].
pub const SCHEMA_VERSION: u32 = 1;

/// Quotes a string for JSON.
pub fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""), '\\' => o.push_str("\\\\"), '\n' => o.push_str("\\n"), '\r' => o.push_str("\\r"), '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
fn num(v: f64) -> String { if v.is_finite() { format!("{v}") } else { "0".into() } }

/// Builder for the settings schema. Fields go into the last [`Schema::section`] (or an untitled first section).
#[derive(Clone, Debug, Default)]
pub struct Schema { sections: Vec<(String, String, Vec<String>)> }

impl Schema {
    pub fn new() -> Schema { Schema::default() }
    /// Starts a titled section (shown as a settings section with its description).
    pub fn section(mut self, title: &str, description: &str) -> Schema { self.sections.push((title.into(), description.into(), vec![])); self }
    fn field(mut self, kind: &str, id: &str, label: &str, description: &str, extra: String) -> Schema {
        if self.sections.is_empty() { self.sections.push((String::new(), String::new(), vec![])); }
        let f = format!("{{\"type\":{},\"id\":{},\"label\":{},\"description\":{}{extra}}}", json_str(kind), json_str(id), json_str(label), json_str(description));
        self.sections.last_mut().unwrap().2.push(f);
        self
    }
    /// An on / off switch.
    pub fn toggle(self, id: &str, label: &str, description: &str, default: bool) -> Schema { self.field("bool", id, label, description, format!(",\"default\":{default}")) }
    /// A one-line text field.
    pub fn text(self, id: &str, label: &str, description: &str, default: &str) -> Schema { self.field("string", id, label, description, format!(",\"default\":{}", json_str(default))) }
    /// A text area.
    pub fn multiline(self, id: &str, label: &str, description: &str, default: &str) -> Schema { self.field("multiline", id, label, description, format!(",\"default\":{}", json_str(default))) }
    /// A secret (API key, token): shown masked, kept in the OS keyring where there is one, never written to Webox's logs.
    pub fn password(self, id: &str, label: &str, description: &str) -> Schema { self.field("password", id, label, description, String::new()) }
    /// A number in `min..=max`; `step` 0 = any value.
    pub fn number(self, id: &str, label: &str, description: &str, default: f64, min: f64, max: f64, step: f64) -> Schema {
        self.field("number", id, label, description, format!(",\"default\":{},\"min\":{},\"max\":{},\"step\":{}", num(default), num(min), num(max), num(step)))
    }
    /// One of `options` (value, label).
    pub fn choice(self, id: &str, label: &str, description: &str, default: &str, options: &[(&str, &str)]) -> Schema {
        let opts: Vec<String> = options.iter().map(|(v, l)| format!("{{\"value\":{},\"label\":{}}}", json_str(v), json_str(l))).collect();
        self.field("choice", id, label, description, format!(",\"default\":{},\"options\":[{}]", json_str(default), opts.join(",")))
    }
    /// A button; pressing it calls [`crate::Plugin::settings_action`] with `id`.
    pub fn button(self, id: &str, label: &str, description: &str) -> Schema { self.field("button", id, label, description, String::new()) }
    /// A file path with a "Browse…" button that opens the system's file dialog, filtered to `extensions` (without dots;
    /// empty = all files) under the name `filter`. The value is the absolute path ("" = none). Hosts from before this field
    /// type leave the field out.
    pub fn file(self, id: &str, label: &str, description: &str, filter: &str, extensions: &[&str]) -> Schema {
        let ext: Vec<String> = extensions.iter().map(|e| json_str(e)).collect();
        self.field("file", id, label, description, format!(",\"default\":\"\",\"filter\":{},\"extensions\":[{}]", json_str(filter), ext.join(",")))
    }

    pub fn to_json(&self) -> String {
        let secs: Vec<String> = self.sections.iter().map(|(t, d, f)| format!("{{\"title\":{},\"description\":{},\"fields\":[{}]}}", json_str(t), json_str(d), f.join(","))).collect();
        format!("{{\"version\":{SCHEMA_VERSION},\"sections\":[{}]}}", secs.join(","))
    }
}

/// One setting value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value { Bool(bool), Number(f64), Text(String), Null }

/// The current values, as Webox passes them: a flat JSON object of booleans, numbers and strings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values(pub Vec<(String, Value)>);

impl Values {
    pub fn get(&self, id: &str) -> Option<&Value> { self.0.iter().find(|(k, _)| k == id).map(|(_, v)| v) }
    pub fn bool(&self, id: &str) -> Option<bool> { match self.get(id)? { Value::Bool(b) => Some(*b), _ => None } }
    pub fn number(&self, id: &str) -> Option<f64> { match self.get(id)? { Value::Number(n) => Some(*n), _ => None } }
    pub fn text(&self, id: &str) -> Option<&str> { match self.get(id)? { Value::Text(s) => Some(s), _ => None } }

    /// Reads a flat JSON object; anything else (nested values, bad JSON) gives what was read up to there.
    pub fn parse(json: &str) -> Values {
        let mut p = P { s: json.as_bytes(), i: 0 };
        let mut out = vec![];
        p.ws();
        if !p.eat(b'{') { return Values(out); }
        loop {
            p.ws();
            if p.eat(b'}') { break; }
            let Some(k) = p.string() else { break };
            p.ws();
            if !p.eat(b':') { break; }
            p.ws();
            let Some(v) = p.value() else { break };
            out.push((k, v));
            p.ws();
            if p.eat(b',') { continue; }
            break;
        }
        Values(out)
    }
}

struct P<'a> { s: &'a [u8], i: usize }
impl P<'_> {
    fn ws(&mut self) { while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() { self.i += 1 } }
    fn eat(&mut self, c: u8) -> bool { if self.s.get(self.i) == Some(&c) { self.i += 1; true } else { false } }
    fn lit(&mut self, w: &str) -> bool { if self.s[self.i..].starts_with(w.as_bytes()) { self.i += w.len(); true } else { false } }
    fn value(&mut self) -> Option<Value> {
        match self.s.get(self.i)? {
            b'"' => self.string().map(Value::Text),
            b't' => self.lit("true").then_some(Value::Bool(true)),
            b'f' => self.lit("false").then_some(Value::Bool(false)),
            b'n' => self.lit("null").then_some(Value::Null),
            _ => {
                let st = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') { self.i += 1 }
                std::str::from_utf8(&self.s[st..self.i]).ok()?.parse().ok().map(Value::Number)
            }
        }
    }
    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(h, 16).ok()
    }
    fn string(&mut self) -> Option<String> {
        if !self.eat(b'"') { return None; }
        let mut buf: Vec<u8> = vec![];
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(buf).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'n' => '\n', b'r' => '\r', b't' => '\t', b'b' => '\u{8}', b'f' => '\u{c}', b'/' => '/', b'\\' => '\\', b'"' => '"',
                        b'u' => {
                            let mut u = self.hex4()?;
                            if (0xD800..0xDC00).contains(&u) && self.lit("\\u") { let lo = self.hex4()?; u = 0x10000 + ((u - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF); }
                            char::from_u32(u).unwrap_or('\u{fffd}')
                        }
                        _ => return None,
                    };
                    let mut b = [0u8; 4];
                    buf.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                }
                c => buf.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_json_shape() {
        let j = Schema::new().toggle("a", "A \"x\"", "", true).section("S", "d").number("n", "N", "", 2.0, 0.0, 5.0, 1.0)
            .choice("c", "C", "", "x", &[("x", "X"), ("y", "Y")]).password("p", "P", "").to_json();
        assert!(j.starts_with("{\"version\":1,\"sections\":[{\"title\":\"\""));
        assert!(j.contains("\"label\":\"A \\\"x\\\"\"") && j.contains("\"min\":0,\"max\":5,\"step\":1") && j.contains("{\"value\":\"y\",\"label\":\"Y\"}"));
    }

    #[test]
    fn values_parse() {
        let v = Values::parse(r#" { "a": true, "n": -2.5e1, "s": "x\"\nä🙂", "z": null, "b": false } "#);
        assert_eq!(v.bool("a"), Some(true));
        assert_eq!(v.number("n"), Some(-25.0));
        assert_eq!(v.text("s"), Some("x\"\n\u{e4}\u{1f642}"));
        assert_eq!(v.get("z"), Some(&Value::Null));
        assert_eq!(v.bool("b"), Some(false));
        assert_eq!(Values::parse("nope").0.len(), 0);
        assert_eq!(Values::parse(r#"{"a":1,"b":{"x":1}}"#).0.len(), 1);
    }
}
