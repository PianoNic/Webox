//! Navigation model of the Settings page and the search index.
//!
//! Rows are (title, icon, depth); titles are English pack sources, translated when drawn (page title = nav title). A parent
//! row selects its own page and expands: Audio > Smart Audio, Notifications > Messages / Calls / Meetings, Meetings > Join
//! options / Layout / Lobby, Calling > Calling dock / Block list.
//!
//! Pages for features a plain Webex cloud account doesn't have are not shown: Accounts (UC/CUCM accounts), Phone service
//! (UDS/UC login), Customer Assist notifications, AI Assistant / MCP servers / Developer, Video background.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Page {
    #[default] General, Audio, SmartAudio, Video, Share, Notifications, NotifMessages, NotifCalls, NotifMeetings, Appearance, Accessibility, Shortcuts,
    Privacy, Messaging, Meetings, MeetJoin, MeetLayout, MeetLobby, Calling, CallDock, CallBlock, Devices, Accessories,
    /// Customize the interface: show / hide / order UI elements (ui/customize.rs); took over the rail part of "App sidebar".
    Customize,
    /// Webox only: the installed plugins.
    Plugins,
}

/// (page, English title (translated with `tr` when drawn), icon, depth) in display order.
pub(crate) const NAV: [(Page, &str, &str, u8); 25] = [
    (Page::General, "General", "settings-regular", 0),
    (Page::Audio, "Audio", "speaker-regular", 0),
    (Page::SmartAudio, "Smart Audio", "audio-enhancement-regular", 1),
    (Page::Video, "Video", "video-regular", 0),
    (Page::Share, "Sharing content", "share-screen-regular", 0),
    (Page::Notifications, "Notifications", "alert-regular", 0),
    (Page::NotifMessages, "Message Notifications", "chat-regular", 1),
    (Page::NotifCalls, "Call Notifications", "handset-regular", 1),
    (Page::NotifMeetings, "Meeting Notifications", "meetings-regular", 1),
    (Page::Appearance, "Appearance", "appearance-regular", 0),
    (Page::Customize, "Customize the interface", "adjust-regular", 0),
    (Page::Accessibility, "Accessibility", "accessibility-regular", 0),
    (Page::Shortcuts, "Keyboard shortcuts", "keyboard-regular", 0),
    (Page::Privacy, "Privacy", "private-regular", 0),
    (Page::Messaging, "Messaging", "chat-regular", 0),
    (Page::Meetings, "Meetings", "meetings-regular", 0),
    (Page::MeetJoin, "Join options", "", 1),
    (Page::MeetLayout, "Layout", "", 1),
    (Page::MeetLobby, "Lobby", "", 1),
    (Page::Calling, "Calling", "handset-regular", 0),
    (Page::CallDock, "Calling dock", "", 1),
    (Page::CallBlock, "Block List", "", 1),
    (Page::Devices, "Devices", "devices-regular", 0),
    (Page::Accessories, "Accessories", "headset-regular", 0),
    (Page::Plugins, "Plugins", "integrations-regular", 0), // Webox only, after the Webex entries
];

/// The page title in the active language.
pub(crate) fn title(p: Page) -> &'static str { NAV.iter().find(|n| n.0 == p).map_or("", |n| crate::ui::widgets::tr(n.1)) }
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
        "accessories" => Page::Accessories, "customize" | "sidebar" => Page::Customize, "plugins" => Page::Plugins, _ => return None,
    })
}

