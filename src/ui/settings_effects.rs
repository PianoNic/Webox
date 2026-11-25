//! Settings that act app-wide, applied once per frame (`appwide_frame`, hook in frame.rs) or asked by the code
//! that needs them (`quiet_now`). The settings pages only store the values (settings JSON); this file makes them take effect:
//! * `show_presence` (General / Privacy "Show availability and status"): off hides every presence badge and line, and
//!   a change is sent as the user feature toggle `user-presence-enabled`.
//! * `msg_font_size` (Accessibility > Font size): the message text scale (ui/messages/ JobBuilder).
//! * `home` (General > Landing screen): the page shown at start.
//! * `quiet_hours` / `quiet_day` / `quiet_start` / `quiet_end` (Notifications > Quiet hours): no toasts and no message sounds
//!   inside the window. The page has one day row: the hours apply on that weekday (and run past midnight into the next day
//!   when the end is before the start).
//! * the custom status' end: an expired status is removed from the settings.
use crate::*;
use std::sync::atomic::Ordering;

fn hhmm_min(s: &str) -> Option<i64> { let (h, m) = s.split_once(':')?; Some(h.trim().parse::<i64>().ok()? * 60 + m.trim().parse::<i64>().ok()?) }

/// Whether local time (`local_ms` = Unix ms shifted by the zone offset) is inside the quiet hours of weekday `day` (0 = Monday)
/// from `start` to `end` ("HH:MM").
pub(crate) fn in_quiet_hours(local_ms: i64, day: usize, start: &str, end: &str) -> bool {
    let (Some(s), Some(e)) = (hhmm_min(start), hhmm_min(end)) else { return false };
    let days = local_ms.div_euclid(86_400_000);
    let wd = ((days + 3).rem_euclid(7)) as usize; // 1970-01-01 was a Thursday (index 3)
    let m = local_ms.rem_euclid(86_400_000) / 60_000;
    if s < e { wd == day && m >= s && m < e } else if s > e { (wd == day && m >= s) || (wd == (day + 1) % 7 && m < e) } else { false }
}

impl App {
    /// True while the configured quiet hours run (notifications muted).
    pub(crate) fn quiet_now(&self) -> bool {
        if !self.settings["quiet_hours"].as_bool().unwrap_or(false) { return false; }
        let n = crate::i18n::now_ms();
        let local = n + crate::i18n::local_offset_secs(n / 1000) as i64 * 1000;
        let day = self.settings["quiet_day"].as_u64().unwrap_or(0) as usize;
        in_quiet_hours(local, day, self.settings["quiet_start"].as_str().unwrap_or("18:00"), self.settings["quiet_end"].as_str().unwrap_or("08:00"))
    }

    pub(crate) fn appwide_frame(&mut self, ctx: &egui::Context) {
        let shown = self.settings["show_presence"] != false;
        super::widgets::PRESENCE_SHOWN.store(shown, Ordering::Relaxed);
        super::widgets::MSG_SCALE.store(self.msg_font_scale().to_bits(), Ordering::Relaxed);
        // A change of "Show availability and status" goes to the service (only a change the user made: the first value is the baseline).
        let kid = egui::Id::new("appwide-presence-shown");
        let last: Option<bool> = ctx.data(|d| d.get_temp(kid));
        if last.is_some_and(|l| l != shown) {
            if let Some(c) = &self.client { c.req(Req::SetFeature { key: cloud::PRESENCE_FEATURE.into(), val: json!(shown) }); }
        }
        ctx.data_mut(|d| d.insert_temp(kid, shown));
        // Landing screen, once per run.
        let hid = egui::Id::new("appwide-home-done");
        if !ctx.data(|d| d.get_temp::<bool>(hid)).unwrap_or(false) {
            ctx.data_mut(|d| d.insert_temp(hid, true));
            let tab = match self.settings["home"].as_str() { Some("Meetings") => Some(Tab::Meetings), Some("Calling") => Some(Tab::Calling), Some("Contacts") => Some(Tab::Contacts), _ => None };
            if let Some(t) = tab { self.set_tab(t); }
        }
        // --screenshot: WEBOX_SHOT_APPWIDE=health|updates|feedback|device|profile|sessions|hide|picker|help opens that dialog / submenu.
        if self.shot.is_some() && self.frames == 40 {
            if let Ok(k) = std::env::var("WEBOX_SHOT_APPWIDE") {
                use super::app_dialogs as d;
                match k.as_str() {
                    "health" => super::menu::menu_open(ctx, d::HEALTH), "updates" => super::menu::menu_open(ctx, d::UPDATES),
                    "feedback" => super::menu::menu_open(ctx, d::FEEDBACK), "device" => super::menu::menu_open(ctx, d::DEVICE),
                    "profile" => super::menu::menu_open(ctx, d::EDIT_PROFILE), "sessions" => super::menu::menu_open(ctx, d::SESSIONS),
                    "hide" => super::menu::menu_open(ctx, d::HIDE_FROM), "picker" => d::open_time_picker(ctx, "dnd"),
                    "help" => { super::menu::menu_open(ctx, "status"); super::menu::menu_open(ctx, "status-help"); }
                    _ => {}
                }
            }
        }
        // An expired custom status is forgotten.
        if let Some(u) = self.settings["custom_status"]["until"].as_i64() {
            if u > 0 && u <= crate::i18n::now_ms() {
                self.settings["custom_status"] = serde_json::Value::Null;
                if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(iso: &str) -> i64 { crate::i18n::parse_iso_ms(iso).unwrap() }

    #[test]
    fn quiet_hours_window() {
        // 2026-11-16 is a Monday (day 0).
        assert!(in_quiet_hours(ms("2026-11-16T12:00:00.000Z"), 0, "09:00", "17:00"));
        assert!(!in_quiet_hours(ms("2026-11-16T17:00:00.000Z"), 0, "09:00", "17:00"));
        assert!(!in_quiet_hours(ms("2026-11-17T12:00:00.000Z"), 0, "09:00", "17:00")); // Tuesday
        // Overnight Monday 18:00 → Tuesday 08:00.
        assert!(in_quiet_hours(ms("2026-11-16T23:00:00.000Z"), 0, "18:00", "08:00"));
        assert!(in_quiet_hours(ms("2026-11-17T07:59:00.000Z"), 0, "18:00", "08:00"));
        assert!(!in_quiet_hours(ms("2026-11-17T08:00:00.000Z"), 0, "18:00", "08:00"));
        assert!(!in_quiet_hours(ms("2026-11-16T07:00:00.000Z"), 0, "18:00", "08:00")); // Monday morning belongs to Sunday's night
        assert!(in_quiet_hours(ms("2026-11-23T07:00:00.000Z"), 6, "18:00", "08:00")); // Sunday's night runs into Monday
    }
}
