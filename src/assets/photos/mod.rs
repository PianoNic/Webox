//! Profile pictures and presence details delivered by the cloud thread (`Resp::Avatars`, `Resp::Presence`).
use eframe::egui;
use std::collections::HashMap;

pub mod round;
mod round_measure;

/// A decoded message image: one or more frames (texture, delay in seconds); animated GIFs have several.
/// `bytes` = texture memory (RGBA), `used` = the last pass it was asked for (for the memory budget).
pub struct Anim { frames: Vec<(egui::TextureHandle, f32)>, size: [usize; 2], total: f32, start: Option<f64>, bytes: usize, used: u64 }

/// Decoded message images (all frames of all GIFs) kept in texture memory at most; beyond it the ones not drawn for the
/// longest time are dropped and decoded again (on a worker) when they come back on screen.
const IMAGE_TEX_BUDGET: usize = 256 * 1024 * 1024;

/// Message images are decoded on worker threads (a big picture or a long GIF would block the UI thread for tens to hundreds
/// of ms); the textures are created there too (egui's texture manager is thread-safe) and handed back with the key's generation.
struct Decoded { key: String, generation: u64, anim: Option<Anim> }
struct Job { key: String, generation: u64, bytes: Vec<u8>, ctx: egui::Context }
struct Decoder { tx: std::sync::mpsc::Sender<Job>, rx: std::sync::mpsc::Receiver<Decoded> }
impl Decoder {
    fn start() -> Self {
        let (tx, jobs) = std::sync::mpsc::channel::<Job>();
        let (done, rx) = std::sync::mpsc::channel::<Decoded>();
        let jobs = std::sync::Arc::new(std::sync::Mutex::new(jobs));
        let n = std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1).clamp(1, 3);
        for i in 0..n {
            let (jobs, done) = (jobs.clone(), done.clone());
            let _ = std::thread::Builder::new().name(format!("image-decode-{i}")).spawn(move || loop {
                let job = { let Ok(rx) = jobs.lock() else { break }; let Ok(j) = rx.recv() else { break }; j };
                let anim = decode_anim(&job.ctx, &job.key, &job.bytes);
                if done.send(Decoded { key: job.key, generation: job.generation, anim }).is_err() { break }
                job.ctx.request_repaint();
            });
        }
        Decoder { tx, rx }
    }
}

pub const MAX_FRAMES: usize = 60;
pub const MAX_FRAME_SIDE: u32 = 512;
/// Largest side of a static image texture (typical GPU limit is 8192–16384).
pub const MAX_TEXTURE_SIDE: u32 = 8192;
const MAX_ANIM_BYTES: usize = 96 * 1024 * 1024;

fn upload(ctx: &egui::Context, name: String, img: &image::RgbaImage) -> egui::TextureHandle {
    ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()), egui::TextureOptions::LINEAR)
}

/// Hard stop for pathological GIFs (decoding is sequential and costs time even for frames we drop).
const MAX_RAW_FRAMES: usize = 2000;
/// Every frame composites the whole canvas, so the frame count is also capped by canvas size: at most this many canvas pixels in
/// total (4096 x 4096 allows 128 frames, 500 x 500 the full MAX_RAW_FRAMES). A tiny GIF with a huge canvas and many 1 x 1 frames
/// would otherwise keep a decode worker busy for minutes.
const GIF_MAX_WORK_PIXELS: u64 = 2 * 1024 * 1024 * 1024;

/// How many frames of this GIF are decoded at most (from the header's canvas size).
pub(crate) fn gif_frame_budget(bytes: &[u8]) -> usize {
    let Some(s) = bytes.get(6..10) else { return 0 };
    let px = (u16::from_le_bytes([s[0], s[1]]) as u64 * u16::from_le_bytes([s[2], s[3]]) as u64).max(1);
    ((GIF_MAX_WORK_PIXELS / px) as usize).clamp(1, MAX_RAW_FRAMES)
}

