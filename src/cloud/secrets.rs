//! Client credentials that are not part of the source tree. `build.rs` embeds them from the git-ignored
//! `secrets.toml` at the repo root (or the same-named env vars at build time); the env vars below also
//! override them at run time. See `secrets.example.toml` and docs/dev-setup.md.
mod built {
    include!(concat!(env!("OUT_DIR"), "/secrets.rs"));
}

fn pick(var: &str, built: Option<&str>) -> Option<String> {
    std::env::var(var).ok().or_else(|| built.map(String::from)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// The OAuth client id the sign-in uses (`WEBOX_CLIENT_ID`).
pub fn client_id() -> Option<String> { pick("WEBOX_CLIENT_ID", built::CLIENT_ID) }
/// The OAuth client secret for the token endpoint's Basic auth (`WEBOX_CLIENT_SECRET`).
pub fn client_secret() -> Option<String> { pick("WEBOX_CLIENT_SECRET", built::CLIENT_SECRET) }
/// The Giphy API key for the GIF picker (`WEBOX_GIPHY_API_KEY`).
pub fn giphy_api_key() -> Option<String> { pick("WEBOX_GIPHY_API_KEY", built::GIPHY_API_KEY) }
/// Shown on the login screen when a build has no OAuth client.
pub const NOT_CONFIGURED: &str = "Sign-in is not configured in this build. Add secrets.toml (see docs/dev-setup.md) and rebuild.";
