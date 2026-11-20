//! Typing indicators, both directions.
//!
//! Sending:
//! * `{"conversationId": <id>, "eventType": "status.start_typing"}` POSTed to the conversation's `url` + `/status/typing`.
//! * `status.stop_typing` is never sent.
//! * When: on every composer change; empty text resets the timer; otherwise, if the 5000 ms timer is not running (and no
//!   message is being edited), it sends and starts the timer. So: at most one start every 5 s while text is being typed.
//!   (The UI side of this is ui/typing.rs.)
//!
//! Receiving:
//! * Mercury `status.start_typing` / `status.stop_typing` with `data.conversationId` and `data.actor`; the actor may be an
//!   object with `id` or a plain id, both are accepted.
//! * The service only delivers them to a device registered for the conversation: a POST with an empty body to the
//!   conversation url + `/status/register`, sent when a conversation is shown.
//! * Every `conversation.activity` event carries `data.conversation.registrationTTLMs`; the registration ends at
//!   `now + ttl * rand(85..99) / 100`. A conversation is registered again when it is opened and that end has passed.
//! * Expiry: a typer is kept until `status.stop_typing`, their message, or 10 s without a new start. Senders repeat the
//!   start every 5 s, so a typer stays visible while typing.
use super::*;

pub const SEND_EVERY_MS: i64 = 5000;
pub const TYPER_EXPIRY_MS: i64 = 10_000;
pub const START: &str = "status.start_typing";
pub const STOP: &str = "status.stop_typing";

/// The body of `POST <conversation url>/status/typing`.
pub fn typing_body(conversation: &str) -> Value { json!({"conversationId": conversation, "eventType": START}) }

/// A typing event → (conversation, actor id, typing). None for anything else or a missing key.
pub fn parse_event(data: &Value) -> Option<(String, String, bool)> {
    let on = match data["eventType"].as_str()? { START => true, STOP => false, _ => return None };
    let conv = data["conversationId"].as_str().filter(|s| !s.is_empty())?.to_string();
    let a = &data["actor"];
    let actor = a.as_str().map(String::from).or_else(|| ["id", "entryUUID"].iter().map(|k| st(a, k)).find(|s| !s.is_empty()))?;
    Some((conv, actor, on))
}

/// When the registration from a `registrationTTLMs` ends: now + ttl × 0.85…0.99 (`factor` = the random 85..99).
pub fn registration_end(now_ms: i64, ttl_ms: i64, factor: i64) -> i64 { now_ms + ttl_ms * factor / 100 }

#[derive(Default)]
pub struct TypingState {
    /// Conversation id → its `url` (from the list answer).
    pub conv_url: HashMap<String, String>,
    /// Conversation id → end of my typing registration (Unix ms).
    registered_until: HashMap<String, i64>,
}

fn now_ms() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) }

impl Session {
    /// The conversation's own url (the list's `url`), else the conversation service's.
    fn conv_url(&self, id: &str) -> String {
        self.typing.conv_url.get(id).filter(|u| u.starts_with("https://") && webex_host(u)).cloned()
            .unwrap_or_else(|| self.url("conversation", &format!("/conversations/{id}")))
    }

    /// I am typing in `space` (the UI throttles to one per 5 s). WRITE, the user's own typing.
    pub(super) fn send_typing(&mut self, space: &str) -> Result<(), String> {
        let url = format!("{}/status/typing", self.conv_url(space));
        if !webex_host(&url) { return Ok(()) }
        if let Err(e) = self.http("POST", &url, Some(typing_body(space))) { dbg_conn(&format!("typing: send failed ({})", http_status(&e).map_or("transport".into(), |c| c.to_string()))); }
        Ok(())
    }

    /// A conversation is shown: register for its typing events unless the last registration still runs.
    pub(super) fn register_typing(&mut self, space: &str) {
        if self.typing.registered_until.get(space).is_some_and(|t| *t > now_ms()) { return }
        let url = format!("{}/status/register", self.conv_url(space));
        if !webex_host(&url) { return }
        match self.http("POST", &url, None) {
            // The answer's TTL (if any) is not known; the next activity's registrationTTLMs sets the real end.
            Ok(_) => { self.typing.registered_until.insert(space.to_string(), now_ms() + 60_000); }
            Err(e) => dbg_conn(&format!("typing: register failed ({})", http_status(&e).map_or("transport".into(), |c| c.to_string()))),
        }
    }

    /// `data.conversation.registrationTTLMs` of an activity event.
    pub(super) fn typing_ttl(&mut self, space: &str, ttl_ms: i64) {
        use rand::Rng;
        let end = registration_end(now_ms(), ttl_ms, rand::thread_rng().gen_range(85..=99));
        self.typing.registered_until.insert(space.to_string(), end);
    }

    /// A Mercury typing event → the UI (someone else; my own other devices are ignored).
    pub(super) fn typing_event(&mut self, data: &Value) {
        let Some((space, actor, on)) = parse_event(data) else { return };
        if actor == self.user || self.actor_is_me(&json!({"id": actor})) { return }
        // The participant behind the uuid (participants carry e-mail-style ids next to it).
        let person = self.spaces.get(&space).and_then(|s| s.participants.iter().find(|p| p.id == actor || self.uuids.get(&p.id).is_some_and(|u| *u == actor)).cloned())
            .unwrap_or(Person { id: actor, ..Default::default() });
        let _ = self.out.send(Resp::Typing { space, person, on });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_as_the_original_sends_it() {
        assert_eq!(typing_body("c-1"), json!({"conversationId": "c-1", "eventType": "status.start_typing"}));
    }

    #[test]
    fn inbound_events() {
        assert_eq!(parse_event(&json!({"eventType": "status.start_typing", "conversationId": "c", "actor": {"id": "u-2"}})), Some(("c".into(), "u-2".into(), true)));
        assert_eq!(parse_event(&json!({"eventType": "status.stop_typing", "conversationId": "c", "actor": "u-2"})), Some(("c".into(), "u-2".into(), false)));
        assert_eq!(parse_event(&json!({"eventType": "status.start_typing", "actor": {"id": "u-2"}})), None, "missing conversation id");
        assert_eq!(parse_event(&json!({"eventType": "status.start_typing", "conversationId": "c"})), None, "missing actor");
        assert_eq!(parse_event(&json!({"eventType": "conversation.activity", "conversationId": "c", "actor": "x"})), None);
    }

    #[test]
    fn registration_end_is_85_to_99_percent_of_the_ttl() {
        assert_eq!(registration_end(1_000, 100_000, 85), 86_000);
        assert_eq!(registration_end(1_000, 100_000, 99), 100_000);
    }

    #[test]
    fn typing_events_reach_the_ui_except_my_own() {
        let (mut s, rx) = super::super::live_list::tests::session();
        s.spaces.insert("c".into(), Space { id: "c".into(), participants: vec![Person { id: "erika@example.com".into(), name: "Erika Beispiel".into(), email: "erika@example.com".into(), status: String::new() }], ..Default::default() });
        s.uuids.insert("erika@example.com".into(), "u-2".into());
        s.typing_event(&json!({"eventType": "status.start_typing", "conversationId": "c", "actor": {"id": "u-2"}}));
        s.typing_event(&json!({"eventType": "status.start_typing", "conversationId": "c", "actor": {"id": "me"}}));
        let r: Vec<Resp> = rx.try_iter().collect();
        assert_eq!(r.len(), 1);
        assert!(matches!(&r[0], Resp::Typing { space, person, on: true } if space == "c" && person.name == "Erika Beispiel"));
    }
}
