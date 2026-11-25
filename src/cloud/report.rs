//! What becomes of a job's error. Background work (Mercury events, call state reads, refreshes the app starts by itself)
//! only goes to the debug log; what the user started shows a short message, never a URL, response body or
//! tracking id.
use super::{Job, Req};

/// Whether the user started this job (a click / Enter), so a failure is worth a message.
pub(super) fn user_started(job: &Job) -> bool {
    match job {
        Job::Ui(r) => !matches!(r, Req::Spaces | Req::Messages(_) | Req::Meetings | Req::Ack(..) | Req::FullImage(_) | Req::MessagesBefore(_)
            | Req::DirectorySearch(_) | Req::Flags | Req::CallHistory | Req::Calendar | Req::Token { .. } | Req::Typing(_)),
        _ => false,
    }
}

/// The message for the UI (`None`: log only), and the log line (no tokens; addresses removed).
pub(super) fn report(user: bool, e: &str) -> (Option<String>, String) {
    let log = format!("{} failed: {}", if user { "request" } else { "background request" }, log_safe(e));
    (user.then(|| friendly(e)), log)
}

/// An error without anything that looks like an address, cut to 160 characters.
fn log_safe(e: &str) -> String {
    e.split_whitespace().map(|w| if w.contains('@') { "<addr>" } else { w }).collect::<Vec<_>>().join(" ").chars().take(160).collect()
}

/// A short text for an error. Our own messages stay; a raw HTTP / transport error ("GET https://… → 429 {…}")
/// becomes a sentence, after whatever prefix the message had ("Call failed: …").
pub(super) fn friendly(e: &str) -> String {
    let raw_at = ["GET ", "POST ", "PUT ", "DELETE ", "PATCH ", "https://", "http://"].iter().filter_map(|p| e.find(p)).min();
    let Some(at) = raw_at else { return e.chars().take(160).collect() };
    let prefix = e[..at].trim().trim_end_matches(':').trim();
    let code = e[at..].split(" → ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|c| c.parse::<u16>().ok());
    let text = match code {
        Some(429 | 503) => crate::trq!("Webex is busy right now. Please try again in a moment.").into_owned(),
        Some(401 | 403) => crate::trq!("Don’t have permission").into_owned(),
        Some(404) => crate::trq!("Not found.").into_owned(),
        Some(c) => crate::trq!("Webex answered with error {{0}}.", c),
        None => crate::trq!("No connection to Webex.").into_owned(),
    };
    if prefix.is_empty() { text } else { format!("{prefix}: {text}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THROTTLED: &str = "GET https://locus-a.example.com/locus/api/v1/loci/123 → 429 {\"message\":\"Too Many Requests\",\"trackingId\":\"CLIENT_abc\"}";

    #[test]
    fn a_background_429_is_logged_not_shown() {
        let jobs = [Job::Locus(serde_json::json!({"tick": true})), Job::Activity(serde_json::Value::Null), Job::Ui(Req::Spaces), Job::Ui(Req::CallHistory)];
        for j in &jobs {
            assert!(!user_started(j));
            let (ui, log) = report(user_started(j), THROTTLED);
            assert!(ui.is_none());
            assert!(log.starts_with("background request failed: GET") && !log.contains("Bearer"));
        }
    }

    #[test]
    fn a_user_action_shows_a_short_text() {
        assert!(user_started(&Job::Ui(Req::Send("s".into(), "t".into()))));
        let ui = report(true, THROTTLED).0.unwrap();
        assert_eq!(ui, crate::trq!("Webex is busy right now. Please try again in a moment."));
        let ui = friendly("Call failed: POST https://locus-a.example.com/locus/api/v1/loci/call → 400 {\"trackingId\":\"x\"}");
        assert_eq!(ui, format!("Call failed: {}", crate::trq!("Webex answered with error {{0}}.", 400)));
        assert_eq!(friendly("https://conv-a.example.com/x: Connection Failed: tls"), crate::trq!("No connection to Webex."));
        assert_eq!(friendly("A call is already in progress."), "A call is already in progress.");
        for s in [ui.as_str()] { assert!(!s.contains("http") && !s.contains("tracking")); }
        assert!(log_safe("GET https://x/people/a@example.com → 404").contains("<addr>"));
    }
}
