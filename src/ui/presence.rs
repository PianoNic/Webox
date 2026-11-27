//! The line under a person's name in lists and chat headers: "Available", "Active 3 hours ago", "Active on 11/13/2026",
//! "Do not disturb until 11/23/2026, 8:55 AM", "In a meeting", "Out of office until …" …
//!
//! The wording is the packs' sources ("Do not disturb until {{0}}", "Active {{0}} hours ago", "Active on {{0}}",
//! "Out of office until {{0}}", …), translated into the active language. Dates come from `i18n::format_date_time` (local zone,
//! the language's pattern).
use crate::i18n::{self, DateStyle};

/// What is known about one person's presence (all of it comes from the presence services; see `cloud/people/`).
#[derive(Clone, Debug, Default)]
pub struct PresenceView<'a> {
    /// "active", "inactive", "DoNotDisturb", "meeting", "call", "presenting", "OutOfOffice" or "" (unknown).
    pub status: &'a str,
    /// End of the status (DND, out of office…) in Unix ms.
    pub until: Option<i64>,
    /// The person's custom status / out-of-office text.
    pub text: &'a str,
    /// Last activity in Unix ms (0 = unknown).
    pub last_active: i64,
}

/// The final line (empty when nothing is known), in the active language.
pub fn presence_detail(v: &PresenceView, now_ms: i64) -> String {
    presence_detail_with(v, now_ms, &|ctx, src| i18n::tr(ctx, src).into_owned(), &i18n::format_date_time)
}

const HDR: &str = "ApplicationHeaderController";
const Q: &str = "QObject";

/// Same as [`presence_detail`] with the translator and the date formatter injected (tests use fixed ones).
pub fn presence_detail_with(v: &PresenceView, now_ms: i64, tr: &dyn Fn(&str, &str) -> String, fmt: &dyn Fn(i64, DateStyle) -> String) -> String {
    let with = |src: &str, arg: String| tr(Q, src).replace("{{0}}", &arg);
    // "Until" only counts while it is in the future.
    let until = v.until.filter(|u| *u > now_ms);
    let until_line = |plain: (&str, &str), with_until: &str| match until {
        Some(u) => with(with_until, fmt(u, DateStyle::DateTime)),
        None => tr(plain.0, plain.1),
    };
    match v.status {
        "active" => if v.text.is_empty() { tr(HDR, "Available") } else { v.text.to_string() },
        "inactive" => last_active_line(v.last_active, now_ms, &with, tr, fmt),
        "DoNotDisturb" => until_line((Q, "Do not disturb"), "Do not disturb until {{0}}"),
        "OutOfOffice" => {
            let base = until_line((Q, "Out of Office"), "Out of office until {{0}}");
            if v.text.is_empty() { base } else { format!("{base} – {}", v.text) }
        }
        "busy" => until_line((Q, "Busy"), "Busy until {{0}}"),
        "away" => until_line((Q, "Away"), "Away until {{0}}"),
        "meeting" => tr(Q, "In a meeting"),
        "call" => tr(Q, "In a call"),
        "presenting" => tr(Q, "Presenting"),
        _ => v.text.to_string(),
    }
}

/// "Active now" … "Active 3 hours ago" / "Active yesterday" / "Active on 11/13/2026" for a person who is away.
fn last_active_line(last: i64, now: i64, with: &dyn Fn(&str, String) -> String, tr: &dyn Fn(&str, &str) -> String, fmt: &dyn Fn(i64, DateStyle) -> String) -> String {
    if last <= 0 { return String::new() }
    let mins = ((now - last) / 60_000).max(0);
    if mins < 1 { return tr(Q, "Active now") }
    if mins < 5 { return tr(Q, "Active in past few minutes") }
    if mins < 60 { return with("Active {{0}} minutes ago", mins.to_string()) }
    let hours = mins / 60;
    if hours == 1 { return tr(Q, "Active 1 hour ago") }
    if hours < 24 { return with("Active {{0}} hours ago", hours.to_string()) }
    if fmt(last, DateStyle::Date) == fmt(now - 86_400_000, DateStyle::Date) { return tr(Q, "Active yesterday") }
    with("Active on {{0}}", fmt(last, DateStyle::Date))
}

