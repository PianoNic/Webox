//! Plugin hooks on the camera picture (host methods of contract 1.6, docs/plugins.md "Camera pictures"):
//!
//! * `camera.filter`: a plugin's frame filter edits every captured picture in place (RGBA, full size) before anything uses it.
//!   `apply` is the one place every camera frame passes (the capture worker in mod.rs calls it right after decoding; the
//!   Settings > Video preview and the call's camera, `send.rs`, so what the far end sees is filtered too). Each filter runs on a thread of its own,
//!   and the camera thread waits at most `BUDGET` per frame for all of them. A filter that misses it is skipped for that frame:
//!   its previous result is shown again (a replaced background never flashes the real room), or black before it has one. A
//!   filter still busy with an earlier frame is skipped at once. Without filters `apply` is one atomic load.
//! * `camera.source`: a video file (looping, at its own pace) or an image instead of the camera, `file_size` big.
//! * `media.open` / `media.read` / `media.close`: the pictures of a video (looping) or an image at one size, copied into a
//!   plugin's buffer (a moving background). Videos decode with the platform's framework (media/video, no codec of our own) on a
//!   thread of their own, a picture or two ahead.
//!
//! Everything is local: no picture leaves the machine through these hooks.
use super::{Fail, Mode, Pix, Raw, Source};
use collabhost_plugin_api::{FrameFn, VideoFrame, FRAME_RGBA};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long the camera thread waits per frame for the filters (all of them together). 30 fps leave 33 ms per frame.
pub const BUDGET: Duration = Duration::from_millis(20);
/// How long removing a filter waits for a call that is still running.
const STOP_WAIT: Duration = Duration::from_secs(2);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> { m.lock().unwrap_or_else(|e| e.into_inner()) }

/// One picture on its way through a filter.
struct Job { w: u32, h: u32, data: Vec<u8>, seq: u64, time_us: u64 }

struct Filter {
    /// The plugin's key.
    owner: String,
    ctx: usize,
    /// Pictures to the filter's thread (taken when the filter is removed, which ends the thread).
    jobs: Mutex<Option<SyncSender<Job>>>,
    /// Filtered pictures back (late ones too).
    done: Mutex<Receiver<Job>>,
    /// A picture is out at the filter's thread.
    busy: AtomicBool,
    /// The last filtered picture: shown again when the filter misses a frame.
    last: Mutex<Option<Job>>,
    stop: Arc<AtomicBool>,
    exited: Mutex<Receiver<()>>,
    thread: std::thread::ThreadId,
    ran: AtomicU64,
    skipped: AtomicU64,
}

/// The registered filters in the order they run (the app has one, `CHAIN`; tests make their own).
struct Chain { list: Mutex<Vec<Arc<Filter>>>, active: AtomicUsize }
static CHAIN: Chain = Chain::new();

/// Registers `f(ctx, frame)` as `owner`'s frame filter (replacing an earlier one of the same plugin).
pub fn add_filter(owner: &str, f: FrameFn, ctx: usize) -> Result<(), String> { CHAIN.add(owner, f, ctx) }
/// Removes `owner`'s filter (only the one registered with `ctx`, when given) and waits until its thread runs no call any more,
/// so the plugin may free what the filter uses afterwards.
pub fn remove_filter(owner: &str, ctx: Option<usize>) { CHAIN.remove(owner, ctx) }
/// Runs every plugin's frame filter on `raw` (see the module comment). Without filters `raw` comes back untouched; with them
/// as full-size RGBA.
pub fn apply(raw: Raw, seq: u64, time_us: u64) -> Raw { CHAIN.apply(raw, seq, time_us) }
/// (frames filtered in time, frames skipped) of `owner`'s filter.
#[cfg(test)]
pub fn filter_stats(owner: &str) -> Option<(u64, u64)> { CHAIN.stats(owner) }

impl Chain {
    const fn new() -> Chain { Chain { list: Mutex::new(Vec::new()), active: AtomicUsize::new(0) } }

