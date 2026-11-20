//! Offline tests: the schema as plugin-api writes it, validation, persistence (keyring and file fallback, in memory), and a native
//! plugin (the in-tree Slash Commands) told about every change, before start, and asked for a button action.
use super::*;
use collabhost_plugin_api::Schema as Builder;
use serde_json::json;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("webox-settings-test-{tag}-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn full() -> Schema {
    Schema::parse(&Builder::new().toggle("b", "B", "d", true).section("Mehr", "x").text("t", "T", "", "hi").multiline("m", "M", "", "a\nb")
        .password("p", "P", "").number("n", "N", "", 50.0, 0.0, 10.0, 0.5).choice("c", "C", "", "nope", &[("x", "X"), ("y", "Y")])
        .button("go", "Go", "").to_json()).unwrap()
}

#[test]
fn parses_what_plugin_api_writes() {
    let s = full();
    assert_eq!(s.sections.len(), 2);
    assert_eq!((s.sections[0].title.as_str(), s.sections[1].title.as_str()), ("", "Mehr"));
    assert_eq!(s.fields().count(), 7);
    // A default outside the range is clamped, an unknown choice default falls back to the first option.
    assert_eq!(s.field("n").unwrap().kind, Kind::Number { default: 10.0, min: 0.0, max: 10.0, step: 0.5 });
    assert!(matches!(&s.field("c").unwrap().kind, Kind::Choice { default, .. } if default == "x"));
    assert_eq!(s.field("p").unwrap().kind, Kind::Secret);
}

