//! The GIF picker popup of the composer (Giphy is called directly): a search row ("Nach GIFs suchen" with the magnifier
//! button, the cancel-search button while there is text), below it a 3-column masonry grid of the trending GIFs
//! (`fixed_width_downsampled`, animated, decoded lazily for the visible tiles only) that turns into search results while typing
//! (300 ms debounce), infinite scroll by offset paging, keyboard navigation, and a footer with the "POWERED BY GIPHY" logo and
//! the "Bedingungen anzeigen" link. No tabs, no categories, no favourites, no stars: those live in the built-in plugin
//! "GIF-Extras" (`src/extras/gif_extras/`), reached only through the hook in `ext.rs`.
//!
//! Layout (px): 652 x 406, vertical layout without spacing or margins; search row 52 high (background + 1 px bottom border
//! `wx-headingColor-background`): 40 px spacer, 28 x 28 ghost search button (`search-light.svg`), the search field (min 464 x 28,
//! background `wx-headingColor-background`, text `inputText-primary`), 28 x 28 ghost cancel button (`cancel-light.svg`, hidden
//! until there is text), 40 px spacer; then the scroll area (no border, transparent, vertical bar as needed); the footer is
//! 32 high with margins 16 / 0 / 16 / 0: a 75 px spacer, the logo (max 110 x 12) between two expanding spacers, then the
//! "Bedingungen anzeigen" link (16 px regular underlined; tooltip "Giphy-Geschäftsbedingungen anzeigen"); the empty and
//! error states use 15 px / 13 px text. Grid: paddings 12 left, 8 top / right / bottom, 8 px gaps (three 200 px columns next
//! to the 16 px arrow scroll bar); popup corners 6 px.
//! Colours: `wx-giphy-background` black / white, `wx-headingColor-background` gray-80 #3B3B3B / gray-10 #EDEDED,
//! `wx-giphyFocusBorder-background` = every tile's 2 px border (gray-80 / gray-10, blue-30 when hovered or keyboard-selected).
//! The popup host (see `place`) is centred over the GIF button, a 20 x 10 caret pointing down at it, 1 px `wx-default-border`
//! around a `background-primary` body, a 4 px drop shadow.
//!
//! Picking a GIF stages it in the composer as an attachment chip (100 x 60 bricklet); Enter sends it via `Req::SendGif`.
//! Nothing is ever sent from the picker itself.
//!
//! Hook (composer): `gif_picker_toggle(ctx)` from the GIF button, `gif_picker(ui, t, anchor, space)` and
//! `gif_stage_chip(ui, t, crect, space)` every frame, `gif_send_staged(space, now)` when the composer sends.
//! Offline: `--sample` never touches the network (generated sample GIFs). Screenshot runs: env
//! `WEBOX_SHOT_GIF=1|search|stage|empty|error` opens the picker (or the staged chip / empty / error state) on the first frames;
//! the GIF-Extras plugin reads its own modes (see `src/extras/gif_extras/`). `WEBOX_SHOT_GIF_EXTRAS=0|1` overrides the plugin switch
//! in screenshot runs without saving it.
use crate::extras::giphy::{self, Debounce, Event, FetchError, Gif, Kind};
use crate::assets::photos::Photos;
use crate::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};

pub(crate) mod ext;

/// The popup size (px).
pub(crate) const W: f32 = 652.0;
pub(crate) const H: f32 = 406.0;
/// Search row height, its fixed side spacers and the 28 x 28 buttons.
const SEARCH_H: f32 = 52.0;
const SEARCH_SIDE: f32 = 40.0;
const BTN: f32 = 28.0;
/// Footer height and side margins, the fixed spacer before the logo, the logo's maximum size.
const FOOT_H: f32 = 32.0;
const FOOT_MARGIN: f32 = 16.0;
const FOOT_SPACER: f32 = 75.0;
const LOGO_MAX: Vec2 = Vec2::new(110.0, 12.0);
/// Grid metrics: 3 columns, 8 px gaps; GRID_PAD / BAR_W are the GIF-Extras plugin's (the standard picker's are PAD_* below).
pub(crate) const COLS: usize = 3;
pub(crate) const GRID_PAD: f32 = 8.0;
pub(crate) const GAP: f32 = 8.0;
const BAR_W: f32 = 10.0;
const MAX_IN_FLIGHT: usize = 6;
/// Decoded animated thumbnails kept at once (each holds up to 60 textures); older ones are dropped and re-decoded on demand.
const MAX_DECODED: usize = 48;
const MAX_BYTES_CACHE: usize = 400;
// The Giphy marks (`assets/cisco/images/giphy`); without them the footer says it in text.
pub(crate) fn giphy_art(name: &str) -> Option<&'static [u8]> { crate::assets::cisco_assets::get(&format!("images/giphy/{name}")) }