/// Largest GIF canvas (logical screen) we decode: ten header bytes can claim 65535 x 65535, and the decoder allocates the whole
/// canvas (RGBA) before the first frame (~17 GB).
const GIF_MAX_SIDE: u32 = 4096;
const GIF_MAX_PIXELS: u64 = 16 * 1024 * 1024;
/// What the GIF decoder may allocate at once (canvas + one frame buffer at the largest allowed canvas).
const GIF_MAX_ALLOC: u64 = 256 * 1024 * 1024;

/// The logical screen size in the GIF header (bytes 6..10, little-endian) is within `GIF_MAX_SIDE` / `GIF_MAX_PIXELS`.
pub(crate) fn gif_canvas_ok(bytes: &[u8]) -> bool {
    let Some(s) = bytes.get(6..10) else { return false };
    let (w, h) = (u16::from_le_bytes([s[0], s[1]]) as u32, u16::from_le_bytes([s[2], s[3]]) as u32);
    w <= GIF_MAX_SIDE && h <= GIF_MAX_SIDE && w as u64 * h as u64 <= GIF_MAX_PIXELS
}

/// A GIF decoder for bytes from the network: refused when the header's canvas is too large (before anything is decoded), and
/// with allocation limits for whatever the frames claim later.
pub(crate) fn gif_decoder(bytes: &[u8]) -> image::ImageResult<image::codecs::gif::GifDecoder<std::io::Cursor<&[u8]>>> {
    use image::ImageDecoder;
    if !gif_canvas_ok(bytes) { return Err(image::ImageError::Limits(image::error::LimitError::from_kind(image::error::LimitErrorKind::DimensionError))); }
    let mut d = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))?;
    let mut l = image::Limits::default(); // non_exhaustive: set the fields one by one
    l.max_image_width = Some(GIF_MAX_SIDE);
    l.max_image_height = Some(GIF_MAX_SIDE);
    l.max_alloc = Some(GIF_MAX_ALLOC);
    d.set_limits(l)?;
    Ok(d)
}

/// Keeps every second frame and gives it the delay of the pair, so the total duration is unchanged.
pub(crate) fn halve_frames<T>(frames: Vec<(T, f32)>) -> Vec<(T, f32)> {
    let mut out: Vec<(T, f32)> = Vec::with_capacity(frames.len() / 2 + 1);
    for (i, f) in frames.into_iter().enumerate() {
        if i % 2 == 0 { out.push(f) } else if let Some(last) = out.last_mut() { last.1 += f.1 }
    }
    out
}