#[test]
fn schema_edge_cases() {
    assert!(Schema::parse("[]").is_err() && Schema::parse("{}").is_err() && Schema::parse("x").is_err());
    let flat = Schema::parse(r#"{"fields":[{"type":"bool","id":"a","label":"A"},{"type":"bool","id":"a","label":"again"},
        {"type":"rocket","id":"r","label":"R"},{"type":"bool","id":"bad id","label":"B"},{"type":"number","id":"n","label":"N","min":5,"max":1},
        {"type":"choice","id":"c","label":"C","options":[]},{"type":"string","id":"s","label":""}]}"#).unwrap();
    assert_eq!((flat.fields().count(), flat.skipped), (1, 6));
    let many: Vec<String> = (0..100).map(|i| format!(r#"{{"type":"bool","id":"f{i}","label":"F"}}"#)).collect();
    assert_eq!(Schema::parse(&format!(r#"{{"fields":[{}]}}"#, many.join(","))).unwrap().fields().count(), schema::MAX_FIELDS);
}

#[test]
fn validation() {
    let s = full();
    assert!(s.validate("b", &json!(false)).is_ok() && s.validate("b", &json!("yes")).is_err());
    assert!(s.validate("t", &json!("one line")).is_ok() && s.validate("t", &json!("two\nlines")).is_err());
    assert!(s.validate("t", &json!("x".repeat(schema::TEXT_MAX + 1))).is_err());
    assert!(s.validate("m", &json!("two\nlines")).is_ok());
    assert!(s.validate("n", &json!(2.5)).is_ok());
    assert!(s.validate("n", &json!(2.25)).unwrap_err().contains("Schritten"));
    assert!(s.validate("n", &json!(11)).unwrap_err().contains("zwischen"));
    assert!(s.validate("c", &json!("y")).is_ok() && s.validate("c", &json!("z")).is_err());
    assert!(s.validate("go", &json!(true)).is_err() && s.validate("missing", &json!(1)).is_err());
}

#[test]
fn effective_values_fall_back_to_defaults() {
    let s = full();
    let stored: Map<String, Value> = serde_json::from_value(json!({ "b": "not a bool", "t": "kept", "n": 3, "c": "gone" })).unwrap();
    let e = s.effective(&stored, &Map::new());
    assert_eq!(Value::Object(e), json!({ "b": true, "t": "kept", "m": "a\nb", "p": "", "n": 3.0, "c": "x" }));
}

#[test]
fn values_and_secrets_persist() {
    let d = tmp("store");
    let s = full();
    let mut st = Store::new(&d);
    st.set_secrets(Box::new(secret::MemSecrets::default()));
    st.save_value("dev.webox.x", "t", &json!("saved")).unwrap();
    assert_eq!(st.save_secret("dev.webox.x", "p", "s3cret").unwrap(), SecretAt::Keyring);
    let file = std::fs::read_to_string(d.join("plugin-settings/dev.webox.x.json")).unwrap();
    assert!(!file.contains("s3cret"), "a keyring secret is not written to the file");
    let l = st.load("dev.webox.x", &s);
    assert_eq!((l.values["t"].as_str(), l.secrets["p"].as_str(), l.in_file.len()), (Some("saved"), Some("s3cret"), 0));
    // No keyring: kept in the file and marked.
    st.set_secrets(Box::new(secret::MemSecrets { fail: true, ..Default::default() }));
    assert_eq!(st.save_secret("dev.webox.x", "p", "other").unwrap(), SecretAt::File);
    let l = st.load("dev.webox.x", &s);
    assert_eq!((l.secrets["p"].as_str(), l.in_file.clone()), (Some("other"), vec!["p".to_string()]));
    assert_eq!(st.save_secret("dev.webox.x", "p", "").unwrap(), SecretAt::Unset);
    assert!(st.load("dev.webox.x", &s).secrets.is_empty());
    // Odd keys still make a safe file name.
    st.save_value("../evil key", "t", &json!("x")).unwrap();
    assert!(d.join("plugin-settings/.._evil_key.json").is_file());
}

/// Touches the real OS keyring (a throw-away entry, removed again): `cargo test -- --ignored os_keyring_roundtrip`.
#[test]
#[ignore]
fn os_keyring_roundtrip() {
    use secret::Secrets;
    let mut k = secret::OsSecrets;
    let (plugin, id) = ("dev.webox.keyring-test", format!("probe-{}", std::process::id()));
    if k.set(plugin, &id, "v\u{e4}lue").is_err() { eprintln!("no keyring here; skipping"); return; }
    assert_eq!(k.get(plugin, &id).as_deref(), Some("v\u{e4}lue"));
    k.delete(plugin, &id);
    assert_eq!(k.get(plugin, &id), None);
}

#[test]
fn native_plugin_gets_its_settings() {
    let name = format!("{}collabhost_slash_commands.{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_EXTENSION);
    let Some(built) = crate::extras::plugin_repo::tests::built(&name) else { eprintln!("build the workspace first; skipping"); return };
    let d = tmp("native");
    std::fs::create_dir_all(d.join("plugins")).unwrap();
    std::fs::copy(&built, d.join("plugins").join(&name)).unwrap();
    let open = || { let mut p = crate::extras::plugins::Plugins::with_root(&d, d.join("data")); p.use_memory_secrets(); p };
    let mut p = open();
    let i = p.list.iter().position(|e| e.name == "Slash Commands").expect("listed");
    let s = p.list[i].settings.schema.clone().expect("the plugin declares settings");
    assert!(s.field("smiley").is_some() && s.field("shrug_text").is_some());
    p.set_enabled(i, true);
    assert_eq!(p.on_send("ok :)").as_deref(), Some("ok \u{1f642}"));
    // A change reaches the running plugin at once.
    p.set_setting(i, "smiley", json!(false)).unwrap();
    assert_eq!(p.on_send("ok :)").as_deref(), Some("ok :)"));
    p.set_setting(i, "shrug_text", json!("meh")).unwrap();
    assert_eq!(p.command("/shrug").as_deref(), Some("meh"));
    // Refused values change nothing.
    assert!(p.set_setting(i, "smiley", json!("no")).is_err());
    assert!(p.set_setting(i, "shrug_text", json!("a\nb")).is_err());
    assert_eq!(p.command("/shrug").as_deref(), Some("meh"));
    assert_eq!(p.settings_action(i, "reset_count").as_deref(), Some("Zähler zurückgesetzt"));
    assert_eq!(p.settings_action(i, "smiley"), None, "only buttons are actions");
    assert!(p.search_settings("shrug").iter().any(|h| h.label == "Text für /shrug"));
    drop(p);
    // Saved, and handed over before the next start.
    let p = open();
    assert_eq!(p.command("/shrug").as_deref(), Some("meh"));
    assert_eq!(p.on_send(":)").as_deref(), Some(":)"));
}

#[test]
fn accent_theme_plugin_follows_theme_and_settings() {
    use crate::ui::theme::{accents, Th, ThemeId};
    let name = format!("{}collabhost_pride_theme.{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_EXTENSION);
    let Some(built) = crate::extras::plugin_repo::tests::built(&name) else { eprintln!("build the workspace first; skipping"); return };
    let d = tmp("pride");
    std::fs::create_dir_all(d.join("plugins")).unwrap();
    std::fs::copy(&built, d.join("plugins").join(&name)).unwrap();
    let mut p = crate::extras::plugins::Plugins::with_root(&d, d.join("data"));
    let i = p.list.iter().position(|e| e.name == "Pride Theme").expect("listed");
    assert!(p.is_design(i), "an accent theme counts as a design plugin");
    let (dark, light) = (Th::of(ThemeId::Dark), Th::of(ThemeId::Light));
    assert!(p.accents(&accents::info(&dark)).is_none(), "off: no accents");
    p.set_enabled(i, true);
    let a = p.accents(&accents::info(&dark)).expect("on: accents");
    let b = p.accents(&accents::info(&light)).unwrap();
    assert_ne!(a.link, b.link, "lightness follows the base theme");
    assert!(a.gradient_len == 6, "rainbow gradient by default");
    // The settings change the colours at once.
    p.set_setting(i, "flag", json!("bi")).unwrap();
    p.set_setting(i, "gradient", json!(false)).unwrap();
    let c = p.accents(&accents::info(&dark)).unwrap();
    assert!(c.link != a.link && c.gradient_len == 0);
    assert!(p.set_setting(i, "flag", json!("unknown-flag")).is_err());
    // Applying keeps backgrounds and body text.
    let mut t = dark;
    let r = accents::apply(&mut t, &c);
    assert_eq!((t.card, t.text1), (dark.card, dark.text1));
    assert!(r.gradient.is_empty());
}

#[test]
fn pride_decorations_keep_text_readable_on_every_theme() {
    use crate::ui::theme::{accents, decor, Th, ThemeId};
    use collabhost_plugin_api::color::{contrast, mix};
    let name = format!("{}collabhost_pride_theme.{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_EXTENSION);
    let Some(built) = crate::extras::plugin_repo::tests::built(&name) else { eprintln!("build the workspace first; skipping"); return };
    let d = tmp("pride-decor");
    std::fs::create_dir_all(d.join("plugins")).unwrap();
    std::fs::copy(&built, d.join("plugins").join(&name)).unwrap();
    let mut p = crate::extras::plugins::Plugins::with_root(&d, d.join("data"));
    let i = p.list.iter().position(|e| e.name == "Pride Theme").expect("listed");
    assert!(p.decor(&accents::info(&Th::of(ThemeId::Dark))).is_none(), "off: nothing");
    p.set_enabled(i, true);
    let pack = |c: eframe::egui::Color32| u32::from_be_bytes([c.r(), c.g(), c.b(), 0xFF]);
    for flag in ["rainbow", "progress", "trans", "bi", "lesbian", "pan", "nonbinary", "ace"] {
        p.set_setting(i, "flag", json!(flag)).unwrap();
        for id in ThemeId::EVERY {
            let t = Th::of(id);
            let dec = decor::from_api(&p.decor(&accents::info(&t)).expect("decorations"));
            assert!(!dec.stripes.is_empty() && dec.flags != 0);
            for (w, base) in [(&dec.window, t.win), (&dec.list, t.card), (&dec.content, t.card)] {
                let Some(w) = w else { continue };
                for s in &w.stops {
                    let b = mix(pack(base), pack(*s), w.opacity as f64);
                    for txt in [t.text1, t.text2] {
                        let need = 4.5f64.min(contrast(pack(txt), pack(base)));
                        assert!(contrast(pack(txt), b) >= need - 0.05, "{flag} {id:?}: text over the wash");
                    }
                }
            }
        }
    }
    // a decoration switched off in the settings is off in Webox at once
    p.set_setting(i, "rail_stripe", json!(false)).unwrap();
    let dec = decor::from_api(&p.decor(&accents::info(&Th::of(ThemeId::Dark))).unwrap());
    assert!(dec.flags & collabhost_plugin_api::deco::RAIL_STRIPE == 0 && dec.flags & collabhost_plugin_api::deco::SEND_BURST != 0);
}

#[test]
fn builtin_plugin_uses_the_same_schema() {
    let d = tmp("builtin");
    let mut p = crate::extras::plugins::Plugins::with_root(&d, d.join("data"));
    let key = crate::extras::gif_extras::KEY;
    let i = p.list.iter().position(|e| e.key == key).unwrap();
    assert!(p.list[i].settings.schema.as_ref().is_some_and(|s| s.field("stars_chat").is_some()));
    assert!(p.setting_bool(key, "stars_chat", false), "default on");
    p.set_setting(i, "stars_chat", json!(false)).unwrap();
    assert!(!crate::extras::plugins::Plugins::with_root(&d, d.join("data")).setting_bool(key, "stars_chat", true), "saved");
}
