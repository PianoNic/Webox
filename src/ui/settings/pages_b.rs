//! Settings pages, part 2: Benachrichtigungen (+ Nachrichten / Anrufe / Meetings), Meetings (+ Beitritt / Layout / Lobby),
//! Anrufe (+ Dock / Sperrliste), Geräte, Zubehör, App-Seitenleiste. German labels from the pack where it has them.
//! Keys shared with notify.rs: notif_desktop, notif_preview, notif_mentions_only, notif_taskbar_flash, close_to_tray,
//! meeting_toast_enabled, meeting_toast_minutes.
use super::kit::{self, section};
use super::pages_a::text_note;
use crate::*;

const DAYS: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];

fn times() -> Vec<(String, String)> { (0..48).map(|i| { let s = format!("{:02}:{:02}", i / 2, (i % 2) * 30); (s.clone(), s) }).collect() }

impl App {
    // Benachrichtigungen.
    pub(super) fn page_notifications(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, "Desktopbenachrichtigungen", "", true, |ui| { // keys shared with notify.rs
            self.pcheck(ui, t, "notif_desktop", true, "Desktopbenachrichtigungen anzeigen");
            self.pcheck(ui, t, "notif_preview", true, "Nachrichtenvorschau anzeigen"); // pack: "Show Message Previews"
            self.pcheck(ui, t, "notif_taskbar_flash", true, "Benachrichtigungen in der Taskleiste"); // pack: "Taskbar notifications"
        });
        // Quiet hours: day / start / end combo boxes + "Ruhige Stunden löschen" text button.
        section(ui, t, "Ruhezeiten", "Legen Sie Ruhezeiten fest, um Benachrichtigungen stummzuschalten und Ihre Verfügbarkeit außerhalb der Arbeitszeiten zu aktualisieren. Meeting-Erinnerungen werden weiterhin gesendet.", false, |ui| {
            self.pcheck(ui, t, "quiet_hours", false, "Ruhezeiten");
            let on = self.sb("quiet_hours", false);
            let day = self.settings["quiet_day"].as_u64().unwrap_or(0) as usize;
            ui.add_enabled_ui(on, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Tag").size(kit::SECOND).color(t.text2));
                        let l: Vec<String> = DAYS.iter().map(|s| s.to_string()).collect();
                        if let Some(i) = kit::combo(ui, &mut self.icons, t, "qh-day", 160.0, &l, day.min(6)) { self.sset("quiet_day", json!(i)); }
                    });
                    for (k, lab, def) in [("quiet_start", "Startzeit", "18:00"), ("quiet_end", "Endzeit", "08:00")] {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(lab).size(kit::SECOND).color(t.text2));
                            let ts = times();
                            let cur = self.ss(k, def);
                            let i = ts.iter().position(|x| x.0 == cur).unwrap_or(0);
                            let l: Vec<String> = ts.iter().map(|x| x.1.clone()).collect();
                            if let Some(n) = kit::combo(ui, &mut self.icons, t, k, 110.0, &l, i) { self.sset(k, json!(ts[n].0)); }
                        });
                    }
                });
            });
            ui.add_space(6.0);
            text_note(ui, t, concat!("Diese basieren auf Ihrer lokalen Zeitzone, die Sie in den ", crate::app_name!(), "-Einstellungen festgelegt haben."));
            if kit::link(ui, t, "Ruhige Stunden löschen").clicked() { self.sset("quiet_hours", json!(false)); self.toast("Ruhezeiten gelöscht", now); }
        });
        section(ui, t, "", "Passen Sie die Benachrichtigungstöne unter Audio an.", false, |ui| {
            if kit::link(ui, t, "Ändern").clicked() { ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("settings-goto"), super::nav::Page::Audio)); }
        });
    }

    pub(super) fn page_notif_messages(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, "Bereiche", "Wählen Sie hier die Benachrichtigungseinstellungen für Ihre Bereiche. Benachrichtigungen für Direktnachrichten und Anrufe sind standardmäßig aktiviert. Sie können sie über das Menü „Bereichsinformationen“ stummschalten.", true, |ui| {
            // Radio group: all / mentions / off; the keys of notify.rs follow: notif_mentions_only, notif_desktop.
            let cur = self.ss("msg_notif", "all");
            let opts = [("all", "Alle Nachrichten (Standard)", "Alle Nachrichten, Antworten und @Erwähnungen"), ("mentions", "Direktnachrichten und @Erwähnungen an mich", "Direktnachrichten und wenn Sie in einem Bereich erwähnt werden"), ("off", "Aus", "")];
            let mut pick = None;
            for (v, l, d) in opts { if kit::radio(ui, t, l, cur == v, d) { pick = Some(v); } ui.add_space(4.0); }
            if let Some(v) = pick {
                self.sset("msg_notif", json!(v));
                self.sset("notif_mentions_only", json!(v == "mentions"));
                if v == "off" { self.sset("notif_desktop", json!(false)); } else if !self.sb("notif_desktop", true) { self.sset("notif_desktop", json!(true)); }
            }
        });
        section(ui, t, "Sounds", "", false, |ui| { self.sound_section_messages(ui, t, now); });
        ui.add_space(8.0);
        if kit::pill(ui, t, "Auf Standard zurücksetzen", false, true).clicked() {
            for k in ["msg_notif", "notif_mentions_only", "notif_desktop", "snd_msg_dm", "snd_msg_space", "snd_msg_favorite"] { self.settings.as_object_mut().map(|o| o.remove(k)); }
            self.sset("msg_notif", json!("all"));
            self.reset_prefs_cache();
            self.toast("Nachrichtenbenachrichtigungen auf die Standardeinstellung zurückgesetzt.", now);
        }
    }

    pub(super) fn page_notif_calls(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, "Direkte Anrufe", "", true, |ui| { self.sound_picker(ui, t, now, "ringtone", "Klingelton für direkte Anrufe"); });
        section(ui, t, "Anrufübernahme", "Sie können festlegen, dass Sie keine Anrufübernahmebenachrichtigungen erhalten.", false, |ui| {
            self.sound_picker(ui, t, now, "call_pickup", "Ton für Anrufübernahme"); // stored, no pickup feature in Webox
            self.pcheck(ui, t, "pickup_mute_group", false, "Benachrichtigungen für die Gruppenrufannahme ausschalten");
            self.pcheck(ui, t, "pickup_mute_watchlist", false, "Benachrichtigungen für Watchlist-Anrufübernahme ausschalten");
        });
    }

    pub(super) fn page_notif_meetings(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        // Scheduled meetings: the keys of the meeting reminder toast.
        section(ui, t, "Angesetzte Meetings", "Sie können auswählen, wann Sie Benachrichtigungen für angesetzte Meetings erhalten.", true, |ui| {
            self.pcheck(ui, t, "meeting_toast_enabled", true, "Benachrichtigung vor angesetzten Meetings anzeigen");
            let en = self.sb("meeting_toast_enabled", true);
            let cur = self.settings["meeting_toast_minutes"].as_u64().unwrap_or(5);
            let opts = [(5u64, "5 Minuten vor Startzeit"), (1, "1 Minute vor Startzeit"), (0, "Zum Startzeitpunkt")];
            let l: Vec<String> = opts.iter().map(|o| o.1.to_string()).collect();
            let i = opts.iter().position(|o| o.0 == cur).unwrap_or(0);
            ui.add_enabled_ui(en, |ui| { if let Some(n) = kit::combo(ui, &mut self.icons, t, "meeting-toast-min", 280.0, &l, i) { self.sset("meeting_toast_minutes", json!(opts[n].0)); } });
        });
        section(ui, t, "Meeting-Aktionstöne", "", false, |ui| {
            self.pcheck(ui, t, "meeting_mute_beep", false, "Einen Signalton wiedergeben, wenn ich die Stummschaltung aktiviere oder meine Stummschaltung aufhebe");
        });
        section(ui, t, "Benachrichtigungen im Meeting", "Sie können auswählen, ob für Meeting-Benachrichtigungen ein Klingelton abgespielt wird.", false, |ui| {
            // Sounds per action: stored, no in-meeting events in Webox yet.
            for (k, l) in [("meeting_join", "Ton, wenn ein Teilnehmer dem Meeting beitritt"), ("meeting_leave", "Ton, wenn ein Teilnehmer das Meeting verlässt"),
                ("meeting_hand", "Ton, wenn ein Teilnehmer die Hand hebt"), ("meeting_lobby", "Ton, wenn ein Teilnehmer in der Lobby wartet"), ("meeting_msg", "Ton, wenn ich eine Nachricht erhalte")] {
                self.sound_picker(ui, t, now, k, l);
            }
            text_note(ui, t, "Sie hören diesen Ton nur als Gastgeber oder Mitgastgeber.");
        });
        section(ui, t, "Aufzeichnungen", "", false, |ui| { self.pcheck(ui, t, "notify_recordings", true, "Anzeigen, wenn Aufzeichnungen bereit sind"); });
    }

    // Meetings (general / join options / layout / lobby).
    pub(super) fn page_meetings(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, "Meetings ansetzen", "Welche Funktion zum Ansetzen möchten Sie zum Ansetzen von Meetings verwenden?", true, |ui| {
            self.pradio(ui, t, "scheduler", "webex", &[("webex", "Funktion zum Ansetzen von Webex-Site-Meetings zum Ansetzen von Meetings verwenden"), ("outlook", "Microsoft Outlook zum Ansetzen von Meetings verwenden")]);
        });
        section(ui, t, "Mein persönlicher Raum", "", false, |ui| {
            let (name, link) = (self.me.name.clone(), self.ss("personal_room_link", ""));
            for (l, v) in [("Name des persönlichen Raums", if name.is_empty() { "—".to_string() } else { format!("{name}'s Raum") }), ("Link zum persönlichen Raum", if link.is_empty() { "—".into() } else { link })] {
                ui.label(RichText::new(l).font(semibold(kit::BODY)).color(t.text1));
                ui.horizontal(|ui| { ui.label(RichText::new(&v).size(kit::BODY).color(t.text2)); if kit::pill(ui, t, "Kopieren", false, v != "—").clicked() { ui.ctx().copy_text(v.clone()); self.toast("Kopiert", now); } });
                ui.add_space(6.0);
            }
            text_note(ui, t, "Ihre Gastgeber-PIN ist erforderlich, wenn Sie einen Raum über ein Videosystem betreten oder von einem Telefon aus anrufen. Behandeln Sie Ihre Gastgeber-PIN stets vertraulich.");
        });
        section(ui, t, "Meeting vorübergehend verlassen", concat!("Erlauben Sie ", crate::app_name!(), ", anderen Ihren Status Abwesend anzuzeigen, Ihr Mikrofon stummzuschalten, Ihre Videofunktion auszuschalten und durch das Symbol eines Heißgetränks zu ersetzen, wenn Sie eine Zeitlang nicht vor der Kamera sind."), false, |ui| {
            self.pcheck(ui, t, "step_away", true, "Setzt den Status automatisch auf „Abwesend“");
        });
    }
    pub(super) fn page_meet_join(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, "Beitrittsoptionen", "Diese Optionen gelten, wenn Sie einem Meeting beitreten, sofern sie nicht von den Einstellungen der Webex-Seite überschrieben werden.", true, |ui| {
            self.pradio(ui, t, "join_mode", "default", &[("default", "Meine standardmäßigen Beitrittsoptionen verwenden"), ("last", "Meine letzten Beitrittsoptionen verwenden")]);
            ui.add_space(8.0);
            self.pcheck(ui, t, "join_mute", true, "Meinen Ton nach Meeting-Beitritt ausschalten");
            self.pcheck(ui, t, "join_video", false, "Mein Video bei Meeting-Beitritt übertragen");
        });
        section(ui, t, "Audio-Verbindung", "", false, |ui| {
            self.pcombo(ui, t, "join_audio", "computer", &[("computer", "Computer für Audio verwenden"), ("callme", "Mich anrufen"), ("none", "Nicht verbinden")], 320.0);
        });
        section(ui, t, "Mich anrufen um (erforderlich)", "", false, |ui| {
            let mut s = self.ss("callme_number", "");
            if kit::text_field(ui, t, &mut s, "Telefonnummer", 280.0).changed() { self.sset("callme_number", json!(s)); }
        });
        section(ui, t, "Zuletzt verwendete Telefonnummern", "Ihre zuletzt in Meetings verwendeten Telefonnummern werden hier angezeigt. Derzeit sind keine kürzlich gewählten Telefonnummern verfügbar.", false, |_| {});
        section(ui, t, "Untertitel", "", false, |ui| { self.pcheck(ui, t, "join_captions", false, "Untertitel automatisch einschalten, wenn Sie einem Meeting beitreten."); });
    }
    pub(super) fn page_meet_layout(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, "Anzahl der Videos pro Seite in der Rasteransicht", "Wählen Sie die Standardnummer der Teilnehmervideos aus, um die Rasteransicht auf den einzelnen Seiten anzuzeigen. Wenn es mehr Videos als die ausgewählte Nummer gibt, können Sie zu anderen Seiten navigieren, um alle anzuzeigen. Sie können im Meeting die Anzahl der Videos pro Seite anpassen, indem Sie die Ansicht vergrößern oder verkleinern. Wie viele Videos Sie auf einer Seite sehen, kann durch die Fenstergröße und Anzahl der Teilnehmer im Meeting eingeschränkt sein.", true, |ui| {
            self.pradio(ui, t, "grid_videos", "9", &[("4", "4"), ("9", "9"), ("16", "16"), ("25", "25")]);
        });
    }
    pub(super) fn page_meet_lobby(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, "Lobby", "Sie hören diesen Sound nur, wenn Sie ein Gastgeber oder Mitgastgeber sind.", true, |ui| {
            self.sound_picker(ui, t, now, "lobby", "Ton für einen Teilnehmer, der in Lobbys wartet");
        });
    }

    // Anrufe (general / dock / block list).
    pub(super) fn page_calling(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, "Eingehende Anrufe", "Wenn dieser Computer für Anrufe verwendet wird:", true, |ui| {
            self.pcheck(ui, t, "answer_with_video", false, "Anrufe mit eingeschaltetem Video annehmen");
            text_note(ui, t, "Diese Option gilt nur für Ihr Video. Sie sehen das Video der anderen Person nur dann, wenn es eingeschaltet ist.");
            self.pcheck(ui, t, "bring_hold_into_view", true, "Gehaltene Anrufe nach Beendigung des aktiven Anrufs anzeigen");
        });
        section(ui, t, "Für Anruf markieren", concat!("Markieren Sie die Nummer vor dem Anruf in der ", crate::app_name!(), "-App, um eine Vorschau anzuzeigen."), false, |ui| {
            self.pcheck(ui, t, "highlight_to_call", false, "Ausgewählte Nummer vor dem Anrufen prüfen");
        });
        section(ui, t, "Rufumleitung", "Wählen Sie die Nummern, an die eingehende Anrufe weitergeleitet werden sollen.", false, |ui| {
            self.pradio(ui, t, "call_forward", "none", &[("none", "Anrufe nicht umleiten"), ("number", "Nummer hinzufügen")]);
        });
        section(ui, t, "Zusätzliche Anrufeinstellungen", "", false, |ui| {
            if kit::link(ui, t, "Auf Benutzerportal zugreifen").clicked() { system::open("https://settings.webex.com/"); }
        });
    }
    pub(super) fn page_call_dock(&mut self, ui: &mut egui::Ui, t: &Th, ) {
        section(ui, t, "Anruf-Dock", "Über das Anruf-Dock erhalten Sie direkt schnellen Zugriff auf Webex Calling-Funktionen wie das Multi-Anruf-Fenster, Voicemail, Anrufprotokoll, Einstellungen und mehr.", true, |ui| {
            self.pcheck(ui, t, "calling_dock", true, "Anruf-Dock aktivieren");
        });
        section(ui, t, "Fenstereinstellungen", "", false, |ui| {
            self.pcheck(ui, t, "dock_stay_on_top", false, "Anruf-Dock im Vordergrund beibehalten");
        });
        section(ui, t, "Optionen für die Benachrichtigung bei eingehendem Anruf", "Wählen Sie aus, wo Ihre Benachrichtigungen für eingehende Anrufe priorisiert werden.", false, |ui| {
            self.pradio(ui, t, "dock_incoming", "default", &[("default", "Standard-Anrufbenachrichtigung verwenden"), ("front", "Anruf-Dock für eingehende Anrufe im Vordergrund anzeigen")]);
        });
    }
    pub(super) fn page_call_block(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, "Sperrliste", "Neue Nummer zur Sperrliste hinzufügen", true, |ui| {
            let mut s = self.ss("block_input", "");
            let r = ui.horizontal(|ui| {
                let ch = kit::text_field(ui, t, &mut s, "Telefonnummer", 240.0).changed();
                let add = kit::pill(ui, t, "Nummer hinzufügen", true, !s.trim().is_empty()).clicked();
                (ch, add)
            }).inner;
            if r.0 { self.settings["block_input"] = json!(s.clone()); }
            if r.1 {
                let mut list: Vec<String> = self.settings["block_list"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
                list.push(s.trim().to_string());
                self.sset("block_list", json!(list)); self.sset("block_input", json!(""));
                self.toast("Die Nummer wurde zur Sperrliste hinzugefügt.", now);
            }
            ui.add_space(8.0);
            let list: Vec<String> = self.settings["block_list"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            let mut del = None;
            for (i, n) in list.iter().enumerate() {
                ui.horizontal(|ui| { ui.label(RichText::new(n).size(kit::BODY).color(t.text1)); if kit::pill(ui, t, "Blockierung aufheben", false, true).clicked() { del = Some(i); } });
            }
            if let Some(i) = del { let mut l = list; l.remove(i); self.sset("block_list", json!(l)); }
        });
    }

    // Geräte / Zubehör / Seitenleiste.
    pub(super) fn page_devices(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, "Gerätesuche und -verbindung", "", true, |ui| {
            self.pcheck(ui, t, "devices_auto_connect", true, "Automatisch Verbindung herstellen");
            if kit::pill(ui, t, "Liste aktualisieren", false, true).clicked() { self.toast("Aktualisiert", now); }
            ui.add_space(8.0);
            text_note(ui, t, "Es wurden keine Geräte gefunden.");
        });
    }
    pub(super) fn page_accessories(&mut self, ui: &mut egui::Ui, t: &Th) { text_note(ui, t, "Es wurde kein Zubehör erkannt."); }
    pub(super) fn page_sidebar(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, "App-Seitenleiste", "Organisieren Sie Registerkarten mittels Drag-and-Drop. Zum Verschieben über die Tastatur drücken Sie die Leertaste zum Auswählen und verwenden Sie dann die obere und untere Pfeiltaste.", true, |ui| {
            for (name, key) in [("Nachrichten", "sb_messaging"), ("Meetings", "sb_meetings"), ("Anrufe", "sb_calling"), ("Whiteboards", "sb_whiteboards"), ("Kontakte", "sb_contacts")] { self.pcheck(ui, t, key, true, name); }
        });
    }
}
