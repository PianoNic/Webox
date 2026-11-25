//! Videos in the message list: the inline video card, its player, the full-screen viewer, poster frames.
//!
//! A shared video is a card like a picture (the share's thumbnail or the first frame as poster, a play button, a hover banner
//! with name, size and download) and plays inline. The file is downloaded and decrypted into a playback cache, then played with
//! the platform's framework (`media::video`).
//! One video plays at a time; starting another stops the first. A card that is not on screen pauses its video, and after a
//! minute off screen the player is closed.
use super::*;
use crate::app::files::{self as fs_, Phase, Then};
use crate::media::video::{self as mv, format, Player};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

mod bar;
mod card;
mod viewer;
pub(super) use card::card_size;

/// An attachment the list shows as a video card.
pub(super) fn is_video(a: &Attachment) -> bool { format::is_video(&a.mime, &a.name) }
#[cfg(test)]
mod tests;

/// Diameter of the play button.
const PLAY_XL: f32 = 40.0;
/// Height of the hover banner and of the control bar.
const BANNER_H: f32 = 52.0;
/// Seconds a paused video stays open while its card is off screen.
const OFFSCREEN_CLOSE_S: f64 = 60.0;
/// The playback cache (decrypted shares fetched to play them) is trimmed to this size, oldest first.
const CACHE_MAX: u64 = 2 << 30;

/// The video that plays (or is paused) right now.
struct Active {
    key: String,
    name: String,
    player: Player,
    tex: Option<egui::TextureHandle>,
    /// Size of the picture in `tex`.
    size: [usize; 2],
    /// Pass in which the card / viewer drew it last, and when.
    drawn: u64,
    drawn_at: f64,
    viewer: bool,
    /// The window was not full screen before the viewer made it so.
    restore_windowed: bool,
    volume_open: bool,
    /// Control bar opacity (300 ms fade) and the last time the pointer moved over the video.
    bar: f32,
    moved_at: f64,
    /// While the seek bar is dragged: the position under the pointer.
    scrub: Option<f64>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    /// A card whose file is still coming down: it starts playing once it has arrived.
    static WANTED: RefCell<Option<String>> = const { RefCell::new(None) };
    static POSTER_TEX: RefCell<HashMap<PathBuf, (egui::TextureHandle, [usize; 2])>> = RefCell::new(HashMap::new());
    static HOOKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// First frames decoded on worker threads (poster of a video share without thumbnail, the composer's preview), with the duration.
enum Poster { Pending, Ready(egui::ColorImage, f64), Failed }
static POSTERS: Mutex<Option<HashMap<PathBuf, Poster>>> = Mutex::new(None);

/// The poster texture of a local file and its duration; starts decoding it the first time (None until it is ready).
pub(crate) fn poster(ctx: &egui::Context, path: &Path) -> Option<(egui::TextureId, [usize; 2], f64)> {
    if let Some((t, s)) = POSTER_TEX.with(|m| m.borrow().get(path).map(|(t, s)| (t.id(), *s))) {
        return Some((t, s, poster_duration(path)));
    }
    let mut g = POSTERS.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    match m.get(path) {
        None => {
            m.insert(path.to_path_buf(), Poster::Pending);
            let (p, c) = (path.to_path_buf(), ctx.clone());
            std::thread::spawn(move || {
                let r = mv::first_frame(&p).map(|(f, i)| (egui::ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba), i.duration));
                let mut g = POSTERS.lock().unwrap_or_else(|e| e.into_inner());
                g.get_or_insert_with(HashMap::new).insert(p, match r { Ok((img, d)) => Poster::Ready(img, d), Err(_) => Poster::Failed });
                c.request_repaint();
            });
            None
        }
        Some(Poster::Ready(img, _)) => {
            let size = img.size;
            let tex = ctx.load_texture(format!("video-poster:{}", path.display()), img.clone(), egui::TextureOptions::LINEAR);
            let id = tex.id();
            POSTER_TEX.with(|t| t.borrow_mut().insert(path.to_path_buf(), (tex, size)));
            drop(g);
            Some((id, size, poster_duration(path)))
        }
        _ => None,
    }
}
fn poster_duration(path: &Path) -> f64 {
    POSTERS.lock().ok().and_then(|g| g.as_ref().and_then(|m| match m.get(path) { Some(Poster::Ready(_, d)) => Some(*d), _ => None })).unwrap_or(0.0)
}
fn poster_failed(path: &Path) -> bool { POSTERS.lock().ok().is_some_and(|g| g.as_ref().is_some_and(|m| matches!(m.get(path), Some(Poster::Failed)))) }

