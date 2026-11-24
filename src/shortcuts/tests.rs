use super::*;

fn c(s: &str) -> Combo { Combo::parse(s).unwrap_or_else(|| panic!("parse {s}")) }

/// The table marks `id` UNSUPPORTED in its Windows (`qt`) column.
fn unsupported(id: &str) -> bool {
    let v: serde_json::Value = serde_json::from_str(json().trim_start_matches('\u{feff}')).unwrap();
    v["keyboardShortcutGroups"].as_array().into_iter().flatten().flat_map(|g| g["shortcuts"].as_array().into_iter().flatten())
        .any(|s| s["command"] == id && s["keyCombination"]["qt"] == "UNSUPPORTED")
}

#[test]
fn parses_qt_key_strings() {
    assert_eq!(c("Ctrl+Shift+N"), Combo { ctrl: true, alt: false, shift: true, key: Key::N });
    assert_eq!(c("Alt+LeftArrow").key, Key::ArrowLeft);
    assert_eq!(c("Alt+DownArrow").key, Key::ArrowDown);
    assert_eq!(c("PgUp"), Combo { ctrl: false, alt: false, shift: false, key: Key::PageUp });
    assert_eq!(c("Shift+PgDn").key, Key::PageDown);
    assert_eq!(c("Ctrl++").key, Key::Plus);
    assert_eq!(c("Ctrl+-").key, Key::Minus);
    assert_eq!(c("Ctrl+=").key, Key::Equals);
    assert_eq!(c("Ctrl+,").key, Key::Comma);
    assert_eq!(c("Ctrl+/").key, Key::Slash);
    assert_eq!(c("Ctrl+Shift+Spacebar").key, Key::Space);
    assert_eq!(c("Shift+Return").key, Key::Enter);
    assert_eq!(c("Ctrl+F6").key, Key::F6);
    assert_eq!(c("F1").key, Key::F1);
    assert_eq!(c("Alt+Shift+0").key, Key::Num0);
    assert_eq!(c("Ctrl+Shift+Alt+H"), c("Ctrl+Alt+Shift+H")); // the JSON mixes modifier orders
    assert_eq!(Combo::parse("UNSUPPORTED"), None);
    assert_eq!(Combo::parse("Command+DownArrow"), None); // the macOS strings of Minimize / Maximize
    assert_eq!(Combo::parse(""), None);
    assert_eq!(Combo::parse("Ctrl+"), None);
}

#[test]
fn round_trips_and_displays_in_german() {
    for s in ["Ctrl+Alt+Shift+D", "Alt+LeftArrow", "Ctrl+Shift+Spacebar", "PgUp", "Ctrl++", "F1", "Ctrl+Shift+Return"] { assert_eq!(c(s).qt(), s); assert_eq!(c(&c(s).qt()), c(s)); }
    #[cfg(not(target_os = "macos"))] // macOS: keys_macos.rs
    {
        assert_eq!(c("Ctrl+Shift+N").display(), "Strg+Umschalt+N");
        assert_eq!(c("Ctrl+Alt+Shift+D").display(), "Strg+Alt+Umschalt+D");
        assert_eq!(c("PgUp").display(), "Bild↑");
    }
}

#[test]
fn matches_events_exactly_but_symbols_ignore_shift() {
    let m = |ctrl: bool, alt: bool, shift: bool| Modifiers { ctrl, alt, shift, command: ctrl, ..Default::default() };
    assert!(c("Ctrl+N").matches(Key::N, &m(true, false, false)));
    assert!(!c("Ctrl+N").matches(Key::N, &m(true, false, true))); // Ctrl+Shift+N is another command
    assert!(!c("Ctrl+N").matches(Key::N, &m(true, true, false)));
    assert!(c("Ctrl+Shift+N").matches(Key::N, &m(true, false, true)));
    assert!(c("Ctrl+/").matches(Key::Slash, &m(true, false, true))); // "/" is Shift+7 on a German keyboard
    assert!(c("Ctrl++").matches(Key::Plus, &m(true, false, true)));
    assert!(!c("Ctrl+Shift+N").matches(Key::N, &m(true, false, false)));
    assert!(c("F1").matches(Key::F1, &m(false, false, false)));
    assert!(Combo::from_event(Key::Q, &m(true, false, false)).capturable());
    assert!(!Combo::from_event(Key::Q, &m(false, false, false)).capturable());
    assert!(Combo::from_event(Key::F5, &m(false, false, false)).capturable());
}

