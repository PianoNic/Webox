//! The small slice of JOSE (JWE/JWS/JWK) the Webex key service and conversations use.
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::engine::general_purpose::STANDARD as B64_STD;
use base64::Engine;
use rand::RngCore;
use rsa::{BigUint, Oaep, RsaPublicKey};
use serde_json::{json, Value};

pub fn b64(b: &[u8]) -> String { B64.encode(b) }
/// Standard base64 (with padding) — HTTP Basic credentials.
pub fn b64_std(b: &[u8]) -> String { B64_STD.encode(b) }
pub fn unb64(s: &str) -> Result<Vec<u8>, String> { B64.decode(s.trim_end_matches('=')).map_err(|e| e.to_string()) }

fn gcm(key: &[u8], header_b64: &str, plain: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    let mut iv = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut iv);
    let c = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let mut ct = c.encrypt(Nonce::from_slice(&iv), Payload { msg: plain, aad: header_b64.as_bytes() }).map_err(|e| e.to_string())?;
    let tag = ct.split_off(ct.len() - 16);
    Ok((iv.to_vec(), ct, tag))
}

/// Compact JWE with alg "dir", enc "A256GCM" (message bodies, space titles, KMS requests).
pub fn encrypt_dir(key: &[u8], kid: &str, plain: &[u8]) -> Result<String, String> {
    let h = b64(json!({"alg": "dir", "enc": "A256GCM", "kid": kid}).to_string().as_bytes());
    let (iv, ct, tag) = gcm(key, &h, plain)?;
    Ok(format!("{h}..{}.{}.{}", b64(&iv), b64(&ct), b64(&tag)))
}

/// Like [`encrypt_dir`] without a `kid` in the header: how received file `scr`s look (`{"alg":"dir","enc":"A256GCM"}` only).
pub fn encrypt_dir_nokid(key: &[u8], plain: &[u8]) -> Result<String, String> {
    let h = b64(json!({"alg": "dir", "enc": "A256GCM"}).to_string().as_bytes());
    let (iv, ct, tag) = gcm(key, &h, plain)?;
    Ok(format!("{h}..{}.{}.{}", b64(&iv), b64(&ct), b64(&tag)))
}

/// Compact JWE with alg "RSA-OAEP", enc "A256GCM" (the first message to the key service).
pub fn encrypt_rsa(public: &RsaPublicKey, kid: &str, plain: &[u8]) -> Result<String, String> {
    // The recipient key's kid goes in the header; the key service picks its key by it.
    let mut hd = json!({"alg": "RSA-OAEP", "enc": "A256GCM"});
    if !kid.is_empty() { hd["kid"] = json!(kid); }
    let h = b64(hd.to_string().as_bytes());
    let mut cek = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut cek);
    let ek = public.encrypt(&mut rand::thread_rng(), Oaep::new::<sha1::Sha1>(), &cek).map_err(|e| e.to_string())?;
    let (iv, ct, tag) = gcm(&cek, &h, plain)?;
    Ok(format!("{h}.{}.{}.{}.{}", b64(&ek), b64(&iv), b64(&ct), b64(&tag)))
}

pub fn decrypt_dir(key: &[u8], jwe: &str) -> Result<Vec<u8>, String> {
    let p: Vec<&str> = jwe.split('.').collect();
    if p.len() != 5 { return Err("not a JWE".into()); }
    let c = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    // remote data: `Nonce::from_slice` panics unless the IV is exactly 12 bytes
    let (iv, tag) = (unb64(p[2])?, unb64(p[4])?);
    if iv.len() != 12 { return Err("bad iv length".into()); }
    if tag.len() != 16 { return Err("bad tag length".into()); }
    let mut ct = unb64(p[3])?;
    ct.extend(tag);
    c.decrypt(Nonce::from_slice(&iv), Payload { msg: &ct, aad: p[0].as_bytes() }).map_err(|_| "decryption failed".into())
}

/// AES-256-GCM open for file content (SCR): `tag` separate, or appended to `ct` when empty.
pub fn gcm_open(key: &[u8], iv: &[u8], aad: &[u8], ct: &[u8], tag: &[u8]) -> Result<Vec<u8>, String> {
    if iv.len() != 12 { return Err("bad iv length".into()); }
    let c = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let mut buf = ct.to_vec();
    buf.extend_from_slice(tag);
    c.decrypt(Nonce::from_slice(iv), Payload { msg: &buf, aad }).map_err(|_| "content decryption failed".to_string())
}

