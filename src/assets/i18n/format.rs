//! Locale formats for dates and times.
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
}

pub(super) const fn f(date: &'static str, time: &'static str, sep: &'static str) -> Formats {
    Formats { date, time, sep, am: "AM", pm: "PM" }
}

/// CLDR short date/time patterns.
pub fn formats_for(code: &str) -> Formats {
    match code {
        "de" => f("dd.MM.y", "HH:mm", ", "),
        "en_GB" => f("dd/MM/y", "HH:mm", ", "),
        "ar" => f("d/M/y", "h:mm a", ", "),
        "bg" => f("d.MM.y", "H:mm", ", "),
        "ca" => f("d/M/y", "H:mm", ", "),
        "cs" => f("d. M. y", "H:mm", " "),
        "cy_GB" => f("dd/MM/y", "HH:mm", ", "),
        "da" => f("dd.MM.y", "HH.mm", " "),
        "es" => f("d/M/y", "H:mm", ", "),
        "es_MX" => f("dd/MM/y", "H:mm", ", "),
        "fr" => f("dd/MM/y", "HH:mm", " "),
        "fr_CA" => f("y-MM-dd", "HH 'h' mm", " "),
        "ga" => f("dd/MM/y", "HH:mm", ", "),
        "he" => f("d.M.y", "H:mm", ", "),
        "hr" => f("dd. MM. y.", "HH:mm", " "),
        "hu" => f("y. MM. dd.", "H:mm", " "),
        "id_ID" => f("dd/MM/y", "HH.mm", ", "),
        "it" => f("dd/MM/y", "HH:mm", ", "),
        "ja" => f("y/MM/dd", "H:mm", " "),
        "ko" => Formats { am: "오전", pm: "오후", ..f("y. M. d.", "a h:mm", " ") },
        "nb" => f("dd.MM.y", "HH:mm", ", "),
        "nl" => f("dd-MM-y", "HH:mm", ", "),
        "pl" => f("d.MM.y", "HH:mm", ", "),
        "pt" => f("dd/MM/y", "HH:mm", ", "),
        "ro" => f("dd.MM.y", "HH:mm", ", "),
        "ru" => f("dd.MM.y", "HH:mm", ", "),
        "sl" => f("d. M. y", "HH:mm", ", "),
        "sr" => f("d.M.y.", "HH:mm", ", "),
        "sv" => f("y-MM-dd", "HH:mm", " "),
        "tr" => f("d.MM.y", "HH:mm", " "),
        "uk" => f("dd.MM.y", "HH:mm", ", "),
        "zh" => f("y/M/d", "HH:mm", " "),
        "zh_TW" => Formats { am: "上午", pm: "下午", ..f("y/M/d", "ah:mm", " ") },
        // en (US) and everything unknown.
        _ => f("M/d/y", "h:mm a", ", "),
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
    /// `13.11.2026, 16:48`
    DateTime,
}

/// Format `ts_ms` (unix ms, UTC) with an explicit zone offset and locale code (pure; used by tests).
pub fn format_date_time_in(ts_ms: i64, offset_secs: i32, style: DateStyle, code: &str) -> String {
    let p = parts(ts_ms.div_euclid(1000) + offset_secs as i64);
    let fm = formats_for(code);
    match style {
        DateStyle::Date => apply_pattern(fm.date, &p, &fm),
        DateStyle::DateTime => format!("{}{}{}", apply_pattern(fm.date, &p, &fm), fm.sep, apply_pattern(fm.time, &p, &fm)),
    }
}

/// Format `ts_ms` (unix ms, UTC) in the local time zone with the active language's locale.
pub fn format_date_time(ts_ms: i64, style: DateStyle) -> String {
    format_date_time_in(ts_ms, local_offset_secs(ts_ms.div_euclid(1000)), style, active_code())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}
