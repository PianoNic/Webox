//! What can be done to a space as a whole: favourite (verbs "tag"/"untag" with FAVORITE), hide,
//! rename (verb "update" with a new encrypted title), add a person and leave. Adding and leaving
//! also tell the key service who may read the space (a kmsMessage on the space's KMS resource).
use super::*;

pub const FAVORITE_TAG: &str = "FAVORITE";

impl Session {
    /// An encrypted request for the key service, carried inside a conversation activity.
    fn kms_message(&self, mut req: Value) -> Result<String, String> {
        let (kid, key) = self.kms.as_ref().and_then(|k| k.eph.clone()).ok_or("no key-service session")?;
        req["client"] = self.client();
        req["requestId"] = json!(jose::uuid());
        jose::encrypt_dir(&key, &kid, req.to_string().as_bytes())
    }

    fn resource(&self, id: &str) -> Result<String, String> {
        self.spaces.get(id).map(|s| s.kms_resource.clone()).filter(|r| !r.is_empty()).ok_or_else(|| "this space has no key-service resource".to_string())
    }

    /// Favourite / unfavourite: the simple verbs `favorite` / `unfavorite` with the conversation as object.
    pub(super) fn favorite(&mut self, id: &str, on: bool) -> Result<(), String> {
        let _ = FAVORITE_TAG;
        self.submit(id, super::spaces_ext::simple_body(if on { "favorite" } else { "unfavorite" }, id))
    }

    pub(super) fn hide(&mut self, id: &str) -> Result<(), String> {
        self.submit(id, json!({"verb": "hide", "object": {"id": id, "objectType": "conversation"}}))
    }

    pub(super) fn rename(&mut self, id: &str, title: &str) -> Result<(), String> {
        let key_url = self.key_url(id)?;
        let key = self.key(&key_url)?;
        let jwe = jose::encrypt_dir(&key, &key_url, title.as_bytes())?;
        self.submit(id, json!({"verb": "update", "encryptionKeyUrl": key_url,
            "object": {"objectType": "conversation", "displayName": jwe}}))
    }

    pub(super) fn add_person(&mut self, id: &str, person: &str) -> Result<(), String> {
        // The person goes in as a uuid; participant ids may be e-mail style.
        let uuid = self.uuids.get(person).cloned().unwrap_or_else(|| person.to_string());
        let person = uuid.as_str();
        let resource = self.resource(id)?;
        let kms = self.kms_message(json!({"method": "create", "uri": format!("{resource}/authorizations"), "userIds": [person]}))?;
        self.submit(id, json!({"verb": "add", "object": {"objectType": "person", "id": person}, "kmsMessage": kms}))
    }

    pub(super) fn leave(&mut self, id: &str) -> Result<(), String> {
        let resource = self.resource(id)?;
        let kms = self.kms_message(json!({"method": "delete", "uri": format!("{resource}/authorizations?authId={}", self.user)}))?;
        let me = self.user.clone();
        self.submit(id, json!({"verb": "leave", "object": {"objectType": "person", "id": me}, "kmsMessage": kms}))
    }
}