#[test]
fn registry_is_the_json_with_unique_commands_and_german_labels() {
    let r = registry();
    assert!(r.cmds.len() > 160, "{} commands", r.cmds.len());
    let mut ids: Vec<_> = r.cmds.iter().map(|c| c.id).collect();
    ids.sort();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate command ids");
    for c in &r.cmds {
        assert!(table::spec(c.id).is_some(), "{} has no table entry", c.id);
        assert!(!c.label.is_empty() && c.label != c.id, "{} has no German label", c.id);
        assert_eq!(r.index(c.id).map(|i| r.cmds[i].id), Some(c.id));
    }
    let mut labels: Vec<_> = r.cmds.iter().map(|c| c.label).collect();
    labels.sort();
    let n = labels.len();
    labels.dedup();
    assert_eq!(labels.len(), n, "two commands share a description");
    // Windows column, flags.
    let get = |id: &str| &r.cmds[r.index(id).unwrap()];
    assert_eq!(get("Search").default, Some(c("Ctrl+F")));
    assert_eq!(get("HistoryBack").default, Some(c("Alt+Left")));
    assert_eq!(get("ZoomIn").default, Some(c("Ctrl++")));
    assert!(get("OpenHome").default.is_none() && unsupported("OpenHome")); // UNSUPPORTED: no default on Windows
    assert!(get("Minimize").default.is_none()); // macOS string in the table
    assert!(!get("Quit").customizable && get("Search").customizable);
    assert!(get("AnswerCall").default_global && get("AnswerCall").can_be_global);
    assert!(!get("EndCall").default_global && get("EndCall").can_be_global);
    assert_eq!(get("ShortcutMenu").label, "Suche nach Tastenkombination öffnen");
    assert_eq!(get("OpenSpaces").group, Group::General);
    assert_eq!(get("ToggleBold").group, Group::Formatting);
    assert_eq!(get("DeclineCall").group, Group::Calling);
    assert!(r.cmds.iter().all(|c| c.default.is_some() || unsupported(c.id) || matches!(c.id, "Minimize" | "Maximize")));
}

/// Default keys that two different commands share. Only combinations used in different contexts (meeting-only vs. messaging) may do so.
#[test]
fn default_duplicates_are_known() {
    let r = registry();
    let mut seen: HashMap<Combo, &str> = HashMap::new();
    let mut dups = Vec::new();
    for c in &r.cmds { if let Some(d) = c.default { if let Some(o) = seen.insert(d, c.id) { dups.push((o, c.id)); } } }
    eprintln!("shared defaults: {dups:?}");
    // The page's own commands (General / Messaging / Formatting) never share a key with each other.
    let own: Vec<_> = dups.iter().filter(|(a, b)| {
        let (a, b) = (&r.cmds[r.index(a).unwrap()], &r.cmds[r.index(b).unwrap()]);
        !matches!(a.kind, table::Kind::Native) && !matches!(b.kind, table::Kind::Native) && a.group != Group::Calling && b.group != Group::Calling && a.group != Group::Developer && b.group != Group::Developer
    }).collect();
    assert!(own.is_empty(), "{own:?}");
}

fn state_with(ov: &[(&str, Ov)]) -> (Vec<Resolved>, &'static [Command]) {
    let r = registry();
    let map: HashMap<usize, Ov> = ov.iter().map(|(id, o)| (r.index(id).unwrap(), *o)).collect();
    (resolve(&r.cmds, &map), &r.cmds)
}

#[test]
fn conflict_detection_and_fates() {
    let r = registry();
    let i = |id: &str| r.index(id).unwrap();
    let (res, cmds) = state_with(&[]);
    // Ctrl+Shift+N is CreateSpace: assigning it to another customizable command conflicts, the other is disabled.
    let cf = find_conflict(cmds, &res, i("ShowAll"), c("Ctrl+Shift+N")).unwrap();
    assert_eq!((cf.other, cf.fate), (i("CreateSpace"), Fate::Disable));
    assert!(conflict_message(cmds, &cf, c("Ctrl+Shift+N"), false).starts_with("Strg+Umschalt+N wird von Bereich erstellen verwendet. Wenn Sie fortfahren möchten, wird die Tastenkombination für Bereich erstellen deaktiviert."));
    assert!(conflict_message(cmds, &cf, c("Ctrl+Shift+N"), true).contains("Wenn Sie die Werte auf die Standardwerte zurücksetzen"));
    // No conflict for a free combination, none with itself.
    assert!(find_conflict(cmds, &res, i("ShowAll"), c("Ctrl+Alt+Shift+F9")).is_none());
    assert!(find_conflict(cmds, &res, i("CreateSpace"), c("Ctrl+Shift+N")).is_none());
    // A command with a fixed key (Quit, not customizable) can only be refused.
    let q = find_conflict(cmds, &res, i("ShowAll"), c("Ctrl+Q")).unwrap();
    assert_eq!((q.other, q.fate), (i("Quit"), Fate::Locked));
    // The other command is on a custom key whose default is free: it goes back to that default.
    let (res, cmds) = state_with(&[("ShowAll", Ov { key: Some(Some(c("Ctrl+Alt+Shift+F9"))), global: None })]);
    let cf = find_conflict(cmds, &res, i("ShowDrafts"), c("Ctrl+Alt+Shift+F9")).unwrap();
    assert_eq!((cf.other, cf.fate), (i("ShowAll"), Fate::ResetTo(c("Alt+Shift+A"))));
    assert!(conflict_message(cmds, &cf, c("Ctrl+Alt+Shift+F9"), false).ends_with("auf Strg+Alt+Umschalt+F9 zurückgesetzt.") || conflict_message(cmds, &cf, c("Ctrl+Alt+Shift+F9"), false).contains("auf Alt+Umschalt+A zurückgesetzt."));
}

