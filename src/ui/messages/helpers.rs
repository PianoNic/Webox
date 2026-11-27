//! Small helpers of the message list: links, reaction picker state, read marks, local time and day labels, image sizes.
use super::*;

/// Opens a link in the default browser, only for http / https (and "www." completed to https).
pub(super) fn open_web(u: &str) { if let Some(u) = web_url(u) { system::open(&u); } }

/// The URL to open for a clicked link: http(s) as is, "www." completed to https, anything else (file:, javascript:, UNC paths, ...) nothing.
pub(super) fn web_url(u: &str) -> Option<String> {
    let u = if u.starts_with("www.") { format!("https://{u}") } else { u.to_string() };
    (u.starts_with("https://") || u.starts_with("http://")).then_some(u)
}

/// True while the shared emoji picker is open on behalf of a reaction to message `msg` (the composer must not draw it then).
pub(super) fn react_picker_for(ctx: &egui::Context, msg: &str) -> bool {
    ctx.data(|d| d.get_temp::<String>(Id::new("react-picker-target"))).as_deref() == Some(msg) && ctx.data(|d| d.get_temp::<bool>(Id::new("emoji-open"))).unwrap_or(false)
}
/// Whether the picker is currently serving a reaction (any message).
pub(crate) fn react_picker_active(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<String>(Id::new("react-picker-target"))).is_some() && ctx.data(|d| d.get_temp::<bool>(Id::new("emoji-open"))).unwrap_or(false)
}

/// Row counter of the current frame (reset by `message_list`); `next_seq` numbers a row, `cur_seq` is that number for the helpers
/// drawing parts of the same row.
pub(super) fn next_seq(ctx: &egui::Context) -> u32 {
    ctx.data_mut(|d| { let v = d.get_temp_mut_or_insert_with(Id::new("msg-seq"), || 0u32); *v += 1; let n = *v; d.insert_temp(Id::new("msg-seq-cur"), n); n })
}
pub(crate) fn cur_seq(ctx: &egui::Context) -> u32 { ctx.data(|d| d.get_temp::<u32>(Id::new("msg-seq-cur"))).unwrap_or(0) }

/// What a reply's quote says about the quoted message: the author line, the summary of what it shared ("Shared a file: …", see
/// `cloud::Shared`) and its text (whitespace collapsed).
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub(crate) struct QuoteRef { pub author: String, pub shared: Option<cloud::Shared>, pub text: String }

/// The message a reply quotes, newest first among the messages above `from`:
///   * a quote of shared files (`shared`): a message of the quote's author that shared that file (by name) / that many files,
///     else anybody's;
///   * a quote with text: one whose text equals the quoted text (whitespace-insensitive), else one that contains it;
///   * only the author (a picture quoted by an older Webox): the author's last message that shared something.
pub(super) fn jump_target(msgs: &[Message], from: &str, q: &QuoteRef) -> Option<String> {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let upto = msgs.iter().position(|m| m.id == from).unwrap_or(msgs.len());
    let above = || msgs[..upto].iter().rev().filter(|m| m.notice.is_empty());
    let author = q.author.trim();
    let by = |m: &Message| !author.is_empty() && m.author.trim().eq_ignore_ascii_case(author);
    let pick = |hit: &dyn Fn(&Message) -> bool| above().find(|m| by(m) && hit(m)).or_else(|| above().find(|m| hit(m))).map(|m| m.id.clone());
    let text = norm(&q.text);
    if let Some(s) = &q.shared {
        let has = |m: &Message| { let names = cloud::shared_names(m); match (s, s.name()) { (_, Some(n)) => names.iter().any(|x| x == n), (cloud::Shared::Files(k), None) => names.len() == *k as usize, _ => false } };
        if let Some(id) = pick(&has) { return Some(id); }
    }
    if text.is_empty() {
        return if q.shared.is_none() { above().find(|m| by(m) && !cloud::shared_names(m).is_empty()).map(|m| m.id.clone()) } else { None };
    }
    above().find(|m| norm(&m.text) == text).or_else(|| above().find(|m| norm(&m.text).contains(&text))).map(|m| m.id.clone())
}

