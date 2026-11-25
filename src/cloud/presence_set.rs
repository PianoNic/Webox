//! Setting my OWN presence (Available / Away / Do not disturb + duration). WRITE on the real account,
//! only ever sent on the user's own click in the status menu; tests check the body only.
//!
//! Protocol: `POST apheleia /events` with `{subject: device.userId, eventType: status, label: device.userId (not for "dnd"),
//! ttl}`; status is one of `active | inactive | ooo | dnd`, ttl in seconds. "Until turned off" and a user-set
//! "Away" (no end) use ttl -1, the value apheleia itself uses for "no known expiration".
use super::*;

/// TTL of a plain "Available" (s).
pub const ACTIVE_TTL: i64 = 600;
/// TTL for "no end".
pub const NO_END_TTL: i64 = -1;

/// The body of `POST apheleia /events`.
pub fn status_event_body(user: &str, event_type: &str, ttl: i64) -> Value {
    let mut b = json!({"subject": user, "eventType": event_type, "ttl": ttl});
    if event_type != "dnd" { b["label"] = json!(user); }
    b
}

/// The UI's status name → apheleia's event type (active | inactive | ooo | dnd).
pub fn event_type(status: &str) -> &'static str {
    match status { "DoNotDisturb" => "dnd", "inactive" => "inactive", "OutOfOffice" => "ooo", _ => "active" }
}

/// The user feature toggle behind "Show availability and status".
pub const PRESENCE_FEATURE: &str = "user-presence-enabled";

/// Body of `POST feature /features/users/<userId>/user`.
pub fn feature_body(key: &str, val: Value) -> Value { json!({"key": key, "mutable": true, "val": val}) }

fn now_ms() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) }

impl Session {
    /// Sets my presence and, once the service accepted it, tells the UI (my status + its end).
    pub(super) fn set_presence(&mut self, status: &str, ttl: i64) -> Result<(), String> {
        if self.user.is_empty() { return Err("presence: no user id yet".into()); }
        let body = status_event_body(&self.user, event_type(status), ttl);
        self.http("POST", &self.url("apheleia", "/events"), Some(body))?;
        let until = (ttl > 0 && status != "active").then(|| now_ms() + ttl * 1000);
        let ids: Vec<String> = [self.my_email.clone(), self.user.clone()].into_iter().filter(|s| !s.is_empty()).collect();
        super::people::send_presence(&self.out, ids.iter().map(|i| (i.clone(), status.to_string(), now_ms())).collect());
        let _ = self.out.send(Resp::PresenceDetail(ids.iter().map(|i| (i.clone(), until, String::new())).collect()));
        Ok(())
    }

    /// The keepalive's tick (job loop, about once a second): learns my latest status from the UI, and sends Active with ttl 600
    /// when the keepalive says so (see cloud/keepalive.rs). Not before my user id is known.
    pub(super) fn keepalive_tick(&mut self) {
        let now = now_ms();
        if let Some(st) = keepalive::take_self_status() { self.keep.on_self_status(&st, now, keepalive::random_delay()); }
        if self.user.is_empty() { return }
        if self.keep.tick(now, keepalive::input_idle_s(now), keepalive::random_delay()) {
            dbg_conn("presence keepalive: Active, ttl 600");
            if let Err(e) = self.set_presence("active", ACTIVE_TTL) { dbg_conn(&format!("presence keepalive failed: {}", http_status(&e).map_or("transport".into(), |c| c.to_string()))); }
        }
    }

    /// Sets one of MY user feature toggles (WRITE, only on the user's click in Settings).
    pub(super) fn set_feature(&mut self, key: &str, val: Value) -> Result<(), String> {
        if self.user.is_empty() { return Err("feature: no user id yet".into()); }
        let url = self.url("feature", &format!("/features/users/{}/user", self.user));
        if !webex_host(&url) { return Err("feature: no feature service".into()); }
        self.http("POST", &url, Some(feature_body(key, val))).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_as_the_sdk_sends_it() {
        assert_eq!(status_event_body("u-1", "active", 600), json!({"subject": "u-1", "eventType": "active", "label": "u-1", "ttl": 600}));
        // "dnd" carries no label.
        assert_eq!(status_event_body("u-1", "dnd", 1800), json!({"subject": "u-1", "eventType": "dnd", "ttl": 1800}));
        assert_eq!(status_event_body("u-1", "inactive", -1)["label"], "u-1");
    }

    #[test]
    fn feature_toggle_body() {
        assert_eq!(feature_body(PRESENCE_FEATURE, json!(false)), json!({"key": "user-presence-enabled", "mutable": true, "val": false}));
    }

    #[test]
    fn ui_status_to_event_type() {
        assert_eq!(event_type("DoNotDisturb"), "dnd");
        assert_eq!(event_type("inactive"), "inactive");
        assert_eq!(event_type("active"), "active");
        assert_eq!(event_type("OutOfOffice"), "ooo");
    }
}
