//! The settings schema a plugin declares (JSON, docs/plugins.md "Plugin settings"), parsed and checked, and the rules a value
//! has to pass before it is saved. Built-in plugins use the same format (they build it with plugin-api's `Schema` too).
use serde_json::{Map, Value};

pub const MAX_FIELDS: usize = 64;
pub const TEXT_MAX: usize = 1000;
pub const MULTILINE_MAX: usize = 20_000;
pub const SECRET_MAX: usize = 4096;
/// Longest file path a `file` field takes.
pub const PATH_MAX: usize = 4096;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Bool { default: bool },
    Text { default: String, multiline: bool },
    /// Kept in the OS keyring where there is one; never has a default.
    Secret,
    Number { default: f64, min: f64, max: f64, step: f64 },
    Choice { default: String, options: Vec<(String, String)> },
    /// A file path (`""` = none) picked with the system dialog, filtered to `extensions` named `filter`.
    File { filter: String, extensions: Vec<String> },
    /// Calls the plugin back; holds no value.
    Button,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field { pub id: String, pub label: String, pub description: String, pub kind: Kind }

#[derive(Clone, Debug, PartialEq)]
pub struct Section { pub title: String, pub description: String, pub fields: Vec<Field> }

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Schema { pub sections: Vec<Section>, /// fields that were left out (unknown type, bad values)
    pub skipped: usize }

fn s(v: &Value, k: &str) -> String { v.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string() }
fn valid_id(id: &str) -> bool { !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)) }
fn cut(t: String, n: usize) -> String { t.chars().take(n).collect() }

fn field(v: &Value) -> Option<Field> {
    let id = s(v, "id");
    let label = cut(s(v, "label"), 200);
    if !valid_id(&id) || label.is_empty() { return None; }
    let n = |k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
    let kind = match s(v, "type").as_str() {
        "bool" => Kind::Bool { default: v.get("default").and_then(Value::as_bool).unwrap_or(false) },
        t @ ("string" | "multiline") => {
            let multiline = t == "multiline";
            let mut d = v.get("default").and_then(Value::as_str).unwrap_or_default().to_string();
            if !multiline { d = d.replace(['\n', '\r'], " "); }
            Kind::Text { default: cut(d, if multiline { MULTILINE_MAX } else { TEXT_MAX }), multiline }
        }
        "password" => Kind::Secret,
        "number" => {
            let (min, max) = (n("min")?, n("max")?);
            if min > max { return None; }
            let step = n("step").unwrap_or(0.0).max(0.0);
            Kind::Number { default: n("default").unwrap_or(min).clamp(min, max), min, max, step }
        }
        "choice" => {
            let options: Vec<(String, String)> = v.get("options")?.as_array()?.iter()
                .map(|o| (s(o, "value"), cut(s(o, "label"), 200))).filter(|(v, l)| !v.is_empty() && !l.is_empty()).take(100).collect();
            if options.is_empty() { return None; }
            let d = s(v, "default");
            Kind::Choice { default: if options.iter().any(|o| o.0 == d) { d } else { options[0].0.clone() }, options }
        }
        "file" => Kind::File { filter: cut(s(v, "filter"), 100), extensions: v.get("extensions").and_then(Value::as_array).into_iter().flatten()
            .filter_map(Value::as_str).map(|e| e.trim_start_matches(".").to_ascii_lowercase()).filter(|e| !e.is_empty() && e.len() <= 16 && e.bytes().all(|b| b.is_ascii_alphanumeric())).take(32).collect() },
        "button" => Kind::Button,
        _ => return None,
    };
    Some(Field { id, label, description: cut(s(v, "description"), 1000), kind })
}