/// Search index: (page, setting label, short description). Labels are the page's English sources (`{{APP_NAME}}` filled
/// and translated by [`search`]).
pub(crate) const INDEX: &[(Page, &str, &str)] = &[
    (Page::General, "Start {{APP_NAME}} when my computer starts", "Sign In"),
    (Page::General, "Show availability and status", "Availability and Status"),
    (Page::General, "Connect %1 to Microsoft Outlook", "Outlook Settings"),
    (Page::General, "Landing screen", "What you see first when {{APP_NAME}} opens"),
    (Page::General, "Default file location", "Folder for downloaded files"),
    (Page::General, "Local time zone", "Time zone for your profile and quiet hours"),
    (Page::General, "Multitasking", "Floating mini window for a call or meeting"),
    (Page::General, "Language", "Interface language"),
    (Page::General, "Minimize to the notification area when closing", "Close window"),
    (Page::Audio, "Speaker", "Output device and volume"),
    (Page::Audio, "Microphone", "Input device and input level"),
    (Page::Audio, "Ringtones and alerts", "Ringtone, custom sound"),
    (Page::Audio, "Bypass Windows signal processing", "Advanced audio settings"),
    (Page::SmartAudio, "Microphone audio", "Smart Audio"),
    (Page::SmartAudio, "External audio", "Smart Audio"),
    (Page::Video, "Camera", "Camera resolution, brightness, contrast"),
    (Page::Video, "Self view in meetings", "Show only when my video is on"),
    (Page::Video, "Video enhancements", "Touch up my look"),
    (Page::Share, "Maximum content resolution", "Resolution and frame rate"),
    (Page::Notifications, "Quiet hours", "Mute notifications"),
    (Page::Notifications, "Show desktop notifications", "Notifications"),
    (Page::Notifications, "Show Message Previews", "Preview in notifications"),
    (Page::Notifications, "Taskbar notifications", "Taskbar flashes"),
    (Page::NotifMessages, "All messages (default)", "Message Notifications"),
    (Page::NotifMessages, "Sounds for direct message notifications", "Sounds"),
    (Page::NotifMessages, "Sounds for space notifications", "Sounds"),
    (Page::NotifMessages, "Sounds for favorite space notifications", "Sounds"),
    (Page::NotifCalls, "Direct calls ringtone", "Call Notifications"),
    (Page::NotifCalls, "Call pickup tone", "Call pickup"),
    (Page::NotifMeetings, "Scheduled meetings", "Notification before a meeting starts"),
    (Page::NotifMeetings, "Play a beep when I mute", "Meeting action sounds"),
    (Page::NotifMeetings, "Sound when a participant joins the meeting", "In-meeting notifications"),
    (Page::Appearance, "Themes", "Choose how you want {{APP_NAME}} to look."),
    (Page::Appearance, "Default Dark, Light", "Themes"),
    (Page::Appearance, "Bronze, Jade, Lavender, Indigo", "Themes (dark and light)"),
    (Page::Appearance, "Always use dark theme", "Meetings"),
    (Page::Appearance, "Recognize hand gestures", "Meetings & Calls"),
    (Page::Appearance, "Display density", "Default, Compact"),
    (Page::Appearance, "Pause GIFs", "only play while Webox is the active window"),
    (Page::Accessibility, "Font size", "Size of messages"),
    (Page::Messaging, "Convert emoticons to emoji", ":) xD :D"),
    (Page::Shortcuts, "Keyboard shortcuts", "Description, Shortcut, Global"),
    (Page::Privacy, "Show availability and status", "Privacy"),
    (Page::Privacy, "Hide my availability and status", "People"),
    (Page::Messaging, "Enable spell check", "Spell check language"),
    (Page::Messaging, "Show compact view", "Space list"),
    (Page::Messaging, "Threads", "Expand or collapse threads"),
    (Page::Messaging, "Read receipts", "Seen by Read Receipts"),
    (Page::Messaging, "Autofill names", "Autocomplete"),
    (Page::Messaging, "Suggest emoji automatically", "Emoji autocomplete"),
    (Page::Messaging, "Message font size", "Accessibility"),
    (Page::Meetings, "Schedule meetings", "Calendar and scheduler"),
    (Page::Meetings, "My Personal Room", "Link, number, host PIN"),
    (Page::Meetings, "Step away from meeting", "Automatically stepped away"),
    (Page::MeetJoin, "Start my video when I join a meeting", "Join options"),
    (Page::MeetJoin, "Mute my audio when I join a meeting", "Join options"),
    (Page::MeetJoin, "Call me at", "Phone numbers"),
    (Page::MeetLayout, "Number of videos per page in grid view", "Layout"),
    (Page::MeetLobby, "Sound for participant waiting in the lobbys", "Lobby"),
    (Page::Calling, "Answer calls with my video on", "Incoming Calls"),
    (Page::Calling, "Call Forward", "Additional numbers to ring"),
    (Page::Calling, "Voicemail", "Additional Call Settings"),
    (Page::CallDock, "Turn on the calling dock", "Calling dock"),
    (Page::CallDock, "Keep the calling dock in front", "Window settings"),
    (Page::CallBlock, "Add a number", "Block List"),
    (Page::Devices, "Refresh list", "Device discovery and connection"),
    (Page::Accessories, "Accessories", "Headsets and accessories"),
    (Page::Customize, "App sidebar", "Show, hide and order tabs"),
    (Page::Customize, "Customize the interface", "Show, hide and order elements"),
    (Page::Customize, "Message box toolbar", "Show, hide and order the buttons under the message box"),
    (Page::Customize, "More menu", "Voicemail, Plugins & Themes, About Webox, Show labels"),
    (Page::Customize, "Top bar", "Hide search, +, Connect to a device, AI Assistant"),
    (Page::Customize, "Space tabs", "Hide People, Content, Meetings, +Apps"),
    (Page::Customize, "Message actions", "Reactions, Edit, Reply, Forward on hover"),
    (Page::Plugins, "Plugins", "Turn installed plugins on or off"),
    (Page::Plugins, "Design plugins", "Override the colors of the chosen theme"),
    (Page::Plugins, "Plugin folder", "Reload plugins, open folder"),
    (Page::Plugins, "Available plugins", "Install and update plugins from repositories"),
    (Page::Plugins, "Plugin repositories", "Add or remove repositories"),
    (Page::Plugins, "Check for updates automatically", "Look for new plugin versions once a day"),
];

