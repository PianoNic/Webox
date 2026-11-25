//! Plugin repositories (Aniyomi-style extension repos, docs/plugins.md): the official repository plus ones the person adds, each an
//! index URL. Indexes are fetched when the Plugins page opens, on "Refresh", and once a day when "Automatically check for
//! updates" is on; downloads run on worker threads and report progress. Installing itself (moving the verified file into place and
//! loading it) happens on the UI thread through `Plugins::install`. The list of repositories, the auto-check switch and whether the
//! first-install warning was confirmed are kept in `plugin-repos.json` in the config folder; indexes are cached in `plugin-repos/`.
pub mod index;
pub mod net;
pub mod sample;
pub mod store;
#[cfg(test)]
pub(crate) mod tests;

use index::{FileRef, IndexPlugin, RepoIndex};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// The official repository. Can be changed in plugin-repos.json (`official`) or with `WEBOX_PLUGIN_REPO`.
pub const OFFICIAL_URL: &str = "https://raw.githubusercontent.com/PianoNic/WeboxPlugins/repo/index.min.json";
const DAY: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub official: String,
    pub user: Vec<String>,
    pub auto_update: bool,
    /// Unix seconds of the last automatic check.
    pub last_check: u64,
    /// The "native code" warning was confirmed once (shown before the first install at all).
    pub warned: bool,
}
impl Default for Config {
    fn default() -> Self { Config { official: OFFICIAL_URL.into(), user: vec![], auto_update: true, last_check: 0, warned: false } }
}
impl Config {
    pub fn load(p: &Path) -> Config {
        let v: Value = std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let d = Config::default();
        let official = std::env::var("WEBOX_PLUGIN_REPO").ok().or_else(|| v["official"].as_str().map(String::from)).filter(|s| !s.is_empty());
        Config {
            official: official.unwrap_or(d.official),
            user: v["repos"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default(),
            auto_update: v["autoUpdate"].as_bool().unwrap_or(d.auto_update),
            last_check: v["lastCheck"].as_u64().unwrap_or(0),
            warned: v["warned"].as_bool().unwrap_or(false),
        }
    }
    pub fn save(&self, p: &Path) -> Result<(), String> {
        let v = json!({ "official": self.official, "repos": self.user, "autoUpdate": self.auto_update, "lastCheck": self.last_check, "warned": self.warned });
        store::write_atomic(p, serde_json::to_string_pretty(&v).unwrap_or_default().as_bytes())
    }
}

#[derive(Clone, Debug, Default)]
pub struct RepoState { pub url: String, pub index: Option<RepoIndex>, pub error: Option<String>, pub loading: bool }
impl RepoState {
    /// The index's name, else the URL's host and path.
    pub fn title(&self) -> String {
        self.index.as_ref().and_then(|i| i.name.clone()).unwrap_or_else(|| self.url.trim_start_matches("https://").to_string())
    }
}

/// One plugin offered by one repository.
#[derive(Clone, Debug)]
pub struct Avail { pub repo: String, pub repo_title: String, pub official: bool, pub plugin: IndexPlugin, pub compat: Result<FileRef, String> }

/// A running download.
pub struct Job { pub pkg: String, pub version: String, pub done: Arc<AtomicU64>, pub total: Arc<AtomicU64> }
impl Job { pub fn fraction(&self) -> Option<f32> { let t = self.total.load(Ordering::Relaxed); (t > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / t as f32) } }

enum Msg { Index(String, Result<RepoIndex, String>), Download(String, Result<(store::Installed, PathBuf), String>) }

/// What the window has to act on after `tick`.
pub enum Event { Downloaded(store::Installed, PathBuf), Failed(String, String), UpdatesFound(usize) }

pub struct Repos {
    pub cfg: Config, pub states: Vec<RepoState>, pub jobs: Vec<Job>,
    cfg_path: PathBuf, cache_dir: PathBuf, root: PathBuf, policy: net::Policy,
    tx: Sender<Msg>, rx: Receiver<Msg>,
    /// --sample: offline demo indexes, no network, no installs.
    pub sample: bool,
    last_open: f64, auto_running: bool,
}

pub fn now_secs() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) }

