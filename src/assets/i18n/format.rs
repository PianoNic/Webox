//! Locale formats for dates, times and numbers, and the relative-time strings.
use super::*;

// Locale formats.

/// Per-locale short formats. Patterns: y/yyyy, M/MM, d/dd, H/HH, h/hh, m/mm, a (AM/PM text), 'literal'.
#[derive(Debug, Clone, Copy)]
pub struct Formats {
    pub date: &'static str,
    pub time: &'static str,
    /// Joins date and time.
    pub sep: &'static str,
    pub am: &'static str,
    pub pm: &'static str,
    pub group: &'static str,
    pub decimal: &'static str,
}

pub(super) const fn f(date: &'static str, time: &'static str, sep: &'static str, group: &'static str, decimal: &'static str) -> Formats {
    Formats { date, time, sep, am: "AM", pm: "PM", group, decimal }
}

/// CLDR short date/time patterns.
pub fn formats_for(code: &str) -> Formats {
    match code {
        "de" => f("dd.MM.y", "HH:mm", ", ", ".", ","),
        "en_GB" => f("dd/MM/y", "HH:mm", ", ", ",", "."),
        "ar" => f("d/M/y", "h:mm a", ", ", ",", "."),
        "bg" => f("d.MM.y", "H:mm", ", ", "\u{a0}", ","),
        "ca" => f("d/M/y", "H:mm", ", ", ".", ","),
        "cs" => f("d. M. y", "H:mm", " ", "\u{a0}", ","),
        "cy_GB" => f("dd/MM/y", "HH:mm", ", ", ",", "."),
        "da" => f("dd.MM.y", "HH.mm", " ", ".", ","),
        "es" => f("d/M/y", "H:mm", ", ", ".", ","),
        "es_MX" => f("dd/MM/y", "H:mm", ", ", ",", "."),
        "fr" => f("dd/MM/y", "HH:mm", " ", "\u{202f}", ","),
        "fr_CA" => f("y-MM-dd", "HH 'h' mm", " ", "\u{a0}", ","),
        "ga" => f("dd/MM/y", "HH:mm", ", ", ",", "."),
        "he" => f("d.M.y", "H:mm", ", ", ",", "."),
        "hr" => f("dd. MM. y.", "HH:mm", " ", ".", ","),
        "hu" => f("y. MM. dd.", "H:mm", " ", "\u{a0}", ","),
        "id_ID" => f("dd/MM/y", "HH.mm", ", ", ".", ","),
        "it" => f("dd/MM/y", "HH:mm", ", ", ".", ","),
        "ja" => f("y/MM/dd", "H:mm", " ", ",", "."),
        "ko" => Formats { am: "오전", pm: "오후", ..f("y. M. d.", "a h:mm", " ", ",", ".") },
        "nb" => f("dd.MM.y", "HH:mm", ", ", "\u{a0}", ","),
        "nl" => f("dd-MM-y", "HH:mm", ", ", ".", ","),
        "pl" => f("d.MM.y", "HH:mm", ", ", "\u{a0}", ","),
        "pt" => f("dd/MM/y", "HH:mm", ", ", ".", ","),
        "ro" => f("dd.MM.y", "HH:mm", ", ", ".", ","),
        "ru" => f("dd.MM.y", "HH:mm", ", ", "\u{a0}", ","),
        "sl" => f("d. M. y", "HH:mm", ", ", ".", ","),
        "sr" => f("d.M.y.", "HH:mm", ", ", ".", ","),
        "sv" => f("y-MM-dd", "HH:mm", " ", "\u{a0}", ","),
        "tr" => f("d.MM.y", "HH:mm", " ", ".", ","),
        "uk" => f("dd.MM.y", "HH:mm", ", ", "\u{a0}", ","),
        "zh" => f("y/M/d", "HH:mm", " ", ",", "."),
        "zh_TW" => Formats { am: "上午", pm: "下午", ..f("y/M/d", "ah:mm", " ", ",", ".") },
        // en (US) and everything unknown.
        _ => f("M/d/y", "h:mm a", ", ", ",", "."),
    }
}

