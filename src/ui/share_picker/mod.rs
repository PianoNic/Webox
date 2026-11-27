//! "Share content": picks the screen or window to share, with live thumbnails, the "optimise for" choice and "Share computer
//! audio". Its own borderless window, 1080 x 772 (at least 832 x 584), like the Webex share dialog: title bar, the grid of
//! sources ("Screen", then "Application"; cells 200 x 156) on the left, the 280 wide "Sharing options" panel on the right,
//! an 80 high bottom bar with "Share" and the button that hides / shows the options (`view.rs`).
//!
//! The call code opens it with `open()` and takes the user's pick with `take_choice()` (source + encoder settings); closing
//! the dialog leaves no choice. Thumbnails come from a worker thread once a second while the dialog is open.
//!
//! `WEBOX_SHOT_SHARE=1` (with `--screenshot`) draws the dialog inside the main window for a screenshot; `WEBOX_SAMPLE_SHARE=1`
//! opens it at start and runs a local share of the pick (capture + encode only, nothing is sent) with a small status card.
mod view;

use crate::media::share::{self, Kind, Optimize, Opts, ShareSource, ShareStream, Status};
use crate::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// What the user picked.
#[allow(dead_code)] // read by the call code
#[derive(Clone, Debug)]
pub(crate) struct Choice { pub source: ShareSource, pub optimize: Optimize, pub opts: Opts, pub audio: bool }

type Key = (Kind, isize);
type Pic = (u32, u32, Vec<u8>);

/// The worker's latest results.
#[derive(Default)]
struct Found { list: Vec<ShareSource>, pics: Vec<(Key, Option<Pic>)>, gen: u64 }

pub(crate) struct Picker {
    sources: Vec<ShareSource>,
    sel: Option<Key>,
    optimize: Optimize,
    audio: bool,
    options: bool,
    tex: HashMap<Key, egui::TextureHandle>,
    /// Sources whose thumbnail came back empty (minimised windows, the portal entries).
    no_pic: HashSet<Key>,
    seen: u64,
    found: Arc<Mutex<Found>>,
    run: Arc<AtomicBool>,
    scroll: f32,
}

static PICKER: Mutex<Option<Picker>> = Mutex::new(None);
static CHOICE: Mutex<Option<Choice>> = Mutex::new(None);
/// The local share started by `WEBOX_SAMPLE_SHARE` (status card in the main window).
static LOCAL: Mutex<Option<ShareStream>> = Mutex::new(None);

fn key(s: &ShareSource) -> Key { (s.kind, s.handle) }

/// Opens the dialog (again: brings the list up to date).
pub(crate) fn open() { open_with(false) }

/// `sample`: made-up screens and windows with test pictures (screenshots show nothing of this machine).
fn open_with(sample: bool) {
    let mut g = PICKER.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_some() { return; }
    let sources = if sample { sample_sources() } else { share::sources() };
    let sel = sources.iter().find(|s| s.primary).or(sources.first()).map(key);
    let (found, run) = (Arc::new(Mutex::new(Found::default())), Arc::new(AtomicBool::new(true)));
    let (f2, r2) = (found.clone(), run.clone());
    let _ = std::thread::Builder::new().name("webox-share-thumbs".into()).spawn(move || thumbs(f2, r2, sample));
    *g = Some(Picker { sources, sel, optimize: Optimize::Text, audio: false, options: true, tex: HashMap::new(), no_pic: HashSet::new(), seen: 0, found, run, scroll: 0.0 });
}

#[allow(dead_code)] // for the call code: a share started elsewhere, the call ended
pub(crate) fn close() {
    if let Some(p) = PICKER.lock().unwrap_or_else(|e| e.into_inner()).take() { p.run.store(false, Ordering::SeqCst); }
}

#[allow(dead_code)]
pub(crate) fn is_open() -> bool { PICKER.lock().map(|p| p.is_some()).unwrap_or(false) }

/// The pick, once (after "Share").
#[allow(dead_code)]
pub(crate) fn take_choice() -> Option<Choice> { CHOICE.lock().unwrap_or_else(|e| e.into_inner()).take() }

