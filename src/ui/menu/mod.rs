//! The popup toolkit: one implementation for every menu, popover and dialog of Webox, in the Webex desktop look.
//!
//! Theme tokens: `menulistbackground-primary-background` = #000000, `...-border` = white @ 20 %, `popover-primary-background`
//! = #000000, `modal-primary-background` = #000000, `modal-primary-border` = white @ 20 %, `listitem-textprimary` = white @ 95 %,
//! `listitem-textsecondary` = white @ 70 %, disabled = white @ 40 % (dark theme).
//!
//! Geometry: list width 284, item height 40, item radius 8, left icon 24, icon-text spacing 8, item left margin 8,
//! balloon radius 12, caret 12, outer padding 8 (the caret side is 20 = caret 12 + 8), separators take 11 px.
//! Item fill = `listitem-background`: hovered / focused white 7 %, pressed 11 %.
#![allow(dead_code)] // a toolkit: other modules (messages, calls) pick what they need
use crate::*;

#[derive(Clone)]
pub(crate) struct Item { pub id: String, pub icon: Option<&'static str>, pub label: String, pub enabled: bool, pub danger: bool, pub checked: bool, pub shortcut: Option<String>,
    /// A chevron at the right: the item opens a submenu (the caller opens it; see `menu_item_rect`).
    pub submenu: bool,
    /// A round avatar (name for the initials, optional photo) instead of an icon; makes the row 48 pt high.
    pub avatar: Option<(String, Option<egui::TextureId>)>,
    /// A presence dot instead of an icon.
    pub dot: Option<Color32>,
    /// The dark rounded fill of a prompt row (e.g. "Legen Sie einen Status fest.").
    pub boxed: bool,
    /// A second, dimmer line under the label (the avatar menu's "Verfügbarkeit" / "<presence>"); makes the row 48 high.
    pub sub: Option<String> }

/// The avatar menu's header card: cover banner, big avatar on its lower edge, name and e-mail centred. Drawn edge to edge
/// at the top of the balloon.
#[derive(Clone)]
pub(crate) struct ProfileCard { pub name: String, pub email: String, pub photo: Option<egui::TextureId>, pub cover: Option<(egui::TextureId, [usize; 2])> }

#[derive(Clone)]
pub(crate) enum Entry { Item(Item), Sep, Title(String), Profile(ProfileCard) }

/// Profile card geometry (pt): card height, banner height, avatar size and the name / e-mail / first item positions.
const CARD_H: f32 = 236.0;
const CARD_BANNER: f32 = 102.0;
const CARD_AVATAR: f32 = 87.0;
const CARD_GAP: f32 = 7.0;

impl Item {
    pub(crate) fn new(id: &str, label: impl Into<String>) -> Self { Item { id: id.into(), icon: None, label: label.into(), enabled: true, danger: false, checked: false, shortcut: None, submenu: false, avatar: None, dot: None, boxed: false, sub: None } }
    pub(crate) fn sub(mut self, s: impl Into<String>) -> Self { self.sub = Some(s.into()); self }
    pub(crate) fn icon(mut self, icon: &'static str) -> Self { self.icon = Some(icon); self }
    pub(crate) fn disabled(mut self) -> Self { self.enabled = false; self }
    pub(crate) fn enabled(mut self, on: bool) -> Self { self.enabled = on; self }
    pub(crate) fn danger(mut self) -> Self { self.danger = true; self }
    pub(crate) fn checked(mut self, on: bool) -> Self { self.checked = on; self }
    pub(crate) fn shortcut(mut self, s: &str) -> Self { self.shortcut = Some(s.into()); self }
    pub(crate) fn submenu(mut self) -> Self { self.submenu = true; self }
    pub(crate) fn avatar(mut self, name: &str, photo: Option<egui::TextureId>) -> Self { self.avatar = Some((name.into(), photo)); self }
    pub(crate) fn dot(mut self, c: Color32) -> Self { self.dot = Some(c); self }
    pub(crate) fn boxed(mut self) -> Self { self.boxed = true; self }
    pub(crate) fn entry(self) -> Entry { Entry::Item(self) }
}

/// Which side of the anchor the menu opens on (the caret points back at the anchor).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Side { Below, Above, Right }
/// Alignment of the menu against the anchor along the other axis.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Align { Left, Center, Right }

/// Whether a light theme is active (`t.light`).
pub(crate) fn light(t: &Th) -> bool { t.light }

struct Pal { bg: Color32, border: Color32, hover: Color32, pressed: Color32, text: Color32, text2: Color32, dim: Color32, sep: Color32, danger: Color32 }
/// menulistbackground-primary-* (#000000 / #FFFFFF, border 20 %), listitem-background hovered / pressed, label-primary / -secondary
/// text. `danger`: #F55868 dark, #D43519 light.
fn pal(t: &Th) -> Pal {
    Pal { bg: t.tk("menulistbackground-primary-background"), border: t.tk("menulistbackground-primary-border"), hover: t.tk("listitem-background:hovered"),
          pressed: t.tk("listitem-background:pressed"), text: t.tk("label-primary-text"), text2: t.tk("label-secondary-text"),
          dim: t.tk("label-primary-text:disabled"), sep: t.tk("menulistbackground-primary-border"), danger: if t.light { hex(0xD43519) } else { hex(0xF55868) } }
}

fn key(k: &str) -> egui::Id { egui::Id::new(("webex-menu-open", k)) }
/// The balloon body (radius 10; without caret and shadow) the menu k was last drawn at: the tray menu window's shape without a compositor.
pub(crate) fn menu_body(ctx: &egui::Context, k: &str) -> Option<Rect> { ctx.data(|d| d.get_temp(egui::Id::new(("webex-menu-body", k)))) }
pub(crate) fn menu_is_open(ctx: &egui::Context, k: &str) -> bool { ctx.data(|d| d.get_temp::<bool>(key(k))).unwrap_or(false) }
pub(crate) fn menu_open(ctx: &egui::Context, k: &str) { ctx.data_mut(|d| d.insert_temp(key(k), true)); }
pub(crate) fn menu_close(ctx: &egui::Context, k: &str) { ctx.data_mut(|d| d.insert_temp(key(k), false)); }
/// The rect an item of the open menu `k` was drawn at last frame (to anchor a submenu).
pub(crate) fn menu_item_rect(ctx: &egui::Context, k: &str, id: &str) -> Option<Rect> {
    ctx.data(|d| d.get_temp::<Vec<(String, Rect)>>(egui::Id::new(("webex-menu-rects", k)))).and_then(|v| v.into_iter().find(|(i, _)| i == id).map(|(_, r)| r))
}
pub(crate) fn menu_toggle(ctx: &egui::Context, k: &str) { let o = menu_is_open(ctx, k); ctx.data_mut(|d| d.insert_temp(key(k), !o)); }

// Minimum item width 264 + the panel padding 8 on each side = 280.
const WIDTH: f32 = 280.0;
const ITEM_H: f32 = 40.0;
const SEP_H: f32 = 11.0;
const TITLE_H: f32 = 32.0;
const PAD: f32 = 8.0;
const CARET: f32 = 12.0;

/// The new scroll offset after a wheel / touch pad delta (positive delta = content moves down = offset decreases), kept in 0..=max.
pub(crate) fn wheel_offset(off: f32, delta: f32, max: f32) -> f32 { (off - delta).clamp(0.0, max.max(0.0)) }

/// The offset that brings the row `[row_top, row_top + row_h]` (content coordinates) into a view of height `view_h` showing
/// `[off, off + view_h]`, moving as little as possible.
pub(crate) fn scroll_into_view(off: f32, row_top: f32, row_h: f32, view_h: f32, max: f32) -> f32 {
    let o = if row_top < off { row_top } else if row_top + row_h > off + view_h { row_top + row_h - view_h } else { off };
    o.clamp(0.0, max.max(0.0))
}

fn entry_h(e: &Entry) -> f32 { match e { Entry::Item(it) => if it.avatar.is_some() || it.sub.is_some() { 48.0 } else { ITEM_H }, Entry::Sep => SEP_H, Entry::Title(_) => TITLE_H,
    Entry::Profile(_) => CARD_H - PAD + CARD_GAP } }

/// A menu anchored at `anchor`. Returns the id of the picked item (the menu closes itself on a pick, Esc, or a click elsewhere).
pub(crate) fn webex_menu(ctx: &egui::Context, icons: &mut icons::Icons, t: &Th, k: &str, anchor: Rect, side: Side, align: Align, caret: bool, items: &[Entry]) -> Option<String> {
    menu_impl(ctx, icons, t, k, anchor, side, align, caret, items, None)
}

/// `width`: an exact menu width (a dropdown list is as wide as its field); None = 284 or wider when needed.
pub(crate) fn menu_impl(ctx: &egui::Context, icons: &mut icons::Icons, t: &Th, k: &str, anchor: Rect, side: Side, align: Align, caret: bool, items: &[Entry], width: Option<f32>) -> Option<String> {
    if !menu_is_open(ctx, k) { return None; }
    let p = pal(t);
    // A `dropdown-*` key is a combo box list, not a menu: 4 pt below the field, its width, padding 8, radius 12, rows 32 pt with
    // secondary text at margin 10 and radius 6, at most 180 high (= 5 rows) with a scroll bar; it opens scrolled to the current
    // entry, highlighted (white 7 % on #000000; track 16 pt wide 8 pt from the edge, thumb 8 pt wide).
    let combo = k.starts_with("dropdown-");
    let eh = |e: &Entry| if combo { 32.0 } else { entry_h(e) };
    let sel_id = egui::Id::new(("webex-menu-sel", k));
    let mut sel: Option<usize> = ctx.data(|d| d.get_temp(sel_id));
    let body_h = PAD * 2.0 + items.iter().map(eh).sum::<f32>();
    // The tray menu is its own window (platform/notify/hub.rs), exactly the balloon: no screen margin, no caret strip, no shadow.
    let tray = k == "tray";
    let cd = if caret { CARET } else if tray { 0.0 } else { 4.0 };
    // Width: 284, widened when a label (plus icon, shortcut, check) needs more.
    let need = items.iter().map(|e| match e {
        Entry::Item(it) => {
            let lw = ctx.fonts(|f| f.layout_no_wrap(it.label.clone(), FontId::proportional(16.0), p.text).size().x);
            let sw = it.shortcut.as_ref().map_or(0.0, |s| ctx.fonts(|f| f.layout_no_wrap(s.clone(), FontId::proportional(13.0), p.text2).size().x) + 24.0);
            PAD * 2.0 + 12.0 + if it.icon.is_some() || it.dot.is_some() || it.avatar.is_some() { 36.0 } else { 0.0 } + lw + sw.max(if it.checked || it.submenu { 36.0 } else { 8.0 }) + 8.0
        }
        _ => 0.0,
    }).fold(0.0_f32, f32::max);
    let (w, full_h) = (width.unwrap_or(WIDTH.max(need)), body_h + if side == Side::Right { 0.0 } else { cd });
    let screen = ctx.screen_rect();
    // A menu taller than the window scrolls (wheel / touch pad over it; the arrow keys bring the highlighted row into view).
    let h = full_h.min((screen.height() - if tray { 0.0 } else { 16.0 }).max(80.0));
    let h = if combo { h.min(180.0 + cd) } else { h };
    let overflow = full_h > h + 0.5;
    let (mut x, mut y) = match side {
        Side::Below => (match align { Align::Left => anchor.left(), Align::Center => anchor.center().x - w / 2.0, Align::Right => anchor.right() - w }, anchor.bottom() + if combo || tray { 0.0 } else { 2.0 }),
        Side::Above => (match align { Align::Left => anchor.left(), Align::Center => anchor.center().x - w / 2.0, Align::Right => anchor.right() - w }, anchor.top() - 2.0 - h),
        Side::Right => (anchor.right() + cd, anchor.top() - PAD),
    };
    let m = if tray { 0.0 } else { 8.0 };
    x = x.clamp(m, (screen.right() - w - m).max(m));
    y = y.clamp(m, (screen.bottom() - h - m).max(m));
    let outer = Rect::from_min_size(pos2(x, y), Vec2::new(w, h));
    // The balloon body (without the caret strip).
    let body = match side { Side::Below => Rect::from_min_max(pos2(x, y + cd), outer.max), Side::Above => Rect::from_min_max(outer.min, pos2(x + w, y + h - cd)), Side::Right => outer };
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("webex-menu-body", k)), body)); // menu_body

    let view_h = body.height() - 2.0 * PAD;
    let max_off = (body_h - 2.0 * PAD - view_h).max(0.0);
    let mut moved = false;
    // On opening, the current entry becomes the list's top row as far as the list can scroll.
    let mut open_at: Option<usize> = None;
    if combo && sel.is_none() { if let Some(c) = items.iter().position(|e| matches!(e, Entry::Item(it) if it.checked)) { sel = Some(c); open_at = Some(c); } }
    let mut picked: Option<String> = None;
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
    // Keyboard: Up / Down move the highlight over the enabled items, Enter picks it.
    let sel_ok = |i: usize| matches!(&items[i], Entry::Item(it) if it.enabled);
    let n = items.len();
    if n > 0 {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            let mut i = sel.map_or(0, |s| s + 1);
            while i < n && !sel_ok(i) { i += 1; }
            if i < n { sel = Some(i); moved = true; }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            let mut i = sel.map_or(n as isize - 1, |s| s as isize - 1);
            while i >= 0 && !sel_ok(i as usize) { i -= 1; }
            if i >= 0 { sel = Some(i as usize); moved = true; }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) { if let Some(Entry::Item(it)) = sel.and_then(|s| items.get(s)) { picked = Some(it.id.clone()); } }
    }

    let mut rects: Vec<(String, Rect)> = Vec::new();
    egui::Area::new(egui::Id::new(("webex-menu", k))).fixed_pos(outer.min).order(if k.starts_with("dropdown-") { egui::Order::Debug } else { egui::Order::Foreground }).interactable(true).show(ctx, |ui| {
        ui.set_clip_rect(outer.expand(24.0));
        ui.allocate_exact_size(outer.size(), Sense::hover());
        let pt = ui.painter().clone();
        // Soft shadow, body, 1 px border and the caret; panel radius 10 (combo list 12).
        let br = if combo { 12.0 } else { 10.0 };
        if !tray { pt.add(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(if light(t) { 40 } else { 140 }) }.as_shape(body, br)); }
        pt.rect_filled(body, br, p.bg);
        pt.rect_stroke(body, br, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        if caret && side != Side::Right {
            let cx = anchor.center().x.clamp(body.left() + 22.0, body.right() - 22.0);
            let (base_y, tip_y) = if side == Side::Below { (body.top(), body.top() - cd) } else { (body.bottom(), body.bottom() + cd) };
            let (a, b, tip) = (pos2(cx - 11.0, base_y), pos2(cx + 11.0, base_y), pos2(cx, tip_y));
            pt.add(egui::Shape::convex_polygon(vec![pos2(a.x, base_y + if side == Side::Below { 1.0 } else { -1.0 }), pos2(b.x, base_y + if side == Side::Below { 1.0 } else { -1.0 }), tip], p.bg, Stroke::NONE));
            pt.line_segment([a, tip], Stroke::new(1.0, p.border));
            pt.line_segment([b, tip], Stroke::new(1.0, p.border));
        }
        let off_id = egui::Id::new(("webex-menu-off", k));
        let mut off: f32 = if overflow { ctx.data(|d| d.get_temp(off_id)).unwrap_or(0.0) } else { 0.0 };
        if let (Some(c), true) = (open_at, overflow) { off = (c as f32 * 32.0).min(max_off); }
        if overflow && ui.rect_contains_pointer(outer) { let d = ctx.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta.y)); off = wheel_offset(off, d, max_off); }
        ui.set_clip_rect(if combo { body.shrink2(Vec2::new(0.0, PAD)) } else { body }); // the combo's list clips inside the padding
        let pt = ui.painter().clone();
        let mut want_off = off;
        let mut yy = body.top() + PAD - off;
        for (i, e) in items.iter().enumerate() {
            let h = eh(e);
            // Combo with a scrollbar: the rows leave room for it (bar 16 + padding 8).
            let row = Rect::from_min_size(pos2(body.left() + PAD, yy), Vec2::new(body.width() - PAD * 2.0 - if combo && overflow { 24.0 } else { 0.0 }, h));
            let rr = if combo { 6.0 } else { 8.0 };
            if overflow && moved && sel == Some(i) { want_off = scroll_into_view(off, yy + off - (body.top() + PAD), h, view_h, max_off); }
            match e {
                Entry::Sep => { pt.hline((row.left() + 4.0)..=(row.right() - 4.0), row.center().y, Stroke::new(1.0, p.sep)); }
                Entry::Title(s) => { pt.text(pos2(row.left() + 12.0, row.center().y), egui::Align2::LEFT_CENTER, s, FontId::proportional(13.0), p.text2); }
                Entry::Profile(c) => { // the avatar menu's header card, edge to edge from the balloon top
                    let card = Rect::from_min_size(pos2(body.left(), yy - PAD), Vec2::new(body.width(), CARD_H));
                    let banner = Rect::from_min_size(card.min, Vec2::new(card.width(), CARD_BANNER));
                    let cr = egui::CornerRadius { nw: 12, ne: 12, sw: 0, se: 0 };
                    match c.cover {
                        Some((tid, [w, h])) => {
                            let (ia, ra) = (w as f32 / h.max(1) as f32, banner.width() / banner.height());
                            let uv = if ia > ra { let u = ra / ia; Rect::from_min_max(pos2((1.0 - u) / 2.0, 0.0), pos2((1.0 + u) / 2.0, 1.0)) }
                                     else { let v = ia / ra; Rect::from_min_max(pos2(0.0, (1.0 - v) / 2.0), pos2(1.0, (1.0 + v) / 2.0)) };
                            egui::Image::from_texture(egui::load::SizedTexture::new(tid, banner.size())).uv(uv).corner_radius(cr).paint_at(ui, banner);
                        }
                        None => { pt.rect_filled(banner, cr, t.avatar); } // no cover: flat
                    }
                    let ac = pos2(card.center().x, banner.bottom());
                    let mut au = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(ac, Vec2::splat(CARD_AVATAR))));
                    super::widgets::avatar_photo(&mut au, icons, t, &c.name, CARD_AVATAR, false, None, None, c.photo);
                    pt.text(pos2(card.center().x, card.top() + 170.0), egui::Align2::CENTER_CENTER, &c.name, super::widgets::semibold(16.0), p.text);
                    pt.text(pos2(card.center().x, card.top() + 193.0), egui::Align2::CENTER_CENTER, &c.email, FontId::proportional(14.0), p.text2);
                }
                Entry::Item(it) => {
                    let resp = ui.interact(row, egui::Id::new(("webex-menu-item", k, i)), if it.enabled { Sense::click() } else { Sense::hover() });
                    if resp.hovered() && it.enabled { sel = Some(i); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                    let down = resp.is_pointer_button_down_on() && it.enabled;
                    if down { pt.rect_filled(row, rr, p.pressed); } else if sel == Some(i) && it.enabled { pt.rect_filled(row, rr, p.hover); }
                    if it.boxed && !(down || sel == Some(i)) { pt.rect_filled(row, 8.0, if light(t) { alpha(Color32::BLACK, 0.06) } else { hex(0x292929) }); }
                    let col = if !it.enabled { p.dim } else if it.danger { p.danger } else { p.text };
                    // Menu item: margins 12 / 12, spacing 12, icon 16 in a 24 slot, title 16 px / 400, sub-text 14 px / 400.
                    let mut tx = row.left() + if combo { 10.0 } else { 12.0 };
                    if let Some((nm, ph)) = &it.avatar {
                        let r = Rect::from_center_size(pos2(row.left() + 12.0 + 16.0, row.center().y), Vec2::splat(32.0));
                        let mut ac = ui.new_child(egui::UiBuilder::new().max_rect(r));
                        super::widgets::avatar_photo(&mut ac, icons, t, nm, 32.0, false, None, None, *ph);
                        tx = row.left() + 12.0 + 32.0 + 12.0;
                    }
                    if let Some(c) = it.dot { pt.circle_filled(pos2(row.left() + 12.0 + 12.0, row.center().y), 6.0, c); tx = row.left() + 12.0 + 24.0 + 12.0; }
                    if let Some(ic) = it.icon { icons.paint(ui, ic, Rect::from_center_size(pos2(row.left() + 12.0 + 12.0, row.center().y), Vec2::splat(16.0)), col); tx = row.left() + 12.0 + 24.0 + 12.0; }
                    let right_pad = if it.checked || it.submenu { 36.0 } else if let Some(s) = &it.shortcut { ctx.fonts(|f| f.layout_no_wrap(s.clone(), FontId::proportional(13.0), p.text2).size().x) + 24.0 } else { 8.0 };
                    match &it.sub { // two-line row: label #F3F3F3, sub-line text2
                        Some(s) => {
                            super::chrome::fit_text(&pt, tx, row.center().y - 9.0, &it.label, FontId::proportional(16.0), col, row.right() - right_pad - tx);
                            super::chrome::fit_text(&pt, tx, row.center().y + 11.0, s, FontId::proportional(14.0), p.text2, row.right() - right_pad - tx);
                        }
                        None => { super::chrome::fit_text(&pt, tx, row.center().y, &it.label, FontId::proportional(if combo { 14.0 } else { 16.0 }), col, row.right() - right_pad - tx); }
                    }
                    if let Some(sc) = &it.shortcut { pt.text(pos2(row.right() - 10.0, row.center().y), egui::Align2::RIGHT_CENTER, sc, FontId::proportional(13.0), p.text2); }
                    if it.submenu { icons.paint(ui, "arrow-right-regular", Rect::from_center_size(pos2(row.right() - 18.0, row.center().y), Vec2::splat(14.0)), col); }
                    rects.push((it.id.clone(), row));
                    if it.checked { icons.paint(ui, "check-bold", Rect::from_center_size(pos2(row.right() - 18.0, row.center().y), Vec2::splat(16.0)), col); }
                    if resp.clicked() { picked = Some(it.id.clone()); }
                }
            }
            yy += h;
        }
        if overflow { ctx.data_mut(|d| d.insert_temp(off_id, want_off)); if (want_off - off).abs() > 0.5 { ctx.request_repaint(); } }
        if combo && overflow { // track white 11 %, thumb white 46 %, 16 / 8 pt wide
            let track = Rect::from_min_max(pos2(body.right() - PAD - 16.0, body.top() + PAD), pos2(body.right() - PAD, body.bottom() - PAD));
            let base = if light(t) { Color32::BLACK } else { Color32::WHITE };
            pt.rect_filled(track, 0.0, alpha(base, 0.11));
            let content = (body_h - 2.0 * PAD).max(1.0);
            let th = (track.height() * view_h / content).max(16.0);
            let ty = track.top() + (track.height() - th) * if max_off > 0.0 { off / max_off } else { 0.0 };
            pt.rect_filled(Rect::from_min_size(pos2(track.center().x - 4.0, ty), Vec2::new(8.0, th)), 4.0, alpha(base, 0.46));
        }
    });
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("webex-menu-rects", k)), rects));
    // A click anywhere else closes the menu (the anchor itself toggles it, so it is excluded).
    if let Some(pos) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None }) {
        if !outer.expand(2.0).contains(pos) && !anchor.contains(pos) { close = true; }
    }
    if picked.is_some() { close = true; }
    ctx.data_mut(|d| match sel { Some(s) => d.insert_temp(sel_id, s), None => d.remove::<usize>(sel_id) });
    if close { menu_close(ctx, k); ctx.data_mut(|d| d.remove::<usize>(sel_id)); }
    picked
}

#[derive(Clone)]
pub(crate) struct Button { pub id: String, pub label: String, pub primary: bool, pub danger: bool }
impl Button {
    pub(crate) fn primary(id: &str, label: &str) -> Self { Button { id: id.into(), label: label.into(), primary: true, danger: false } }
    pub(crate) fn danger(id: &str, label: &str) -> Self { Button { id: id.into(), label: label.into(), primary: true, danger: true } }
    pub(crate) fn secondary(id: &str, label: &str) -> Self { Button { id: id.into(), label: label.into(), primary: false, danger: false } }
}

/// A modal dialog: dimmed backdrop, centred card (min width 402, #000000, 1 px white @ 20 % border, radius 12),
/// title, text, optional extra content, pill buttons right-aligned. Returns the clicked button id; "cancel" for the X, Esc.
pub(crate) fn webex_dialog(ctx: &egui::Context, t: &Th, k: &str, title: &str, body: &str, content: Option<&mut dyn FnMut(&mut egui::Ui)>, buttons: &[Button]) -> Option<String> {
    webex_dialog_bg(ctx, t, k, title, body, content, buttons, None)
}

/// `background-primary` of a plain dialog: #121212 dark, white light.
pub(crate) fn background_primary(t: &Th) -> Color32 { t.tk("background-primary") }

/// Gives dialog `k` a fixed card width (e.g. 568 for the status dialog); call before `webex_dialog*`.
pub(crate) fn set_dialog_width(ctx: &egui::Context, k: &str, w: f32) { ctx.data_mut(|d| d.insert_temp(dialog_width_id(k), w)); }
fn dialog_width_id(k: &str) -> egui::Id { egui::Id::new(("webex-dialog-w", k)) }

#[cfg(test)]
mod scroll_tests;

#[cfg(test)]
mod combo_tests;

mod dialog;
#[allow(unused_imports)]
pub(crate) use dialog::*;