impl Schema {
    /// Reads a schema. Fails on a broken document; single bad fields are skipped (`skipped`).
    pub fn parse(json: &str) -> Result<Schema, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| crate::trq!("Settings are not valid JSON: {{0}}", e))?;
        if !v.is_object() { return Err(crate::trq!("Settings must be a JSON object").into_owned()); }
        let secs: Vec<Value> = match (v.get("sections").and_then(Value::as_array), v.get("fields").and_then(Value::as_array)) {
            (Some(s), _) => s.clone(),
            (None, Some(f)) => vec![serde_json::json!({ "fields": f })],
            _ => return Err(crate::trq!("Settings without \"sections\" or \"fields\"").into_owned()),
        };
        let mut out = Schema::default();
        let mut ids = std::collections::HashSet::new();
        let mut count = 0;
        for sec in &secs {
            let mut fields = vec![];
            for f in sec.get("fields").and_then(Value::as_array).into_iter().flatten() {
                match field(f) {
                    Some(f) if count < MAX_FIELDS && ids.insert(f.id.clone()) => { count += 1; fields.push(f) }
                    _ => out.skipped += 1,
                }
            }
            let (title, description) = (cut(s(sec, "title"), 200), cut(s(sec, "description"), 1000));
            if !fields.is_empty() || !title.is_empty() { out.sections.push(Section { title, description, fields }); }
        }
        Ok(out)
    }

    pub fn fields(&self) -> impl Iterator<Item = &Field> { self.sections.iter().flat_map(|s| s.fields.iter()) }
    pub fn field(&self, id: &str) -> Option<&Field> { self.fields().find(|f| f.id == id) }
    pub fn is_empty(&self) -> bool { self.fields().next().is_none() }

    /// Checks a new value for `id`; returns it normalised (numbers as f64, text as is).
    pub fn validate(&self, id: &str, v: &Value) -> Result<Value, String> {
        let f = self.field(id).ok_or_else(|| crate::trq!("unknown setting '{{0}}'", id))?;
        match &f.kind {
            Kind::Bool { .. } => v.as_bool().map(Value::Bool).ok_or_else(|| crate::trq!("must be on or off").into_owned()),
            Kind::Text { multiline, .. } => {
                let t = v.as_str().ok_or_else(|| crate::trq!("must be text").into_owned())?;
                let max = if *multiline { MULTILINE_MAX } else { TEXT_MAX };
                if !multiline && t.contains(['\n', '\r']) { return Err(crate::trq!("must not contain a line break").into_owned()); }
                if t.chars().count() > max { return Err(crate::trq!("at most {{0}} characters", max)); }
                Ok(Value::String(t.to_string()))
            }
            Kind::Secret => {
                let t = v.as_str().ok_or_else(|| crate::trq!("must be text").into_owned())?;
                if t.chars().count() > SECRET_MAX { return Err(crate::trq!("at most {{0}} characters", SECRET_MAX)); }
                Ok(Value::String(t.to_string()))
            }
            Kind::Number { min, max, step, .. } => {
                let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| crate::trq!("must be a number").into_owned())?;
                if x < *min || x > *max { return Err(crate::trq!("must be between {{0}} and {{1}}", fmt_num(*min), fmt_num(*max))); }
                if *step > 0.0 {
                    let k = (x - min) / step;
                    if (k - k.round()).abs() > 1e-6 { return Err(crate::trq!("must be in steps of {{0}}", fmt_num(*step))); }
                }
                Ok(serde_json::json!(x))
            }
            Kind::Choice { options, .. } => {
                let t = v.as_str().ok_or_else(|| crate::trq!("must be one of the options").into_owned())?;
                if options.iter().any(|o| o.0 == t) { Ok(Value::String(t.into())) } else { Err(crate::trq!("must be one of the options").into_owned()) }
            }
            Kind::File { .. } => {
                let t = v.as_str().ok_or_else(|| crate::trq!("must be a file path").into_owned())?;
                if t.contains(['\n', '\r']) { return Err(crate::trq!("must not contain a line break").into_owned()); }
                if t.chars().count() > PATH_MAX { return Err(crate::trq!("at most {{0}} characters", PATH_MAX)); }
                Ok(Value::String(t.to_string()))
            }
            Kind::Button => Err(crate::trq!("a button has no value").into_owned()),
        }
    }

    /// Every value the plugin sees: the stored one where it is still valid, else the default; secrets from `secrets` ("" = unset).
    pub fn effective(&self, stored: &Map<String, Value>, secrets: &Map<String, Value>) -> Map<String, Value> {
        let mut out = Map::new();
        for f in self.fields() {
            let v = match &f.kind {
                Kind::Button => continue,
                Kind::Secret => secrets.get(&f.id).filter(|v| v.is_string()).cloned().unwrap_or_else(|| Value::String(String::new())),
                k => stored.get(&f.id).and_then(|v| self.validate(&f.id, v).ok()).unwrap_or_else(|| default_of(k)),
            };
            out.insert(f.id.clone(), v);
        }
        out
    }
}

pub fn default_of(k: &Kind) -> Value {
    match k {
        Kind::Bool { default } => Value::Bool(*default),
        Kind::Text { default, .. } => Value::String(default.clone()),
        Kind::Number { default, .. } => serde_json::json!(default),
        Kind::Choice { default, .. } => Value::String(default.clone()),
        Kind::Secret | Kind::Button | Kind::File { .. } => Value::String(String::new()),
    }
}

/// 2 -> "2", 0.25 -> "0.25".
pub fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 { format!("{}", x as i64) } else { format!("{x}") }
}