/// Records a laid-out list row (message, notice, join/leave row) for the scroll anchor: `(id, top, bottom)` in content space
/// (relative to the list's first row), in list order.
pub(super) fn record_row(ctx: &egui::Context, id: &str, r: Rect) {
    let top0 = ctx.data(|d| d.get_temp::<f32>(Id::new("msg-top0"))).unwrap_or(0.0);
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<(String, f32, f32)>>(Id::new("msg-rowspans")).push((id.to_string(), r.top() - top0, r.bottom() - top0)));
}

/// The scroll anchor: the first row that is (partly) visible at offset `off`, as (id, content-space top). Its on-screen y is
/// `top - off`; keeping that while rows above it change height keeps what you look at in place.
pub(super) fn pick_anchor(rows: &[(String, f32, f32)], off: f32) -> Option<(String, f32)> {
    rows.iter().find(|(_, _, b)| *b > off + 0.5).map(|(id, t, _)| (id.clone(), *t))
}

/// What the message list does after laying out a frame (see `scroll_fix`).
#[derive(Debug, Default, PartialEq)]
pub(super) struct ScrollFix {
    /// Offset for the next pass (the anchor moved by this frame's height changes).
    pub set: Option<f32>,
    /// The frame was laid out with an offset that no longer fits (pinned list grew / shrank, anchor moved, first frame of a
    /// chat): throw it away and lay it out again before anything is shown (`Context::request_discard`).
    pub redo: bool,
    /// The anchor for the next frame: the first visible row and its content-space top.
    pub anchor: Option<(String, f32)>,
}

/// One frame of the message list's scroll bookkeeping. `rows` = this frame's rows (id, top, bottom), `prev` = last frame's
/// anchor, `off` / `max` = this frame's offset and maximum, `applied` = the offset it was laid out with when pinned,
/// `pinned` = it stays at the bottom, `explicit` = an offset set on purpose this frame (scrollbar, jump to a message) that
/// must not be corrected, `first` = the chat's first frame with content.
/// - Pinned to the bottom: nothing to correct, but a frame laid out with the previous maximum shows the content shifted when
///   the list's height changed (late images, link previews, "Seen by", a reload replacing the list): redo it.
/// - Scrolled up: the first visible row keeps its on-screen y while rows above it change height (older pages, the spinner row,
///   join/leave and date rows, pictures); the frame is redone with the corrected offset, so the jump is never shown.
#[allow(clippy::too_many_arguments)]
pub(super) fn scroll_fix(rows: &[(String, f32, f32)], prev: Option<&(String, f32)>, off: f32, max: f32, applied: f32, pinned: bool, explicit: bool, first: bool) -> ScrollFix {
    if pinned {
        let redo = first || (!explicit && (max - applied).abs() > 0.5);
        return ScrollFix { set: None, redo, anchor: pick_anchor(rows, max) };
    }
    if !explicit {
        if let Some((aid, ay)) = prev {
            if let Some((_, ny, _)) = rows.iter().find(|(i, _, _)| i == aid) {
                if let Some(to) = anchored_offset(off, *ay, *ny) {
                    let to = to.min(max);
                    return ScrollFix { set: Some(to), redo: true, anchor: Some((aid.clone(), *ny)) };
                }
            }
        }
    }
    ScrollFix { set: None, redo: first, anchor: pick_anchor(rows, off) }
}

/// Keeps the first message where it was on screen when content above it changes height: `prev_y` / `new_y` are its content-space
/// positions in the previous / current layout. `Some(new offset)` when the offset has to move (by the difference, never below 0).
pub(super) fn anchored_offset(off: f32, prev_y: f32, new_y: f32) -> Option<f32> {
    let shift = new_y - prev_y;
    (shift.abs() > 0.5).then(|| (off + shift).max(0.0))
}