/// Two screens and four windows of nobody (screenshot runs).
fn sample_sources() -> Vec<ShareSource> {
    let s = |kind, handle, name: &str, app: &str, size: (u32, u32), minimized| ShareSource { kind, handle, index: handle as u32, name: name.into(), app: app.into(), size, primary: handle == 1, minimized };
    vec![
        s(Kind::Screen, 1, &format!("{} 1", crate::trq!("Screen")), "", (1920, 1080), false),
        s(Kind::Screen, 2, &format!("{} 2", crate::trq!("Screen")), "", (2560, 1440), false),
        s(Kind::Window, 11, "Quarterly planning.xlsx - Excel", "EXCEL", (1600, 900), false),
        s(Kind::Window, 12, "Project notes - Notepad", "notepad", (900, 700), false),
        s(Kind::Window, 13, "Draft presentation - PowerPoint", "POWERPNT", (1400, 900), false),
        s(Kind::Window, 14, "Documents", "explorer", (1000, 640), true),
    ]
}

/// Lists the sources and makes their thumbnails, once a second until the dialog closes.
fn thumbs(found: Arc<Mutex<Found>>, run: Arc<AtomicBool>, sample: bool) {
    let mut gen = 0;
    while run.load(Ordering::SeqCst) {
        let list = if sample { sample_sources() } else { share::sources() };
        let pics: Vec<(Key, Option<Pic>)> = list.iter().map(|s| (key(s), if sample { sample_pic(s) } else { share::thumbnail(s, (320, 204)) })).collect();
        gen += 1;
        if let Ok(mut f) = found.lock() { *f = Found { list, pics, gen }; }
        crate::ui::wake();
        for _ in 0..10 { if !run.load(Ordering::SeqCst) { return; } std::thread::sleep(std::time::Duration::from_millis(100)); }
    }
}

/// A test picture as the thumbnail of a made-up source (none for the minimised one).
fn sample_pic(s: &ShareSource) -> Option<Pic> {
    if s.minimized { return None; }
    let (w, h) = crate::media::share::convert::fit(s.size.0, s.size.1, (320, 204));
    let mut px = vec![0u8; (w * h * 4) as usize];
    crate::media::share::convert::test_picture(s.handle as u32 * 40, w, h, &mut px);
    Some((w, h, px))
}

impl Picker {
    /// Takes the worker's latest list and pictures into textures.
    fn refresh(&mut self, ctx: &egui::Context) {
        let Ok(mut f) = self.found.lock() else { return };
        if f.gen == self.seen { return; }
        self.seen = f.gen;
        // the list keeps its order; closed windows go, new ones join at the end of their section
        let fresh = std::mem::take(&mut f.list);
        self.sources.retain(|s| fresh.iter().any(|n| key(n) == key(s)));
        for n in &fresh { match self.sources.iter_mut().find(|s| key(s) == key(n)) { Some(s) => *s = n.clone(), None => self.sources.push(n.clone()) } }
        self.sources.sort_by_key(|s| view::is_app(s));
        if self.sel.is_some_and(|k| !self.sources.iter().any(|s| key(s) == k)) { self.sel = None; }
        for (k, pic) in std::mem::take(&mut f.pics) {
            match pic {
                Some((w, h, px)) => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                    match self.tex.get_mut(&k) { Some(t) => t.set(img, egui::TextureOptions::LINEAR), None => { self.tex.insert(k, ctx.load_texture(format!("share-thumb-{k:?}"), img, egui::TextureOptions::LINEAR)); } }
                    self.no_pic.remove(&k);
                }
                None => { self.tex.remove(&k); self.no_pic.insert(k); }
            }
        }
    }

    fn choice(&self) -> Option<Choice> {
        let source = self.sources.iter().find(|s| Some(key(s)) == self.sel)?.clone();
        Some(Choice { source, optimize: self.optimize, opts: Opts::preset(self.optimize), audio: self.audio })
    }
}

/// What the dialog's content asks for.
pub(super) enum Act { None, Close, Share }