impl Repos {
    pub fn new(config_dir: &Path, install_root: PathBuf, sample: bool) -> Repos {
        let cfg_path = config_dir.join("plugin-repos.json");
        let (tx, rx) = channel();
        let mut r = Repos { cfg: Config::load(&cfg_path), states: vec![], jobs: vec![], cfg_path, cache_dir: config_dir.join("plugin-repos"),
            root: install_root, policy: net::Policy::strict(), tx, rx, sample, last_open: f64::NEG_INFINITY, auto_running: false };
        r.sync_states();
        if sample { sample::fill(&mut r); } else { r.load_cached(); }
        r
    }
    #[cfg(test)]
    pub fn with_policy(mut self, p: net::Policy) -> Repos { self.policy = p; self }

    pub fn urls(&self) -> Vec<String> { std::iter::once(self.cfg.official.clone()).chain(self.cfg.user.iter().cloned()).collect() }
    pub fn is_official(&self, url: &str) -> bool { url == self.cfg.official }
    fn save(&self) { if !self.sample { let _ = self.cfg.save(&self.cfg_path); } }

    fn sync_states(&mut self) {
        let urls = self.urls();
        self.states.retain(|s| urls.contains(&s.url));
        for u in urls { if !self.states.iter().any(|s| s.url == u) { self.states.push(RepoState { url: u, ..Default::default() }); } }
        let order = self.urls();
        self.states.sort_by_key(|s| order.iter().position(|u| *u == s.url));
    }
    /// Shows the cached copy of every index at once (before the network answers).
    fn load_cached(&mut self) {
        for s in &mut self.states {
            if let Some(c) = net::read_cache(&self.cache_dir, &s.url) { s.index = index::parse(&c.body).ok(); }
        }
    }

    /// The normalised URL of a repository the person wants to add, or why it can not be added (HTTPS only, an index .json
    /// file, not yet in the list).
    pub fn check_repo_url(&self, url: &str) -> Result<String, String> {
        let p = net::parse_url(url, self.policy)?;
        if !p.path().to_ascii_lowercase().ends_with(".json") { return Err(crate::trq!("The address must point to the index file (ending in .json, e.g. index.min.json)").into_owned()); }
        let u = p.to_string();
        if self.urls().contains(&u) { return Err(crate::trq!("This repository is already in the list").into_owned()); }
        Ok(u)
    }

    pub fn add_repo(&mut self, url: &str) -> Result<(), String> {
        let u = self.check_repo_url(url)?;
        self.cfg.user.push(u.clone());
        self.save();
        self.sync_states();
        self.refresh_one(&u);
        Ok(())
    }
    pub fn remove_repo(&mut self, url: &str) {
        self.cfg.user.retain(|u| u != url);
        self.save();
        self.sync_states();
    }
    pub fn set_auto_update(&mut self, on: bool) { self.cfg.auto_update = on; self.save(); }
    pub fn confirm_warning(&mut self) { self.cfg.warned = true; self.save(); }

    pub fn refresh(&mut self) { for u in self.urls() { self.refresh_one(&u); } }
    pub fn refresh_one(&mut self, url: &str) {
        if self.sample { return; }
        // Tests stay offline: only the in-process fixture on 127.0.0.1 is ever contacted.
        #[cfg(test)]
        if !url.starts_with("http://127.0.0.1") { return; }
        let Some(s) = self.states.iter_mut().find(|s| s.url == url) else { return };
        if s.loading { return; }
        s.loading = true;
        let (tx, url, dir, p) = (self.tx.clone(), url.to_string(), self.cache_dir.clone(), self.policy);
        std::thread::spawn(move || {
            let r = net::fetch_index(&url, &dir, p).and_then(|t| index::parse(&t));
            let _ = tx.send(Msg::Index(url, r));
        });
    }
    /// Called when the Plugins page is drawn: refreshes when it was not shown a moment ago (= it was just opened).
    pub fn page_shown(&mut self, now: f64) {
        if now - self.last_open > 2.0 { self.refresh(); }
        self.last_open = now;
    }
    pub fn busy(&self) -> bool { !self.jobs.is_empty() || self.states.iter().any(|s| s.loading) }