/// Decodes a PNG/JPEG/WebP (one frame) or a GIF (all frames, at most `MAX_FRAMES`, each at most `MAX_FRAME_SIDE` px).
/// A failure is logged as "download unusable" vs "unsupported format" with the byte count and format only.
fn decode_anim(ctx: &egui::Context, key: &str, bytes: &[u8]) -> Option<Anim> {
    // Full-resolution viewer images get bigger frames; static ones are capped at what a GPU texture can hold.
    let (max_side, max_mem) = if key.starts_with("full:") { (1024, 192 * 1024 * 1024) } else { (MAX_FRAME_SIDE, MAX_ANIM_BYTES) };
    use image::AnimationDecoder;
    let fmt = image::guess_format(bytes).ok();
    if fmt == Some(image::ImageFormat::Gif) {
        let frames = gif_decoder(bytes).map(|d| d.into_frames());
        // Long GIFs: instead of cutting the loop short, keep every 2nd, 4th, … frame and add the dropped frames' delays to the
        // kept one, so the loop duration stays right with at most MAX_FRAMES frames in memory.
        let mut buf: Vec<(image::RgbaImage, f32)> = vec![];
        let mut size = [0usize; 2];
        let budget = gif_frame_budget(bytes);
        match frames {
            Ok(frames) => for (i, f) in frames.enumerate() {
                if i >= budget { break }
                let Ok(f) = f else { break };
                let (n, d) = f.delay().numer_denom_ms();
                let delay = (n as f32 / d.max(1) as f32 / 1000.0).max(0.02);
                let mut img = f.into_buffer();
                if img.width().max(img.height()) > max_side {
                    let s = max_side as f32 / img.width().max(img.height()) as f32;
                    img = image::imageops::resize(&img, ((img.width() as f32 * s) as u32).max(1), ((img.height() as f32 * s) as u32).max(1), image::imageops::FilterType::Triangle);
                }
                size = [img.width() as usize, img.height() as usize];
                buf.push((img, delay));
                if buf.len() > MAX_FRAMES || buf.len() * size[0] * size[1] * 4 > max_mem { buf = halve_frames(buf); }
            },
            Err(e) => { crate::cloud::dbg_conn(&format!("image decode failed: gif header ({} bytes): {e}", bytes.len())); return None }
        }
        let out: Vec<(egui::TextureHandle, f32)> = buf.into_iter().enumerate().map(|(i, (img, d))| (upload(ctx, format!("img:{key}:{i}"), &img), d)).collect();
        if out.is_empty() { crate::cloud::dbg_conn(&format!("image decode failed: gif without frames ({} bytes)", bytes.len())); return None }
        let total = out.iter().map(|f| f.1).sum();
        let bytes = out.len() * size[0] * size[1] * 4;
        return Some(Anim { frames: out, size, total, start: None, bytes, used: 0 });
    }
    match image::load_from_memory(bytes) {
        Ok(img) => {
            let mut img = img.to_rgba8();
            if img.width().max(img.height()) > MAX_TEXTURE_SIDE { // larger than a GPU texture can be: scale down
                let s = MAX_TEXTURE_SIDE as f32 / img.width().max(img.height()) as f32;
                img = image::imageops::resize(&img, ((img.width() as f32 * s) as u32).max(1), ((img.height() as f32 * s) as u32).max(1), image::imageops::FilterType::Triangle);
            }
            let size = [img.width() as usize, img.height() as usize];
            let tex = upload(ctx, format!("img:{key}"), &img);
            // A space's round picture: keep its pixels for the screen-resolution round textures (round.rs).
            if key.starts_with("spaceavatar:") { round::register(ctx, tex.id(), &img); }
            Some(Anim { frames: vec![(tex, 1.0)], size, total: 1.0, start: None, bytes: size[0] * size[1] * 4, used: 0 }) }
        Err(e) => {
            let kind = if fmt.is_some() { "unsupported or damaged" } else { "unrecognised (not an image)" };
            crate::cloud::dbg_conn(&format!("image decode failed: {kind}, format {fmt:?}, {} bytes: {e}", bytes.len()));
            None
        }
    }
}

#[derive(Default, Clone)]
pub struct HistoryEntry { pub requested: Option<std::time::Instant>, pub reached_start: bool, pub last_prepended: usize, pub generation: u32,
    /// The first page of the space failed to load (Resp::MessagesFailed); cleared when messages arrive or on retry.
    pub first_failed: bool }

/// What the message list needs to know about paging back in time in one space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryState {
    /// An older page was asked for and hasn't arrived (a request unanswered for 30 s counts as failed).
    pub loading: bool,
    /// The first message of the chat is loaded; no more older pages exist.
    pub reached_start: bool,
    /// How many messages the last older page put in front of the list (keep the scroll position by this many rows).
    pub last_prepended: usize,
    /// Increases with every arrived older page: compare with the value you saw to react to `last_prepended` once.
    pub generation: u32,
}

