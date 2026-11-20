//! After my own write (post, reply, edit, quote, GIF, delete, reaction): apply what the service answered instead of reloading
//! the space. The POST answer is merged into the space's raw cache and the cache is folded again (no request); the mercury
//! event of the same activity arrives shortly after and is de-duplicated by id. Only when that event hasn't arrived within
//! `ECHO_WAIT` is the newest page loaded again. Participants (read marks) are fetched again only when the membership changed.
use super::*;

impl Session {
    /// What follows a successful write to `space`.
    pub(crate) fn after_write(&mut self, space: &str) -> Result<(), String> {
        let posted = self.hist.posted.take();
        if !self.hist.acts.contains_key(space) { return self.messages(space) } // nothing loaded to apply it to: load it
        let Some((answer, temp)) = posted else { return self.reload_newest(space) };
        let act = st(&answer, "id");
        if !act.is_empty() && answer["verb"].is_string() {
            // A delete whose target isn't in the cache is a reaction counted only inside a summary: its name isn't known
            // here, so the newest page (with the service's new summary) is loaded instead.
            if answer["verb"] == "delete" && !self.cached(space, &st(&answer["object"], "id")) { return self.reload_newest(space) }
            // Its live event came first (already merged and shown): nothing left to wait for.
            if self.merge_acts(space, vec![answer]) == 0 { return Ok(()) }
            self.refold(space);
        }
        // An answer that isn't the activity (unexpected) shows nothing yet: the live event will, else the reload after ECHO_WAIT.
        self.hist.echo.push(Echo { space: space.to_string(), id: act, temp, due: std::time::Instant::now() + ECHO_WAIT });
        Ok(())
    }

    /// Whether an activity id is in a space's raw cache.
    pub(super) fn cached(&self, space: &str, id: &str) -> bool {
        !id.is_empty() && self.hist.acts.get(space).is_some_and(|l| l.iter().any(|a| st(a, "id") == id))
    }

    /// Folds a space's raw cache again and hands the list to the UI (no request; decrypted texts come from the cache).
    pub(super) fn refold(&mut self, space: &str) {
        let key_url = self.spaces.get(space).map(|s| s.key_url.clone()).unwrap_or_default();
        let acts = self.hist.acts.get(space).cloned().unwrap_or_default();
        let msgs = self.fold_in(space, &acts, &key_url);
        if let Some(m) = msgs.first() { self.hist.first.insert(space.to_string(), m.id.clone()); }
        let _ = self.out.send(Resp::Messages(space.to_string(), msgs));
        self.fetch_images();
    }

    /// The fallback: the newest page again (merged into what is known, folded), without participants / covers.
    pub(super) fn reload_newest(&mut self, space: &str) -> Result<(), String> {
        let v = self.http("GET", &self.url("conversation", &newest_query(space, now_ms())), None)?;
        self.merge_acts(space, v["items"].as_array().cloned().unwrap_or_default());
        self.refold(space);
        Ok(())
    }

    /// A live event arrived: it confirms my write with the same activity id or clientTempId.
    pub(super) fn echo_arrived(&mut self, a: &Value) {
        let (id, temp) = (st(a, "id"), st(a, "clientTempId"));
        self.hist.echo.retain(|e| !((!e.id.is_empty() && e.id == id) || (!e.temp.is_empty() && e.temp == temp)));
    }

    /// Spaces whose write wasn't confirmed by its live event in time (each once); called from the job loop's idle tick.
    pub(crate) fn echo_overdue(&mut self) -> Result<(), String> {
        for space in take_overdue(&mut self.hist.echo, std::time::Instant::now()) {
            dbg_conn("live event of my write didn't arrive: reloading the space's newest page");
            self.reload_newest(&space)?;
        }
        Ok(())
    }
}

/// Removes the waits that are due at `now` and returns their spaces (de-duplicated, in order).
pub(super) fn take_overdue(echo: &mut Vec<Echo>, now: std::time::Instant) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    echo.retain(|e| if e.due <= now { if !out.contains(&e.space) { out.push(e.space.clone()); } false } else { true });
    out
}

