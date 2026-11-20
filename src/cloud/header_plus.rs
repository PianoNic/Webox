//! One-to-one spaces for "Neue Nachricht" (header + > "Nachricht senden") and "Zu Webex einladen".
//! WRITE actions on the real account: they run only on the user's own click (send in the "Neue Nachricht" pane, "Einladung
//! senden"); tests and --sample never reach the service.
//!
//! Protocol:
//! - Every participant becomes a uuid (my uuid first). With exactly two, the existing 1:1 is looked up first:
//!   GET conversation `conversations/user/{uuid}` with
//!   `uuidEntryFormat=true&personRefresh=true&activitiesLimit=0&includeConvWithDeletedUserUUID=true&includeParticipants=true`;
//!   only a 404 leads to creating one: POST conversation `conversations` with `tags: ['ONE_ON_ONE']`. The body:
//!   `activities.items` = a `create` activity, then an `add` activity `{objectType:'person', id}` for every participant
//!   (me first), `objectType: 'conversation'`, `kmsMessage {method:'create', uri:'/resources', userIds:[uuids], keyUris:[]}`;
//!   no top-level `participants`, no displayName. An activity is `{actor:{objectType:'person', id: me},
//!   objectType:'activity', verb[, object]}`. A new unbound key's uri goes into `kmsMessage.keyUris`, and
//!   `encryptionKeyUrl` and `defaultActivityEncryptionKeyUrl` are set; the kmsMessage goes out wrapped for the KMS
//!   (as `Session::create` does).
//! - An e-mail becomes a uuid via POST conversation `/users` with the body `[{email}]` (plus `?shouldCreateUsers=true`
//!   when creating = inviting); the answer is keyed by the e-mail (`{<email>: {id, ...}}`).
use super::*;

/// One activity: `{actor, objectType:"activity", verb[, object]}`.
fn expand(me: &str, verb: &str, object: Option<Value>) -> Value {
    let mut a = json!({"actor": {"objectType": "person", "id": me}, "objectType": "activity", "verb": verb});
    if let Some(o) = object { a["object"] = o; }
    a
}

/// The KMS request inside `kmsMessage` (before wrapping): one resource for both users, bound to the new key.
pub(super) fn kms_resource(me: &str, other: &str, key_uri: &str) -> Value {
    json!({"method": "create", "uri": "/resources", "userIds": [me, other], "keyUris": [key_uri]})
}

/// The body of POST conversation `/conversations` for a new one-to-one space.
pub(super) fn direct_body(me: &str, other: &str, key_uri: &str, kms_message: &str) -> Value {
    let person = |id: &str| json!({"objectType": "person", "id": id});
    json!({
        "activities": {"items": [expand(me, "create", None), expand(me, "add", Some(person(me))), expand(me, "add", Some(person(other)))]},
        "objectType": "conversation",
        "kmsMessage": kms_message,
        "tags": ["ONE_ON_ONE"],
        "encryptionKeyUrl": key_uri,
        "defaultActivityEncryptionKeyUrl": key_uri,
    })
}

/// GET path (conversation service) of the existing one-to-one space with `uuid`.
pub(super) fn lookup_path(uuid: &str) -> String {
    format!("/conversations/user/{uuid}?uuidEntryFormat=true&personRefresh=true&activitiesLimit=0&includeConvWithDeletedUserUUID=true&includeParticipants=true")
}

/// POST path + body that turn an e-mail into a uuid (`create` = invite: the service creates the user).
pub(super) fn users_request(email: &str, create: bool) -> (String, Value) {
    (if create { "/users?shouldCreateUsers=true".into() } else { "/users".into() }, json!([{"email": email}]))
}

/// The uuid in the `/users` answer (keyed by the e-mail; the service may change its case).
pub(super) fn uuid_from_users(resp: &Value, email: &str) -> Option<String> {
    let m = resp.as_object()?;
    m.iter().find(|(k, _)| k.eq_ignore_ascii_case(email)).map(|(_, v)| st(v, "id")).filter(|s| !s.is_empty())
}

/// `true` for the "no such one-to-one space" answer of the lookup (our `http()` errors read "METHOD url  404 …").
pub(super) fn is_not_found(e: &str) -> bool { e.contains("  404") }

