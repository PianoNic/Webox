//! The line under a person's name in lists and chat headers: "Verfügbar", "Vor 3 Stunden aktiv", "Am 13.11.2026 aktiv",
//! "Bitte nicht stören bis 23.11.2026, 08:55", "In einem Meeting", "Abwesend bis …" …
//!
//! The wording is the German pack's: "Do not disturb until {{0}}" → "Bitte nicht stören bis {{0}}", "Active {{0}} hours ago"
//! → "Vor {{0}} Stunden aktiv", "Active on {{0}}" → "Am {{0}} aktiv", "Out of office until {{0}}" → "Abwesend bis {{0}}" (the
//! pack has no "Nicht im Büro"), and so on. Dates come from `i18n::format_date_time` (local zone, the language's pattern).
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

/// "Jetzt aktiv" … "Vor 3 Stunden aktiv" / "Gestern aktiv" / "Am 13.11.2026 aktiv" for a person who is away.
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
        if self.settings["show_presence"] == false { return String::new(); } // "Verfügbarkeit und Status anzeigen" off
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

    /// The German pack's exact strings for everything this module asks for.
    fn de(ctx: &str, src: &str) -> String {
        let _ = ctx;
        match src {
            "Available" => "Verfügbar", "Do not disturb" => "Bitte nicht stören", "Do not disturb until {{0}}" => "Bitte nicht stören bis {{0}}",
            "Out of Office" => "Abwesend", "Out of office until {{0}}" => "Abwesend bis {{0}}", "Busy" => "Beschäftigt", "Busy until {{0}}" => "Beschäftigt bis {{0}}",
            "Away" => "Abwesend", "Away until {{0}}" => "Abwesend bis {{0}}", "In a meeting" => "In einem Meeting", "In a call" => "In einem Anruf", "Presenting" => "Präsentation",
            "Active now" => "Jetzt aktiv", "Active in past few minutes" => "Aktiv in den letzten paar Minuten", "Active {{0}} minutes ago" => "Vor {{0}} Minuten aktiv",
            "Active 1 hour ago" => "Vor 1 Stunde aktiv", "Active {{0}} hours ago" => "Vor {{0}} Stunden aktiv", "Active yesterday" => "Gestern aktiv", "Active on {{0}}" => "Am {{0}} aktiv",
            other => other,
        }.to_string()
    }
    /// German pattern in UTC+1 (winter time), independent of the machine's zone.
    fn fmt(ms: i64, s: DateStyle) -> String { i18n::format_date_time_in(ms, 3600, s, "de") }
    fn ms(iso: &str) -> i64 { i18n::parse_iso_ms(iso).unwrap() }
    fn line(status: &str, until: Option<i64>, text: &str, last: i64, now: i64) -> String {
        presence_detail_with(&PresenceView { status, until, text, last_active: last }, now, &de, &fmt)
    }
    const NOW: &str = "2026-11-16T10:00:00.000Z";

    #[test]
    fn available_and_custom_text() {
        assert_eq!(line("active", None, "", 0, ms(NOW)), "Verfügbar");
        assert_eq!(line("active", None, "Im Homeoffice", 0, ms(NOW)), "Im Homeoffice");
        assert_eq!(line("", None, "", 0, ms(NOW)), "");
    }

    #[test]
    fn do_not_disturb_with_and_without_end() {
        // "Bitte nicht stören bis 23.11.2026, 08:55".
        assert_eq!(line("DoNotDisturb", Some(ms("2026-11-23T07:55:00.000Z")), "", 0, ms(NOW)), "Bitte nicht stören bis 23.11.2026, 08:55");
        assert_eq!(line("DoNotDisturb", None, "", 0, ms(NOW)), "Bitte nicht stören");
        // An end that has passed is no end.
        assert_eq!(line("DoNotDisturb", Some(ms("2026-11-01T07:55:00.000Z")), "", 0, ms(NOW)), "Bitte nicht stören");
    }

    #[test]
    fn out_of_office_and_other_states() {
        assert_eq!(line("OutOfOffice", Some(ms("2026-11-20T16:00:00.000Z")), "", 0, ms(NOW)), "Abwesend bis 20.11.2026, 17:00");
        assert_eq!(line("OutOfOffice", None, "Urlaub", 0, ms(NOW)), "Abwesend – Urlaub");
        assert_eq!(line("meeting", None, "", 0, ms(NOW)), "In einem Meeting");
        assert_eq!(line("call", None, "", 0, ms(NOW)), "In einem Anruf");
        assert_eq!(line("presenting", None, "", 0, ms(NOW)), "Präsentation");
        assert_eq!(line("busy", Some(ms("2026-11-16T12:00:00.000Z")), "", 0, ms(NOW)), "Beschäftigt bis 16.11.2026, 13:00");
    }

    #[test]
    fn last_active_steps() {
        let now = ms(NOW);
        let ago = |mins: i64| line("inactive", None, "", now - mins * 60_000, now);
        assert_eq!(ago(0), "Jetzt aktiv");
        assert_eq!(ago(3), "Aktiv in den letzten paar Minuten");
        assert_eq!(ago(42), "Vor 42 Minuten aktiv");
        assert_eq!(ago(60), "Vor 1 Stunde aktiv");
        assert_eq!(ago(3 * 60), "Vor 3 Stunden aktiv");
        assert_eq!(ago(23 * 60 + 59), "Vor 23 Stunden aktiv");
        assert_eq!(ago(30 * 60), "Gestern aktiv");
        assert_eq!(line("inactive", None, "", ms("2026-11-13T13:37:00.000Z"), now), "Am 13.11.2026 aktiv");
        assert_eq!(line("inactive", None, "", 0, now), "");
    }
}
