//! `ui.dialog`: a plugin shows a read-only dialog: a title, tabs of rich text (spans as in `message.keep`), and optionally one
//! switch that shows each tab's second text instead (e.g. "Show differences" for a diff). Webox draws it
//! (ui/app_dialogs.rs); nothing goes back to the plugin. It does not block, so it may be called from `on_event` (the UI
//! thread). A new dialog replaces the one on screen; it closes with its X / Esc / "Close", or when its plugin is switched off.
use super::kept::{spans, SPAN_PLAIN};
use super::{post, Ctx};
use crate::cloud::Span;
use serde_json::{json, Value};

/// At most this many tabs.
const TABS_MAX: usize = 50;

/// One tab: its label (or a time the window writes as a local date and time), its text, the text while the switch is on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tab {
    pub label: String,
    /// Unix ms; > 0 and no `label`: the label is this time in local time ("25.11.2026, 14:05").
    pub time_ms: i64,
    pub text: Vec<Span>,
    /// Shown while the switch is on; empty = `text`.
    pub alt: Vec<Span>,
    /// Shown greyed out and cannot be picked; `hint` is its tooltip.
    pub disabled: bool,
    pub hint: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dialog {
    /// The key of the plugin that opened it (closed when that plugin is switched off).
    pub plugin: String,
    pub title: String,
    pub tabs: Vec<Tab>,
    /// The tab shown first.
    pub selected: usize,
    /// The switch: its label (None = no switch) and whether it starts on.
    pub switch: Option<(String, bool)>,
    /// Counts up with every dialog, so the window resets its tab / switch state for a new one.
    pub serial: u64,
}

/// `ui.dialog {"title", "tabs": [{"label"?, "time_ms"?, "text": spans|string, "alt"?: spans|string, "disabled"?, "hint"?}],
/// "selected"?, "switch"?: {"label", "on"?}}` -> `{}` (or `{"error"}` without a title or tabs).
pub(super) fn parse(plugin: &str, a: &Value) -> Result<Dialog, String> {
    let title = a.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).ok_or("title missing")?;
    let st = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().chars().take(120).collect::<String>();
    let tabs: Vec<Tab> = a.get("tabs").and_then(Value::as_array).into_iter().flatten().take(TABS_MAX).map(|t| Tab {
        label: st(t, "label"), time_ms: t.get("time_ms").and_then(Value::as_i64).unwrap_or(0),
        text: spans(t.get("text").unwrap_or(&Value::Null), SPAN_PLAIN), alt: spans(t.get("alt").unwrap_or(&Value::Null), SPAN_PLAIN),
        disabled: t.get("disabled").and_then(Value::as_bool).unwrap_or(false), hint: st(t, "hint"),
    }).collect();
    if tabs.is_empty() { return Err("tabs missing".into()); }
    let n = tabs.len();
    let mut selected = a.get("selected").and_then(Value::as_u64).map_or(n - 1, |i| (i as usize).min(n - 1));
    if tabs[selected].disabled { selected = tabs.iter().rposition(|t| !t.disabled).unwrap_or(selected); }
    let switch = a.get("switch").filter(|s| s.is_object()).map(|s| (st(s, "label"), s.get("on").and_then(Value::as_bool).unwrap_or(false))).filter(|s| !s.0.is_empty());
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    Ok(Dialog { plugin: plugin.to_string(), title: title.chars().take(120).collect(), tabs, selected, switch, serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed) })
}

/// The method: queued for the window (next frame).
pub(super) fn ui_dialog(c: &Ctx, a: &Value) -> Value {
    match parse(&c.plugin, a) {
        Ok(d) => { post(c, |o| o.dialog = Some(d)); json!({}) }
        Err(e) => json!({ "error": e }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extras::plugins::kept::{SPAN_ADDED, SPAN_REMOVED};

    #[test]
    fn a_dialog_needs_a_title_and_tabs() {
        assert!(parse("p", &json!({ "tabs": [{ "text": "a" }] })).is_err());
        assert!(parse("p", &json!({ "title": "T", "tabs": [] })).is_err());
        let d = parse("p", &json!({ "title": "Edit history", "switch": { "label": "Show differences", "on": true }, "tabs": [
            { "time_ms": 5, "disabled": true, "hint": "not logged", "text": "" },
            { "time_ms": 10, "text": "Hello world", "alt": [{ "text": "Hello " }, { "text": "world", "kind": "removed" }, { "text": "everyone", "kind": "added" }] },
            { "label": "Current", "text": "Hello everyone" } ] })).unwrap();
        assert_eq!((d.plugin.as_str(), d.title.as_str(), d.tabs.len(), d.selected), ("p", "Edit history", 3, 2), "the last tab is shown first");
        assert_eq!(d.switch, Some(("Show differences".to_string(), true)));
        assert_eq!(d.tabs[1].alt, [("Hello ".to_string(), SPAN_PLAIN), ("world".to_string(), SPAN_REMOVED), ("everyone".to_string(), SPAN_ADDED)]);
        assert!(d.tabs[0].disabled && d.tabs[0].text.is_empty());
        // A disabled tab is never the one shown.
        assert_eq!(parse("p", &json!({ "title": "T", "selected": 0, "tabs": [{ "disabled": true, "text": "a" }, { "text": "b" }] })).unwrap().selected, 1);
        assert_eq!(parse("p", &json!({ "title": "T", "selected": 9, "tabs": [{ "text": "a" }, { "text": "b" }] })).unwrap().selected, 1);
        assert_eq!(parse("p", &json!({ "title": "T", "switch": { "label": "" }, "tabs": [{ "text": "a" }] })).unwrap().switch, None);
    }
}
