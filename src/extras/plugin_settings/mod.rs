//! Plugin settings (plugin-api contract 1.2): the schema a plugin declares (`schema`), where the values are kept (`Store`: one
//! file per plugin in `<config>/plugin-settings/<key>.json`, secrets in the OS keyring through `secret`), and which values the
//! plugin sees (`Schema::effective`: stored where still valid, else the default). Native and built-in plugins share all of it;
//! `crate::extras::plugins` calls the plugin back on every change, and the Plugins page draws the form from the schema.
pub mod schema;
pub mod secret;
#[cfg(test)]
mod tests;

pub use schema::{Field, Kind, Schema};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Where a secret setting is kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SecretAt { Unset, Keyring, File }

/// What is saved for one plugin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stored { pub values: Map<String, Value>, pub secrets: Map<String, Value>, pub in_file: Vec<String> }

pub struct Store { dir: PathBuf, secrets: Box<dyn secret::Secrets> }

/// File name for a plugin key (package id, `builtin.*`, or a library name).
fn file_key(key: &str) -> String { key.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect() }

impl Store {
    pub fn new(config: &Path) -> Store { Store { dir: config.join("plugin-settings"), secrets: Box::new(secret::OsSecrets) } }
    pub fn set_secrets(&mut self, s: Box<dyn secret::Secrets>) { self.secrets = s; }
    fn path(&self, key: &str) -> PathBuf { self.dir.join(format!("{}.json", file_key(key))) }

    fn read_file(&self, key: &str) -> Value {
        std::fs::read_to_string(self.path(key)).ok().and_then(|t| serde_json::from_str(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
    }
    fn write_file(&self, key: &str, v: &Value) -> Result<(), String> {
        crate::extras::plugin_repo::store::write_atomic(&self.path(key), serde_json::to_string_pretty(v).unwrap_or_default().as_bytes())
    }

    /// Values and secrets of `key` for the fields in `schema`.
    pub fn load(&self, key: &str, schema: &Schema) -> Stored {
        let f = self.read_file(key);
        let mut st = Stored { values: f["values"].as_object().cloned().unwrap_or_default(), ..Default::default() };
        for fd in schema.fields().filter(|fd| fd.kind == Kind::Secret) {
            if let Some(v) = f["fileSecrets"].get(&fd.id).and_then(Value::as_str) {
                st.secrets.insert(fd.id.clone(), Value::String(v.into()));
                st.in_file.push(fd.id.clone());
            } else if f["secrets"].get(&fd.id).is_some() {
                if let Some(v) = self.secrets.get(key, &fd.id) { st.secrets.insert(fd.id.clone(), Value::String(v)); }
            }
        }
        st
    }

    /// Saves an already validated value.
    pub fn save_value(&self, key: &str, id: &str, v: &Value) -> Result<(), String> {
        let mut f = self.read_file(key);
        if !f["values"].is_object() { f["values"] = json!({}); }
        f["values"][id] = v.clone();
        self.write_file(key, &f)
    }

    /// Saves (or with "" removes) a secret: in the keyring if it takes it, else in the file (returned as `SecretAt::File`).
    pub fn save_secret(&mut self, key: &str, id: &str, v: &str) -> Result<SecretAt, String> {
        let mut f = self.read_file(key);
        for k in ["secrets", "fileSecrets"] { if !f[k].is_object() { f[k] = json!({}); } }
        f["fileSecrets"].as_object_mut().unwrap().remove(id);
        f["secrets"].as_object_mut().unwrap().remove(id);
        let at = if v.is_empty() {
            self.secrets.delete(key, id);
            SecretAt::Unset
        } else if self.secrets.set(key, id, v).is_ok() {
            // Only a marker: the value itself is in the keyring.
            f["secrets"][id] = json!("keyring");
            SecretAt::Keyring
        } else {
            f["fileSecrets"][id] = json!(v);
            SecretAt::File
        };
        self.write_file(key, &f)?;
        Ok(at)
    }
}

/// A plugin's settings while it is listed: its schema, the values it sees (shared with the native callback context).
#[derive(Default)]
pub struct State {
    pub schema: Option<Schema>,
    pub error: Option<String>,
    pub values: std::sync::Arc<std::sync::Mutex<Map<String, Value>>>,
    pub in_file: Vec<String>,
}
impl State {
    pub fn values(&self) -> Map<String, Value> { self.values.lock().unwrap_or_else(|e| e.into_inner()).clone() }
    /// One value, without copying the others (built-in plugins read theirs every frame).
    pub fn value(&self, id: &str) -> Option<Value> { self.values.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned() }
    pub fn json(&self) -> String { Value::Object(self.values()).to_string() }
    pub fn secret_at(&self, id: &str) -> SecretAt {
        let set = self.value(id).as_ref().and_then(Value::as_str).is_some_and(|s| !s.is_empty());
        if !set { SecretAt::Unset } else if self.in_file.iter().any(|x| x == id) { SecretAt::File } else { SecretAt::Keyring }
    }
    /// Sets the schema (or its parse error) and loads the values.
    pub fn load(&mut self, store: &Store, key: &str, json: Option<&str>) {
        let (schema, error) = match json.map(Schema::parse) { Some(Ok(s)) if !s.is_empty() => (Some(s), None), Some(Ok(_)) | None => (None, None), Some(Err(e)) => (None, Some(e)) };
        self.error = error;
        self.reload(store, key, schema);
    }
    pub fn reload(&mut self, store: &Store, key: &str, schema: Option<Schema>) {
        let eff = match &schema {
            Some(s) => { let st = store.load(key, s); self.in_file = st.in_file.clone(); s.effective(&st.values, &st.secrets) }
            None => Map::new(),
        };
        *self.values.lock().unwrap_or_else(|e| e.into_inner()) = eff;
        self.schema = schema;
    }
}