/// Where a video's bytes are.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Src { File(PathBuf), Fetching(u32), Remote, Failed(String), Preparing }

/// Folder of the playback cache (decrypted videos): this user's private temporary folder (`files::private_tmp`); None when it
/// cannot be made private (then nothing is cached or played from the cache).
fn cache_dir() -> Option<PathBuf> { fs_::private_tmp("video-cache").ok() }
/// The cached file of a share: one folder per attachment key (hashed), the file under its own (safe) name so the framework sees
/// its extension.
pub(super) fn cache_path(key: &str, name: &str) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    Some(cache_dir()?.join(format!("{:016x}", h.finish())).join(fs_::safe_name(name)))
}
/// Removes the oldest cached videos until the cache is under `CACHE_MAX` (and everything older than a day).
fn trim_cache(keep: &Path) {
    let Some(Ok(rd)) = cache_dir().map(std::fs::read_dir) else { return };
    let mut items: Vec<(std::time::SystemTime, u64, PathBuf)> = rd.flatten().filter_map(|e| {
        let p = e.path();
        let (size, at) = std::fs::read_dir(&p).ok()?.flatten().filter_map(|f| f.metadata().ok()).fold((0u64, std::time::UNIX_EPOCH), |(s, t), m| (s + m.len(), t.max(m.modified().unwrap_or(t))));
        Some((at, size, p))
    }).collect();
    items.sort();
    let mut total: u64 = items.iter().map(|i| i.1).sum();
    let day = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400);
    for (at, size, p) in items {
        if keep.starts_with(&p) { continue; }
        if total > CACHE_MAX || at < day { let _ = std::fs::remove_dir_all(&p); total = total.saturating_sub(size); }
    }
}

use mv::synth::SAMPLE_KEY;
static SAMPLE: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);
/// The sample clip, made on a worker the first time (encoding takes a moment).
fn sample_src(ctx: &egui::Context) -> Src {
    let g = SAMPLE.lock().unwrap_or_else(|e| e.into_inner());
    match &*g {
        Some(Some(p)) => Src::File(p.clone()),
        Some(None) => Src::Failed(crate::trq!("We ran into an issue while trying to play the video. Try again in a bit.").into_owned()),
        None => {
            static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let c = ctx.clone();
                std::thread::spawn(move || { let p = mv::synth::sample_clip(); *SAMPLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(p); c.request_repaint(); });
            }
            Src::Preparing
        }
    }
}

/// Where the bytes of attachment `a` are: a file of this computer (pending upload, a file sent from here, the sample clip), an
/// earlier download or the playback cache, a download on its way, or only on the service.
pub(super) fn source(ctx: &egui::Context, a: &Attachment) -> Src {
    if a.key == SAMPLE_KEY { return sample_src(ctx); }
    if let Some(p) = super::super::attach::local_path(&a.key) { return Src::File(p); }
    match fs_::download(&a.key) {
        Some(d) if d.phase == Phase::Done && d.path.exists() => Src::File(d.path),
        Some(d) if matches!(d.phase, Phase::Preparing | Phase::Uploading) => Src::Fetching(super::super::attach::percent(d.got, d.total)),
        Some(d) if d.then == Then::Play => match d.phase { Phase::Failed(e) => Src::Failed(e), _ => Src::Remote },
        _ => match cache_path(&a.key, &a.name) { Some(c) if c.exists() => Src::File(c), _ => Src::Remote },
    }
}

fn speaker(settings: &serde_json::Value) -> String { crate::media::audio::Choice::load(settings).output }