impl App {
    /// Draws the dialog while it is open (every frame, from frame.rs), and the local test share's card.
    pub(crate) fn share_picker_frame(&mut self, ctx: &egui::Context, t: &Th) {
        if self.frames == 2 && self.shot.is_some() && std::env::var_os("WEBOX_SHOT_SHARE").is_some() { open_with(true); }
        static SAMPLE: std::sync::Once = std::sync::Once::new();
        if self.shot.is_none() && std::env::var_os("WEBOX_SAMPLE_SHARE").is_some() { SAMPLE.call_once(open); }
        self.local_share_card(ctx, t);
        let Some(mut p) = PICKER.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        p.refresh(ctx);
        let mut act = Act::None;
        if self.shot.is_some() {
            // screenshot runs: inside the main window (secondary windows are not in the picture)
            let screen = ctx.screen_rect();
            let r = Rect::from_center_size(screen.center(), Vec2::new(view::SIZE.x.min(screen.width() - 16.0), view::SIZE.y.min(screen.height() - 16.0)));
            egui::Area::new(egui::Id::new("share-picker-shot")).fixed_pos(r.min).order(egui::Order::Foreground).show(ctx, |ui| {
                ui.set_min_size(r.size());
                act = self.share_picker_ui(ui, ctx, t, &mut p, r, false);
            });
        } else {
            let b = egui::ViewportBuilder::default().with_title(crate::tr!("ShareDialogTitleBar", "Share content").to_string()).with_decorations(false)
                .with_inner_size(view::SIZE).with_min_inner_size(view::MIN).with_taskbar(true).with_transparent(false);
            ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("webox-share-picker"), b, |vctx, _class| {
                if vctx.input(|i| i.viewport().close_requested()) { act = Act::Close; }
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(vctx, |ui| {
                    let r = ui.max_rect();
                    let a = self.share_picker_ui(ui, vctx, t, &mut p, r, true);
                    if !matches!(a, Act::None) { act = a; }
                });
                crate::ui::chrome::edge_overlay(vctx); // Linux: no server-side border to resize by
                vctx.request_repaint_after(std::time::Duration::from_millis(500));
            });
        }
        match act {
            Act::None => { *PICKER.lock().unwrap_or_else(|e| e.into_inner()) = Some(p); }
            Act::Close => { p.run.store(false, Ordering::SeqCst); }
            Act::Share => {
                p.run.store(false, Ordering::SeqCst);
                let c = p.choice();
                if std::env::var_os("WEBOX_SAMPLE_SHARE").is_some() {
                    if let Some(c) = &c { *LOCAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(share::start(&c.source, c.opts)); }
                } else {
                    *CHOICE.lock().unwrap_or_else(|e| e.into_inner()) = c;
                }
            }
        }
    }

    /// `WEBOX_SAMPLE_SHARE`: what the local share does (size, encoder, rate), with a button that ends it.
    fn local_share_card(&mut self, ctx: &egui::Context, t: &Th) {
        let mut g = LOCAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = g.as_ref() else { return };
        let id = egui::Id::new("local-share-stats");
        let (mut n, mut bytes, mut keys, t0): (u32, u64, u32, f64) = ctx.data(|d| d.get_temp(id)).unwrap_or((0, 0, 0, ctx.input(|i| i.time)));
        for au in s.drain() { n += 1; bytes += au.data.len() as u64; keys += au.keyframe as u32; }
        let secs = (ctx.input(|i| i.time) - t0).max(0.1);
        ctx.data_mut(|d| d.insert_temp(id, (n, bytes, keys, t0)));
        let line = match s.status() {
            Status::Starting => "…".to_string(),
            Status::Running { width, height, encoder } => format!("{width} × {height}, {:.1} fps, {:.0} kbit/s, {keys} key frames\n{encoder}", n as f64 / secs, bytes as f64 * 8.0 / secs / 1000.0),
            Status::Ended => "Ended".into(),
            Status::Failed(e) => e,
        };
        let mut stop = false;
        egui::Area::new(egui::Id::new("local-share-card")).anchor(egui::Align2::LEFT_BOTTOM, [20.0, -20.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            super::menu::toast_frame(t).show(ui, |ui| {
                ui.set_max_width(420.0);
                ui.label(RichText::new("Local share test (nothing is sent)").font(F::Body.semibold()).color(t.text1));
                ui.label(RichText::new(line).size(13.0).color(t.text2));
                if ui.button(crate::trq!("Stop sharing")).clicked() { stop = true; }
            });
        });
        if stop { *g = None; ctx.data_mut(|d| d.remove::<(u32, u64, u32, f64)>(id)); }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Screens come first ("Screen"), windows after ("Application"); the pick carries the preset of its optimisation.
    #[test]
    fn sections_and_choice() {
        let list = sample_sources();
        let first_app = list.iter().position(view::is_app).unwrap();
        assert!(list[..first_app].iter().all(|s| s.kind == Kind::Screen) && list[first_app..].iter().all(view::is_app));
        let p = Picker { sources: list.clone(), sel: Some(key(&list[first_app])), optimize: Optimize::Motion, audio: true, options: true, tex: HashMap::new(),
            no_pic: HashSet::new(), seen: 0, found: Arc::default(), run: Arc::default(), scroll: 0.0 };
        let c = p.choice().unwrap();
        assert_eq!((c.source.handle, c.opts, c.audio), (list[first_app].handle, Opts::preset(Optimize::Motion), true));
        assert!(sample_pic(&list[0]).is_some() && sample_pic(list.iter().find(|s| s.minimized).unwrap()).is_none());
    }
}
