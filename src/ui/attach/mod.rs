//! Attachments in the UI: staging files in the composer (file dialog, drag & drop, pasted images, screen captures),
//! the staged-file bricklets, sending them (encrypted upload, `cloud/files/`), the upload strip with progress / cancel / retry, and
//! the file actions used by message file cards, the "Content" tab and the image viewer (Open, Download, Save as,
//! Show in folder).
//!
//! Staged files are drawn as file bricklets:
//! * size 208 x 60 (100 x 60 for a Giphy; only Giphys hide the file info, images show it).
//! * card = rect with top +8 / right -8 (200 x 52), radius 5, `wx-fileBrickletStaging-background`;
//!   image box = the card's left 80 px, left corners radius 4, right edge square, `wx-fileBrickletStagingImage-background`;
//!   the preview is scaled to fit 66 x 45 keeping the ratio (also up) and centred in the box; without a preview the
//!   file-type SVG goes into the box inset 30 / 5 (20 px wide), `wx-default-text`.
//!   Info: the name 16 px / 400, `wx-headingColor-text`, elided at the right, bottom-aligned in
//!   (box right + 10, top + 9) .. (card right - 20, height / 2 + 2); the size below it, top-aligned, `wx-subheading-text`.
//! * Close: a 12 x 12 button with margins 10, top right; tooltip "Remove file".
//! * The staging area: margins 16 / 0 / 16 / 0, height 68 .. 92, scrolling vertically; a flow layout (margins 4 / 0 / 0 / 4,
//!   horizontal spacing 20, vertical spacing 5).
//! Colours (dark / light): gray-90 #292929 / gray-05 #F7F7F7, gray-70 #545454 / gray-10 #EDEDED, gray-05 / gray-100,
//! gray-20 #DEDEDE / gray-90 #292929, gray-40 #B2B2B2 / gray-70 #545454; `wx-progressbar` gray-80 / gray-20, checked blue-50
//! #00A0D1. Strings: "File: %1, %2", "Remove file", "A message can have at most %1 files", "%1 is too large (file size: %2)",
//! "Uploading <b>{{0}}</b>", "Attachments are not allowed", "You can’t add attachments while editing a message. Try again with a
//! new message", "Open", "Save as", "Show in folder", "Retry", "Download", "Download complete",
//! "Download failed", "Failed!". The upload strip above the composer is Webox's own.
use crate::*;
use crate::app::files::{self as fs_, Phase, Then};
use super::composer::Chip;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Bricklet width with the file info (pt); only Giphys are 100 wide.
pub(super) const CHIP_W: f32 = 208.0;
pub(super) const CHIP_H: f32 = 60.0;
/// Staging area: side margins 16, minimum / maximum height 68 / 92.
pub(super) const AREA_SIDE: f32 = 16.0;
const AREA_MIN_H: f32 = 68.0;
const AREA_MAX_H: f32 = 92.0;
/// Its flow layout: margins left 4 / bottom 4, spacing 20 across and 5 down.
const FLOW_LEFT: f32 = 4.0;
const FLOW_BOTTOM: f32 = 4.0;
const FLOW_HSPACE: f32 = 20.0;
const FLOW_VSPACE: f32 = 5.0;
const STRIP_H: f32 = 40.0; // upload strip height

/// Top-left of each bricklet inside the staging area (FlowLayout: left to right, a new row when the next one does not fit) and
/// the height of the whole content. `avail` is the area's inner width (without the 16 px side margins).
pub(super) fn flow_layout(n: usize, avail: f32) -> (Vec<egui::Vec2>, f32) {
    let (mut x, mut y, mut out) = (FLOW_LEFT, 0.0, Vec::with_capacity(n));
    for _ in 0..n {
        if x > FLOW_LEFT && x + CHIP_W > avail { x = FLOW_LEFT; y += CHIP_H + FLOW_VSPACE; }
        out.push(Vec2::new(x, y));
        x += CHIP_W + FLOW_HSPACE;
    }
    let h = if n == 0 { 0.0 } else { y + CHIP_H + FLOW_BOTTOM };
    (out, h)
}

