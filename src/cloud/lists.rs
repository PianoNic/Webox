//! The space list, creating a space and the meeting list.
use super::*;

impl Session {
    pub(super) fn spaces(&mut self) -> Result<(), String> {
        let url = self.url("conversation", &format!("/conversations?conversationsLimit={CONVERSATION_PAGE}&activitiesLimit=0&participantsLimit=10"));
        let v = self.http("GET", &url, None)?;
        let items = v["items"].as_array().cloned().unwrap_or_default();
        let mut people: HashMap<String, Person> = HashMap::new();
        let mut teams: HashMap<String, Team> = HashMap::new();
        let mut list = vec![];
        for c in &items { list.push(self.space_of(c, &mut people, &mut teams)); }
        name_team_generals(&mut list, &mut teams, &crate::tr!("QObject", "General"));
        for sp in &list { self.spaces.insert(sp.id.clone(), sp.clone()); }
        list.sort_by(|a, b| b.last.cmp(&a.last));
        let mut people: Vec<Person> = people.into_values().collect();
        // Statuses are filled in below, once the full set of people to ask about is known.
        people.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        dbg_conn(&format!("spaces: {} loaded", list.len()));
        let _ = self.out.send(Resp::Spaces(list));
        let _ = self.out.send(Resp::Teams(teams.into_values().collect()));
        self.scan_space_extras(&items); // notification settings, moderators, team facts
        self.fetch_images(); // space avatars (and any queued image) in the background
        let mut want: Vec<Person> = self.me_person().into_iter().collect();
        want.extend(self.spaces.values().filter(|s| s.direct).flat_map(|s| s.participants.clone()));
        want.extend(people.iter().cloned());
        let mut seen = std::collections::HashSet::new();
        want.retain(|p| seen.insert(p.id.clone()));
        let last_active = self.presence(&want);
        for (id, status, _) in &last_active { if let Some(p) = people.iter_mut().find(|p| p.id == *id) { p.status = status.clone(); } }
        self.subscribe_presence(&want);
        self.load_avatars(&want);
        let _ = self.out.send(Resp::People(people));
        people::send_presence(&self.out, last_active);
        Ok(())
    }