impl App {
    /// Starts (or resumes) the video of `a`: opens the file when it is here, else fetches it first and plays when it has arrived.
    pub(super) fn video_start(&mut self, ctx: &egui::Context, a: &Attachment, now: f64) {
        let resumed = ACTIVE.with(|s| s.borrow().as_ref().filter(|x| x.key == a.key).map(|x| { x.player.play(); }).is_some());
        if resumed { return; }
        match source(ctx, a) {
            Src::File(p) => {
                // an unknown container is not handed to the framework (GStreamer would also follow playlists such as HLS / DASH)
                if !format::detect(&head(&p), &a.name, &a.mime).is_some_and(format::playable) { self.toast(crate::trq!("Can't play video inline").into_owned(), now); return; }
                let player = Player::open(&p, Some(&speaker(&self.settings)), Some(ctx.clone()));
                player.set_volume(bar::last_volume());
                player.play();
                WANTED.with(|w| *w.borrow_mut() = None);
                hook(ctx);
                ACTIVE.with(|s| *s.borrow_mut() = Some(Active { key: a.key.clone(), name: a.name.clone(), player, tex: None, size: [0, 0], drawn: ctx.cumulative_pass_nr(), drawn_at: now,
                    viewer: false, restore_windowed: false, volume_open: false, bar: 1.0, moved_at: now, scrub: None }));
            }
            Src::Remote | Src::Failed(_) => {
                let Some(path) = cache_path(&a.key, &a.name) else {
                    self.toast(crate::trq!("We ran into an issue while trying to play the video. Try again in a bit.").into_owned(), now);
                    return;
                };
                if let Some(d) = path.parent() { let _ = std::fs::create_dir_all(d); }
                trim_cache(&path);
                WANTED.with(|w| *w.borrow_mut() = Some(a.key.clone()));
                stop_active();
                self.file_fetch_to(&a.key, &a.name, &path, Then::Play, now);
            }
            Src::Fetching(_) | Src::Preparing => WANTED.with(|w| *w.borrow_mut() = Some(a.key.clone())),
        }
    }

    /// The card's download button / the bar's "Download": the file into the download folder. A video that is already here
    /// (played from the cache, or a local file) is copied instead of downloaded again.
    pub(super) fn video_download(&mut self, ctx: &egui::Context, a: &Attachment, now: f64) {
        match source(ctx, a) {
            Src::File(p) if super::super::attach::local_path(&a.key).is_none() && a.key != SAMPLE_KEY && fs_::download(&a.key).is_some_and(|d| d.then != Then::Play) => {
                let _ = p; // an earlier "Download": announce it again
                self.file_fetch(&a.key, &a.name, Then::Nothing, now);
            }
            Src::File(p) => {
                let dest = fs_::unique_path(&fs_::download_dir(&self.settings), &a.name);
                self.file_write_local(&a.key, &a.name, &p, &dest, now);
                // a copy of the playback cache came from the chat: Mark-of-the-Web like a download (not for files of this computer)
                if super::super::attach::local_path(&a.key).is_none() && a.key != SAMPLE_KEY { fs_::mark_downloaded(&dest); }
                self.toast(format!("{} – {}", a.name, crate::ui::widgets::tr("Download complete")), now);
            }
            _ => self.file_fetch(&a.key, &a.name, Then::Nothing, now),
        }
    }
}

/// The copy in the download folder (or the local file) that "Show in folder" can show; never the playback cache.
pub(super) fn shown_file(a: &Attachment) -> Option<PathBuf> {
    if let Some(p) = super::super::attach::local_path(&a.key) { return Some(p); }
    fs_::download(&a.key).filter(|d| d.phase == Phase::Done && d.then != Then::Play && d.path.exists() && !cache_dir().is_some_and(|c| d.path.starts_with(c))).map(|d| d.path)
}

fn head(p: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut b = vec![0u8; 4096];
    let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut b)).unwrap_or(0);
    b.truncate(n);
    b
}

/// Closes the open video (dropping the player stops its decoder and its sound).
fn stop_active() { drop(ACTIVE.with(|s| s.borrow_mut().take())); }

/// Once per pass (end of the pass): a video whose card was not drawn pauses, and after a while off screen it is closed; a fetch
/// that has finished starts its video when the card is drawn next.
fn hook(ctx: &egui::Context) {
    if HOOKED.with(|h| h.replace(true)) { return; }
    ctx.on_end_pass("webox-video", std::sync::Arc::new(|ctx: &egui::Context| {
        let pass = ctx.cumulative_pass_nr();
        let now = ctx.input(|i| i.time);
        let close = ACTIVE.with(|s| {
            let s = s.borrow();
            let Some(a) = s.as_ref() else { return false };
            if a.drawn + 1 < pass && a.player.playing() { a.player.pause(); }
            a.drawn + 1 < pass && now - a.drawn_at > OFFSCREEN_CLOSE_S
        });
        if close { ACTIVE.with(|s| *s.borrow_mut() = None); }
    }));
}

/// The key of the attachment whose video is open, if any.
pub(super) fn active_key() -> Option<String> { ACTIVE.with(|s| s.borrow().as_ref().map(|a| a.key.clone())) }
pub(super) fn wanted(key: &str) -> bool { WANTED.with(|w| w.borrow().as_deref() == Some(key)) }