    fn add(&self, owner: &str, f: FrameFn, ctx: usize) -> Result<(), String> {
        self.remove(owner, None);
        let (job_tx, job_rx) = sync_channel::<Job>(1);
        let (done_tx, done_rx) = channel::<Job>();
        let (exit_tx, exit_rx) = channel::<()>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let t = std::thread::Builder::new().name(format!("camera-filter-{owner}")).spawn(move || {
            let _exit = exit_tx; // dropped when the thread ends: tells `remove_filter` no call runs any more
            while let Ok(mut j) = job_rx.recv() {
                if stop2.load(Ordering::SeqCst) { break; }
                let mut v = VideoFrame { size: std::mem::size_of::<VideoFrame>() as u32, format: FRAME_RGBA, width: j.w, height: j.h, stride: j.w * 4,
                    data: j.data.as_mut_ptr(), seq: j.seq, time_us: j.time_us };
                // the plugin-api shim catches panics already; this covers filters written without it
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(ctx as *mut std::ffi::c_void, &mut v)));
                if done_tx.send(j).is_err() { break; }
            }
        }).map_err(|e| e.to_string())?;
        let filter = Filter { owner: owner.to_string(), ctx, jobs: Mutex::new(Some(job_tx)), done: Mutex::new(done_rx), busy: AtomicBool::new(false),
            last: Mutex::new(None), stop, exited: Mutex::new(exit_rx), thread: t.thread().id(), ran: AtomicU64::new(0), skipped: AtomicU64::new(0) };
        let mut l = lock(&self.list);
        l.push(Arc::new(filter));
        self.active.store(l.len(), Ordering::SeqCst);
        Ok(())
    }

    fn remove(&self, owner: &str, ctx: Option<usize>) {
        let gone: Vec<Arc<Filter>> = {
            let mut l = lock(&self.list);
            let (gone, keep) = l.drain(..).partition(|f| f.owner == owner && ctx.map_or(true, |c| c == f.ctx));
            *l = keep;
            self.active.store(l.len(), Ordering::SeqCst);
            gone
        };
        for f in gone {
            f.stop.store(true, Ordering::SeqCst);
            lock(&f.jobs).take();
            // a filter that removes itself from inside its own call must not wait for itself
            if std::thread::current().id() != f.thread { let _ = lock(&f.exited).recv_timeout(STOP_WAIT); }
        }
    }

    /// (frames filtered in time, frames skipped) of `owner`'s filter.
    #[cfg(test)]
    fn stats(&self, owner: &str) -> Option<(u64, u64)> {
        lock(&self.list).iter().find(|f| f.owner == owner).map(|f| (f.ran.load(Ordering::Relaxed), f.skipped.load(Ordering::Relaxed)))
    }

    fn apply(&self, raw: Raw, seq: u64, time_us: u64) -> Raw {
        if self.active.load(Ordering::Relaxed) == 0 { return raw; }
        let filters = lock(&self.list).clone();
        if filters.is_empty() || !raw.valid() || raw.pix == Pix::Mjpg { return raw; }
        let (w, h) = (raw.w, raw.h);
        let mut data = to_rgba(raw);
        let deadline = Instant::now() + BUDGET;
        for f in &filters { data = f.run(Job { w, h, data, seq, time_us }, deadline); }
        Raw { pix: Pix::Rgba, w, h, stride: w as usize * 4, uv_off: 0, data }
    }
}

