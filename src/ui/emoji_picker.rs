//! The emoji picker popup: a dark popover with a pill search field, a scrolling grid grouped by category with German headings
//! ("Kürzlich verwendet" first), a skin-tone selector and a row of eight category tabs at the bottom (the active tab shows a
//! thick light bar on its top edge).
//!
//! Data: order, groups and German names/keywords come from Unicode `emoji-test.txt` + CLDR `de` annotations (generated into
//! `assets/emoji_data.tsv`); the glyphs are the Twemoji files in `assets/emoji`.
//! Colours are theme tokens: search `wx-emojiPickerSearch-*`, cell `wx-emojiPickerEmoji-background`, category bar
//! `wx-emojiPickerCategory-*`, popup border `wx-popup-widget-border`.
//!
//! Reusable: `App::emoji_picker_toggle(ctx)` opens/closes it, `App::emoji_picker(ui, anchor, &mut |emoji| ..)` draws it
//! (call every frame; it does nothing while closed) and reports a picked emoji through the callback. Picks are recorded in
//! "Kürzlich verwendet" (settings JSON `emoji_recents`, newest first, at most 30).
use crate::*;
use std::sync::OnceLock;

struct Entry { group: u8, ch: String, name: String, hay: String }

fn entries() -> &'static Vec<Entry> {
    static DATA: OnceLock<Vec<Entry>> = OnceLock::new();
    DATA.get_or_init(|| include_str!("../../assets/emoji_data.tsv").lines().filter_map(|l| {
        let mut f = l.split('\t');
        let group: u8 = f.next()?.parse().ok()?;
        let _file = f.next()?;
        let ch = f.next()?.to_string();
        let name = f.next()?.to_string();
        let kw = f.next().unwrap_or("");
        // Only emoji with a colour picture: the picker never draws a monochrome / boxed font glyph.
        if !icons::emoji_exists(&ch) { return None; }
        Some(Entry { group, hay: format!("{} {}", name, kw.replace('|', " ")).to_lowercase(), ch, name })
    }).collect())
}

const TITLES: [&str; 8] = ["Kürzlich verwendet", "Smileys & Emotionen", "Personen & Gesten", "Essen & Trinken", "Reisen & Orte", "Natur & Aktivitäten", "Symbole & Objekte", "Flaggen"];
const TAB_ICONS: [&str; 8] = ["recents-regular", "emoji-happy-regular", "raise-hand-regular", "food-regular", "location-regular", "green-leaf-regular", "favorites-regular", "flag-regular"];
/// Skin tones of the selector (#FFCC4C, #FFDCB9, #E6BB91, #C78E63, #A36134, #5C4437).
const TONES: [u32; 6] = [0xFFCC4C, 0xFFDCB9, 0xE6BB91, 0xC78E63, 0xA36134, 0x5C4437];

const W: f32 = 350.0;
const H: f32 = 390.0;
const CELL: f32 = 40.0;
const GLYPH: f32 = 32.0;
const HEAD_H: f32 = 32.0;
const TAB_H: f32 = 44.0;

#[derive(Clone, Default)]
struct Ui { query: String, sel: Option<usize>, jump: Option<f32>, tone_open: bool, focus: bool, scroll_to_sel: bool }

enum Row { Head(usize, String), Cells(Vec<usize>) }

/// The skin-toned form of an emoji (tone 1..=5 = U+1F3FB..U+1F3FF after the first code point), if the set has it.
fn toned(ch: &str, tone: usize) -> Option<String> {
    if tone == 0 { return None; }
    let mut it = ch.chars();
    let first = it.next()?;
    let rest: String = it.filter(|c| *c != '\u{fe0f}').collect();
    let m = char::from_u32(0x1F3FA + tone as u32)?;
    let s = format!("{first}{m}{rest}");
    icons::emoji_exists(&s).then_some(s)
}

