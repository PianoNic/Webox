//! Everything done to a space, its people or a team beyond what `space_ops.rs` already covers: read state,
//! per-space notifications, moderators, removing people, archiving, teams (create, add/remove a space, members, leave,
//! archive, join a team space) and the full participant list of a space.
//!
//! Protocol notes:
//! - Conversation verbs: `leave`, `add`, `assignModerator` / `unassignModerator`, `tag` / `untag` (mute tags
//!   `MESSAGE_NOTIFICATIONS_OFF/ON`, `MENTION_NOTIFICATIONS_OFF/ON`), the simple verbs `favorite` / `unfavorite` / `hide` /
//!   `unhide`, `acknowledge` / `unacknowledge`, `archive` / `unarchive`.
//! - Teams: create = POST conversation `/teams` (a conversation creation with objectType "team" + summary); add / remove a
//!   space on the team's general conversation; add / remove members; archive / unarchive; join a team space =
//!   POST `<team.url>/conversations/<id>/participants`; list = GET `<team.url>/conversations`; the team list request is
//!   `/teams?includeTeamMembers=false&includeTeamConversations=true`.
//! - "Leave" on a 1:1 hides it (a `HIDE` activity comes back).
//! Nothing here runs on its own: every write is sent only when the user picks the action. Tests cover the bodies only.
use super::*;
use std::sync::Mutex;

pub const MSG_OFF: &str = "MESSAGE_NOTIFICATIONS_OFF";
pub const MSG_ON: &str = "MESSAGE_NOTIFICATIONS_ON";
pub const MENTION_OFF: &str = "MENTION_NOTIFICATIONS_OFF";
pub const MENTION_ON: &str = "MENTION_NOTIFICATIONS_ON";
/// The four tags that together describe a space's notification setting.
pub const MUTE_TAGS: [&str; 4] = [MENTION_OFF, MENTION_ON, MSG_OFF, MSG_ON];
/// Participants asked for when the Personen tab opens (`participantsLimit`; the list request only carries 10).
pub const PEOPLE_LIMIT: u32 = 500;

/// A space's notification setting ("Global", "All messages", "@Mentions only", "Off"; "Off and hide the space" =
/// `Off` + hide). Tags: messages muted = MESSAGE_…_OFF, mentions muted = MENTION_…_OFF, unmuted = …_ON, Global = no tag;
/// "@Mentions only" = messages off + mentions on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Notify { #[default] Global, All, Mentions, Off }

impl Notify {
    pub fn from_tags(tags: &[&str]) -> Self {
        let has = |t: &str| tags.contains(&t);
        if has("MUTED") || (has(MSG_OFF) && has(MENTION_OFF)) { Notify::Off }
        else if has(MSG_OFF) { Notify::Mentions }
        else if has(MSG_ON) || has(MENTION_ON) { Notify::All }
        else { Notify::Global }
    }
    /// The tags that express this setting (after all mute tags were removed).
    pub fn tags(self) -> &'static [&'static str] {
        match self { Notify::Global => &[], Notify::All => &[MSG_ON, MENTION_ON], Notify::Mentions => &[MSG_OFF, MENTION_ON], Notify::Off => &[MSG_OFF, MENTION_OFF] }
    }
    pub fn muted(self) -> bool { self == Notify::Off }
}

/// What the space list does not carry in `Space`: notification setting and moderators (person ids as in `Person.id`, plus uuids).
#[derive(Clone, Debug, Default)]
pub struct SpaceInfo { pub id: String, pub notify: Notify, pub moderators: Vec<String>, pub locked: bool }

/// A team as the conversation list describes it (`team.generalConversationUuid`, `team.url`, `team.archived`).
#[derive(Clone, Debug, Default)]
pub struct TeamInfo { pub id: String, pub general: String, pub url: String, pub archived: bool }

/// A space of a team (GET `<team.url>/conversations`): `joined` = I am a participant.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TeamSpace { pub id: String, pub title: String, pub joined: bool }