/// When to ask for the page before the oldest loaded message: only after the chat has settled (>= 3 frames with content) and either
/// the user has scrolled up within one screen of the top (never while the list is pinned to the bottom, whatever the offset says
/// during the first frames), or the whole chat is shorter than the view (fill it up). Never while loading / after the start.
#[allow(clippy::too_many_arguments)]
/// `settled`: the last older page has been laid out and anchored for a few frames (no request on the strength of an offset
/// that is about to be corrected).
pub(super) fn should_request_older(seen_frames: u32, pinned: bool, off: f32, view_h: f32, max: f32, loading: bool, reached: bool, has_msgs: bool, settled: bool) -> bool {
    if seen_frames < 3 || loading || reached || !has_msgs || !settled { return false; }
    (!pinned && off < view_h) || max < 0.5
}

/// The readers of "Seen by": (lower-case email, person id, name hint), newest reader first; never me.
/// The banner belongs to the conversation's newest readable activity, whoever wrote it - not to my newest message: every
/// participant but me whose read time is at or after it is a reader, sorted by read time, newest first.
/// Here the anchor is the newest list entry that is a message (not a system notice); a person is a reader when their read
/// mark sits at or after the anchor in the list (marks whose activity isn't loaded can't be placed and are ignored). The
/// anchor's author counts as a reader too and, without a mark, sorts last (posting time <= any later acknowledge).
/// Typing people aren't known here.
pub(super) fn seen_keys(msgs: &[Message], marks: &[cloud::ReadMark]) -> Vec<(String, String, String)> {
    let Some(anchor) = msgs.iter().rposition(|m| m.notice.is_empty()) else { return Vec::new() };
    // (read time, key); an empty time sorts last.
    let mut out: Vec<(String, (String, String, String))> = Vec::new();
    let mut add = |time: &str, email: &str, pid: &str, name: &str| {
        let e = email.to_lowercase();
        match out.iter_mut().find(|o| (!e.is_empty() && o.1 .0 == e) || (e.is_empty() && !pid.is_empty() && o.1 .1 == pid)) {
            Some(o) => { if time > o.0.as_str() { o.0 = time.to_string(); } }
            None => out.push((time.to_string(), (e, pid.to_string(), name.to_string()))),
        }
    };
    // A mark counts when the first message with its activity id is at or after the anchor: in the (short) tail and not before
    // it. Runs every frame: marks outside the tail are skipped without a scan of the whole list.
    let tail = &msgs[anchor..];
    for mk in marks {
        if !tail.iter().any(|m| m.id == mk.activity) || msgs[..anchor].iter().any(|m| m.id == mk.activity) { continue; }
        add(&mk.time, &mk.email, &mk.person, "");
    }
    for m in &msgs[anchor..] { if !m.mine && !m.email.is_empty() && m.notice.is_empty() { add("", &m.email, "", &m.author); } }
    out.sort_by(|a, b| b.0.cmp(&a.0)); // stable: equal times keep the order found
    out.into_iter().map(|o| o.1).collect()
}

/// Up to 10 readers all show; with more, 9 avatars show and the "+N" chip shows the rest. Returns (avatars shown, overflow count).
pub(super) fn seen_overflow(n: usize) -> (usize, usize) {
    let shown = if n > MAX_SEEN { MAX_SEEN - 1 } else { n };
    (shown, n - shown)
}

/// "Seen by" row: (label x, x of the first avatar slot) for a row centred at `cx`, a label `label_w` wide, `shown` avatars and an
/// overflow pill `overflow_w` wide (0 when none). The group [label, gap 12.6 pt, slots of 32 pt, pill] is centred at cx - 20.
pub(super) fn seen_layout(cx: f32, label_w: f32, shown: usize, overflow_w: f32) -> (f32, f32) {
    let g = label_w + 12.6 + shown as f32 * 32.0 + overflow_w;
    let left = cx - 20.0 - g / 2.0;
    (left, left + label_w + 12.6)
}

pub(super) fn minutes(t: &str) -> Option<i32> {
    let (h, m) = t.split_once(':')?;
    Some(h.trim().parse::<i32>().ok()? * 60 + m.trim().parse::<i32>().ok()?)
}

/// Hard line breaks as the sender wrote them: CRLF, lone CR, U+2028/2029 and <br> all become "\n".
pub(super) fn normalize_breaks(s: &str) -> String {
    let mut o = s.replace("\r\n", "\n").replace(['\r', '\u{2028}', '\u{2029}', '\u{85}'], "\n");
    for br in ["<br>", "<br/>", "<br />", "<BR>"] { o = o.replace(br, "\n"); }
    o
}