impl Session {
    /// A person (uuid, the composer's `Person.id` or an e-mail) → uuid; `create` invites unknown e-mail addresses.
    fn direct_uuid(&self, who: &str, create: bool) -> Result<String, String> {
        if who.len() == 36 && who.matches('-').count() == 4 { return Ok(who.to_string()); }
        if let Some(u) = self.uuids.get(who) { return Ok(u.clone()); }
        if !who.contains('@') { return Err("unknown person".into()); }
        let (path, body) = users_request(who, create);
        let v = self.http("POST", &self.url("conversation", &path), Some(body))?;
        uuid_from_users(&v, who).ok_or_else(|| "Diese Person konnte nicht gefunden werden.".to_string())
    }

    /// The one-to-one space with `who`: the existing one, else a new one, then `text` in it.
    /// Returns the space id. Only ever called for the user's own click.
    pub(super) fn direct_with(&mut self, who: &str, text: &str, create_user: bool) -> Result<String, String> {
        let uuid = self.direct_uuid(who, create_user)?;
        let id = match self.http("GET", &self.url("conversation", &lookup_path(&uuid)), None) {
            Ok(v) => st(&v, "id"),
            Err(e) if is_not_found(&e) => self.create_direct(&uuid)?,
            Err(e) => return Err(e),
        };
        self.spaces()?; // the key of the (new) space comes with the list
        if !text.trim().is_empty() { self.send(&id, text)?; }
        Ok(id)
    }

    fn create_direct(&mut self, other: &str) -> Result<String, String> {
        let v = self.kms_call(json!({"method": "create", "uri": "/keys", "count": 1}))?;
        let k = &v["keys"][0];
        let (uri, key) = (st(k, "uri"), jose::oct_key(&k["jwk"])?);
        self.keys.insert(uri.clone(), key);
        let (kid, eph) = self.kms.as_ref().and_then(|k| k.eph.clone()).ok_or("no key-service session")?;
        let mut resource = kms_resource(&self.user, other, &uri);
        resource["client"] = self.client();
        resource["requestId"] = json!(jose::uuid());
        let kms = jose::encrypt_dir(&eph, &kid, resource.to_string().as_bytes())?;
        let body = direct_body(&self.user, other, &uri, &kms);
        let v = self.http("POST", &self.url("conversation", "/conversations"), Some(body))?;
        Ok(st(&v, "id"))
    }
}

// Groups from "Neue Nachricht" (several recipients): the same activities / kmsMessage as the 1:1, every participant
// added, no tag, plus `displayName` only when the user typed a name (encrypted with the new key).

/// The body of POST conversation `/conversations` for a new group space with `others` (uuids) and an optional encrypted title.
pub(super) fn group_body(me: &str, others: &[String], key_uri: &str, kms_message: &str, title_jwe: Option<&str>) -> Value {
    let person = |id: &str| json!({"objectType": "person", "id": id});
    let mut items = vec![expand(me, "create", None), expand(me, "add", Some(person(me)))];
    items.extend(others.iter().map(|o| expand(me, "add", Some(person(o)))));
    let mut b = json!({"activities": {"items": items}, "objectType": "conversation", "kmsMessage": kms_message,
        "encryptionKeyUrl": key_uri, "defaultActivityEncryptionKeyUrl": key_uri});
    if let Some(t) = title_jwe { b["displayName"] = json!(t); }
    b
}

/// KMS resource for a group: me first, then everyone added.
pub(super) fn group_kms_resource(me: &str, others: &[String], key_uri: &str) -> Value {
    let mut ids = vec![me.to_string()];
    ids.extend(others.iter().cloned());
    json!({"method": "create", "uri": "/resources", "userIds": ids, "keyUris": [key_uri]})
}

// People search in the directory:
// - service `directorySearch` from the u2c catalog; one POST per keystroke of a search;
// - path "search/orgid/" + orgId + "/entities", body keys queryString, resultSize, requestId,
//   requestedTypes ("PERSON" with people, "ROBOT" with bots);
// - the HTTP answer is empty; the results arrive as a Mercury event `directory.search` whose data holds
//   `requestId`, `sequence`, `finished` and `directoryEntities`; each entity is read like an identity user:
//   `id`, `displayName` / `name.givenName` + `name.familyName`, `emails[{value, primary}]`, `entityProviderType`.
// - orgId: the identity user's `meta.organizationID`, read from SCIM Users/me.
// The event payload is read from `data` and from `data.data`.

/// Request id -> query of the directory searches in flight (Mercury answers carry only the id).
static DIR_REQ: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

pub(super) const DIR_RESULT_SIZE: u32 = 20;
pub(super) fn directory_path(org: &str) -> String { format!("/search/orgid/{org}/entities") }
pub(super) fn directory_body(q: &str, request_id: &str) -> Value {
    json!({"queryString": q, "resultSize": DIR_RESULT_SIZE, "requestId": request_id, "requestedTypes": ["PERSON", "ROBOT"]})
}