/// An index text in the active language (`{{APP_NAME}}` / `%1` = the app name).
fn shown(s: &str) -> String { crate::i18n::args(&crate::i18n::tr("QObject", s), &[&crate::app_name!()]) }

/// Case-insensitive substring search over label + description, then over the elements of Customize the interface (label,
/// group). Matches the shown (translated) text and the English source. Returns the shown texts.
pub(crate) fn search(q: &str) -> Vec<(Page, String, String)> {
    let q = q.trim().to_lowercase();
    if q.is_empty() { return vec![]; }
    let hit = |s: &str| shown(s).to_lowercase().contains(&q) || s.to_lowercase().contains(&q);
    let mut v: Vec<(Page, String, String)> = INDEX.iter().filter(|(_, l, d)| hit(l) || hit(d)).map(|e| (e.0, shown(e.1), shown(e.2))).collect();
    for g in crate::ui::customize::GROUPS {
        for e in g.els.iter().filter(|e| hit(e.label)) {
            let l = shown(e.label);
            if !v.iter().any(|r| r.0 == Page::Customize && r.1 == l) { v.push((Page::Customize, l, shown(g.title))); }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nav_tree_and_index_are_consistent() {
        for (p, t, _, d) in NAV { assert!(!t.is_empty()); assert_eq!(d == 1, parent(p).is_some(), "{t}"); }
        for (p, _, _) in INDEX { assert!(NAV.iter().any(|n| n.0 == *p)); }
        assert!(has_children(Page::Notifications) && !has_children(Page::Video));
        assert!(search("mute").iter().any(|e| e.0 == Page::NotifMeetings));
        assert!(search("").is_empty());
        // Customize the interface: groups and single elements are found; the old sidebar page name still opens it.
        for q in ["interface", "App sidebar", "Add GIFs", "Recommended", "Connect to a device", "Whiteboards"] {
            assert!(search(q).iter().any(|e| e.0 == Page::Customize), "{q}");
        }
        assert_eq!(by_name("sidebar"), Some(Page::Customize));
        assert_eq!(by_name("customize"), Some(Page::Customize));
    }
}