impl Filter {
    /// Runs the filter on `job` until `deadline`: the filtered picture, or (missed) the last one / black.
    fn run(&self, job: Job, deadline: Instant) -> Vec<u8> {
        let done = lock(&self.done);
        // a late picture of an earlier frame: the newest result there is
        while let Ok(j) = done.try_recv() { self.busy.store(false, Ordering::SeqCst); *lock(&self.last) = Some(j); }
        let (w, h) = (job.w, job.h);
        if !self.busy.load(Ordering::SeqCst) {
            let sent = match lock(&self.jobs).as_ref() { Some(tx) => tx.try_send(job), None => Err(TrySendError::Disconnected(job)) };
            match sent {
                Ok(()) => {
                    self.busy.store(true, Ordering::SeqCst);
                    match done.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(j) => {
                            self.busy.store(false, Ordering::SeqCst);
                            self.ran.fetch_add(1, Ordering::Relaxed);
                            let out = j.data.clone();
                            *lock(&self.last) = Some(j);
                            return out;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => self.busy.store(false, Ordering::SeqCst),
                    }
                }
                // removed meanwhile: the picture passes unchanged
                Err(TrySendError::Disconnected(j)) | Err(TrySendError::Full(j)) => return j.data,
            }
        }
        self.skipped.fetch_add(1, Ordering::Relaxed);
        match lock(&self.last).as_ref() {
            Some(l) if (l.w, l.h) == (w, h) => l.data.clone(),
            _ => black(w as usize, h as usize),
        }
    }
}

fn black(w: usize, h: usize) -> Vec<u8> {
    let mut v = vec![0u8; w * h * 4];
    for p in v.chunks_exact_mut(4) { p[3] = 255; }
    v
}

/// Converts a decoded frame to tightly packed RGBA (a packed RGBA frame is taken as it is).
pub fn to_rgba(raw: Raw) -> Vec<u8> {
    let (w, h, s) = (raw.w as usize, raw.h as usize, raw.stride);
    if raw.pix == Pix::Rgba && s == w * 4 { let mut d = raw.data; d.truncate(w * h * 4); return d; }
    let mut out = vec![255u8; w * h * 4];
    let d = &raw.data;
    let convert = |y: usize, o: &mut [u8]| {
        let row = &d[y * s..];
        match raw.pix {
            Pix::Rgba => o.copy_from_slice(&row[..w * 4]),
            Pix::Bgra => for (o, p) in o.chunks_exact_mut(4).zip(row.chunks_exact(4)) { o[0] = p[2]; o[1] = p[1]; o[2] = p[0]; },
            Pix::Rgb24 => for (o, p) in o.chunks_exact_mut(4).zip(row.chunks_exact(3)) { o[..3].copy_from_slice(p); },
            // two pixels share one chroma pair: its terms are worked out once
            Pix::Yuyv => for (o, p) in o.chunks_exact_mut(8).zip(row.chunks_exact(4)) { yuv2(o, p[0], p[2], p[1], p[3]); },
            Pix::Nv12 => {
                let c = &d[raw.uv_off + (y / 2) * s..];
                for ((o, l), uv) in o.chunks_exact_mut(8).zip(row.chunks_exact(2)).zip(c.chunks_exact(2)) { yuv2(o, l[0], l[1], uv[0], uv[1]); }
                if w % 2 == 1 { let k = w - 1; o[k * 4..k * 4 + 3].copy_from_slice(&super::yuv(row[k], c[k & !1], c[(k & !1) + 1])); }
            }
            Pix::Mjpg => {}
        }
    };
    // a 720p frame in about four bands at once (scoped threads, all joined here); small frames on this thread
    let bands = if w * h >= 320 * 240 { std::thread::available_parallelism().map_or(1, |n| n.get()).min(4) } else { 1 };
    let rows = h.div_ceil(bands).next_multiple_of(2).max(2);
    std::thread::scope(|sc| {
        // the other bands first, band 0 on this thread meanwhile
        for (b, part) in out.chunks_mut(rows * w * 4).enumerate().rev() {
            let convert = &convert;
            let mut work = move || for (i, o) in part.chunks_exact_mut(w * 4).enumerate() { convert(b * rows + i, o) };
            if b == 0 { work() } else { sc.spawn(work); }
        }
    });
    out
}

/// Two RGBA pixels (BT.601 limited range, like `super::yuv`) from their lumas and the chroma pair they share.
#[inline]
fn yuv2(o: &mut [u8], y0: u8, y1: u8, u: u8, v: u8) {
    let (d, e) = (u as i32 - 128, v as i32 - 128);
    let (r, g, b) = (409 * e + 128, -100 * d - 208 * e + 128, 516 * d + 128);
    let cl = |x: i32| (x >> 8).clamp(0, 255) as u8;
    for (o, y) in o.chunks_exact_mut(4).zip([y0, y1]) {
        let c = (y as i32 - 16).max(0) * 298;
        o[0] = cl(c + r); o[1] = cl(c + g); o[2] = cl(c + b);
    }
}

