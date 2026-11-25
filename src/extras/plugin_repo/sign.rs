//! Signed repository indexes. A repository publishes `<index>.minisig` next to its index file: a minisign signature
//! (Ed25519 over the BLAKE2b-512 hash of the index file's exact bytes, the format `minisign -S` writes). The index lists
//! every plugin binary's SHA-256 and size, and every download is checked against those, so a valid index signature covers
//! the binaries too.
//!
//! Webox has one public key built in (`OFFICIAL_PUBLIC_KEY`, the official repository's). An index whose signature is
//! missing or does not verify with it is "unsigned" / "badly signed": its plugins show a warning and can only be installed
//! when the person allows unsigned repositories (`Config::allow_unsigned`). Third-party repositories are unsigned in this
//! sense unless they are signed with the same key.
//!
//! While no key is built in (`enforced()` is false) nothing is checked or blocked ("warn only"): each repository just says
//! its signature was not checked.

/// The official repository's minisign public key: the second line of `minisign.pub` (base64, starts with `RW`).
///
/// TODO(owner): create the signing key and put its public half here. Until then verification runs in "warn only" mode.
///   1. On a trusted machine: `minisign -G -p webox-plugins.pub -s webox-plugins.key` (choose a password), or with the Rust
///      tool: `cargo install rsign2 && rsign generate -p webox-plugins.pub -s webox-plugins.key`.
///   2. In github.com/PianoNic/WeboxPlugins > Settings > Secrets and variables > Actions, add the secret
///      `MINISIGN_SECRET_KEY` (the whole content of webox-plugins.key) and `MINISIGN_PASSWORD` (its password). The publish
///      job then writes `index.min.json.minisig` and `index.json.minisig` to the `repo` branch (see its xtask `sign`).
///   3. Paste the second line of webox-plugins.pub below, e.g. `pub const OFFICIAL_PUBLIC_KEY: &str = "RWQ...";`, and release.
///      Publish once with the key before releasing a Webox that has it, or the official repository reads as unsigned.
///   Keep webox-plugins.key offline and backed up; never commit it. A new key needs a new Webox release.
pub const OFFICIAL_PUBLIC_KEY: &str = "";

/// The signature file's URL for an index URL (`.../index.min.json` -> `.../index.min.json.minisig`).
pub fn sig_url(index_url: &str) -> String { format!("{index_url}.minisig") }

/// Largest signature file read (a minisign signature is about 300 bytes).
pub const SIG_MAX: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Trust {
    /// No key built in: nothing checked ("warn only").
    #[default]
    NotChecked,
    /// Signed with the built-in key, and the signature matches the index.
    Signed,
    /// The repository publishes no signature.
    Unsigned,
    /// A signature is there but does not verify (another key, or the index was changed after signing).
    BadSignature,
}

impl Trust {
    /// Whether this repository's plugins may be installed (`allow_unsigned`: the person allowed unsigned repositories).
    pub fn installable(&self, key: &str, allow_unsigned: bool) -> bool { key.is_empty() || allow_unsigned || *self == Trust::Signed }
}

/// Whether signatures are checked at all (a key is built in).
pub fn enforced(key: &str) -> bool { !key.is_empty() }

/// Checks `sig` (the `.minisig` file's text, `None` when the repository has none) for exactly `index` with `key`.
pub fn verify(key: &str, index: &[u8], sig: Option<&str>) -> Trust {
    if key.is_empty() { return Trust::NotChecked; }
    let Some(sig) = sig else { return Trust::Unsigned };
    let ok = (|| -> Result<(), minisign_verify::Error> {
        let pk = minisign_verify::PublicKey::from_base64(key.trim())?;
        // only prehashed (BLAKE2b) signatures, the format minisign writes since 0.8
        pk.verify(index, &minisign_verify::Signature::decode(sig)?, false)
    })();
    if ok.is_ok() { Trust::Signed } else { Trust::BadSignature }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A throwaway test key pair (its secret half was never stored) and a signature made with it over `INDEX`.
    pub(crate) const TEST_KEY: &str = "RWSBByuXP1fbjAzhUxtgr7mEcGbr9XiG5rwCMGWRlFtkvZmGJn414bCn";
    const OTHER_KEY: &str = "RWRwCRV/cVZQ8On3HuySrRubHuRfAiMZOrVVnmaL8fJINi9KWl5Os7XM";
    pub(crate) const INDEX: &str = r#"{"name":"Signed","plugins":[]}"#;
    pub(crate) const INDEX_SIG: &str = "untrusted comment: signature from minisign secret key\nRUSBByuXP1fbjKXbjg2jkZez6+Jwa+h0ePbg9xrlccyOhyXwJzOTumgfstKt0iWVUEQxz6g7NLx8JApOVgQX0LfKdic9PrUdswo=\ntrusted comment: timestamp:1764000000\tfile:index.min.json\nUQ/XDy5VrvKS9OMjADMcjJoxnOfGIExnTWpH8vdXyvFw9ujCnA/vlnt8wOh3UK3myCCIaNJWKSLxI8eXInOxBQ==\n";

    #[test]
    fn a_signed_index_verifies_only_unchanged_and_with_its_key() {
        assert_eq!(verify(TEST_KEY, INDEX.as_bytes(), Some(INDEX_SIG)), Trust::Signed);
        assert_eq!(verify(TEST_KEY, br#"{"name":"Signed","plugins":[1]}"#, Some(INDEX_SIG)), Trust::BadSignature, "changed after signing");
        assert_eq!(verify(OTHER_KEY, INDEX.as_bytes(), Some(INDEX_SIG)), Trust::BadSignature, "another key");
        assert_eq!(verify(TEST_KEY, INDEX.as_bytes(), Some(&INDEX_SIG.replace("timestamp:1764000000", "timestamp:1"))), Trust::BadSignature, "trusted comment changed");
        assert_eq!(verify(TEST_KEY, INDEX.as_bytes(), Some("not a signature")), Trust::BadSignature);
        assert_eq!(verify(TEST_KEY, INDEX.as_bytes(), None), Trust::Unsigned);
        assert_eq!(verify("", INDEX.as_bytes(), None), Trust::NotChecked, "no key built in: warn only");
    }

    #[test]
    fn only_signed_repositories_install_once_a_key_is_built_in() {
        for t in [Trust::Unsigned, Trust::BadSignature, Trust::NotChecked] {
            assert!(!t.installable(TEST_KEY, false), "{t:?}");
            assert!(t.installable(TEST_KEY, true), "{t:?} allowed by the person");
            assert!(t.installable("", false), "{t:?} warn only");
        }
        assert!(Trust::Signed.installable(TEST_KEY, false));
        assert!(enforced(TEST_KEY) && !enforced(""));
    }

    #[test]
    fn signature_sits_next_to_the_index() {
        assert_eq!(sig_url("https://raw.githubusercontent.com/PianoNic/WeboxPlugins/repo/index.min.json"),
            "https://raw.githubusercontent.com/PianoNic/WeboxPlugins/repo/index.min.json.minisig");
        // the built-in key, once set, is a minisign public key
        if !OFFICIAL_PUBLIC_KEY.is_empty() { assert!(minisign_verify::PublicKey::from_base64(OFFICIAL_PUBLIC_KEY).is_ok()); }
    }
}
