use super::*;

static LOCK: Mutex<()> = Mutex::new(());

fn de() -> Catalog {
    let z = pack("de").expect("the German pack (assets/cisco/i18n)");
    Catalog::from_pack(z).unwrap()
}

#[test]
fn embedded_de_known_strings() {
    if !crate::assets::cisco_assets::embedded() { return; } // needs the translation packs (docs/cisco-assets.md)
    let c = de();
    assert!(c.len() > 22000, "{}", c.len());
    assert_eq!(c.get_exact("QObject", "Active now"), Some("Jetzt aktiv"));
    assert_eq!(c.get_exact("CustomCalendar", "Today"), Some("Heute"));
    assert_eq!(c.get_exact("AVSettingsView", "Audio options"), Some("Audiooptionen"));
    assert_eq!(c.get_exact("NotifyMeAvailableToast", "Active 3 minutes ago"), Some("Vor 3 Minuten aktiv"));
    // Unknown context falls back to any context carrying the source.
    assert_eq!(c.get("NoSuchContext", "Active now"), Some("Jetzt aktiv"));
    assert_eq!(c.get("QObject", "definitely not a key"), None);
}

#[test]
fn numerus_rules() {
    // de/en: n==1 -> form 0, else 1.
    assert_eq!(numerus_index(&[1, 1], 1), 0);
    assert_eq!(numerus_index(&[1, 1], 0), 1);
    assert_eq!(numerus_index(&[1, 1], 5), 1);
    // Pseudo-Russian: n%10==1 && n%100!=11 -> 0 ; n%10 in 2..4 && n%100 not in 12..14 -> 1 ; else 2.
    let ru = [0x11, 1, 0xfd, 0x20 | 0x08 | 1, 11, 0xff, 0x14, 2, 4, 0xfd, 0x28 | 4, 12, 14];
    assert_eq!(numerus_index(&ru, 21), 0);
    assert_eq!(numerus_index(&ru, 11), 2);
    assert_eq!(numerus_index(&ru, 23), 1);
    assert_eq!(numerus_index(&ru, 13), 2);
    assert_eq!(numerus_index(&ru, 5), 2);
}

#[test]
fn real_qm_parser_matches_embedded_pack() {
    if !crate::assets::cisco_assets::embedded() { return; } // needs the translation packs (docs/cisco-assets.md)
    let p = std::path::Path::new(&std::env::var("LOCALAPPDATA").unwrap_or_default())
        .join("CiscoSparkLauncher/dynamic_dependencies/translations/spark-windows-client.de.qm");
    let Ok(bytes) = std::fs::read(&p) else { return }; // no installed pack on this machine
    let qm = parse_qm(&bytes).unwrap();
    assert_eq!(qm.messages.len(), 24110);
    assert_eq!(qm.numerus_rules, vec![1, 1]);
    let m = qm.messages.iter().find(|m| m.context == "CustomCalendar" && m.source == "Today").unwrap();
    assert_eq!(m.translations, vec![Some("Heute".to_string())]);
    let n = qm.messages.iter().find(|m| m.source == "Last updated %n days ago").unwrap();
    assert_eq!(n.translations.len(), 2);
    let full = Catalog::from_qm(&qm);
    let emb = de();
    assert_eq!(full.get("QObject", "Active now"), emb.get("QObject", "Active now"));
    assert_eq!(full.get_n("CxBasicAgentDelegate", "Last updated %n days ago", 2), Some("Zuletzt aktualisiert vor %n Tagen"));
}

#[test]
fn parse_qm_rejects_garbage() {
    assert!(parse_qm(b"nope").is_err());
    let mut d = QM_MAGIC.to_vec();
    d.extend([0x69, 0, 0, 0, 5, 3, 0, 0, 0, 9]); // truncated translation
    assert!(parse_qm(&d).is_err());
}

#[test]
fn all_packs_decode() {
    // Every selectable language except English has a pack.
    assert_eq!(PACK_CODES.len() + 1, LANGUAGES.len());
    for l in LANGUAGES {
        assert!(l.code == "en" || PACK_CODES.contains(&l.code), "{}", l.code);
    }
    if !crate::assets::cisco_assets::embedded() { return; } // needs the translation packs (docs/cisco-assets.md)
    for code in PACK_CODES {
        let c = Catalog::from_pack(pack(code).unwrap()).unwrap_or_else(|e| panic!("{code}: {e}"));
        assert!(c.len() >= 20, "{code} {}", c.len());
    }
}