/// Downloaded and decoded thumbnails (shared by the standard picker and the GIF-Extras plugin's popup; each owns one).
#[derive(Default)]
pub(crate) struct Thumbs { bytes: HashMap<String, Rc<Vec<u8>>>, failed: HashSet<String>, requested: HashSet<String>, in_flight: usize, photos: Photos, decoded: HashSet<String>, pub(crate) offline: bool }

impl Thumbs {
    /// Starts the download of a thumbnail once (bounded number in flight; the bytes arrive as `Event::Thumb` on `tx`).
    pub(crate) fn want(&mut self, tx: Option<&Sender<Event>>, id: &str, url: &str) {
        if url.is_empty() || self.offline || self.bytes.contains_key(id) || self.failed.contains(id) || self.requested.contains(id) || self.in_flight >= MAX_IN_FLIGHT { return }
        if let Some(tx) = tx { self.requested.insert(id.to_string()); self.in_flight += 1; giphy::fetch_thumb(tx.clone(), id.to_string(), url.to_string()); }
    }
    /// Marks a thumbnail as requested for a load of the caller's own (e.g. a local file; also offline); false when it is already
    /// known. The bytes must come back as `Event::Thumb` on the same channel.
    pub(crate) fn claim(&mut self, id: &str) -> bool {
        if self.bytes.contains_key(id) || self.failed.contains(id) || self.requested.contains(id) { return false }
        self.requested.insert(id.to_string()); self.in_flight += 1; true
    }
    /// An `Event::Thumb` answer.
    pub(crate) fn arrived(&mut self, id: String, bytes: Option<Vec<u8>>) {
        self.in_flight = self.in_flight.saturating_sub(1);
        match bytes { Some(b) => { self.bytes.insert(id, Rc::new(b)); } None => { self.failed.insert(id); } }
    }
    pub(crate) fn insert(&mut self, id: &str, b: Rc<Vec<u8>>) { self.bytes.insert(id.to_string(), b); }
    pub(crate) fn get(&self, id: &str) -> Option<Rc<Vec<u8>>> { self.bytes.get(id).cloned() }
    pub(crate) fn busy(&self) -> bool { self.in_flight > 0 }
    /// Texture (and pixel size) of a thumbnail's current frame; decodes it on first use.
    pub(crate) fn tex(&mut self, ctx: &egui::Context, id: &str, now: f64) -> Option<(egui::TextureId, [usize; 2])> {
        if !self.decoded.contains(id) {
            let b = self.bytes.get(id)?.clone();
            self.photos.add_images(vec![(id.to_string(), (*b).clone())]);
            self.decoded.insert(id.to_string());
        }
        self.photos.image_frame(ctx, id, now)
    }
    /// Keeps the decoded set bounded (call once per frame).
    pub(crate) fn budget(&mut self) { if self.decoded.len() > MAX_DECODED { self.photos = Photos::default(); self.decoded.clear(); } }
    /// Drops the downloaded bytes except `keep` when the cache grew large (call when a popup opens).
    pub(crate) fn trim(&mut self, keep: &HashSet<String>) {
        if self.bytes.len() <= MAX_BYTES_CACHE { return }
        self.bytes.retain(|k, _| keep.contains(k));
        self.requested = self.bytes.keys().cloned().collect();
        self.decoded.clear(); self.photos = Photos::default();
    }
}