/// Scales `src` (`sw` x `sh` RGBA) to fill `dw` x `dh` (aspect kept, the overhang cut off evenly), bilinear.
pub fn cover(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![255u8; dw * dh * 4];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh * 4 { return out; }
    let s = (dw as f32 / sw as f32).max(dh as f32 / sh as f32);
    let (ox, oy) = ((sw as f32 - dw as f32 / s) / 2.0, (sh as f32 - dh as f32 / s) / 2.0);
    let pos = |o: usize, off: f32, max: usize| { let f = ((o as f32 + 0.5) / s + off - 0.5).clamp(0.0, (max - 1) as f32); let i = f as usize; (i, (i + 1).min(max - 1), ((f - i as f32) * 256.0) as u32) };
    let xs: Vec<(usize, usize, u32)> = (0..dw).map(|x| pos(x, ox, sw)).collect();
    for y in 0..dh {
        let (y0, y1, fy) = pos(y, oy, sh);
        let (r0, r1) = (&src[y0 * sw * 4..(y0 + 1) * sw * 4], &src[y1 * sw * 4..(y1 + 1) * sw * 4]);
        let o = &mut out[y * dw * 4..(y + 1) * dw * 4];
        for (x, &(x0, x1, fx)) in xs.iter().enumerate() {
            for c in 0..3 {
                let top = r0[x0 * 4 + c] as u32 * (256 - fx) + r0[x1 * 4 + c] as u32 * fx;
                let bot = r1[x0 * 4 + c] as u32 * (256 - fx) + r1[x1 * 4 + c] as u32 * fx;
                o[x * 4 + c] = ((top * (256 - fy) + bot * fy) >> 16) as u8;
            }
        }
    }
    out
}

/// A picture file the `image` crate reads (by its content, not its name), as RGBA. None: not a picture (a video, presumably).
fn still(path: &Path) -> Result<Option<(usize, usize, Vec<u8>)>, String> {
    let r = image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
    if r.format().is_none() { return Ok(None); }
    let img = r.decode().map_err(|e| format!("{}: {e}", crate::tr!("ImageRender", "Image")))?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok(Some((w as usize, h as usize, img.into_raw())))
}

/// A video (looping) or an image as pictures of one size (`cover`).
pub struct Clip {
    pub w: usize, pub h: usize,
    /// The current picture and its time (s; keeps growing across loops; 0 for an image).
    pub cur: Vec<u8>, pub pts: f64,
    rx: Option<Receiver<(f64, Vec<u8>)>>,
}

impl Clip {
    /// Opens `path` for pictures of `w` x `h`. Blocks until the first picture is there (a video: up to 10 s).
    pub fn open(path: &Path, w: usize, h: usize) -> Result<Clip, String> {
        if !(16..=3840).contains(&w) || !(16..=2160).contains(&h) { return Err(crate::trq!("Size outside 16x16 to 3840x2160").into_owned()); }
        if let Some((sw, sh, px)) = still(path)? { return Ok(Clip { w, h, cur: cover(&px, sw, sh, w, h), pts: 0.0, rx: None }); }
        let (tx, rx) = sync_channel::<(f64, Vec<u8>)>(2);
        let (otx, orx) = channel::<String>();
        let p = path.to_path_buf();
        std::thread::Builder::new().name("camera-clip".into()).spawn(move || {
            let mut l = match crate::media::video::Looping::open(&p, crate::media::video::sync::MAX_SIZE) { Ok(l) => l, Err(e) => { let _ = otx.send(e); return; } };
            loop {
                let f = match l.next() { Ok(f) => f, Err(e) => { let _ = otx.send(e); return; } };
                // ends when the clip is dropped (the receiver goes away)
                if tx.send((f.pts, cover(&f.rgba, f.w, f.h, w, h))).is_err() { return; }
            }
        }).map_err(|e| e.to_string())?;
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok((pts, cur)) => Ok(Clip { w, h, cur, pts, rx: Some(rx) }),
            Err(_) => Err(orx.try_recv().unwrap_or_else(|_| crate::trq!("The video can't be opened").into_owned())),
        }
    }
    pub fn is_still(&self) -> bool { self.rx.is_none() }
    /// Moves on to the next picture if one is decoded within `wait`; false: the current one stays.
    pub fn advance(&mut self, wait: Duration) -> bool {
        let Some(rx) = &self.rx else { return false };
        match if wait.is_zero() { rx.try_recv().ok() } else { rx.recv_timeout(wait).ok() } {
            Some((pts, px)) => { self.pts = pts; self.cur = px; true }
            None => false,
        }
    }
}