#[derive(Default)]
pub struct Photos {
    bytes: HashMap<String, Vec<u8>>,
    tex: HashMap<String, Option<egui::TextureHandle>>,
    /// Decrypted message images (by `Attachment.key`), decoded lazily like the photos.
    images: HashMap<String, Vec<u8>>,
    image_tex: HashMap<String, Option<Anim>>,
    /// Last-active time (Unix ms) by person id / my email, from the presence service.
    pub last_active: HashMap<String, i64>,
    /// (status end in Unix ms, custom / out-of-office text) by person id / my email, from `Resp::PresenceDetail`.
    pub presence_info: HashMap<String, (Option<i64>, String)>,
    /// Status by person id for people in no loaded space (directory hits, cloud/presence_sub.rs), kept live by `Resp::Presence`.
    pub outside_status: HashMap<String, String>,
    /// Paging state per space (see `App::history_state`).
    pub history: HashMap<String, HistoryEntry>,
    /// Thumbnail keys whose original file was already asked for.
    full_requested: std::collections::HashSet<String>,
    /// Read marks per space id (see `cloud::ReadMark`), live from acknowledgements.
    pub read_marks: HashMap<String, Vec<crate::cloud::ReadMark>>,
    /// Worker threads decoding message images (started on first use), and per key: the generation of its bytes (bumped when
    /// new bytes arrive, so a late result for old bytes is dropped), the generation being decoded, and the decoded size
    /// (kept after the textures were dropped for the memory budget, so layout doesn't jump when it comes back).
    decoder: Option<Decoder>,
    generation: HashMap<String, u64>,
    decoding: HashMap<String, u64>,
    known_size: HashMap<String, [usize; 2]>,
    /// Texture memory of `image_tex`, and the pass whose finished decodes were last collected.
    tex_bytes: usize,
    collected: Option<u64>,
    /// Profile-picture textures replaced by new bytes: their round textures (round.rs) are dropped on the next `texture` call.
    forget_round: Vec<egui::TextureId>,
}

impl Photos {
    /// Drops the full-resolution image of `key` (bytes and textures) when the viewer closes.
    pub fn release_full(&mut self, key: &str) {
        let k = format!("full:{key}");
        self.images.remove(&k);
        self.drop_decoded(&k);
        self.decoding.remove(&k);
        self.known_size.remove(&k);
        self.full_requested.remove(key);
    }

    pub fn add_images(&mut self, items: Vec<(String, Vec<u8>)>) {
        for (k, b) in items {
            self.drop_decoded(&k);
            self.known_size.remove(&k);
            self.decoding.remove(&k);
            *self.generation.entry(k.clone()).or_insert(0) += 1;
            self.images.insert(k, b);
        }
    }

    fn drop_decoded(&mut self, key: &str) {
        if let Some(Some(a)) = self.image_tex.remove(key) {
            self.tex_bytes = self.tex_bytes.saturating_sub(a.bytes);
            if key.starts_with("spaceavatar:") { self.forget_round.extend(a.frames.iter().map(|f| f.0.id())); }
        }
    }

    /// Takes the finished decodes (once per pass) and keeps texture memory within `IMAGE_TEX_BUDGET`.
    fn collect(&mut self, pass: u64) {
        if self.collected == Some(pass) { return; }
        self.collected = Some(pass);
        let Some(d) = &self.decoder else { return };
        let done: Vec<Decoded> = d.rx.try_iter().collect();
        for r in done {
            if self.decoding.get(&r.key) != Some(&r.generation) { // newer bytes arrived meanwhile, or released
                if let Some(a) = &r.anim { self.forget_round.extend(a.frames.iter().map(|f| f.0.id())); }
                continue
            }
            self.decoding.remove(&r.key);
            if let Some(a) = &r.anim { self.tex_bytes += a.bytes; self.known_size.insert(r.key.clone(), a.size); }
            self.image_tex.insert(r.key, r.anim);
        }
        if self.tex_bytes > IMAGE_TEX_BUDGET {
            // Least recently drawn first; never what was drawn in this or the previous pass.
            let mut old: Vec<(u64, String)> = self.image_tex.iter().filter_map(|(k, a)| a.as_ref().filter(|a| a.used + 1 < pass).map(|a| (a.used, k.clone()))).collect();
            old.sort();
            for (_, k) in old { if self.tex_bytes <= IMAGE_TEX_BUDGET { break } self.drop_decoded(&k); }
        }
    }

