//! Shared parts of the main navigation pages (Anrufe, Voicemail, Meetings, Kontakte, Teams, Whiteboards): the two-card frame
//! (list card + detail card, like the messaging screen), header, list rows, pill / round buttons, empty states and context menus.
//!
//! List card = theme-gradient-secondary (`vgrad`, tokens t.card/t.card_b), detail card = theme-gradient-primary (t.conv/t.conv_b),
//! 4 px gap, 10 px radius, rows 50 px (as `App::row`). Buttons: buttonJoin-primary (green), button-secondary-border white @ 30 %,
//! button-ghost hover white @ 7 %; pill 32 pt, round button 48 pt.
#![allow(clippy::too_many_arguments)]
use crate::*;
use super::menu::{self, Align, Entry, Side};

/// Per-page UI state, kept in egui's temp memory so no shared struct has to change.
#[derive(Clone, Default)]
pub(crate) struct PageState {
    pub calls_tab: u8, pub dial_open: bool, pub call_sel: Option<usize>,
    pub vm_tab: u8, pub vm_sel: Option<usize>, pub vm_deleted: HashSet<usize>, pub vm_read: HashSet<usize>, pub vm_playing: Option<usize>,
    pub meet_sel: Option<usize>, pub meet_view: u8, pub cal_prompt_closed: bool,
    pub contact_sel: Option<String>, pub contact_q: String, pub contact_card: Option<(String, String, Rect)>,
    pub call_deleted: HashSet<String>, pub call_confirm_delete: Option<String>, pub join_dialog: Option<String>, pub cal_dialog: bool, pub team_open: Option<String>,
    pub team_tab: u8, pub new_team: Option<(String, String)>, pub wb_q: String, pub wb_sort: u8,
    /// Open context menu: (key of the row, anchor).
    pub menu_target: Option<(String, Rect)>,
}

pub(crate) const PAGE_STATE: &str = "page-state";

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Pill { Join, Primary, Secondary }

pub(crate) struct RowSpec<'a> {
    pub id: String, pub title: &'a str, pub sub: &'a str, pub sub_icon: Option<&'static str>, pub sub_col: Option<Color32>, pub title_col: Option<Color32>,
    pub right: &'a str, pub selected: bool, pub bold: bool, pub status: Option<&'a str>, pub photo: Option<egui::TextureId>,
    pub space: bool, pub av_bg: Option<Color32>, pub action: Option<(&'static str, &'static str)>, pub dot: bool, pub icon: Option<&'static str>,
}
impl<'a> RowSpec<'a> {
    pub fn new(id: impl Into<String>, title: &'a str) -> Self {
        RowSpec { id: id.into(), title, sub: "", sub_icon: None, sub_col: None, title_col: None, right: "", selected: false, bold: false, status: None, photo: None,
            space: false, av_bg: None, action: None, dot: false, icon: None }
    }
}
#[derive(Default)]
pub(crate) struct RowOut { pub clicked: bool, pub double: bool, pub action: bool, pub menu: Option<Rect> }

impl App {
    pub(crate) fn pst(&self, ctx: &egui::Context) -> PageState { ctx.data(|d| d.get_temp::<PageState>(egui::Id::new(PAGE_STATE))).unwrap_or_default() }
    pub(crate) fn set_pst(&self, ctx: &egui::Context, s: PageState) { ctx.data_mut(|d| d.insert_temp(egui::Id::new(PAGE_STATE), s)); }

    /// The main area of every page except Messaging: Plugins/Settings keep their single card, Whiteboards is a single card, the others a list card + detail card.
    pub(crate) fn pages_frame(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let gap = egui::Margin { left: 0, right: 4, top: 0, bottom: 0 };
        if matches!(self.tab, Tab::Plugins | Tab::Settings | Tab::Whiteboards) {
            egui::CentralPanel::default().frame(egui::Frame::default().inner_margin(gap)).show(ctx, |ui| {
                let card = ui.available_rect_before_wrap();
                let _ = ui.allocate_rect(card, Sense::hover());
                vgrad(ui.painter(), card, 10.0, t.card, t.card_b); crate::ui::theme::decor::card(ui.painter(), card, 10.0, true);
                let (dx, dy) = if self.tab == Tab::Whiteboards { (24.0, 20.0) } else { (24.0, 24.0) };
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(card.min + Vec2::new(dx, dy), card.max - Vec2::splat(24.0))));
                c.set_min_size(c.available_size());
                self.page(&mut c, t, now);
            });
            return;
        }
        let card_w = self.list_card_width(ctx);
        egui::SidePanel::left("page-list").exact_width(card_w + 4.0).resizable(false).show_separator_line(false)
            .frame(egui::Frame::default().inner_margin(gap)).show(ctx, |ui| {
            let card = ui.available_rect_before_wrap();
            let _ = ui.allocate_rect(card, Sense::hover());
            vgrad(ui.painter(), card, 10.0, t.card, t.card_b); crate::ui::theme::decor::card(ui.painter(), card, 10.0, true);
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(card.left() + 8.0, card.top() + 18.0), pos2(card.right() - 6.0, card.bottom()))));
            c.set_min_size(c.available_size());
            self.page_list(&mut c, t, now);
        });
        self.list_splitter(ctx, t, card_w);
        egui::CentralPanel::default().frame(egui::Frame::default().inner_margin(gap)).show(ctx, |ui| {
            let card = ui.available_rect_before_wrap();
            let _ = ui.allocate_rect(card, Sense::hover());
            vgrad(ui.painter(), card, 10.0, t.conv, t.conv_b); crate::ui::theme::decor::card(ui.painter(), card, 10.0, false);
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(card));
            c.set_min_size(card.size());
            self.page_detail(&mut c, t, now);
        });
        self.page_menus(ctx, t, now);
    }

    fn page_list(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        match self.tab {
            Tab::Calling => self.calls_list(ui, t, now),
            Tab::Voicemail => self.voicemail_list(ui, t, now),
            Tab::Meetings => self.meetings_list(ui, t, now),
            Tab::Contacts => self.contacts_list(ui, t, now),
            Tab::Teams => self.teams_list(ui, t, now),
            _ => {}
        }
    }
    fn page_detail(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        match self.tab {
            Tab::Calling => self.calls_detail(ui, t, now),
            Tab::Voicemail => self.voicemail_detail(ui, t, now),
            Tab::Meetings => self.meetings_detail(ui, t, now),
            Tab::Contacts => self.contacts_detail(ui, t, now),
            Tab::Teams => self.teams_detail(ui, t, now),
            _ => {}
        }
    }
    fn page_menus(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        match self.tab {
            Tab::Calling => self.calls_menu(ctx, t, now),
            Tab::Voicemail => self.voicemail_menu(ctx, t, now),
            Tab::Meetings => self.meetings_menu(ctx, t, now),
            Tab::Contacts => self.contacts_menu(ctx, t, now),
            Tab::Teams => self.teams_menu(ctx, t, now),
            _ => {}
        }
        self.contact_card_popup(ctx, t, now);
    }

    /// Opens the shared context menu `key` for the row `target` (anchored at `at`).
    pub(crate) fn open_page_menu(&self, ctx: &egui::Context, key: &str, target: &str, at: Rect) {
        let mut s = self.pst(ctx);
        s.menu_target = Some((target.to_string(), at));
        self.set_pst(ctx, s);
        menu::menu_open(ctx, key);
    }

    /// Title row of a list card: title (semibold 19), optional filter icon and "+" circle on the right, as in `space_list`. Returns (plus clicked, filter response).
    pub(crate) fn list_header(&mut self, ui: &mut egui::Ui, t: &Th, title: &str, plus_tip: Option<&str>, extra: Option<(&'static str, &str)>) -> (bool, Option<egui::Response>) {
        let mut plus = false;
        let mut ex = None;
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.label(RichText::new(title).font(semibold(19.0)).color(t.text1));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                if let Some(tip) = plus_tip {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 14.0, if resp.hovered() { t.white_btn.gamma_multiply(0.85) } else { t.white_btn });
                    self.icons.paint(ui, "plus-regular", Rect::from_center_size(r.center(), Vec2::splat(16.0)), t.white_btn_text);
                    if resp.on_hover_text(tip).clicked() { plus = true; }
                    ui.add_space(5.0);
                }
                if let Some((icon, tip)) = extra {
                    ex = Some(self.icons.button(ui, icon, 28.0, 18.0, t.text2, t.hover).on_hover_text(tip));
                }
            });
        });
        (plus, ex)
    }

    /// A pill button: Join = green, Primary = light, Secondary = outlined.
    pub(crate) fn pill(&mut self, ui: &mut egui::Ui, t: &Th, label: &str, kind: Pill, icon: Option<&'static str>) -> egui::Response {
        let font = semibold(14.0);
        let tw = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font.clone(), t.text1).size().x);
        let iw = if icon.is_some() { 22.0 } else { 0.0 };
        let (r, resp) = ui.allocate_exact_size(Vec2::new(tw + iw + 32.0, 32.0), Sense::click());
        let hov = resp.hovered();
        if hov { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        let (fill, fg, stroke) = match kind {
            Pill::Join => (if hov { t.green.gamma_multiply(0.85) } else { t.green }, hex(0x121212), Stroke::NONE),
            Pill::Primary => (if hov { t.white_btn.gamma_multiply(0.85) } else { t.white_btn }, t.white_btn_text, Stroke::NONE),
            Pill::Secondary => (if hov { alpha(t.text1, 0.1) } else { Color32::TRANSPARENT }, t.text1, Stroke::new(1.0, alpha(t.text1, 0.3))), // button-secondary-border white @ 30 %
        };
        ui.painter().rect_filled(r, 16.0, fill);
        if stroke.width > 0.0 { ui.painter().rect_stroke(r, 16.0, stroke, egui::StrokeKind::Inside); }
        let mut x = r.left() + 16.0;
        if let Some(ic) = icon { self.icons.paint(ui, ic, Rect::from_center_size(pos2(x + 8.0, r.center().y), Vec2::splat(16.0)), fg); x += 22.0; }
        ui.painter().text(pos2(x, r.center().y), egui::Align2::LEFT_CENTER, label, font, fg);
        resp
    }

    /// A round icon button with a label underneath (the expanded meetings header, the contact card actions).
    pub(crate) fn round_labeled(&mut self, ui: &mut egui::Ui, t: &Th, icon: &'static str, label: &str, fill: Color32, icon_col: Color32, w: f32) -> egui::Response {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 48.0 + 8.0 + 18.0), Sense::click());
        let c = pos2(r.center().x, r.top() + 24.0);
        let f = if resp.is_pointer_button_down_on() { fill.gamma_multiply(0.7) } else if resp.hovered() { fill.gamma_multiply(0.85) } else { fill };
        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        ui.painter().circle_filled(c, 24.0, f);
        self.icons.paint(ui, icon, Rect::from_center_size(c, Vec2::splat(24.0)), icon_col);
        fit_label(ui, pos2(r.center().x, r.top() + 48.0 + 8.0 + 9.0), label, semibold(13.0), t.text1, w);
        resp
    }

    /// One 50 px list row (same geometry as `App::row`) with avatar, title, optional sub line with a leading icon, time on the right and, on hover, an action button and "···".
    pub(crate) fn page_row(&mut self, ui: &mut egui::Ui, t: &Th, s: &RowSpec) -> RowOut {
        let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 51.0), Sense::hover());
        let r = Rect::from_min_max(full.min + Vec2::new(12.0, 0.0), pos2(full.max.x - 4.0, full.min.y + 50.0));
        let resp = ui.interact(r, ui.id().with(("pg-row", &s.id)), Sense::click());
        let hovered = ui.rect_contains_pointer(r);
        if s.selected { ui.painter().rect_filled(r, 25.0, t.selected); if crate::ui::theme::decor::on(collabhost_plugin_api::deco::SELECTED_ROW) { crate::ui::theme::decor::outline(ui.painter(), r, 25.0, 1.5, &crate::ui::theme::decor::stripes(), 0.0); } }
        else if resp.is_pointer_button_down_on() { ui.painter().rect_filled(r, 25.0, t.ink_a(0.11)); }
        else if resp.hovered() { ui.painter().rect_filled(r, 25.0, t.ink_a(0.05)); }
        let mut out = RowOut { clicked: resp.clicked(), double: resp.double_clicked(), ..Default::default() };
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(pos2(r.left() + 24.0, r.center().y), Vec2::splat(32.0))));
        if let Some(ic) = s.icon {
            let cc = c.max_rect().center();
            ui.painter().circle_filled(cc, 16.0, s.av_bg.unwrap_or(t.avatar));
            self.icons.paint(ui, ic, Rect::from_center_size(cc, Vec2::splat(18.0)), t.avatar_text);
        } else { avatar_photo(&mut c, &mut self.icons, t, s.title, 32.0, s.space, s.av_bg, s.status, s.photo); }
        let tx = r.left() + 52.0;
        let show_actions = hovered || s.selected;
        let right_w = if show_actions { if s.action.is_some() { 74.0 } else { 40.0 } } else if s.right.is_empty() { 22.0 } else { 70.0 };
        let max_w = r.right() - right_w - tx;
        let tf = if s.bold { semibold(15.6) } else { FontId::proportional(15.6) };
        let tc = s.title_col.unwrap_or(t.text1);
        if s.sub.is_empty() && s.sub_icon.is_none() {
            super::chrome::fit_text(ui.painter(), tx, r.center().y, s.title, tf, tc, max_w);
        } else {
            super::chrome::fit_text(ui.painter(), tx, r.center().y - 9.5, s.title, tf, tc, max_w);
            let sc = s.sub_col.unwrap_or(t.text2);
            let mut sx = tx;
            if let Some(ic) = s.sub_icon { self.icons.paint(ui, ic, Rect::from_center_size(pos2(sx + 7.0, r.center().y + 10.0), Vec2::splat(14.0)), sc); sx += 19.0; }
            super::chrome::fit_text(ui.painter(), sx, r.center().y + 10.0, &s.sub.replace('\n', " "), FontId::proportional(13.0), sc, max_w - (sx - tx));
        }
        if show_actions {
            let more = Rect::from_center_size(r.right_center() - Vec2::new(26.0, 0.0), Vec2::splat(24.0));
            let mresp = ui.interact(more, ui.id().with(("pg-more", &s.id)), Sense::click());
            if mresp.hovered() { ui.painter().circle_filled(more.center(), 12.0, t.ink_a(0.12)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            self.icons.paint(ui, "more-regular", Rect::from_center_size(more.center(), Vec2::splat(16.0)), t.text2);
            if mresp.clicked() { out.menu = Some(more); out.clicked = false; }
            if let Some((icon, tip)) = s.action {
                let ar = Rect::from_center_size(more.center() - Vec2::new(30.0, 0.0), Vec2::splat(26.0));
                let aresp = ui.interact(ar, ui.id().with(("pg-act", &s.id)), Sense::click());
                if aresp.hovered() { ui.painter().circle_filled(ar.center(), 13.0, t.ink_a(0.12)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                self.icons.paint(ui, icon, Rect::from_center_size(ar.center(), Vec2::splat(16.0)), t.text1);
                if aresp.on_hover_text(tip).clicked() { out.action = true; out.clicked = false; }
            }
        } else {
            if !s.right.is_empty() { ui.painter().text(pos2(r.right() - 12.0, r.center().y), egui::Align2::RIGHT_CENTER, s.right, FontId::proportional(12.5), t.text2); }
            if s.dot { ui.painter().circle_filled(r.right_center() - Vec2::new(if s.right.is_empty() { 20.0 } else { 6.0 }, if s.right.is_empty() { 0.0 } else { 14.0 }), 5.0, t.unread); }
        }
        if resp.secondary_clicked() { out.menu = ui.ctx().input(|i| i.pointer.interact_pos()).map(|p| Rect::from_center_size(p, Vec2::splat(2.0))); out.clicked = false; }
        out
    }

    /// Search field of a list card (rounded, t.search fill, magnifier left, clear button right).
    pub(crate) fn list_search(&mut self, ui: &mut egui::Ui, t: &Th, text: &mut String, hint: &str) {
        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width() - 6.0, 32.0), Sense::hover());
        ui.painter().rect_filled(r, 16.0, t.ink_a(0.07));
        self.icons.paint(ui, "search-regular", Rect::from_center_size(r.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(16.0)), t.text2);
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(r.min + Vec2::new(34.0, 0.0), r.max - Vec2::new(34.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        inner.add(egui::TextEdit::singleline(text).hint_text(RichText::new(hint).color(t.text2).size(14.5)).font(FontId::proportional(14.5)).frame(false).desired_width(r.width() - 68.0));
        if !text.is_empty() {
            let cr = Rect::from_center_size(r.right_center() - Vec2::new(18.0, 0.0), Vec2::splat(22.0));
            let resp = ui.interact(cr, ui.id().with("search-clear"), Sense::click());
            if resp.hovered() { ui.painter().circle_filled(cr.center(), 11.0, t.ink_a(0.12)); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            let (c, d) = (cr.center(), 4.0);
            ui.painter().line_segment([c - Vec2::splat(d), c + Vec2::splat(d)], Stroke::new(1.3, t.text1));
            ui.painter().line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], Stroke::new(1.3, t.text1));
            if resp.on_hover_text("Suche löschen").clicked() { text.clear(); }
        }
    }

    /// Centred illustration + title + text (+ optional link / button) in the middle of the available area of `ui`. Returns true when the link is clicked.
    pub(crate) fn empty_state(&mut self, ui: &mut egui::Ui, t: &Th, art: &'static str, art_size: f32, title: &str, body: &str, link: Option<&str>) -> bool {
        let area = ui.available_rect_before_wrap();
        let w = (area.width() - 48.0).clamp(120.0, 420.0);
        let g_title = ui.fonts(|f| f.layout(title.to_string(), semibold(18.0), t.text1, w));
        let g_body = ui.fonts(|f| f.layout(body.to_string(), FontId::proportional(14.5), t.text2, w));
        let link_h = if link.is_some() { 34.0 } else { 0.0 };
        let total = art_size + 18.0 + g_title.size().y + if body.is_empty() { 0.0 } else { 8.0 + g_body.size().y } + link_h;
        let mut y = (area.center().y - total / 2.0).max(area.top() + 8.0);
        let cx = area.center().x;
        self.icons.art(ui, art, Rect::from_center_size(pos2(cx, y + art_size / 2.0), Vec2::splat(art_size)));
        y += art_size + 18.0;
        let tp = pos2(cx - g_title.size().x / 2.0, y);
        y += g_title.size().y;
        ui.painter().galley(tp, g_title, t.text1);
        if !body.is_empty() {
            y += 8.0;
            ui.painter().galley(pos2(cx - g_body.size().x / 2.0, y), g_body.clone(), t.text2);
            y += g_body.size().y;
        }
        let mut clicked = false;
        if let Some(l) = link {
            let lw = ui.fonts(|f| f.layout_no_wrap(l.to_string(), semibold(14.0), t.link).size().x);
            let lr = Rect::from_min_size(pos2(cx - lw / 2.0, y + 10.0), Vec2::new(lw, 20.0));
            let resp = ui.interact(lr, ui.id().with(("empty-link", l)), Sense::click());
            if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
            ui.painter().text(lr.left_center(), egui::Align2::LEFT_CENTER, l, semibold(14.0), t.link);
            if resp.hovered() { ui.painter().hline(lr.x_range(), lr.bottom() - 1.0, Stroke::new(1.0, t.link)); }
            clicked = resp.clicked();
        }
        clicked
    }

    /// Shows the shared context menu `key` for the row stored by `open_page_menu`; returns (row key, picked item id).
    pub(crate) fn page_menu(&mut self, ctx: &egui::Context, t: &Th, key: &str, items: &[Entry]) -> Option<(String, String)> {
        let target = self.pst(ctx).menu_target?;
        let picked = menu::webex_menu(ctx, &mut self.icons, t, key, target.1, Side::Below, Align::Left, false, items)?;
        Some((target.0, picked))
    }
}

/// Centred single-line text, truncated.
pub(crate) fn fit_label(ui: &egui::Ui, center: egui::Pos2, text: &str, font: FontId, color: Color32, max_w: f32) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(10.0));
    let g = ui.painter().layout_job(job);
    ui.painter().galley(center - g.size() / 2.0, g, color);
}

/// The "Heute" / "Gestern" / weekday text of a Unix-ms timestamp for list rows ("16:20" today).
pub(crate) fn when_text(ms: i64, now_ms: i64) -> String {
    let day = 86_400_000_i64;
    let local_off = 0; // UTC (the data layer's times are UTC too)
    let (d0, d1) = ((ms + local_off).div_euclid(day), (now_ms + local_off).div_euclid(day));
    match d1 - d0 {
        0 => { let s = (ms + local_off).rem_euclid(day) / 1000; format!("{:02}:{:02}", s / 3600, s % 3600 / 60) }
        1 => "Gestern".into(),
        2..=6 => ["Do", "Fr", "Sa", "So", "Mo", "Di", "Mi"][(d0.rem_euclid(7)) as usize].into(), // 1970-01-01 was a Thursday
        _ => { let (y, m, d) = civil(d0); let _ = y; format!("{d:02}.{m:02}.") }
    }
}
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468; let era = z.div_euclid(146_097); let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1; let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + (m <= 2) as i64, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn when_text_formats() {
        let now = 1_700_000_000_000_i64; // 2023-11-14 22:13:20 UTC
        assert_eq!(when_text(now - 3_600_000, now), "21:13");
        assert_eq!(when_text(now - 86_400_000, now), "Gestern");
        assert_eq!(when_text(now - 86_400_000 * 3, now), "Sa"); // 2023-11-11 was a Saturday
        assert_eq!(when_text(now - 86_400_000 * 30, now), "15.10.");
    }
}

