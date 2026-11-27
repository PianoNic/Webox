//! Offline tests: index parsing, compatibility, transport rules, the repository list, and a full install / update / uninstall
//! against an in-process HTTP server on 127.0.0.1 (plain http is allowed there only through the test-only policy).
use super::*;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A built plugin library (`cargo build` puts it next to the test binary's folder).
pub(crate) fn built(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut c: Vec<PathBuf> = exe.ancestors().skip(1).take(3).map(|d| d.join(name)).collect();
    c.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug").join(name));
    c.into_iter().find(|p| p.is_file())
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("webox-repo-test-{tag}-{}-{}", std::process::id(), now_secs()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn sha(b: &[u8]) -> String { use sha2::Digest; net::hex(&sha2::Sha256::digest(b)) }
const LOOPBACK: net::Policy = net::Policy { allow_loopback_http: true };

type Route = (u16, Vec<(String, String)>, Vec<u8>);
/// Minimal HTTP/1.1 server: one request per connection, routes by path, answers 304 when If-None-Match matches the route's ETag.
struct Fixture { base: String, routes: Arc<Mutex<HashMap<String, Route>>>, hits: Arc<Mutex<Vec<(String, u16)>>> }
impl Fixture {
    fn start() -> Fixture {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let (routes, hits): (Arc<Mutex<HashMap<String, Route>>>, Arc<Mutex<Vec<(String, u16)>>>) = Default::default();
        let (r, h) = (routes.clone(), hits.clone());
        std::thread::spawn(move || for s in l.incoming().flatten() {
            let mut rd = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            if rd.read_line(&mut line).is_err() { continue; }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            let mut inm = None;
            loop {
                let mut hl = String::new();
                if rd.read_line(&mut hl).is_err() || hl.trim().is_empty() { break; }
                if let Some((k, v)) = hl.split_once(':') { if k.eq_ignore_ascii_case("if-none-match") { inm = Some(v.trim().to_string()); } }
            }
            let (mut st, hd, body) = r.lock().unwrap().get(&path).cloned().unwrap_or((404, vec![], vec![]));
            let etag = hd.iter().find(|(k, _)| k == "ETag").map(|x| x.1.clone());
            if st == 200 && etag.is_some() && etag == inm { st = 304; }
            h.lock().unwrap().push((path, st));
            let mut out = format!("HTTP/1.1 {st} X\r\nConnection: close\r\n");
            for (k, v) in &hd { out += &format!("{k}: {v}\r\n"); }
            let body: &[u8] = if st == 304 { &[] } else { &body };
            out += &format!("Content-Length: {}\r\n\r\n", body.len());
            let mut s = s;
            let _ = s.write_all(out.as_bytes()).and_then(|_| s.write_all(body));
        });
        Fixture { base, routes, hits }
    }
    fn put(&self, path: &str, st: u16, headers: &[(&str, &str)], body: impl Into<Vec<u8>>) {
        self.routes.lock().unwrap().insert(path.into(), (st, headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), body.into()));
    }
    fn url(&self, p: &str) -> String { format!("{}{p}", self.base) }
}

fn plugin_json(pkg: &str, version: &str, contract: &str, file: &str, bytes: &[u8]) -> String {
    format!(r#"{{"name":"Test {pkg}","pkg":"{pkg}","version":"{version}","contract":"{contract}","description":"d","authors":"Alex Example",
        "files":{{"{}":{{"file":"{file}","sha256":"{}","size":{}}}}}}}"#, index::platform(), sha(bytes), bytes.len())
}

fn wait(r: &mut Repos, installed: &[(String, String, String)], until: impl Fn(&Repos, &[Event]) -> bool) -> Vec<Event> {
    let t = Instant::now();
    let mut all = vec![];
    loop {
        all.extend(r.tick(installed));
        if until(r, &all) || t.elapsed() > Duration::from_secs(20) { return all; }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn parses_the_array_and_the_object_form() {
    let a = format!("[{},{{\"pkg\":\"bad\"}}]", plugin_json("dev.webox.a", "1.0.0", "1.1", "a.dll", b"x"));
    let i = index::parse(&a).unwrap();
    assert_eq!((i.plugins.len(), i.skipped, i.name.clone()), (1, 1, None));
    assert_eq!(i.plugins[0].authors, vec!["Alex Example".to_string()]);
    let o = format!(r#"{{"name":"Repo","description":"D","formatVersion":1,"plugins":[{}]}}"#, plugin_json("dev.webox.b", "2.0.0", "1.0", "b.so", b"y"));
    let i = index::parse(&o).unwrap();
    assert_eq!((i.name.as_deref(), i.plugins[0].version.as_str()), (Some("Repo"), "2.0.0"));
    assert!(index::parse("{\"name\":\"x\"}").is_err());
    assert!(index::parse("nope").is_err());
    let sub = plugin_json("dev.webox.d", "1.0.0", "1.0", "files/dev.webox.d/1.0.0/windows-x86_64/d.dll", b"z");
    assert_eq!(index::parse(&format!("[{sub}]")).unwrap().plugins.len(), 1);
    // Path traversal in a file name, a bad hash or a bad package id drop the entry.
    for bad in [plugin_json("dev.webox.c", "1.0.0", "1.0", "../c.dll", b"z"), plugin_json("dev.webox.c", "1.0.0", "1.0", "files/../c.dll", b"z"),
        plugin_json("dev.webox.c", "1.0.0", "1.0", "/c.dll", b"z"), plugin_json("Dev..x", "1.0.0", "1.0", "c.dll", b"z"),
        plugin_json("dev.webox.c", "1.0", "1.0", "c.dll", b"z"), plugin_json("dev.webox.c", "1.0.0", "x", "c.dll", b"z")] {
        assert_eq!(index::parse(&format!("[{bad}]")).unwrap().plugins.len(), 0, "{bad}");
    }
}

#[test]
fn compatibility() {
    use collabhost_plugin_api::{CONTRACT_MAJOR as MA, CONTRACT_MINOR as MI};
    assert!(index::check_contract(MA, MI).is_ok());
    assert!(index::check_contract(MA, 0).is_ok());
    assert!(index::check_contract(MA, MI + 1).is_err());
    assert!(index::check_contract(MA + 1, 0).is_err());
    let p = |c: &str| index::parse(&format!("[{}]", plugin_json("dev.webox.a", "1.0.0", c, "a.dll", b"x"))).unwrap().plugins.remove(0);
    assert!(index::compatible(&p(&format!("{MA}.{MI}")), "46.10.0").is_ok());
    assert!(index::compatible(&p(&format!("{}.0", MA + 1)), "46.10.0").unwrap_err().contains(&format!("{}.0", MA + 1)));
    let mut q = p("1.0");
    q.min_webox = Some("99.0.0".into());
    assert!(index::compatible(&q, "46.10.0").unwrap_err().contains("99.0.0"));
    q.min_webox = None;
    q.files = [("plan9-mips".to_string(), q.files.values().next().unwrap().clone())].into();
    assert!(index::compatible(&q, "46.10.0").unwrap_err().contains(&index::platform()));
    assert!(index::newer("1.10.0", "1.9.0") && !index::newer("1.0.0", "1.0.0") && !index::newer("x", "1.0.0"));
}

#[test]
fn transport_rules() {
    let s = net::Policy::strict();
    assert!(net::parse_url("http://example.com/i.json", s).is_err());
    assert!(net::parse_url("file:///C:/i.json", s).is_err());
    assert!(net::parse_url("https://user:pw@example.com/i.json", s).is_err());
    assert!(net::parse_url("http://127.0.0.1:1/i.json", s).is_err());
    assert!(net::parse_url("http://127.0.0.1:1/i.json", LOOPBACK).is_ok());
    let u = net::resolve("https://raw.githubusercontent.com/o/r/repo/index.min.json", "files/p.x/1.0.0/windows-x86_64/x.dll", s).unwrap();
    assert_eq!(u.as_str(), "https://raw.githubusercontent.com/o/r/repo/files/p.x/1.0.0/windows-x86_64/x.dll");
    assert!(net::resolve("https://example.com/i.json", "http://example.com/x.dll", s).is_err());
    assert!(net::redirect_ok("example.com", &url::Url::parse("https://objects.githubusercontent.com/x").unwrap()));
    assert!(net::redirect_ok("example.com", &url::Url::parse("https://example.com/y").unwrap()));
    assert!(!net::redirect_ok("example.com", &url::Url::parse("https://evil.example.net/y").unwrap()));
}

#[test]
fn repo_list_persists() {
    let d = tmp("cfg");
    let mut r = Repos::new(&d, d.join("root"), false);
    assert_eq!(r.urls(), vec![OFFICIAL_URL.to_string()]);
    assert!(r.add_repo("http://example.com/i.json").is_err());
    assert!(r.check_repo_url("https://example.com/plugins/").unwrap_err().contains(".json"));
    assert!(r.check_repo_url(OFFICIAL_URL).unwrap_err() == crate::trq!("This repository is already in the list"));
    assert_eq!(r.check_repo_url(" https://example.com/x/INDEX.JSON ").unwrap(), "https://example.com/x/INDEX.JSON");
    r.add_repo("https://example.com/a/index.json").unwrap();
    assert!(r.add_repo("https://example.com/a/index.json").is_err());
    r.add_repo("https://example.org/b/index.json").unwrap();
    r.set_auto_update(false);
    r.confirm_warning();
    let r2 = Repos::new(&d, d.join("root"), false);
    assert_eq!(r2.cfg.user, vec!["https://example.com/a/index.json".to_string(), "https://example.org/b/index.json".to_string()]);
    assert!(!r2.cfg.auto_update && r2.cfg.warned);
    assert_eq!(r2.states.len(), 3);
    let mut r3 = r2;
    r3.remove_repo("https://example.com/a/index.json");
    assert_eq!(Repos::new(&d, d.join("root"), false).cfg.user, vec!["https://example.org/b/index.json".to_string()]);
    assert!(Config::default().auto_update);
}

#[test]
fn index_cache_uses_etag() {
    let f = Fixture::start();
    let body = format!("[{}]", plugin_json("dev.webox.a", "1.0.0", "1.0", "a.dll", b"x"));
    f.put("/index.json", 200, &[("ETag", "\"v1\"")], body.clone());
    let d = tmp("etag");
    assert_eq!(net::fetch_index(&f.url("/index.json"), &d, LOOPBACK).unwrap(), body);
    assert_eq!(net::fetch_index(&f.url("/index.json"), &d, LOOPBACK).unwrap(), body);
    let hits: Vec<u16> = f.hits.lock().unwrap().iter().map(|h| h.1).collect();
    assert_eq!(hits, vec![200, 304]);
}

#[test]
fn refuses_sha_mismatch_size_and_foreign_redirects() {
    let f = Fixture::start();
    f.put("/p.dll", 200, &[], b"payload".to_vec());
    f.put("/away.dll", 302, &[("Location", "https://evil.example.net/p.dll")], vec![]);
    f.put("/hop.dll", 302, &[("Location", "/p.dll")], vec![]);
    let d = tmp("sha");
    let (a, b) = (AtomicU64::new(0), AtomicU64::new(0));
    let good = FileRef { file: "p.dll".into(), sha256: sha(b"payload"), size: 7 };
    let u = |p: &str| url::Url::parse(&f.url(p)).unwrap();
    let dest = d.join("p.dll");
    let bad = FileRef { sha256: sha(b"other"), ..good.clone() };
    assert!(net::download(&u("/p.dll"), &bad, &dest, LOOPBACK, &a, &b).unwrap_err().contains("SHA-256"));
    assert!(!dest.exists(), "an unverified file must not stay on disk");
    assert!(net::download(&u("/p.dll"), &FileRef { size: 3, ..good.clone() }, &dest, LOOPBACK, &a, &b).is_err());
    assert!(!dest.exists());
    assert!(net::download(&u("/away.dll"), &good, &dest, LOOPBACK, &a, &b).unwrap_err() == crate::trq!("Redirect to {{0}} refused", "evil.example.net"));
    net::download(&u("/hop.dll"), &good, &dest, LOOPBACK, &a, &b).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"payload");
}

#[test]
fn install_update_uninstall() {
    let name = format!("{}collabhost_slash_commands.{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_EXTENSION);
    let Some(lib) = built(&name) else { eprintln!("build the workspace first; skipping"); return };
    if crate::system::side_loading_protection() { eprintln!("side-loading policy on; skipping"); return; }
    let bytes = std::fs::read(lib).unwrap();
    let f = Fixture::start();
    let pkg = "dev.webox.plugins.slash-commands";
    let index = |v: &str| format!(r#"{{"name":"Fixture","plugins":[{}]}}"#, plugin_json(pkg, v, "1.1", &name, &bytes));
    f.put("/repo/index.json", 200, &[], index("1.0.0"));
    f.put(&format!("/repo/{name}"), 200, &[], bytes.clone());
    let d = tmp("install");
    let root = d.join("data");
    let mut plugins = crate::extras::plugins::Plugins::with_root(&d, root.clone());
    let mut r = Repos::new(&d, root.clone(), false).with_policy(LOOPBACK).with_key("");
    r.add_repo(&f.url("/repo/index.json")).unwrap();
    wait(&mut r, &[], |r, _| !r.states.iter().any(|s| s.loading));
    let a = r.available().into_iter().find(|a| a.plugin.pkg == pkg).expect("listed");
    assert!(!a.official);
    r.start_install(&a).unwrap();
    let ev = wait(&mut r, &[], |_, e| !e.is_empty());
    let Some(Event::Downloaded(inst, staged)) = ev.into_iter().next() else { panic!("download failed") };
    plugins.install(inst, &staged).unwrap();
    let e = plugins.list.iter().find(|e| e.source.as_ref().is_some_and(|s| s.pkg == pkg)).unwrap();
    assert!(e.enabled && e.error.is_none());
    assert_eq!(e.source.as_ref().unwrap().repo, f.url("/repo/index.json"));
    assert!(plugins.command("/shrug").is_some());
    assert!(root.join(pkg).join("1.0.0").join(&name).is_file());

    // Update: a newer version in the index replaces the old folder and is loaded at once.
    f.put("/repo/index.json", 200, &[], index("1.0.1"));
    r.refresh();
    let have = vec![(pkg.to_string(), "1.0.0".to_string(), f.url("/repo/index.json"))];
    wait(&mut r, &have, |r, _| !r.states.iter().any(|s| s.loading));
    let up = r.update_for(pkg, "1.0.0", &have[0].2).expect("update offered");
    r.start_install(&up).unwrap();
    let Some(Event::Downloaded(inst, staged)) = wait(&mut r, &have, |_, e| !e.is_empty()).into_iter().next() else { panic!() };
    plugins.install(inst, &staged).unwrap();
    assert!(!root.join(pkg).join("1.0.0").exists() && root.join(pkg).join("1.0.1").join(&name).is_file());
    assert_eq!(plugins.installed(pkg).unwrap().version, "1.0.1");
    assert_eq!(store::read(&root).len(), 1);

    // A file changed on disk is not loaded again.
    drop(plugins);
    let p = root.join(pkg).join("1.0.1").join(&name);
    let mut b = std::fs::read(&p).unwrap();
    b.push(0);
    std::fs::write(&p, b).unwrap();
    let mut plugins = crate::extras::plugins::Plugins::with_root(&d, root.clone());
    let e = plugins.list.iter().find(|e| e.source.is_some()).unwrap();
    assert!(e.error.as_deref().unwrap_or_default().contains("SHA-256"), "{:?}", e.error);

    plugins.uninstall(pkg).unwrap();
    assert!(!root.join(pkg).exists() && store::read(&root).is_empty() && plugins.installed(pkg).is_none());
}

/// End to end against a repository folder generated by the plugins monorepo (`cargo run -p xtask -- index ... --repo <dir>`):
/// `WEBOX_REPO_E2E=<dir> cargo test -- --ignored installs_from_a_generated_repo`. Serves the folder on 127.0.0.1 and installs,
/// runs and uninstalls every plugin built for this platform.
#[test]
#[ignore]
fn installs_from_a_generated_repo() {
    let Some(dir) = std::env::var_os("WEBOX_REPO_E2E").map(PathBuf::from) else { eprintln!("WEBOX_REPO_E2E not set; skipping"); return };
    let f = Fixture::start();
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p); continue; }
            let rel = p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/");
            f.put(&format!("/{rel}"), 200, &[], std::fs::read(&p).unwrap());
        }
    }
    let d = tmp("e2e");
    let root = d.join("data");
    let mut plugins = crate::extras::plugins::Plugins::with_root(&d, root.clone());
    let mut r = Repos::new(&d, root.clone(), false).with_policy(LOOPBACK).with_key("");
    r.add_repo(&f.url("/index.min.json")).unwrap();
    wait(&mut r, &[], |r, _| !r.states.iter().any(|s| s.loading));
    let st = r.states.iter().find(|s| !r.is_official(&s.url)).unwrap();
    assert!(st.error.is_none(), "{:?}", st.error);
    let list: Vec<Avail> = r.available().into_iter().filter(|a| a.compat.is_ok()).collect();
    assert!(!list.is_empty(), "nothing built for {}", index::platform());
    for a in &list {
        r.start_install(a).unwrap();
        let Some(Event::Downloaded(inst, staged)) = wait(&mut r, &[], |_, e| !e.is_empty()).into_iter().next() else { panic!("{} failed", a.plugin.pkg) };
        plugins.install(inst, &staged).unwrap();
        let e = plugins.list.iter().find(|e| e.source.as_ref().is_some_and(|s| s.pkg == a.plugin.pkg)).unwrap();
        assert!(e.error.is_none() && e.enabled, "{:?}", e.error);
        assert_eq!(e.version, a.plugin.version, "the library reports the index's version");
        eprintln!("installed {} {} ({})", a.plugin.pkg, a.plugin.version, e.name);
    }
    if plugins.installed("dev.webox.plugins.slash-commands").is_some() { assert!(plugins.command("/shrug x").is_some_and(|t| t.starts_with("x "))); }
    for a in &list { plugins.uninstall(&a.plugin.pkg).unwrap(); }
    assert!(store::read(&root).is_empty());
}

#[test]
fn staged_file_with_wrong_hash_is_not_installed() {
    let d = tmp("stage");
    let root = d.join("data");
    let mut plugins = crate::extras::plugins::Plugins::with_root(&d, root.clone());
    let stage = store::stage_dir(&root, "dev.webox.x").unwrap();
    let f = stage.join("x.dll");
    std::fs::write(&f, b"not what was pinned").unwrap();
    let inst = store::Installed { pkg: "dev.webox.x".into(), name: "X".into(), version: "1.0.0".into(), file: "x.dll".into(), sha256: sha(b"pinned"), repo: "https://example.com/i.json".into() };
    assert!(plugins.install(inst, &f).is_err());
    assert!(!stage.exists() && !root.join("dev.webox.x").join("1.0.0").exists());
}

#[test]
fn sample_repos_show_compatibility() {
    let d = tmp("sample");
    let r = Repos::new(&d, d.join("root"), true);
    let a = r.available();
    assert!(a.iter().any(|a| a.compat.is_ok()) && a.iter().any(|a| a.compat.is_err()));
    assert!(!d.join("plugin-repos.json").exists(), "sample mode writes nothing");
}

/// Index signatures (sign.rs) with a test key: a signed index reads as signed (also from the cache), a missing or wrong
/// signature as unsigned / badly signed, and plugins of those are only installed once unsigned repositories are allowed.
#[test]
fn unsigned_or_badly_signed_repositories_need_the_persons_permission() {
    use sign::tests::{INDEX, INDEX_SIG, TEST_KEY};
    let f = Fixture::start();
    f.put("/signed/index.json", 200, &[], INDEX);
    f.put("/signed/index.json.minisig", 200, &[], INDEX_SIG);
    f.put("/bad/index.json", 200, &[], INDEX.replace("Signed", "Changed"));
    f.put("/bad/index.json.minisig", 200, &[], INDEX_SIG);
    let bytes = b"plugin".to_vec();
    let file = format!("u.{}", std::env::consts::DLL_EXTENSION);
    f.put("/unsigned/index.json", 200, &[], format!("[{}]", plugin_json("dev.webox.u", "1.0.0", "1.0", &file, &bytes)));
    f.put(&format!("/unsigned/{file}"), 200, &[], bytes.clone());
    let d = tmp("sign");
    let mut r = Repos::new(&d, d.join("root"), false).with_policy(LOOPBACK).with_key(TEST_KEY);
    assert!(r.signatures_checked());
    for p in ["/signed/index.json", "/bad/index.json", "/unsigned/index.json"] { r.add_repo(&f.url(p)).unwrap(); }
    wait(&mut r, &[], |r, _| !r.states.iter().any(|s| s.loading));
    let trust = |r: &Repos, p: &str| r.states.iter().find(|s| s.url == f.url(p)).unwrap().trust.clone();
    assert_eq!(trust(&r, "/signed/index.json"), sign::Trust::Signed);
    assert_eq!(trust(&r, "/bad/index.json"), sign::Trust::BadSignature);
    assert_eq!(trust(&r, "/unsigned/index.json"), sign::Trust::Unsigned);
    let a = r.available().into_iter().find(|a| a.plugin.pkg == "dev.webox.u").unwrap();
    assert!(!a.installable && a.trust == sign::Trust::Unsigned);
    // refused even with a stale `Avail` that claims otherwise
    assert!(r.start_install(&Avail { installable: true, trust: sign::Trust::Signed, ..a.clone() }).is_err() && r.job("dev.webox.u").is_none());
    assert!(r.update_for("dev.webox.u", "0.9.0", &a.repo).is_none(), "no update offered from an unsigned repository");
    r.set_allow_unsigned(true);
    assert!(r.available().into_iter().find(|a| a.plugin.pkg == "dev.webox.u").unwrap().installable);
    assert!(r.update_for("dev.webox.u", "0.9.0", &a.repo).is_some());
    if !crate::system::side_loading_protection() {
        r.start_install(&a).unwrap(); // the same plugin downloads now
        assert!(matches!(wait(&mut r, &[], |_, e| !e.is_empty()).first(), Some(Event::Downloaded(..))));
    }
    // the signature is cached with the index: offline, the cached copy still reads as signed
    let r2 = Repos::new(&d, d.join("root"), false).with_policy(LOOPBACK).with_key(TEST_KEY);
    assert!(r2.cfg.allow_unsigned, "the permission is saved");
    assert_eq!(trust(&r2, "/signed/index.json"), sign::Trust::Signed);
    assert_eq!(trust(&r2, "/unsigned/index.json"), sign::Trust::Unsigned);
    // a signature that disappears from the server is forgotten
    f.put("/signed/index.json.minisig", 404, &[], vec![]);
    r.refresh();
    wait(&mut r, &[], |r, _| !r.states.iter().any(|s| s.loading));
    assert_eq!(trust(&r, "/signed/index.json"), sign::Trust::Unsigned);
    // no key built in: nothing is checked, everything installs (warn only)
    let r3 = Repos::new(&d, d.join("root"), false).with_policy(LOOPBACK).with_key("");
    assert!(!r3.signatures_checked() && r3.states.iter().all(|s| s.trust == sign::Trust::NotChecked));
}