#[derive(Default)]
struct State {
    open: bool,
    /// The open popup is the GIF-Extras plugin's (switching the plugin while it is open closes it).
    ext: bool,
    query: String, focus: bool, debounce: Debounce, kind: Option<Kind>, ticket: u64,
    gifs: Vec<Gif>, total: u32, next_offset: u32, loading: bool, error: Option<FetchError>, sel: Option<usize>, scroll_to_sel: bool,
    /// First pages of the trending list, kept for the session (and its paging state).
    trending: Option<(Vec<Gif>, u32, u32)>,
    tx: Option<Sender<Event>>, rx: Option<Receiver<Event>>,
    thumbs: Thumbs,
    /// The GIF staged in the composer, per space.
    staged: HashMap<String, Gif>,
    /// An emoji the plugin's popup handed over (its "Emojis" tab), waiting for the composer.
    pending_emoji: Option<String>,
    sample: bool, sample_all: Vec<Gif>, shot_done: bool,
}

thread_local! { static ST: RefCell<State> = RefCell::new(State::default()); }
fn take() -> State { ST.with(|s| std::mem::take(&mut *s.borrow_mut())) }
fn put(st: State) { ST.with(|s| *s.borrow_mut() = st) }

/// True while the popup is open (for the toolbar button's pressed look).
pub(crate) fn gif_picker_open() -> bool { ST.with(|s| s.try_borrow().map_or(false, |s| s.open)) }

fn fetch(st: &mut State, kind: Kind, offset: u32, rating: &str) {
    if st.sample { return }
    let Some(tx) = st.tx.clone() else { return };
    st.ticket += 1;
    st.loading = true;
    st.error = None;
    st.kind = Some(kind.clone());
    giphy::fetch_page(tx, st.ticket, kind, rating.to_string(), offset);
}

/// Colours of the popup (dark = the app's dark theme). bg = wx-giphy-background, head = wx-headingColor-background,
/// focus = wx-giphyFocusBorder-background (hovered / focused), cancel = wx-giphyIntegrationWidgetCancelSearchButton-background.
/// Popup host: body = background-primary (gray-100 #121212 / white), border = wx-default-border (gray-80 / gray-10),
/// shadow = wx-popupDropShadow (black 16 %). tile_edge = wx-giphyFocusBorder-background in its normal state (the tiles' border).
pub(crate) struct Pal { pub bg: Color32, pub head: Color32, pub text: Color32, pub text2: Color32, pub focus: Color32, pub cancel: Color32, pub tile: Color32, pub link: Color32, pub edge: Color32, pub body: Color32, pub border: Color32, pub shadow: Color32, pub tile_edge: Color32 }
pub(crate) fn pal(dark: bool) -> Pal {
    let shadow = Color32::from_black_alpha(41);
    if dark { Pal { bg: hex(0x000000), head: hex(0x3B3B3B), text: hex(0xF7F7F7), text2: hex(0xB2B2B2), focus: hex(0x52DCFF), cancel: hex(0xB2B2B2), tile: hex(0x292929), link: hex(0x52DCFF), edge: hex(0x3B3B3B), body: hex(0x121212), border: hex(0x3B3B3B), shadow, tile_edge: hex(0x3B3B3B) } }
    else { Pal { bg: hex(0xFFFFFF), head: hex(0xEDEDED), text: hex(0x121212), text2: hex(0x707070), focus: hex(0x52DCFF), cancel: hex(0x707070), tile: hex(0xEDEDED), link: hex(0x0077A3), edge: hex(0xDEDEDE), body: hex(0xFFFFFF), border: hex(0xEDEDED), shadow, tile_edge: hex(0xEDEDED) } }
}

fn ensure_channel(st: &mut State) {
    if st.tx.is_none() { let (tx, rx) = channel(); st.tx = Some(tx); st.rx = Some(rx); }
}

/// Width of one column when the grid area is `inner` wide (padding and scroll bar taken off).
pub(crate) fn col_width(inner: f32, cols: usize) -> f32 { ((inner - 2.0 * GRID_PAD - BAR_W - (cols as f32 - 1.0) * GAP) / cols as f32).floor() }

