//! The usersub presence subscription as the Webex desktop client keeps it (SubscriptionManager / SubscriptionAdapter in
//! spark-windows-desktop-ui.dll 46.11):
//! * `POST usersub/subscribe` with the header `Cisco-Usersub-Device-Caps: subs:2` and the body
//!   `{"users": [{"user": <uuid>, "refresh": true?}], "ttl": 600}`.
//! * After a successful subscribe a contact counts as subscribed for 540 s (it is renewed before then).
//! * `refresh: true` asks the server to push the contact's current state again. It is set for a contact that is not in
//!   the book (never subscribed, or forgotten after a socket drop / resume from sleep / eviction) and for one whose
//!   subscription lapsed more than 60 s ago. Without it a renewal only brings changes from then on, so a state that changed
//!   while the subscription was gone stays stale.
//! * The server can evict subscriptions (`usersub.subscription_status` `statuses[]`, or `subStatuses[]` on a
//!   `usersub.userstate`): an entry whose status is not "success" and that says `evicted: true`. Evicted contacts are
//!   forgotten, so the next renewal subscribes them again with `refresh`.
use serde_json::{json, Value};
use std::collections::HashMap;

/// The ttl asked for (s).
pub const SUB_TTL_S: i64 = 600;
/// How long a successful subscribe counts (ms): 540 s, renewed before the 600 s ttl runs out.
pub const SUB_VALID_MS: i64 = 540_000;
/// A subscription that lapsed longer than this (ms) is renewed with `refresh`.
pub const SUB_LATE_MS: i64 = 60_000;

/// When each contact's subscription runs out (Unix ms), by uuid.
#[derive(Default, Debug)]
pub struct Subs { until: HashMap<String, i64> }

impl Subs {
    /// Whether the subscribe for `uuid` asks for its current state again.
    pub fn refresh(&self, uuid: &str, now: i64) -> bool { self.until.get(uuid).is_none_or(|u| now > u + SUB_LATE_MS) }
    /// The service accepted the subscribe for `uuids` at `now`.
    pub fn subscribed(&mut self, uuids: &[String], now: i64) { for u in uuids { self.until.insert(u.clone(), now + SUB_VALID_MS); } }
    /// Socket gone / back: nothing counts as subscribed any more (the next subscribe refreshes everyone).
    pub fn forget_all(&mut self) { self.until.clear(); }
    /// The server dropped these subscriptions.
    pub fn forget(&mut self, uuids: &[String]) { for u in uuids { self.until.remove(u); } }
}

/// The subscribe body: one entry per contact, `refresh` only where asked.
pub fn subscribe_body(users: &[(String, bool)]) -> Value {
    let users: Vec<Value> = users.iter().map(|(u, r)| if *r { json!({"user": u, "refresh": true}) } else { json!({"user": u}) }).collect();
    json!({"users": users, "ttl": SUB_TTL_S})
}

/// The contacts a `usersub.subscription_status` (`statuses[]`) or `usersub.userstate` (`subStatuses[]`) says were evicted.
pub fn evicted(data: &Value) -> Vec<String> {
    ["statuses", "subStatuses"].iter().filter_map(|k| data[*k].as_array()).flatten()
        .filter(|s| !s["status"].as_str().is_some_and(|x| x.eq_ignore_ascii_case("success")) && s["evicted"].as_bool() == Some(true))
        .filter_map(|s| s["user"].as_str().map(str::to_string)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: &str = "00000000-0000-4000-8000-00000000000a";
    const B: &str = "00000000-0000-4000-8000-00000000000b";

    #[test]
    fn refresh_when_new_lapsed_or_forgotten() {
        let mut s = Subs::default();
        let t0 = 1_000_000_000_000;
        assert!(s.refresh(A, t0), "never subscribed");
        s.subscribed(&[A.into(), B.into()], t0);
        assert!(!s.refresh(A, t0 + 540_000), "the regular renewal");
        assert!(!s.refresh(A, t0 + 540_000 + 60_000), "a little late is fine");
        assert!(s.refresh(A, t0 + 540_000 + 60_001), "lapsed more than a minute: changes were missed");
        s.forget(&[A.into()]);
        assert!(s.refresh(A, t0 + 1) && !s.refresh(B, t0 + 1), "evicted: only that one");
        s.forget_all();
        assert!(s.refresh(B, t0 + 1), "after a socket drop everyone");
    }

    #[test]
    fn body_as_the_desktop_client_sends_it() {
        assert_eq!(subscribe_body(&[(A.into(), true), (B.into(), false)]),
            json!({"users": [{"user": A, "refresh": true}, {"user": B}], "ttl": 600}));
    }

    #[test]
    fn only_evicted_failures_count() {
        let ev = json!({"eventType": "usersub.subscription_status", "statuses": [
            {"user": A, "status": "evicted", "evicted": true}, {"user": B, "status": "success", "ttl": 600},
            {"user": "c", "status": "failure", "evicted": false}]});
        assert_eq!(evicted(&ev), vec![A.to_string()]);
        let st = json!({"eventType": "usersub.userstate", "user": B, "subStatuses": [{"user": B, "status": "Failed", "evicted": true}]});
        assert_eq!(evicted(&st), vec![B.to_string()]);
        assert!(evicted(&json!({"user": A, "compositions": []})).is_empty());
    }
}