/// The emoji of a reaction chip: legacy names (thumbsup, heart, ...) through the table, otherwise the emoji characters
/// inside the name (tone included); `None` when there are none, so such a reaction is skipped instead of drawn as raw text.
pub(super) fn chip_emoji(name: &str) -> Option<String> {
    if let Some(r) = REACTIONS.iter().find(|r| r.0 == name) { return Some(r.1.to_string()); }
    let cl: Vec<&str> = emoji_text::clusters(name).into_iter().map(|(a, b)| &name[a..b]).collect();
    (!cl.is_empty()).then(|| cl.concat())
}

/// The data layer delivers UTC: `Message.date` is the UTC day and `Message.time` the UTC "HH:MM" of the activity. The UI shows local
/// time: the system time zone incl. daylight saving (`i18n::local_offset_secs`).
pub(super) fn utc_to_local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> (i32, u32, u32, u32, u32) {
    let secs = crate::i18n::days_from_civil(y as i64, mo as i64, d as i64) * 86_400 + h as i64 * 3600 + mi as i64 * 60;
    let p = crate::i18n::parts(secs + crate::i18n::local_offset_secs(secs) as i64);
    (p.year as i32, p.month as u32, p.day as u32, p.hour as u32, p.minute as u32)
}

/// (UTC "YYYY-MM-DD", UTC "HH:MM") -> the same moment in local time.
pub(super) fn to_local(date: &str, time: &str) -> Option<(String, String)> {
    let (y, rest) = date.split_once('-')?;
    let (mo, d) = rest.split_once('-')?;
    let (h, mi) = time.split_once(':')?;
    let (y, mo, d) = (y.parse::<i32>().ok()?, mo.parse::<u32>().ok()?, d.parse::<u32>().ok()?);
    let (h, mi) = (h.trim().parse::<u32>().ok()?, mi.trim().parse::<u32>().ok()?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 { return None; }
    let (y, mo, d, h, mi) = utc_to_local(y, mo, d, h, mi);
    Some((format!("{y:04}-{mo:02}-{d:02}"), format!("{h:02}:{mi:02}")))
}

/// (today, yesterday) as "YYYY-MM-DD" in the clock the messages are shown in: local for real data, the sample's own UTC days offline.
/// Worked out once per minute; a new minute also covers midnight.
pub(super) fn day_ref(real: bool) -> (String, String) {
    if !real { return (system::today(), system::days_ago(1)); }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    static CACHE: std::sync::Mutex<Option<(u64, (String, String))>> = std::sync::Mutex::new(None);
    if let Some((min, v)) = CACHE.lock().ok().and_then(|c| c.clone()) { if min == secs / 60 { return v; } }
    let hm = format!("{:02}:{:02}", secs % 86_400 / 3600, secs % 3600 / 60);
    let f = |d: String| to_local(&d, &hm).map_or(d, |x| x.0);
    let v = (f(system::today()), f(system::days_ago(1)));
    if let Ok(mut c) = CACHE.lock() { *c = Some((secs / 60, v.clone())); }
    v
}

// Messages arriving from the service: each message is converted to local time once when it arrives (`prepare_arrived`, called
// from App::poll), keeping the UTC values in `Message.utc`; all of them are converted again only when the time zone changes
// (`zone_changed`, checked once a minute). The "Today" / "Yesterday" labels compare the local date with `day_ref` at drawing
// time, so midnight needs no conversion.

/// Messages that just arrived from the service: date / time made local once and the content revision stamped.
pub(crate) fn prepare_arrived(list: &mut [Message]) {
    for m in list.iter_mut() { localise(m); m.rev = content_rev(m); }
}

/// `date` / `time` UTC -> local, once (`utc` keeps what was received). Values that aren't a date / time stay as they are.
pub(crate) fn localise(m: &mut Message) {
    if m.utc.is_some() { return; }
    if let Some((d, t)) = to_local(&m.date, &m.time) {
        m.utc = Some((std::mem::replace(&mut m.date, d), std::mem::replace(&mut m.time, t)));
    }
}

/// Converts again from the UTC values (the time zone changed).
pub(crate) fn relocalise(m: &mut Message) {
    if let Some((d, t)) = m.utc.take() { m.date = d; m.time = t; }
    localise(m);
}

/// The message's (date, time) as received: UTC for localised messages, else the fields themselves.
pub(crate) fn utc_date_time(m: &Message) -> (&str, &str) {
    m.utc.as_ref().map_or((m.date.as_str(), m.time.as_str()), |u| (u.0.as_str(), u.1.as_str()))
}

/// A hash of everything the list's layout of a message reads from the message itself (never 0; 0 = unknown).
pub(crate) fn content_rev(m: &Message) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&m.id, &m.author, &m.text, &m.html, m.edited, &m.notice, m.reactions.is_empty(), m.deleted, &m.history, &m.current, m.edit_tag, m.collapsed).hash(&mut h);
    for a in &m.images { (&a.key, a.width, a.height).hash(&mut h); }
    for f in &m.files { (&f.key, &f.name, f.size).hash(&mut h); }
    for p in &m.previews { (&p.title, &p.description, &p.site, &p.image_key).hash(&mut h); }
    h.finish().max(1)
}

