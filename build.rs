//! Embeds every icon in assets/icons (Momentum, MIT) and assets/cisco/icons (Webex client icons),
//! every emoji in assets/emoji (Twemoji), all other files of assets/cisco/
//! (docs/cisco-assets.md) and the optional secrets.toml, as sorted tables.
use std::{env, fs, path::{Path, PathBuf}};

/// Brand logos that the general name rules below would miss.
const BRAND_MARKS: &[&str] = &["facebook", "webex_16", "webex_log_24", "webexicon_80"];

fn manifest_dir() -> PathBuf { PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()) }

/// One sorted table of every `.svg` in `dirs` (the first folder wins on equal names). A folder is only watched when it
/// exists: a missing path would rerun this script on every build.
fn table(dirs: &[&str], var: &str, out: &mut String) {
    let mut found: Vec<(String, PathBuf)> = vec![];
    for dir in dirs {
        let root = manifest_dir().join(dir);
        if !root.exists() { continue; }
        println!("cargo:rerun-if-changed={dir}");
        for e in fs::read_dir(&root).into_iter().flatten().flatten() {
            let Some(n) = e.file_name().to_str().and_then(|n| n.strip_suffix(".svg")).map(String::from) else { continue };
            if !is_brand_mark(&n) && !found.iter().any(|f| f.0 == n) { found.push((n, e.path())); }
        }
    }
    found.sort();
    out.push_str(&format!("pub static {var}: &[(&str, &[u8])] = &[\n"));
    for (n, p) in found {
        out.push_str(&format!("    ({n:?}, include_bytes!({:?})),\n", p.display().to_string()));
    }
    out.push_str("];\n");
}

/// Every file under `assets/cisco/` except its icons (those join `ICON_TABLE`), keyed by the path
/// relative to that folder with `/` separators. Empty when the folder is missing: the app then falls back.
/// `assets/` itself is watched (it always exists), so creating or deleting `assets/cisco/` reruns this script.
fn cisco(out: &mut String) {
    let root = manifest_dir().join("assets/cisco");
    let mut files: Vec<(String, PathBuf)> = vec![];
    println!("cargo:rerun-if-changed=assets");
    if root.exists() {
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
                if rel == "icons" || rel.starts_with("icons/") { continue; }
                if p.is_dir() { stack.push(p); } else { files.push((rel, p)); }
            }
        }
    }
    files.sort();
    out.push_str("pub static CISCO: &[(&str, &[u8])] = &[\n");
    for (rel, p) in files {
        out.push_str(&format!("    ({rel:?}, include_bytes!({:?})),\n", p.display().to_string()));
    }
    out.push_str("];\n");
}

/// Logos and wordmarks (Webex, Cisco, third parties) are never bundled, even if a copy lands in the folder.
fn is_brand_mark(n: &str) -> bool {
    let l = n.to_lowercase();
    BRAND_MARKS.contains(&n) || ["logo", "wordmark", "lockup", "webexicon", "webex_", "cisco_", "facebook", "google", "microsoft", "onedrive", "googleplay", "fedramp", "box_", "sparklogo", "sparkvdi"]
        .iter().any(|b| l.contains(b))
}

/// The keys of `secrets.toml` and the environment variables that override them at build time.
const SECRETS: &[(&str, &str, &str)] = &[
    ("webex_client_id", "WEBOX_CLIENT_ID", "CLIENT_ID"),
    ("webex_client_secret", "WEBOX_CLIENT_SECRET", "CLIENT_SECRET"),
    ("giphy_api_key", "WEBOX_GIPHY_API_KEY", "GIPHY_API_KEY"),
];

/// Reads `key = "value"` lines from `secrets.toml` at the repo root (if present) and writes
/// `Option<&str>` constants; a non-empty build-time env var wins over the file. Nothing is required: a build
/// without secrets runs and shows the "not configured" states.
fn secrets(out: &mut String) {
    let file = manifest_dir().join("secrets.toml");
    // Only watched when it exists: a missing path would rerun this script on every build. So after creating
    // or deleting the file, run `cargo clean -p webox` once (edits to an existing file are picked up).
    // Always watched: registered only when present, a checkout built before secrets.toml existed never picked it up after a
    // pull (sign-in stayed "not configured" until `cargo clean`).
    println!("cargo:rerun-if-changed=secrets.toml");
    let text = fs::read_to_string(&file).unwrap_or_default();
    for (key, var, name) in SECRETS {
        println!("cargo:rerun-if-env-changed={var}");
        let from_file = text.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| k.trim() == *key)
            .map(|(_, v)| v.trim().trim_matches('"').to_string());
        let value = env::var(var).ok().filter(|v| !v.trim().is_empty()).or(from_file).filter(|v| !v.trim().is_empty());
        out.push_str(&format!("pub const {name}: Option<&str> = {value:?};\n"));
    }
}

fn main() {
    let mut out = String::new();
    table(&["assets/icons", "assets/cisco/icons"], "ICON_TABLE", &mut out);
    table(&["assets/emoji"], "EMOJI_TABLE", &mut out);
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("asset_tables.rs"), out).unwrap();
    let mut cis = String::new();
    cisco(&mut cis);
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("cisco_assets.rs"), cis).unwrap();
    let mut sec = String::new();
    secrets(&mut sec);
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("secrets.rs"), sec).unwrap();
}