/// Height the staging area takes in the composer: the content, kept between 68 and 92 (more rows scroll); 0 without files.
pub(super) fn staging_height(n: usize, avail: f32) -> f32 {
    if n == 0 { 0.0 } else { flow_layout(n, avail).1.clamp(AREA_MIN_H, AREA_MAX_H) }
}

/// Height the upload strips of `space` take above the composer (other banners, e.g. the quote, sit above them).
pub(crate) fn upload_strips_height(space: &str) -> f32 {
    let n = fs_::uploads().iter().filter(|u| u.space == space && !matches!(u.phase, Phase::Done | Phase::Cancelled)).count();
    if n == 0 { 0.0 } else { 6.0 + n as f32 * (STRIP_H + 4.0) }
}
pub(crate) const EDIT_REFUSED: &str = "You can’t add attachments while editing a message. Try again with a new message";
const NOT_ALLOWED: &str = "Attachments are not allowed";
/// Pasted images larger than this are not decoded for the bricklet preview (an icon is shown instead).
const PREVIEW_MAX_BYTES: u64 = 30 * 1024 * 1024;

fn staging_bg(light: bool) -> Color32 { if light { hex(0xF7F7F7) } else { hex(0x292929) } }
fn staging_img_bg(light: bool) -> Color32 { if light { hex(0xEDEDED) } else { hex(0x545454) } }
fn default_text(light: bool) -> Color32 { if light { hex(0x121212) } else { hex(0xF7F7F7) } } // wx-default-text
fn heading_text(light: bool) -> Color32 { if light { hex(0x292929) } else { hex(0xDEDEDE) } } // wx-headingColor-text
fn subheading_text(light: bool) -> Color32 { if light { hex(0x545454) } else { hex(0xB2B2B2) } } // wx-subheading-text

/// The preview size: scaled to fit 66 x 45 keeping the ratio, up or down.
pub(super) fn preview_size(w: f32, h: f32) -> Vec2 {
    if w <= 0.0 || h <= 0.0 { return Vec2::ZERO; }
    let s = (66.0 / w).min(45.0 / h);
    Vec2::new(w * s, h * s)
}
fn bar_track(light: bool) -> Color32 { if light { hex(0xDEDEDE) } else { hex(0x3B3B3B) } }
const BAR_FILL: Color32 = hex(0x00A0D1);

/// Local attachment keys (pending uploads, sample-mode files) -> the file on disk.
static LOCAL: Mutex<Option<HashMap<String, PathBuf>>> = Mutex::new(None);
/// What is needed to send an upload again after it failed ("Retry").
#[derive(Clone)]
struct Retry { space: String, files: Vec<PathBuf>, text: String, markdown: bool, mentions: Vec<crate::richtext::MentionRef>, parent: Option<String> }
static RETRY: Mutex<Option<HashMap<String, Retry>>> = Mutex::new(None);
/// Downloads whose end was already announced.
static ANNOUNCED: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

pub(crate) fn local_path(key: &str) -> Option<PathBuf> { LOCAL.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(key).cloned()) }
fn set_local(key: &str, p: &Path) { LOCAL.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key.into(), p.into()); }

pub(crate) fn file_icon(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "pdf" => "file-pdf-regular", "doc" | "docx" | "odt" | "rtf" => "file-word-regular", "xls" | "xlsx" | "csv" | "ods" => "file-spreadsheet-regular",
        "ppt" | "pptx" | "odp" => "file-powerpoint-regular", "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" => "file-image-regular",
        "zip" | "rar" | "7z" | "gz" | "tar" => "file-zip-regular", "mp3" | "wav" | "m4a" | "ogg" => "file-audio-regular", "mp4" | "mov" | "mkv" | "avi" | "webm" => "file-video-regular",
        "js" | "ts" | "rs" | "py" | "c" | "cpp" | "h" | "java" | "json" | "xml" | "html" | "css" => "file-code-regular",
        _ => "file-text-regular",
    }
}

