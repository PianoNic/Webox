//! Plugin translations (contract 1.6, no ABI change). Plugins write their texts in English. A plugin that subscribes to the
//! event `i18n.strings` gets `{"lang": "<active code>", "fallbacks": ["de_CH", "de"]}` once it starts and whenever the language
//! changes, and answers `{"strings": {"English source": "translation", ...}}`. Webox keeps that table per plugin and applies it
//! to everything it shows from the plugin: name, description, settings (section titles, labels, help, choice options),
//! `message.menu` / `space.menu` entries, `ui.dialog` (title, tab labels and hints, the switch) and toasts. A missing entry or
//! no answer shows the English text. Method `i18n.language` returns the same `{"lang", "fallbacks"}` for the plugin's own
//! dynamic text. Built-in plugins translate through Webox's own tables (`crate::trq!`).
use super::{lock, Entry, Plugins};
use crate::extras::plugin_settings as ps;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One plugin's table: English source -> translation. Shared with its host context (toasts and dialogs come from its threads).
pub(super) type Strings = Arc<Mutex<HashMap<String, String>>>;

/// The event and the method.
pub(super) const EVENT: &str = "i18n.strings";
pub(super) const METHOD: &str = "i18n.language";

/// The English texts of an entry, kept so they can be translated again after a language change.
#[derive(Default)]
pub struct Labels {
    name: String,
    description: String,
    schema: Option<ps::Schema>,
    /// A built-in plugin's schema builder (its texts go through `crate::trq!` when it runs).
    builtin_schema: Option<fn() -> String>,
    pub(super) strings: Strings,
}

/// `text` in the plugin's language: its table's entry, else the English source.
pub(super) fn tr(strings: &Strings, text: &str) -> String { lock(strings).get(text).cloned().unwrap_or_else(|| text.to_string()) }

/// The payload of `i18n.strings` and the result of `i18n.language`.
pub(super) fn language() -> Value { json!({ "lang": crate::i18n::active_code(), "fallbacks": crate::i18n::fallbacks() }) }

/// The table in an answer: `{"strings": {...}}`; anything else (or a non-string value) is left out.
pub(super) fn parse(answer: &str) -> HashMap<String, String> {
    let v: Value = serde_json::from_str(answer).unwrap_or(Value::Null);
    v.get("strings").and_then(Value::as_object).into_iter().flatten()
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).filter(|(k, v)| !k.is_empty() && !v.is_empty()).collect()
}

/// A copy of `schema` with every shown text through `t`.
fn translate_schema(schema: &ps::Schema, t: impl Fn(&str) -> String) -> ps::Schema {
    let mut s = schema.clone();
    for sec in &mut s.sections {
        sec.title = t(&sec.title);
        sec.description = t(&sec.description);
        for f in &mut sec.fields {
            f.label = t(&f.label);
            f.description = t(&f.description);
            match &mut f.kind {
                ps::Kind::Choice { options, .. } => for o in options { o.1 = t(&o.1); },
                ps::Kind::File { filter, .. } => *filter = t(filter),
                _ => {}
            }
        }
    }
    s
}

impl Labels {
    /// Remembers a built-in plugin's English name, description and schema builder.
    pub(super) fn builtin(name: &str, description: &str, schema: Option<fn() -> String>) -> Self {
        Labels { name: name.into(), description: description.into(), builtin_schema: schema, ..Default::default() }
    }
}

/// Remembers the English texts of a freshly loaded plugin (`Entry::name` / `description` / schema as the plugin gave them).
pub(super) fn capture(e: &mut Entry) {
    e.labels.name = e.name.clone();
    e.labels.description = e.description.clone();
    e.labels.schema = e.settings.schema.clone();
}

/// With `ask`, asks the loaded plugin for its table (only if it subscribes to `i18n.strings`; no answer = none), then shows its
/// texts in the active language.
pub(super) fn refresh(e: &mut Entry, ask: bool) {
    if let Some(l) = e.loaded.as_ref().filter(|_| ask) {
        let table = l.event(EVENT, &language().to_string()).map(|a| parse(&a)).unwrap_or_default();
        *lock(&e.labels.strings) = table;
    }
    relabel(e);
}

/// Writes the translated texts into the entry (what the plugins page, the settings search and the toasts read).
fn relabel(e: &mut Entry) {
    if e.builtin {
        if !e.labels.name.is_empty() { e.name = crate::trq!(e.labels.name.as_str()).into_owned(); }
        if !e.labels.description.is_empty() { e.description = crate::trq!(e.labels.description.as_str()).into_owned(); }
        if let Some(s) = e.labels.builtin_schema.and_then(|f| ps::Schema::parse(&f()).ok()) { e.settings.schema = Some(s); }
        return;
    }
    if e.labels.name.is_empty() { return; } // never loaded: listed by its file name / install record
    let strings = e.labels.strings.clone();
    let t = |s: &str| if s.is_empty() { String::new() } else { tr(&strings, s) };
    e.name = t(&e.labels.name);
    e.description = t(&e.labels.description);
    if let Some(s) = &e.labels.schema { e.settings.schema = Some(translate_schema(s, t)); }
}