pub enum SpaceOp {
    MarkRead(String), MarkUnread(String),
    /// (space, setting); `hide` = "Off and hide the space".
    Notify(String, Notify, bool),
    /// (space, person id as in `Person.id`).
    RemovePerson(String, String),
    /// (space, person, on).
    Moderator(String, String, bool),
    /// Load the full participant list (read-only GET) → `SpaceEvent::People`.
    People(String),
    Archive(String),
    TeamCreate { name: String, summary: String, people: Vec<String> },
    TeamAddSpace { team: String, space: String },
    TeamRemoveSpace { team: String, space: String },
    TeamAddMember { team: String, person: String },
    TeamLeave(String),
    TeamArchive(String),
    /// Read-only GET of a team's spaces → `SpaceEvent::TeamSpaces`.
    TeamSpaces(String),
    TeamJoin { team: String, space: String },
    /// Create a space inside a team with these participants.
    TeamCreateSpace { team: String, title: String, people: Vec<String> },
    /// Create a group space with these participants in the same request.
    CreateWith { title: String, people: Vec<String> },
}

pub enum SpaceEvent {
    Info(Vec<SpaceInfo>, Vec<TeamInfo>),
    /// (space, people, moderator ids).
    People(String, Vec<Person>, Vec<String>),
    TeamSpaces(String, Vec<TeamSpace>),
}

/// Team facts kept between requests (the Session struct is shared code, so they live here).
static TEAMS: Mutex<Option<HashMap<String, TeamInfo>>> = Mutex::new(None);

fn team_info(id: &str) -> Option<TeamInfo> { TEAMS.lock().ok().and_then(|m| m.as_ref().and_then(|m| m.get(id).cloned())) }
/// Whether the last list load knew this team (cloud/live_list/: a new conversation of an unknown team reloads the list).
pub(super) fn team_known(id: &str) -> bool { team_info(id).is_some() }

// Pure body builders (unit-tested).

/// `tag` / `untag`: object = the conversation plus the tags.
pub fn tag_body(verb: &str, space: &str, tags: &[&str]) -> Value {
    json!({"verb": verb, "object": {"id": space, "objectType": "conversation", "tags": tags}})
}
/// Simple verbs (favorite / unfavorite / hide / unhide): object = target = the conversation.
pub fn simple_body(verb: &str, space: &str) -> Value { json!({"verb": verb, "object": {"id": space, "objectType": "conversation"}}) }
/// Mark unread: verb `unacknowledge` on the newest message, object shaped like `acknowledge`.
pub fn unack_body(activity: &str) -> Value { json!({"verb": "unacknowledge", "object": {"id": activity, "objectType": "activity"}}) }
/// Mark read: `acknowledge` of the newest message (as the read receipt).
pub fn ack_body(activity: &str) -> Value { json!({"verb": "acknowledge", "object": {"id": activity, "objectType": "activity"}}) }
/// `assignModerator` / `unassignModerator`.
pub fn moderator_body(on: bool, person: &str) -> Value {
    json!({"verb": if on { "assignModerator" } else { "unassignModerator" }, "object": {"id": person, "objectType": "person"}})
}
/// `leave(conversation, participant)`: removes another person (kmsMessage deletes their authorization).
pub fn remove_body(person: &str, kms: &str) -> Value {
    json!({"verb": "leave", "object": {"id": person, "objectType": "person"}, "kmsMessage": kms})
}
/// The KMS request of `leave` / `remove`: `<KRO>/authorizations?authId=<id>`.
pub fn kms_delete_auth(resource: &str, auth_id: &str) -> Value {
    json!({"method": "delete", "uri": format!("{resource}/authorizations?authId={auth_id}")})
}
/// Add a space to a team: on the team's general conversation, object = the space; the space's KRO authorizes the general KRO.
pub fn team_add_space_body(space: &str, kms: &str) -> Value {
    json!({"verb": "add", "object": {"id": space, "objectType": "conversation"}, "kmsMessage": kms})
}
pub fn kms_team_add(space_kro: &str, general_kro: &str) -> Value {
    json!({"method": "create", "uri": format!("{space_kro}/authorizations"), "userIds": [general_kro]})
}
/// Remove a space from a team: verb "remove" on the general conversation; kms `<general KRO>/authorizations?authId=<space id>`.
pub fn team_remove_space_body(space: &str, kms: &str) -> Value {
    json!({"verb": "remove", "object": {"id": space, "objectType": "conversation"}, "kmsMessage": kms})
}
/// Archive: object = target = the team (or a conversation); objectType decides which.
pub fn archive_body(id: &str, object_type: &str) -> Value {
    json!({"verb": "archive", "objectType": "activity", "object": {"id": id, "objectType": object_type}, "target": {"id": id, "objectType": object_type}})
}
/// A conversation creation (a team when `team` is set): create + one add per participant (me first), kmsMessage creating
/// the resource for all of them with the new key.
pub fn creation_body(me: &str, people: &[String], key_uri: &str, title_jwe: &str, summary_jwe: Option<&str>, kms: &str, team: bool) -> Value {
    let me_o = json!({"id": me, "objectType": "person"});
    let mut items = vec![json!({"verb": "create", "actor": me_o})];
    let mut parts = vec![me_o.clone()];
    for p in std::iter::once(me.to_string()).chain(people.iter().filter(|p| p.as_str() != me).cloned()) {
        let o = json!({"id": p, "objectType": "person"});
        items.push(json!({"verb": "add", "actor": me_o, "object": o}));
        if p != me { parts.push(o); }
    }
    let mut b = json!({"objectType": if team { "team" } else { "conversation" }, "displayName": title_jwe,
        "encryptionKeyUrl": key_uri, "defaultActivityEncryptionKeyUrl": key_uri, "kmsMessage": kms,
        "participants": {"items": parts}, "activities": {"items": items}});
    if let Some(s) = summary_jwe { b["summary"] = json!(s); }
    b
}