/// One directory entity as a person (None without an id).
pub(super) fn directory_person(e: &Value) -> Option<Person> {
    let id = st(e, "id");
    if id.is_empty() { return None; }
    let emails = e["emails"].as_array().cloned().unwrap_or_default();
    let email = emails.iter().find(|m| m["primary"].as_bool() == Some(true)).or(emails.first()).map(|m| st(m, "value")).unwrap_or_default();
    let name = [st(e, "displayName"), format!("{} {}", st(&e["name"], "givenName"), st(&e["name"], "familyName")).trim().to_string()]
        .into_iter().find(|n| !n.is_empty()).unwrap_or_else(|| email.clone());
    Some(Person { id, name, email, status: String::new() })
}

/// A `directory.search` event's data -> (request id, finished, people).
pub(super) fn directory_event(data: &Value) -> (String, bool, Vec<Person>) {
    let d = if data["directoryEntities"].is_array() || data["requestId"].is_string() { data } else { &data["data"] };
    let people = d["directoryEntities"].as_array().into_iter().flatten().filter_map(directory_person).collect();
    (st(d, "requestId"), d["finished"].as_bool().unwrap_or(true), people)
}

impl Session {
    /// Starts a directory search (read-only); the answer comes over Mercury (`Job::Directory`).
    pub(super) fn directory_search(&mut self, q: &str) -> Result<(), String> {
        let org = self.svc.get(ORG_KEY).cloned().ok_or("no organisation id")?;
        let rid = jose::uuid();
        { let mut r = DIR_REQ.lock().unwrap(); r.push((rid.clone(), q.to_string())); let n = r.len(); if n > 32 { r.drain(..n - 32); } }
        self.http("POST", &self.url("directorySearch", &directory_path(&org)), Some(directory_body(q, &rid))).map(|_| ())
    }

    /// A `directory.search` Mercury event: hands the people to the UI with the query they belong to.
    pub(super) fn directory_event(&mut self, data: &Value) {
        let (rid, finished, people) = directory_event(data);
        let q = DIR_REQ.lock().unwrap().iter().find(|(r, _)| *r == rid).map(|(_, q)| q.clone());
        if let Some(q) = q {
            for p in &people { self.uuids.insert(p.email.clone(), p.id.clone()); }
            // The people found get their presence now and stay subscribed (cloud/presence_sub.rs).
            let statuses = self.subscribe_new(&people, None);
            let mut people = people;
            for p in people.iter_mut() { if let Some((_, s, _)) = statuses.iter().find(|(id, _, _)| *id == p.id) { p.status = s.clone(); } }
            let _ = self.out.send(Resp::Directory(q, people, finished));
        }
    }

    /// A group space with `who` (uuids / Person.ids / e-mails) and an optional name, then `text` in it. User click only.
    pub(super) fn group_with(&mut self, title: Option<&str>, who: &[String], text: &str) -> Result<String, String> {
        let others: Vec<String> = who.iter().map(|w| self.direct_uuid(w, false)).collect::<Result<_, _>>()?;
        let v = self.kms_call(json!({"method": "create", "uri": "/keys", "count": 1}))?;
        let k = &v["keys"][0];
        let (uri, key) = (st(k, "uri"), jose::oct_key(&k["jwk"])?);
        self.keys.insert(uri.clone(), key.clone());
        let (kid, eph) = self.kms.as_ref().and_then(|k| k.eph.clone()).ok_or("no key-service session")?;
        let mut resource = group_kms_resource(&self.user, &others, &uri);
        resource["client"] = self.client();
        resource["requestId"] = json!(jose::uuid());
        let kms = jose::encrypt_dir(&eph, &kid, resource.to_string().as_bytes())?;
        let title_jwe = match title.map(str::trim).filter(|t| !t.is_empty()) { Some(t) => Some(jose::encrypt_dir(&key, &uri, t.as_bytes())?), None => None };
        let v = self.http("POST", &self.url("conversation", "/conversations"), Some(group_body(&self.user, &others, &uri, &kms, title_jwe.as_deref())))?;
        let id = st(&v, "id");
        self.spaces()?;
        if !text.trim().is_empty() { self.send(&id, text)?; }
        Ok(id)
    }
}

