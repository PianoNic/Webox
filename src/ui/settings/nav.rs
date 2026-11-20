//! Navigation model of the Settings page and the search index.
//!
//! Rows are (title, icon, depth); titles come from the German pack (page title = nav title). A parent row selects its own
//! page and expands: Audio > Smart Audio, Benachrichtigungen > Nachrichten / Anrufe / Meetings, Meetings > Beitrittsoptionen /
//! Layout / Lobby, Anrufe > Anruf-Dock / Sperrliste.
//!
//! Pages for features a plain Webex cloud account doesn't have are not shown: Konten (UC/CUCM accounts), Telefonservice
//! (UDS/UC login), Kundenbetreuer-Benachrichtigungen (Customer Assist agents), KI-Assistent / MCP-Server / Entwickler,
//! Video-Hintergrund.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Page {
    #[default] General, Audio, SmartAudio, Video, Share, Notifications, NotifMessages, NotifCalls, NotifMeetings, Appearance, Accessibility, Shortcuts,
    Privacy, Messaging, Meetings, MeetJoin, MeetLayout, MeetLobby, Calling, CallDock, CallBlock, Devices, Accessories, Sidebar,
    /// Webox only: the installed plugins.
    Plugins,
}

/// (page, German title, icon, depth) in display order.
pub(crate) const NAV: [(Page, &str, &str, u8); 25] = [
    (Page::General, "Allgemein", "settings-regular", 0),
    (Page::Audio, "Audio", "speaker-regular", 0),
    (Page::SmartAudio, "Smart Audio", "audio-enhancement-regular", 1),
    (Page::Video, "Video", "video-regular", 0),
    (Page::Share, "Freigabe von Inhalten", "share-screen-regular", 0),
    (Page::Notifications, "Benachrichtigungen", "alert-regular", 0),
    (Page::NotifMessages, "Nachrichtenbenachrichtigungen", "chat-regular", 1),
    (Page::NotifCalls, "Anrufbenachrichtigungen", "handset-regular", 1),
    (Page::NotifMeetings, "Meeting-Benachrichtigungen", "meetings-regular", 1),
    (Page::Appearance, "Erscheinungsbild", "appearance-regular", 0),
    (Page::Accessibility, "Barrierefreiheit", "accessibility-regular", 0),
    (Page::Shortcuts, "Tastenkombinationen", "keyboard-regular", 0),
    (Page::Privacy, "Privatfunktion", "private-regular", 0),
    (Page::Messaging, "Messaging", "chat-regular", 0),
    (Page::Meetings, "Meetings", "meetings-regular", 0),
    (Page::MeetJoin, "Beitrittsoptionen", "", 1),
    (Page::MeetLayout, "Layout", "", 1),
    (Page::MeetLobby, "Lobby", "", 1),
    (Page::Calling, "Anrufe", "handset-regular", 0),
    (Page::CallDock, "Anruf-Dock", "", 1),
    (Page::CallBlock, "Sperrliste", "", 1),
    (Page::Devices, "Geräte", "devices-regular", 0),
    (Page::Accessories, "Zubehör", "headset-regular", 0),
    (Page::Sidebar, "App-Seitenleiste", "", 0),
    (Page::Plugins, "Plugins", "integrations-regular", 0), // Webox only, after the Webex entries
];

pub(crate) fn title(p: Page) -> &'static str { NAV.iter().find(|n| n.0 == p).map_or("", |n| n.1) }
pub(crate) fn parent(p: Page) -> Option<Page> {
    match p {
        Page::SmartAudio => Some(Page::Audio), Page::NotifMessages | Page::NotifCalls | Page::NotifMeetings => Some(Page::Notifications),
        Page::MeetJoin | Page::MeetLayout | Page::MeetLobby => Some(Page::Meetings), Page::CallDock | Page::CallBlock => Some(Page::Calling), _ => None,
    }
}
pub(crate) fn has_children(p: Page) -> bool { NAV.iter().any(|n| parent(n.0) == Some(p)) }

/// Page ids for `WEBOX_SHOT_SETTINGS=<name>` (screenshot runs).
pub(crate) fn by_name(s: &str) -> Option<Page> {
    let n = s.to_lowercase();
    Some(match n.as_str() {
        "general" | "0" => Page::General, "audio" => Page::Audio, "smartaudio" => Page::SmartAudio, "video" => Page::Video, "share" => Page::Share,
        "notifications" => Page::Notifications, "messages" | "notifmessages" => Page::NotifMessages, "calls" | "notifcalls" => Page::NotifCalls,
        "notifmeetings" => Page::NotifMeetings, "appearance" => Page::Appearance, "accessibility" => Page::Accessibility, "shortcuts" => Page::Shortcuts,
        "privacy" => Page::Privacy, "messaging" => Page::Messaging, "meetings" => Page::Meetings, "join" => Page::MeetJoin, "layout" => Page::MeetLayout,
        "lobby" => Page::MeetLobby, "calling" => Page::Calling, "dock" => Page::CallDock, "block" => Page::CallBlock, "devices" => Page::Devices,
        "accessories" => Page::Accessories, "sidebar" => Page::Sidebar, "plugins" => Page::Plugins, _ => return None,
    })
}