impl Plugins {
    /// Once per frame: after a language change every plugin is asked for its table again and its texts are translated anew.
    pub fn sync_language(&mut self) {
        let g = crate::i18n::generation();
        if self.i18n_gen == g { return; }
        self.i18n_gen = g;
        for e in &mut self.list { let ask = e.enabled; refresh(e, ask); }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::load_in_process;
    use collabhost_plugin_api::{Host, Plugin};

    /// Subscribes to `i18n.strings` and answers a small German table; shows toasts at start and a menu on request.
    #[derive(Default)]
    struct Tr { lang: Option<String>, asked: Vec<String> }
    impl Plugin for Tr {
        const NAME: &'static str = "Counter"; const AUTHOR: &'static str = ""; const VERSION: &'static str = "";
        const DESCRIPTION: &'static str = "Counts messages";
        const EVENTS: &'static str = "i18n.strings message.menu";
        fn start(&mut self, h: Host) {
            self.lang = h.call("i18n.language", "{}");
            h.call("ui.toast", r#"{"text":"Counter reset"}"#);
            h.call("ui.toast", r#"{"text":"Not in the table"}"#);
        }
        fn on_event(&mut self, name: &str, payload: &str) -> Option<String> {
            match name {
                "i18n.strings" => { self.asked.push(payload.into()); Some(r#"{"strings":{"Counter":"Zähler","Counter reset":"Zähler zurückgesetzt","Count":"Zählen"}}"#.into()) }
                "message.menu" => Some(r#"{"items":[{"id":"a","label":"Count"},{"id":"b","label":"Untranslated"}]}"#.into()),
                _ => None,
            }
        }
    }

    /// A 1.6 plugin without the event.
    #[derive(Default)]
    struct Plain;
    impl Plugin for Plain {
        const NAME: &'static str = "Counter"; const AUTHOR: &'static str = ""; const VERSION: &'static str = ""; const DESCRIPTION: &'static str = "";
        const EVENTS: &'static str = "message.menu";
        fn start(&mut self, h: Host) { h.call("ui.toast", r#"{"text":"Counter reset"}"#); }
        fn on_event(&mut self, _: &str, _: &str) -> Option<String> { Some(r#"[{"id":"a","label":"Count"}]"#.into()) }
    }

    #[test]
    fn a_plugin_table_translates_what_webox_shows_and_misses_stay_english() {
        let (p, dir) = load_in_process::<Tr>(6);
        let e = p.list.last().unwrap();
        assert_eq!(e.name, "Zähler");
        assert_eq!(e.description, "Counts messages", "missing key: English");
        // asked before the start, with the active language
        let l = e.loaded.as_ref().unwrap();
        let tr = unsafe { &*(l.inst as *const Tr) };
        assert_eq!(tr.asked.len(), 1);
        let asked: serde_json::Value = serde_json::from_str(&tr.asked[0]).unwrap();
        assert_eq!(asked["lang"], crate::i18n::active_code());
        assert!(asked["fallbacks"].as_array().is_some_and(|f| f.iter().any(|c| c == crate::i18n::active_code())));
        let lang: serde_json::Value = serde_json::from_str(tr.lang.as_deref().unwrap()).unwrap();
        assert_eq!(lang, asked, "i18n.language answers what the event sends");
        assert_eq!(p.out.lock().unwrap().toasts, vec![("Zähler".to_string(), "Zähler zurückgesetzt".to_string()), ("Zähler".to_string(), "Not in the table".to_string())]);
        let labels: Vec<String> = p.message_menu(&serde_json::json!({})).into_iter().map(|m| m.label).collect();
        assert_eq!(labels, ["Zählen", "Untranslated"]);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_plugin_without_the_event_is_unchanged() {
        let (p, dir) = load_in_process::<Plain>(6);
        assert_eq!(p.list.last().unwrap().name, "Counter");
        assert_eq!(p.out.lock().unwrap().toasts, vec![("Counter".to_string(), "Counter reset".to_string())]);
        assert_eq!(p.message_menu(&serde_json::json!({}))[0].label, "Count");
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tables_and_schemas() {
        // empty keys or values and non-string values are left out
        assert_eq!(super::parse(r#"{"strings":{"":"x","a":"","Bad":5,"ok":"gut"}}"#).len(), 1);
        assert!(super::parse("no json").is_empty());
        let s = crate::extras::plugin_settings::Schema::parse(r#"{"sections":[{"title":"General","fields":[
            {"id":"mode","type":"choice","label":"Mode","options":[{"value":"a","label":"Fast"},{"value":"b","label":"Slow"}]}]}]}"#).unwrap();
        let t = super::translate_schema(&s, |x| match x { "General" => "Allgemein".into(), "Fast" => "Schnell".into(), o => o.to_string() });
        assert_eq!((t.sections[0].title.as_str(), t.sections[0].fields[0].label.as_str(), t.sections[0].fields[0].id.as_str()), ("Allgemein", "Mode", "mode"));
        let crate::extras::plugin_settings::Kind::Choice { options, .. } = &t.sections[0].fields[0].kind else { panic!("choice") };
        assert_eq!(options, &[("a".to_string(), "Schnell".to_string()), ("b".to_string(), "Slow".to_string())]);
    }
}