/// Text of the upload strip, the pack's "Uploading <b>{{0}}</b>": the text before the bold name, the name, the text after it.
fn uploading_parts(names: &[String]) -> (String, String, String) {
    let n = match names.len() { 0 => String::new(), 1 => names[0].clone(), k => format!("{} (+{})", names[0], k - 1) };
    let pat = tr("Uploading <b>{{0}}</b>");
    let (before, after) = pat.split_once("<b>{{0}}</b>").unwrap_or(("", pat));
    (before.to_string(), n, after.to_string())
}

/// Percentage of an upload / download (0..=100).
pub(crate) fn percent(done: u64, total: u64) -> u32 { if total == 0 { 0 } else { ((done as f64 / total as f64) * 100.0).round().clamp(0.0, 100.0) as u32 } }

/// Simulated progress of a sample-mode upload (no service): 2.5 s from start to done (screenshot runs: 20 s, so the strip is caught
/// half-way).
pub(crate) fn sample_progress(started: f64, now: f64) -> f32 {
    let secs = if std::env::var_os("WEBOX_SHOT_COMPOSER").is_some_and(|v| v == "upload") { 20.0 } else { 2.5 };
    ((now - started) / secs).clamp(0.0, 1.0) as f32
}

/// Ctrl+V went down since the last frame (`paste_key_poll` reads the key once per frame, so the edge is never stale; a press
/// while the window is not focused does not count).
pub(super) fn paste_key_edge(ctx: &egui::Context) -> bool { ctx.data(|d| d.get_temp::<u64>(egui::Id::new("paste-key-edge"))) == Some(ctx.cumulative_pass_nr()) }
pub(crate) fn paste_key_poll(ctx: &egui::Context, window_focused: bool) -> bool {
    let (edge, pass) = (paste_key_down_edge(ctx) && window_focused, ctx.cumulative_pass_nr());
    if edge { ctx.data_mut(|d| d.insert_temp(egui::Id::new("paste-key-edge"), pass)); }
    edge
}
/// egui-winit swallows Ctrl+V when the clipboard has no text (no `Event::Paste`, no key event), so the key state is read from
/// Windows directly (edge-detected).
#[cfg(windows)]
fn paste_key_down_edge(_ctx: &egui::Context) -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
    static WAS: AtomicBool = AtomicBool::new(false);
    let down = |k: i32| unsafe { (GetAsyncKeyState(k) as u16 & 0x8000) != 0 };
    let now = down(VK_CONTROL.0 as i32) && down(0x56); // 'V'
    !WAS.swap(now, Ordering::SeqCst) && now
}
/// Elsewhere egui-winit drops the press the same way, but the release of V still arrives (see `PasteKeys`).
#[cfg(not(windows))]
fn paste_key_down_edge(ctx: &egui::Context) -> bool {
    let id = egui::Id::new("paste-keys");
    let mut s = ctx.data(|d| d.get_temp::<PasteKeys>(id)).unwrap_or_default();
    let edge = ctx.input(|i| s.frame(&i.events, i.modifiers.command, i.key_down(Key::V)));
    ctx.data_mut(|d| d.insert_temp(id, s));
    edge
}

/// Image paste without a key press: a release of V whose press egui never saw (egui-winit dropped it as a paste command), with
/// Ctrl held since the last V release and no text paste since then (that press was a text paste). Fed once per frame.
/// Not needed on Windows (the key state is read from the OS there); its unit test runs everywhere.
#[derive(Clone, Copy, Default, Debug)]
#[cfg(any(not(windows), test))]
pub(super) struct PasteKeys { v_down: bool, cmd: bool, text_paste: bool }
#[cfg(any(not(windows), test))]
impl PasteKeys {
    /// `cmd_now` / `v_down_now`: Ctrl and V held at the end of this frame.
    pub(super) fn frame(&mut self, events: &[egui::Event], cmd_now: bool, v_down_now: bool) -> bool {
        let is_v = |k: &Key, p: &Option<Key>| *k == Key::V || *p == Some(Key::V);
        let (mut edge, mut pressed) = (false, self.v_down);
        for e in events {
            match e {
                egui::Event::Paste(_) => self.text_paste = true,
                egui::Event::Key { key, physical_key, pressed: down, modifiers, .. } => {
                    self.cmd |= modifiers.command;
                    if !is_v(key, physical_key) { continue; }
                    if *down { pressed = true; continue; }
                    if !pressed && self.cmd && !self.text_paste { edge = true; }
                    (pressed, self.cmd, self.text_paste) = (false, false, false);
                }
                _ => {}
            }
        }
        self.cmd |= cmd_now;
        self.v_down = v_down_now;
        edge
    }
}

