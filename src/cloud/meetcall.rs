//! Call history, the calendar's meeting list, scheduling and the personal room.
//!
//! * **Call history** = the janus service:
//!   `GET {janus}/history/userSessions?from=<ISO, now - 10 days>&limit=50&includeNewSessionTypes=true&sort=DESC`,
//!   delete = `POST {janus}/history/userSessions/markAsDeleted` `{deleteSessionIds:[{sessionId, endTime:<ms>}]}`.
//!   Records are `UserSession`s (`sessionType, startTime, endTime, direction, disposition, other.id, other.name,
//!   other.callbackAddress, …`). Live changes arrive over mercury as `janus.user_sessions` / `janus.user_recent_sessions`.
//! * **Meeting list** = the calendar service (U2C service `calendar`): `GET {calendar}/calendarEvents` with
//!   `fromDate` / `toDate` (yesterday .. +7 days); encrypted fields (`encryptedSubject`, `encryptedLocation`, `webexURL`,
//!   `spaceMeetURL`, `meetingJoinInfo.meetingJoinURL`, `encryptedOrganizer.encryptedName`, all with `encryptionKeyUrl`);
//!   end = start + `durationMinutes`.
//! * **Scheduling**: accounts with a Webex site schedule on that site; without a site URL this uses the public Webex REST
//!   API `POST https://webexapis.com/v1/meetings` (`title`, `start`, `end`, `timezone`, `agenda`, `invitees[{email}]`);
//!   its answer's `webLink` is the join link.
//! * **Personal room**: the public API `GET https://webexapis.com/v1/meetingPreferences/personalMeetingRoom`
//!   (`personalMeetingRoomLink`).
use super::*;

pub const HYDRA: &str = "https://webexapis.com/v1"; // public Webex REST API (developer.webex.com)
const HISTORY_DAYS: i64 = 10; // days of call history asked for
const HISTORY_LIMIT: u32 = 50; // call history records per request

/// One janus `UserSession` as the call list needs it. Times in Unix ms (UTC).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallRecord {
    pub session_id: String, pub start_ms: i64, pub end_ms: i64, pub secs: u32,
    /// `direction` "OUTGOING" / "INCOMING".
    pub outgoing: bool,
    /// `disposition` as sent (e.g. "CANCELED", "Answered", "Initiated", "MISSED"; compared case-insensitively).
    pub disposition: String,
    pub name: String, pub email: String, pub callback: String, pub phone: String, pub other_id: String,
    pub session_type: String, pub conversation_url: String,
}
impl CallRecord {
    /// An incoming session nobody answered on any device.
    pub fn missed(&self) -> bool { !self.outgoing && !self.disposition.eq_ignore_ascii_case("answered") }
}

/// One calendar event (decrypted).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CalEvent {
    pub id: String, pub title: String, pub start_ms: i64, pub end_ms: i64, pub organizer: String, pub location: String,
    pub link: String, pub space_url: String, pub recurring: bool, pub attendees: usize,
}

/// What the scheduler dialog sends (times in Unix ms).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScheduleReq { pub title: String, pub start_ms: i64, pub end_ms: i64, pub agenda: String, pub invitees: Vec<String>, pub timezone: String }

/// ISO-8601 UTC with milliseconds, as JavaScript's `Date.toISOString()`.
pub fn iso_utc(ms: i64) -> String {
    let p = crate::i18n::parts(ms.div_euclid(1000));
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", p.year, p.month, p.day, p.hour, p.minute, p.second, ms.rem_euclid(1000))
}

pub fn history_url(janus: &str, now_ms: i64) -> String {
    format!("{janus}/history/userSessions?from={}&limit={HISTORY_LIMIT}&includeNewSessionTypes=true&sort=DESC", iso_utc(now_ms - HISTORY_DAYS * 86_400_000))
}