    /// The decoded pixel size of an image: known once it was decoded (also after its textures were dropped for the memory
    /// budget). Asks for the decode like `image` does.
    pub fn image_size(&mut self, ctx: &egui::Context, key: &str) -> Option<[usize; 2]> {
        let _ = self.anim(ctx, key);
        self.known_size.get(key).copied()
    }

    /// The encoded bytes of a message image (`key`, or `full:<key>` for the original) for the viewer's Kopieren /
    /// Speichern unter.
    pub fn image_bytes(&self, key: &str) -> Option<&[u8]> { self.images.get(key).map(|b| b.as_slice()) }

    /// The first frame of an inline message image as (texture, [width, height] in pixels); `None` while it isn't
    /// downloaded or can't be decoded. Animated GIFs: use `image_frame` to get the frame for the current time.
    pub fn image(&mut self, ctx: &egui::Context, key: &str) -> Option<(egui::TextureId, [usize; 2])> {
        let a = self.anim(ctx, key)?;
        Some((a.frames[0].0.id(), a.size))
    }

    /// The frame to show at `now` (seconds, any monotonic clock such as egui's `input.time`); loops forever and honours the
    /// frame delays. Schedules the next repaint itself, so call it only for images that are on screen.
    pub fn image_frame(&mut self, ctx: &egui::Context, key: &str, now: f64) -> Option<(egui::TextureId, [usize; 2])> {
        let a = self.anim(ctx, key)?;
        if a.frames.len() < 2 || a.total <= 0.0 { return Some((a.frames[0].0.id(), a.size)); }
        let start = *a.start.get_or_insert(now);
        let t = ((now - start) as f32).rem_euclid(a.total);
        let (mut acc, mut idx) = (0.0, 0);
        for (i, (_, d)) in a.frames.iter().enumerate() { if t < acc + d { idx = i; break } acc += d; idx = i; }
        let left = (acc + a.frames[idx].1 - t).max(0.005);
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(left));
        Some((a.frames[idx].0.id(), a.size))
    }

    /// The decoded image, or `None` while it is being decoded on a worker (the caller shows its loading state, as while the
    /// bytes are still downloading) / can't be decoded. Unit tests decode in place.
    fn anim(&mut self, ctx: &egui::Context, key: &str) -> Option<&mut Anim> {
        let pass = ctx.cumulative_pass_nr();
        self.collect(pass);
        for id in self.forget_round.drain(..) { round::forget(ctx, id); }
        if !self.images.contains_key(key) { return None; }
        if !self.image_tex.contains_key(key) {
            if cfg!(test) {
                let a = decode_anim(ctx, key, &self.images[key]);
                if let Some(a) = &a { self.tex_bytes += a.bytes; self.known_size.insert(key.to_string(), a.size); }
                self.image_tex.insert(key.to_string(), a);
            } else {
                if !self.decoding.contains_key(key) {
                    let generation = self.generation.get(key).copied().unwrap_or(0);
                    let d = self.decoder.get_or_insert_with(Decoder::start);
                    if d.tx.send(Job { key: key.to_string(), generation, bytes: self.images[key].clone(), ctx: ctx.clone() }).is_ok() {
                        self.decoding.insert(key.to_string(), generation);
                    }
                }
                return None;
            }
        }
        let a = self.image_tex.get_mut(key).and_then(|a| a.as_mut())?;
        a.used = pass;
        Some(a)
    }
    pub fn add(&mut self, items: Vec<(String, Vec<u8>)>) {
        for (k, b) in items {
            if let Some(Some(t)) = self.tex.remove(&k) { self.forget_round.push(t.id()); }
            self.bytes.insert(k, b);
        }
    }

    /// The decoded picture for a person id, uuid or email (decoded lazily, cached); `None` if there is none.
    pub fn texture(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureId> {
        let key = if self.bytes.contains_key(key) { key.to_string() } else { key.to_lowercase() };
        for id in self.forget_round.drain(..) { round::forget(ctx, id); }
        if !self.bytes.contains_key(&key) { return None; }
        if !self.tex.contains_key(&key) {
            let t = match image::load_from_memory(&self.bytes[&key]) {
                Ok(img) => {
                    let img = img.to_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    // Mipmapped for the fallback drawing (scaled mesh); the avatars themselves use round.rs textures.
                    let opts = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
                    let t = ctx.load_texture(format!("photo:{key}"), egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), opts);
                    round::register(ctx, t.id(), &img);
                    Some(t)
                }
                Err(e) => { crate::cloud::dbg_conn(&format!("avatar decode failed ({} bytes): {e}", self.bytes[&key].len())); None }
            };
            self.tex.insert(key.clone(), t);
        }
        self.tex[&key].as_ref().map(|t| t.id())
    }

    #[allow(dead_code)] // for the UI: "Vor 3 Stunden aktiv" under a contact's name
    /// "Vor 3 Stunden aktiv"-style text for a last-active time; empty if unknown.
    pub fn last_active_text(&self, key: &str, now_ms: i64) -> String {
        let Some(&ms) = self.last_active.get(key).filter(|m| **m > 0) else { return String::new() };
        let mins = ((now_ms - ms) / 60_000).max(0);
        let (n, unit) = if mins < 60 { (mins, if mins == 1 { "Minute" } else { "Minuten" }) }
            else if mins < 24 * 60 { (mins / 60, if mins / 60 == 1 { "Stunde" } else { "Stunden" }) }
            else { (mins / 1440, if mins / 1440 == 1 { "Tag" } else { "Tagen" }) };
        format!("Vor {n} {unit} aktiv")
    }
}

