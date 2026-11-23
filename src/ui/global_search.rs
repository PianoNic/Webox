//! The global search popup (header pill "Suchen, treffen und anrufen", Strg+F).
//!
//! Layout: `modal-primary-background` (#000000 dark; secondary #1A1A1A / light #F7F7F7), a 2 px `modal-primary-border`
//! (white @ 20 %), search field 28 pt high at 20 pt from the top with 24 pt side margins and a round ghost close button, tab
//! bar 28 pt high, result list on the left (50 % of the width), a 250 pt "Filter" column on the right in
//! `modal-secondary-background` (gray-97 #1A1A1A, radius 8) with the "Personen" and "Bereiche" filter fields; section headings
//! 32 pt (`text-secondary`); rows 50 pt, radius 25, 32 pt avatar at 8 pt, title 15 pt on `listitem-textprimary`, second line
//! `text-secondary`, space activity time at the right, round S action buttons "Nachricht" / "Anrufen" / "Video" shown on
//! hover or selection; "Mehr anzeigen" / "Weniger anzeigen" rows (28 pt, link colour); a lighthouse illustration with "Nach
//! Personen, Bereichen und Geräten suchen" before typing, a desert illustration with "Keine Ergebnisse gefunden"; tabs
//! Personen / Bereiche / Nachrichten. Webox's own choices: popup size / position (640 x 560 under the header), the tab "Alle",
//! 3 results per section, the message row layout, the button colour; there is no recents list.
//! Data: `App.people`, `App.spaces`, the loaded `App.messages`: all local, nothing is sent.
use crate::*;

#[derive(Clone, Default, PartialEq)]
struct Gs { open: bool, q: String, tab: u8, sel: usize, more: [bool; 3], pf: String, sf: String, focus: bool }
fn key() -> egui::Id { egui::Id::new("global-search-state") }
fn get(ctx: &egui::Context) -> Gs { ctx.data(|d| d.get_temp::<Gs>(key())).unwrap_or_default() }
fn set(ctx: &egui::Context, g: Gs) { ctx.data_mut(|d| d.insert_temp(key(), g)); }
fn pill_key() -> egui::Id { egui::Id::new("global-search-pill") }

pub(crate) fn open(ctx: &egui::Context) { set(ctx, Gs { open: true, focus: true, ..Default::default() }); }
pub(crate) fn close(ctx: &egui::Context) { let mut g = get(ctx); g.open = false; set(ctx, g); }
pub(crate) fn toggle(ctx: &egui::Context) { if get(ctx).open { close(ctx) } else { open(ctx) } }
/// The header pill's rect (so a click on it is not taken as "outside").
pub(crate) fn set_pill(ctx: &egui::Context, r: Rect) { ctx.data_mut(|d| d.insert_temp(pill_key(), r)); }
/// `WEBOX_SHOT_SEARCH=<query>` (screenshots only): opens the popup with that query typed.
pub(crate) fn shot_open(ctx: &egui::Context, q: &str) { set(ctx, Gs { open: true, q: q.into(), ..Default::default() }); }

const LIMIT: usize = 3; // results per section before "Mehr anzeigen"
const NAMES: [&str; 3] = ["Personen", "Bereiche", "Nachrichten"];