/// The `hmac` the conversation service requires on a posted reaction:
/// HMAC-SHA256, keyed with the parent activity id, over `JSON({k,kid,kty}) + parentId + displayName` where the JWK
/// is the parent's content key (in exactly that key order), hex-encoded in lower case.
pub fn reaction_hmac(key: &[u8], key_uri: &str, parent_id: &str, name: &str) -> String {
    use hmac::{Hmac, Mac};
    let source = format!(r#"{{"k":"{}","kid":"{}","kty":"oct"}}{parent_id}{name}"#, b64(key), key_uri);
    let mut m = <Hmac<sha2::Sha256> as Mac>::new_from_slice(parent_id.as_bytes()).expect("hmac accepts any key length");
    m.update(source.as_bytes());
    m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn header(token: &str) -> Value {
    token.split('.').next().and_then(|h| unb64(h).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)
}

/// Verifies an RS256 JWS from the key service and returns its payload.
pub fn verify_rs256(public: &RsaPublicKey, jws: &str) -> Result<Vec<u8>, String> {
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::signature::Verifier;
    let p: Vec<&str> = jws.split('.').collect();
    if p.len() != 3 { return Err("not a JWS".into()); }
    let sig = Signature::try_from(unb64(p[2])?.as_slice()).map_err(|e| e.to_string())?;
    VerifyingKey::<sha2::Sha256>::new(public.clone())
        .verify(format!("{}.{}", p[0], p[1]).as_bytes(), &sig).map_err(|_| "key service signature is wrong".to_string())?;
    unb64(p[1])
}

pub fn rsa_from_jwk(jwk: &Value) -> Result<RsaPublicKey, String> {
    let n = unb64(jwk["n"].as_str().ok_or("jwk without n")?)?;
    let e = unb64(jwk["e"].as_str().ok_or("jwk without e")?)?;
    RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e)).map_err(|e| e.to_string())
}

/// A fresh P-256 key for the ECDHE handshake, with its public half as a JWK.
pub fn ec_keypair() -> (p256::SecretKey, Value) {
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    let sk = p256::SecretKey::random(&mut rand::thread_rng());
    let pt = sk.public_key().to_encoded_point(false);
    (sk, json!({"kty": "EC", "crv": "P-256", "x": b64(pt.x().unwrap()), "y": b64(pt.y().unwrap())}))
}

/// Shared 256-bit key from our secret and the key service's public JWK (ECDH, then HKDF-SHA256).
pub fn ecdh_hkdf(sk: &p256::SecretKey, server_jwk: &Value) -> Result<[u8; 32], String> {
    let mut sec1 = vec![4u8];
    sec1.extend(unb64(server_jwk["x"].as_str().ok_or("jwk without x")?)?);
    sec1.extend(unb64(server_jwk["y"].as_str().ok_or("jwk without y")?)?);
    let pk = p256::PublicKey::from_sec1_bytes(&sec1).map_err(|e| e.to_string())?;
    let z = p256::ecdh::diffie_hellman(sk.to_nonzero_scalar(), pk.as_affine());
    let mut out = [0u8; 32];
    hkdf::Hkdf::<sha2::Sha256>::new(None, z.raw_secret_bytes().as_slice()).expand(&[], &mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn oct_key(jwk: &Value) -> Result<Vec<u8>, String> { unb64(jwk["k"].as_str().ok_or("key without k")?) }

pub fn uuid() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

#[cfg(test)]
mod tests {
    /// Expected value computed independently (python hmac/hashlib) from the same formula.
    #[test]
    fn reaction_hmac_vector() {
        let key: Vec<u8> = (0u8..32).collect();
        assert_eq!(
            super::reaction_hmac(&key, "kms://kms-a.wbx2.com/keys/11111111-2222-3333-4444-555555555555", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee", "heart"),
            "0eff39aa34110549b60d9455cafc6c44f897843720f5137c5c311cf594a5433f");
    }

    #[test]
    fn scr_jwe_without_kid_round_trips() {
        let k = [9u8; 32];
        let t = super::encrypt_dir_nokid(&k, b"{\"loc\":\"x\"}").unwrap();
        assert_eq!(super::header(&t), serde_json::json!({"alg": "dir", "enc": "A256GCM"})); // as received file scrs
        assert_eq!(super::decrypt_dir(&k, &t).unwrap(), b"{\"loc\":\"x\"}");
    }

    #[test]
    fn dir_round_trip() {
        let k = [7u8; 32];
        let t = super::encrypt_dir(&k, "kms://x/keys/1", b"hello").unwrap();
        assert_eq!(super::decrypt_dir(&k, &t).unwrap(), b"hello");
        assert_eq!(super::header(&t)["kid"], "kms://x/keys/1");
    }

    /// A crafted message / space title with a short IV or tag is an error, not a panic of the cloud thread.
    #[test]
    fn bad_iv_or_tag_length_is_an_error() {
        let k = [7u8; 32];
        let t = super::encrypt_dir(&k, "kms://x/keys/1", b"hello").unwrap();
        let p: Vec<&str> = t.split('.').collect();
        let with = |i: usize, v: &str| p.iter().enumerate().map(|(j, s)| if j == i { v.to_string() } else { s.to_string() }).collect::<Vec<_>>().join(".");
        for iv in [String::new(), "AAAA".into(), super::b64(&[0u8; 11]), super::b64(&[0u8; 13])] { assert!(super::decrypt_dir(&k, &with(2, &iv)).is_err(), "iv {iv:?}"); }
        for tag in [String::new(), super::b64(&[0u8; 15])] { assert!(super::decrypt_dir(&k, &with(4, &tag)).is_err(), "tag {tag:?}"); }
        assert!(super::decrypt_dir(&k, &format!("{}....", p[0])).is_err());
        assert!(super::gcm_open(&k, &[0u8; 5], b"", b"x", b"").is_err());
    }
}