impl crate::App {
    /// Paging state of `space` for the message list (all false/0 until an older page was requested).
    #[allow(dead_code)]
    pub(crate) fn history_state(&self, space: &str) -> HistoryState {
        match self.photos.history.get(space) {
            Some(h) => HistoryState { loading: h.requested.map_or(false, |t| t.elapsed().as_secs() < 30), reached_start: h.reached_start, last_prepended: h.last_prepended, generation: h.generation },
            None => HistoryState::default(),
        }
    }
    /// Asks for the page of messages before the oldest one loaded (lazy: call it when the user scrolls to the top). No-op while
    /// a page is loading or once the start of the chat was reached. The answer replaces `self.messages[space]` (full list).
    #[allow(dead_code)]
    pub(crate) fn load_older_messages(&mut self, space: &str) {
        let s = self.history_state(space);
        if s.loading || s.reached_start { return }
        if let Some(c) = &self.client {
            self.photos.history.entry(space.to_string()).or_default().requested = Some(std::time::Instant::now());
            c.req(crate::cloud::Req::MessagesBefore(space.to_string()));
        }
    }
    /// Starts loading the original file behind the inline thumbnail `key` (`Attachment.key`); once per key. Only for image
    /// types and files up to ~25 MB. When it arrives, `image_full` / `image_full_frame` return it.
    #[allow(dead_code)]
    pub(crate) fn request_full_image(&mut self, key: &str) {
        if self.photos.full_requested.insert(key.to_string()) {
            if let Some(c) = &self.client { c.req(crate::cloud::Req::FullImage(key.to_string())); }
        }
    }
    /// The full-resolution image once loaded as (texture, pixel size; scaled down only above 8192 px); `None` while it
    /// loads, so the viewer keeps showing the thumbnail scaled up.
    #[allow(dead_code)]
    pub(crate) fn image_full(&mut self, ctx: &egui::Context, key: &str) -> Option<(egui::TextureId, [usize; 2])> { self.photos.image(ctx, &format!("full:{key}")) }
    /// Animated-GIF variant of `image_full` (frame for `now_secs`, schedules its own repaint).
    #[allow(dead_code)]
    pub(crate) fn image_full_frame(&mut self, ctx: &egui::Context, key: &str, now_secs: f64) -> Option<(egui::TextureId, [usize; 2])> { self.photos.image_frame(ctx, &format!("full:{key}"), now_secs) }
    /// Frees the full-resolution image (call when the viewer closes).
    #[allow(dead_code)]
    pub(crate) fn release_full_image(&mut self, key: &str) { self.photos.release_full(key); }

