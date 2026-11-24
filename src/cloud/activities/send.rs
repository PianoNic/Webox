//! Sending to a space: posts, replies, edits, GIFs, deletes, reactions, read receipts and catch-up.
use super::*;

impl Session {
    /// An activity's `actor` is me if its id / entryUUID is my device user uuid, or its id / email is my account email
    /// (participant and actor ids come in both forms).
    pub(crate) fn actor_is_me(&self, actor: &Value) -> bool {
        let (id, uuid, mail) = (st(actor, "id"), st(actor, "entryUUID"), st(actor, "emailAddress"));
        id == self.user || uuid == self.user
            || (!self.my_email.is_empty() && [&id, &mail].iter().any(|s| s.eq_ignore_ascii_case(&self.my_email)))
    }

    pub(crate) fn key_url(&self, id: &str) -> Result<String, String> {
        self.spaces.get(id).map(|s| s.key_url.clone()).filter(|k| !k.is_empty()).ok_or_else(|| "this space has no key".to_string())
    }

    /// Posts one activity to a space.
    pub(crate) fn submit(&mut self, id: &str, mut body: Value) -> Result<(), String> {
        body["target"] = json!({"id": id, "objectType": "conversation"});
        let temp = jose::uuid();
        body["clientTempId"] = json!(temp);
        // Every activity is objectType "activity" with the actor as a person object.
        body["objectType"] = json!("activity");
        if body["actor"].is_null() { body["actor"] = json!({"objectType": "person", "id": self.user}); }
        let what = friendly_failure(&body);
        // The answer is the activity as the service stored it: `after_write` applies it instead of a reload.
        self.http("POST", &self.url("conversation", "/activities"), Some(body)).map(|v| { self.hist.posted = Some((v, temp)); }).map_err(|e| {
            // Keep the server's reason (status + error code only) in the trace, show the user a short message.
            let code = e.split(" → ").nth(1).unwrap_or("").split_whitespace().next().unwrap_or("").to_string();
            let ecode = e.split("\"errorCode\":").nth(1).map(|r| r.chars().take_while(|c| c.is_ascii_digit()).collect::<String>()).unwrap_or_default();
            dbg_conn(&format!("activity rejected: http {code} errorCode {ecode}"));
            if code.starts_with('4') { what.to_string() } else { e }
        })
    }

    pub(super) fn post(&mut self, id: &str, text: &str, parent: Option<(&str, &str)>) -> Result<(), String> {
        self.post_rich(id, text, false, &[], parent)
    }

    /// A person id as the composer knows it (`Person.id`, e-mail style) → the Webex uuid the service wants in mentions.
    pub(super) fn person_uuid(&self, id: &str) -> Option<String> {
        if id.len() == 36 && id.matches('-').count() == 4 { return Some(id.to_string()); }
        self.uuids.get(id).cloned().or_else(|| (id.eq_ignore_ascii_case(&self.my_email) && !self.user.is_empty()).then(|| self.user.clone()))
    }

    /// Posts a message: `displayName` (the plain rendition) and, when something is formatted or someone is mentioned,
    /// `content` (HTML) and `mentions`, both encrypted with the space's key (see `richtext.rs`).
    /// `markdown`: the composer's markdown mode (or its rich toolbar's markers) → interpreted; otherwise the text is literal.
    pub(crate) fn post_rich(&mut self, id: &str, text: &str, markdown: bool, mentions: &[crate::richtext::MentionRef], parent: Option<(&str, &str)>) -> Result<(), String> {
        let resolved: Vec<crate::richtext::MentionRef> = mentions.iter().filter_map(|m| if m.all { Some(m.clone()) } else { self.person_uuid(&m.id).map(|u| crate::richtext::MentionRef { id: u, ..m.clone() }) }).collect();
        let r = crate::richtext::render(text, markdown, &resolved);
        let key_url = self.key_url(id)?;
        let key = self.key(&key_url)?;
        let object = comment_object(&r, text, &mut |t| jose::encrypt_dir(&key, &key_url, t.as_bytes()))?;
        let mut body = json!({"verb": "post", "encryptionKeyUrl": key_url, "object": object});
        if let Some((pid, kind)) = parent { body["parent"] = json!({"id": pid, "type": kind}); }
        self.submit(id, body)
    }