// --- camera.source --------------------------------------------------------------------------------------------------------

static SOURCE: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);

/// `owner` replaces the camera with the file at `path`; None brings the camera back (only from the plugin that set the file).
/// The preview takes it into its key, so it reopens.
pub fn set_source(owner: &str, path: Option<PathBuf>) -> Result<(), String> {
    if let Some(p) = &path { if !p.is_file() { return Err(crate::trq!("File not found: {{0}}", p.display())); } }
    let mut s = lock(&SOURCE);
    if path.is_some() || s.as_ref().is_some_and(|(o, _)| o == owner) { *s = path.map(|p| (owner.to_string(), p)); }
    Ok(())
}
/// The file that replaces the camera, if a plugin set one.
pub fn source() -> Option<PathBuf> { lock(&SOURCE).as_ref().map(|(_, p)| p.clone()) }

/// The height a file source is made at: the camera's maximum resolution, at most 720p (what a call sends), 16:9.
pub fn file_size(max_h: u32) -> (usize, usize) {
    let h = (max_h.clamp(180, 720) as usize) & !1;
    ((h * 16 / 9) & !1, h)
}

/// A video file or an image as the camera (`camera.source`).
struct FileSource { clip: Clip, name: String, t0: Instant, first: Option<f64>, next_at: Instant }
impl Source for FileSource {
    fn mode(&self) -> Mode { Mode { pix: Pix::Rgba, w: self.clip.w as u32, h: self.clip.h as u32, fps: 30.0 } }
    fn modes(&self) -> Vec<Mode> { vec![self.mode()] }
    fn name(&self) -> String { self.name.clone() }
    fn next(&mut self) -> Result<Option<Raw>, Fail> {
        let due = if self.clip.is_still() {
            self.next_at += Duration::from_millis(33);
            self.next_at
        } else {
            if !self.clip.advance(Duration::from_millis(500)) { return Ok(None); }
            let first = *self.first.get_or_insert(self.clip.pts);
            self.t0 + Duration::from_secs_f64((self.clip.pts - first).max(0.0))
        };
        let now = Instant::now();
        // paced at the file's own times (late pictures go out at once; never more than 1 s of sleep)
        if due > now { std::thread::sleep((due - now).min(Duration::from_secs(1))); }
        let (w, h) = (self.clip.w as u32, self.clip.h as u32);
        Ok(Some(Raw { pix: Pix::Rgba, w, h, stride: w as usize * 4, uv_off: 0, data: self.clip.cur.clone() }))
    }
}

/// Opens the plugin's file source in place of the camera.
pub fn open_source(path: &Path, max_h: u32) -> Result<Box<dyn Source>, Fail> {
    let (w, h) = file_size(max_h);
    let clip = Clip::open(path, w, h).map_err(|e| Fail::Other(format!("{}: {e}", path.display())))?;
    let name = path.file_name().map_or_else(|| crate::tr!("FileTab", "File").into_owned(), |n| n.to_string_lossy().into_owned());
    Ok(Box::new(FileSource { clip, name, t0: Instant::now(), first: None, next_at: Instant::now() }))
}

// --- media.open / media.read / media.close --------------------------------------------------------------------------------