#[test]
fn resolve_chain() {
    assert_eq!(resolve_code("de-CH"), "de");
    assert_eq!(resolve_code("de_DE.UTF-8"), "de");
    assert_eq!(resolve_code("en-GB"), "en_GB");
    assert_eq!(resolve_code("en-AU"), "en");
    assert_eq!(resolve_code("zh-Hant-TW"), "zh_TW");
    assert_eq!(resolve_code("zh-CN"), "zh");
    assert_eq!(resolve_code("es-419"), "es_MX");
    assert_eq!(resolve_code("fr-CA"), "fr_CA");
    assert_eq!(resolve_code("fr-CH"), "fr");
    assert_eq!(resolve_code("no-NO"), "nb");
    assert_eq!(resolve_code("xx"), "en");
    assert_eq!(resolve_code(""), "en");
}

#[test]
fn substitution() {
    assert_eq!(args("%1 of %2", &[&3, &"x"]), "3 of x");
    assert_eq!(args("%2 %1 %3", &[&"a", &"b"]), "b a %3");
    assert_eq!(args("Vor {{0}} Stunden", &[&5]), "Vor 5 Stunden");
    assert_eq!(args("100% sure %L1", &[&7]), "100% sure 7");
    assert_eq!(args("{{0}}1 Tag", &[&""]), "1 Tag");
    assert_eq!(args("ü %1 ä", &[&1]), "ü 1 ä");
}

#[test]
fn live_switching_and_plural() {
    if !crate::assets::cisco_assets::embedded() { return; } // needs the translation packs (docs/cisco-assets.md)
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_language("en");
    let g0 = generation();
    assert!(is_source_language());
    assert_eq!(tr("CustomCalendar", "Today"), "Today");
    assert_eq!(trn("RoomsListDelegate", "%n archived spaces", 3), "3 archived spaces");
    set_language("de");
    assert!(generation() > g0);
    assert_eq!(active_code(), "de");
    assert_eq!(tr("CustomCalendar", "Today"), "Heute");
    assert_eq!(tr("X", "no translation here"), "no translation here");
    assert_eq!(trn("RoomsListDelegate", "%n archived spaces", 3), "3 archivierte Bereiche");
    assert_eq!(crate::tr!("ConversationHelper", "%1 people", 4), "4 Personen");
    assert_eq!(crate::trq!("Active now"), "Jetzt aktiv");
    assert_eq!(crate::trn!("CxBasicAgentDelegate", "Last updated %n hours ago", 5), "Zuletzt aktualisiert vor 5 Stunden");
    set_language("de-CH");
    assert_eq!(active_code(), "de");
    let g1 = generation();
    set_language("de-CH");
    assert_eq!(generation(), g1);
    set_language("system");
    assert_eq!(state().read().unwrap().setting, "system");
    set_language("en");
}

#[test]
fn settings_roundtrip() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_language("fr");
    assert_eq!(active_code(), "fr");
    // Small packs only hold the shared strings; the rest stays English.
    assert_eq!(tr("NoCtx", "zzz"), "zzz");
    set_language("en");
}

#[test]
fn civil_dates() {
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(civil_from_days(days_from_civil(2026, 11, 13)), (2026, 11, 13));
    assert_eq!(civil_from_days(-1), (1969, 12, 31));
    assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    let p = parts(parse_iso_ms("2026-11-13T15:48:23Z").unwrap() / 1000);
    assert_eq!((p.year, p.month, p.day, p.hour, p.minute, p.second, p.weekday), (2026, 11, 13, 15, 48, 23, 4)); // Friday
}

#[test]
fn iso_parsing() {
    assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_iso_ms("1970-01-01T00:00:01.5Z"), Some(1500));
    assert_eq!(parse_iso_ms("1970-01-01T01:00:00+01:00"), Some(0));
    assert_eq!(parse_iso_ms("1970-01-01T00:00:00.123456-0100"), Some(3_600_123));
    assert_eq!(parse_iso_ms("1970-01-02"), Some(86_400_000));
    assert_eq!(parse_iso_ms("garbage"), None);
    assert_eq!(parse_iso_ms("2026-13-01T00:00:00Z"), None);
}