    /// Who has read `space` up to which message (everyone but me). Empty until acknowledgements arrive live.
    #[allow(dead_code)]
    pub(crate) fn read_marks(&self, space: &str) -> &[crate::cloud::ReadMark] { self.photos.read_marks.get(space).map(|v| v.as_slice()).unwrap_or(&[]) }

    /// An inline message image (`Message.images[i].key`) as (texture, [w, h]) once downloaded and decrypted.
    #[allow(dead_code)]
    pub(crate) fn image(&mut self, ctx: &egui::Context, key: &str) -> Option<(egui::TextureId, [usize; 2])> { self.photos.image(ctx, key) }
    /// The banner for a chat with `person`: their own cover if they chose one (`Resp::Covers`), else the global default
    /// (`"@default"`), else `None` (then use the user's own `self.cover`). Pass the person's id and email.
    #[allow(dead_code)]
    pub(crate) fn cover_for(&mut self, ctx: &egui::Context, id: &str, email: &str) -> Option<(egui::TextureId, [usize; 2])> {
        // No "@default" fallback: a person who never chose a cover gets a flat header.
        for k in [id.to_string(), email.to_lowercase()] {
            if k.is_empty() { continue }
            if let Some(t) = self.photos.image(ctx, &format!("cover:{k}")) { return Some(t); }
        }
        None
    }
    /// The custom cover of a group space (`Resp::SpaceCovers`), or `None` when the space has none (then draw no banner).
    /// Not the user's own cover.
    #[allow(dead_code)]
    pub(crate) fn space_cover(&mut self, ctx: &egui::Context, space_id: &str) -> Option<(egui::TextureId, [usize; 2])> {
        self.photos.image(ctx, &format!("spacecover:{space_id}"))
    }
    /// Whether GIFs play by themselves in the chat list (settings key `gif_autoplay`, default true). When false the messages
    /// view should draw the first frame (`App::image`) with a play overlay and only call `image_frame` after a click.
    #[allow(dead_code)]
    pub(crate) fn gif_autoplay(&self) -> bool { self.settings["gif_autoplay"].as_bool().unwrap_or(true) }
    /// The round picture of a space that has one (`Resp::SpaceAvatars`, e.g. a company logo), else `None` (draw the letter).
    #[allow(dead_code)]
    pub(crate) fn space_avatar(&mut self, ctx: &egui::Context, space_id: &str) -> Option<(egui::TextureId, [usize; 2])> {
        self.photos.image(ctx, &format!("spaceavatar:{space_id}"))
    }
    /// Like `image`, but for animated GIFs returns the frame for the current time and schedules the next repaint.
    /// Call it with `ui.input(|i| i.time)` only for images that are visible.
    #[allow(dead_code)]
    pub(crate) fn image_frame(&mut self, ctx: &egui::Context, key: &str, now_secs: f64) -> Option<(egui::TextureId, [usize; 2])> { self.photos.image_frame(ctx, key, now_secs) }

    /// The profile picture of a person (by `Person.id`, uuid or email) as an egui texture, if one was downloaded.
    pub(crate) fn photo(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::TextureId> {
        if key.is_empty() { return None; }
        self.photos.texture(ctx, key)
    }
}

// Regression guard: these accessors are called by the UI; the build fails if one of them disappears or changes signature.
const _: fn(&mut crate::App, &egui::Context, &str) -> Option<(egui::TextureId, [usize; 2])> = crate::App::space_avatar;
const _: fn(&mut crate::App, &egui::Context, &str) -> Option<(egui::TextureId, [usize; 2])> = crate::App::space_cover;
const _: fn(&crate::App, &str) -> HistoryState = crate::App::history_state;
const _: fn(&mut crate::App, &str) = crate::App::load_older_messages;

#[cfg(test)]
mod tests;