static CLIPS: Mutex<Vec<(u64, String, Arc<Mutex<Clip>>)>> = Mutex::new(Vec::new());
static NEXT_CLIP: AtomicU64 = AtomicU64::new(1);
/// Clips one plugin may hold open.
const CLIPS_MAX: usize = 4;

pub fn media_open(owner: &str, path: &Path, w: usize, h: usize) -> Result<(u64, bool), String> {
    if lock(&CLIPS).iter().filter(|c| c.1 == owner).count() >= CLIPS_MAX { return Err(crate::trq!("{{0}} open files at most", CLIPS_MAX)); }
    let clip = Clip::open(path, w, h)?;
    let still = clip.is_still();
    let id = NEXT_CLIP.fetch_add(1, Ordering::SeqCst);
    lock(&CLIPS).push((id, owner.to_string(), Arc::new(Mutex::new(clip))));
    Ok((id, still))
}

/// Copies clip `id`'s next picture (the current one again if the next is not decoded yet) into `dst`, rows `stride` apart.
/// The size must be the one it was opened with. The time of the picture.
pub fn media_read(owner: &str, id: u64, dst: &mut [u8], w: usize, h: usize, stride: usize) -> Result<f64, String> {
    let clip = lock(&CLIPS).iter().find(|c| c.0 == id && c.1 == owner).map(|c| c.2.clone()).ok_or("unbekannte id")?;
    let mut c = lock(&clip);
    if (w, h) != (c.w, c.h) { return Err(crate::trq!("Size {{0}}x{{1}} expected", c.w, c.h)); }
    if stride < w * 4 || dst.len() < stride * (h - 1) + w * 4 { return Err(crate::trq!("Buffer too small").into_owned()); }
    c.advance(Duration::ZERO);
    for y in 0..h { dst[y * stride..y * stride + w * 4].copy_from_slice(&c.cur[y * w * 4..(y + 1) * w * 4]); }
    Ok(c.pts)
}

pub fn media_close(owner: &str, id: Option<u64>) { lock(&CLIPS).retain(|c| !(c.1 == owner && id.map_or(true, |i| i == c.0))); }