/// The local UTC offset in minutes right now (changes with the time zone setting and daylight saving).
pub(crate) fn zone_offset_minutes() -> i64 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (d, hm) = (system::today(), format!("{:02}:{:02}", secs % 86_400 / 3600, secs % 3600 / 60));
    let Some((ld, lt)) = to_local(&d, &hm) else { return 0 };
    let mins = |date: &str, time: &str| -> Option<i64> {
        let mut p = date.split('-').map(|x| x.parse::<i64>().ok());
        let (y, mo, dd) = (p.next()??, p.next()??, p.next()??);
        let (h, mi) = time.split_once(':')?;
        Some(system::days_from_civil(y, mo as u32, dd as u32) * 1440 + h.parse::<i64>().ok()? * 60 + mi.parse::<i64>().ok()?)
    };
    mins(&ld, &lt).zip(mins(&d, &hm)).map_or(0, |(l, u)| l - u)
}

/// True once per time-zone change (checked by the caller at most once a minute); the first call only records the zone.
pub(crate) fn zone_changed() -> bool {
    static LAST: std::sync::Mutex<Option<i64>> = std::sync::Mutex::new(None);
    let now = zone_offset_minutes();
    let Ok(mut l) = LAST.lock() else { return false };
    let changed = l.is_some_and(|o| o != now);
    *l = Some(now);
    changed
}

/// "2026-11-13" -> "13.11.2026".
pub(super) fn date_de(date: &str) -> String {
    match date.split('-').collect::<Vec<_>>()[..] { [y, m, d] => format!("{d}.{m}.{y}"), _ => date.to_string() }
}

/// The time next to the sender's name: "HH:MM" for today and yesterday (the day line above says which), the full German date
/// "13.11.2026, 16:48" for everything older.
/// Grouping window: a message row starts a new group when it is more than 60 000 ms after the previous row; same sender and
/// message-type checks come on top.
pub(super) fn grouped_in_time(prev_ms: i64, ms: i64) -> bool { ms <= prev_ms + 60_000 }

pub(super) fn header_time(date: &str, time: &str, today: &str, yesterday: &str) -> String {
    if date.is_empty() || date == today || date == yesterday { time.to_string() } else { format!("{}, {time}", date_de(date)) }
}

pub(super) fn day_label(date: &str, today: &str, yesterday: &str) -> String {
    if date == today { tr("Today").into() } else if date == yesterday { tr("Yesterday").into() } else { date_de(date) }
}

