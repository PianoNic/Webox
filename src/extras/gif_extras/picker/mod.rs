//! The plugin's extended GIF popup (drawn in the standard popup's place and size while the plugin is on), modelled on other
//! chat apps with Webox's own colours / fonts / frame: tabs "GIFs | Stickers | Emojis"; the GIF and Sticker tabs open on a
//! home screen of tiles with a darkened looping thumbnail behind a bold centred label: "Favorites" (latest favourite),
//! "Trending GIFs" / "Trending stickers" (trending, with the `trending-regular` icon) and Giphy's top-level categories
//! (`/v1/gifs/categories`: name + picture); a tile runs that search / opens trending / the favourites list, the arrow in the
//! search field goes back, typing searches. Stickers use `/v1/stickers/trending|search` (same key) and are sent
//! exactly like GIFs. A star on each tile saves it to the plugin's local favourites (`favs.rs`). The Emojis tab embeds the
//! emoji picker. "No results", "Favorites", ... come from the language pack; "Search GIFs", "Trending GIFs", the
//! tab labels, the sticker search hint and the empty-favourites texts are Webox's own wording.
//! The setting "Compact size" switches to the smaller popup of earlier versions (`layout.rs`), read every frame.
//! Screenshot runs (with `WEBOX_SHOT_GIF_EXTRAS=1`): `WEBOX_SHOT_GIF=1|stickers|emoji|trending|search|favs|favs-empty|upfavs|home-favs`,
//! `WEBOX_SHOT_GIF_COMPACT=0|1` overrides the size setting.
use super::{upload_favs, upload_stars, Plugin};
use crate::extras::giphy::{self, Category, Debounce, Event, FetchError, Gif, Kind};
use crate::ui::gif_picker::ext::PopupOut;
use crate::ui::gif_picker::{giphy_art, grid_step, masonry_gap, pal, Thumbs};
use crate::*;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};

const PAD: f32 = 12.0;
const FOOT_H: f32 = 34.0;
/// Top row with the tabs (28 pt pills) and the space under it before the search field.
const HEAD_H: f32 = 46.0;