/// The clipboard's image as PNG (arboard reads CF_DIBV5 / PNG on Windows, image/png on X11). On Wayland the native clipboard
/// first (data-control, `platform::linux::clipboard`); arboard's X11 read through XWayland only where that protocol is missing.
pub(super) fn clipboard_png() -> Option<Vec<u8>> {
    #[cfg(target_os = "linux")]
    if crate::platform::linux::session() == crate::platform::linux::Session::Wayland {
        if let Some(png) = crate::platform::linux::clipboard::image_png() { return png; }
    }
    let mut cb = arboard::Clipboard::new().ok()?;
    let img = cb.get_image().ok()?;
    let rgba = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba).write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}
/// Pasted images, per space, read off the UI thread (the X11 clipboard can take seconds to answer, a Wayland
/// source app as long as it needs to write the image).
static PASTED: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());
/// Reads the clipboard's image for `space` on a thread; `take_pasted` hands it over.
pub(super) fn clipboard_png_async(ctx: &egui::Context, space: &str) {
    let (ctx, space) = (ctx.clone(), space.to_string());
    std::thread::Builder::new().name("webox-paste".into()).spawn(move || {
        let Some(png) = clipboard_png() else { return };
        PASTED.lock().unwrap_or_else(|e| e.into_inner()).push((space, png));
        ctx.request_repaint();
    }).ok();
}
pub(super) fn take_pasted(space: &str) -> Option<Vec<u8>> {
    let mut d = PASTED.lock().unwrap_or_else(|e| e.into_inner());
    let i = d.iter().position(|(s, _)| s == space)?;
    Some(d.remove(i).1)
}

impl App {
    /// The composer's "Attach a file" (toolbar, Ctrl+O, the welcome banner's "Share a file"): the open-file dialog;
    /// the chosen files are staged by the composer of that space.
    pub(crate) fn attach_pick(&mut self, space: &str, now: f64) {
        if matches!(self.compose, Compose::Edit { .. }) { self.toast(tr(EDIT_REFUSED), now); return; }
        if !fs_::uploads_allowed() { self.toast(tr(NOT_ALLOWED), now); return; }
        if self.shot.is_some() { return; } // screenshot runs never open dialogs
        fs_::pick_files_async(space);
    }

    /// Adds files to the staged bricklets: folders are skipped, the size limit and the files-per-message limit are checked with
    /// the pack's messages; an image also gets its preview.
    pub(super) fn stage_paths(&mut self, chips: &mut Vec<Chip>, paths: Vec<PathBuf>, edit: bool, now: f64) {
        if paths.is_empty() { return; }
        if edit { self.toast(tr(EDIT_REFUSED), now); return; }
        if !fs_::uploads_allowed() { self.toast(tr(NOT_ALLOWED), now); return; }
        for p in paths {
            let Ok(meta) = std::fs::metadata(&p) else { continue };
            if meta.is_dir() || chips.iter().any(|c| c.path == p) { continue; }
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if chips.len() >= fs_::MAX_FILES { self.toast(crate::trq!("A message can have at most %1 files", fs_::MAX_FILES), now); break; }
            if meta.len() > fs_::limit_bytes() { self.toast(crate::trq!("%1 is too large (file size: %2)", name, fs_::size_text(meta.len())), now); continue; }
            let head = { use std::io::Read; let mut b = [0u8; 64]; let n = std::fs::File::open(&p).and_then(|mut f| f.read(&mut b)).unwrap_or(0); b[..n].to_vec() };
            let image = fs_::mime_of(&name, &head).starts_with("image/");
            if image && meta.len() <= PREVIEW_MAX_BYTES {
                if let Ok(b) = std::fs::read(&p) { self.photos.add_images(vec![(staged_key(&p), b)]); }
            }
            chips.push(Chip { name, size: meta.len(), path: p, image });
        }
    }