/// `base` at opacity `a`, composited like Qt does (premultiplied in gamma space). `Color32::from_rgba_unmultiplied` converts
/// through linear light, which turns white@0.07 into ~white@0.34 on screen - far too bright for the theme's alpha tokens.
pub(super) fn wa(base: Color32, a: f32) -> Color32 {
    let al = (a * 255.0).round() as u32;
    let p = |c: u8| ((c as u32 * al + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(p(base.r()), p(base.g()), p(base.b()), al as u8)
}

/// A rounded rectangle filled with (a part of) a texture, with antialiased corners (`widgets::fill_convex_aa`).
pub(super) fn round_image(p: &egui::Painter, r: Rect, radius: f32, uv: Rect, tex: TextureId, tint: Color32) {
    use egui::emath::GuiRounding;
    let r = r.round_to_pixels(p.ctx().pixels_per_point()); // straight edges on whole pixels, like egui's own rects
    let uv_at = |pt: egui::Pos2| pos2(uv.left() + (pt.x - r.left()) / r.width() * uv.width(), uv.top() + (pt.y - r.top()) / r.height() * uv.height());
    let pts = crate::ui::widgets::rounded_rect_points(r, [radius; 4], p.ctx().pixels_per_point(), f32::INFINITY);
    crate::ui::widgets::fill_convex_aa(p, &pts, tex, 1, |q| (tint, uv_at(q)));
}

/// File category of a name. Categories: text (.rtf .txt .xml .one .docx .json .doc), spreadsheet (.xls .xlsx .csv),
/// presentation (.ppt .pptx), audio (.mp3 .wav .flac), video (.mov .mp4 .avi .flv .wmv .mkv .mpg), image (.jpg .jpeg .bmp
/// .raw .tif .tiff .paint .ico .png .svg .gif), archive (.zip .rar .tgz .gz .7z), pdf; everything else is "other".
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum FileKind { Text, Spreadsheet, Presentation, Audio, Video, Image, Archive, Pdf, Other }
pub(super) fn file_kind(name: &str) -> FileKind {
    let ext = match name.rfind('.') { Some(i) if i > 0 => name[i + 1..].to_lowercase(), _ => return FileKind::Other };
    match ext.as_str() {
        "rtf" | "txt" | "xml" | "one" | "doc" | "docx" | "json" => FileKind::Text,
        "xls" | "xlsx" | "csv" => FileKind::Spreadsheet,
        "ppt" | "pptx" => FileKind::Presentation,
        "mp3" | "wav" | "flac" => FileKind::Audio,
        "mov" | "mp4" | "avi" | "flv" | "wmv" | "mkv" | "mpg" => FileKind::Video,
        "jpg" | "jpeg" | "bmp" | "raw" | "tif" | "tiff" | "paint" | "ico" | "png" | "svg" | "gif" => FileKind::Image,
        "zip" | "rar" | "tgz" | "gz" | "7z" => FileKind::Archive,
        "pdf" => FileKind::Pdf,
        _ => FileKind::Other,
    }
}

/// Glyph of the 64 pt icon box of a file card (21 pt high, previewBox-icon colour). "Other" (no extension, .pub, .xltx) is
/// `folder-filled`; the known categories use the matching icon family (pdf, audio, image, spreadsheet, text, video, zip,
/// powerpoint, default document) in the filled weight.
pub(super) fn file_icon(name: &str) -> &'static str {
    match file_kind(name) {
        FileKind::Text => "file-text-filled", FileKind::Spreadsheet => "file-spreadsheet-filled", FileKind::Presentation => "file-powerpoint-filled",
        FileKind::Audio => "file-audio-filled", FileKind::Video => "file-video-filled", FileKind::Image => "file-image-filled",
        FileKind::Archive => "file-zip-filled", FileKind::Pdf => "file-pdf-filled", FileKind::Other => "folder-filled",
    }
}

/// Width of a file card in the gallery view (pt); its height is 52 pt.
pub(super) const FILE_CARD_W: f32 = 300.0;
pub(super) const FILE_CARD_H: f32 = 52.0;
/// File cards sit in a flow as wide as the message, 8 pt apart.
pub(super) const FILE_FLOW_GAP: f32 = 8.0;

/// Card positions of `n` file cards in a flow `avail` wide: (x, y) offsets, card width, total height. A card never gets wider
/// than 300; when even one does not fit it shrinks to the available width.
pub(super) fn file_flow(n: usize, avail: f32) -> (Vec<(f32, f32)>, f32, f32) {
    if n == 0 { return (vec![], 0.0, 0.0); }
    let w = FILE_CARD_W.min(avail.max(1.0));
    let per_row = (((avail + FILE_FLOW_GAP) / (w + FILE_FLOW_GAP)).floor() as usize).max(1);
    let pos = (0..n).map(|i| (((i % per_row) as f32) * (w + FILE_FLOW_GAP), ((i / per_row) as f32) * (FILE_CARD_H + FILE_FLOW_GAP))).collect();
    let rows = n.div_ceil(per_row) as f32;
    (pos, w, rows * FILE_CARD_H + (rows - 1.0) * FILE_FLOW_GAP)
}

/// The size text under a file (see `app::files::size_text`).
pub(super) fn human_size(b: u64) -> String { crate::app::files::size_text(b) }

/// Size of an inline image: width min(320, available), height = width / aspect, and when that exceeds 320 the height is 320
/// and the width follows the aspect.
pub(super) fn image_box(a: &Attachment, avail: f32, tex: Option<[usize; 2]>) -> Vec2 {
    // Once the picture is decoded its real pixel size decides (the activity's width/height can describe another box, which
    // would stretch the picture); the metadata only sizes the placeholder before that.
    let (w, h) = match tex { Some([w, h]) if w > 0 && h > 0 => (w as f32, h as f32), _ if a.width > 0 && a.height > 0 => (a.width as f32, a.height as f32), _ => (1.0, 1.0) };
    if w > 1.0 && w <= SMALL_IMG && h <= SMALL_IMG { return Vec2::new(w, h); } // a small image is shown at its own size
    let aspect = w / h.max(1.0);
    let max_w = IMG_MAX.min(avail);
    let mut bw = max_w;
    let mut bh = bw / aspect;
    if bh > IMG_MAX { bh = IMG_MAX; bw = IMG_MAX * aspect; }
    Vec2::new(bw, bh)
}

#[cfg(test)]
mod file_card_tests {
    use super::*;

    /// File cards: 300 wide side by side with 8 between, wrapping when the message is narrower.
    #[test]
    fn file_cards_flow_side_by_side() {
        let (pos, w, h) = file_flow(2, 700.0);
        assert_eq!((w, h), (300.0, 52.0));
        assert_eq!(pos, vec![(0.0, 0.0), (308.0, 0.0)]); // 8 apart
        let (pos, _, h) = file_flow(3, 610.0); // two per row
        assert_eq!(pos[2], (0.0, 60.0));
        assert_eq!(h, 52.0 * 2.0 + 8.0);
        let (pos, w, h) = file_flow(2, 250.0); // narrower than one card: one per row, card shrinks
        assert_eq!((w, pos[1], h), (250.0, (0.0, 60.0), 112.0));
        assert_eq!(file_flow(0, 500.0).2, 0.0);
    }

    #[test]
    fn icons_follow_the_originals_categories() {
        // Files without a known extension are "other" -> the folder glyph.
        for n in ["vps-xl", "vps-xl.pub", "Spesenabrechnung_2026.xltx"] { assert_eq!(file_icon(n), "folder-filled", "{n}"); }
        assert_eq!(file_kind("Bericht.PDF"), FileKind::Pdf);
        assert_eq!(file_kind("a.tar.gz"), FileKind::Archive);
        assert_eq!(file_kind("song.flac"), FileKind::Audio);
        assert_eq!(file_kind("plan.xlsx"), FileKind::Spreadsheet);
        assert_eq!(file_kind(".hidden"), FileKind::Other);
    }

    /// Middle elision: the same number of characters on both sides of "…" ("Spesenabre…_2026.xltx").
    #[test]
    fn names_are_elided_in_the_middle() {
        let w = |s: &str| s.chars().count() as f32 * 10.0; // a fixed-width stand-in font
        let e = |s: &str, max: f32| super::super::text::elide_middle_by(s, max, &w);
        assert_eq!(e("Spesenabrechnung_2026.xltx", 210.0), "Spesenabre…_2026.xltx");
        assert_eq!(e("vps-xl", 60.0), "vps-xl"); // fits: unchanged
        assert_eq!(e("abcdefgh", 50.0), "ab…gh");
        assert_eq!(e("abcdefgh", 25.0), "…");
    }
}