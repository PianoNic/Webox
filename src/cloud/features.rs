//! Feature toggles and client policies of the signed-in account.
//!
//! * Toggles come with the device registration: `features.{developer, entitlement, user}[{key, val, type, mutable,
//!   lastModified}]`. `val` is a string read as a number, else `true` / `false`, else text.
//! * Live changes arrive over mercury as `featureToggle_update` with `data.featureToggle {key, val, type}`; only user and
//!   developer toggles are taken.
//! * Client policies (`ucf-giphy-rating`, `cover-image-selection`, …) sit elsewhere in the same device answer; they are
//!   looked up there.
//!
//! The session writes, the UI reads (`App::feature`), so the store is a process-wide snapshot.
use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureValue { Bool(bool), Num(f64), Str(String) }

impl FeatureValue {
    /// A toggle value: a number, else true/false (any case), else the text.
    pub fn parse(val: &Value) -> FeatureValue {
        let s = match val { Value::String(s) => s.clone(), Value::Bool(b) => return FeatureValue::Bool(*b), Value::Number(n) => return FeatureValue::Num(n.as_f64().unwrap_or(0.0)), Value::Null => String::new(), v => v.to_string() };
        if let Ok(n) = s.trim().parse::<f64>() { if !s.trim().is_empty() { return FeatureValue::Num(n) } }
        match s.to_ascii_lowercase().as_str() { "true" => FeatureValue::Bool(true), "false" => FeatureValue::Bool(false), _ => FeatureValue::Str(s) }
    }
    pub fn as_bool(&self) -> Option<bool> { match self { FeatureValue::Bool(b) => Some(*b), FeatureValue::Num(n) => Some(*n != 0.0), FeatureValue::Str(_) => None } }
    pub fn as_str(&self) -> String { match self { FeatureValue::Bool(b) => b.to_string(), FeatureValue::Num(n) => n.to_string(), FeatureValue::Str(s) => s.clone() } }
}

#[derive(Default)]
pub struct Features {
    /// keyType ("developer" / "entitlement" / "user") → key → value.
    pub toggles: HashMap<&'static str, HashMap<String, FeatureValue>>,
    /// The device answer, for its policies.
    pub device: Value,
}

static STORE: RwLock<Option<Features>> = RwLock::new(None);

const TYPES: [&str; 3] = ["developer", "entitlement", "user"];

impl Features {
    pub fn from_device(dev: &Value) -> Features {
        let mut f = Features { device: dev.clone(), ..Default::default() };
        for t in TYPES {
            let m = f.toggles.entry(t).or_default();
            for x in dev["features"][t].as_array().into_iter().flatten() {
                if let Some(k) = x["key"].as_str() { m.insert(k.to_string(), FeatureValue::parse(&x["val"])); }
            }
        }
        f
    }
    /// A toggle by type (`keyType`, key).
    pub fn get_typed(&self, key_type: &str, key: &str) -> Option<&FeatureValue> { self.toggles.get(key_type).and_then(|m| m.get(key)) }
    /// A key regardless of its type (user over developer over entitlement; keys are unique in practice),
    /// else a client policy of the device answer.
    pub fn get(&self, key: &str) -> Option<FeatureValue> {
        ["user", "developer", "entitlement"].iter().find_map(|t| self.get_typed(t, key).cloned())
            .or_else(|| super::find_policy(&self.device, key).map(|v| FeatureValue::parse(&Value::String(v))))
    }
    /// `featureToggle_update`: user / developer toggles only. Returns whether something changed.
    pub fn update(&mut self, toggle: &Value) -> bool {
        let t = toggle["type"].as_str().unwrap_or("").to_ascii_lowercase();
        let t: &'static str = match t.as_str() { "user" => "user", "developer" => "developer", _ => return false };
        let Some(k) = toggle["key"].as_str() else { return false };
        let v = FeatureValue::parse(&toggle["val"]);
        self.toggles.entry(t).or_default().insert(k.to_string(), v.clone()) != Some(v)
    }
}

/// Every toggle as (key, value), user over developer over entitlement (the data API's `me.features`).
pub fn all() -> Vec<(String, Value)> {
    let Ok(s) = STORE.read() else { return vec![] };
    let Some(f) = s.as_ref() else { return vec![] };
    let mut m: std::collections::BTreeMap<String, Value> = Default::default();
    for t in ["entitlement", "developer", "user"] {
        for (k, v) in f.toggles.get(t).into_iter().flatten() {
            m.insert(k.clone(), match v { FeatureValue::Bool(b) => Value::Bool(*b), FeatureValue::Num(n) => serde_json::json!(n), FeatureValue::Str(s) => Value::String(s.clone()) });
        }
    }
    m.into_iter().collect()
}

/// The session's registration arrived: replace the snapshot.
pub fn store(f: Features) { if let Ok(mut s) = STORE.write() { *s = Some(f); } }
/// A live toggle change (mercury `featureToggle_update`, `data.featureToggle`).
pub fn live_update(toggle: &Value) -> bool { STORE.write().ok().and_then(|mut s| s.as_mut().map(|f| f.update(toggle))).unwrap_or(false) }
/// Look a key up in the current snapshot (`None` before sign-in or when the account doesn't have it).
pub fn feature(key: &str) -> Option<FeatureValue> { STORE.read().ok().and_then(|s| s.as_ref().and_then(|f| f.get(key))) }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    /// Shape of a real answer (keys only: deletedTime, key, lastModified, mutable, type, val).
    #[test]
    fn toggles_and_policies_like_the_sdk() {
        let dev = json!({"features": {
            "developer": [{"key": "desktop-giphy-with-policy", "val": "true", "type": "DEV", "mutable": true, "lastModified": "x"},
                          {"key": "n", "val": "42", "type": "DEV"}, {"key": "s", "val": "Manual", "type": "DEV"}],
            "entitlement": [{"key": "squared-call-initiation", "val": "true", "type": "ENTITLEMENT"}],
            "user": [{"key": "u", "val": "FALSE", "type": "USER"}]},
            "policies": [{"name": "ucf-giphy-rating", "value": "G"}]});
        let mut f = Features::from_device(&dev);
        assert_eq!(f.get("desktop-giphy-with-policy"), Some(FeatureValue::Bool(true)));
        assert_eq!(f.get("n"), Some(FeatureValue::Num(42.0)));
        assert_eq!(f.get("s"), Some(FeatureValue::Str("Manual".into())));
        assert_eq!(f.get("u"), Some(FeatureValue::Bool(false)));
        assert_eq!(f.get_typed("entitlement", "squared-call-initiation"), Some(&FeatureValue::Bool(true)));
        assert_eq!(f.get("ucf-giphy-rating").map(|v| v.as_str()), Some("G".into()), "a policy of the device answer");
        assert_eq!(f.get("ucf-disable-shrunken-heads"), None);
        // Live: user / developer only.
        assert!(f.update(&json!({"key": "u", "val": "true", "type": "USER"})));
        assert_eq!(f.get("u"), Some(FeatureValue::Bool(true)));
        assert!(!f.update(&json!({"key": "u", "val": "true", "type": "USER"})), "no change");
        assert!(!f.update(&json!({"key": "squared-call-initiation", "val": "false", "type": "ENTITLEMENT"})));
        assert_eq!(f.get("squared-call-initiation"), Some(FeatureValue::Bool(true)));
        assert!(f.update(&json!({"key": "new-dev", "val": "x", "type": "DEVELOPER"})));
    }
}
