//! Plugin settings on the host side: changing a value (validated against the plugin's schema, saved, then the plugin is told with
//! the full set of values), settings buttons, reading a value (built-in plugins ask here), and the entries of the settings search.
use super::{Loaded, Plugins};
use crate::extras::plugin_settings::{self as ps, Kind, SecretAt};
use collabhost_plugin_api::cs;
use serde_json::Value;

/// Hands the current values to a native plugin (`settings_changed` and the 1.6 `settings.changed` event; before every start
/// and after every change).
pub(super) fn push_settings(l: &Loaded, st: &ps::State) {
    if st.schema.is_none() { return; }
    let json = st.json();
    if let Some(f) = l.c.changed { let j = cs(&json); f(l.inst, j.as_ptr()) }
    l.event("settings.changed", &json);
}

/// One hit of the settings search inside a plugin's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit { pub key: String, pub plugin: String, pub label: String, pub description: String }

impl Plugins {
    /// Validates and saves a new value for setting `id` of plugin `i`, then tells the plugin.
    pub fn set_setting(&mut self, i: usize, id: &str, v: Value) -> Result<(), String> {
        self.generation += 1;
        let e = self.list.get_mut(i).ok_or_else(|| crate::trq!("no plugin").into_owned())?;
        let schema = e.settings.schema.clone().ok_or_else(|| crate::trq!("this plugin has no settings").into_owned())?;
        let v = schema.validate(id, &v)?;
        if schema.field(id).is_some_and(|f| f.kind == Kind::Secret) {
            self.settings_store.save_secret(&e.key, id, v.as_str().unwrap_or_default())?;
        } else {
            self.settings_store.save_value(&e.key, id, &v)?;
        }
        e.settings.reload(&self.settings_store, &e.key, Some(schema));
        if let Some(l) = &e.loaded { push_settings(l, &e.settings) }
        Ok(())
    }

    /// The current value of setting `id` of the plugin with `key` (defaults filled in).
    pub fn setting(&self, key: &str, id: &str) -> Option<Value> { self.list.iter().find(|e| e.key == key)?.settings.value(id) }
    pub fn setting_bool(&self, key: &str, id: &str, fallback: bool) -> bool { self.setting(key, id).and_then(|v| v.as_bool()).unwrap_or(fallback) }

    /// Where a secret setting is kept.
    pub fn secret_at(&self, i: usize, id: &str) -> SecretAt { self.list.get(i).map_or(SecretAt::Unset, |e| e.settings.secret_at(id)) }

    /// A settings button of a native plugin was pressed: its message, if any. (Built-in plugins answer in the app, see the page.)
    pub fn settings_action(&mut self, i: usize, id: &str) -> Option<String> {
        self.generation += 1;
        let e = self.list.get(i)?;
        if !e.settings.schema.as_ref()?.field(id).is_some_and(|f| f.kind == Kind::Button) { return None; }
        let l = e.loaded.as_ref()?;
        let f = l.c.action?;
        let c = cs(id);
        l.take(f(l.inst, c.as_ptr()))
    }

    /// Settings of all listed plugins whose label, description or section title contains `q` (case-insensitive).
    pub fn search_settings(&self, q: &str) -> Vec<Hit> {
        let q = q.trim().to_lowercase();
        if q.is_empty() { return vec![]; }
        let mut out = vec![];
        for e in &self.list {
            let Some(s) = &e.settings.schema else { continue };
            for sec in &s.sections {
                for f in &sec.fields {
                    if [&f.label, &f.description, &sec.title, &e.name].iter().any(|t| t.to_lowercase().contains(&q)) {
                        out.push(Hit { key: e.key.clone(), plugin: e.name.clone(), label: f.label.clone(), description: f.description.clone() });
                    }
                }
            }
        }
        out
    }

    /// Test / sample runs: keep secrets in memory instead of the OS keyring.
    pub fn use_memory_secrets(&mut self) { self.settings_store.set_secrets(Box::new(ps::secret::MemSecrets::default())); }
}
