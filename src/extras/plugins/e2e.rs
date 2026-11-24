//! `plugin_reacts_to_a_message_end_to_end` (`--ignored`, manual): loads a built plugin library the way Webox does, gives it
//! settings, fires `message.received` for a 1:1 message with history and prints what the plugin did (toasts, logs, sends).
//! Made for Auto-Antwort against a real provider; nothing reaches Webex (there is no client: `space.send` only fills the outbox,
//! and the test fails if anything lands there).
//!
//! - `WEBOX_E2E_PLUGIN`: path of the built library (copied into a temp config folder).
//! - `WEBOX_E2E_SETTINGS`: a plugin-settings file (`{"values": {...}}`) whose values are applied; `mode` is forced to "suggest".
//! - `WEBOX_E2E_SECRET`: optional `<plugin key>/<setting id>` of a secret to copy from the OS keyring (e.g.
//!   `dev.webox.plugins.auto-reply/token`); it stays in memory and is never printed.
//! - `WEBOX_E2E_WAIT`: seconds to wait for a toast (default 150).
use crate::extras::plugin_settings::secret::{OsSecrets, Secrets};
use serde_json::Value;

#[test]
#[ignore]
fn plugin_reacts_to_a_message_end_to_end() {
    let Some(lib) = std::env::var_os("WEBOX_E2E_PLUGIN").map(std::path::PathBuf::from) else { eprintln!("WEBOX_E2E_PLUGIN not set; skipping"); return };
    let dir = std::env::temp_dir().join(format!("webox-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("plugins")).unwrap();
    std::fs::copy(&lib, dir.join("plugins").join(lib.file_name().unwrap())).unwrap();
    let mut p = super::Plugins::with_root(&dir, dir.join("installed"));
    p.use_memory_secrets();
    let key = super::stem_of(&lib);
    let i = p.list.iter().position(|e| e.key == key).expect("listed");
    p.set_enabled(i, true);
    assert!(p.list[i].error.is_none(), "{:?}", p.list[i].error);
    eprintln!("loaded {} {}", p.list[i].name, p.list[i].version);

    if let Some(f) = std::env::var_os("WEBOX_E2E_SETTINGS") {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        for (id, val) in v["values"].as_object().into_iter().flatten() {
            if let Err(e) = p.set_setting(i, id, val.clone()) { eprintln!("setting {id}: {e}"); }
        }
    }
    p.set_setting(i, "mode", Value::from("suggest")).unwrap(); // never "send"
    if let Some((k, id)) = std::env::var("WEBOX_E2E_SECRET").ok().as_deref().and_then(|s| s.rsplit_once('/')) {
        let t = OsSecrets.get(k, id).expect("secret in the OS keyring");
        p.set_setting(i, id, Value::from(t)).unwrap();
    }

    // The 1:1 with Manuel: an older exchange, then "hoi" and "hoi 2" arriving live, as `Resp::Incoming` hands them on.
    let msg = |id: &str, author: &str, mine: bool, text: &str| crate::cloud::Message { id: id.into(), author: author.into(), mine, text: text.into(), ..Default::default() };
    let mut list = vec![msg("m0", "Manuel Wullschleger", false, "Kannst du mir das Protokoll von gestern schicken?"), msg("m1", "Ich", true, "Klar, kommt gleich")];
    for (id, text) in [("m2", "hoi"), ("m3", "hoi 2")] {
        list.push(msg(id, "Manuel Wullschleger", false, text));
        p.note_space("dm-manuel", &list);
        p.message_received("dm-manuel", id, "Manuel Wullschleger", text, false, true, false);
    }

    let wait = std::env::var("WEBOX_E2E_WAIT").ok().and_then(|w| w.parse().ok()).unwrap_or(150.0);
    let t0 = std::time::Instant::now();
    let (mut toasts, mut logs) = (vec![], vec![]);
    while t0.elapsed().as_secs_f64() < wait && toasts.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let mut o = p.out.lock().unwrap();
        assert!(o.sends_to.is_empty() && o.sends.is_empty(), "the plugin tried to send");
        toasts.append(&mut o.toasts);
        logs.append(&mut o.logs);
    }
    eprintln!("after {:.1} s", t0.elapsed().as_secs_f64());
    for l in &logs { eprintln!("log: {l}"); }
    for (n, t) in &toasts { eprintln!("toast: {n}: {t}"); }
    if toasts.is_empty() { eprintln!("no toast"); }
    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
}