/// Where the organisation id is kept (in the service map, next to the catalog urls).
pub(super) const ORG_KEY: &str = "@orgId";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_body_is_the_sdk_grouped_payload() {
        let b = group_body("me", &["a".into(), "b".into()], "kms://k", "JWE", None);
        assert!(b.get("tags").is_none() && b.get("displayName").is_none() && b.get("participants").is_none());
        let items = b["activities"]["items"].as_array().unwrap();
        assert_eq!(items.iter().map(|a| a["verb"].as_str().unwrap()).collect::<Vec<_>>(), ["create", "add", "add", "add"]);
        assert_eq!(items[3]["object"]["id"], "b");
        assert_eq!(group_body("me", &["a".into()], "k", "m", Some("T-JWE"))["displayName"], "T-JWE");
        assert_eq!(group_kms_resource("me", &["a".into(), "b".into()], "k")["userIds"], json!(["me", "a", "b"]));
    }

    #[test]
    fn directory_request_and_event() {
        assert_eq!(directory_path("org-1"), "/search/orgid/org-1/entities");
        let b = directory_body("andr", "r-1");
        assert_eq!(b["queryString"], "andr");
        assert_eq!(b["requestId"], "r-1");
        assert_eq!(b["requestedTypes"], json!(["PERSON", "ROBOT"]));
        assert!(b.get("queryTypes").is_none());
        let ev = json!({"eventType": "directory.search", "requestId": "r-1", "sequence": 0, "finished": true, "directoryEntities": [
            {"id": "u-1", "displayName": "Jonas Weber", "entityProviderType": "CI_USER", "emails": [{"value": "x@y.example"}, {"value": "a@y.example", "primary": true}]},
            {"id": "u-2", "name": {"givenName": "Ann", "familyName": "Bell"}, "emails": [{"value": "ann@y.example"}]},
            {"displayName": "no id"}]});
        let (rid, fin, people) = directory_event(&ev);
        assert_eq!((rid.as_str(), fin, people.len()), ("r-1", true, 2));
        assert_eq!(people[0].email, "a@y.example");
        assert_eq!(people[1].name, "Ann Bell");
        let (rid, _, people) = directory_event(&json!({"data": {"requestId": "r-2", "directoryEntities": [{"id": "u"}]}}));
        assert_eq!((rid.as_str(), people.len()), ("r-2", 1));
    }

    #[test]
    fn direct_body_is_the_sdk_one_on_one_payload() {
        let b = direct_body("me-uuid", "other-uuid", "kms://k/1", "JWE");
        assert_eq!(b["tags"], json!(["ONE_ON_ONE"]));
        assert_eq!(b["objectType"], "conversation");
        assert!(b.get("participants").is_none());
        assert!(b.get("displayName").is_none());
        let items = b["activities"]["items"].as_array().unwrap();
        assert_eq!(items.iter().map(|a| a["verb"].as_str().unwrap()).collect::<Vec<_>>(), ["create", "add", "add"]);
        assert!(items.iter().all(|a| a["objectType"] == "activity" && a["actor"] == json!({"objectType": "person", "id": "me-uuid"})));
        assert!(items[0].get("object").is_none());
        assert_eq!(items[1]["object"], json!({"objectType": "person", "id": "me-uuid"}));
        assert_eq!(items[2]["object"], json!({"objectType": "person", "id": "other-uuid"}));
        assert_eq!(b["encryptionKeyUrl"], "kms://k/1");
        assert_eq!(b["defaultActivityEncryptionKeyUrl"], "kms://k/1");
        assert_eq!(b["kmsMessage"], "JWE");
    }

    #[test]
    fn kms_resource_binds_both_users_to_the_key() {
        assert_eq!(kms_resource("a", "b", "kms://k/1"), json!({"method": "create", "uri": "/resources", "userIds": ["a", "b"], "keyUris": ["kms://k/1"]}));
    }

    #[test]
    fn lookup_and_users_requests() {
        assert_eq!(lookup_path("u-1"), "/conversations/user/u-1?uuidEntryFormat=true&personRefresh=true&activitiesLimit=0&includeConvWithDeletedUserUUID=true&includeParticipants=true");
        assert_eq!(users_request("a@b.example", false), ("/users".to_string(), json!([{"email": "a@b.example"}])));
        assert_eq!(users_request("a@b.example", true).0, "/users?shouldCreateUsers=true");
        assert_eq!(uuid_from_users(&json!({"A@b.example": {"id": "u-9"}}), "a@b.example").as_deref(), Some("u-9"));
        assert_eq!(uuid_from_users(&json!({"a@b.example": {"errorCode": 1}}), "a@b.example"), None);
        assert!(is_not_found("GET https://x/conversations/user/u  404 {}"));
        assert!(!is_not_found("GET https://x/conversations/user/u  500 {}"));
    }
}