    pub(crate) fn send(&mut self, id: &str, text: &str) -> Result<(), String> { self.post(id, text, None) }

    /// Shares a Giphy GIF (`Req::SendGif`): a `share` activity whose file item points at Giphy's public CDN; nothing is uploaded and
    /// the "encryption" of the `scr` only hides the location from the service, as in real shares (see `gif_share_body`).
    pub(crate) fn send_gif(&mut self, id: &str, url: &str, title: &str, w: u32, h: u32) -> Result<(), String> {
        let url = url.replacen("http://", "https://", 1);
        if !url.starts_with("https://") || !images::giphy_host(&url) { return Err("GIF-Adresse nicht erlaubt".into()); }
        let key_url = self.key_url(id)?;
        let key = self.key(&key_url)?;
        // fileSize is a real value in received shares: ask Giphy's CDN (plain HEAD, no token), 0 if it won't say.
        let size = self.agent.head(&url).call().ok().and_then(|r| r.header("content-length").and_then(|v| v.parse::<u64>().ok())).unwrap_or(0);
        let mut enc_text = |t: &str| jose::encrypt_dir(&key, &key_url, t.as_bytes());
        let mut enc_scr = |v: &Value| jose::encrypt_dir_nokid(&key, v.to_string().as_bytes());
        let body = gif_share_body(&key_url, title, &url, w, h, size, &jose::uuid(), &iso_now(), &mut enc_text, &mut enc_scr)?;
        let mut full = body.clone();
        full["target"] = json!({"id": id, "objectType": "conversation"});
        let temp = jose::uuid();
        full["clientTempId"] = json!(temp);
        full["objectType"] = json!("activity");
        full["actor"] = json!({"objectType": "person", "id": self.user});
        self.http("POST", &self.url("conversation", GIF_SHARE_PATH), Some(full)).map(|v| { self.hist.posted = Some((v, temp)); })
            .map_err(|e| { dbg_conn("gif share rejected on /content"); if e.contains(" → 4") { "GIF konnte nicht gesendet werden".to_string() } else { e } })
    }

    pub(crate) fn delete(&mut self, id: &str, activity: &str) -> Result<(), String> {
        self.submit(id, json!({"verb": "delete", "object": {"id": activity, "objectType": "activity"}}))
    }