    /// One conversation of the list answer (or of a single-conversation GET, cloud/live_list/) as a `Space`: title,
    /// participants, team, cover / avatar jobs, read state. Remembers it in `self.spaces`; other people go to `people`, a new
    /// team to `teams`.
    pub(super) fn space_of(&mut self, c: &Value, people: &mut HashMap<String, Person>, teams: &mut HashMap<String, Team>) -> Space {
        {
            let tags: Vec<&str> = c["tags"].as_array().map(|t| t.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let parts: Vec<Person> = c["participants"]["items"].as_array().map(|p| p.iter().map(|p| Person {
                id: st(p, "id"), name: st(p, "displayName"), email: st(p, "emailAddress"), status: String::new() }).collect()).unwrap_or_default();
            for (p, raw) in parts.iter().zip(c["participants"]["items"].as_array().into_iter().flatten()) { self.remember_uuid(&p.id, raw); }
            for p in &parts {
                if self.is_me(p) { let _ = self.out.send(Resp::Me(p.clone())); } else { people.insert(p.id.clone(), p.clone()); }
            }
            let direct = tags.contains(&"ONE_ON_ONE");
            let key_url = st(c, "encryptionKeyUrl");
            // The conversation's own url: typing requests go there (cloud/typing.rs).
            if !st(c, "url").is_empty() { self.typing.conv_url.insert(st(c, "id"), st(c, "url")); }
            // A direct chat whose other person is gone (participants = only me) still carries its own displayName
            // (decrypt() passes non-JWE text through): use it, then "Untitled".
            let title = if direct {
                let other = parts.iter().find(|p| !self.is_me(p)).map(|p| p.name.clone());
                let own = if other.as_deref().is_some_and(|n| !n.trim().is_empty()) || st(c, "displayName").is_empty() { String::new() } else { self.decrypt(&st(c, "displayName"), &key_url) };
                direct_title(other.as_deref(), &own, &crate::tr!("ConversationHelper", "Untitled"))
            } else { self.decrypt(&st(c, "displayName"), &key_url) };
            // A space's own cover: an extension with an avatar-service url (`extensions.items[].avatarUrl`).
            if let Some(u) = c["extensions"]["items"].as_array().into_iter().flatten().map(|e| st(e, "avatarUrl")).find(|u| u.starts_with("https://") && webex_host(u)) {
                self.space_cover_urls.insert(st(c, "id"), u);
            }
            // A space's own picture (round avatar): `avatar.files.items[0] {url, scr, mimeType}` under `avatarEncryptionKeyUrl`.
            if let Some(f) = c["avatar"]["files"]["items"].as_array().and_then(|a| a.first()) {
                let (scr, ku) = (st(f, "scr"), Some(st(c, "avatarEncryptionKeyUrl")).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.clone()));
                if !scr.is_empty() && st(f, "mimeType").starts_with("image/") {
                    self.img_jobs.push(images::ImgJob { key: format!("spaceavatar:{}", st(c, "id")), scr, key_url: ku, fallback: None });
                }
            }
            if std::env::var_os("WEBOX_TEAM_PROBE").is_some() && !c["team"].is_null() {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new(); st(&c["team"], "id").hash(&mut h);
                let keys: Vec<String> = c["team"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
                let colourish: Vec<(String, String)> = keys.iter().filter(|k| { let l = k.to_lowercase(); l.contains("color") || l.contains("colour") || l.contains("theme") || l.contains("avatar") || l.contains("image") })
                    .map(|k| (k.clone(), match &c["team"][k] { Value::String(s) if k.to_lowercase().contains("color") => s.chars().take(20).collect(), Value::String(s) => format!("s{}", s.len()), v => v.to_string().chars().take(40).collect() })).collect();
                dbg_conn(&format!("TEAMPROBE id#{:04x} keys {keys:?} colourish {colourish:?}", h.finish() as u16));
            }
            let team = st(&c["team"], "id");
            if !team.is_empty() && !teams.contains_key(&team) {
                let name = self.decrypt(&st(&c["team"], "displayName"), &st(&c["team"], "encryptionKeyUrl"));
                teams.insert(team.clone(), Team { id: team.clone(), name, color: parse_team_color(&st(&c["team"], "teamColor")) });
            }
            let last = st(c, "lastReadableActivityDate");
            let sp = Space { id: st(c, "id"), title, direct, team, unread: last > st(c, "lastSeenActivityDate"),
                favorite: tags.contains(&"FAVORITE"), public: is_public(&tags), hidden: tags.contains(&"HIDDEN"),
                kms_resource: st(c, "kmsResourceObjectUrl"), last_seen: st(c, "lastSeenActivityDate"), last, key_url, participants: parts,
                closed: false, announcement: false,
                // the conversation's meeting locus ("sync conversation locus_url"); a running meeting seen before stays known
                locus_url: Some(st(c, "locusUrl")).filter(|u| webex_host(u)).or_else(|| self.spaces.get(&st(c, "id")).map(|s| s.locus_url.clone())).unwrap_or_default(),
                meeting: self.spaces.get(&st(c, "id")).is_some_and(|s| s.meeting) };
            // Set after building: `parts` moved into the struct above.
            let self_moderator = c["participants"]["items"].as_array().into_iter().flatten()
                .any(|p| p["roomProperties"]["isModerator"].as_bool() == Some(true) && self.is_me(&Person { id: st(p, "id"), name: String::new(), email: st(p, "emailAddress"), status: String::new() }));
            let sp = Space { closed: closed_direct(sp.direct, tags.contains(&"READ_ONLY"), sp.participants.iter().filter(|p| !self.is_me(p)).count(),
                    features::feature(LEFT_COMPANY_RULE).and_then(|v| v.as_bool()) == Some(true)),
                announcement: announcement_blocked(sp.direct, tags.contains(&"ANNOUNCEMENT"), self_moderator), ..sp };
            self.spaces.insert(sp.id.clone(), sp.clone());
            sp
        }
    }

    pub(super) fn meetings(&mut self) -> Result<(), String> {
        let v = self.http("GET", &self.url("locus", "/loci?startingSoon=true&includePMRs=true"), None)?;
        // a space meeting among them marks its space as "meeting in progress" (or not any more)
        for l in v["loci"].as_array().into_iter().flatten() { self.space_meeting_locus(l, &locus::locus_url_of(l)); }
        let list =v["loci"].as_array().cloned().unwrap_or_default().iter().map(|l| Meeting {
            title: [st(&l["info"], "webExMeetingName"), st(&l["meeting"], "title"), st(&l["info"], "topic")].into_iter().find(|t| !t.is_empty()).unwrap_or_else(|| "Meeting".into()),
            start: hhmm(&st(&l["meeting"], "startTime")),
            link: [st(&l["info"], "webExMeetingLink"), st(&l["info"], "sipUri")].into_iter().find(|t| !t.is_empty()).unwrap_or_default(),
        }).collect();
        let _ = self.out.send(Resp::Meetings(list));
        Ok(())
    }
}

/// A public space: tag `OPEN`, or `DISCOVERABLE` (a favourite public space in the list answer carries `DISCOVERABLE` and a
/// `discoverability` object, no `OPEN`).
pub fn is_public(tags: &[&str]) -> bool { tags.contains(&"OPEN") || tags.contains(&"DISCOVERABLE") }

/// Team names by team id, from the last list load (the team start sentence of a general space needs the name, while the
/// space itself is titled "General").
static TEAM_NAMES: std::sync::Mutex<Option<HashMap<String, String>>> = std::sync::Mutex::new(None);
pub(super) fn team_name(id: &str) -> Option<String> {
    TEAM_NAMES.lock().ok().and_then(|m| m.as_ref().and_then(|m| m.get(id).cloned())).filter(|n| !n.is_empty())
}

/// A team's own space is its general space (tags `TEAM`, id == `team.id` == `team.generalConversationUuid`).
/// In the list answer its `team` object has no `displayName`; the team's name is the general conversation's own displayName.
/// Webex lists that space as "General" ("Allgemein") with the team's name below it in the team colour. So each
/// team without a name takes it from its general space, and the general space gets the localised title `general`.
pub fn name_team_generals(list: &mut [Space], teams: &mut HashMap<String, Team>, general: &str) {
    for s in list.iter_mut().filter(|s| !s.team.is_empty() && s.id == s.team) {
        let t = teams.entry(s.team.clone()).or_insert_with(|| Team { id: s.team.clone(), name: String::new(), color: None });
        if t.name.trim().is_empty() { t.name = s.title.clone(); }
        s.title = general.to_string();
    }
    if let Ok(mut m) = TEAM_NAMES.lock() { m.get_or_insert_with(HashMap::new).extend(teams.values().map(|t| (t.id.clone(), t.name.clone()))); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(id: &str, team: &str, title: &str) -> Space { Space { id: id.into(), team: team.into(), title: title.into(), ..Default::default() } }

    /// The general space is titled "General" with the team's name below it, even when the team object has no name.
    #[test]
    fn general_space_is_titled_general_and_names_its_team() {
        let mut list = vec![sp("T", "T", "Product Team"), sp("S", "T", "Design"), sp("X", "", "Lunch")];
        let mut teams: HashMap<String, Team> = [("T".to_string(), Team { id: "T".into(), name: String::new(), color: Some((1, 2, 3)) })].into();
        name_team_generals(&mut list, &mut teams, "Allgemein");
        assert_eq!(list[0].title, "Allgemein");
        assert_eq!(teams["T"].name, "Product Team");
        assert_eq!(teams["T"].color, Some((1, 2, 3)));
        assert_eq!(list[1].title, "Design"); // the team's other spaces keep their own titles
        assert_eq!(list[2].title, "Lunch");
        assert_eq!(team_name("T").as_deref(), Some("Product Team"));
    }

    /// A name the team object did carry is kept; a team only known from its general space is created.
    #[test]
    fn team_names_from_the_team_object_win_and_missing_teams_are_added() {
        let mut list = vec![sp("A", "A", "Old title"), sp("B", "B", "Ops")];
        let mut teams: HashMap<String, Team> = [("A".to_string(), Team { id: "A".into(), name: "Alpha".into(), color: None })].into();
        name_team_generals(&mut list, &mut teams, "General");
        assert_eq!(teams["A"].name, "Alpha");
        assert_eq!(teams["B"].name, "Ops");
        assert!(list.iter().all(|s| s.title == "General"));
    }

    #[test]
    fn open_and_discoverable_spaces_are_public() {
        assert!(is_public(&["OPEN", "JOINED"]));
        assert!(is_public(&["FAVORITE", "DISCOVERABLE"]));
        assert!(!is_public(&["FAVORITE", "WELCOME"]));
        assert!(!is_public(&["LOCKED", "TEAM", "JOINED"]));
    }
}