    /// Every plugin of every repository, the official one first.
    pub fn available(&self) -> Vec<Avail> {
        let v = env!("CARGO_PKG_VERSION");
        self.states.iter().filter_map(|s| s.index.as_ref().map(|i| (s, i))).flat_map(|(s, i)| i.plugins.iter().map(move |p| Avail {
            repo: s.url.clone(), repo_title: s.title(), official: s.url == self.cfg.official, plugin: p.clone(),
            compat: index::compatible(p, v).cloned(),
        })).collect()
    }
    /// The newest compatible version of `pkg` newer than `have`; the repository it was installed from wins a tie.
    pub fn update_for(&self, pkg: &str, have: &str, from: &str) -> Option<Avail> {
        self.available().into_iter().filter(|a| a.plugin.pkg == pkg && a.compat.is_ok() && index::newer(&a.plugin.version, have))
            .max_by(|a, b| semver::Version::parse(&a.plugin.version).ok().cmp(&semver::Version::parse(&b.plugin.version).ok()).then((a.repo == from).cmp(&(b.repo == from))))
    }
    pub fn job(&self, pkg: &str) -> Option<&Job> { self.jobs.iter().find(|j| j.pkg == pkg) }

    /// Starts downloading `a` (the caller has shown the warnings). The file is verified on the worker thread.
    pub fn start_install(&mut self, a: &Avail) -> Result<(), String> {
        if self.sample { return Err(crate::trq!("No plugins are installed in sample mode").into_owned()); }
        if crate::system::side_loading_protection() { return Err(super::plugins::policy_block_reason()); }
        let f = a.compat.clone()?;
        if self.job(&a.plugin.pkg).is_some() { return Err(crate::trq!("already downloading").into_owned()); }
        let url = net::resolve(&a.repo, &f.file, self.policy)?;
        let file = url.path_segments().and_then(|mut s| s.next_back()).unwrap_or_default().to_string();
        if !index::valid_file_name(&file) || !file.ends_with(&format!(".{}", std::env::consts::DLL_EXTENSION)) {
            return Err(crate::trq!("unexpected file name '{{0}}'", file));
        }
        let inst = store::Installed { pkg: a.plugin.pkg.clone(), name: a.plugin.name.clone(), version: a.plugin.version.clone(), file,
            sha256: f.sha256.clone(), repo: a.repo.clone() };
        let stage = store::stage_dir(&self.root, &inst.pkg)?;
        let job = Job { pkg: inst.pkg.clone(), version: inst.version.clone(), done: Default::default(), total: Arc::new(AtomicU64::new(f.size)) };
        let (done, total, tx, p) = (job.done.clone(), job.total.clone(), self.tx.clone(), self.policy);
        self.jobs.push(job);
        std::thread::spawn(move || {
            let dest = stage.join(&inst.file);
            let r = net::download(&url, &f, &dest, p, &done, &total).map(|_| (inst.clone(), dest));
            if r.is_err() { let _ = std::fs::remove_dir_all(&stage); }
            let _ = tx.send(Msg::Download(inst.pkg, r));
        });
        Ok(())
    }

    /// Collects finished work; starts the daily check when it is due. `installed`: (pkg, version, repo) of repository plugins.
    pub fn tick(&mut self, installed: &[(String, String, String)]) -> Vec<Event> {
        let mut ev = vec![];
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Index(url, r) => if let Some(s) = self.states.iter_mut().find(|s| s.url == url) {
                    s.loading = false;
                    match r { Ok(i) => { s.index = Some(i); s.error = None; } Err(e) => s.error = Some(e) }
                },
                Msg::Download(pkg, r) => {
                    self.jobs.retain(|j| j.pkg != pkg);
                    ev.push(match r { Ok((i, p)) => Event::Downloaded(i, p), Err(e) => Event::Failed(pkg, e) });
                }
            }
        }
        if self.auto_running && !self.states.iter().any(|s| s.loading) {
            self.auto_running = false;
            let n = installed.iter().filter(|(p, v, r)| self.update_for(p, v, r).is_some()).count();
            if n > 0 { ev.push(Event::UpdatesFound(n)); }
        }
        if !self.sample && self.cfg.auto_update && !installed.is_empty() && now_secs().saturating_sub(self.cfg.last_check) >= DAY {
            self.cfg.last_check = now_secs();
            self.save();
            self.auto_running = true;
            self.refresh();
        }
        ev
    }
}