    /// Removes my reaction. `token` is either a real reaction activity id or `"<message id>|<name>"` (from a summary): the
    /// latter is resolved now with `GET <conversation>/parents/<message>?activityType=reaction`, only for this message.
    pub(crate) fn unreact(&mut self, id: &str, token: &str) -> Result<(), String> {
        let Some((msg, name)) = token.split_once('|') else { return self.delete(id, token) };
        let r = self.http("GET", &self.url("conversation", &format!("/conversations/{id}/parents/{msg}?activityType=reaction")), None)?;
        let key_url = self.key_url(id).unwrap_or_default();
        let items: Vec<Value> = r["items"].as_array().cloned().or_else(|| r.as_array().cloned()).unwrap_or_default();
        for a in items {
            if !self.actor_is_me(&a["actor"]) { continue }
            let ku = Some(st(&a, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.clone());
            if self.decrypt(&st(&a["object"], "displayName"), &ku) == name { return self.delete(id, &st(&a, "id")); }
        }
        Err("Reaktion nicht gefunden".into())
    }

    /// Read receipt: only when the space has something unseen (`lastSeen < lastReadable`) and for the newest readable
    /// activity (a message, not an info row). `activity` is what the UI shows last; the receipt goes to the newest post/share
    /// known for the space instead when that is not one. The space's `last_seen` moves on locally, so a reload of the same
    /// list (reactions, edits) doesn't send it again. Verb "acknowledge", object {id, objectType "activity"}.
    pub(crate) fn ack(&mut self, id: &str, activity: &str) -> Result<(), String> {
        let acts = self.hist.acts.get(id).cloned().unwrap_or_default();
        let readable = |a: &&Value| (a["verb"] == "post" || a["verb"] == "share") && a["parent"]["type"] != "edit";
        let target = acts.iter().filter(readable).find(|a| st(a, "id") == activity).or_else(|| acts.iter().filter(readable).last());
        let Some(target) = target.cloned() else { return Ok(()) };
        let (tid, published) = (st(&target, "id"), st(&target, "published"));
        let last_seen = self.spaces.get(id).map(|s| s.last_seen.clone()).unwrap_or_default();
        if !ack_due(&last_seen, &published) { return Ok(()) }
        // My own message: the service already counts it as seen, so only lastSeen moves on locally.
        if self.actor_is_me(&target["actor"]) { if let Some(s) = self.spaces.get_mut(id) { s.last_seen = published; } return Ok(()) }
        self.submit(id, json!({"verb": "acknowledge", "object": {"id": tid, "objectType": "activity"}}))?;
        if let Some(s) = self.spaces.get_mut(id) { s.last_seen = published; s.unread = false; }
        Ok(())
    }

    /// After the socket was down: catch up on the conversations and re-subscribe presence. Reloads the conversation list
    /// (which re-reads presence and re-subscribes) and every opened space whose last activity changed while the socket was
    /// down, so messages sent in the gap appear without reopening the chat.
    /// `GET /conversations/catchup?sinceDate=<ms>` answers only the conversations that changed since then
    /// (`items[{id, tags, lastSeenActivityDate, …}]`).
    pub(crate) fn catchup(&mut self, since_ms: i64) -> Result<(), String> {
        let changed: Option<Vec<String>> = match self.http("GET", &self.url("conversation", &catchup_query(since_ms)), None) {
            Ok(v) => Some(v["items"].as_array().into_iter().flatten().map(|c| st(c, "id")).filter(|i| !i.is_empty()).collect()),
            Err(e) => { dbg_conn(&format!("catchup request failed ({}), reloading everything", http_status(&e).map_or("transport".to_string(), |c| c.to_string()))); None }
        };
        dbg_conn(&format!("mercury back: catching up, {} conversations changed", changed.as_ref().map_or("?".to_string(), |c| c.len().to_string())));
        // Flags set / cleared on another device while the socket was down are re-read on every reconnect. The flag request has
        // no `sinceDate`, so it reads the full list.
        if let Err(e) = self.flags_load() { dbg_conn(&format!("catchup: flags not re-read ({})", http_status(&e).map_or("no service".into(), |c| c.to_string()))); }
        if changed.as_ref().is_some_and(|c| c.is_empty()) {
            // No list reload (which would re-read presence): read the statuses that changed while the socket was down.
            let mut want = self.presence_people();
            want.extend(self.extra_people());
            let statuses = self.presence(&want);
            crate::cloud::people::send_presence(&self.out, statuses);
            return Ok(())
        }
        self.spaces()?; // the list (titles, unread, new spaces) and a fresh presence subscription
        let opened: Vec<String> = self.hist.acts.keys().cloned().collect();
        for id in opened {
            if changed.as_ref().map_or(true, |c| c.contains(&id)) { self.messages(&id)?; }
        }
        Ok(())
    }

    /// A reaction: `object {objectType:"reaction2", displayName:<JWE of the plain name>, hmac}` plus the activity's
    /// `encryptionKeyUrl` (the parent message's key). The hmac is over the *plain* name.
    pub(crate) fn react(&mut self, id: &str, msg: &str, name: &str) -> Result<(), String> {
        let parent_key = self.http("GET", &self.url("conversation", &format!("/activities/{msg}")), None).ok()
            .map(|a| st(&a, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or(self.key_url(id)?);
        let key = self.key(&parent_key)?;
        let hmac = jose::reaction_hmac(&key, &parent_key, msg, name);
        let jwe = jose::encrypt_dir(&key, &parent_key, name.as_bytes())?;
        self.submit(id, json!({"verb": "add", "encryptionKeyUrl": parent_key, "parent": {"id": msg, "type": "reaction"},
            "object": {"objectType": "reaction2", "displayName": jwe, "hmac": hmac}}))
    }
}
