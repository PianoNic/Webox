//! Turning activities into messages and notices: folding, join and leave rows, the HMAC check, merging pages.
use super::*;

impl Session {
    pub(super) fn activity(&mut self, a: &Value, key_url: &str) -> Option<Message> {
        if a["verb"] != "post" && a["verb"] != "share" { return None; }
        let key_url = Some(st(a, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.to_string());
        let mut text = self.decrypt(&st(&a["object"], "displayName"), &key_url);
        // Rich text: `object.content` is encrypted with the same key; only keep it when it really is markup.
        let html = { let c = st(&a["object"], "content"); if c.is_empty() { String::new() } else { self.decrypt(&c, &key_url) } };
        let mut images = vec![];
        let mut files_out: Vec<Attachment> = vec![];
        if a["verb"] == "share" {
            let mut names = vec![];
            for (idx, f) in a["object"]["files"]["items"].as_array().into_iter().flatten().enumerate() {
                // Each file carries its own key url; its `image` block is the thumbnail (own url + scr, 512 px wide).
                let fk = Some(st(f, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.clone());
                let name = self.decrypt(&st(f, "displayName"), &fk);
                let (mime, img) = (st(f, "mimeType"), &f["image"]);
                let (url, scr, fallback) = image_source(f, &name);
                // The file itself (its own scr) can be downloaded / saved later (Req::DownloadFile).
                let desc = super::super::files::FileDesc { name: name.clone(), mime: mime.clone(), size: f["fileSize"].as_u64().unwrap_or(0), scr: st(f, "scr"), key_url: fk.clone() };
                if (mime.starts_with("image/") || img.is_object()) && !url.is_empty() && !scr.is_empty() {
                    // `url` is not unique (every Giphy share has url "https://giphy.com"): the key is activity + index + hash of url/scr.
                    let key = attachment_key(&st(a, "id"), idx, &url, &scr);
                    if !desc.scr.is_empty() { super::super::files::remember(&key, desc); }
                    let giphy = if images::giphy_host(&url) { self.giphy_of(&scr, &fk, &name) } else { None };
                    images.push(Attachment { name, mime, size: f["fileSize"].as_u64().unwrap_or(0), key: key.clone(),
                        width: img["width"].as_u64().unwrap_or(0) as u32, height: img["height"].as_u64().unwrap_or(0) as u32, giphy });
                    // The original file (own url/scr), for the full-resolution viewer.
                    if fallback.is_none() && img.is_object() && !st(f, "scr").is_empty() && st(f, "scr") != scr {
                        self.full_desc.insert(key.clone(), images::FullDesc { scr: st(f, "scr"), key_url: fk.clone(), mime: st(f, "mimeType") });
                    }
                    self.img_jobs.push(images::ImgJob { key, scr, key_url: fk, fallback });
                } else {
                    // A downloadable file card: key from the file's own url / scr.
                    let key = attachment_key(&st(a, "id"), idx, &st(f, "url"), &st(f, "scr"));
                    files_out.push(Attachment { name: name.clone(), mime: desc.mime.clone(), size: desc.size, key: key.clone(), ..Default::default() });
                    if !desc.scr.is_empty() { super::super::files::remember(&key, desc); }
                    names.push(name);
                }
            }
            // One "📎 name" line per file (the message list draws one card per line).
            if !names.is_empty() { text = format!("{text}{}{}", if text.is_empty() { "" } else { "\n" }, names.iter().map(|n| format!("📎 {n}")).collect::<Vec<_>>().join("\n")); }
        }
        let parent = if a["parent"]["type"] == "reply" { st(&a["parent"], "id") } else { String::new() };
        Some(Message { id: st(a, "id"), author: st(&a["actor"], "displayName"), email: st(&a["actor"], "emailAddress"),
            text, time: hhmm(&st(a, "published")), date: st(a, "published").get(..10).unwrap_or("").to_string(), mine: self.actor_is_me(&a["actor"]), parent, images, html, files: files_out, published_ms: super::super::people::iso_ms(&st(a, "published")), ..Default::default() })
    }

    /// The info row for a non-message activity, or `None`, worded with the language pack's strings (context "QObject",
    /// e.g. "{{0}} left the space.", "This starts the space, created by {{0}}.", "{{0}} was unavailable.",
    /// "You had a call ({{0}})"). `space` is the conversation the activity belongs to: activities carry no `target` in a
    /// listing, so 1:1 vs. group comes from the space (`tags` ONE_ON_ONE in the conversation list) or the create's own
    /// `object.tags`.
    pub(crate) fn notice_text(&mut self, a: &Value, key_url: &str, space: &str) -> Option<String> {
        let sp = self.spaces.get(space).cloned();
        let tagged = |v: &Value, t: &str| v["tags"].as_array().is_some_and(|x| x.iter().any(|x| x == t));
        let direct = sp.as_ref().is_some_and(|s| s.direct) || tagged(&a["object"], "ONE_ON_ONE") || tagged(&a["target"], "ONE_ON_ONE");
        // A team's own (general) space: tag TEAM and id == team.id; it gets the "team" sentences ("{{0}} left the team.",
        // "This starts the team, created by {{0}}." …), every other space the "space" ones.
        let team = sp.as_ref().is_some_and(|s| !s.team.is_empty() && s.team == s.id) || tagged(&a["object"], "TEAM");
        // A person's shown name: displayName, else the space's participant with that id / uuid / e-mail, else e-mail. `None` when
        // nothing is known (the server's made-up `create` of an old 1:1 has an actor with only id/entryUUID): the sentence without
        // a name ("This starts the space.") is used then instead of inventing one.
        let name_of = |s: &Self, p: &Value| -> Option<String> {
            if let Some(n) = Some(st(p, "displayName")).filter(|n| !n.trim().is_empty()) { return Some(n); }
            let keys = [st(p, "id"), st(p, "entryUUID"), st(p, "emailAddress"), st(p, "email")];
            let hit = sp.as_ref().and_then(|sp| sp.participants.iter().find(|q| keys.iter().any(|k| !k.is_empty()
                && (q.id.eq_ignore_ascii_case(k) || q.email.eq_ignore_ascii_case(k) || s.uuids.get(&q.id).is_some_and(|u| u == k)))));
            hit.map(|q| q.name.clone()).filter(|n| !n.trim().is_empty())
                .or_else(|| [st(p, "emailAddress"), st(p, "email")].into_iter().find(|e| !e.is_empty()))
        };
        let is_me = |s: &Self, p: &Value| s.actor_is_me(p) || (!s.my_email.is_empty() && st(p, "email").eq_ignore_ascii_case(&s.my_email));
        let same = |x: &Value, y: &Value| { let (i, u) = (st(x, "id"), st(x, "entryUUID")); (!i.is_empty() && i == st(y, "id")) || (!u.is_empty() && u == st(y, "entryUUID")) };
        let o = &a["object"];
        let actor = &a["actor"];
        match (a["verb"].as_str().unwrap_or(""), o["objectType"].as_str().unwrap_or("")) {
            ("update", "locusSessionSummary") => {
                let parts: Vec<&Value> = o["participants"]["items"].as_array().into_iter().flatten().collect();
                let initiator = parts.iter().find(|p| p["isInitiator"] == true).copied();
                let by_me = initiator.is_some_and(|p| is_me(self, p));
                let secs = o["duration"].as_u64().unwrap_or(0);
                let person = |s: &Self, p: &Value| name_of(s, &p["person"]).or_else(|| name_of(s, p));
                Some(match (direct, secs, by_me) {
                    (true, 0, true) => {
                        let other = parts.iter().find(|p| !is_me(self, p)).and_then(|p| person(self, p));
                        crate::trq!("{{0}} was unavailable.", other.unwrap_or_default())
                    }
                    (true, 0, false) => crate::trq!("You missed a call.").into_owned(),
                    (true, _, _) => crate::trq!("You had a call ({{0}})", call_duration(secs)),
                    (false, 0, true) => crate::trq!("You started a meeting but no one was available.").into_owned(),
                    (false, 0, false) => crate::trq!("{{0}} started a meeting but no one was available.", initiator.and_then(|p| person(self, p)).unwrap_or_default()),
                    (false, _, _) => crate::trq!("You had a meeting ({{0}})", call_duration(secs)),
                })
            }
            ("leave", "person") => {
                let self_left = actor.is_null() || same(actor, o);
                let who = name_of(self, o).unwrap_or_default();
                let by = name_of(self, actor).unwrap_or_default();
                Some(match (team, self_left, is_me(self, o), is_me(self, actor)) {
                    (false, true, true, _) => crate::trq!("You left the space.").into_owned(),
                    (false, true, false, _) => crate::trq!("{{0}} left the space.", who),
                    (false, false, _, true) => crate::trq!("You removed {{0}} from this space.", who),
                    (false, false, _, false) => crate::trq!("{{0}} removed {{1}} from this space.", by, who),
                    (true, true, true, _) => crate::trq!("You left the team.").into_owned(),
                    (true, true, false, _) => crate::trq!("{{0}} left the team.", who),
                    (true, false, _, true) => crate::trq!("You removed {{0}} from this team.", who),
                    (true, false, _, false) => crate::trq!("{{0}} removed {{1}} from this team.", by, who),
                })
            }
            ("add", "person") if !direct => {
                let who = name_of(self, o).unwrap_or_default();
                let by = name_of(self, actor).unwrap_or_default();
                Some(match (team, same(actor, o), is_me(self, o), is_me(self, actor)) {
                    (false, true, true, _) => crate::trq!("You joined this space.").into_owned(),
                    (false, true, false, _) => crate::trq!("{{0}} joined this space.", who),
                    (false, false, _, true) => crate::trq!("You added {{0}} to this space.", who),
                    (false, false, true, false) => crate::trq!("{{0}} added you to this space. Welcome.", by),
                    (false, false, false, false) => crate::trq!("{{0}} added {{1}} to this space.", by, who),
                    (true, true, true, _) => crate::trq!("You joined this team.").into_owned(),
                    (true, true, false, _) => crate::trq!("{{0}} joined this team.", who),
                    (true, false, _, true) => crate::trq!("You added {{0}} to this team.", who),
                    (true, false, true, false) => crate::trq!("{{0}} added you to this team. Welcome.", by),
                    (true, false, false, false) => crate::trq!("{{0}} added {{1}} to this team.", by, who),
                })
            }
            // A deleted message ("Sie haben Ihre Nachricht gelöscht. 15.12.2025, 15:08"). The service replaces the activity by a
            // `tombstone` with the same id: `actor` deleted it, `object.actor` wrote it. A tombstone of an edit is not shown.
            // Strings: QObject "You deleted your message." … "{{0}} deleted {{1}}'s reply".
            ("tombstone", _) if a["parent"]["type"] != "edit" => {
                let reply = a["parent"]["type"] == "reply";
                let author = &o["actor"];
                let own = author.is_null() || same(actor, author);
                let (by, of) = (name_of(self, actor).unwrap_or_default(), name_of(self, author).unwrap_or_default());
                Some(match (reply, is_me(self, actor), own) {
                    (false, true, true) => crate::trq!("You deleted your message.").into_owned(),
                    (false, true, false) => crate::trq!("You deleted {{0}}'s message.", of),
                    (false, false, true) => crate::trq!("{{0}} deleted their own message.", by),
                    (false, false, false) => crate::trq!("{{0}} deleted {{1}}'s message.", by, of),
                    (true, true, true) => crate::trq!("You deleted your reply").into_owned(),
                    (true, true, false) => crate::trq!("You deleted {{0}}'s reply", of),
                    (true, false, true) => crate::trq!("{{0}} deleted their own reply", by),
                    (true, false, false) => crate::trq!("{{0}} deleted {{1}}'s reply", by, of),
                })
            }
            ("update", "conversation") if !st(o, "displayName").is_empty() => {
                let ku = Some(st(a, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.to_string());
                let title = self.decrypt(&st(o, "displayName"), &ku);
                Some(if is_me(self, actor) { crate::trq!("You renamed this space to \"{{0}}\".", title) }
                     else { crate::trq!("{{0}} renamed this space to \"{{1}}\".", name_of(self, actor).unwrap_or_default(), title) })
            }
            // A 1:1 has no start sentence (every "This starts the …" string is for a space or a team).
            ("create", "conversation") if !direct => {
                let ku = Some(st(a, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.to_string());
                let own = st(o, "displayName");
                // A general space is titled "General" in the list: its start sentence names the team.
                let known = |s: &Space| if team { super::super::lists::team_name(&s.team).unwrap_or_else(|| s.title.clone()) } else { s.title.clone() };
                let title = Some(if own.is_empty() { sp.as_ref().map(known).unwrap_or_default() } else { self.decrypt(&own, &ku) }).filter(|t| !t.trim().is_empty());
                Some(match (team, title, name_of(self, actor)) {
                    (false, Some(t), Some(by)) => crate::trq!("This starts the \"{{0}}\" space, created by {{1}}.", t, by),
                    (false, None, Some(by)) => crate::trq!("This starts the space, created by {{0}}.", by),
                    (false, Some(t), None) => crate::trq!("This starts the \"{{0}}\" space.", t),
                    (false, None, None) => crate::trq!("This starts the space.").into_owned(),
                    (true, Some(t), Some(by)) => crate::trq!("This starts the \"{{0}}\" team, created by {{1}}.", t, by),
                    (true, None, Some(by)) => crate::trq!("This starts the team, created by {{0}}.", by),
                    (true, Some(t), None) => crate::trq!("This starts the \"{{0}}\" team.", t),
                    (true, None, None) => crate::trq!("This starts the team.").into_owned(),
                })
            }
            _ => None,
        }
    }

    /// `fold_in` without a space (no 1:1 / participant context for the info rows).
    #[allow(dead_code)]
    pub(crate) fn fold(&mut self, acts: &[Value], key_url: &str) -> Vec<Message> { self.fold_in("", acts, key_url) }

    /// Builds the visible message list from raw activities: edits replace text, deletes remove,
    /// reactions are counted on the message they belong to (minus deleted reactions). `space`: the conversation id.
    pub(crate) fn fold_in(&mut self, space: &str, acts: &[Value], key_url: &str) -> Vec<Message> {
        if std::env::var_os("WEBOX_NOTICE_PROBE").is_some() {
            fn shape(v: &Value, d: u8) -> String {
                match v {
                    Value::Object(m) if d > 0 => format!("{{{}}}", m.iter().map(|(k, x)| format!("{}:{}", if k.len() > 24 { "<long>" } else { k }, shape(x, d - 1))).collect::<Vec<_>>().join(",")),
                    Value::Object(_) => "obj".into(),
                    Value::Array(a) => format!("[{}x{}]", a.len(), a.first().map(|x| shape(x, d.saturating_sub(1))).unwrap_or_default()),
                    Value::String(s) => if s.len() < 20 && !s.contains('@') && !s.contains('-') { format!("\"{s}\"") } else { format!("s{}", s.len()) },
                    Value::Number(n) => n.to_string(), Value::Bool(b) => b.to_string(), Value::Null => "null".into(),
                }
            }
            let mut seen: HashSet<String> = HashSet::new();
            for a in acts {
                let k = format!("{}/{}/{}", st(a, "verb"), st(&a["object"], "objectType"), st(&a["parent"], "type"));
                if matches!(k.as_str(), "post/comment/" | "share/content/" | "add/reaction2Summary/reactionSummary") || !seen.insert(k.clone()) { continue }
                dbg_conn(&format!("NOTICE-SHAPE {k} object={} target={}", shape(&a["object"], 4), shape(&a["target"], 1)));
            }
        }
        let mut list: Vec<Message> = vec![];
        let mut edits = vec![];
        let mut deleted = HashSet::new();
        let mut reacts: Vec<(String, String, String, bool, String, String)> = vec![]; // (message, name, reaction id, mine, published, actor)
        // The listing holds reaction *summaries* ("add" / object.reaction2Summary, parent.type "reactionSummary"): cumulative
        // snapshots, several per message, so only the newest per message counts.
        let mut summaries: HashMap<String, &Value> = HashMap::new();
        let create_ms = acts.iter().find(|a| a["verb"] == "create").map(|a| super::super::people::iso_ms(&st(a, "published")));
        let mut joins: HashMap<String, JoinLeave> = HashMap::new(); // info rows that may collapse (add / leave of a person)
        for a in acts {
            let pid = st(&a["parent"], "id");
            match a["verb"].as_str().unwrap_or("") {
                // The `delete` that made a tombstone points at the tombstone (same id): the "… deleted …" row stays.
                "delete" if a["object"]["verb"] != "tombstone" => { deleted.insert(st(&a["object"], "id")); }
                "delete" => {}
                "add" if a["object"]["objectType"] == "reaction2" => {
                    let ku = Some(st(a, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.to_string());
                    let name = self.decrypt(&st(&a["object"], "displayName"), &ku);
                    let actor = [st(&a["actor"], "entryUUID"), st(&a["actor"], "id")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
                    reacts.push((pid, name, st(a, "id"), self.actor_is_me(&a["actor"]), st(a, "published"), actor));
                }
                "add" if a["object"]["objectType"] == "reaction2Summary" => {
                    let newer = summaries.get(&pid).map_or(true, |o| st(o, "published") <= st(a, "published"));
                    if newer { summaries.insert(pid, a); }
                }
                "post" | "share" => if let Some(m) = self.activity(a, key_url) {
                    if a["parent"]["type"] == "edit" { edits.push((pid, m.text)) } else { list.push(m) }
                },
                // The members added within seconds of the space's creation are not announced.
                "add" if a["object"]["objectType"] == "person" && create_ms.map_or(false, |c| (super::super::people::iso_ms(&st(a, "published")) - c).abs() < 5000) => {}
                _ => if let Some(text) = self.notice_text(a, key_url, space) {
                    let p = st(a, "published");
                    list.push(Message { id: st(a, "id"), notice: text, time: hhmm(&p), date: p.get(..10).unwrap_or("").to_string(), ..Default::default() });
                    if let Some(j) = self.join_leave(a, space) { joins.insert(st(a, "id"), j); }
                },
            }
        }
        for (pid, text) in edits { if let Some(m) = list.iter_mut().find(|m| m.id == pid) { m.text = text; m.edited = true; } }
        // A deleted message's row is its tombstone ("… deleted …", `merge_acts`): only other rows go.
        list.retain(|m| !m.notice.is_empty() || !deleted.contains(&m.id));
        // A single reaction (a live event merged into the cache) that is not newer than the message's newest summary is already
        // counted in that summary (a later page load brings the summary that includes it): only newer ones are added on top.
        let summary_at: HashMap<String, String> = summaries.iter().map(|(m, a)| (m.clone(), st(a, "published"))).collect();
        reacts.retain(|r| summary_at.get(&r.0).map_or(true, |s| r.4 > *s));
        for (msg, a) in summaries {
            let Some(m) = list.iter_mut().find(|m| m.id == msg) else { continue };
            let ku = Some(st(a, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.to_string());
            let mut out: Vec<Reaction> = vec![];
            for r in a["object"]["reactions"].as_array().into_iter().flatten() {
                let users = r["users"].as_array().map_or(0, |u| u.len());
                let mut name = self.decrypt(&st(r, "displayName"), &ku);
                // A skin-tone modifier that applies to every reactor is part of the emoji.
                for md in r["modifiers"].as_array().into_iter().flatten() {
                    if md["modifiedUsersIndex"].as_array().map_or(0, |i| i.len()) == users && users > 0 {
                        // Only the Fitzpatrick tone characters belong to the emoji; named modifiers ("yellow" = the default tone) don't.
                        let tone = self.decrypt(&st(md, "displayName"), &ku);
                        if tone.chars().all(|c| ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)) { name.push_str(&tone); }
                    }
                }
                let count = r["count"].as_u64().map(|c| c as u32).unwrap_or(users as u32);
                let mine = r["users"].as_array().into_iter().flatten().any(|u| st(u, "id") == self.user);
                let users = reactors(r);
                // The summary doesn't carry my own reaction activity id; `unreact` resolves this token lazily.
                out.push(Reaction { mine: mine.then(|| format!("{msg}|{name}")), name, count, users });
            }
            m.reactions = out;
        }
        for (msg, name, rid, mine, published, actor) in reacts.into_iter().filter(|r| !deleted.contains(&r.2)) {
            let Some(m) = list.iter_mut().find(|m| m.id == msg) else { continue };
            let who = Reactor { id: actor, published, ..Default::default() };
            match m.reactions.iter_mut().find(|r| r.name == name) {
                Some(r) => { r.count += 1; if mine { r.mine = Some(rid) } if !who.id.is_empty() && !r.users.iter().any(|u| u.id == who.id) { r.users.push(who) } }
                None => m.reactions.push(Reaction { name, count: 1, mine: mine.then_some(rid), users: if who.id.is_empty() { vec![] } else { vec![who] } }),
            }
        }
        self.name_reactors(space, acts, &mut list);
        collapse_join_leave(list, &joins)
    }

    /// Gives every reactor (a uuid) the name and e-mail the session knows: read from their reaction activities
    /// (`reactor_names`), an actor of a loaded activity (`uuidEntryFormat` actors carry displayName / emailAddress), or the
    /// space's participant with that uuid. Me: my e-mail too (the UI calls me "Sie").
    fn name_reactors(&self, space: &str, acts: &[Value], list: &mut [Message]) {
        if !list.iter().any(|m| m.reactions.iter().any(|r| !r.users.is_empty())) { return }
        let mut known: HashMap<String, (String, String)> = HashMap::new();
        for p in self.spaces.get(space).map(|s| s.participants.as_slice()).unwrap_or_default() {
            let Some(u) = self.uuids.get(&p.id).cloned().or_else(|| (p.id.len() == 36).then(|| p.id.clone())) else { continue };
            if !p.name.trim().is_empty() { known.insert(u, (p.name.clone(), p.email.clone())); }
        }
        remember_actors(&mut known, acts);
        known.extend(self.hist.names.iter().map(|(k, v)| (k.clone(), v.clone())));
        for u in list.iter_mut().flat_map(|m| m.reactions.iter_mut()).flat_map(|r| r.users.iter_mut()) {
            if let Some((n, e)) = known.get(&u.id) { u.name = n.clone(); u.email = e.clone(); }
            if u.id == self.user && u.email.is_empty() { u.email = self.my_email.clone(); }
        }
    }

    /// The names of the people who reacted to `msg` (`Req::ReactorNames`, read-only): `GET <conversation>/parents/<msg>
    /// ?activityType=reaction` lists each reaction activity with its actor (uuid, displayName, emailAddress). Remembered per
    /// uuid, then the space is folded again.
    pub(crate) fn reactor_names(&mut self, space: &str, msg: &str) -> Result<(), String> {
        let q = format!("/conversations/{space}/parents/{msg}?activityType=reaction{}", super::activity_flags());
        let r = self.http("GET", &self.url("conversation", &q), None)?;
        let items: Vec<Value> = r["items"].as_array().cloned().or_else(|| r.as_array().cloned()).unwrap_or_default();
        let n = remember_actors(&mut self.hist.names, &items);
        dbg_conn(&format!("reactor names: {} reactions, {n} with a named actor, for a message in {space}", items.len()));
        if self.hist.acts.contains_key(space) { self.refold(space); }
        Ok(())
    }

    /// What the join/leave collapsing needs to know about an add / leave info row (`None` for any other row, and in a 1:1).
    pub(super) fn join_leave(&self, a: &Value, space: &str) -> Option<JoinLeave> {
        let (o, actor) = (&a["object"], &a["actor"]);
        if o["objectType"] != "person" || self.spaces.get(space).is_some_and(|s| s.direct) { return None }
        let add = match a["verb"].as_str()? { "add" => true, "leave" => false, _ => return None };
        let same = |x: &Value, y: &Value| { let (i, u) = (st(x, "id"), st(x, "entryUUID")); (!i.is_empty() && i == st(y, "id")) || (!u.is_empty() && u == st(y, "entryUUID")) };
        let person = Person { id: [st(o, "entryUUID"), st(o, "id")].into_iter().find(|s| !s.is_empty()).unwrap_or_default(),
            name: st(o, "displayName"), email: st(o, "emailAddress"), status: String::new() };
        Some(JoinLeave { add, own: actor.is_null() || same(actor, o), actor: [st(actor, "entryUUID"), st(actor, "id")].into_iter().find(|s| !s.is_empty()).unwrap_or_default(),
            actor_name: st(actor, "displayName"), actor_me: self.actor_is_me(actor), person })
    }

    /// Read-only self-check: real reactions carry the server-accepted `object.hmac`. Recompute it with candidate formulas
    /// from the parent message's key and compare (counts only are logged). The listing only holds reaction *summaries*;
    /// the individual reactions come from `GET <conversation>/parents/<parentId>?activityType=reaction`.
    pub(crate) fn hmac_oracle(&mut self, space: &str, acts: &[Value], space_key: &str) {
        use hmac::{Hmac, Mac};
        // Diagnostic only (`WEBOX_HMAC_ORACLE=1`): it reads reactions of a space; nothing is ever sent.
        if self.oracle_done || std::env::var_os("WEBOX_HMAC_ORACLE").is_none() { return }
        let hx = |key: &[u8], msg: &str| -> String {
            let mut m = <Hmac<sha2::Sha256> as Mac>::new_from_slice(key).unwrap();
            m.update(msg.as_bytes());
            m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
        };
        let parents: Vec<String> = acts.iter().filter(|a| a["parent"]["type"] == "reactionSummary").map(|a| st(&a["parent"], "id")).take(3).collect();
        if parents.is_empty() { return }
        let mut tally: std::collections::BTreeMap<String, (u32, u32)> = Default::default();
        let (mut seen, mut with_hmac, mut shown) = (0, 0, false);
        for pid in parents {
            let Some(parent) = acts.iter().find(|p| st(p, "id") == pid) else { continue };
            let r = self.http("GET", &self.url("conversation", &format!("/conversations/{space}/parents/{pid}?activityType=reaction")), None);
            let Ok(r) = r else { dbg_conn("ORACLE reaction children request failed"); continue };
            let items: Vec<Value> = r["items"].as_array().cloned().or_else(|| r.as_array().cloned()).unwrap_or_default();
            for a in items.iter().take(4) {
                seen += 1;
                if !shown {
                    shown = true;
                    let keys = |v: &Value| v.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>());
                    dbg_conn(&format!("ORACLE reaction activity keys {:?} object keys {:?} parent keys {:?}", keys(a), keys(&a["object"]), keys(&a["parent"])));
                }
                let Some(want) = a["object"]["hmac"].as_str() else { continue };
                dbg_conn(&format!("ORACLE sample: hmac len {} hex {} ; displayName parts {} len {} ; reaction key == parent key {} ; == space key {} ; parent.id==summary parent {}",
                    want.len(), want.chars().all(|c| c.is_ascii_hexdigit()), st(&a["object"], "displayName").split('.').count(), st(&a["object"], "displayName").len(),
                    st(a, "encryptionKeyUrl") == st(parent, "encryptionKeyUrl"), st(a, "encryptionKeyUrl") == space_key, st(&a["parent"], "id") == pid));
                with_hmac += 1;
                let name_enc = st(&a["object"], "displayName");
                let name_raw = { let ku = Some(st(a, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or(space_key.to_string()); self.decrypt(&name_enc, &ku) };
                let pkey_url = Some(st(parent, "encryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or(space_key.to_string());
                let mut variants: Vec<(String, Vec<u8>, String)> = vec![];
                for (kl, ku) in [("parentKey", pkey_url.clone()), ("spaceKey", space_key.to_string()), ("reactionKey", st(a, "encryptionKeyUrl"))] {
                    if ku.is_empty() { continue }
                    let Ok(key) = self.key(&ku) else { continue };
                    let k = jose::b64(&key);
                    let forms = [("kkidkty", format!(r#"{{"k":"{k}","kid":"{ku}","kty":"oct"}}"#)), ("ktykkid", format!(r#"{{"kty":"oct","k":"{k}","kid":"{ku}"}}"#)),
                        ("ktykidk", format!(r#"{{"kty":"oct","kid":"{ku}","k":"{k}"}}"#)), ("kkty", format!(r#"{{"k":"{k}","kty":"oct"}}"#))];
                    for (jl, j) in forms {
                        for (pl, p) in [("id", pid.clone()), ("url", st(parent, "url"))] {
                            variants.push((format!("{kl}/{jl}/{pl}/keyId"), pid.as_bytes().to_vec(), format!("{j}{p}{name_raw}")));
                            variants.push((format!("{kl}/{jl}/{pl}/keyUrl"), st(parent, "url").as_bytes().to_vec(), format!("{j}{p}{name_raw}")));
                        }
                    }
                    for (jl, j) in [("kkidkty-encname", format!(r#"{{"k":"{k}","kid":"{ku}","kty":"oct"}}"#))] {
                        variants.push((format!("{kl}/{jl}/id/keyId"), pid.as_bytes().to_vec(), format!("{j}{pid}{name_enc}")));
                    }
                    variants.push((format!("{kl}/rawkey-over-id+name"), key.clone(), format!("{pid}{name_raw}")));
                    variants.push((format!("{kl}/rawkey-over-name"), key.clone(), name_raw.clone()));
                }
                for (label, key, msg) in variants {
                    let e = tally.entry(label).or_default();
                    e.1 += 1;
                    if hx(&key, &msg) == want { e.0 += 1; }
                }
            }
        }
        self.oracle_done = true;
        let hits: Vec<String> = tally.iter().filter(|(_, v)| v.0 > 0).map(|(k, v)| format!("{k}: {}/{}", v.0, v.1)).collect();
        dbg_conn(&format!("ORACLE reactions fetched {seen}, with hmac {with_hmac}, {} variants tested, matching: {hits:?}", tally.len()));
    }
    /// Adds activities to a space's raw cache (deduplicated by id, oldest first); returns how many were new.
    /// A conversation has one `create`, but for older 1:1s the service makes one up on every `maxDate` request, each time with
    /// a new id, the conversation's own `published` and a nameless actor. Deduplicating by id alone would add another
    /// "created" row on every scroll to the top, so only the first `create` is kept.
    ///
    /// A deleted message: the service replaces the activity by a `tombstone` with the same id (and the deletion time), so a
    /// cached post/share is replaced when its tombstone arrives (a reload) and turned into one when a `delete` of it is known
    /// (a live event or my own delete, which only point at it). Without that the message stayed, as the id is already known.
    pub(crate) fn merge_acts(&mut self, space: &str, items: Vec<Value>) -> usize {
        let list = self.hist.acts.entry(space.to_string()).or_default();
        let mut known: HashMap<String, usize> = list.iter().enumerate().map(|(i, a)| (st(a, "id"), i)).collect();
        let mut has_create = list.iter().any(|a| a["verb"] == "create");
        let mut added = 0;
        for a in items {
            if a["verb"] == "create" { if has_create { continue } has_create = true; }
            match known.get(&st(&a, "id")) {
                Some(&i) => if a["verb"] == "tombstone" && list[i]["verb"] != "tombstone" { list[i] = a; added += 1; },
                None => { known.insert(st(&a, "id"), list.len()); list.push(a); added += 1; }
            }
        }
        added += tombstone_deleted(list);
        if added > 0 { list.sort_by(|a, b| st(a, "published").cmp(&st(b, "published"))); }
        added
    }
}

/// Turns every cached post/share that a cached `delete` points at into the tombstone the service makes of it: same id and
/// parent, `actor` = who deleted it, `object.actor` = who wrote it, `published` = the deletion time (the official client shows
/// "Sie haben Ihre Nachricht gelöscht. 15.12.2025, 15:08" in its place). Returns how many were turned.
fn tombstone_deleted(list: &mut [Value]) -> usize {
    let deletes: HashMap<String, (Value, String)> = list.iter().filter(|a| a["verb"] == "delete")
        .map(|d| (st(&d["object"], "id"), (d["actor"].clone(), st(d, "published")))).filter(|(id, _)| !id.is_empty()).collect();
    if deletes.is_empty() { return 0 }
    let mut n = 0;
    for a in list.iter_mut().filter(|a| a["verb"] == "post" || a["verb"] == "share") {
        let Some((by, at)) = deletes.get(&st(a, "id")) else { continue };
        let published = if at.is_empty() { st(a, "published") } else { at.clone() };
        *a = json!({"id": st(a, "id"), "verb": "tombstone", "published": published, "actor": by, "parent": a["parent"].clone(), "target": a["target"].clone(),
            "object": {"id": st(a, "id"), "objectType": "activity", "verb": a["verb"].clone(), "actor": a["actor"].clone()}});
        n += 1;
    }
    n
}

/// Remembers the named actors of `items` by uuid (their entryUUID and id); returns how many had a name.
pub(super) fn remember_actors(names: &mut HashMap<String, (String, String)>, items: &[Value]) -> usize {
    let mut n = 0;
    for a in items {
        let actor = &a["actor"];
        let name = st(actor, "displayName");
        if name.trim().is_empty() { continue }
        n += 1;
        for k in [st(actor, "entryUUID"), st(actor, "id")] { if !k.is_empty() { names.insert(k, (name.clone(), st(actor, "emailAddress"))); } }
    }
    n
}

/// The reactors of one entry of a reaction summary (`users: [{id, published}]`), in the service's order, each once.
pub(crate) fn reactors(entry: &Value) -> Vec<Reactor> {
    let mut out: Vec<Reactor> = vec![];
    for u in entry["users"].as_array().into_iter().flatten() {
        let id = st(u, "id");
        if !id.is_empty() && !out.iter().any(|r| r.id == id) { out.push(Reactor { id, published: st(u, "published"), ..Default::default() }); }
    }
    out
}