pub(super) fn apply_pattern(pat: &str, p: &Parts, fm: &Formats) -> String {
    let c: Vec<char> = pat.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '\'' {
            i += 1;
            while i < c.len() && c[i] != '\'' {
                out.push(c[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if !"yMdHhms a".contains(ch) || ch == ' ' {
            out.push(ch);
            i += 1;
            continue;
        }
        let mut n = 1;
        while i + n < c.len() && c[i + n] == ch {
            n += 1;
        }
        i += n;
        let h12 = if p.hour % 12 == 0 { 12 } else { p.hour % 12 };
        let v = match ch {
            'y' => {
                if n == 2 {
                    format!("{:02}", p.year % 100)
                } else {
                    p.year.to_string()
                }
            }
            'M' => pad(p.month, n),
            'd' => pad(p.day, n),
            'H' => pad(p.hour, n),
            'h' => pad(h12, n),
            'm' => pad(p.minute, n),
            's' => pad(p.second, n),
            _ => (if p.hour < 12 { fm.am } else { fm.pm }).to_string(),
        };
        out.push_str(&v);
    }
    out
}
pub(super) fn pad(v: i64, n: usize) -> String {
    if n >= 2 {
        format!("{v:02}")
    } else {
        v.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateStyle {
    /// `13.11.2026`
    Date,
    /// `16:48`
    Time,
    /// `13.11.2026, 16:48`
    DateTime,
}

/// Format `ts_ms` (unix ms, UTC) with an explicit zone offset and locale code (pure; used by tests).
pub fn format_date_time_in(ts_ms: i64, offset_secs: i32, style: DateStyle, code: &str) -> String {
    let p = parts(ts_ms.div_euclid(1000) + offset_secs as i64);
    let fm = formats_for(code);
    match style {
        DateStyle::Date => apply_pattern(fm.date, &p, &fm),
        DateStyle::Time => apply_pattern(fm.time, &p, &fm),
        DateStyle::DateTime => format!("{}{}{}", apply_pattern(fm.date, &p, &fm), fm.sep, apply_pattern(fm.time, &p, &fm)),
    }
}

/// Format `ts_ms` (unix ms, UTC) in the local time zone with the active language's locale.
pub fn format_date_time(ts_ms: i64, style: DateStyle) -> String {
    format_date_time_in(ts_ms, local_offset_secs(ts_ms.div_euclid(1000)), style, active_code())
}

/// `"2026-11-13T15:48:00Z"` -> local wall-clock time in the locale's time format (`16:48`, `4:48 PM`).
/// Unparsable input is returned unchanged.
pub fn local_hhmm(iso_utc: &str) -> String {
    match parse_iso_ms(iso_utc) {
        Some(ms) => format_date_time(ms, DateStyle::Time),
        None => iso_utc.to_string(),
    }
}
/// `"2026-11-13T15:48:00Z"` -> local `13.11.2026, 16:48`.
pub fn local_date_time(iso_utc: &str) -> String {
    match parse_iso_ms(iso_utc) {
        Some(ms) => format_date_time(ms, DateStyle::DateTime),
        None => iso_utc.to_string(),
    }
}

/// Day separator label: "Today" / "Yesterday" or the locale date. `now_ms` is injected for tests; use [`now_ms`] otherwise.
pub fn day_label(ts_ms: i64, now_ms: i64) -> String {
    let off = |t: i64| local_offset_secs(t.div_euclid(1000)) as i64;
    let day = |t: i64| (t.div_euclid(1000) + off(t)).div_euclid(86400);
    let d = day(ts_ms);
    let today = day(now_ms);
    if d == today {
        tr("CustomCalendar", "Today").into_owned()
    } else if d == today - 1 {
        tr("CxAgentInteractionsListController", "Yesterday").into_owned()
    } else {
        format_date_time(ts_ms, DateStyle::Date)
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

pub(super) fn group_digits(mut digits: String, sep: &str) -> String {
    let mut out = String::new();
    while digits.len() > 3 {
        let cut = digits.len() - 3;
        out = format!("{}{}{}", sep, &digits[cut..], out);
        digits.truncate(cut);
    }
    format!("{digits}{out}")
}

/// Integer with the locale's thousands separator.
pub fn format_int_in(n: i64, code: &str) -> String {
    let fm = formats_for(code);
    let s = group_digits(n.unsigned_abs().to_string(), fm.group);
    if n < 0 { format!("-{s}") } else { s }
}
pub fn format_int(n: i64) -> String {
    format_int_in(n, active_code())
}
/// Decimal number with `dp` fraction digits.
pub fn format_decimal_in(v: f64, dp: usize, code: &str) -> String {
    let fm = formats_for(code);
    let s = format!("{:.*}", dp, v.abs());
    let (i, fr) = s.split_once('.').map_or((s.as_str(), None), |(a, b)| (a, Some(b)));
    let mut out = group_digits(i.to_string(), fm.group);
    if let Some(fr) = fr {
        out.push_str(fm.decimal);
        out.push_str(fr);
    }
    if v < 0.0 && out.chars().any(|c| c.is_ascii_digit() && c != '0') { format!("-{out}") } else { out }
}
pub fn format_decimal(v: f64, dp: usize) -> String {
    format_decimal_in(v, dp, active_code())
}

// Relative times.

/// Which family of relative-time strings to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelKind {
    /// `{{0}} hours ago`
    Plain,
    /// `Active {{0}} hours ago`, `Active now`, `Active yesterday`
    Active,
    /// `Created {{0}} days ago`
    Created,
    /// `Edited {{0}} days ago`
    Edited,
    /// `Latest activity {{0}} hours ago`
    LatestActivity,
}

pub(super) fn unit_text(kind: RelKind, unit: &str, n: i64) -> String {
    // Key shapes in the pack's QObject context: "{{0}} hours ago", "1 hour ago",
    // "Active {{0}} hours ago", "Active 1 hour ago", "Created {{0}} days ago", "Created {{0}}1 day ago", ...
    let prefix = match kind {
        RelKind::Plain => "",
        RelKind::Active => "Active ",
        RelKind::Created => "Created ",
        RelKind::Edited => "Edited ",
        RelKind::LatestActivity => "Latest activity ",
    };
    if n == 1 && unit != "minute" {
        let key = match kind {
            RelKind::Plain => format!("1 {unit} ago"),
            RelKind::Active => format!("Active 1 {unit} ago"),
            _ => format!("{prefix}{{{{0}}}}1 {unit} ago"),
        };
        // `{{0}}1 day` keys carry an empty `{{0}}` prefix argument.
        return args(&tr("QObject", &key), &[&""]);
    }
    let key = format!("{prefix}{{{{0}}}} {unit}s ago");
    args(&tr("QObject", &key), &[&n])
}

/// Relative time for an age in milliseconds, e.g. `relative_kind(3*3600_000, RelKind::Plain)` -> "Vor 3 Stunden".
/// Thresholds: <2 min "A moment ago" ("Vor Kurzem"), <60 min minutes, <24 h hours, <30 d days, <365 d months, else years.
pub fn relative_kind(ms_ago: i64, kind: RelKind) -> String {
    let ms_ago = ms_ago.max(0);
    let min = ms_ago / 60_000;
    let hours = ms_ago / 3_600_000;
    let days = ms_ago / 86_400_000;
    match kind {
        RelKind::Active => {
            if min < 1 {
                return tr("QObject", "Active now").into_owned();
            }
            if min < 2 {
                return tr("QObject", "Active in past few minutes").into_owned();
            }
        }
        RelKind::LatestActivity if min < 5 => return tr("QObject", "Latest activity a few minutes ago").into_owned(),
        _ => {}
    }
    if min < 2 {
        return match kind {
            RelKind::Created => tr("QObject", "Created a moment ago").into_owned(),
            RelKind::Edited => tr("QObject", "Edited a moment ago").into_owned(),
            _ => tr("QObject", "A moment ago").into_owned(),
        };
    }
    if hours < 1 {
        return unit_text(kind, "minute", min);
    }
    if days < 1 {
        return unit_text(kind, "hour", hours);
    }
    if days == 1 && matches!(kind, RelKind::Active) {
        return tr("QObject", "Active yesterday").into_owned();
    }
    if days == 1 && matches!(kind, RelKind::LatestActivity) {
        return tr("QObject", "Latest activity yesterday").into_owned();
    }
    if matches!(kind, RelKind::Active | RelKind::LatestActivity) && days >= 2 {
        // There is no "Active {{0}} days ago": switch to a date ("Active on {{0}}").
        let d = format_date_time(now_ms() - ms_ago, DateStyle::Date);
        return if kind == RelKind::Active { args(&tr("QObject", "Active on {{0}}"), &[&d]) } else { d };
    }
    if days < 30 {
        return unit_text(kind, "day", days);
    }
    if days < 365 {
        return unit_text(kind, "month", days / 30);
    }
    unit_text(kind, "year", days / 365)
}

/// [`relative_kind`] with [`RelKind::Plain`]: "A moment ago", "Vor 3 Stunden", "vor 1 Tag", ...
pub fn relative_time(ms_ago: i64) -> String {
    relative_kind(ms_ago, RelKind::Plain)
}
