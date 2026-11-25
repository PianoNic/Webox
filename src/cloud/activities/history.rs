//! Loading a space: the first page, older pages, the cover picture, read marks and incoming activities.
use super::*;

impl Session {
    pub(crate) fn messages(&mut self, id: &str) -> Result<(), String> {
        let key_url = self.spaces.get(id).map(|s| s.key_url.clone()).unwrap_or_default();
        let v = self.http("GET", &self.url("conversation", &newest_query(id, now_ms())), None)?;
        self.merge_acts(id, v["items"].as_array().cloned().unwrap_or_default());
        // Everything known about the space (older pages that were loaded stay), folded together.
        let acts = self.hist.acts.get(id).cloned().unwrap_or_default();
        self.hmac_oracle(id, &acts, &key_url);
        self.load_person_cover(id);
        let msgs = self.fold_in(id, &acts, &key_url);
        if let Some(m) = msgs.first() { self.hist.first.insert(id.to_string(), m.id.clone()); }
        let _ = self.out.send(Resp::Messages(id.to_string(), msgs));
        self.fetch_images();
        self.load_read_marks(id);
        self.load_space_cover(id);
        Ok(())
    }

    /// The custom cover of a group space (a banner is shown only when the space has its own cover). Resolves the extension's
    /// avatar url like the user cover (`imageUrls` Dark/Light/thumbnail), downloads it once and sends `Resp::SpaceCovers`.
    pub(super) fn load_space_cover(&mut self, space: &str) {
        let Some(url) = self.space_cover_urls.get(space).cloned() else { return };
        if !self.space_covers_done.insert(space.to_string()) { return }
        use std::io::Read;
        let a = match self.http("GET", &url, None) { Ok(a) => a, Err(e) => { dbg_conn(&format!("space cover: {}", e.split('→').nth(1).unwrap_or("").chars().take(30).collect::<String>())); return } };
        let urls = &a["imageUrls"];
        let pick = ["Dark.png", "Light.png", "thumbnail"].iter().map(|k| st(urls, k)).find(|u| u.starts_with("https://"))
            .or_else(|| urls.as_object().and_then(|m| m.values().find_map(|x| x.as_str().filter(|s| s.starts_with("https://")).map(String::from))));
        let Some(u) = pick else { dbg_conn(&format!("space cover: no image url, answer keys {:?}", a.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()))); return };
        let mut req = self.agent.get(&u);
        if images::token_host(&u) { req = req.set("Authorization", &format!("Bearer {}", self.token)); }
        let mut buf = vec![];
        let ok = req.call().ok().map(|r| r.into_reader().take(COVER_MAX_BYTES).read_to_end(&mut buf).is_ok() && !buf.is_empty()).unwrap_or(false);
        dbg_conn(&format!("space cover ({:?}): {} bytes, ok {ok}", st(&a, "imageName"), buf.len()));
        if ok { let _ = self.out.send(Resp::SpaceCovers(vec![(space.to_string(), buf)])); }
    }

    /// Older messages ("load more" at the top of a chat): the activities before the oldest one known, page size
    /// `HISTORY_PAGE` (`GET /activities?conversationId=…&maxDate=<oldest published>&limit=…`). The page is merged into the
    /// space's raw cache and the whole cache is folded again, so reaction summaries / edits / deletes that live in newer or
    /// older activities attach to the right message in both directions. Answers with
    /// `Resp::OlderMessages(space, full list, prepended, reached_start)`.
    pub(crate) fn older_messages(&mut self, id: &str) -> Result<(), String> {
        let key_url = self.spaces.get(id).map(|s| s.key_url.clone()).unwrap_or_default();
        let oldest_of = |s: &Self| s.hist.acts.get(id).and_then(|a| a.first()).map(|a| st(a, "published")).filter(|p| !p.is_empty());
        if oldest_of(self).is_none() { return self.messages(id) } // nothing loaded yet: load the newest page first
        // The start is reached at the `create` activity.
        let has_create = |s: &Self| s.hist.acts.get(id).is_some_and(|l| l.iter().any(|a| a["verb"] == "create"));
        let mut reached = self.hist.done.contains(id) || has_create(self);
        // One step = HISTORY_PAGE new visible rows (messages; reactions / edits / acks don't count): request again until enough
        // or the start.
        let (mut visible, mut requests) = (0usize, 0u32);
        while !reached && visible < HISTORY_PAGE as usize && requests < HISTORY_MAX_REQUESTS {
            let Some(oldest) = oldest_of(self) else { break };
            let v = self.http("GET", &self.url("conversation", &older_query(id, &oldest)), None)?;
            let items: Vec<Value> = v["items"].as_array().cloned().unwrap_or_default();
            visible += items.iter().filter(|a| (a["verb"] == "post" || a["verb"] == "share") && a["parent"]["type"] != "edit").count();
            let n = self.merge_acts(id, items);
            requests += 1;
            reached = n == 0 || has_create(self);
            dbg_conn(&format!("older messages: {n} new activities ({visible} visible so far), reached start {reached}"));
        }
        if reached { self.hist.done.insert(id.to_string()); }
        let acts = self.hist.acts.get(id).cloned().unwrap_or_default();
        let msgs = self.fold_in(id, &acts, &key_url);
        let prepended = self.hist.first.get(id).and_then(|f| msgs.iter().position(|m| m.id == *f)).unwrap_or(0);
        if let Some(m) = msgs.first() { self.hist.first.insert(id.to_string(), m.id.clone()); }
        let _ = self.out.send(Resp::OlderMessages(id.to_string(), msgs, prepended, reached));
        self.fetch_images();
        Ok(())
    }

    /// "Seen by" history: `GET /conversations/<id>?participantAckFilter=all` returns each participant's
    /// `roomProperties {lastSeenActivityUUID, lastSeenActivityDate, lastAckDate, lastAckTime}`.
    /// Live acknowledgements (mercury) that are newer are kept.
    pub(super) fn load_read_marks(&mut self, space: &str) {
        let url = self.url("conversation", &format!("/conversations/{space}?participantAckFilter=all&participantsLimit=100&activitiesLimit=0"));
        let Ok(v) = self.http("GET", &url, None) else { dbg_conn("read marks: request failed"); return };
        let mut n = 0;
        for p in v["participants"]["items"].as_array().into_iter().flatten() {
            let rp = &p["roomProperties"];
            let activity = st(rp, "lastSeenActivityUUID");
            let (id, email) = (st(p, "id"), st(p, "emailAddress").to_lowercase());
            let who = Person { id: id.clone(), email: email.clone(), ..Default::default() };
            if activity.is_empty() || self.is_me(&who) || st(p, "entryUUID") == self.user { continue }
            let time = Some(st(rp, "lastAckDate")).filter(|t| !t.is_empty()).unwrap_or_else(|| st(rp, "lastSeenActivityDate"));
            let key = if email.is_empty() { id.clone() } else { email.clone() };
            let marks = self.read_marks.entry(space.to_string()).or_default();
            if marks.get(&key).map_or(true, |m| m.time <= time) { marks.insert(key, ReadMark { person: id, email, activity, time }); n += 1; }
        }
        dbg_conn(&format!("read marks: {n} people for a space"));
        let list: Vec<ReadMark> = self.read_marks.get(space).map(|m| m.values().cloned().collect()).unwrap_or_default();
        let _ = self.out.send(Resp::ReadMarks(space.to_string(), list));
    }

    /// Someone acknowledged (read) a message: remember it per person and tell the UI the space's full read list.
    /// (History comes from `load_read_marks`; acknowledgements arrive live over mercury.)
    pub(super) fn read_mark(&mut self, space: &str, a: &Value) {
        let actor = &a["actor"];
        let (email, uuid) = (st(actor, "emailAddress").to_lowercase(), st(actor, "entryUUID"));
        let mut person = st(actor, "id");
        if let Some((id, _)) = self.uuids.iter().find(|(_, u)| **u == uuid || **u == person) { person = id.clone(); }
        let me = Person { id: person.clone(), email: email.clone(), ..Default::default() };
        if self.is_me(&me) || person == self.user || uuid == self.user { return }
        let key = if email.is_empty() { person.clone() } else { email.clone() };
        self.read_marks.entry(space.to_string()).or_default().insert(key, ReadMark { person, email, activity: st(&a["object"], "id"), time: st(a, "published") });
        let list: Vec<ReadMark> = self.read_marks[space].values().cloned().collect();
        let _ = self.out.send(Resp::ReadMarks(space.to_string(), list));
    }

    /// A live event: a new top-level message is added directly; anything else (edit, delete, reaction, reply) is merged into
    /// the space's raw cache and the cache is folded again, without a request. An event I already have (the echo of my own
    /// write, applied from the POST answer) is only de-duplicated. The read marks (participants) are fetched again only when
    /// the membership changed. A space that was never loaded here loads nothing: only its list entry changes, and a new
    /// message or reply arrives as `Resp::Incoming`.
    pub(crate) fn incoming(&mut self, a: &Value) -> Result<(), String> {
        let id = st(&a["target"], "id");
        if a["verb"] == "acknowledge" {
            // Mine (this or another of my devices) moves my read state; anyone else's is a read mark (cloud/live_list/).
            if self.actor_is_me(&a["actor"]) { self.self_read(&id, a); } else { self.read_mark(&id, a); }
            return Ok(());
        }
        // A conversation the list doesn't have (a new 1:1, added to a space): fetched once and added (cloud/live_list/).
        if !self.spaces.contains_key(&id) { self.unknown_space(&id, a); }
        // Rename, hide, favourite, mute, lock, moderators, left/removed, team changes made elsewhere (cloud/live_list/).
        if self.spaces.contains_key(&id) { self.list_activity(&id, a); }
        self.note_readable(&id, a); // keeps the space's last readable time
        self.echo_arrived(a);
        let plain = (a["verb"] == "post" || a["verb"] == "share") && a["parent"].is_null();
        let reply = (a["verb"] == "post" || a["verb"] == "share") && a["parent"]["type"] == "reply";
        let cached = self.hist.acts.contains_key(&id);
        // A space not opened here needs no history for an event: the list was patched above, a thread reply still arrives as
        // a new message (moves the space up, unread, toast); edits / deletes / reactions show when it is opened (that load
        // reads them).
        if a["verb"] == "delete" {
            let target = st(&a["object"], "id");
            dbg_conn(&format!("live delete {} of {target} (object verb {:?}) in {id}: space loaded {cached}, target cached {}",
                st(a, "id"), st(&a["object"], "verb"), self.cached(&id, &target)));
        }
        if !plain && !cached && !reply { return Ok(()); }
        if !plain && cached {
            if self.merge_acts(&id, vec![a.clone()]) > 0 {
                self.refold(&id);
                if super::echo::membership_change(a) { self.load_read_marks(&id); }
            }
            return Ok(());
        }
        let key_url = self.spaces.get(&id).map(|s| s.key_url.clone()).unwrap_or_default();
        if cached && self.merge_acts(&id, vec![a.clone()]) == 0 { return Ok(()); } // known already (my own write's echo)
        // Its delete came first (another page or event): it is a tombstone now, shown by folding the list again.
        if cached && self.hist.acts.get(&id).is_some_and(|l| l.iter().any(|x| x["verb"] == "tombstone" && st(x, "id") == st(a, "id"))) { self.refold(&id); return Ok(()); }
        if let Some(m) = self.activity(a, &key_url) { let _ = self.out.send(Resp::Incoming(id, m)); self.fetch_images(); }
        Ok(())
    }
}
