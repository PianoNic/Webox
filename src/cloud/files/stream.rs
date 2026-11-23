//! AES-256-GCM decrypted chunk by chunk, so a download goes into its file as it arrives instead of sitting in memory whole.
//! Same result as `jose::gcm_open`, including its rule that an empty SCR tag means the tag is the stream's last 16 bytes.
//! The plaintext must not be used before `finish` has checked the tag: the caller writes it to a `.part` file and renames it
//! only after that.
use aes::cipher::{BlockEncrypt, KeyInit};
use ctr::cipher::{KeyIvInit, StreamCipher};
use ghash::universal_hash::UniversalHash;

pub(crate) struct GcmStream {
    ctr: ctr::Ctr32BE<aes::Aes256>,
    ghash: ghash::GHash,
    tag_mask: [u8; 16],
    /// Ciphertext not yet hashed (less than one block).
    partial: Vec<u8>,
    /// The tag travels at the end of the stream: the last 16 bytes seen are held back.
    tail: Option<Vec<u8>>,
    aad_len: u64,
    ct_len: u64,
}

impl GcmStream {
    pub fn new(key: &[u8], iv: &[u8], aad: &[u8], tag_in_stream: bool) -> Result<Self, String> {
        if key.len() != 32 { return Err("bad key length".into()); }
        if iv.len() != 12 { return Err("bad iv length".into()); }
        let aes = aes::Aes256::new_from_slice(key).map_err(|e| e.to_string())?;
        let mut h = aes::Block::default();
        aes.encrypt_block(&mut h);
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(iv);
        j0[15] = 1;
        let mut mask = aes::Block::from(j0);
        aes.encrypt_block(&mut mask);
        let mut ctr0 = j0;
        ctr0[15] = 2; // inc32(J0)
        let mut ghash = ghash::GHash::new(&h);
        ghash.update_padded(aad);
        Ok(GcmStream { ctr: ctr::Ctr32BE::<aes::Aes256>::new_from_slices(key, &ctr0).map_err(|e| e.to_string())?, ghash, tag_mask: mask.into(),
            partial: Vec::with_capacity(16), tail: tag_in_stream.then(Vec::new), aad_len: aad.len() as u64, ct_len: 0 })
    }

    /// Decrypts the next piece of ciphertext; the plaintext is appended to `out`.
    pub fn update(&mut self, data: &[u8], out: &mut Vec<u8>) {
        let data: std::borrow::Cow<[u8]> = match &mut self.tail {
            None => data.into(),
            Some(t) => {
                t.extend_from_slice(data);
                if t.len() <= 16 { return; }
                let keep = t.split_off(t.len() - 16);
                std::mem::replace(t, keep).into()
            }
        };
        self.ct_len += data.len() as u64;
        // hash whole blocks of ciphertext
        let mut rest: &[u8] = &data;
        if !self.partial.is_empty() {
            let need = (16 - self.partial.len()).min(rest.len());
            self.partial.extend_from_slice(&rest[..need]);
            rest = &rest[need..];
            if self.partial.len() == 16 { self.ghash.update(&[*ghash::Block::from_slice(&self.partial)]); self.partial.clear(); }
        }
        let whole = rest.len() / 16 * 16;
        let blocks: Vec<ghash::Block> = rest[..whole].chunks_exact(16).map(|c| *ghash::Block::from_slice(c)).collect();
        self.ghash.update(&blocks);
        self.partial.extend_from_slice(&rest[whole..]);
        let start = out.len();
        out.extend_from_slice(&data);
        self.ctr.apply_keystream(&mut out[start..]);
    }

    /// Checks the tag (`tag`, or the held-back last 16 bytes when it travelled in the stream).
    pub fn finish(mut self, tag: &[u8]) -> Result<(), String> {
        if !self.partial.is_empty() { let p = std::mem::take(&mut self.partial); self.ghash.update_padded(&p); }
        let mut lens = [0u8; 16];
        lens[..8].copy_from_slice(&(self.aad_len * 8).to_be_bytes());
        lens[8..].copy_from_slice(&(self.ct_len * 8).to_be_bytes());
        self.ghash.update(&[lens.into()]);
        let s = self.ghash.finalize();
        let want: Vec<u8> = match &self.tail { Some(t) => t.clone(), None => tag.to_vec() };
        if want.len() != 16 { return Err("content decryption failed".into()); }
        let diff = s.iter().zip(&self.tag_mask).zip(&want).fold(0u8, |d, ((s, m), w)| d | (s ^ m ^ w));
        if diff == 0 { Ok(()) } else { Err("content decryption failed".into()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::{Aead, Payload};

    fn seal(key: &[u8; 32], iv: &[u8; 12], aad: &[u8], msg: &[u8]) -> Vec<u8> {
        aes_gcm::Aes256Gcm::new_from_slice(key).unwrap().encrypt(aes_gcm::Nonce::from_slice(iv), Payload { msg, aad }).unwrap()
    }

    #[test]
    fn matches_aes_gcm_in_any_chunking() {
        let (key, iv) = ([7u8; 32], [3u8; 12]);
        let aad = b"2026-01-01T00:00:00.000Z";
        for len in [0usize, 1, 15, 16, 17, 100, 4096 + 5] {
            let msg: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
            let sealed = seal(&key, &iv, aad, &msg);
            let (ct, tag) = sealed.split_at(sealed.len() - 16);
            for chunk in [1usize, 7, 16, 33, 1000] {
                // tag kept in the SCR
                let mut s = GcmStream::new(&key, &iv, aad, false).unwrap();
                let mut out = vec![];
                for c in ct.chunks(chunk) { s.update(c, &mut out); }
                assert_eq!(out, msg, "len {len} chunk {chunk}");
                assert!(s.finish(tag).is_ok());
                // tag at the end of the stream
                let mut s = GcmStream::new(&key, &iv, aad, true).unwrap();
                let mut out = vec![];
                for c in sealed.chunks(chunk) { s.update(c, &mut out); }
                assert_eq!(out, msg);
                assert!(s.finish(&[]).is_ok());
            }
        }
    }

    #[test]
    fn a_wrong_tag_or_byte_fails() {
        let (key, iv) = ([1u8; 32], [2u8; 12]);
        let sealed = seal(&key, &iv, b"a", b"hello video");
        let (ct, tag) = sealed.split_at(sealed.len() - 16);
        let mut bad = tag.to_vec();
        bad[0] ^= 1;
        let mut s = GcmStream::new(&key, &iv, b"a", false).unwrap();
        s.update(ct, &mut vec![]);
        assert!(s.finish(&bad).is_err());
        let mut ct2 = ct.to_vec();
        ct2[3] ^= 0x80;
        let mut s = GcmStream::new(&key, &iv, b"a", false).unwrap();
        s.update(&ct2, &mut vec![]);
        assert!(s.finish(tag).is_err());
        let mut s = GcmStream::new(&key, &iv, b"b", false).unwrap(); // other aad
        s.update(ct, &mut vec![]);
        assert!(s.finish(tag).is_err());
    }
}