impl crate::App {
    /// The presence line for `p` (id, e-mail and status as the cloud thread filed them), using the live details.
    pub(crate) fn presence_line(&self, p: &crate::Person, now_ms: i64) -> String {
        if self.settings["show_presence"] == false { return String::new(); } // "Show availability and status" off
        let keys = [p.id.as_str(), p.email.as_str()];
        let (until, text) = keys.iter().find_map(|k| self.photos.presence_info.get(*k).or_else(|| self.photos.presence_info.get(&k.to_lowercase())))
            .map(|(u, t)| (*u, t.as_str())).unwrap_or((None, ""));
        let last_active = keys.iter().find_map(|k| self.photos.last_active.get(*k).copied()).unwrap_or(0);
        presence_detail(&PresenceView { status: &p.status, until, text, last_active }, now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// English: the sources as they are.
    fn en(_ctx: &str, src: &str) -> String { src.to_string() }
    /// English (US) pattern in UTC+1 (winter time), independent of the machine's zone.
    fn fmt(ms: i64, s: DateStyle) -> String { i18n::format_date_time_in(ms, 3600, s, "en") }
    fn ms(iso: &str) -> i64 { i18n::parse_iso_ms(iso).unwrap() }
    fn line(status: &str, until: Option<i64>, text: &str, last: i64, now: i64) -> String {
        presence_detail_with(&PresenceView { status, until, text, last_active: last }, now, &en, &fmt)
    }
    const NOW: &str = "2026-11-16T10:00:00.000Z";

    #[test]
    fn available_and_custom_text() {
        assert_eq!(line("active", None, "", 0, ms(NOW)), "Available");
        assert_eq!(line("active", None, "Working from home", 0, ms(NOW)), "Working from home");
        assert_eq!(line("", None, "", 0, ms(NOW)), "");
    }

    #[test]
    fn do_not_disturb_with_and_without_end() {
        assert_eq!(line("DoNotDisturb", Some(ms("2026-11-23T07:55:00.000Z")), "", 0, ms(NOW)), "Do not disturb until 11/23/2026, 8:55 AM");
        assert_eq!(line("DoNotDisturb", None, "", 0, ms(NOW)), "Do not disturb");
        // An end that has passed is no end.
        assert_eq!(line("DoNotDisturb", Some(ms("2026-11-01T07:55:00.000Z")), "", 0, ms(NOW)), "Do not disturb");
    }

    #[test]
    fn out_of_office_and_other_states() {
        assert_eq!(line("OutOfOffice", Some(ms("2026-11-20T16:00:00.000Z")), "", 0, ms(NOW)), "Out of office until 11/20/2026, 5:00 PM");
        assert_eq!(line("OutOfOffice", None, "Vacation", 0, ms(NOW)), "Out of Office – Vacation");
        assert_eq!(line("meeting", None, "", 0, ms(NOW)), "In a meeting");
        assert_eq!(line("call", None, "", 0, ms(NOW)), "In a call");
        assert_eq!(line("presenting", None, "", 0, ms(NOW)), "Presenting");
        assert_eq!(line("busy", Some(ms("2026-11-16T12:00:00.000Z")), "", 0, ms(NOW)), "Busy until 11/16/2026, 1:00 PM");
    }

    #[test]
    fn last_active_steps() {
        let now = ms(NOW);
        let ago = |mins: i64| line("inactive", None, "", now - mins * 60_000, now);
        assert_eq!(ago(0), "Active now");
        assert_eq!(ago(3), "Active in past few minutes");
        assert_eq!(ago(42), "Active 42 minutes ago");
        assert_eq!(ago(60), "Active 1 hour ago");
        assert_eq!(ago(3 * 60), "Active 3 hours ago");
        assert_eq!(ago(23 * 60 + 59), "Active 23 hours ago");
        assert_eq!(ago(30 * 60), "Active yesterday");
        assert_eq!(line("inactive", None, "", ms("2026-11-13T13:37:00.000Z"), now), "Active on 11/13/2026");
        assert_eq!(line("inactive", None, "", 0, now), "");
    }
}