/// Search index: (page, setting label, short description). Labels are the page's German labels.
pub(crate) const INDEX: &[(Page, &str, &str)] = &[
    (Page::General, concat!(crate::app_name!(), " zusammen mit meinem Computer starten"), "Anmelden"),
    (Page::General, "Verfügbarkeit und Status anzeigen", "Verfügbarkeit und Status"),
    (Page::General, concat!("Verbindung von ", crate::app_name!(), " mit Microsoft Outlook herstellen"), "Outlook-Einstellungen"),
    (Page::General, "Zielbildschirm", concat!("Was beim Öffnen von ", crate::app_name!(), " zuerst angezeigt wird")),
    (Page::General, "Standardmäßiger Dateispeicherort", "Ordner für heruntergeladene Dateien"),
    (Page::General, "Lokale Zeitzone", "Zeitzone für Ihr Profil und Ruhezeiten"),
    (Page::General, "Multitasking", "Unverankertes Minifenster für Anruf oder Meeting"),
    (Page::General, "CO₂-bewusster Modus", "Videoauflösung reduzieren"),
    (Page::General, "Sprache", "Sprache der Oberfläche"),
    (Page::General, "Beim Schließen in den Infobereich minimieren", "Fenster schließen"),
    (Page::Audio, "Lautsprecher", "Ausgabegerät und Lautstärke"),
    (Page::Audio, "Mikrofon", "Eingabegerät und Eingangspegel"),
    (Page::Audio, "Klingeltöne und Warnungen", "Klingelton, Benutzerdefinierter Ton"),
    (Page::Audio, "Windows-Signalverarbeitung umgehen", "Erweiterte Audioeinstellungen"),
    (Page::SmartAudio, "Mikrofonaudio", "Smart Audio"),
    (Page::SmartAudio, "Externe Audiokommunikation", "Smart Audio"),
    (Page::Video, "Kamera", "Kameraauflösung, Helligkeit, Kontrast"),
    (Page::Video, "Selbstansicht in Meetings", "Nur anzeigen, wenn mein Video aktiviert ist"),
    (Page::Video, "Videoverbesserungen", "Look auffrischen"),
    (Page::Share, "Maximale Auflösung der Inhalte", "Auflösung und Bildrate"),
    (Page::Notifications, "Ruhezeiten", "Benachrichtigungen stummschalten"),
    (Page::Notifications, "Desktopbenachrichtigungen anzeigen", "Benachrichtigungen"),
    (Page::Notifications, "Nachrichtenvorschau anzeigen", "Vorschau in Benachrichtigungen"),
    (Page::Notifications, "Benachrichtigungen in der Taskleiste", "Taskleiste blinkt"),
    (Page::NotifMessages, "Alle Nachrichten (Standard)", "Nachrichtenbenachrichtigungen"),
    (Page::NotifMessages, "Töne für Benachrichtigungen über Direktnachrichten", "Sounds"),
    (Page::NotifMessages, "Töne für Bereichsbenachrichtigungen", "Sounds"),
    (Page::NotifMessages, "Töne für Benachrichtigungen über Favoriten-Bereiche", "Sounds"),
    (Page::NotifCalls, "Klingelton für direkte Anrufe", "Anrufbenachrichtigungen"),
    (Page::NotifCalls, "Ton für Anrufübernahme", "Anrufübernahme"),
    (Page::NotifMeetings, "Angesetzte Meetings", "Benachrichtigung vor Meeting-Beginn"),
    (Page::NotifMeetings, "Einen Signalton wiedergeben, wenn ich die Stummschaltung aktiviere", "Meeting-Aktionstöne"),
    (Page::NotifMeetings, "Ton, wenn ein Teilnehmer dem Meeting beitritt", "In-Meeting-Benachrichtigungen"),
    (Page::Appearance, "Themen", concat!("Legen Sie fest, wie ", crate::app_name!(), " aussehen soll.")),
    (Page::Appearance, "Standard Dunkel, Hell", "Themen"),
    (Page::Appearance, "Bronze, Jade, Lavendel, Indigoblau", "Themen (dunkel und hell)"),
    (Page::Appearance, "Immer dunkles Thema verwenden", "Meetings"),
    (Page::Appearance, "Handgesten erkennen", "Meetings und Anrufe"),
    (Page::Accessibility, "Schriftgröße", "Größe der Nachrichten"),
    (Page::Shortcuts, "Tastenkombinationen", "Beschreibung, Tastenkombination, Global"),
    (Page::Privacy, "Verfügbarkeit und Status anzeigen", "Privatfunktion"),
    (Page::Privacy, "Meine Verfügbarkeit und meinen Status ausblenden", "Personen"),
    (Page::Messaging, "Rechtschreibprüfung aktivieren", "Sprache der Rechtschreibprüfung"),
    (Page::Messaging, "Threads", "Threads erweitern oder reduzieren"),
    (Page::Messaging, "Lesebestätigungen", "Durch Lesebestätigungen gesehen"),
    (Page::Messaging, "Namen automatisch eintragen", "Autovervollständigung"),
    (Page::Messaging, "Emojis automatisch vorschlagen", "Emoji-Autovervollständigung"),
    (Page::Messaging, "Nachrichten-Schriftgröße", "Barrierefreiheit"),
    (Page::Meetings, "Meetings ansetzen", "Kalender und Terminplaner"),
    (Page::Meetings, "Mein persönlicher Raum", "Link, Nummer, Gastgeber-PIN"),
    (Page::Meetings, "Meeting vorübergehend verlassen", "Automatisch Abwesend"),
    (Page::MeetJoin, "Mein Video bei Meeting-Beitritt übertragen", "Beitrittsoptionen"),
    (Page::MeetJoin, "Meinen Ton nach Meeting-Beitritt ausschalten", "Beitrittsoptionen"),
    (Page::MeetJoin, "Mich anrufen um", "Telefonnummern"),
    (Page::MeetLayout, "Anzahl der Videos pro Seite in der Rasteransicht", "Layout"),
    (Page::MeetLobby, "Ton für einen Teilnehmer, der in Lobbys wartet", "Lobby"),
    (Page::Calling, "Anrufe mit eingeschaltetem Video annehmen", "Eingehende Anrufe"),
    (Page::Calling, "Rufumleitung", "Zusätzliche Nummern für Anrufe"),
    (Page::Calling, "Voicemail", "Zusätzliche Anrufeinstellungen"),
    (Page::CallDock, "Anruf-Dock aktivieren", "Anruf-Dock"),
    (Page::CallDock, "Anruf-Dock im Vordergrund beibehalten", "Fenstereinstellungen"),
    (Page::CallBlock, "Nummer hinzufügen", "Sperrliste"),
    (Page::Devices, "Liste aktualisieren", "Gerätesuche und -verbindung"),
    (Page::Accessories, "Zubehör", "Headsets und Zubehör"),
    (Page::Sidebar, "App-Seitenleiste", "Registerkarten per Drag-and-Drop organisieren"),
    (Page::Plugins, "Plugins", "Installierte Plugins aktivieren oder deaktivieren"),
    (Page::Plugins, "Design-Plugins", "Überschreiben die Farben des gewählten Themas"),
    (Page::Plugins, "Plugin-Ordner", "Plugins neu laden, Ordner öffnen"),
    (Page::Plugins, "Verfügbare Plugins", "Plugins aus Repositories installieren und aktualisieren"),
    (Page::Plugins, "Plugin-Repositories", "Repositories hinzufügen oder entfernen"),
    (Page::Plugins, "Automatisch nach Updates suchen", "Einmal am Tag nach neuen Plugin-Versionen suchen"),
];

/// Case-insensitive substring search over label + description.
pub(crate) fn search(q: &str) -> Vec<(Page, &'static str, &'static str)> {
    let q = q.trim().to_lowercase();
    if q.is_empty() { return vec![]; }
    INDEX.iter().filter(|(_, l, d)| l.to_lowercase().contains(&q) || d.to_lowercase().contains(&q)).map(|e| (e.0, e.1, e.2)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nav_tree_and_index_are_consistent() {
        for (p, t, _, d) in NAV { assert!(!t.is_empty()); assert_eq!(d == 1, parent(p).is_some(), "{t}"); }
        for (p, _, _) in INDEX { assert!(NAV.iter().any(|n| n.0 == *p)); }
        assert!(has_children(Page::Notifications) && !has_children(Page::Video));
        assert!(search("stumm").iter().any(|e| e.0 == Page::NotifMeetings));
        assert!(search("").is_empty());
    }
}