/// Everything a plugin set up here goes when it stops or unloads: its filter (waiting for a running call), its source, its clips.
pub fn release(owner: &str) {
    remove_filter(owner, None);
    let _ = set_source(owner, None);
    media_close(owner, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::AtomicU32;

    fn frame(w: u32, h: u32, v: u8) -> Raw { Raw { pix: Pix::Rgb24, w, h, stride: w as usize * 3, uv_off: 0, data: vec![v; (w * h * 3) as usize] } }

    static CALLS: AtomicU32 = AtomicU32::new(0);
    /// Inverts the picture and counts its calls.
    extern "C" fn invert(_: *mut c_void, f: *mut VideoFrame) -> i32 {
        let f = unsafe { &mut *f };
        let d = unsafe { std::slice::from_raw_parts_mut(f.data, (f.stride * f.height) as usize) };
        for p in d.chunks_exact_mut(4) { for c in &mut p[..3] { *c = 255 - *c; } }
        CALLS.fetch_add(1, Ordering::SeqCst);
        1
    }
    /// Takes 1.5 s per frame except the first: far over the budget (so the checks below hold on a slow, busy CI runner too).
    extern "C" fn slow(ctx: *mut c_void, f: *mut VideoFrame) -> i32 {
        let n = unsafe { &*(ctx as *const AtomicU32) }.fetch_add(1, Ordering::SeqCst);
        let f = unsafe { &mut *f };
        let d = unsafe { std::slice::from_raw_parts_mut(f.data, (f.stride * f.height) as usize) };
        for p in d.chunks_exact_mut(4) { p[0] = 200; }
        if n > 0 { std::thread::sleep(Duration::from_millis(1500)); }
        1
    }

    // One chain of its own (the app's is global and the camera tests run next to this one).
    #[test]
    fn filters_run_per_frame_slow_ones_are_skipped_and_removal_waits() {
        let c = Chain::new();
        // No filter: the frame comes back as it was.
        let r = c.apply(frame(8, 4, 10), 1, 0);
        assert_eq!((r.pix, r.data.len()), (Pix::Rgb24, 8 * 4 * 3));

        // A filter: called once per frame, its change is in the picture, the picture is RGBA.
        c.add("inv", invert, 0).unwrap();
        for i in 1..=5 {
            let r = c.apply(frame(8, 4, 10), i, i * 33_000);
            assert_eq!((r.pix, r.stride, r.data.len()), (Pix::Rgba, 32, 8 * 4 * 4));
            assert_eq!(&r.data[..4], &[245, 245, 245, 255]);
        }
        assert_eq!(CALLS.load(Ordering::SeqCst), 5);
        assert_eq!(c.stats("inv"), Some((5, 0)));
        c.remove("inv", None);
        assert!(c.stats("inv").is_none());
        assert_eq!(c.apply(frame(8, 4, 10), 6, 0).pix, Pix::Rgb24);

        // A slow filter: the frame is not held up past the budget; it shows the filter's last picture instead (never the
        // unfiltered one), and frames while the filter is still busy are skipped at once.
        let n: &'static AtomicU32 = Box::leak(Box::new(AtomicU32::new(0)));
        c.add("slow", slow, n as *const AtomicU32 as usize).unwrap();
        // the first call is fast (on a busy machine the thread may start late: then a later call brings its result)
        let mut r = c.apply(frame(8, 4, 10), 1, 0);
        for _ in 0..200 { if r.data[0] == 200 { break; } std::thread::sleep(Duration::from_millis(5)); r = c.apply(frame(8, 4, 10), 1, 0); }
        assert_eq!(r.data[0], 200);
        let t = Instant::now();
        let r = c.apply(frame(8, 4, 50), 2, 0);
        // not held up until the filter is done (1.5 s); the budget is 20 ms, the rest is slack for a busy machine
        assert!(t.elapsed() < Duration::from_millis(700), "{:?}", t.elapsed());
        assert_eq!((r.data[0], r.data[1]), (200, 10), "the last filtered picture, not the new raw one");
        let t = Instant::now();
        let r = c.apply(frame(8, 4, 50), 3, 0);
        assert!(t.elapsed() < Duration::from_millis(500), "busy: skipped, not waited for ({:?})", t.elapsed());
        assert_eq!(r.data[0], 200);
        let (_, skipped) = c.stats("slow").unwrap();
        assert!(skipped >= 2, "{skipped}");
        // Removing it waits until the running call is over: afterwards it is never called again.
        c.remove("slow", None);
        let calls = n.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(n.load(Ordering::SeqCst), calls);
        // Owner and ctx: another plugin's filter stays, a stale ctx removes nothing.
        c.add("a", invert, 1).unwrap();
        c.remove("a", Some(2));
        assert!(c.stats("a").is_some());
        c.remove("a", None);
        assert!(c.stats("a").is_none());
    }

    extern "C" fn nothing(_: *mut c_void, _: *mut VideoFrame) -> i32 { 0 }

    /// By hand (release): what the hook costs the camera thread per 1280 x 720 frame.
    /// `cargo test --release --bin webox hook_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn hook_cost() {
        let (w, h) = (1280u32, 720u32);
        let nv12 = || Raw { pix: Pix::Nv12, w, h, stride: w as usize, uv_off: (w * h) as usize, data: vec![120u8; (w * h * 3 / 2) as usize] };
        let median = |f: &mut dyn FnMut()| { let mut t: Vec<f64> = (0..60).map(|_| { let s = Instant::now(); f(); s.elapsed().as_secs_f64() * 1000.0 }).collect(); t.sort_by(|a, b| a.partial_cmp(b).unwrap()); t[30] };
        let c = Chain::new();
        let none = median(&mut || { std::hint::black_box(c.apply(nv12(), 1, 0)); });
        let convert = median(&mut || { std::hint::black_box(to_rgba(nv12())); });
        c.add("n", nothing, 0).unwrap();
        let mut seq = 0;
        let one = median(&mut || { seq += 1; std::hint::black_box(c.apply(nv12(), seq, 0)); });
        c.remove("n", None);
        let alloc = median(&mut || { std::hint::black_box(nv12()); });
        println!("1280x720 NV12, median of 60 (each includes making the test frame, {alloc:.2} ms):");
        println!("  no filter: {none:.2} ms, NV12 -> RGBA: {convert:.2} ms, one empty filter (convert + hand-off + copy back): {one:.2} ms");
    }

    #[test]
    fn conversions_and_cover() {
        // NV12 grey (Y 126, U = V = 128) and YUYV give the same RGBA.
        let (w, h) = (4u32, 2u32);
        let mut nv = vec![126u8; 8 + 4];
        nv[8..].fill(128);
        let a = to_rgba(Raw { pix: Pix::Nv12, w, h, stride: 4, uv_off: 8, data: nv });
        let b = to_rgba(Raw { pix: Pix::Yuyv, w, h, stride: 8, uv_off: 0, data: [126u8, 128].repeat(8) });
        assert_eq!(a, b);
        assert!(a.chunks_exact(4).all(|p| p == [128, 128, 128, 255]), "{:?}", &a[..4]);
        // a frame big enough to be converted in bands: every pixel as the reference conversion has it
        let (w, h) = (642usize, 362usize);
        let data: Vec<u8> = (0..w * h * 3 / 2).map(|i| (i * 7919 % 251) as u8).collect();
        let raw = || Raw { pix: Pix::Nv12, w: w as u32, h: h as u32, stride: w, uv_off: w * h, data: data.clone() };
        let out = to_rgba(raw());
        let r = raw();
        for y in (0..h).step_by(7) { for x in 0..w { assert_eq!(&out[(y * w + x) * 4..(y * w + x) * 4 + 3], &r.rgb(x as u32, y as u32), "{x},{y}"); } }
        let bgra = to_rgba(Raw { pix: Pix::Bgra, w: 1, h: 1, stride: 4, uv_off: 0, data: vec![1, 2, 3, 0] });
        assert_eq!(bgra, vec![3, 2, 1, 255]);
        // cover: a 4x1 strip (left half red, right half blue) into 2x2 keeps the middle: both colours, cut evenly.
        let src: Vec<u8> = [[255, 0, 0, 255], [255, 0, 0, 255], [0, 0, 255, 255], [0, 0, 255, 255]].concat();
        let out = cover(&src, 4, 1, 2, 2);
        assert_eq!(out.len(), 16);
        assert!(out[0] > 128 && out[2] < 128 && out[4] < 128 && out[6] > 128, "{out:?}");
        assert_eq!(file_size(1080), (1280, 720));
        assert_eq!(file_size(360), (640, 360));
    }

    #[test]
    fn an_image_is_a_still_source_and_a_clip() {
        let dir = std::env::temp_dir().join(format!("webox-cam-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bg.png");
        image::RgbaImage::from_pixel(64, 36, image::Rgba([0, 200, 0, 255])).save(&p).unwrap();
        let mut s = open_source(&p, 360).ok().expect("opens");
        let m = s.mode();
        assert_eq!((m.w, m.h, m.pix), (640, 360, Pix::Rgba));
        let f = s.next().unwrap().unwrap();
        assert_eq!(&f.data[..4], &[0, 200, 0, 255]);
        // media.open / read / close: copied into a buffer with a stride, refused at another size
        let (id, still) = media_open("p", &p, 32, 18).unwrap();
        assert!(still);
        let mut buf = vec![0u8; 140 * 18];
        assert_eq!(media_read("p", id, &mut buf, 32, 18, 140), Ok(0.0));
        assert_eq!(&buf[..4], &[0, 200, 0, 255]);
        assert!(media_read("p", id, &mut buf, 16, 18, 140).is_err());
        assert!(media_read("other", id, &mut buf, 32, 18, 140).is_err(), "another plugin's clip");
        media_close("p", Some(id));
        assert!(media_read("p", id, &mut buf, 32, 18, 140).is_err());
        assert!(Clip::open(&dir.join("missing.mp4"), 32, 18).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
