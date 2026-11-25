//! Offline demo repositories for `--sample` (screenshots): made-up plugins, no network, nothing installable.
use super::{index, Repos};

const FAKE_SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const SAMPLE_USER_REPO: &str = "https://example.com/webox-plugins/index.min.json";

fn entry(name: &str, pkg: &str, version: &str, contract: &str, desc: &str, author: &str, platform: &str) -> String {
    format!(r#"{{"name":"{name}","pkg":"{pkg}","version":"{version}","contract":"{contract}","description":"{desc}","authors":["{author}"],
        "files":{{"{platform}":{{"file":"{pkg}-{version}.bin","sha256":"{FAKE_SHA}","size":180224}}}}}}"#)
}

pub fn fill(r: &mut Repos) {
    let here = index::platform();
    let official = format!(r#"{{"name":"Webox Plugins","description":"Official plugins","plugins":[{},{},{},{}]}}"#,
        entry("Slash Commands", "dev.webox.plugins.slash-commands", "1.4.0", "1.4", "Adds /shrug, /tableflip and /count, and turns :) into a smiley.", "Webox examples", &here),
        entry("Pride Theme", "dev.webox.plugins.pride-theme", "1.4.0", "1.4", "Rainbow accents across Webox, in the colours of the Pride flag.", "Webox examples", &here),
        entry("Focus Timer", "dev.webox.plugins.focus-timer", "0.3.0", "1.1", "Sets your status to Do not disturb for a pomodoro and back.", "Webox", &here),
        entry("Markdown Tables", "dev.webox.plugins.md-tables", "2.0.0", "2.0", "Renders pipe tables in messages.", "Webox", &here));
    let user = format!(r#"[{},{}]"#,
        entry("Quick Replies", "com.example.quick-replies", "1.2.0", "1.0", "Saved replies on /qr.", "Alex Example", &here),
        entry("Pixel Theme", "com.example.pixel-theme", "0.9.1", "1.1", "A retro palette.", "Alex Example", "macos-aarch64"));
    r.cfg.user = vec![SAMPLE_USER_REPO.into()];
    r.sync_states();
    for s in &mut r.states {
        let body = if s.url == SAMPLE_USER_REPO { &user } else { &official };
        s.index = index::parse(body).ok();
    }
}