/// Masonry placement: every GIF goes under the currently shortest column, height from its aspect ratio.
pub(crate) fn masonry(gifs: &[Gif], colw: f32, cols: usize) -> (Vec<Rect>, f32) {
    let cols = cols.max(1);
    let mut heights = vec![0.0f32; cols];
    let mut out = Vec::with_capacity(gifs.len());
    for g in gifs {
        let h = (colw * g.thumb_h as f32 / g.thumb_w.max(1) as f32).round().max(24.0);
        let mut c = 0;
        for k in 1..cols { if heights[k] < heights[c] { c = k } }
        out.push(Rect::from_min_size(pos2(c as f32 * (colw + GAP), heights[c]), Vec2::new(colw, h)));
        heights[c] += h + GAP;
    }
    (out, heights.iter().cloned().fold(0.0, f32::max))
}

/// Keyboard navigation in the masonry grid: left / right = previous / next GIF, up / down = the nearest GIF above / below in the
/// same column (by vertical position).
pub(crate) fn grid_step(tiles: &[Rect], sel: Option<usize>, key: Key) -> Option<usize> {
    let n = tiles.len();
    if n == 0 { return None }
    let Some(s) = sel.filter(|s| *s < n) else { return Some(0) };
    match key {
        Key::ArrowLeft => Some(s.saturating_sub(1)),
        Key::ArrowRight => Some((s + 1).min(n - 1)),
        Key::ArrowDown | Key::ArrowUp => {
            let r = tiles[s];
            let down = key == Key::ArrowDown;
            let best = tiles.iter().enumerate()
                .filter(|(_, t)| (t.min.x - r.min.x).abs() < 1.0 && if down { t.min.y > r.min.y } else { t.min.y < r.min.y })
                .min_by(|a, b| (a.1.min.y - r.min.y).abs().total_cmp(&(b.1.min.y - r.min.y).abs()));
            Some(best.map_or(s, |(i, _)| i))
        }
        _ => Some(s),
    }
}

/// The standard picker's grid inside `layout().grid`: left / top / right (before the scroll bar) / bottom padding. With the
/// 16 px arrow scroll bar this gives three 200 px columns.
pub(crate) const PAD_L: f32 = 12.0;
pub(crate) const PAD_T: f32 = 8.0;
pub(crate) const PAD_R: f32 = 8.0;
pub(crate) const PAD_B: f32 = 8.0;
/// One tile: border width 2, radius 0, background `wx-giphy-background`, border `wx-giphyFocusBorder-background`
/// (gray-80 / gray-10 normally, blue-30 when hovered or focused); the GIF sits inside with 2 px margins.
pub(crate) const TILE_BORDER: f32 = 2.0;

/// Column width of the standard grid for a `grid_w` wide grid area (paddings and the arrow scroll bar taken off).
pub(crate) fn grid_col_width(grid_w: f32) -> f32 {
    ((grid_w - PAD_L - PAD_R - crate::ui::autoscroll::BAR_W - (COLS as f32 - 1.0) * GAP) / COLS as f32).floor()
}

/// The popup host: a balloon centred over the GIF button with the caret pointing down at it.
/// - Body colour `background-primary`, border `wx-default-border`, shadow `wx-popupDropShadow`.
/// - A 4 px margin around the body for the drop shadow (blur 4, offset 0); the caret is 20 wide (at most 20 % of the width)
///   and 10 high; the content sits inside a 1 px border.
/// - Proposed position: x = button.center.x - width / 2, y = button.top - height.
/// - Moved back onto the screen by the overflow, but by at most half its width / height.
/// - The caret's base sits on the body's edge, its tip at the button's centre x, kept 10 px from the left and 20 px from the right.
/// The screen is the Webox window here (an egui popup cannot leave it).
pub(crate) const SHADOW_M: f32 = 4.0;
pub(crate) const CARET_W: f32 = 20.0;
pub(crate) const CARET_H: f32 = 10.0;
/// Rounded corners of the popup body (px).
pub(crate) const BODY_R: f32 = 6.0;