/// `{userSessions:[UserSession]}` -> records, newest first.
pub fn parse_user_sessions(v: &Value) -> Vec<CallRecord> {
    let ms = |s: &str| crate::i18n::parse_iso_ms(s).unwrap_or(0);
    let mut out: Vec<CallRecord> = v["userSessions"].as_array().into_iter().flatten().filter(|s| s["isDeleted"].as_bool() != Some(true)).map(|s| {
        let o = &s["other"];
        let (start_ms, end_ms) = (ms(&st(s, "startTime")), ms(&st(s, "endTime")));
        let secs = s["durationSeconds"].as_u64().or_else(|| s["durationSecs"].as_u64()).unwrap_or_else(|| ((end_ms - start_ms).max(0) / 1000) as u64) as u32;
        let callback = [st(o, "callbackAddress"), st(&s["links"], "callbackAddress")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
        let email = [st(o, "email"), st(o, "contact")].into_iter().find(|x| x.contains('@')).or_else(|| callback.contains('@').then(|| callback.clone())).unwrap_or_default();
        let name = [st(o, "name"), st(o, "primaryDisplayString"), st(o, "phoneNumber"), email.clone()].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
        CallRecord {
            session_id: st(s, "sessionId"), start_ms, end_ms, secs, outgoing: st(s, "direction").eq_ignore_ascii_case("outgoing"),
            disposition: st(s, "disposition"), name, email, callback, phone: st(o, "phoneNumber"), other_id: st(o, "id"),
            session_type: st(s, "sessionType"), conversation_url: st(&s["links"], "conversationUrl"),
        }
    }).collect();
    out.sort_by(|a, b| b.start_ms.cmp(&a.start_ms));
    out
}

/// `markAsDeleted` body (endTime in milliseconds).
pub fn delete_body(ids: &[(String, i64)]) -> Value {
    json!({"deleteSessionIds": ids.iter().map(|(s, e)| json!({"sessionId": s, "endTime": e})).collect::<Vec<_>>()})
}

pub fn calendar_url(cal: &str, now_ms: i64) -> String {
    format!("{cal}/calendarEvents?fromDate={}&toDate={}", iso_utc(now_ms - 86_400_000), iso_utc(now_ms + 7 * 86_400_000))
}

/// `{items:[event]}` -> events sorted by start; `dec(text, keyUrl)` decrypts one JWE field.
pub fn parse_calendar(v: &Value, mut dec: impl FnMut(&str, &str) -> String) -> Vec<CalEvent> {
    let mut out: Vec<CalEvent> = v["items"].as_array().into_iter().flatten().map(|e| {
        let key = st(e, "encryptionKeyUrl");
        let mut d = |s: String| if s.is_empty() { s } else { dec(&s, &key) };
        let start_ms = crate::i18n::parse_iso_ms(&st(e, "start")).unwrap_or(0);
        let end_ms = start_ms + e["durationMinutes"].as_i64().unwrap_or(0) * 60_000;
        let link = [st(&e["meetingJoinInfo"], "meetingJoinURL"), st(e, "webexURL"), st(e, "spaceMeetURL")].into_iter().find(|x| !x.is_empty()).map(&mut d).unwrap_or_default();
        CalEvent {
            id: st(e, "id"), title: d(st(e, "encryptedSubject")), start_ms, end_ms, organizer: d(st(&e["encryptedOrganizer"], "encryptedName")),
            location: d(st(e, "encryptedLocation")), link, space_url: st(e, "spaceURL"), recurring: e["isRecurring"].as_bool().unwrap_or(false),
            attendees: e["encryptedParticipants"].as_array().map_or(0, |a| a.len()),
        }
    }).collect();
    out.sort_by_key(|e| e.start_ms);
    out
}

/// Body of `POST /v1/meetings` (public API).
pub fn schedule_body(r: &ScheduleReq) -> Value {
    let mut b = json!({"title": r.title, "start": iso_utc(r.start_ms), "end": iso_utc(r.end_ms)});
    if !r.timezone.is_empty() { b["timezone"] = json!(r.timezone); }
    if !r.agenda.trim().is_empty() { b["agenda"] = json!(r.agenda); }
    let inv: Vec<Value> = r.invitees.iter().map(|e| e.trim()).filter(|e| e.contains('@')).map(|e| json!({"email": e})).collect();
    if !inv.is_empty() { b["invitees"] = json!(inv); }
    b
}

impl Session {
    pub(super) fn call_history(&mut self) -> Result<(), String> {
        let janus = self.url("janus", "");
        if janus.is_empty() { return Ok(()); } // no janus service in the catalog: nothing to show
        let v = self.http("GET", &history_url(&janus, crate::i18n::now_ms()), None)?;
        let list = parse_user_sessions(&v);
        dbg_conn(&format!("call history: {} sessions", list.len()));
        let _ = self.out.send(Resp::CallHistory(list));
        Ok(())
    }

    /// WRITE (only from the delete dialog's "Delete").
    pub(super) fn delete_calls(&mut self, ids: &[(String, i64)]) -> Result<(), String> {
        if ids.is_empty() { return Ok(()); }
        let url = self.url("janus", "/history/userSessions/markAsDeleted");
        if !webex_host(&url) { return Err(crate::trq!("Call history isn't reachable").into_owned()); }
        self.http("POST", &url, Some(delete_body(ids)))?;
        self.call_history()
    }

    pub(super) fn calendar(&mut self) -> Result<(), String> {
        let cal = self.url("calendar", "");
        if cal.is_empty() { return Ok(()); }
        let v = self.http("GET", &calendar_url(&cal, crate::i18n::now_ms()), None)?;
        let list = parse_calendar(&v, |t, k| self.decrypt(t, k));
        dbg_conn(&format!("calendar: {} events", list.len()));
        let _ = self.out.send(Resp::Calendar(list));
        Ok(())
    }

    /// WRITE (only from the scheduler's "Schedule").
    pub(super) fn schedule(&mut self, r: &ScheduleReq) -> Result<(), String> {
        let res = self.http("POST", &format!("{HYDRA}/meetings"), Some(schedule_body(r)));
        dbg_conn(&format!("schedule meeting -> {}", if res.is_ok() { "ok" } else { "failed" }));
        let _ = self.out.send(Resp::Scheduled(res.map(|v| st(&v, "webLink")).map_err(|e| super::report::friendly(&e))));
        Ok(())
    }

    pub(super) fn personal_room(&mut self) -> Result<(), String> {
        let res = self.http("GET", &format!("{HYDRA}/meetingPreferences/personalMeetingRoom"), None).map(|v| st(&v, "personalMeetingRoomLink"));
        let res = res.map_err(|e| super::report::friendly(&e));
        let _ = self.out.send(Resp::PersonalRoom(res.and_then(|l| if l.starts_with("https://") { Ok(l) } else { Err(crate::trq!("No personal room found.").into_owned()) })));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_and_urls() {
        assert_eq!(iso_utc(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        let u = history_url("https://janus-a.wbx2.com/janus/api/v1", 10 * 86_400_000 + 5);
        assert_eq!(u, "https://janus-a.wbx2.com/janus/api/v1/history/userSessions?from=1970-01-01T00:00:00.005Z&limit=50&includeNewSessionTypes=true&sort=DESC");
        let c = calendar_url("https://calendar-a.wbx2.com/calendar/api/v1", 86_400_000);
        assert_eq!(c, "https://calendar-a.wbx2.com/calendar/api/v1/calendarEvents?fromDate=1970-01-01T00:00:00.000Z&toDate=1970-01-09T00:00:00.000Z");
    }

    #[test]
    fn user_sessions_parse() {
        // Shape of a janus UserSession; values invented.
        let v = json!({"userSessions": [
            {"sessionId": "s1", "startTime": "2026-11-13T14:50:59.076Z", "endTime": "2026-11-13T14:52:42.513Z", "direction": "OUTGOING", "disposition": "CANCELED",
             "durationSeconds": 0, "sessionType": "SPARK", "other": {"id": "o1", "name": "Peer Person", "callbackAddress": "peer@example.com", "isPrivate": false},
             "links": {"locusUrl": "https://locus-a.wbx2.com/x", "conversationUrl": "https://conv-a.wbx2.com/c/1", "callbackAddress": "peer@example.com"}},
            {"sessionId": "s2", "startTime": "2026-11-13T15:11:20.314Z", "endTime": "2026-11-13T15:11:23.523Z", "direction": "INCOMING", "disposition": "MISSED",
             "sessionType": "SPARK", "other": {"id": "o2", "name": "", "primaryDisplayString": "+41 44 000 00 00", "phoneNumber": "+41440000000", "callbackAddress": "", "isPrivate": false}},
            {"sessionId": "s3", "startTime": "2026-11-12T10:00:00.000Z", "endTime": "2026-11-12T10:05:00.000Z", "direction": "INCOMING", "disposition": "Answered",
             "durationSeconds": 300, "isDeleted": false, "other": {"id": "o3", "name": "C", "email": "c@example.com", "callbackAddress": "x", "isPrivate": false}},
            {"sessionId": "gone", "startTime": "2026-11-12T10:00:00.000Z", "endTime": "2026-11-12T10:05:00.000Z", "isDeleted": true, "other": {}}]});
        let r = parse_user_sessions(&v);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].session_id, "s2"); // newest first
        assert!(r[0].missed() && r[0].email.is_empty() && r[0].name == "+41 44 000 00 00" && r[0].secs == 3);
        assert!(r[1].outgoing && !r[1].missed() && r[1].email == "peer@example.com" && r[1].conversation_url.ends_with("/c/1"));
        assert_eq!((r[2].secs, r[2].email.as_str(), r[2].missed()), (300, "c@example.com", false));
    }

    #[test]
    fn delete_body_matches_sdk() {
        let b = delete_body(&[("s1".into(), 1_794_822_934_000)]);
        assert_eq!(b, json!({"deleteSessionIds": [{"sessionId": "s1", "endTime": 1_794_822_934_000_i64}]}));
    }

    #[test]
    fn calendar_parse_decrypts_fields() {
        let v = json!({"items": [
            {"id": "e2", "start": "2026-11-17T09:00:00.000Z", "durationMinutes": 30, "encryptionKeyUrl": "kms://k/1", "encryptedSubject": "ENC(Second)"},
            {"id": "e1", "start": "2026-11-16T13:00:00.000Z", "durationMinutes": 60, "encryptionKeyUrl": "kms://k/1", "encryptedSubject": "ENC(Weekly)",
             "encryptedLocation": "ENC(Room)", "webexURL": "ENC(https://x.webex.com/meet/a)", "spaceMeetURL": "ENC(https://y)", "isRecurring": true,
             "encryptedOrganizer": {"encryptedName": "ENC(Org)"}, "encryptedParticipants": [{}, {}]}]});
        let mut keys = vec![];
        let r = parse_calendar(&v, |t, k| { keys.push(k.to_string()); t.trim_start_matches("ENC(").trim_end_matches(')').to_string() });
        assert_eq!(r.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["e1", "e2"]);
        assert_eq!((r[0].title.as_str(), r[0].location.as_str(), r[0].organizer.as_str(), r[0].link.as_str()), ("Weekly", "Room", "Org", "https://x.webex.com/meet/a"));
        assert_eq!((r[0].end_ms - r[0].start_ms, r[0].recurring, r[0].attendees), (3_600_000, true, 2));
        assert!(keys.iter().all(|k| k == "kms://k/1"));
        assert!(parse_calendar(&json!({"items": []}), |t, _| t.into()).is_empty()); // an empty calendar answer
    }

    #[test]
    fn schedule_body_shape() {
        let r = ScheduleReq { title: "Sync".into(), start_ms: 3_600_000, end_ms: 5_400_000, agenda: " ".into(), invitees: vec!["a@b.example".into(), "nope".into()], timezone: "Europe/Zurich".into() };
        assert_eq!(schedule_body(&r), json!({"title": "Sync", "start": "1970-01-01T01:00:00.000Z", "end": "1970-01-01T01:30:00.000Z", "timezone": "Europe/Zurich", "invitees": [{"email": "a@b.example"}]}));
    }
}
