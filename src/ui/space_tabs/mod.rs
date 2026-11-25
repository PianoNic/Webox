//! The chat header's tabs beyond "Messages" (People / Profile / Content / Meetings / +Apps), the pinned-messages popover and
//! the in-conversation search.
//!
//! Strings from the German pack. Pinned messages: balloon 400 wide, radius 5, header 46 pt with a centred bold title, empty
//! state with `message-activity-192-cobalt-lime` + "Messages pinned to this space appear here." Meetings rows 50 pt, radius
//! 25, 32 pt icon disc, Join pill 28 pt. Data is real where the data layer has it: participants (`Space.participants`), files
//! / links derived from the loaded messages (`Message.images`, "📎 name" lines, URLs in the text and link previews),
//! meetings from `App.meetings`. Moderators come from the participants' `roomProperties.isModerator` (cloud/spaces_ext/) and
//! get a "Moderator" tag.
use crate::*;
use super::menu::{self, Align, Entry, Item, Side};

#[derive(Clone, Default)]
struct St { space: String, content: u8, people_q: String, find: bool, find_q: String, find_i: usize, focus_find: bool, pins: bool }
fn st_id() -> egui::Id { egui::Id::new("space-tab-state") }
fn st(ctx: &egui::Context) -> St { ctx.data(|d| d.get_temp::<St>(st_id())).unwrap_or_default() }
fn put(ctx: &egui::Context, s: St) { ctx.data_mut(|d| d.insert_temp(st_id(), s)); }

/// Search-in-space (the header's magnifier): opens / closes the search bar of the open chat.
pub(crate) fn find_toggle(ctx: &egui::Context, space: &str) {
    let mut s = st(ctx);
    if s.space != space { s = St { space: space.into(), ..Default::default() }; }
    s.find = !s.find; s.focus_find = s.find; s.pins = false;
    put(ctx, s);
}
/// Screenshot helper: select the Content sub-tab (0 Files, 1 Links, 2 Whiteboards).
pub(crate) fn set_content(ctx: &egui::Context, space: &str, n: u8) { let mut s = st(ctx); s.space = space.into(); s.content = n; put(ctx, s); }
pub(crate) fn pins_toggle(ctx: &egui::Context, space: &str) {
    let mut s = st(ctx);
    if s.space != space { s = St { space: space.into(), ..Default::default() }; }
    s.pins = !s.pins; s.find = false;
    put(ctx, s);
}

fn day_text(date: &str) -> String {
    if date.is_empty() { String::new() } else if date == system::today() { tr("Today").into() } else if date == system::days_ago(1) { tr("Yesterday").into() } else { date.to_string() }
}
fn when(m: &Message) -> String { [day_text(&m.date), m.time.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ") }

/// `text` with every case-insensitive occurrence of `q` in semibold + `hl` colour.
fn hl_job(text: &str, q: &str, size: f32, col: Color32, hl: Color32, max_rows: usize, wrap: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    let reg = TextFormat { font_id: FontId::proportional(size), color: col, ..Default::default() };
    let bold = TextFormat { font_id: semibold(size), color: hl, ..Default::default() };
    let (lt, lq) = (text.to_lowercase(), q.to_lowercase());
    if lq.is_empty() || lt.len() != text.len() || !lt.contains(&lq) { job.append(text, 0.0, reg); }
    else {
        let mut at = 0;
        while let Some(i) = lt[at..].find(&lq) {
            let (s, e) = (at + i, at + i + lq.len());
            if !text.is_char_boundary(s) || !text.is_char_boundary(e) { break; }
            job.append(&text[at..s], 0.0, reg.clone());
            job.append(&text[s..e], 0.0, bold.clone());
            at = e;
        }
        job.append(&text[at..], 0.0, reg);
    }
    job.wrap = egui::text::TextWrapping { max_width: wrap, max_rows, break_anywhere: false, overflow_character: Some('…') };
    job
}

/// File name from a "📎 name" line.
fn clip_name(line: &str) -> Option<String> { line.find('📎').map(|i| line[i + '📎'.len_utf8()..].trim().to_string()).filter(|s| !s.is_empty()) }

/// `key`: the attachment for downloads ("" = unknown); `msg`: the message to show; `image`: opens in the viewer.
struct FileRow { name: String, who: String, when: String, size: u64, key: String, msg: String, image: bool }
struct LinkRow { url: String, who: String, when: String }

fn files_of(msgs: &[Message], sample: bool) -> Vec<FileRow> {
    let mut v = Vec::new();
    for m in msgs.iter().filter(|m| m.notice.is_empty() && !m.id.starts_with("upload:")) {
        for a in &m.images { v.push(FileRow { name: a.name.clone(), who: m.author.clone(), when: when(m), size: a.size, key: a.key.clone(), msg: m.id.clone(), image: true }); }
        let names: Vec<String> = m.text.lines().filter_map(clip_name).collect();
        for (i, n) in names.iter().enumerate() {
            let att = if m.files.len() == names.len() { m.files.get(i) } else { m.files.iter().find(|a| a.name == *n) };
            // Sample data: the same stand-in key the message card uses (ui/messages/ `file_chip`).
            let key = att.map(|a| a.key.clone()).unwrap_or_else(|| if sample { format!("sample-file:{}:{n}", m.id) } else { String::new() });
            v.push(FileRow { name: n.clone(), who: m.author.clone(), when: when(m), size: att.map_or(0, |a| a.size), key, msg: m.id.clone(), image: false });
        }
    }
    v.reverse(); // newest first
    v
}
fn links_of(msgs: &[Message]) -> Vec<LinkRow> {
    let mut v: Vec<LinkRow> = Vec::new();
    for m in msgs.iter().filter(|m| m.notice.is_empty()) {
        let mut urls: Vec<String> = m.text.split_whitespace().map(|w| w.trim_matches(|c: char| ",;)(<>\"'".contains(c)).trim_end_matches(['.', '!', '?']).to_string())
            .filter(|w| w.starts_with("http://") || w.starts_with("https://") || w.starts_with("www.")).collect();
        for p in &m.previews { if !urls.contains(&p.url) { urls.push(p.url.clone()); } }
        for u in urls { if !v.iter().any(|x| x.url == u) { v.push(LinkRow { url: u, who: m.author.clone(), when: when(m) }); } }
    }
    v.reverse();
    v
}
fn file_icon(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "pdf" => "file-pdf-regular", "doc" | "docx" | "odt" => "file-word-regular", "xls" | "xlsx" | "csv" | "ods" => "file-spreadsheet-regular",
        "ppt" | "pptx" | "odp" => "file-powerpoint-regular", "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" => "file-image-regular",
        "zip" | "rar" | "7z" | "gz" | "tar" => "file-zip-regular", "mp3" | "wav" | "m4a" | "ogg" => "file-audio-regular", "mp4" | "mov" | "mkv" | "avi" => "file-video-regular",
        _ => "file-text-regular",
    }
}
/// The file size text (see `app::files::size_text`).
fn size_text(b: u64) -> String { crate::app::files::size_text(b) }

/// A section's centred empty state: illustration (own colours) + text.
fn empty_state(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, art: Option<&'static str>, text: &str) {
    let w = ui.available_width();
    ui.add_space(48.0);
    if let Some(a) = art {
        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 120.0), Sense::hover());
        ic.art(ui, a, Rect::from_center_size(r.center(), Vec2::splat(120.0)));
    }
    ui.add_space(8.0);
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::hover());
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(15.0), t.text2);
}

mod tabs;
mod find;