impl App {
    pub(crate) fn emoji_picker_toggle(&mut self, ctx: &egui::Context) {
        let open = ctx.data(|d| d.get_temp::<bool>(egui::Id::new("emoji-open"))).unwrap_or(false);
        ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("emoji-open"), !open);
            d.insert_temp(egui::Id::new("emoji-ui"), Ui { focus: true, ..Default::default() });
        });
    }

    fn emoji_recents(&self) -> Vec<String> {
        self.settings["emoji_recents"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
    }

    fn emoji_record(&mut self, ch: &str) {
        let mut r = self.emoji_recents();
        r.retain(|x| x != ch);
        r.insert(0, ch.to_string());
        r.truncate(30);
        self.settings["emoji_recents"] = json!(r);
        if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
    }

    /// Draws the picker above `anchor` while it is open. `on_pick` receives the chosen emoji (skin tone applied).
    pub(crate) fn emoji_picker(&mut self, ui: &mut egui::Ui, anchor: Rect, on_pick: &mut dyn FnMut(&str)) {
        self.emoji_picker_impl(ui, anchor, on_pick, None, egui::Order::Foreground);
    }

    /// [`Self::emoji_picker`] on a higher layer (e.g. `Order::Debug` above a dialog, which sits on `Order::Tooltip`).
    pub(crate) fn emoji_picker_over(&mut self, ui: &mut egui::Ui, anchor: Rect, order: egui::Order, on_pick: &mut dyn FnMut(&str)) {
        self.emoji_picker_impl(ui, anchor, on_pick, None, order);
    }

    /// The same picker drawn INSIDE another popup (the GIF picker's "Emojis" tab): no frame / caret, laid out in `rect`, and it
    /// never closes itself (Esc / outside clicks belong to the host). Picks are still recorded under "Kürzlich verwendet".
    pub(crate) fn emoji_picker_embedded(&mut self, ui: &mut egui::Ui, rect: Rect, on_pick: &mut dyn FnMut(&str)) {
        self.emoji_picker_impl(ui, rect, on_pick, Some(rect), egui::Order::Foreground);
    }

    /// Fresh search / scroll state with the search field focused (call when the embedded picker is shown).
    pub(crate) fn emoji_picker_reset(&mut self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("emoji-ui"), Ui { focus: true, ..Default::default() }));
    }

    fn emoji_picker_impl(&mut self, ui: &mut egui::Ui, anchor: Rect, on_pick: &mut dyn FnMut(&str), embed: Option<Rect>, order: egui::Order) {
        let ctx = ui.ctx().clone();
        let open_id = egui::Id::new("emoji-open");
        if embed.is_none() && !ctx.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false) { return; }
        let ui_id = egui::Id::new("emoji-ui");
        let mut st: Ui = ctx.data(|d| d.get_temp(ui_id)).unwrap_or_default();
        let tone: usize = self.settings["emoji_tone"].as_u64().unwrap_or(0).min(5) as usize;

        // Placement: above the anchor with a 10 pt caret, clamped to the window.
        let screen = ctx.screen_rect();
        let left = (anchor.center().x - 40.0).clamp(8.0, (screen.right() - W - 8.0).max(8.0));
        let top = (anchor.top() - 10.0 - H).max(8.0);
        let rect = embed.unwrap_or_else(|| Rect::from_min_size(pos2(left, top), Vec2::new(W, H)));

        // Which emoji are shown: the search results, or recents + every category.
        let all = entries();
        let q = st.query.trim().to_lowercase();
        let recents = self.emoji_recents();
        let mut rows: Vec<Row> = Vec::new();
        let mut flat: Vec<usize> = Vec::new();
        let cols = (((W - 24.0) / CELL).floor() as usize).max(1);
        let push_group = |rows: &mut Vec<Row>, flat: &mut Vec<usize>, tab: usize, title: &str, idx: Vec<usize>| {
            if idx.is_empty() { return; }
            rows.push(Row::Head(tab, title.to_string()));
            for c in idx.chunks(cols) { flat.extend_from_slice(c); rows.push(Row::Cells(c.to_vec())); }
        };
        if q.is_empty() {
            let rec: Vec<usize> = recents.iter().filter_map(|r| all.iter().position(|e| &e.ch == r)).collect();
            push_group(&mut rows, &mut flat, 0, TITLES[0], rec);
            for g in 1..=7u8 { push_group(&mut rows, &mut flat, g as usize, TITLES[g as usize], (0..all.len()).filter(|i| all[*i].group == g).collect()); }
        } else {
            let hits: Vec<usize> = (0..all.len()).filter(|i| all[*i].hay.contains(&q)).collect();
            push_group(&mut rows, &mut flat, 0, &format!("{} Suchergebnisse", hits.len()), hits);
        }
        let row_h = |r: &Row| if matches!(r, Row::Head(..)) { HEAD_H } else { CELL };
        let total: f32 = rows.iter().map(row_h).sum::<f32>() + 8.0;
        let mut head_y: Vec<(usize, f32)> = Vec::new();
        { let mut y = 0.0; for r in &rows { if let Row::Head(t, _) = r { head_y.push((*t, y)); } y += row_h(r); } }

        // Keyboard: Esc closes; arrows move the selection (Left/Right only while the search is empty); Enter picks.
        let mut pick: Option<String> = None;
        let mut close = embed.is_none() && ctx.input(|i| i.key_pressed(Key::Escape));
        let n = flat.len();
        if n > 0 {
            let step = |st: &mut Ui, d: isize| { let cur = st.sel.map_or(-1, |s| s as isize); st.sel = Some((cur + d).clamp(0, n as isize - 1) as usize); st.scroll_to_sel = true; };
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) { step(&mut st, cols as isize); }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) { step(&mut st, -(cols as isize)); }
            if q.is_empty() {
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight)) { step(&mut st, 1); }
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft)) { step(&mut st, -1); }
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                let i = st.sel.unwrap_or(0).min(n - 1);
                let e = &all[flat[i]];
                pick = Some(toned(&e.ch, tone).unwrap_or_else(|| e.ch.clone()));
            }
        }

        egui::Area::new(egui::Id::new(("emoji-popup", embed.is_some()))).fixed_pos(rect.min).order(order).interactable(true).show(&ctx, |ui| {
            ui.set_clip_rect(rect.expand2(Vec2::new(2.0, 14.0)));
            let (_, _) = ui.allocate_exact_size(rect.size(), Sense::click());
            let p = ui.painter().clone();
            let t = self.applied.unwrap_or_else(crate::dark);
            let text2 = t.text2;
            // Body + 1 px border (wx-popup-widget-border = white / black @ 15 %) + the caret pointing at the button. Body: dark #0F0F0F;
            // light: wx-emojiPickerEmoji-background (#FFFFFF).
            let body = if t.light { t.tk("wx-emojiPickerEmoji-background") } else { hex(0x0F0F0F) };
            let edge = t.ink_a(0.15);
            if embed.is_none() {
                p.rect_filled(rect, 6.0, body);
                p.rect_stroke(rect, 6.0, Stroke::new(1.0_f32, edge), egui::StrokeKind::Inside);
                let cx = anchor.center().x.clamp(rect.left() + 14.0, rect.right() - 14.0);
                p.add(egui::Shape::convex_polygon(vec![pos2(cx - 8.0, rect.bottom() - 0.5), pos2(cx + 8.0, rect.bottom() - 0.5), pos2(cx, rect.bottom() + 8.0)], body, Stroke::NONE));
                p.line_segment([pos2(cx - 8.0, rect.bottom()), pos2(cx, rect.bottom() + 8.0)], Stroke::new(1.0_f32, edge));
                p.line_segment([pos2(cx + 8.0, rect.bottom()), pos2(cx, rect.bottom() + 8.0)], Stroke::new(1.0_f32, edge));
            }

            // Search pill (28 pt, radius 14, gray-80; blue-50 outline while focused) with the skin-tone dot at its right.
            let sr = Rect::from_min_size(rect.min + Vec2::new(12.0, 12.0), Vec2::new(W - 24.0, 28.0));
            let search_id = egui::Id::new("emoji-search");
            let focused = ui.memory(|m| m.has_focus(search_id));
            p.rect_filled(sr, 14.0, if ui.rect_contains_pointer(sr) && !focused { t.tk("wx-emojiPickerSearch-background:hovered") } else { t.tk("wx-emojiPickerSearch-background") });
            if focused { p.rect_stroke(sr, 14.0, Stroke::new(1.0_f32, t.tk("wx-emojiPickerSearch-border:focused")), egui::StrokeKind::Inside); }
            self.icons.paint(ui, "search-regular", Rect::from_center_size(sr.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(15.0)), text2);
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(sr.min + Vec2::new(34.0, 0.0), sr.max - Vec2::new(54.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let r = c.add(egui::TextEdit::singleline(&mut st.query).id(search_id).hint_text(RichText::new("Suchen").color(text2).size(14.0)).frame(false).margin(Vec2::ZERO).font(FontId::proportional(14.0)).desired_width(sr.width() - 88.0));
            if st.focus { r.request_focus(); st.focus = false; }
            if r.changed() { st.sel = None; st.jump = Some(0.0); }
            if !st.query.is_empty() {
                let xr = Rect::from_center_size(pos2(sr.right() - 38.0, sr.center().y), Vec2::splat(18.0));
                let xresp = ui.interact(xr, egui::Id::new("emoji-clear"), Sense::click());
                self.icons.paint(ui, "cancel-regular", Rect::from_center_size(xr.center(), Vec2::splat(11.0)), if xresp.hovered() { t.text1 } else { text2 });
                if xresp.clicked() { st.query.clear(); st.jump = Some(0.0); }
            }
            let tr = Rect::from_center_size(pos2(sr.right() - 15.0, sr.center().y), Vec2::splat(20.0));
            let tresp = ui.interact(tr, egui::Id::new("emoji-tone"), Sense::click());
            p.circle_filled(tr.center(), 7.5, hex(TONES[tone]));
            if tresp.hovered() { p.circle_stroke(tr.center(), 9.0, Stroke::new(1.0_f32, t.tk("wx-emojiPickerSkin-background:pressed"))); }
            if tresp.clicked() { st.tone_open = !st.tone_open; }

            // Grid: virtualised (only the visible rows are drawn), headings in semibold.
            let grid = Rect::from_min_max(pos2(rect.left() + 12.0, sr.bottom() + 8.0), pos2(rect.right() - 2.0, rect.bottom() - TAB_H));
            let mut gui = ui.new_child(egui::UiBuilder::new().max_rect(grid));
            let mut sa = crate::ui::widgets::vscroll().id_salt("emoji-scroll").auto_shrink([false; 2]);
            if let Some(y) = st.jump.take() { sa = sa.vertical_scroll_offset(y); }
            else if st.scroll_to_sel {
                if let Some(s) = st.sel { // keep the selected cell in view
                    let mut y = 0.0; let mut count = 0usize; let mut target = None;
                    for r in &rows { match r { Row::Head(..) => y += HEAD_H, Row::Cells(c) => { if s >= count && s < count + c.len() { target = Some(y); } count += c.len(); y += CELL; } } }
                    if let Some(ty) = target {
                        let cur = ctx.data(|d| d.get_temp::<f32>(egui::Id::new("emoji-offset"))).unwrap_or(0.0);
                        if ty < cur { sa = sa.vertical_scroll_offset(ty - HEAD_H.min(ty)); } else if ty + CELL > cur + grid.height() { sa = sa.vertical_scroll_offset(ty + CELL - grid.height()); }
                    }
                }
                st.scroll_to_sel = false;
            }
            gui.spacing_mut().scroll = egui::style::ScrollStyle { bar_width: 8.0, bar_inner_margin: 1.0, bar_outer_margin: 0.0, ..egui::style::ScrollStyle::solid() };
            let out = sa.show_viewport(&mut gui, |ui, vp| {
                let (area, _) = ui.allocate_exact_size(Vec2::new(grid.width() - 12.0, total), Sense::hover());
                if rows.iter().all(|r| matches!(r, Row::Head(..))) || rows.is_empty() {
                    // Nothing found: illustration and message.
                    let c = pos2(area.center().x, area.top() + 90.0);
                    if q.is_empty() { return; }
                    self.icons.art(ui, "emojipicker_illustration", Rect::from_center_size(c, Vec2::splat(110.0)));
                    let g = ui.painter().layout("Wir finden kein passendes Emoji. Versuche es mit etwas anderem, es gibt eine große Auswahl.".to_string(), FontId::proportional(13.5), text2, area.width() - 40.0);
                    ui.painter().galley(pos2(area.left() + 20.0, c.y + 70.0), g, text2);
                    return;
                }
                let mut y = 0.0;
                let mut count = 0usize;
                for r in &rows {
                    let h = row_h(r);
                    let visible = y + h >= vp.top() - 1.0 && y <= vp.bottom() + 1.0;
                    match r {
                        Row::Head(_, title) => if visible { ui.painter().text(pos2(area.left() + 2.0, area.top() + y + HEAD_H * 0.6), egui::Align2::LEFT_CENTER, title, semibold(14.0), t.text1); },
                        Row::Cells(cells) => {
                            for (k, &ei) in cells.iter().enumerate() {
                                if visible {
                                    let e = &all[ei];
                                    let cell = Rect::from_min_size(area.min + Vec2::new(k as f32 * CELL, y), Vec2::splat(CELL));
                                    let resp = ui.interact(cell, egui::Id::new(("emoji-cell", ei, count + k)), Sense::click());
                                    let selected = st.sel == Some(count + k);
                                    if selected { ui.painter().rect_filled(cell.shrink(1.0), 6.0, t.tk("wx-emojiPickerEmoji-background:focused")); } else if resp.hovered() { ui.painter().rect_filled(cell.shrink(1.0), 6.0, t.tk("wx-emojiPickerEmoji-background:hovered")); }
                                    let shown = toned(&e.ch, tone).unwrap_or_else(|| e.ch.clone());
                                    let gr = Rect::from_center_size(cell.center(), Vec2::splat(GLYPH));
                                    // Colour picture only (entries without one were dropped from the list); a failed rasterise stays blank.
                                    let _ = self.icons.emoji(ui, &shown, gr);
                                    if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                                    if resp.clone().on_hover_text(&e.name).clicked() { pick = Some(shown); }
                                }
                            }
                            count += cells.len();
                        }
                    }
                    y += h;
                }
            });
            let offset = out.state.offset.y;
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("emoji-offset"), offset));

            // Category bar: 8 tabs, 1 px gray-80 top edge, the active one with a 4 px light segment (gray-05); disabled = gray-80.
            let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - TAB_H), rect.right_bottom());
            p.hline(bar.x_range(), bar.top(), Stroke::new(1.0_f32, t.tk("wx-emojiPickerCategory-border-top")));
            let active = head_y.iter().rev().find(|(_, y)| *y <= offset + 8.0).map(|(t, _)| *t).unwrap_or(0);
            // WEBOX_SHOT_EMOJI_TAB=<n> (screenshot runs only) scrolls to that category on the first frames.
            if self.shot.is_some() && self.frames > 2 && self.frames < 6 {
                if let Some(n) = std::env::var("WEBOX_SHOT_EMOJI_TAB").ok().and_then(|v| v.parse::<usize>().ok()) {
                    if let Some((_, y)) = head_y.iter().find(|(t, _)| *t == n) { st.jump = Some(*y); }
                }
            }
            let tw = bar.width() / 8.0;
            for tab in 0..8usize {
                let tr = Rect::from_min_size(bar.min + Vec2::new(tab as f32 * tw, 0.0), Vec2::new(tw, TAB_H));
                let target = head_y.iter().find(|(t, _)| *t == tab).map(|(_, y)| *y);
                let enabled = target.is_some() && q.is_empty();
                let resp = ui.interact(tr, egui::Id::new(("emoji-tab", tab)), if enabled { Sense::click() } else { Sense::hover() });
                let on = q.is_empty() && active == tab && enabled;
                if on { p.rect_filled(Rect::from_min_size(tr.min, Vec2::new(tw, 4.0)), 0.0, t.tk("wx-emojiPickerCategory-border-top:checked")); }
                else if enabled && resp.hovered() { p.rect_filled(Rect::from_min_size(tr.min, Vec2::new(tw, 4.0)), 0.0, t.tk("wx-emojiPickerCategory-border-top:hovered")); }
                self.icons.paint(ui, TAB_ICONS[tab], Rect::from_center_size(tr.center() + Vec2::new(0.0, 2.0), Vec2::splat(20.0)), if enabled || !q.is_empty() && tab != 0 { t.tk("wx-emojiPickerCategory-icon") } else { t.tk("wx-emojiPickerCategory-icon:disabled") });
                if enabled { if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); } if resp.on_hover_text(TITLES[tab]).clicked() { st.jump = target; } }
            }

            // Skin-tone chooser under the search field.
            if st.tone_open {
                let tb = Rect::from_min_size(pos2(sr.right() - 6.0 * 26.0 - 8.0, sr.bottom() + 4.0), Vec2::new(6.0 * 26.0 + 8.0, 30.0));
                p.rect_filled(tb, 8.0, t.hover); // listItem-primary hovered (#292929 / #EDEDED)
                p.rect_stroke(tb, 8.0, Stroke::new(1.0_f32, edge), egui::StrokeKind::Inside);
                for (i, col) in TONES.iter().enumerate() {
                    let c = pos2(tb.left() + 17.0 + i as f32 * 26.0, tb.center().y);
                    let r = ui.interact(Rect::from_center_size(c, Vec2::splat(24.0)), egui::Id::new(("emoji-tone-pick", i)), Sense::click());
                    if r.hovered() || i == tone { p.circle_filled(c, 12.0, if i == tone { t.tk("wx-emojiPickerSkin-background:checked") } else { t.tk("wx-emojiPickerSkin-background:pressed") }); }
                    p.circle_filled(c, 8.0, hex(*col));
                    if r.clicked() { self.settings["emoji_tone"] = json!(i); if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); } st.tone_open = false; }
                }
            }
        });

        // Click outside (but not on the opening button) closes the popup.
        let pressed = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
        if let Some(pos) = pressed { if embed.is_none() && !rect.expand2(Vec2::new(0.0, 12.0)).contains(pos) && !anchor.contains(pos) { close = true; } }
        if let Some(e) = pick { self.emoji_record(&e); on_pick(&e); close = embed.is_none(); }
        if close { ctx.data_mut(|d| d.insert_temp(open_id, false)); } else { ctx.data_mut(|d| d.insert_temp(ui_id, st)); }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn norm(s: &str) -> String { s.chars().filter(|c| *c != '\u{fe0f}').map(|c| format!("{:x}", c as u32)).collect::<Vec<_>>().join("-") }

    /// Every picker entry either has NO picture (the picker then uses the system emoji font) or resolves to the emoji
    /// file whose name is exactly its code-point sequence - and that file is emoji artwork, never an icon.
    #[test]
    fn every_entry_resolves_to_its_own_emoji_file() {
        let icons: std::collections::HashSet<&[u8]> = icons::ICON_TABLE.iter().map(|e| e.1).collect();
        for e in entries() {
            let key = icons::emoji_key(&e.ch).unwrap_or_else(|| panic!("{} ({}) has no colour picture", e.name, e.ch));
            assert_eq!(key.replace("-fe0f", ""), norm(&e.ch), "{} ({}) resolved to the wrong file {key}", e.name, e.ch);
            let data = icons::EMOJI_TABLE.iter().find(|x| x.0 == key).expect("file in table").1;
            assert!(!icons.contains(data), "{} ({}) resolved to an icon file {key}", e.name, e.ch);
            assert!(std::str::from_utf8(data).unwrap_or("").contains("viewBox=\"0 0 36 36\""), "{key} is not 36x36 colour artwork");
        }
        assert!(entries().len() > 1900, "only {} picker emoji", entries().len());
    }

    /// The generated list may hold only a handful of emoji newer than the Twemoji set (they are dropped from the picker).
    #[test]
    fn only_a_few_listed_emoji_lack_a_picture() {
        let total = include_str!("../../assets/emoji_data.tsv").lines().count();
        assert_eq!(total - entries().len(), 9, "listed {total}, shown {}", entries().len());
    }
}