#[test]
fn date_formats() {
    let ts = parse_iso_ms("2026-11-13T15:48:00Z").unwrap();
    // CET (+1) in November.
    assert_eq!(format_date_time_in(ts, 3600, DateStyle::DateTime, "de"), "13.11.2026, 16:48");
    assert_eq!(format_date_time_in(ts, 3600, DateStyle::Date, "de"), "13.11.2026");
    assert_eq!(format_date_time_in(ts, 3600, DateStyle::DateTime, "en"), "11/13/2026, 4:48 PM");
    assert_eq!(format_date_time_in(ts, 3600, DateStyle::DateTime, "en_GB"), "13/11/2026, 16:48");
    assert_eq!(format_date_time_in(ts, 3600, DateStyle::DateTime, "fr"), "13/11/2026 16:48");
    assert_eq!(format_date_time_in(ts, 3600, DateStyle::DateTime, "sv"), "2026-11-13 16:48");
    assert_eq!(format_date_time_in(ts, 3600, DateStyle::DateTime, "zh_TW"), "2026/11/13 下午4:48");
    // Midnight / noon in 12 h clocks.
    let mid = parse_iso_ms("2026-01-05T00:05:00Z").unwrap();
    assert!(format_date_time_in(mid, 0, DateStyle::DateTime, "en").ends_with(" 12:05 AM"));
    let noon = parse_iso_ms("2026-01-05T12:00:00Z").unwrap();
    assert!(format_date_time_in(noon, 0, DateStyle::DateTime, "en").ends_with(" 12:00 PM"));
    // Zone shifts the date too.
    let late = parse_iso_ms("2026-12-31T23:30:00Z").unwrap();
    assert_eq!(format_date_time_in(late, 3600, DateStyle::Date, "de"), "01.01.2027");
    // Every locale produces something with the year.
    for l in LANGUAGES {
        assert!(format_date_time_in(ts, 0, DateStyle::DateTime, l.code).contains("2026"), "{}", l.code);
    }
}

// App name (naming.rs, docs/naming.md): the app is Webox, the service stays Webex.

#[test]
fn app_name_constant() {
    assert_eq!(APP_NAME, "Webox");
    assert_eq!(SERVICE_NAME, "Webex");
    assert_eq!(concat!(crate::app_name!(), " zusammen mit meinem Computer starten"), "Webox zusammen mit meinem Computer starten");
}

#[test]
fn app_name_english_sources() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_language("en");
    // App-meaning.
    assert_eq!(tr("GeneralTabWidget", "Start Webex when my computer starts"), "Start Webox when my computer starts");
    assert_eq!(tr("ThemeSelectorWidget", "Choose how you want Webex to look."), "Choose how you want Webox to look.");
    assert_eq!(tr("QObject", "Quit {{APP_NAME}}"), "Quit Webox");
    assert_eq!(tr("QObject", "Exit {{APP_NAME}}"), "Exit Webox");
    assert_eq!(tr("QObject", "Open Webex Settings"), "Open Webox Settings");
    assert_eq!(tr("AnyContext", "Open Webex Settings"), "Open Webox Settings"); // QObject keys match from any context
    assert_eq!(tr("CxBasicTitleBar", "Webex"), "Webox");
    // Service-meaning.
    assert_eq!(tr("QObject", "Invite to {{APP_NAME}}"), "Invite to Webex");
    assert_eq!(tr("QObject", "Learn more about {{APP_NAME}} encryption"), "Learn more about Webex encryption");
    assert_eq!(tr("QObject", "Webex Calling"), "Webex Calling");
    assert_eq!(tr("QObject", "Webex Account"), "Webex Account");
    assert_eq!(tr("QObject", "Webex"), "Webex");
    assert_eq!(tr("WebinarRegistration", "Webex"), "Webex");
    assert_eq!(tr("GeneralTabWidget", "You can let people know when you are active on Webex and let them see your custom status when you set one. You can also see their availability and statuses."),
        "You can let people know when you are active on Webex and let them see your custom status when you set one. You can also see their availability and statuses.");
    // A hard-coded app key only counts in its own context (or QObject).
    assert_eq!(tr("SomeOtherWidget", "Start Webex when my computer starts"), "Start Webex when my computer starts");
    assert_eq!(trn("QObject", "Restart {{APP_NAME}}", 1), "Restart Webox");
}

