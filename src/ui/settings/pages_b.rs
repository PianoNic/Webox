//! Settings pages, part 2: Notifications (+ Messages / Calls / Meetings), Meetings (+ Join / Layout / Lobby),
//! Calling (+ Dock / Block list), Devices, Accessories. Labels are the pack's English sources where it has them.
//! Keys shared with notify.rs: notif_desktop, notif_preview, notif_mentions_only, notif_taskbar_flash, close_to_tray,
//! meeting_toast_enabled, meeting_toast_minutes.
use super::kit::{self, section};
use super::pages_a::text_note;
use crate::*;

const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

fn times() -> Vec<(String, String)> { (0..48).map(|i| { let s = format!("{:02}:{:02}", i / 2, (i % 2) * 30); (s.clone(), s) }).collect() }

impl App {
    // Notifications.
    pub(super) fn page_notifications(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, tr("Desktop notifications"), "", true, |ui| { // keys shared with notify.rs
            self.pcheck(ui, t, "notif_desktop", true, tr("Show desktop notifications"));
            self.pcheck(ui, t, "notif_preview", true, tr("Show Message Previews")); // pack: "Show Message Previews"
            self.pcheck(ui, t, "notif_taskbar_flash", true, tr("Taskbar notifications")); // pack: "Taskbar notifications"
        });
        // Quiet hours: day / start / end combo boxes + "Clear quiet hours" text button.
        section(ui, t, tr("Quiet hours"), tr("Schedule Quiet Hours to mute notifications and update your availability outside working hours. Meeting reminders will still be sent."), false, |ui| {
            self.pcheck(ui, t, "quiet_hours", false, tr("Quiet hours"));
            let on = self.sb("quiet_hours", false);
            let day = self.settings["quiet_day"].as_u64().unwrap_or(0) as usize;
            ui.add_enabled_ui(on, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(tr("Day")).size(kit::SECOND).color(t.text2));
                        let l: Vec<String> = DAYS.iter().map(|s| tr(s).to_string()).collect();
                        if let Some(i) = kit::combo(ui, &mut self.icons, t, "qh-day", 160.0, &l, day.min(6)) { self.sset("quiet_day", json!(i)); }
                    });
                    for (k, lab, def) in [("quiet_start", tr("Start time"), "18:00"), ("quiet_end", tr("End time"), "08:00")] {
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
            text_note(ui, t, &crate::trq!("These are based on your local time zone, set in the %1 settings.", crate::app_name!()));
            if kit::link(ui, t, tr("Clear quiet hours")).clicked() { self.sset("quiet_hours", json!(false)); self.toast(tr("Quiet hours cleared"), now); }
        });
        section(ui, t, "", tr("Adjust the notification sounds under Audio."), false, |ui| {
            if kit::link(ui, t, tr("Change")).clicked() { ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("settings-goto"), super::nav::Page::Audio)); }
        });
    }

    pub(super) fn page_notif_messages(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, tr("Spaces"), tr("Choose your notification preferences for your spaces here. Notifications for direct messages and calls are on by default. You can mute them using the Space Information menu."), true, |ui| {
            // Radio group: all / mentions / off; the keys of notify.rs follow: notif_mentions_only, notif_desktop.
            let cur = self.ss("msg_notif", "all");
            let opts = [("all", tr("All messages (default)"), tr("All messages, replies, and @mentions.")), ("mentions", tr("Direct messages and @mentions to me"), tr("Direct messages and when you're mentioned in a space.")), ("off", tr("Off"), "")];
            let mut pick = None;
            for (v, l, d) in opts { if kit::radio(ui, t, l, cur == v, d) { pick = Some(v); } ui.add_space(4.0); }
            if let Some(v) = pick {
                self.sset("msg_notif", json!(v));
                self.sset("notif_mentions_only", json!(v == "mentions"));
                if v == "off" { self.sset("notif_desktop", json!(false)); } else if !self.sb("notif_desktop", true) { self.sset("notif_desktop", json!(true)); }
            }
        });
        section(ui, t, tr("Sounds"), "", false, |ui| { self.sound_section_messages(ui, t, now); });
        ui.add_space(8.0);
        if kit::pill(ui, t, tr("Reset to default"), false, true).clicked() {
            for k in ["msg_notif", "notif_mentions_only", "notif_desktop", "snd_msg_dm", "snd_msg_space", "snd_msg_favorite"] { self.settings.as_object_mut().map(|o| o.remove(k)); }
            self.sset("msg_notif", json!("all"));
            self.reset_prefs_cache();
            self.toast(tr("Message notifications reset to default."), now);
        }
    }

    pub(super) fn page_notif_calls(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, tr("Direct calls"), "", true, |ui| { self.sound_picker(ui, t, now, "ringtone", tr("Direct calls ringtone")); });
        section(ui, t, tr("Call pickup"), tr("You can choose not to receive call pickup notifications."), false, |ui| {
            self.sound_picker(ui, t, now, "call_pickup", tr("Call pickup tone")); // stored, no pickup feature in Webox
            self.pcheck(ui, t, "pickup_mute_group", false, tr("Turn off group call pickup notifications"));
            self.pcheck(ui, t, "pickup_mute_watchlist", false, tr("Turn off watchlist call pickup notifications"));
        });
    }

    pub(super) fn page_notif_meetings(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        // Scheduled meetings: the keys of the meeting reminder toast.
        section(ui, t, tr("Scheduled meetings"), tr("You can choose when to receive notifications for scheduled meetings."), true, |ui| {
            self.pcheck(ui, t, "meeting_toast_enabled", true, tr("Show a notification before scheduled meetings"));
            let en = self.sb("meeting_toast_enabled", true);
            let cur = self.settings["meeting_toast_minutes"].as_u64().unwrap_or(5);
            let opts = [(5u64, tr("5 minutes before start time")), (1, tr("1 minute before start time")), (0, tr("At start time"))];
            let l: Vec<String> = opts.iter().map(|o| o.1.to_string()).collect();
            let i = opts.iter().position(|o| o.0 == cur).unwrap_or(0);
            ui.add_enabled_ui(en, |ui| { if let Some(n) = kit::combo(ui, &mut self.icons, t, "meeting-toast-min", 280.0, &l, i) { self.sset("meeting_toast_minutes", json!(opts[n].0)); } });
        });
        section(ui, t, tr("Meeting action sounds"), "", false, |ui| {
            self.pcheck(ui, t, "meeting_mute_beep", false, tr("Play a beep when I mute or unmute myself"));
        });
        section(ui, t, tr("In-meeting notifications"), tr("You can choose whether to play a ringtone for meeting notifications."), false, |ui| {
            // Sounds per action: stored, no in-meeting events in Webox yet.
            for (k, l) in [("meeting_join", tr("Sound when a participant joins the meeting")), ("meeting_leave", tr("Sound when a participant leaves the meeting")),
                ("meeting_hand", tr("Sound when a participant raises a hand")), ("meeting_lobby", tr("Sound when a participant is waiting in the lobby")), ("meeting_msg", tr("Sound when I receive a message"))] {
                self.sound_picker(ui, t, now, k, l);
            }
            text_note(ui, t, tr("You'll only hear this sound when you're the host or a cohost."));
        });
        section(ui, t, tr("Recordings"), "", false, |ui| { self.pcheck(ui, t, "notify_recordings", true, tr("Show when recordings are ready")); });
    }

    // Meetings (general / join options / layout / lobby).
    pub(super) fn page_meetings(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, tr("Schedule meetings"), tr("Which scheduler do you want to use to schedule meetings?"), true, |ui| {
            self.pradio(ui, t, "scheduler", "webex", &[("webex", crate::tr!("SettingsMeetingGeneralPage", "Use %1 site meeting scheduler to schedule meetings", "Webex").as_str()), ("outlook", tr("Use Microsoft Outlook to schedule meetings"))]);
        });
        section(ui, t, tr("My Personal Room"), "", false, |ui| {
            let (name, link) = (self.me.name.clone(), self.ss("personal_room_link", ""));
            for (l, v) in [(tr("Personal Room name"), if name.is_empty() { "—".to_string() } else { crate::trq!("%1's room", name) }), (tr("Personal Room link"), if link.is_empty() { "—".into() } else { link })] {
                ui.label(RichText::new(l).font(semibold(kit::BODY)).color(t.text1));
                ui.horizontal(|ui| { ui.label(RichText::new(&v).size(kit::BODY).color(t.text2)); if kit::pill(ui, t, tr("Copy"), false, v != "—").clicked() { ui.ctx().copy_text(v.clone()); self.toast(tr("Copied"), now); } });
                ui.add_space(6.0);
            }
            text_note(ui, t, tr("Your host PIN is required when you enter your room from a video system or call in from a phone. Be sure to keep your host PIN private."));
        });
        section(ui, t, tr("Step away from meeting"), &crate::trq!("Allow Webex to show others your <b>Stepped away</b> status, mute your microphone, turn off your video, and replace your video with a hot beverage icon when you're away from your camera for a while.").replace("<b>", "").replace("</b>", ""), false, |ui| {
            self.pcheck(ui, t, "step_away", true, tr("Automatically set status to stepped away"));
        });
    }
    pub(super) fn page_meet_join(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, tr("Join options"), tr("These options apply when you join a meeting, unless the Webex site settings override them."), true, |ui| {
            self.pradio(ui, t, "join_mode", "default", &[("default", tr("Use my default join options")), ("last", tr("Use my last join options"))]);
            ui.add_space(8.0);
            self.pcheck(ui, t, "join_mute", true, tr("Mute my audio when I join a meeting"));
            self.pcheck(ui, t, "join_video", false, tr("Start my video when I join a meeting"));
        });
        section(ui, t, tr("Audio connection"), "", false, |ui| {
            self.pcombo(ui, t, "join_audio", "computer", &[("computer", tr("Use computer for audio")), ("callme", tr("Call me")), ("none", tr("Don't connect"))], 320.0);
        });
        section(ui, t, tr("Call me at (required)"), "", false, |ui| {
            let mut s = self.ss("callme_number", "");
            if kit::text_field(ui, t, &mut s, tr("Phone number"), 280.0).changed() { self.sset("callme_number", json!(s)); }
        });
        section(ui, t, tr("Recent phone numbers"), tr("The phone numbers you used in recent meetings show here. There are no recently used phone numbers yet."), false, |_| {});
        section(ui, t, tr("Captions"), "", false, |ui| { self.pcheck(ui, t, "join_captions", false, tr("Automatically turn on closed captions when you join a meeting.")); });
    }
    pub(super) fn page_meet_layout(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, tr("Number of videos per page in grid view"), tr("Select the default number of participant videos to view in each page for the grid view layout. When there are more videos than the selected number, you can navigate to other pages to see all of them. In the meeting, you can adjust the number of videos per page by zooming in and out. Your window size and the number of participants in the meeting may limit how many videos you see in a page."), true, |ui| {
            self.pradio(ui, t, "grid_videos", "9", &[("4", "4"), ("9", "9"), ("16", "16"), ("25", "25")]);
        });
    }
    pub(super) fn page_meet_lobby(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, tr("Lobby"), tr("You will hear this sound, only when you are a host or cohost."), true, |ui| {
            self.sound_picker(ui, t, now, "lobby", tr("Sound for participant waiting in the lobbys"));
        });
    }

    // Calling (general / dock / block list).
    pub(super) fn page_calling(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, tr("Incoming Calls"), tr("When this computer is used for calls:"), true, |ui| {
            self.pcheck(ui, t, "answer_with_video", false, tr("Answer calls with my video on"));
            text_note(ui, t, tr("This option applies only to your video, you may still see the other person's video if that person has turned on their video."));
            self.pcheck(ui, t, "bring_hold_into_view", true, tr("Bring held calls into view when the active call ends"));
        });
        section(ui, t, tr("Highlight to call"), &crate::trq!("Highlight a number before you call it in the %1 app to see a preview.", crate::app_name!()), false, |ui| {
            self.pcheck(ui, t, "highlight_to_call", false, tr("Review number before calling"));
        });
        section(ui, t, tr("Call Forward"), tr("Choose numbers to forward incoming calls."), false, |ui| {
            self.pradio(ui, t, "call_forward", "none", &[("none", tr("Do Not Forward Calls")), ("number", tr("Add a number"))]);
        });
        section(ui, t, tr("Additional Call Settings"), "", false, |ui| {
            if kit::link(ui, t, tr("Access User Portal")).clicked() { system::open("https://settings.webex.com/"); }
        });
    }
    pub(super) fn page_call_dock(&mut self, ui: &mut egui::Ui, t: &Th, ) {
        section(ui, t, tr("Calling dock"), tr("The calling dock gives you quick access to Webex Calling features such as the multi-call window, voicemail, call history, settings and more."), true, |ui| {
            // Off: the dock no longer opens by itself during a call (call_win/call.rs); Ctrl+Shift+X still opens it.
            self.pcheck(ui, t, super::DOCK_ENABLED, true, tr("Turn on the calling dock"));
        });
        section(ui, t, tr("Window settings"), "", false, |ui| {
            // The key the dock reads for its window level (= the dock's pin button).
            self.pcheck(ui, t, super::DOCK_KEEP_IN_FRONT, false, tr("Keep the calling dock in front"));
        });
        // The radio group is disabled while the dock is off (SettingsCallingDockPage.qml: `enabled: phoneWidgetOption.isChecked`).
        let dock_on = self.sb(super::DOCK_ENABLED, true);
        section(ui, t, tr("Incoming call notification options"), tr("Choose where your incoming call notifications are prioritized."), false, |ui| {
            ui.add_enabled_ui(dock_on, |ui| { self.pradio(ui, t, "dock_incoming", "default", &[("default", tr("Use the standard call notification")), ("front", tr("Bring the calling dock to the front for incoming calls"))]); });
        });
    }
    pub(super) fn page_call_block(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, tr("Block List"), tr("Add a new number to block list"), true, |ui| {
            let mut s = self.ss("block_input", "");
            let r = ui.horizontal(|ui| {
                let ch = kit::text_field(ui, t, &mut s, tr("Phone number"), 240.0).changed();
                let add = kit::pill(ui, t, tr("Add a number"), true, !s.trim().is_empty()).clicked();
                (ch, add)
            }).inner;
            if r.0 { self.settings["block_input"] = json!(s.clone()); }
            if r.1 {
                let mut list: Vec<String> = self.settings["block_list"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
                list.push(s.trim().to_string());
                self.sset("block_list", json!(list)); self.sset("block_input", json!(""));
                self.toast(tr("The number was added to the block list."), now);
            }
            ui.add_space(8.0);
            let list: Vec<String> = self.settings["block_list"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            let mut del = None;
            for (i, n) in list.iter().enumerate() {
                ui.horizontal(|ui| { ui.label(RichText::new(n).size(kit::BODY).color(t.text1)); if kit::pill(ui, t, tr("Unblock"), false, true).clicked() { del = Some(i); } });
            }
            if let Some(i) = del { let mut l = list; l.remove(i); self.sset("block_list", json!(l)); }
        });
    }

    // Devices / Accessories.
    pub(super) fn page_devices(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        section(ui, t, tr("Device discovery and connection"), "", true, |ui| {
            self.pcheck(ui, t, "devices_auto_connect", true, tr("Connect automatically"));
            if kit::pill(ui, t, tr("Refresh list"), false, true).clicked() { self.toast(tr("Refreshed"), now); }
            ui.add_space(8.0);
            text_note(ui, t, tr("No devices were found."));
        });
    }
    pub(super) fn page_accessories(&mut self, ui: &mut egui::Ui, t: &Th) { text_note(ui, t, tr("No accessories were detected.")); }
}
