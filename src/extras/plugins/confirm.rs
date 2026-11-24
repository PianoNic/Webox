//! `ui.confirm`: a plugin asks the person before it does something (e.g. send a message for an AI client). The calling plugin
//! thread waits while Webox shows a dialog with the plugin's text and two buttons; the answer is `{"ok": true}` only when the
//! person pressed the confirm button. Closing the dialog, a timeout, the plugin being switched off or uninstalled all answer
//! `{"ok": false}`, so nothing waits forever and `stop` never hangs on a pending question.
use super::{lock, post, Ctx, Plugins};
use serde_json::{json, Value};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One question on screen or waiting for its turn (the window shows the oldest).
pub struct Confirm { pub id: u64, pub plugin: String, pub name: String, pub title: String, pub text: String, pub ok: String, pub cancel: String, answer: Sender<bool> }
impl Confirm {
    /// The person answered: the waiting plugin thread gets it.
    pub fn answer(self, ok: bool) { let _ = self.answer.send(ok); }
}
pub type Confirms = Arc<Mutex<Vec<Confirm>>>;

/// How long a question waits for an answer by default / at most (seconds).
const WAIT: u64 = 120;
const WAIT_MAX: u64 = 600;

/// `ui.confirm {"text", "title"?, "ok"?, "cancel"?, "timeout"?}` -> `{"ok": bool}` (plus `"timeout": true` when nobody answered).
/// Blocks the calling thread; refused on the UI thread (it would wait for itself).
pub(super) fn ui_confirm(c: &Ctx, a: &Value) -> Value {
    let Some(text) = a.get("text").and_then(Value::as_str) else { return json!({ "error": "text fehlt" }) };
    if c.ui_thread == Some(std::thread::current().id()) { return json!({ "error": "ui.confirm wartet auf eine Antwort: nur aus einem eigenen Thread des Plugins aufrufen" }); }
    let s = |k: &str, d: &str| a.get(k).and_then(Value::as_str).filter(|t| !t.trim().is_empty()).unwrap_or(d).to_string();
    let wait = Duration::from_secs(a.get("timeout").and_then(Value::as_u64).unwrap_or(WAIT).clamp(1, WAIT_MAX));
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (tx, rx) = channel();
    lock(&c.confirms).push(Confirm { id, plugin: c.plugin.clone(), name: c.name.clone(), title: s("title", &c.name), text: text.to_string(),
        ok: s("ok", "OK"), cancel: s("cancel", "Abbrechen"), answer: tx });
    post(c, |_| {}); // wakes the window to show it
    match rx.recv_timeout(wait) {
        Ok(ok) => json!({ "ok": ok }),
        Err(RecvTimeoutError::Disconnected) => json!({ "ok": false }),
        Err(RecvTimeoutError::Timeout) => {
            lock(&c.confirms).retain(|q| q.id != id);
            post(c, |_| {});
            json!({ "ok": false, "timeout": true })
        }
    }
}

impl Plugins {
    /// The question the window shows now (the oldest one).
    pub fn confirm_shown(&self) -> Option<(u64, String, String, String, String)> {
        lock(&self.confirms).first().map(|q| (q.id, q.title.clone(), format!("{}: {}", q.name, q.text), q.ok.clone(), q.cancel.clone()))
    }
    /// The person answered question `id`.
    pub fn confirm_answer(&self, id: u64, ok: bool) {
        let mut l = lock(&self.confirms);
        if let Some(i) = l.iter().position(|q| q.id == id) { l.remove(i).answer(ok) }
    }
}