    /// Sends files at once as a message of their own, with the same checks as staging them (GIF Extras' "Send favorites on
    /// click" for a kept picture); the composer's own text and staged files stay.
    pub(crate) fn send_files_now(&mut self, space: &str, paths: Vec<PathBuf>, now: f64) {
        let edit = matches!(self.compose, Compose::Edit { .. });
        let mut chips = Vec::new();
        self.stage_paths(&mut chips, paths, edit, now);
        if !chips.is_empty() { self.send_staged(space, chips, String::new(), Vec::new(), now); }
    }

    /// A pasted image / screen capture: written to the app's staging folder under its own name, then staged like a file.
    pub(super) fn stage_bytes(&mut self, chips: &mut Vec<Chip>, name: &str, bytes: &[u8], edit: bool, now: f64) {
        // a temporary file in this user's private folder, removed after sending
        let Ok(dir) = fs_::private_tmp("staged").map(|d| d.join(crate::jose::uuid())) else { return };
        if std::fs::create_dir_all(&dir).is_err() { return; }
        let p = dir.join(fs_::safe_name(name));
        if std::fs::write(&p, bytes).is_ok() { self.stage_paths(chips, vec![p], edit, now); }
    }

    /// The staging area in `area` (already without the 16 px side margins, `staging_height` high): the bricklets flow left to
    /// right in rows and scroll vertically when they need more than 92. Returns the index whose remove button was clicked.
    pub(super) fn draw_chips(&mut self, ui: &mut egui::Ui, t: &Th, area: Rect, chips: &[Chip]) -> Option<usize> {
        let (pos, content_h) = flow_layout(chips.len(), area.width());
        let mut remove = None;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).id_salt("staged-files"));
        egui::ScrollArea::vertical().id_salt("stagedFilesScrollArea").max_height(area.height()).auto_shrink([false, true]).show(&mut child, |ui| {
            let (full, _) = ui.allocate_exact_size(Vec2::new(area.width(), content_h), Sense::hover());
            for (i, ch) in chips.iter().enumerate() {
                let r = Rect::from_min_size(full.min + pos[i], Vec2::new(CHIP_W, CHIP_H));
                if self.draw_bricklet(ui, t, r, ch, i) { remove = Some(i); }
            }
        });
        remove
    }

    /// One file bricklet (208 x 60) at `r`; true when its close button was clicked.
    fn draw_bricklet(&mut self, ui: &mut egui::Ui, t: &Th, r: Rect, ch: &Chip, i: usize) -> bool {
        let light = super::menu::light(t);
        let p = ui.painter().clone();
        let card = Rect::from_min_max(r.min + Vec2::new(0.0, 8.0), pos2(r.right() - 8.0, r.bottom()));
        p.rect_filled(card, 5.0, staging_bg(light));
        let ibox = Rect::from_min_size(card.min, Vec2::new(80.0, card.height()));
        p.rect_filled(ibox, egui::CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, staging_img_bg(light));
        // a video: its first frame (decoded on a worker, `messages::video_poster`) as the preview, with a small play mark
        let video = !ch.image && crate::media::video::format::is_video("", &ch.name);
        let prev = if ch.image { self.image(ui.ctx(), &staged_key(&ch.path)) }
            else if video { super::messages::video_poster(ui.ctx(), &ch.path).map(|(t, s, _)| (t, s)) } else { None };
        match prev {
            Some((tid, [iw, ih])) => {
                let ir = Rect::from_center_size(ibox.center(), preview_size(iw as f32, ih as f32));
                p.with_clip_rect(ibox.intersect(p.clip_rect())).image(tid, ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                if video {
                    p.circle_filled(ibox.center(), 11.0, Color32::from_black_alpha(150));
                    self.icons.paint(ui, "play-filled", Rect::from_center_size(ibox.center() + Vec2::new(1.0, 0.0), Vec2::splat(11.0)), Color32::WHITE);
                }
            }
            // The file-type SVG in the box inset 30 / 5: 20 px wide.
            None => self.icons.paint(ui, file_icon(&ch.name), Rect::from_center_size(ibox.center(), Vec2::splat(20.0)), default_text(light)),
        }
        // Name (bottom-aligned, elided at the right) and size (top-aligned) in 16 px, between the box + 10 and the card's right - 20.
        let (tx, tw) = (ibox.right() + 9.0, card.right() - 20.0 - (ibox.right() + 9.0));
        let font = F::Title.font();
        let one_line = |s: &str, c: Color32| {
            let mut job = egui::text::LayoutJob::single_section(s.to_string(), egui::TextFormat::simple(font.clone(), c));
            job.wrap = egui::text::TextWrapping { max_width: tw, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            ui.fonts(|f| f.layout_job(job))
        };
        let name = one_line(&ch.name, heading_text(light));
        p.galley(pos2(tx, r.top() + 33.0 - name.size().y), name, heading_text(light));
        let size = one_line(&fs_::size_text(ch.size), subheading_text(light));
        p.galley(pos2(tx, r.top() + 32.0), size, subheading_text(light));
        let body = ui.interact(card, ui.id().with(("staged-file", i)), Sense::hover());
        // Accessible name only (no tooltip on the bricklet): "File: %1, %2".
        body.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, crate::trq!("File: %1, %2", ch.name, fs_::size_text(ch.size))));
        // Close button: 12 x 12 at the top-right margin 10 (inside the card's top-right corner).
        let xr = Rect::from_min_size(pos2(r.right() - 10.0 - 12.0, r.top() + 10.0), Vec2::splat(12.0));
        let xs = ui.interact(xr, ui.id().with(("staged-x", i)), Sense::click());
        if xs.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        self.icons.paint(ui, "cancel-bold", xr, default_text(light));
        xs.on_hover_text(tr("Remove file")).clicked()
    }

    /// Send: the staged files (with the composer's text and mentions) go out as one share activity. A pending message appears in
    /// the chat at once; the strip above the composer shows the upload.
    pub(super) fn send_staged(&mut self, space: &str, chips: Vec<Chip>, text: String, mentions: Vec<crate::richtext::MentionRef>, now: f64) {
        let mode = std::mem::take(&mut self.compose);
        let parent = if let Compose::Reply { root, .. } = &mode { Some(root.clone()) } else { None };
        let names: Vec<String> = chips.iter().map(|c| c.name.clone()).collect();
        let total: u64 = chips.iter().map(|c| c.size).sum();
        let (id, _) = fs_::upload_begin(space, names, total, now);
        let mut m = Message { id: id.clone(), author: self.me.name.clone(), email: self.me.email.clone(), time: "now".into(), date: system::today(), mine: true, parent: parent.clone().unwrap_or_default(), ..Default::default() };
        let mut lines: Vec<String> = if text.trim().is_empty() { vec![] } else { vec![text.trim().to_string()] };
        for (i, c) in chips.iter().enumerate() {
            let key = format!("local:{id}:{i}");
            set_local(&key, &c.path);
            let mime = fs_::mime_of(&c.name, &[]);
            if c.image {
                if let Ok(b) = std::fs::read(&c.path) { self.photos.add_images(vec![(key.clone(), b)]); }
                let (w, h) = image::image_dimensions(&c.path).unwrap_or((0, 0));
                m.images.push(cloud::Attachment { name: c.name.clone(), mime, size: c.size, key, width: w, height: h, giphy: None });
            } else {
                lines.push(format!("📎 {}", c.name));
                m.files.push(cloud::Attachment { name: c.name.clone(), mime, size: c.size, key, ..Default::default() });
            }
        }
        m.text = lines.join("\n");
        let list = self.messages.entry(space.to_string()).or_default();
        let at = if m.parent.is_empty() { list.len() } else { list.iter().rposition(|x| x.id == m.parent || x.parent == m.parent).map_or(list.len(), |i| i + 1) };
        list.insert(at, m);
        let files: Vec<PathBuf> = chips.into_iter().map(|c| c.path).collect();
        let markdown = self.composer_is_markdown();
        if self.client.is_some() {
            let r = Retry { space: space.into(), files, text, markdown, mentions, parent };
            RETRY.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id.clone(), r.clone());
            self.request_upload(&id, r);
        }
        // Sample data: `upload_strip` simulates the progress; the pending message stays as the sent one.
    }

    fn request_upload(&mut self, id: &str, r: Retry) {
        if let Some(c) = &self.client {
            c.req(Req::SendFiles(cloud::SendFiles { space: r.space, upload: id.to_string(), files: r.files, text: r.text, markdown: r.markdown, mentions: r.mentions, parent: r.parent }));
        }
    }

    /// The uploads of this space above the composer (floating, the composer does not grow): "Uploading <b>name</b>", a progress
    /// bar, the percentage and a cancel button; a failed one offers "Retry". Finished uploads leave; the reloaded chat shows the
    /// real message.
    pub(crate) fn upload_strip(&mut self, ui: &mut egui::Ui, t: &Th, composer: Rect, space: &str, now: f64) {
        let light = super::menu::light(t);
        let ups: Vec<fs_::Upload> = fs_::uploads().into_iter().filter(|u| u.space == space).collect();
        let mut y = composer.top() - 6.0;
        for u in ups.iter().rev() {
            // Sample data: no service, the progress is simulated.
            let (sent, phase) = if self.client.is_none() && matches!(u.phase, Phase::Preparing | Phase::Uploading) {
                let k = sample_progress(u.started, now);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
                ((u.total as f32 * k) as u64, if k >= 1.0 { Phase::Done } else { Phase::Uploading })
            } else { (u.sent, u.phase.clone()) };
            match &phase {
                Phase::Done => { self.upload_finished(space, &u.id); continue; }
                Phase::Cancelled => {
                    fs_::upload_remove(&u.id);
                    if let Some(l) = self.messages.get_mut(space) { l.retain(|m| m.id != u.id); }
                    continue;
                }
                _ => {}
            }
            let r = Rect::from_min_max(pos2(composer.left() + 20.0, y - STRIP_H), pos2(composer.right() - 20.0, y));
            y -= STRIP_H + 4.0;
            let id = egui::Id::new(("upload-strip", &u.id));
            crate::ui::type_focus::passive(ui.ctx(), egui::LayerId::new(egui::Order::Foreground, id));
            egui::Area::new(id).fixed_pos(r.min).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
                let (r, _) = ui.allocate_exact_size(r.size(), Sense::hover());
                let p = ui.painter();
                p.rect_filled(r, 8.0, if light { Color32::WHITE } else { hex(0x1F1F1F) });
                p.rect_stroke(r, 8.0, Stroke::new(1.0_f32, if light { hex(0xDEDEDE) } else { hex(0x3B3B3B) }), egui::StrokeKind::Inside);
                self.icons.paint(ui, "attachment-regular", Rect::from_center_size(pos2(r.left() + 18.0, r.center().y), Vec2::splat(16.0)), t.text2);
                let failed = if let Phase::Failed(e) = &phase { Some(e.clone()) } else { None };
                let (before, name, rest) = uploading_parts(&u.names);
                let tx = r.left() + 36.0;
                let right_w = if failed.is_some() { 150.0 } else { 220.0 };
                let mut job = egui::text::LayoutJob::default();
                if failed.is_none() && !before.is_empty() { job.append(&before, 0.0, egui::TextFormat::simple(F::Secondary.font(), t.text1)); }
                job.append(&name, 0.0, egui::TextFormat::simple(F::Secondary.semibold(), t.text1));
                if let Some(e) = &failed { job.append(&format!(" – {}", crate::trq!("Failed: %1", e)), 0.0, egui::TextFormat::simple(F::Secondary.font(), hex(0xFC8B98))); }
                else { job.append(&rest, 0.0, egui::TextFormat::simple(F::Secondary.font(), t.text1)); }
                job.wrap = egui::text::TextWrapping { max_width: r.width() - 36.0 - right_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let g = ui.fonts(|f| f.layout_job(job));
                ui.painter().galley(pos2(tx, r.center().y - g.size().y / 2.0), g, t.text1);
                let xr = Rect::from_center_size(pos2(r.right() - 20.0, r.center().y), Vec2::splat(24.0));
                let xs = ui.interact(xr, id.with("x"), Sense::click());
                if xs.hovered() { ui.painter().circle_filled(xr.center(), 12.0, t.hover); }
                self.icons.paint(ui, "cancel-regular", Rect::from_center_size(xr.center(), Vec2::splat(12.0)), t.text1);
                if let Some(_e) = failed {
                    let rr = Rect::from_min_max(pos2(xr.left() - 110.0, r.top() + 7.0), pos2(xr.left() - 8.0, r.bottom() - 7.0));
                    let rs = ui.interact(rr, id.with("retry"), Sense::click());
                    ui.painter().rect_stroke(rr, 13.0, Stroke::new(1.0_f32, t.text2), egui::StrokeKind::Inside);
                    if rs.hovered() { ui.painter().rect_filled(rr, 13.0, t.hover); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                    ui.painter().text(rr.center(), egui::Align2::CENTER_CENTER, tr("Retry"), F::Secondary.semibold(), t.text1);
                    if rs.clicked() { self.upload_retry(&u.id, space, now); }
                    if xs.on_hover_text(tr("Close")).clicked() {
                        fs_::upload_remove(&u.id);
                        if let Some(l) = self.messages.get_mut(space) { l.retain(|m| m.id != u.id); }
                    }
                } else {
                    let pct = percent(sent, u.total);
                    let br = Rect::from_min_max(pos2(xr.left() - 170.0, r.center().y - 2.0), pos2(xr.left() - 56.0, r.center().y + 2.0));
                    ui.painter().rect_filled(br, 2.0, bar_track(light));
                    ui.painter().rect_filled(Rect::from_min_size(br.min, Vec2::new(br.width() * pct as f32 / 100.0, br.height())), 2.0, BAR_FILL);
                    let label = if matches!(phase, Phase::Sharing) { "100 %".to_string() } else { format!("{pct} %") };
                    ui.painter().text(pos2(xr.left() - 10.0, r.center().y), egui::Align2::RIGHT_CENTER, label, F::Caption.font(), t.text2);
                    if xs.on_hover_text(tr("Cancel")).clicked() { fs_::upload_cancel(&u.id); }
                }
            });
        }
    }

    fn upload_finished(&mut self, space: &str, id: &str) {
        fs_::upload_remove(id);
        RETRY.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|m| m.remove(id));
        if self.client.is_some() {
            // The service's own copy arrives with the reload (Req::Messages after the share); drop the pending one.
            if let Some(l) = self.messages.get_mut(space) { l.retain(|m| m.id != id); }
        } else if let Some(l) = self.messages.get_mut(space) {
            let n = l.len();
            if let Some(m) = l.iter_mut().find(|m| m.id == id) { m.id = format!("{space}-local{n}-files"); m.rev = 0; }
        }
    }

    fn upload_retry(&mut self, id: &str, space: &str, now: f64) {
        let r = RETRY.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(id));
        fs_::upload_remove(id);
        let Some(r) = r else { return };
        let total: u64 = r.files.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        let names = r.files.iter().map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).collect();
        let (nid, _) = fs_::upload_begin(space, names, total, now);
        if let Some(m) = self.messages.get_mut(space).and_then(|l| l.iter_mut().find(|m| m.id == id)) { m.id = nid.clone(); m.rev = 0; }
        RETRY.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(nid.clone(), r.clone());
        self.request_upload(&nid, r);
    }

}

fn cloud_finish(p: &Path, then: Then) { match then { Then::Open => fs_::open_file(p), Then::ShowInFolder => fs_::show_in_folder(p), Then::Nothing | Then::Play => {} } }

/// Texture key of a staged image's preview.
fn staged_key(p: &Path) -> String { format!("staged:{}", p.display()) }

#[cfg(test)]
mod tests;

mod receive;