#[derive(Clone)]
enum Row { Section(&'static str), Person(Person), Space(Space), Msg(String, String, Message), More(usize), Less(usize), Placeholder, NoFound }
impl Row { fn selectable(&self) -> bool { !matches!(self, Row::Section(_) | Row::Placeholder | Row::NoFound) } fn height(&self) -> f32 { match self { Row::Section(_) => 32.0, Row::More(_) | Row::Less(_) => 28.0, Row::Placeholder | Row::NoFound => 176.0, _ => 50.0 } } }

fn contains(hay: &str, needle: &str) -> bool { needle.is_empty() || hay.to_lowercase().contains(needle) }

/// Title with the matches in semibold.
fn hl(text: &str, q: &str, size: f32, col: Color32, max_w: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    let reg = TextFormat { font_id: FontId::proportional(size), color: col, ..Default::default() };
    let bold = TextFormat { font_id: semibold(size), color: col, ..Default::default() };
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
    job.wrap = egui::text::TextWrapping { max_width: max_w.max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    job
}

impl App {
    fn gs_rows(&self, g: &Gs) -> Vec<Row> {
        let (q, pf, sf) = (g.q.trim().to_lowercase(), g.pf.trim().to_lowercase(), g.sf.trim().to_lowercase());
        if q.is_empty() && pf.is_empty() && sf.is_empty() { return vec![Row::Placeholder]; }
        let people: Vec<Person> = self.people.iter().filter(|p| (contains(&p.name, &q) || contains(&p.email, &q)) && contains(&p.name, &pf) && sf.is_empty()).cloned().collect();
        let spaces: Vec<Space> = self.spaces.iter().filter(|s| !s.hidden && !s.direct && contains(&s.title, &q) && contains(&s.title, &sf) && pf.is_empty()).cloned().collect();
        let mut msgs: Vec<(String, String, Message)> = Vec::new();
        if !q.is_empty() {
            for s in self.spaces.iter().filter(|s| !s.hidden && contains(&s.title, &sf)) {
                for m in self.messages.get(&s.id).into_iter().flatten().filter(|m| m.notice.is_empty() && m.text.to_lowercase().contains(&q) && contains(&m.author, &pf)) {
                    msgs.push((s.id.clone(), s.title.clone(), m.clone()));
                }
            }
            msgs.sort_by(|a, b| (b.2.date.as_str(), b.2.time.as_str()).cmp(&(a.2.date.as_str(), a.2.time.as_str())));
        }
        let mut rows = Vec::new();
        let mut add = |i: usize, items: Vec<Row>| {
            if items.is_empty() || (g.tab != 0 && g.tab as usize != i + 1) { return; }
            rows.push(Row::Section(NAMES[i]));
            let all = g.tab != 0 || g.more[i];
            let n = items.len();
            rows.extend(items.into_iter().take(if all { usize::MAX } else { LIMIT }));
            if g.tab == 0 && n > LIMIT { rows.push(if g.more[i] { Row::Less(i) } else { Row::More(i) }); }
        };
        add(0, people.into_iter().map(Row::Person).collect());
        add(1, spaces.into_iter().map(Row::Space).collect());
        add(2, msgs.into_iter().map(|(a, b, c)| Row::Msg(a, b, c)).collect());
        if rows.is_empty() { rows.push(Row::NoFound); }
        rows
    }

    /// Picks a result: space / chat / card; closes the popup.
    fn gs_pick(&mut self, ctx: &egui::Context, row: &Row, how: u8, now: f64) {
        match row {
            Row::Person(p) => {
                if how == 2 || how == 3 {
                    if p.email.is_empty() { self.toast("Calls work in direct spaces", now) } else { self.start_call(&p.email, &p.name, now) }
                } else if !self.nav_open_chat(p) { Self::contact_card_open(ctx, p); }
            }
            Row::Space(s) => self.open_space(s.id.clone()),
            Row::Msg(id, _, m) => self.goto_message(ctx, id, &m.id), // open the space AT the message (it flashes)
            _ => return,
        }
        close(ctx);
    }

    pub(crate) fn global_search_popup(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let mut g = get(ctx);
        if !g.open { return; }
        let start = g.clone();
        let light = super::menu::light(t);
        let (bg, bg2, border, text, text2) = if light { (Color32::WHITE, hex(0xF7F7F7), alpha(Color32::BLACK, 0.2), alpha(Color32::BLACK, 0.95), alpha(Color32::BLACK, 0.7)) }
            else { (hex(0x000000), hex(0x1A1A1A), alpha(Color32::WHITE, 0.2), alpha(Color32::WHITE, 0.95), alpha(Color32::WHITE, 0.7)) };
        let screen = ctx.screen_rect();
        let pill: Rect = ctx.data(|d| d.get_temp(pill_key())).unwrap_or(Rect::from_center_size(pos2(screen.center().x, 26.0), Vec2::new(354.0, 30.0)));
        let (w, h) = (640.0_f32.min(screen.width() - 16.0), 560.0_f32.min(screen.height() - 16.0));
        let pos = pos2((pill.center().x - w / 2.0).clamp(8.0, (screen.right() - w - 8.0).max(8.0)), 8.0);
        let outer = Rect::from_min_size(pos, Vec2::new(w, h));
        let rows = self.gs_rows(&g);
        let n = rows.len();
        // Keyboard (consumed first so the text field doesn't see Up / Down / Enter).
        let mut moved = false;
        let sel_ok = |i: usize| rows.get(i).is_some_and(|r| r.selectable());
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) { let mut i = g.sel + 1; while i < n && !sel_ok(i) { i += 1; } if i < n { g.sel = i; moved = true; } }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) { let mut i = g.sel as isize - 1; while i >= 0 && !sel_ok(i as usize) { i -= 1; } if i >= 0 { g.sel = i as usize; moved = true; } }
        if g.sel >= n || !sel_ok(g.sel) { g.sel = (0..n).find(|&i| sel_ok(i)).unwrap_or(0); }
        let mut action: Option<(usize, u8)> = None;
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) && sel_ok(g.sel) { action = Some((g.sel, 0)); }
        let mut do_close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let tab_before = g.tab;
        let mut g2 = g.clone();
        let hover_moves = ctx.input(|i| i.pointer.delta() != Vec2::ZERO);
        egui::Area::new(egui::Id::new("global-search-area")).fixed_pos(outer.min).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.allocate_exact_size(outer.size(), Sense::hover());
            let p = ui.painter().clone();
            p.add(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if light { 50 } else { 170 }) }.as_shape(outer, 12.0));
            p.rect_filled(outer, 12.0, bg);
            p.rect_stroke(outer, 12.0, Stroke::new(2.0_f32, border), egui::StrokeKind::Inside);
            // Field row: field 28 pt high, 20 pt from the top, 24 pt side margins, close button 28 pt.
            let field = Rect::from_min_size(outer.min + Vec2::new(24.0, 20.0), Vec2::new(w - 48.0 - 36.0, 28.0));
            p.rect_filled(field, 14.0, if light { hex(0xEDEDED) } else { hex(0x3B3B3B) });
            p.rect_stroke(field, 14.0, Stroke::new(1.0_f32, t.accent), egui::StrokeKind::Inside); // `focus` is the keyboard-focus ring
            self.icons.paint(ui, "search-regular", Rect::from_center_size(field.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(16.0)), text);
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(field.min + Vec2::new(34.0, 0.0), field.max - Vec2::new(12.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let r = c.add(egui::TextEdit::singleline(&mut g2.q).id(egui::Id::new("gs-input")).hint_text(RichText::new(tr("Search, meet, and call")).color(text2).size(14.5)).font(FontId::proportional(14.5)).frame(false).desired_width(field.width() - 46.0));
            if std::mem::take(&mut g2.focus) { r.request_focus(); }
            let xr = Rect::from_center_size(pos2(outer.right() - 24.0 - 14.0, field.center().y), Vec2::splat(28.0));
            let xresp = ui.interact(xr, egui::Id::new("gs-close"), Sense::click());
            if xresp.hovered() { p.circle_filled(xr.center(), 14.0, t.ink_a(0.12)); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            self.icons.paint(ui, "cancel_16", Rect::from_center_size(xr.center(), Vec2::splat(16.0)), text);
            if xresp.on_hover_text("Suchdialogfeld schließen").clicked() { do_close = true; }
            // Tabs.
            let trow = Rect::from_min_size(outer.min + Vec2::new(24.0, 56.0), Vec2::new(w - 48.0, 28.0));
            let mut th = ui.new_child(egui::UiBuilder::new().max_rect(trow).layout(egui::Layout::left_to_right(egui::Align::Center)));
            th.spacing_mut().item_spacing.x = 8.0;
            for (i, name) in ["Alle", "Personen", "Bereiche", "Nachrichten"].iter().enumerate() {
                if text_tab(&mut th, name, g2.tab == i as u8, t, t.ink_a(0.2)).clicked() { g2.tab = i as u8; }
            }
            // Left column: the result list (50 % of the width), right column: filters (250 pt).
            let top = outer.top() + 96.0;
            let left = Rect::from_min_max(pos2(outer.left(), top), pos2(outer.left() + (w * 0.5).max(340.0).min(w - 274.0), outer.bottom() - 8.0));
            let mut lst = ui.new_child(egui::UiBuilder::new().max_rect(left).layout(egui::Layout::top_down(egui::Align::Min)));
            lst.set_clip_rect(left.intersect(ui.clip_rect()));
            let q = g.q.trim().to_string();
            crate::ui::widgets::vscroll().id_salt("gs-scroll").auto_shrink([false; 2]).show(&mut lst, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for (i, row) in rows.iter().enumerate() {
                    let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row.height()), Sense::hover());
                    let r = Rect::from_min_max(full.min + Vec2::new(8.0, 0.0), full.max - Vec2::new(8.0, 0.0));
                    match row {
                        Row::Section(s) => { ui.painter().text(pos2(r.left() + 4.0, r.center().y), egui::Align2::LEFT_CENTER, s, FontId::proportional(13.5), text2); }
                        Row::Placeholder | Row::NoFound => {
                            let (art, msg) = if matches!(row, Row::Placeholder) { ("lighthouse-filter-192-cobalt-lime", "Nach Personen, Bereichen und Geräten suchen") } else { ("desert-open-results-192-cobalt-lime", "Keine Ergebnisse gefunden") };
                            self.icons.art(ui, art, Rect::from_center_size(pos2(r.left() + 48.0, r.center().y), Vec2::splat(80.0)));
                            let job = { let mut j = egui::text::LayoutJob::simple(msg.into(), FontId::proportional(13.5), text2, r.width() - 112.0); j.wrap.max_rows = 4; j };
                            let gal = ui.painter().layout_job(job);
                            ui.painter().galley(pos2(r.left() + 100.0, r.center().y - gal.size().y / 2.0), gal, text2);
                        }
                        Row::More(k) | Row::Less(k) => {
                            let resp = ui.interact(r, ui.id().with(("gs-more", i)), Sense::click());
                            let on = g.sel == i || resp.hovered();
                            if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); if hover_moves { g2.sel = i; } }
                            if on { ui.painter().rect_filled(r, 14.0, t.ink_a(0.05)); }
                            if matches!(row, Row::More(_)) { ui.painter().text(pos2(r.left() + 4.0, r.center().y), egui::Align2::LEFT_CENTER, "Mehr anzeigen", FontId::proportional(14.0), t.link); }
                            else {
                                ui.painter().text(pos2(r.left() + 4.0, r.center().y), egui::Align2::LEFT_CENTER, NAMES[*k], FontId::proportional(13.5), text2);
                                ui.painter().text(pos2(r.right() - 4.0, r.center().y), egui::Align2::RIGHT_CENTER, "Weniger anzeigen", FontId::proportional(14.0), t.link);
                            }
                            if resp.clicked() || (action == Some((i, 0))) { g2.more[*k] = matches!(row, Row::More(_)); action = None; }
                        }
                        _ => {
                            let resp = ui.interact(r, ui.id().with(("gs-row", i)), Sense::click());
                            if resp.hovered() && hover_moves { g2.sel = i; }
                            let on = g.sel == i || resp.hovered();
                            if on { ui.painter().rect_filled(r, 25.0, t.ink_a(0.05)); }
                            if moved && g.sel == i { ui.scroll_to_rect(r, None); }
                            if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                            let (title, sub, sub_col, status, photo, space_av, when_): (String, String, Color32, Option<String>, Option<egui::TextureId>, Option<Color32>, String) = match row {
                                Row::Person(pp) => {
                                    let live = self.live_person(pp);
                                    let ph = self.photo_for(ctx, &live.id, &live.email);
                                    (live.name.clone(), self.person_line(&live, super::chrome::now_ms()), text2, Some(live.status.clone()), ph, None, String::new())
                                }
                                Row::Space(s) => {
                                    let team = self.teams.iter().find(|x| x.id == s.team).map(|x| x.name.clone());
                                    let (sub, col) = match &team { Some(n) => (n.clone(), team_color(t, &s.team)), None if s.public => (tr("Public").to_string(), hex(0xACB2D7)), None => (String::new(), text2) };
                                    (s.title.clone(), sub, col, None, None, Some(team.as_ref().map_or(t.avatar, |_| team_color(t, &s.team).gamma_multiply(0.62))), s.last.clone())
                                }
                                Row::Msg(_, st_, m) => {
                                    let ph = self.photo_for(ctx, "", &m.email);
                                    (m.text.lines().next().unwrap_or("").to_string(), format!("{} · {} · {}", m.author, st_, m.time), text2, None, ph, None, String::new())
                                }
                                _ => unreachable!(),
                            };
                            let av = Rect::from_center_size(pos2(r.left() + 8.0 + 16.0, r.center().y), Vec2::splat(32.0));
                            let is_space = matches!(row, Row::Space(_));
                            let mut avc = ui.new_child(egui::UiBuilder::new().max_rect(av));
                            avatar_photo(&mut avc, &mut self.icons, t, &title, 32.0, is_space, space_av, status.as_deref(), photo);
                            let tx = r.left() + 8.0 + 32.0 + 10.0;
                            let btns = if matches!(row, Row::Person(_)) && on { 3 } else { 0 };
                            let right = r.right() - 12.0 - if btns > 0 { btns as f32 * 36.0 } else { 0.0 } - if !when_.is_empty() { 40.0 } else { 0.0 };
                            let lay = ui.painter().layout_job(hl(&title, &q, 15.0, text, right - tx));
                            let two = !sub.is_empty();
                            ui.painter().galley(pos2(tx, if two { r.center().y - 17.0 } else { r.center().y - lay.size().y / 2.0 }), lay, text);
                            if two { super::chrome::fit_text(ui.painter(), tx, r.center().y + 10.0, &sub, FontId::proportional(13.0), sub_col, right - tx); }
                            if !when_.is_empty() { ui.painter().text(pos2(r.right() - 12.0, r.center().y - 8.0), egui::Align2::RIGHT_CENTER, &when_, FontId::proportional(12.5), text2); }
                            let mut btn_hit = false;
                            if btns > 0 {
                                // S round buttons (28 pt), normal / hovered / pressed: Message style = button-message-fill-background, Join style =
                                // button-join-fill-background (the same in MomentumDark and MomentumDefault).
                                for (k, (icon, how, fill)) in [("chat-bold", 1u8, [hex(0x1170CF), hex(0x0353A8), hex(0x063A75)]), ("handset-bold", 2, [hex(0x1D805F), hex(0x185E46), hex(0x134231)]), ("video-bold", 3, [hex(0x1D805F), hex(0x185E46), hex(0x134231)])].into_iter().enumerate() {
                                    let br = Rect::from_center_size(pos2(r.right() - 12.0 - 14.0 - (2 - k) as f32 * 36.0, r.center().y), Vec2::splat(28.0));
                                    let bresp = ui.interact(br, ui.id().with(("gs-btn", i, how)), Sense::click());
                                    if bresp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); btn_hit = true; }
                                    ui.painter().circle_filled(br.center(), 14.0, if bresp.is_pointer_button_down_on() { fill[2] } else if bresp.hovered() { fill[1] } else { fill[0] });
                                    self.icons.paint(ui, icon, Rect::from_center_size(br.center(), Vec2::splat(16.0)), Color32::WHITE);
                                    if bresp.on_hover_text(["Chat", "Anrufen", "Videoanruf"][k]).clicked() { action = Some((i, how)); }
                                }
                            }
                            if resp.clicked() && !btn_hit { action = Some((i, 0)); }
                        }
                    }
                }
            });
            // Right column: "Filter" with the two filter fields (modal-secondary-background, radius 8).
            let rc = Rect::from_min_max(pos2(outer.right() - 24.0 - 250.0 + 12.0, top), pos2(outer.right() - 14.0, top + 178.0));
            p.rect_filled(rc, 8.0, bg2);
            p.text(pos2(rc.left() + 12.0, rc.top() + 18.0), egui::Align2::LEFT_CENTER, "Filter", FontId::proportional(14.0), text);
            for (k, (label, val)) in [("Personen", 0), ("Bereiche", 1)].into_iter().enumerate() {
                let y = rc.top() + 40.0 + 68.0 * k as f32;
                p.text(pos2(rc.left() + 12.0, y + 6.0), egui::Align2::LEFT_CENTER, label, FontId::proportional(12.5), text2);
                let fr = Rect::from_min_size(pos2(rc.left() + 12.0, y + 16.0), Vec2::new(rc.width() - 24.0, 32.0));
                p.rect_filled(fr, 8.0, if light { hex(0xEDEDED) } else { hex(0x3B3B3B) });
                self.icons.paint(ui, "search-regular", Rect::from_center_size(fr.left_center() + Vec2::new(14.0, 0.0), Vec2::splat(14.0)), text2);
                let mut fc = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(fr.min + Vec2::new(28.0, 0.0), fr.max - Vec2::new(8.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                let target = if val == 0 { &mut g2.pf } else { &mut g2.sf };
                fc.add(egui::TextEdit::singleline(target).id(egui::Id::new(("gs-filter", val))).hint_text(RichText::new("Suchen").color(text2).size(13.5)).font(FontId::proportional(13.5)).frame(false).desired_width(fr.width() - 40.0));
            }
        });
        if tab_before != g2.tab || g2.q != g.q || g2.pf != g.pf || g2.sf != g.sf { g2.sel = 0; } // (a non-selectable row 0 is corrected next frame)
        // Click outside (the header pill toggles itself).
        if let Some(pt) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None }) {
            if !outer.expand(2.0).contains(pt) && !pill.contains(pt) && !super::menu::menu_is_open(ctx, "contact-more") { do_close = true; }
        }
        if let Some((i, how)) = action { if let Some(row) = rows.get(i).cloned() { self.gs_pick(ctx, &row, how, now); do_close = true; } }
        g2.open = !do_close;
        // One more frame only when the popup's state changed (selection corrected, query / tab changed, closed); input, the text
        // cursor and arriving data wake it otherwise.
        if g2 != start { ctx.request_repaint(); }
        set(ctx, g2);
    }
}