mod flow;
pub(crate) mod layout;

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum Tab { #[default] Gifs, Stickers, Emojis }
/// Home = the tile screen (empty search), Trending / Search = Giphy results, Favorites = the locally saved list.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum View { #[default] Home, Trending, Search, Favorites }

/// The home-screen tiles in order: Favorites, Trending GIFs / stickers, then Giphy's categories.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Tile { Favs, Trending, Cat(usize) }
pub(super) fn home_tiles(ncats: usize) -> Vec<Tile> { let mut v = vec![Tile::Favs, Tile::Trending]; v.extend((0..ncats).map(Tile::Cat)); v }

#[derive(Default)]
pub(super) struct State {
    query: String, focus: bool, debounce: Debounce, kind: Option<Kind>, ticket: u64,
    pub(super) gifs: Vec<Gif>, total: u32, next_offset: u32, loading: bool, error: Option<FetchError>, sel: Option<usize>, scroll_to_sel: bool,
    /// First page of the trending list per tab ([gifs, stickers]), kept for the session (and its paging state).
    trending: [Option<(Vec<Gif>, u32, u32)>; 2],
    pub(super) tab: Tab, pub(super) view: View,
    /// Home-screen categories (session cache) and their loading state.
    cats: Option<Vec<Category>>, cats_loading: bool,
    tx: Option<Sender<Event>>, rx: Option<Receiver<Event>>,
    thumbs: Thumbs,
    sample: bool, sample_all: Vec<Gif>, sample_stk: Vec<Gif>, sample_cats: Vec<Category>, shot_done: bool,
}

fn tab_idx(tab: Tab) -> usize { (tab == Tab::Stickers) as usize }

/// UV rect that crops a texture of `size` to fill `r` (centre crop).
fn cover_uv(size: [usize; 2], r: Rect) -> Rect {
    let (ia, ra) = (size[0] as f32 / size[1].max(1) as f32, r.width() / r.height().max(1.0));
    if ia > ra { let u = ra / ia; Rect::from_min_max(pos2((1.0 - u) / 2.0, 0.0), pos2((1.0 + u) / 2.0, 1.0)) }
    else { let v = ia / ra; Rect::from_min_max(pos2(0.0, (1.0 - v) / 2.0), pos2(1.0, (1.0 + v) / 2.0)) }
}

/// Starts loading a thumbnail: a favourite uploaded GIF's is a local file (read off the UI thread, also in sample mode), every
/// other one is downloaded from Giphy.
fn want(thumbs: &mut Thumbs, tx: Option<&Sender<Event>>, id: &str, url: &str) {
    if upload_favs::is_upload_id(id) {
        if let Some(tx) = tx { if thumbs.claim(id) { upload_favs::load_thumb_async(tx.clone(), id.to_string(), url.to_string()); } }
        return;
    }
    thumbs.want(tx, id, url);
}

impl Plugin {
    /// Draws the popup in `rect` (the core picker's), or in its own smaller place when `compact`.
    pub(super) fn picker_draw(&mut self, app: &mut App, ui: &mut egui::Ui, t: &Th, rect: Rect, anchor: Rect, space: &str, compact: bool) -> PopupOut {
        let ctx = ui.ctx().clone();
        let geo = layout::geo(if compact { layout::compact_rect(ctx.screen_rect(), anchor) } else { rect }, compact);
        let rect = geo.rect;
        let now = ctx.input(|i| i.time);
        let dark = t.text1 == hex(0xF7F7F7);
        let pal = pal(dark);
        let mut out = PopupOut::default();
        // The favourites (loaded on first use) for the stars and the home tile.
        self.favs(app);

        // Answers of the worker threads.
        let mut events = vec![];
        if let Some(rx) = &self.picker.rx { while let Ok(e) = rx.try_recv() { events.push(e) } }
        {
            let st = &mut self.picker;
            for e in events {
                match e {
                    Event::Page { ticket, res } if ticket == st.ticket => {
                        st.loading = false;
                        match res {
                            Ok(p) => {
                                if p.offset == 0 { st.gifs.clear(); }
                                for g in p.gifs { if !st.gifs.iter().any(|x| x.id == g.id) { st.gifs.push(g) } }
                                st.total = p.total;
                                st.next_offset = if p.count == 0 { p.total } else { p.offset + p.count };
                                match st.kind { Some(Kind::Trending) => st.trending[0] = Some((st.gifs.clone(), st.next_offset, st.total)), Some(Kind::StickerTrending) => st.trending[1] = Some((st.gifs.clone(), st.next_offset, st.total)), _ => {} }
                            }
                            Err(e) => { if st.gifs.is_empty() { st.error = Some(e) } else { st.next_offset = st.total } }
                        }
                    }
                    Event::Thumb { id, bytes } => st.thumbs.arrived(id, bytes),
                    Event::Categories(res) => { st.cats_loading = false; st.cats = Some(res.unwrap_or_default()); }
                    _ => {}
                }
            }
            st.thumbs.budget();
        }
        let giphy_tab = self.picker.tab != Tab::Emojis;
        let sticker = self.picker.tab == Tab::Stickers;

        // Search-as-you-type (300 ms): any text = search results, an emptied field = back to the home tiles.
        if giphy_tab {
            if let Some(q) = self.picker.debounce.due(now) {
                let q = q.trim().to_string();
                if q.is_empty() { if self.picker.view == View::Search { self.enter(app, View::Home, ""); } }
                else { self.enter(app, View::Search, &q); }
            }
        }
        if let Some(w) = self.picker.debounce.wait(now) { ctx.request_repaint_after(std::time::Duration::from_secs_f32(w as f32 + 0.01)); }
        if self.picker.loading || self.picker.thumbs.busy() || self.picker.cats_loading { ctx.request_repaint_after(std::time::Duration::from_millis(60)); }

        let grid = geo.grid;
        let (tiles, total_h) = masonry_gap(&self.picker.gifs, geo.colw, geo.cols, geo.gap);
        let n = self.picker.gifs.len();
        let mut pick: Option<usize> = None;
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        if giphy_tab && n > 0 && self.picker.view != View::Home {
            let st = &mut self.picker;
            for key in [Key::ArrowDown, Key::ArrowUp] {
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) { st.sel = grid_step(&tiles, st.sel, key); st.scroll_to_sel = true; }
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) { pick = Some(st.sel.unwrap_or(0).min(n - 1)); }
        }
        let mut open_terms = false;
        let mut load_more = false;
        let mut new_tab: Option<Tab> = None;
        let mut go_home = false;
        let mut open_tile: Option<Tile> = None;
        let mut fav_toggle: Option<usize> = None;
        let mut clear_query = false;
        // Borrowed, not cloned: this runs every frame while the popup is open (`self.favs` was filled above).
        let no_favs = super::favs::Favs::default();
        let favs = self.favs.as_ref().unwrap_or(&no_favs);
        let st = &mut self.picker;
        let tx = st.tx.clone();

        egui::Area::new(egui::Id::new("gif-popup-extras")).fixed_pos(rect.min).order(egui::Order::Foreground).interactable(true).show(&ctx, |ui| {
            ui.set_clip_rect(geo.clip());
            let (_, _) = ui.allocate_exact_size(rect.size(), Sense::click());
            let p = ui.painter().clone();
            let edge = alpha(pal.text, 0.15);
            p.rect_filled(rect, 6.0, pal.bg);
            p.rect_stroke(rect, 6.0, Stroke::new(1.0_f32, edge), egui::StrokeKind::Inside);
            // Compact: a caret under the popup pointing at the GIF button.
            if let Some([a, b, tip]) = geo.caret(anchor) {
                p.add(egui::Shape::convex_polygon(vec![pos2(a.x, a.y - 0.5), pos2(b.x, b.y - 0.5), tip], pal.bg, Stroke::NONE));
                for c in [a, b] { p.line_segment([c, tip], Stroke::new(1.0_f32, edge)); }
            }

            // Tabs: GIFs | Stickers | Emojis (28 pt pills, the selected one filled). Labels are Webox's own (not in the pack).
            let mut x = rect.left() + PAD;
            for (tab, label) in [(Tab::Gifs, crate::trq!("GIFs")), (Tab::Stickers, crate::trq!("Stickers")), (Tab::Emojis, crate::trq!("Emojis"))] {
                let on = st.tab == tab;
                let font = if on { F::Secondary.semibold() } else { F::Secondary.font() };
                let tw = p.layout_no_wrap(label.to_string(), font.clone(), pal.text).size().x;
                let tr = Rect::from_min_size(pos2(x, rect.top() + 9.0), Vec2::new(tw + 24.0, 28.0));
                let resp = ui.interact(tr, egui::Id::new(("gif-tab", tab_idx(tab), tab == Tab::Emojis)), Sense::click());
                if on { p.rect_filled(tr, 14.0, alpha(pal.text, 0.14)); } else if resp.hovered() { p.rect_filled(tr, 14.0, alpha(pal.text, 0.07)); }
                p.text(tr.center(), egui::Align2::CENTER_CENTER, &label, font, if on { pal.text } else { pal.text2 });
                if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                if resp.clicked() { new_tab = Some(tab); }
                x += tr.width() + 4.0;
            }
            p.hline((rect.left() + 1.0)..=(rect.right() - 1.0), rect.top() + HEAD_H - 4.0, Stroke::new(1.0_f32, alpha(pal.text, 0.08)));
            if !giphy_tab { return; } // the Emojis tab is drawn by the embedded emoji picker (below)

            // Search pill: 1 px giphyFocusBorder (blue-30 while focused or hovered); a back arrow replaces the magnifier outside the home screen.
            let sr = Rect::from_min_size(rect.min + Vec2::new(PAD, HEAD_H), Vec2::new(rect.width() - 2.0 * PAD, 28.0));
            let search_id = egui::Id::new("gif-search-extras");
            let focused = ui.memory(|m| m.has_focus(search_id));
            let hovered = ui.rect_contains_pointer(sr);
            p.rect_stroke(sr, 14.0, Stroke::new(1.0_f32, if focused || hovered { pal.focus } else if geo.compact { pal.border } else { pal.edge }), egui::StrokeKind::Inside);
            if st.view == View::Home {
                app.icons.paint(ui, "search-regular", Rect::from_center_size(sr.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(15.0)), pal.text2);
            } else {
                let br = Rect::from_center_size(sr.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(22.0));
                let bresp = ui.interact(br, egui::Id::new("gif-back"), Sense::click());
                if bresp.hovered() { p.circle_filled(br.center(), 11.0, alpha(pal.text, 0.10)); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                app.icons.paint(ui, "arrow-left-regular", Rect::from_center_size(br.center(), Vec2::splat(15.0)), pal.text);
                if bresp.on_hover_text(crate::tr!("AuxiliaryDevicesSettings", "Back")).clicked() { go_home = true; }
            }
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(sr.min + Vec2::new(34.0, 0.0), sr.max - Vec2::new(34.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let hint = if sticker { crate::trq!("Search stickers") } else { crate::trq!("Search GIFs") }; // Webox wording (the pack has neither)
            let r = c.add(egui::TextEdit::singleline(&mut st.query).id(search_id).hint_text(RichText::new(hint).color(pal.text2).size(14.0)).text_color(pal.text)
                .frame(false).margin(Vec2::ZERO).font(F::Body.font()).desired_width(sr.width() - 68.0));
            if st.focus { r.request_focus(); st.focus = false; }
            if r.changed() { let q = st.query.clone(); st.debounce.set(&q, now); }
            if !st.query.is_empty() {
                let xr = Rect::from_center_size(pos2(sr.right() - 18.0, sr.center().y), Vec2::splat(18.0));
                let xresp = ui.interact(xr, egui::Id::new("gif-clear-extras"), Sense::click());
                p.circle_stroke(xr.center(), 8.5, Stroke::new(1.0_f32, pal.cancel));
                app.icons.paint(ui, "cancel-bold", Rect::from_center_size(xr.center(), Vec2::splat(8.0)), pal.cancel);
                if xresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                if xresp.on_hover_text(crate::trq!("Clear entry")).clicked() { clear_query = true; }
            }

            // Content.
            let centre = grid.center();
            if st.view == View::Home {
                // Home screen: tiles (looping thumbnail darkened behind a bold centred label).
                let cats: &[Category] = st.cats.as_deref().unwrap_or(&[]);
                let tl = home_tiles(cats.len());
                let mut gui = ui.new_child(egui::UiBuilder::new().max_rect(grid));
                gui.spacing_mut().scroll = egui::style::ScrollStyle { bar_width: 8.0, bar_inner_margin: 1.0, bar_outer_margin: 0.0, ..egui::style::ScrollStyle::solid() };
                crate::ui::widgets::vscroll().id_salt(("gif-home-scroll", sticker)).auto_shrink([false; 2]).show_viewport(&mut gui, |ui, vp| {
                    let (area, _) = ui.allocate_exact_size(Vec2::new(geo.content_w(), geo.home_h(tl.len())), Sense::hover());
                    for (i, tile) in tl.iter().enumerate() {
                        let rel = geo.home_tile(i);
                        if rel.bottom() < vp.top() - 40.0 || rel.top() > vp.bottom() + 40.0 { continue }
                        let r = rel.translate(area.min.to_vec2());
                        let (label, icon, thumb): (String, Option<&str>, Option<(String, String)>) = match tile {
                            Tile::Favs => (crate::trq!("Favorites").into_owned(), None, favs.latest(sticker).map(|f| (f.id.clone(), f.thumb_url.clone()))),
                            Tile::Trending => {
                                let first = st.trending[tab_idx(st.tab)].as_ref().and_then(|t| t.0.first()).map(|g| (g.id.clone(), g.thumb_url.clone()));
                                (if sticker { crate::trq!("Trending stickers") } else { crate::trq!("Trending GIFs") }.into_owned(), Some("trending-regular"), first)
                            }
                            Tile::Cat(k) => (cats[*k].name.clone(), None, Some((format!("cat:{}", cats[*k].name), cats[*k].thumb_url.clone()))),
                        };
                        if let Some((id, url)) = &thumb { want(&mut st.thumbs, tx.as_ref(), id, url); }
                        let resp = ui.interact(r, egui::Id::new(("gif-home-tile", i, sticker)), Sense::click());
                        ui.painter().rect_filled(r, 6.0, if *tile == Tile::Favs && thumb.is_none() { hex(0x2A6A80) } else { pal.tile });
                        if let Some((id, _)) = &thumb {
                            if let Some((tid, sz)) = st.thumbs.tex(&ctx, id, now) {
                                egui::Image::from_texture((tid, r.size())).uv(cover_uv(sz, r)).tint(Color32::from_gray(if resp.hovered() { 120 } else { 95 })).corner_radius(6.0).paint_at(ui, r);
                            }
                        }
                        // Label: bold, centred, white on the darkened picture (a soft shadow keeps it readable on light pictures).
                        let font = F::Body.semibold();
                        let lw = ui.painter().layout_no_wrap(label.clone(), font.clone(), Color32::WHITE).size().x;
                        let iw = if icon.is_some() { 20.0 } else { 0.0 };
                        let x0 = r.center().x - (lw + iw) / 2.0;
                        if let Some(ic) = icon { app.icons.paint(ui, ic, Rect::from_center_size(pos2(x0 + 8.0, r.center().y), Vec2::splat(16.0)), Color32::WHITE); }
                        ui.painter().text(pos2(x0 + iw + 0.8, r.center().y + 0.8), egui::Align2::LEFT_CENTER, &label, font.clone(), Color32::from_black_alpha(140));
                        ui.painter().text(pos2(x0 + iw, r.center().y), egui::Align2::LEFT_CENTER, &label, font, Color32::WHITE);
                        if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); ui.painter().rect_stroke(r, 6.0, Stroke::new(2.0_f32, pal.focus), egui::StrokeKind::Inside); }
                        if resp.clicked() { open_tile = Some(*tile); }
                    }
                });
                if st.cats.is_none() && st.cats_loading { ui.put(Rect::from_center_size(pos2(centre.x, grid.bottom() - 30.0), Vec2::splat(22.0)), egui::Spinner::new().size(22.0).color(pal.text2)); }
            } else if n == 0 && st.loading {
                ui.put(Rect::from_center_size(centre, Vec2::splat(28.0)), egui::Spinner::new().size(28.0).color(pal.text2));
            } else if n == 0 && st.error.is_some() {
                let g1 = p.layout_no_wrap(crate::trq!("Unable to search GIFs").into_owned(), F::Subtitle.semibold(), pal.text);
                p.galley(pos2(centre.x - g1.size().x / 2.0, centre.y - 24.0), g1, pal.text);
                let g2 = p.layout(crate::trq!("GIFs aren't working right now. Please try again later.").into_owned(), F::Secondary.font(), pal.text2, grid.width() - 50.0);
                p.galley(pos2(centre.x - g2.size().x / 2.0, centre.y + 2.0), g2, pal.text2);
            } else if n == 0 && st.view == View::Favorites {
                // Webox's own wording (no pack string for an empty favourites list).
                app.icons.paint(ui, "favorite-regular", Rect::from_center_size(pos2(centre.x, centre.y - 40.0), Vec2::splat(36.0)), pal.text2);
                let g1 = p.layout_no_wrap(crate::trq!("No favorites yet").into_owned(), F::Subtitle.semibold(), pal.text);
                p.galley(pos2(centre.x - g1.size().x / 2.0, centre.y - 6.0), g1, pal.text);
                let g2 = p.layout(if sticker { crate::trq!("Star stickers to save them here.") } else { crate::trq!("Star GIFs to save them here.") }.into_owned(), F::Secondary.font(), pal.text2, grid.width() - 50.0);
                p.galley(pos2(centre.x - g2.size().x / 2.0, centre.y + 20.0), g2, pal.text2);
            } else if n == 0 && !st.loading {
                if let Some((id, sz)) = giphy_art("search_gif.png").and_then(|b| app.icons.png(ui, "giphy-empty", b)) {
                    p.image(id, Rect::from_center_size(pos2(centre.x, centre.y - 60.0), Vec2::new(sz[0] as f32, sz[1] as f32) / 2.0 * 1.4), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                let g1 = p.layout_no_wrap(crate::tr!("ExchangeListPopup", "No results").into_owned(), F::Subtitle.semibold(), pal.text);
                p.galley(pos2(centre.x - g1.size().x / 2.0, centre.y - 6.0), g1, pal.text);
                let g2 = p.layout(crate::trq!("We couldn't find a GIF for that. Try again with more general keywords.").into_owned(), F::Secondary.font(), pal.text2, grid.width() - 50.0);
                p.galley(pos2(centre.x - g2.size().x / 2.0, centre.y + 20.0), g2, pal.text2);
            } else {
                let mut gui = ui.new_child(egui::UiBuilder::new().max_rect(grid));
                gui.spacing_mut().scroll = egui::style::ScrollStyle { bar_width: 8.0, bar_inner_margin: 1.0, bar_outer_margin: 0.0, ..egui::style::ScrollStyle::solid() };
                let mut sa = crate::ui::widgets::vscroll().id_salt(("gif-scroll-extras", st.view as u8, sticker)).auto_shrink([false; 2]);
                if st.sel.is_none() && st.scroll_to_sel { sa = sa.vertical_scroll_offset(0.0); st.scroll_to_sel = false; }
                let so = sa.show_viewport(&mut gui, |ui, vp| {
                    let extra = if st.loading { 44.0 } else { 0.0 };
                    let (area, _) = ui.allocate_exact_size(Vec2::new(geo.content_w(), total_h + extra), Sense::hover());
                    for (i, rel) in tiles.iter().enumerate() {
                        let r = rel.translate(area.min.to_vec2());
                        if st.sel == Some(i) && st.scroll_to_sel { ui.scroll_to_rect(r.expand2(Vec2::new(0.0, 6.0)), None); st.scroll_to_sel = false; }
                        if rel.bottom() < vp.top() - 120.0 || rel.top() > vp.bottom() + 120.0 { continue }
                        let g = &st.gifs[i];
                        want(&mut st.thumbs, tx.as_ref(), &g.id, &g.thumb_url);
                        if rel.bottom() < vp.top() || rel.top() > vp.bottom() { continue }
                        let resp = ui.interact(r, egui::Id::new(("gif-tile-extras", &g.id)), Sense::click());
                        ui.painter().rect_filled(r, 6.0, pal.tile);
                        if let Some((tid, _)) = st.thumbs.tex(&ctx, &g.id, now) {
                            egui::Image::from_texture((tid, r.size())).corner_radius(6.0).paint_at(ui, r);
                        }
                        if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                        if resp.hovered() || st.sel == Some(i) { ui.painter().rect_stroke(r, 6.0, Stroke::new(2.0_f32, pal.focus), egui::StrokeKind::Inside); }
                        // Favourite star: shown while the tile is hovered (and always on saved ones), filled when saved. Drawn after the tile so it wins the click.
                        let is_fav = favs.contains(&g.id, sticker);
                        if resp.hovered() || is_fav || ui.rect_contains_pointer(r) {
                            let sr = Rect::from_min_size(pos2(r.right() - 28.0, r.top() + 4.0), Vec2::splat(24.0));
                            let sresp = ui.interact(sr, egui::Id::new(("gif-star", &g.id)), Sense::click());
                            ui.painter().circle_filled(sr.center(), 12.0, Color32::from_black_alpha(if sresp.hovered() { 190 } else { 140 }));
                            app.icons.paint(ui, if is_fav { "favorite-filled" } else { "favorite-regular" }, Rect::from_center_size(sr.center(), Vec2::splat(15.0)), if is_fav { hex(0xF2990A) } else { Color32::WHITE });
                            if sresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                            if sresp.on_hover_text(if is_fav { crate::tr!("AppPickerGridViewDelegate", "Remove from favorites") } else { crate::trq!("Add to favorites") }).clicked() { fav_toggle = Some(i); continue; }
                        }
                        if resp.clicked() { pick = Some(i); }
                    }
                    if st.loading { ui.put(Rect::from_center_size(pos2(area.center().x, area.bottom() - 22.0), Vec2::splat(24.0)), egui::Spinner::new().size(24.0).color(pal.text2)); }
                });
                if !st.loading && st.next_offset < st.total && st.kind.is_some() && so.state.offset.y + so.inner_rect.height() > so.content_size.y - 160.0 { load_more = true; }
            }

            // Footer: Giphy logo (2x asset shown at half size) and the terms link.
            let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOOT_H), rect.right_bottom());
            // Compact: the earlier footer (line in the frame's colour, 11 pt texts, the long link wording).
            p.hline(foot.x_range(), foot.top(), Stroke::new(1.0_f32, if geo.compact { edge } else { pal.edge }));
            if let Some((id, sz)) = giphy_art(if dark { "giphy_logo_white@2x.png" } else { "giphy_logo@2x.png" })
                .and_then(|b| app.icons.png(ui, if dark { "giphy-logo-w" } else { "giphy-logo" }, b)) {
                p.image(id, Rect::from_center_size(pos2(foot.left() + PAD + sz[0] as f32 / 4.0, foot.center().y), Vec2::new(sz[0] as f32, sz[1] as f32) / 2.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            } else if geo.compact {
                p.text(pos2(foot.left() + PAD, foot.center().y), egui::Align2::LEFT_CENTER, "Powered by GIPHY", FontId::proportional(11.0), pal.text2);
            } else {
                p.text(pos2(foot.left() + PAD, foot.center().y), egui::Align2::LEFT_CENTER, "POWERED BY GIPHY", FontId::proportional(10.0), pal.text2);
            }
            let (link, size) = if geo.compact { (crate::trq!("View Giphy terms"), 11.0) } else { (crate::trq!("View terms"), 12.0) };
            let lg = p.layout_no_wrap(link.into_owned(), FontId::proportional(size), pal.link);
            let lr = Rect::from_min_size(pos2(foot.right() - PAD - lg.size().x, foot.center().y - lg.size().y / 2.0), lg.size());
            let lresp = ui.interact(lr.expand(3.0), egui::Id::new("gif-terms-extras"), Sense::click());
            if lresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); p.hline(lr.x_range(), lr.bottom(), Stroke::new(1.0_f32, pal.link)); }
            p.galley(lr.min, lg, pal.link);
            if lresp.on_hover_text(crate::trq!("View Giphy terms")).clicked() { open_terms = true; }
        });

        // The Emojis tab: the emoji picker drawn inside this popup (below the tab row).
        if self.picker.tab == Tab::Emojis {
            let er = Rect::from_min_max(pos2(rect.left(), rect.top() + HEAD_H - 6.0), rect.right_bottom());
            let mut got: Option<String> = None;
            app.emoji_picker_embedded(ui, er, &mut |e| got = Some(e.to_string()));
            if let Some(e) = got { out.emoji = Some(e); close = true; }
        }

        if open_terms { system::open(giphy::TERMS_URL); }
        if let Some(tab) = new_tab { self.set_tab(app, &ctx, tab); }
        if go_home || clear_query { self.picker.debounce.reset(); self.enter(app, View::Home, ""); self.picker.focus = clear_query; }
        if let Some(tile) = open_tile {
            self.picker.debounce.reset();
            match tile {
                Tile::Favs => self.enter(app, View::Favorites, ""),
                Tile::Trending => self.enter(app, View::Trending, ""),
                Tile::Cat(k) => { let name = self.picker.cats.as_ref().and_then(|c| c.get(k)).map(|c| c.query.clone()).unwrap_or_default(); self.enter(app, View::Search, &name); }
            }
        }
        if let Some(i) = fav_toggle {
            if let Some(g) = self.picker.gifs.get(i).cloned() {
                let sticker = self.picker.tab == Tab::Stickers;
                if upload_favs::is_upload_id(&g.id) {
                    // An uploaded GIF can only be unstarred here: the entry goes and so do its local files.
                    let dir = upload_stars::upload_fav_dir(app);
                    if let Some(f) = self.favs(app).remove(&g.id) { upload_favs::unstar(&dir, &f); }
                } else { self.favs(app).toggle(&g, sticker); }
                if self.picker.view == View::Favorites { self.enter(app, View::Favorites, ""); self.picker.scroll_to_sel = false; }
            }
        }
        if load_more {
            if let Some(kind) = self.picker.kind.clone() { let o = self.picker.next_offset; self.picker.fetch(kind, o); self.picker.loading = true; }
        }
        // Click outside (but not on the opening button) closes the popup.
        let pressed = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
        if let Some(pos) = pressed { if !geo.hit().contains(pos) && !anchor.contains(pos) { close = true; } }
        if let Some(i) = pick {
            if let Some(g) = self.picker.gifs.get(i).cloned() {
                if upload_favs::is_upload_id(&g.id) {
                    // A kept GIF / picture goes out as a file (encrypted upload), staged in the composer like an attachment, or sent
                    // at once with "Send favorites on click".
                    match upload_favs::stage_copy(&g) {
                        Some(p) if super::send_instantly(app) => app.send_files_now(space, vec![p], now),
                        Some(p) => crate::app::files::push_picks(space, vec![p]),
                        None => app.toast(crate::trq!("The file no longer exists").into_owned(), now),
                    }
                } else { let b = self.picker.thumbs.get(&g.id); out.pick = Some((g, b)); }
            }
            close = true;
        }
        if close { self.picker.debounce.reset(); }
        out.close = close;
        out
    }
}