#[test]
fn app_name_german_pack() {
    if !crate::assets::cisco_assets::embedded() { return; } // needs the translation packs (docs/cisco-assets.md)
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_language("de");
    // App-meaning keys render with Webox.
    assert_eq!(tr("GeneralTabWidget", "Start Webex when my computer starts"), "Webox zusammen mit meinem Computer starten");
    assert_eq!(tr("GeneralTabWidget", "Connect Webex to Microsoft Outlook"), "Verbindung von Webox mit Microsoft Outlook herstellen");
    assert_eq!(tr("QObject", "Choose how you want {{APP_NAME}} to look."), "Legen Sie fest, wie Webox aussehen soll.");
    assert_eq!(tr("QObject", "{{APP_NAME}} uses artificial intelligence to enhance audio."), "Webox verwendet künstliche Intelligenz, um die Audioqualität zu verbessern.");
    assert_eq!(tr("QObject", "Exit {{APP_NAME}}"), "Webox beenden");
    assert_eq!(tr("QObject", "Open Webex Settings"), "Webox-Einstellungen öffnen");
    assert!(tr("SettingsKeyboardShortcutsPage", "Global lets you use Webex shortcuts when the app is minimized or in the background. Disabled shortcuts cannot be made global.")
        .contains("Webox-Tastenkombinationen verwenden"));
    // Service-meaning ones keep Webex.
    assert_eq!(tr("QObject", "Invite to {{APP_NAME}}"), "Zu Webex einladen");
    assert_eq!(tr("QObject", "Webex Account"), "Webex-Konto");
    assert_eq!(tr("QObject", "Webex Calling"), "Webex Calling");
    assert_eq!(tr("QObject", "Learn more about {{APP_NAME}} encryption"), "Weitere Informationen zur Webex-Verschlüsselung");
    assert!(tr("QObject", "We had a problem getting your latest Personal Room information from your {{APP_NAME}} site.").contains("Webex-Site"));
    assert!(tr("GeneralTabWidget", "You can let people know when you are active on Webex and let them see your custom status when you set one. You can also see their availability and statuses.").contains("in Webex aktiv"));
    // The reviewed lists name real pack keys (catches typos): every app key with a German text has "Webex" in it, and
    // only the plain "Webex" window titles (dropped from the pack because unchanged) may be missing.
    let c = de();
    for &(ctx, src) in APP_KEYS {
        match c.get_exact(ctx, src) {
            Some(t) => assert!(t.contains("Webex"), "{ctx} / {src}: {t}"),
            None => assert_eq!(src, "Webex", "app key not in the German pack: {ctx} / {src}"),
        }
    }
    // Placeholder sources: all in the pack, except the two whose German equals the source (dropped by gen_i18n.py).
    let identical = ["{{APP_NAME}}", "{{APP_NAME}} uses artificial intelligence to enhance audio. <a href='https://cisco.com/go/smart-audio-settings'>Learn more</a>"];
    for &src in APP_PLACEHOLDER_SOURCES {
        match c.get_exact("QObject", src) {
            Some(t) => assert!(t.contains("{{APP_NAME}}"), "{src}: {t}"),
            None => assert!(identical.contains(&src), "placeholder source not in the German pack: {src}"),
        }
    }
    set_language("en");
}

#[test]
fn webox_overlay_translates_own_strings() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Works without the official packs: the overlay is Webox's own and always embedded.
    set_language("de");
    assert_eq!(tr("QObject", "Couldn't send the message"), "Nachricht konnte nicht gesendet werden");
    assert_eq!(crate::trq!("Webex answered with error {{0}}.", 400), "Webex hat mit Fehler 400 geantwortet.");
    assert!(!is_source_language());
    assert_eq!(tr("X", "no translation here"), "no translation here");
    set_language("de-CH"); // de_CH -> de
    assert_eq!(tr("QObject", "Couldn't send the message"), "Nachricht konnte nicht gesendet werden");
    set_language("en");
    assert_eq!(tr("QObject", "Couldn't send the message"), "Couldn't send the message");
    assert!(is_source_language());
}

#[test]
fn webox_overlay_file_format() {
    let mut t = HashMap::new();
    overlay::parse("# comment\n\nHello\tHallo\r\nTwo\\tparts\tZwei\\nZeilen\nno tab here\n\tempty source\n", &mut t);
    assert_eq!(t.get("Hello"), Some(&"Hallo"));
    assert_eq!(t.get("Two\tparts"), Some(&"Zwei\nZeilen"));
    assert_eq!(t.len(), 2);
}