/// The notification setting, moderators and lock state of one conversation of the list answer.
pub fn space_info(c: &Value) -> SpaceInfo {
    let tags: Vec<&str> = c["tags"].as_array().map(|t| t.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut moderators = vec![];
    for p in c["participants"]["items"].as_array().into_iter().flatten() {
        if p["roomProperties"]["isModerator"].as_bool() == Some(true) {
            for k in ["id", "entryUUID", "emailAddress"] { let v = st(p, k); if !v.is_empty() { moderators.push(v); } }
        }
    }
    SpaceInfo { id: st(c, "id"), notify: Notify::from_tags(&tags), moderators, locked: tags.contains(&"LOCKED") }
}

pub fn team_of(c: &Value) -> Option<TeamInfo> {
    let t = &c["team"];
    let id = st(t, "id");
    (!id.is_empty()).then(|| TeamInfo { id, general: st(t, "generalConversationUuid"), url: st(t, "url"), archived: t["archived"].as_bool().unwrap_or(false) })
}

fn is_uuid(s: &str) -> bool { s.len() == 36 && s.matches('-').count() == 4 }

impl Session {
    /// Called once per list load (cloud.rs `spaces()`): sends notification settings, moderators and team facts.
    pub(super) fn scan_space_extras(&mut self, items: &[Value]) {
        let infos: Vec<SpaceInfo> = items.iter().map(space_info).collect();
        let mut teams: HashMap<String, TeamInfo> = HashMap::new();
        for c in items { if let Some(t) = team_of(c) { teams.entry(t.id.clone()).or_insert(t); } }
        if let Ok(mut m) = TEAMS.lock() { *m = Some(teams.clone()); }
        let _ = self.out.send(Resp::SpaceEvent(SpaceEvent::Info(infos, teams.into_values().collect())));
    }

    /// A person as the services want it: the Webex uuid (participants also come with e-mail-style ids).
    fn uuid_for(&self, who: &str) -> String {
        if is_uuid(who) { return who.to_string(); }
        if who.eq_ignore_ascii_case(&self.my_email) && !self.user.is_empty() { return self.user.clone(); }
        self.uuids.get(who).cloned().unwrap_or_else(|| who.to_string())
    }

    fn kms_msg(&self, mut req: Value) -> Result<String, String> {
        let (kid, key) = self.kms.as_ref().and_then(|k| k.eph.clone()).ok_or("no key-service session")?;
        req["client"] = self.client();
        req["requestId"] = json!(jose::uuid());
        jose::encrypt_dir(&key, &kid, req.to_string().as_bytes())
    }

    fn kro(&self, id: &str) -> Result<String, String> {
        self.spaces.get(id).map(|s| s.kms_resource.clone()).filter(|r| !r.is_empty()).ok_or_else(|| "this space has no key-service resource".to_string())
    }

    fn team_url(&self, team: &str) -> String {
        team_info(team).map(|t| t.url).filter(|u| u.starts_with("https://") && webex_host(u)).unwrap_or_else(|| self.url("conversation", &format!("/teams/{team}")))
    }

    fn general_of(&self, team: &str) -> Result<String, String> {
        team_info(team).map(|t| t.general).filter(|g| !g.is_empty()).ok_or_else(|| "the team's general space is not known".to_string())
    }

    /// The newest activity id of a space (read-only GET, limit 1), in the same form as `newest_query` (maxDate now + 7 days,
    /// uuid entry format).
    fn newest_activity(&self, space: &str) -> Result<String, String> {
        let v = self.http("GET", &self.url("conversation", &format!("/conversations/{space}/activities?limit=1&maxDate={}{}", now_ms_() + 604_800_000, super::activities::ACTIVITY_FLAGS)), None)?;
        v["items"].as_array().and_then(|a| a.last()).map(|a| st(a, "id")).filter(|s| !s.is_empty()).ok_or_else(|| "the space has no messages".to_string())
    }

    /// A new key (KMS create /keys) → (uri, key).
    fn new_key(&mut self) -> Result<(String, Vec<u8>), String> {
        let v = self.kms_call(json!({"method": "create", "uri": "/keys", "count": 1}))?;
        let k = &v["keys"][0];
        let (uri, key) = (st(k, "uri"), jose::oct_key(&k["jwk"])?);
        self.keys.insert(uri.clone(), key.clone());
        Ok((uri, key))
    }

    pub(super) fn space_op(&mut self, op: SpaceOp) -> Result<(), String> {
        match op {
            SpaceOp::MarkRead(id) => { let a = self.newest_activity(&id)?; self.submit(&id, ack_body(&a)) }
            SpaceOp::MarkUnread(id) => { let a = self.newest_activity(&id)?; self.submit(&id, unack_body(&a)) }
            SpaceOp::Notify(id, n, hide) => {
                self.submit(&id, tag_body("untag", &id, &MUTE_TAGS))?;
                if !n.tags().is_empty() { self.submit(&id, tag_body("tag", &id, n.tags()))?; }
                if hide { self.submit(&id, simple_body("hide", &id))?; }
                self.spaces()
            }
            SpaceOp::RemovePerson(id, p) => {
                let u = self.uuid_for(&p);
                let kms = self.kms_msg(kms_delete_auth(&self.kro(&id)?, &u))?;
                self.submit(&id, remove_body(&u, &kms))?;
                self.people(&id)
            }
            SpaceOp::Moderator(id, p, on) => { let u = self.uuid_for(&p); self.submit(&id, moderator_body(on, &u))?; self.people(&id) }
            SpaceOp::People(id) => self.people(&id),
            SpaceOp::Archive(id) => { self.submit(&id, archive_body(&id, "conversation"))?; self.spaces() }
            SpaceOp::TeamCreate { name, summary, people } => {
                let people: Vec<String> = people.iter().map(|p| self.uuid_for(p)).collect();
                let (uri, key) = self.new_key()?;
                let mut users = vec![self.user.clone()]; users.extend(people.iter().filter(|p| **p != self.user).cloned());
                let kms = self.kms_msg(json!({"method": "create", "uri": "/resources", "userIds": users, "keyUris": [uri]}))?;
                let title = jose::encrypt_dir(&key, &uri, name.as_bytes())?;
                let sum = if summary.trim().is_empty() { None } else { Some(jose::encrypt_dir(&key, &uri, summary.as_bytes())?) };
                let body = creation_body(&self.user, &people, &uri, &title, sum.as_deref(), &kms, true);
                self.http("POST", &self.url("conversation", "/teams"), Some(body))?;
                self.spaces()
            }
            SpaceOp::TeamAddSpace { team, space } => {
                let general = self.general_of(&team)?;
                let kms = self.kms_msg(kms_team_add(&self.kro(&space)?, &self.kro(&general)?))?;
                self.submit(&general, team_add_space_body(&space, &kms))?;
                self.spaces()
            }
            SpaceOp::TeamRemoveSpace { team, space } => {
                let general = self.general_of(&team)?;
                let kms = self.kms_msg(kms_delete_auth(&self.kro(&general)?, &space))?;
                self.submit(&general, team_remove_space_body(&space, &kms))?;
                self.spaces()
            }
            SpaceOp::TeamAddMember { team, person } => {
                let general = self.general_of(&team)?;
                let u = self.uuid_for(&person);
                let kms = self.kms_msg(json!({"method": "create", "uri": format!("{}/authorizations", self.kro(&general)?), "userIds": [u]}))?;
                self.submit(&general, json!({"verb": "add", "object": {"objectType": "person", "id": u}, "kmsMessage": kms}))?;
                self.spaces()
            }
            SpaceOp::TeamLeave(team) => {
                // Leaving a team = leave on its general conversation.
                let general = self.general_of(&team)?;
                let me = self.user.clone();
                let kms = self.kms_msg(kms_delete_auth(&self.kro(&general)?, &me))?;
                self.submit(&general, json!({"verb": "leave", "object": {"objectType": "person", "id": me}, "kmsMessage": kms}))?;
                self.spaces()
            }
            SpaceOp::TeamArchive(team) => {
                let mut b = archive_body(&team, "team");
                b["clientTempId"] = json!(jose::uuid());
                b["actor"] = json!({"objectType": "person", "id": self.user});
                // Team activities go to `<team.url>/activities`.
                self.http("POST", &format!("{}/activities", self.team_url(&team)), Some(b))?;
                self.spaces()
            }
            SpaceOp::TeamSpaces(team) => self.team_spaces(&team),
            SpaceOp::TeamJoin { team, space } => {
                self.http("POST", &format!("{}/conversations/{space}/participants", self.team_url(&team)), None)?;
                self.spaces()?;
                self.team_spaces(&team)
            }
            SpaceOp::TeamCreateSpace { team, title, people } => {
                let general = self.general_of(&team)?;
                let people: Vec<String> = people.iter().map(|p| self.uuid_for(p)).collect();
                let (uri, key) = self.new_key()?;
                let mut users: Vec<String> = vec![self.user.clone()]; users.extend(people.iter().filter(|p| **p != self.user).cloned());
                users.push(self.kro(&general)?); // the general KRO may read the title
                let kms = self.kms_msg(json!({"method": "create", "uri": "/resources", "userIds": users, "keyUris": [uri]}))?;
                let body = creation_body(&self.user, &people, &uri, &jose::encrypt_dir(&key, &uri, title.as_bytes())?, None, &kms, false);
                self.http("POST", &format!("{}/conversations", self.team_url(&team)), Some(body))?;
                self.spaces()
            }
            SpaceOp::CreateWith { title, people } => {
                let people: Vec<String> = people.iter().map(|p| self.uuid_for(p)).collect();
                let (uri, key) = self.new_key()?;
                let mut users = vec![self.user.clone()]; users.extend(people.iter().filter(|p| **p != self.user).cloned());
                let kms = self.kms_msg(json!({"method": "create", "uri": "/resources", "userIds": users, "keyUris": [uri]}))?;
                let body = creation_body(&self.user, &people, &uri, &jose::encrypt_dir(&key, &uri, title.as_bytes())?, None, &kms, false);
                self.http("POST", &self.url("conversation", "/conversations"), Some(body))?;
                self.spaces()
            }
        }
    }

    /// Full participant list + moderators of a space (read-only).
    fn people(&mut self, id: &str) -> Result<(), String> {
        let v = self.http("GET", &self.url("conversation", &format!("/conversations/{id}?activitiesLimit=0&participantsLimit={PEOPLE_LIMIT}")), None)?;
        let raw = v["participants"]["items"].as_array().cloned().unwrap_or_default();
        let mut list = vec![];
        for p in &raw {
            let person = Person { id: st(p, "id"), name: st(p, "displayName"), email: st(p, "emailAddress"), status: String::new() };
            self.remember_uuid(&person.id, p);
            list.push(person);
        }
        let info = space_info(&v);
        let _ = self.out.send(Resp::SpaceEvent(SpaceEvent::People(id.to_string(), list, info.moderators)));
        Ok(())
    }

    /// The spaces of a team (read-only): title decrypted with each space's own key.
    fn team_spaces(&mut self, team: &str) -> Result<(), String> {
        let v = self.http("GET", &format!("{}/conversations", self.team_url(team)), None)?;
        let mut out = vec![];
        for c in v["items"].as_array().cloned().unwrap_or_default() {
            let id = st(&c, "id");
            let title = self.decrypt(&st(&c, "displayName"), &st(&c, "encryptionKeyUrl"));
            out.push(TeamSpace { joined: self.spaces.contains_key(&id), id, title });
        }
        let _ = self.out.send(Resp::SpaceEvent(SpaceEvent::TeamSpaces(team.to_string(), out)));
        Ok(())
    }
}

#[cfg(test)]
mod tests;

/// Unix ms now (for the activity request's maxDate).
fn now_ms_() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }
