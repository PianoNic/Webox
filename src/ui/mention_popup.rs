//! The `@mention` popup of the composer: placement rules below; the list has two sections, "people" (people of the space,
//! plus the @all entry) and "search" (everybody else). Surface: `modal-primary-background` (dark #000000, light white), border
//! `wx-popup-widget-border`. Section titles ("Personen" / "Suchergebnisse"), the avatar size in a 70 px row (40) and the fonts
//! are Webox's own.
use crate::*;

pub(crate) const ROW_H: f32 = 70.0;
pub(crate) const SECTION_HEADER_H: f32 = 32.0;
pub(crate) const SECTION_PADDING: f32 = 16.0;
pub(crate) const MAX_W: f32 = 428.0;
pub(crate) const MAX_H: f32 = 240.0;
pub(crate) const GAP: f32 = 4.0;
pub(crate) const EDGE: f32 = 8.0; // topMargin / bottomMargin

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cand { pub id: String, pub name: String, pub email: String, pub all: bool, pub section: u8 }

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement { pub opens_below: bool, pub height: f32, pub width: f32, pub x: f32, pub y: f32 }

fn section_h(n: usize) -> f32 { if n > 0 { n as f32 * ROW_H + SECTION_HEADER_H + SECTION_PADDING } else { 0.0 } }

/// Placement: `anchor` = the text box, `cursor` = the caret rectangle, `viewport_h` = window height (0 = unknown),
/// `locked` = the direction fixed when the popup opened. Returns the popup rectangle's origin (absolute) and size.
pub(crate) fn place(anchor: Rect, cursor: Rect, viewport_h: f32, people: usize, search: usize, prefer_above_when_usable: bool, locked: Option<bool>) -> Placement {
    let requested = MAX_H.min(section_h(people) + section_h(search));
    let width = MAX_W.min(anchor.width());
    let has_vp = viewport_h > 0.0;
    let above = (cursor.top() - EDGE - GAP).max(0.0);
    let below = if has_vp { (viewport_h - EDGE - cursor.bottom() - GAP).max(0.0) } else { requested };
    let min_complete = requested.min(SECTION_HEADER_H + SECTION_PADDING + ROW_H);
    let usable_above = requested > 0.0 && above >= min_complete;
    let preferred_below = requested > 0.0 && !(prefer_above_when_usable && usable_above) && (requested > above && (requested <= below || below > above));
    let opens_below = locked.unwrap_or(preferred_below);
    let height = if has_vp { requested.min(if opens_below { below } else { above }) } else { requested };
    let x_rel = (cursor.left() - anchor.left()).min((anchor.width() - width).max(0.0)).max(0.0);
    let y = if opens_below { cursor.bottom() + GAP } else { cursor.top() - height - GAP };
    Placement { opens_below, height, width, x: anchor.left() + x_rel, y }
}

/// People of the space and everybody else matching `query`. @all (only in a group space) is the first people entry.
pub(crate) fn candidates(query: &str, group: bool, space_people: &[Person], others: &[Person], me: &Person) -> Vec<Cand> {
    let q = query.trim().to_lowercase();
    let hit = |p: &Person| q.is_empty() || p.name.to_lowercase().split_whitespace().any(|w| w.starts_with(&q)) || p.name.to_lowercase().starts_with(&q)
        || p.name.to_lowercase().contains(&q) || p.email.to_lowercase().starts_with(&q);
    let is_me = |p: &Person| (!me.id.is_empty() && p.id == me.id) || (!me.email.is_empty() && p.email.eq_ignore_ascii_case(&me.email));
    let mut out = Vec::new();
    if group && (q.is_empty() || "all".starts_with(&q) || "alle".starts_with(&q)) {
        out.push(Cand { id: String::new(), name: "all".into(), email: String::new(), all: true, section: 0 });
    }
    let mut seen: HashSet<String> = HashSet::new();
    for p in space_people.iter().filter(|p| !is_me(p) && hit(p)) {
        if seen.insert(p.email.to_lowercase() + &p.id) { out.push(Cand { id: p.id.clone(), name: p.name.clone(), email: p.email.clone(), all: false, section: 0 }); }
    }
    let in_space = |p: &Person| space_people.iter().any(|s| s.id == p.id || (!s.email.is_empty() && s.email.eq_ignore_ascii_case(&p.email)));
    for p in others.iter().filter(|p| !is_me(p) && !in_space(p) && hit(p)).take(10) {
        if seen.insert(p.email.to_lowercase() + &p.id) { out.push(Cand { id: p.id.clone(), name: p.name.clone(), email: p.email.clone(), all: false, section: 1 }); }
    }
    out
}

/// Draws the popup (non-modal, never takes focus). Returns the clicked candidate index.
pub(crate) fn popup(ui_app: &mut App, ctx: &egui::Context, t: &Th, pl: Placement, cands: &[Cand], sel: usize, scroll: bool) -> Option<usize> {
    let light = super::menu::light(t);
    let (bg, border) = if light { (Color32::WHITE, alpha(Color32::BLACK, 0.2)) } else { (hex(0x000000), alpha(Color32::WHITE, 0.15)) }; // modal-primary-background
    let outer = Rect::from_min_size(pos2(pl.x, pl.y), Vec2::new(pl.width, pl.height));
    let mut clicked = None;
    egui::Area::new(egui::Id::new("mention-popup")).fixed_pos(outer.min).order(egui::Order::Foreground).show(ctx, |ui| {
        // The Area must occupy its rect: egui routes the wheel by the layer under the pointer, and a layer without size is never under it.
        ui.allocate_exact_size(outer.size(), Sense::hover());
        ui.painter().rect_filled(outer, 8.0, bg);
        ui.painter().rect_stroke(outer, 8.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(outer.shrink(1.0)).layout(egui::Layout::top_down(egui::Align::Min)));
        c.set_clip_rect(outer.shrink(1.0));
        c.spacing_mut().item_spacing = Vec2::ZERO;
        c.spacing_mut().scroll = egui::style::ScrollStyle { bar_width: 8.0, bar_inner_margin: 1.0, bar_outer_margin: 0.0, ..egui::style::ScrollStyle::solid() };
        crate::ui::widgets::vscroll().id_salt("mention-list").max_height(outer.height() - 2.0).auto_shrink([false, false]).show(&mut c, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let mut last_section = 255u8;
            for (i, cd) in cands.iter().enumerate() {
                if cd.section != last_section {
                    if last_section != 255 { ui.add_space(SECTION_PADDING); }
                    last_section = cd.section;
                    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), SECTION_HEADER_H), Sense::hover());
                    ui.painter().text(pos2(r.left() + 16.0, r.center().y), egui::Align2::LEFT_CENTER, if cd.section == 0 { "Personen" } else { "Suchergebnisse" }, FontId::proportional(14.0), t.text2);
                }
                let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                if scroll && i == sel { ui.scroll_to_rect(r, None); }
                if resp.hovered() || i == sel { ui.painter().rect_filled(r, 0.0, if resp.hovered() { t.hover } else { t.selected }); }
                let ac = pos2(r.left() + 16.0 + 20.0, r.center().y);
                if cd.all {
                    ui.painter().circle_filled(ac, 20.0, t.accent);
                    ui_app.icons.paint(ui, "mention-regular", Rect::from_center_size(ac, Vec2::splat(20.0)), Color32::WHITE);
                } else {
                    let photo = ui_app.photo_for(ctx, &cd.id, &cd.email);
                    let ar = Rect::from_center_size(ac, Vec2::splat(40.0));
                    let mut a = ui.new_child(egui::UiBuilder::new().max_rect(ar));
                    avatar_photo(&mut a, &mut ui_app.icons, t, &cd.name, 40.0, false, None, None, photo);
                }
                let tx = r.left() + 16.0 + 40.0 + 12.0;
                let clip = Rect::from_min_max(pos2(tx, r.top()), pos2(r.right() - 16.0, r.bottom()));
                let p = ui.painter().with_clip_rect(clip);
                let (title, sub) = if cd.all { ("Alle erwähnen".to_string(), String::new()) } else { (cd.name.clone(), cd.email.clone()) };
                if sub.is_empty() { p.text(pos2(tx, r.center().y), egui::Align2::LEFT_CENTER, title, semibold(15.0), t.text1); }
                else {
                    p.text(pos2(tx, r.center().y - 9.0), egui::Align2::LEFT_CENTER, title, semibold(15.0), t.text1);
                    p.text(pos2(tx, r.center().y + 10.0), egui::Align2::LEFT_CENTER, sub, FontId::proportional(13.0), t.text2);
                }
                if resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                if resp.clicked() { clicked = Some(i); }
            }
        });
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect { Rect::from_min_size(pos2(x, y), Vec2::new(w, h)) }

    #[test]
    fn section_heights_and_cap() {
        assert_eq!(section_h(0), 0.0);
        assert_eq!(section_h(1), 70.0 + 32.0 + 16.0);
        // 2 rows in each section would be 2 * 218 = 436 > 240: capped.
        let p = place(r(20.0, 600.0, 800.0, 30.0), r(100.0, 600.0, 1.0, 20.0), 720.0, 2, 2, false, None);
        assert_eq!(p.height, 240.0);
        assert_eq!(p.width, 428.0);
    }

    #[test]
    fn opens_above_when_it_fits_else_below() {
        let anchor = r(20.0, 600.0, 800.0, 30.0);
        let above = place(anchor, r(100.0, 600.0, 1.0, 20.0), 720.0, 1, 0, false, None);
        assert!(!above.opens_below);
        assert_eq!(above.y, 600.0 - above.height - 4.0);
        // Caret near the top: not enough room above, plenty below.
        let below = place(anchor, r(100.0, 60.0, 1.0, 20.0), 720.0, 2, 0, false, None);
        assert!(below.opens_below);
        assert_eq!(below.y, 80.0 + 4.0);
        // A locked direction wins; the height is clipped to the room.
        let locked = place(anchor, r(100.0, 60.0, 1.0, 20.0), 720.0, 2, 0, false, Some(false));
        assert!(!locked.opens_below);
        assert_eq!(locked.height, 60.0 - 8.0 - 4.0);
        // x never leaves the anchor.
        let edge = place(anchor, r(790.0, 600.0, 1.0, 20.0), 720.0, 1, 0, false, None);
        assert_eq!(edge.x, 20.0 + (800.0 - 428.0));
    }

    #[test]
    fn candidates_sections() {
        let me = Person { id: "me".into(), name: "Ich".into(), email: "ich@x".into(), status: String::new() };
        let p = |id: &str, n: &str| Person { id: id.into(), name: n.into(), email: format!("{id}@x"), status: String::new() };
        let c = candidates("", true, &[me.clone(), p("a", "Anna")], &[p("a", "Anna"), p("b", "Bob")], &me);
        assert_eq!(c.iter().map(|c| (c.name.as_str(), c.section)).collect::<Vec<_>>(), vec![("all", 0), ("Anna", 0), ("Bob", 1)]);
        let c = candidates("bo", true, &[], &[p("b", "Bob"), p("c", "Carl")], &me);
        assert_eq!(c.len(), 1);
        assert!(candidates("zz", false, &[], &[], &me).is_empty());
        assert!(candidates("", false, &[], &[], &me).is_empty());
    }
}