/// Where the popup goes for a GIF button at `button` in a `screen` sized window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
    /// The whole popup window (shadow margins included).
    pub window: Rect,
    /// The balloon body (1 px border inside it), without the caret.
    pub body: Rect,
    /// The picker content (652 x 406).
    pub content: Rect,
    /// The caret: base centre x on `body.bottom()`, tip at `tip`.
    pub tip: egui::Pos2,
}

pub(crate) fn place(screen: Rect, button: Rect) -> Placement {
    let size = Vec2::new(W + 2.0 + 2.0 * SHADOW_M, H + 2.0 + CARET_H + 2.0 * SHADOW_M);
    let mut x = (button.center().x - (size.x / 2.0).floor()).round();
    let mut y = button.top() - size.y;
    // Back onto the screen, at most by half the size (right / bottom checked first).
    let (hw, hh) = ((size.x / 2.0).floor(), (size.y / 2.0).floor());
    if x + size.x > screen.right() { x -= (x + size.x - screen.right()).min(hw); } else if x < screen.left() { x += (screen.left() - x).min(hw); }
    if y + size.y > screen.bottom() { y -= (y + size.y - screen.bottom()).min(hh); } else if y < screen.top() { y += (screen.top() - y).min(hh); }
    let window = Rect::from_min_size(pos2(x, y), size);
    let central = window.shrink(SHADOW_M);
    let body = Rect::from_min_max(central.min, pos2(central.right(), central.bottom() - CARET_H));
    let content = Rect::from_min_size(central.min + Vec2::splat(1.0), Vec2::new(W, H));
    let tx = button.center().x.round().clamp(central.left() + CARET_W / 2.0, central.right() - CARET_W);
    Placement { window, body, content, tip: pos2(tx, central.bottom()) }
}

impl Placement {
    /// The caret triangle (base left, tip, base right).
    pub(crate) fn caret(&self) -> [egui::Pos2; 3] {
        [pos2(self.tip.x - CARET_W / 2.0, self.body.bottom()), self.tip, pos2(self.tip.x + CARET_W / 2.0, self.body.bottom())]
    }
    /// Body or caret under `p` (clicks there stay in the popup).
    pub(crate) fn contains(&self, p: egui::Pos2) -> bool {
        self.body.contains(p) || (p.y >= self.body.bottom() && p.y <= self.tip.y && (p.x - self.tip.x).abs() <= (CARET_W / 2.0) * (self.tip.y - p.y) / CARET_H)
    }
}

/// The GIF-Extras plugin's popup rectangle (its own frame, not the standard one): above the button, clamped to the window.
pub(crate) fn popup_rect(screen: Rect, anchor: Rect) -> Rect {
    let left = (anchor.center().x - 40.0).clamp(8.0, (screen.right() - W - 8.0).max(8.0));
    let top = (anchor.top() - 10.0 - H).max(8.0);
    Rect::from_min_size(pos2(left, top), Vec2::new(W, H))
}

/// The standard popup's parts (for the layout test and the drawing code).
pub(crate) struct Layout { pub search: Rect, pub icon: Rect, pub field: Rect, pub cancel: Rect, pub grid: Rect, pub foot: Rect, pub logo_x: (f32, f32) }
pub(crate) fn layout(rect: Rect) -> Layout {
    let search = Rect::from_min_size(rect.min, Vec2::new(rect.width(), SEARCH_H));
    let y = search.center().y - BTN / 2.0;
    let icon = Rect::from_min_size(pos2(search.left() + SEARCH_SIDE, y), Vec2::splat(BTN));
    let cancel = Rect::from_min_size(pos2(search.right() - SEARCH_SIDE - BTN, y), Vec2::splat(BTN));
    let field = Rect::from_min_max(pos2(icon.right(), y), pos2(cancel.left(), y + BTN));
    let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOOT_H), rect.right_bottom());
    let grid = Rect::from_min_max(pos2(rect.left(), search.bottom()), pos2(rect.right(), foot.top()));
    // Logo between two expanding spacers after the fixed one (the link's width is known only when drawing).
    Layout { search, icon, field, cancel, grid, foot, logo_x: (foot.left() + FOOT_MARGIN + FOOT_SPACER, foot.right() - FOOT_MARGIN) }
}

#[cfg(test)]
mod tests;

mod draw;
mod flow;