#[test]
fn overrides_resolve_and_a_taken_default_disables_the_command() {
    let r = registry();
    let i = |id: &str| r.index(id).unwrap();
    // CreateSpace moved away, Quit removed, ZoomIn global flag.
    let (res, _) = state_with(&[("CreateSpace", Ov { key: Some(Some(c("Ctrl+Alt+Shift+F9"))), global: None }), ("ReloadSpace", Ov { key: Some(None), global: None }),
        ("EndCall", Ov { key: None, global: Some(true) })]);
    assert_eq!(res[i("CreateSpace")].key, Some(c("Ctrl+Alt+Shift+F9")));
    assert!(res[i("CreateSpace")].custom);
    assert_eq!(res[i("ReloadSpace")].key, None);
    assert!(res[i("EndCall")].global && res[i("EndCall")].key.is_some()); // canBeGlobal, switched on by the user
    assert!(!res[i("Search")].global);
    assert!(res[i("AnswerCall")].global); // global by default
    // Another command moved onto Ctrl+F: Search's default is taken -> Search is disabled and says by whom.
    let (res, _) = state_with(&[("ShowAll", Ov { key: Some(Some(c("Ctrl+F"))), global: None })]);
    assert_eq!(res[i("Search")].key, None);
    assert_eq!(res[i("Search")].default_taken_by, Some(i("ShowAll")));
    // A command that cannot be global never is, whatever the settings say.
    let (res, _) = state_with(&[("Search", Ov { key: None, global: Some(true) })]);
    assert!(!res[i("Search")].global);
}

#[test]
fn settings_json_round_trip() {
    let r = registry();
    let mut st = State::default();
    st.load(&json!({ "shortcuts": { "ToggleBold": { "key": "Ctrl+Shift+B" }, "ReloadSpace": { "key": "" }, "EndCall": { "global": true }, "Nope": { "key": "Ctrl+X" }, "Search": { "key": "garbage" } } }));
    let i = |id: &str| r.index(id).unwrap();
    assert_eq!(st.res[i("ToggleBold")].key, Some(c("Ctrl+Shift+B")));
    assert_eq!(st.res[i("ReloadSpace")].key, None);
    assert!(st.res[i("EndCall")].global);
    assert_eq!(st.res[i("Search")].key, Some(c("Ctrl+F"))); // unparsable override ignored
    let j = st.to_json();
    assert_eq!(j["ToggleBold"]["key"], "Ctrl+Shift+B");
    assert_eq!(j["ReloadSpace"]["key"], "");
    assert_eq!(j["EndCall"]["global"], true);
    assert!(j.get("Nope").is_none());
}

#[test]
fn digits_match_physically_and_exact_shift_wins() {
    let m = |ctrl: bool, alt: bool, shift: bool| Modifiers { ctrl, alt, shift, command: ctrl, ..Default::default() };
    // Alt+Shift+1 on DE / CH: logical "!", physical 1.
    assert!(c("Alt+Shift+1").matches_at(Key::Exclamationmark, Some(Key::Num1), &m(false, true, true), true));
    assert!(!c("Alt+Shift+2").matches_at(Key::Exclamationmark, Some(Key::Num1), &m(false, true, true), true));
    assert!(c("Alt+Shift+0").matches_at(Key::Equals, Some(Key::Num0), &m(false, true, true), false));
    // Ctrl+Shift+0 on DE is "=" with Shift: exact for Superscript, only loose for Subscript.
    let (eq, sh) = (Key::Equals, m(true, false, true));
    assert!(c("Ctrl+Shift+=").matches_at(eq, Some(Key::Num0), &sh, false));
    assert!(!c("Ctrl+=").matches_at(eq, Some(Key::Num0), &sh, false) && c("Ctrl+=").matches_at(eq, Some(Key::Num0), &sh, true));
    assert!(!c("Ctrl+Shift+0").matches_at(eq, Some(Key::Num0), &m(true, true, true), true));
}

#[test]
fn take_key_drops_the_text_of_alt_keys_only() {
    let key = |modifiers| egui::Event::Key { key: Key::K, physical_key: Some(Key::K), pressed: true, repeat: false, modifiers };
    let text = || egui::Event::Text("k".into());
    let mut ev = vec![key(Modifiers::ALT), text(), text()];
    take_key(&mut ev, 0);
    assert_eq!(ev.len(), 1);
    let mut ev = vec![key(Modifiers::NONE), text()];
    take_key(&mut ev, 0);
    assert!(matches!(ev.as_slice(), [egui::Event::Text(_)]));
}