/// A live activity that changes who is in the space (add / leave / remove of a person): the read marks are fetched again.
pub(super) fn membership_change(a: &Value) -> bool {
    matches!(a["verb"].as_str(), Some("add" | "leave" | "remove")) && a["object"]["objectType"] == "person"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Receiver;

    /// A session without any service address: every request fails at once (so a test sees any reload as an error), nothing
    /// goes over the network. Texts are plain (not encrypted), which `decrypt` passes through.
    fn session() -> (Session, Receiver<Resp>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ktx, krx) = std::sync::mpsc::channel();
        let s = Session { hist: Default::default(), space_cover_urls: Default::default(), space_covers_done: Default::default(), full_desc: Default::default(), read_marks: Default::default(), covers_done: Default::default(), oracle_done: false, calls: Default::default(), img_jobs: vec![], images_done: Default::default(), uuids: Default::default(), avatars_done: Default::default(), agent: ureq::agent(), token: String::new(), my_email: String::new(), kms_token: String::new(), kms_loop: ktx, svc: Default::default(), device: String::new(),
            user: "me".into(), kms: None, keys: Default::default(), kms_rx: krx, spaces: Default::default(), out: tx, cover: String::new(), keep: Default::default(), live: Default::default(), extra: Default::default(), typing: Default::default() };
        (s, rx)
    }
    fn post(id: &str, text: &str, at: &str, actor: &str) -> Value {
        json!({"id": id, "verb": "post", "published": at, "actor": {"id": actor}, "target": {"id": "sp"}, "object": {"objectType": "comment", "displayName": text}})
    }
    /// The messages of the last `Resp::Messages` sent, and how many such answers there were.
    fn lists(rx: &Receiver<Resp>) -> (usize, Vec<Message>) {
        let mut n = 0; let mut last = vec![];
        for r in rx.try_iter() { if let Resp::Messages(_, m) = r { n += 1; last = m; } }
        (n, last)
    }

    #[test]
    fn a_send_is_applied_from_the_answer_and_its_echo_only_deduplicated() {
        let (mut s, rx) = session();
        s.merge_acts("sp", vec![post("a1", "hallo", "2026-11-17T09:00:00.000Z", "other")]);
        s.hist.posted = Some((post("a2", "meins", "2026-11-17T09:01:00.000Z", "me"), "tmp-1".into()));
        assert!(s.after_write("sp").is_ok(), "no request may be made (a reload would fail here)");
        let (n, msgs) = lists(&rx);
        assert_eq!(n, 1);
        assert_eq!(msgs.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["hallo", "meins"]);
        assert_eq!(s.hist.echo.len(), 1, "waits for the live event");
        // The live event of the same activity: confirmed, nothing sent again.
        assert!(s.incoming(&post("a2", "meins", "2026-11-17T09:01:00.000Z", "me")).is_ok());
        assert!(s.hist.echo.is_empty());
        assert!(rx.try_iter().next().is_none(), "the echo is only de-duplicated");
    }

    #[test]
    fn the_echo_matches_by_client_temp_id_too_and_a_late_one_needs_no_wait() {
        let (mut s, rx) = session();
        s.merge_acts("sp", vec![post("a1", "x", "2026-11-17T09:00:00.000Z", "other")]);
        // An answer that isn't the activity: nothing to show yet, wait for the event (matched by clientTempId).
        s.hist.posted = Some((json!({}), "tmp-9".into()));
        assert!(s.after_write("sp").is_ok());
        assert_eq!(s.hist.echo.len(), 1);
        let mut ev = post("a3", "gif", "2026-11-17T09:02:00.000Z", "me");
        ev["clientTempId"] = json!("tmp-9");
        assert!(s.incoming(&ev).is_ok());
        assert!(s.hist.echo.is_empty());
        assert!(rx.try_iter().any(|r| matches!(r, Resp::Incoming(_, m) if m.id == "a3")), "a new top-level message is still shown");
        // The event came before the answer was applied: already known, no wait left behind.
        s.hist.posted = Some((post("a3", "gif", "2026-11-17T09:02:00.000Z", "me"), "tmp-9".into()));
        assert!(s.after_write("sp").is_ok());
        assert!(s.hist.echo.is_empty());
    }

    #[test]
    fn an_edit_and_a_reply_are_folded_in_without_a_request() {
        let (mut s, rx) = session();
        s.merge_acts("sp", vec![post("a1", "alt", "2026-11-17T09:00:00.000Z", "me")]);
        let mut edit = post("e1", "neu", "2026-11-17T09:05:00.000Z", "me");
        edit["parent"] = json!({"id": "a1", "type": "edit"});
        s.hist.posted = Some((edit, "t".into()));
        assert!(s.after_write("sp").is_ok());
        let (_, msgs) = lists(&rx);
        assert_eq!((msgs[0].text.as_str(), msgs[0].edited, msgs.len()), ("neu", true, 1));
        // Someone else's live reply: merged and folded, no reload.
        let mut reply = post("r1", "antwort", "2026-11-17T09:06:00.000Z", "other");
        reply["parent"] = json!({"id": "a1", "type": "reply"});
        assert!(s.incoming(&reply).is_ok());
        let (n, msgs) = lists(&rx);
        assert_eq!(n, 1);
        assert!(msgs.iter().any(|m| m.id == "r1" && m.parent == "a1"));
    }

    #[test]
    fn a_live_reaction_counts_once_next_to_the_summary() {
        let (mut s, rx) = session();
        let summary = json!({"id": "s1", "verb": "add", "published": "2026-11-17T09:01:00.000Z", "parent": {"id": "a1", "type": "reactionSummary"},
            "object": {"objectType": "reaction2Summary", "reactions": [{"displayName": "heart", "count": 2, "users": [{"id": "x"}, {"id": "y"}]}]}});
        s.merge_acts("sp", vec![post("a1", "hi", "2026-11-17T09:00:00.000Z", "other"), summary]);
        let react = |id: &str, at: &str| json!({"id": id, "verb": "add", "published": at, "actor": {"id": "z"}, "parent": {"id": "a1", "type": "reaction"},
            "target": {"id": "sp"}, "object": {"objectType": "reaction2", "displayName": "heart"}});
        // Older than the summary: already counted in it.
        assert!(s.incoming(&react("r-old", "2026-11-17T09:00:30.000Z")).is_ok());
        let (_, msgs) = lists(&rx);
        assert_eq!(msgs[0].reactions[0].count, 2);
        // Newer: on top of the summary.
        assert!(s.incoming(&react("r-new", "2026-11-17T09:02:00.000Z")).is_ok());
        let (_, msgs) = lists(&rx);
        assert_eq!(msgs[0].reactions[0].count, 3);
    }

    #[test]
    fn overdue_waits_are_taken_once_per_space() {
        let now = std::time::Instant::now();
        let e = |sp: &str, due| Echo { space: sp.into(), id: String::new(), temp: String::new(), due };
        let mut w = vec![e("a", now), e("a", now), e("b", now + std::time::Duration::from_secs(5))];
        assert_eq!(take_overdue(&mut w, now), ["a"]);
        assert_eq!(w.len(), 1);
        assert!(take_overdue(&mut w, now).is_empty());
    }

    #[test]
    fn only_membership_changes_refetch_the_participants() {
        assert!(membership_change(&json!({"verb": "add", "object": {"objectType": "person"}})));
        assert!(membership_change(&json!({"verb": "leave", "object": {"objectType": "person"}})));
        assert!(!membership_change(&json!({"verb": "add", "object": {"objectType": "reaction2"}})));
        assert!(!membership_change(&json!({"verb": "post", "object": {"objectType": "comment"}})));
    }
}
