//! The key service (KMS): ECDHE, key requests and decrypting message text.
use super::*;

impl Session {
    pub(super) fn client(&self) -> Value { json!({"clientId": self.device, "credential": {"userId": self.user, "bearer": self.kms_token}}) }

    pub(super) fn post_kms(&self, jwe: String) -> Result<(), String> {
        let k = self.kms.as_ref().ok_or("no key service")?;
        // The synchronous /kms/messages/sync endpoint is refused for this client ("not authorized to use the synchronous
        // message api"), so use the async endpoint; answers arrive over the socket (or inline).
        let r = self.http("POST", &self.url("encryption", "/kms/messages"), Some(json!({"destination": k.cluster, "kmsMessages": [jwe]})))?;
        dbg_conn(&format!("POST kms/messages -> {}", r.to_string().chars().take(200).collect::<String>()));
        for m in r["kmsMessages"].as_array().into_iter().flatten() {
            if let Some(s) = m.as_str() { let _ = self.kms_loop.send(s.to_string()); }
        }
        Ok(())
    }

    /// Waits for the key service's answer to `rid`, which arrives over the socket.
    pub(super) fn await_kms(&self, rid: &str, open: impl Fn(&str) -> Option<Vec<u8>>) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(KMS_TIMEOUT_SECS);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Ok(msg) = self.kms_rx.recv_timeout(left) else { break };
            let opened = open(&msg);
            if opened.is_none() { dbg_conn("kms message received but could not be opened/verified"); }
            if let Some(v) = opened.and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                if v["requestId"] == rid {
                    let code = v["status"].as_u64().unwrap_or(200);
                    return if code < 300 { Ok(v) } else { Err(format!("key service said {code}: {}", st(&v, "reason"))) };
                }
            }
        }
        Err("the key service didn't answer in time".into())
    }

    pub(super) fn ecdhe(&mut self) -> Result<(), String> {
        let k = self.kms.as_ref().ok_or("no key service")?;
        let (sk, jwk) = jose::ec_keypair();
        let rid = jose::uuid();
        // The uri is `${kmsCluster}/ecdhe`; the JWE header carries the server key's kid.
        let req = json!({"client": self.client(), "requestId": rid, "method": "create", "uri": format!("{}/ecdhe", k.cluster), "jwk": jwk});
        self.post_kms(jose::encrypt_rsa(&k.rsa, &k.kid, req.to_string().as_bytes())?)?;
        let rsa = k.rsa.clone();
        let v = self.await_kms(&rid, |m| jose::verify_rs256(&rsa, m).ok())?;
        let key = jose::ecdh_hkdf(&sk, &v["key"]["jwk"])?;
        self.kms.as_mut().unwrap().eph = Some((st(&v["key"], "uri"), key));
        Ok(())
    }

    pub(super) fn kms_call(&self, mut req: Value) -> Result<Value, String> {
        let (kid, key) = self.kms.as_ref().and_then(|k| k.eph.clone()).ok_or("no key-service session")?;
        let rid = jose::uuid();
        req["client"] = self.client();
        req["requestId"] = json!(rid);
        self.post_kms(jose::encrypt_dir(&key, &kid, req.to_string().as_bytes())?)?;
        self.await_kms(&rid, |m| jose::decrypt_dir(&key, m).ok())
    }

    pub(super) fn key(&mut self, uri: &str) -> Result<Vec<u8>, String> {
        if let Some(k) = self.keys.get(uri) { return Ok(k.clone()); }
        // An answer can be lost when the socket reconnects at that moment: ask again once.
        let v = match self.kms_call(json!({"method": "retrieve", "uri": uri})) {
            Err(e) if e.contains("didn't answer") => { dbg_conn("key retrieve timed out, retrying once"); self.kms_call(json!({"method": "retrieve", "uri": uri}))? }
            r => r?,
        };
        let k = jose::oct_key(&v["key"]["jwk"])?;
        self.keys.insert(uri.to_string(), k.clone());
        Ok(k)
    }

    pub(super) fn decrypt(&mut self, text: &str, key_url: &str) -> String {
        if text.split('.').count() != 5 { return text.to_string(); }
        if let Some(p) = self.hist.plain.get(text) { return p.clone(); } // already decrypted (a refold of a known space)
        let out = self.decrypt_uncached(text, key_url);
        if let Some(p) = out.as_ref().ok() { self.hist.plain.insert(text.to_string(), p.clone()); }
        out.unwrap_or_else(|_| "🔒 (couldn't decrypt)".into())
    }

    fn decrypt_uncached(&mut self, text: &str, key_url: &str) -> Result<String, ()> {
        let kid = jose::header(text)["kid"].as_str().map(String::from).filter(|k| !k.is_empty()).unwrap_or_else(|| key_url.to_string());
        let r = self.key(&kid).and_then(|k| jose::decrypt_dir(&k, text));
        if let Err(e) = &r {
            let host = |u: &str| u.strip_prefix("kms://").and_then(|r| r.split('/').next()).unwrap_or("?").to_string();
            dbg_conn(&format!("decrypt failed: {} (key host {}, our cluster {})", e.chars().take(160).collect::<String>(), host(&kid),
                self.kms.as_ref().map(|k| host(&k.cluster)).unwrap_or_default()));
        }
        r.map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|_| ())
    }
}
