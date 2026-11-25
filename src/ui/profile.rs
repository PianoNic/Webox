//! The contact card of a person: the "Profile" tab of a direct chat and the card popup opened from the people list / global search.
//!
//! Pack strings: "Message", "Email", "Copy email", "Email copied", "Availability", "More",
//! "Call" / "Video call", "Add a contact", "Show organization". Sizes: photo 96, pills 32 high, fields 13/15 pt.
//! Only fields the data layer really has are shown (name, email, presence); phone / department / manager / organisation are
//! not fetched.
use crate::*;
use super::menu::{self, Align, Entry, Item, Side};


#[derive(Clone, Copy, PartialEq)]
enum Pill { Green, Outline }

/// A 32 pt pill button with an icon and a label; returns the response.
fn pill(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, icon: &'static str, label: &str, style: Pill) -> egui::Response {
    let tw = ui.fonts(|f| f.layout_no_wrap(label.to_string(), F::Body.semibold(), t.text1).size().x);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(tw + 56.0, 32.0), Sense::click());
    if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
    let down = resp.is_pointer_button_down_on();
    match style {
        // Join style = emerald-60 / -70 / -80 (as the banner's call buttons).
        Pill::Green => { ui.painter().rect_filled(r, 16.0, if down { hex(0x17422C) } else if resp.hovered() { hex(0x1C5E3D) } else { hex(0x248052) }); }
        Pill::Outline => {
            if resp.hovered() || down { ui.painter().rect_filled(r, 16.0, t.ink_a(if down { 0.14 } else { 0.08 })); }
            ui.painter().rect_stroke(r, 16.0, Stroke::new(1.0_f32, t.ink_a(0.3)), egui::StrokeKind::Inside);
        }
    }
    let fg = if style == Pill::Green { Color32::WHITE } else { t.text1 };
    ic.paint(ui, icon, Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), Vec2::splat(16.0)), fg);
    ui.painter().text(pos2(r.left() + 34.0, r.center().y), egui::Align2::LEFT_CENTER, label, F::Body.semibold(), fg);
    resp
}

fn card_key() -> egui::Id { egui::Id::new("contact-card-person") }

impl App {
    /// The direct space with `p`, opened; false when there is none.
    pub(crate) fn nav_open_chat(&mut self, p: &Person) -> bool {
        let id = self.spaces.iter().find(|s| s.direct && self.direct_person(s).is_some_and(|d|
            (!p.id.is_empty() && d.id == p.id) || (!p.email.is_empty() && d.email.eq_ignore_ascii_case(&p.email)) || d.name == p.name)).map(|s| s.id.clone());
        match id { Some(id) => { self.open_space(id); true } None => false }
    }

    /// Opens the card popup for `p` (drawn by `nav_card_popup`).
    pub(crate) fn contact_card_open(ctx: &egui::Context, p: &Person) {
        ctx.data_mut(|d| d.insert_temp(card_key(), (p.id.clone(), p.name.clone(), p.email.clone(), p.status.clone())));
    }

    /// The person's presence, from the live people list when it knows them better than the participant entry.
    pub(crate) fn live_person(&self, p: &Person) -> Person {
        let mut q = p.clone();
        if let Some(x) = self.people.iter().find(|x| (!x.id.is_empty() && x.id == p.id) || (!x.email.is_empty() && x.email.eq_ignore_ascii_case(&p.email))) {
            if q.name.is_empty() { q.name = x.name.clone(); }
            if q.email.is_empty() { q.email = x.email.clone(); }
            if !x.status.is_empty() { q.status = x.status.clone(); }
        }
        if self.me.id == p.id || (!p.email.is_empty() && self.me.email.eq_ignore_ascii_case(&p.email)) { q.status = self.me.status.clone(); }
        // Someone in no loaded space (search result, its card): the status followed for them (cloud/presence_sub.rs).
        else if let Some(s) = self.photos.outside_status.get(&p.id).filter(|s| !s.is_empty()) { q.status = s.clone(); }
        q
    }

    /// The card content: photo with presence badge, name, presence line, the action pills and the fields.
    pub(crate) fn contact_content(&mut self, ui: &mut egui::Ui, t: &Th, p: &Person, now: f64) {
        let p = self.live_person(p);
        let ctx = ui.ctx().clone();
        let photo = self.photo_for(&ctx, &p.id, &p.email);
        ui.horizontal_top(|ui| {
            avatar_photo(ui, &mut self.icons, t, &p.name, 96.0, false, None, Some(&p.status), photo);
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(10.0);
                ui.label(RichText::new(&p.name).font(F::Display.semibold()).color(t.text1));
                let line = self.person_line(&p, super::chrome::now_ms());
                if !line.is_empty() { ui.add_space(2.0); ui.label(RichText::new(line).size(14.0).color(t.text2)); }
            });
        });
        ui.add_space(16.0);
        let me = p.id == self.me.id || (!p.email.is_empty() && p.email.eq_ignore_ascii_case(&self.me.email));
        let mut more_anchor = Rect::NOTHING;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if !me {
                if pill(ui, &mut self.icons, t, "handset-regular", tr("Call"), Pill::Green).clicked() {
                    if p.email.is_empty() { self.toast(tr("Calls work in direct spaces"), now) } else { self.start_call(&p.email, &p.name, now) }
                }
                if pill(ui, &mut self.icons, t, "video-regular", tr("Video call"), Pill::Green).clicked() {
                    if p.email.is_empty() { self.toast(tr("Calls work in direct spaces"), now) } else { self.start_call_with(&p.email, &p.name, now, true) }
                }
                if pill(ui, &mut self.icons, t, "chat-regular", tr("Message"), Pill::Outline).clicked() {
                    // Opens the 1:1 (or creates it, see `open_or_create_direct`).
                    if self.open_or_create_direct(&p, now) { ctx.data_mut(|d| d.remove::<(String, String, String, String)>(card_key())); }
                }
            }
            let more = pill(ui, &mut self.icons, t, "more-regular", tr("More"), Pill::Outline);
            more_anchor = more.rect;
            if more.clicked() { menu::menu_toggle(&ctx, "contact-more"); }
        });
        // "Add a contact" (a contacts-service write) and "Show organization" (org-chart data) are not implemented: both
        // shown disabled, not as no-ops.
        let items = vec![
            Item::new("add", tr("Add a contact")).icon("participant-add-regular").disabled().entry(),
            Item::new("org", tr("Show organization")).icon("people-regular").disabled().entry(),
            Entry::Sep,
            Item::new("copy", tr("Copy email")).icon("copy-regular").enabled(!p.email.is_empty()).entry(),
        ];
        if let Some(a) = menu::webex_menu(&ctx, &mut self.icons, t, "contact-more", more_anchor, Side::Below, Align::Left, true, &items) {
            if a == "copy" { ctx.copy_text(p.email.clone()); self.toast(tr("Email copied"), now); }
        }
        ui.add_space(22.0);
        let field = |ui: &mut egui::Ui, label: &str, value: &str| {
            ui.label(RichText::new(label).size(13.0).color(t.text2));
            ui.add_space(1.0);
            ui.label(RichText::new(value).size(15.0).color(t.text1));
            ui.add_space(14.0);
        };
        if !p.email.is_empty() {
            ui.label(RichText::new(tr("Email")).size(13.0).color(t.text2));
            ui.add_space(1.0);
            ui.horizontal(|ui| {
                let r = ui.add(egui::Label::new(RichText::new(&p.email).size(15.0).color(t.link)).sense(Sense::click()));
                if r.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                if r.clicked() { system::open(&format!("mailto:{}", p.email)); }
                let (cr, cresp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                if cresp.hovered() { ui.painter().circle_filled(cr.center(), 12.0, t.hover); }
                self.icons.paint(ui, "copy-regular", Rect::from_center_size(cr.center(), Vec2::splat(15.0)), t.text2);
                if cresp.on_hover_text(tr("Copy email")).clicked() { ctx.copy_text(p.email.clone()); self.toast(tr("Email copied"), now); }
            });
            ui.add_space(14.0);
        }
        let avail = presence_text(&p.status);
        if !avail.is_empty() { field(ui, tr("Availability"), avail); }
    }

    /// The card popup (a person picked in the people list or the global search): modal-primary balloon, closes on Esc / click outside / X.
    pub(crate) fn nav_card_popup(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let Some((id, name, email, status)) = ctx.data(|d| d.get_temp::<(String, String, String, String)>(card_key())) else { return };
        let p = Person { id, name, email, status };
        let light = menu::light(t);
        let (bg, border) = if light { (Color32::WHITE, alpha(Color32::BLACK, 0.2)) } else { (hex(0x000000), alpha(Color32::WHITE, 0.2)) }; // modal-primary-*
        let screen = ctx.screen_rect();
        let w = 380.0;
        let pos = pos2(screen.center().x - w / 2.0, (screen.center().y - 190.0).max(60.0));
        let mut rect = Rect::NOTHING;
        let mut close = false;
        egui::Area::new(egui::Id::new("contact-card-area")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            let frame = egui::Frame::default().fill(bg).stroke(Stroke::new(1.0_f32, border)).corner_radius(12.0).inner_margin(24)
                .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if light { 50 } else { 170 }) });
            let r = frame.show(ui, |ui| {
                ui.set_width(w - 48.0);
                self.contact_content(ui, t, &p, now);
            });
            rect = r.response.rect;
            // X at the top right (as the dialogs).
            let xr = Rect::from_center_size(pos2(rect.right() - 22.0, rect.top() + 22.0), Vec2::splat(24.0));
            let xresp = ui.interact(xr, egui::Id::new("contact-card-x"), Sense::click());
            if xresp.hovered() { ui.painter().circle_filled(xr.center(), 12.0, t.ink_a(0.12)); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            self.icons.paint(ui, "cancel-regular", Rect::from_center_size(xr.center(), Vec2::splat(14.0)), t.text1);
            if xresp.clicked() { close = true; }
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) && !menu::menu_is_open(ctx, "contact-more") { close = true; }
        if let Some(pos) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None }) {
            if !rect.expand(2.0).contains(pos) && !menu::menu_is_open(ctx, "contact-more") { close = true; }
        }
        if close { ctx.data_mut(|d| d.remove::<(String, String, String, String)>(card_key())); }
    }

    /// The "Profile" tab of a direct chat: the same card, in the chat body.
    pub(crate) fn profile_tab(&mut self, b: &mut egui::Ui, t: &Th, person: Option<&Person>, now: f64) {
        crate::ui::widgets::vscroll().auto_shrink([false; 2]).show(b, |ui| {
            egui::Frame::default().inner_margin(egui::Margin { left: 32, right: 32, top: 28, bottom: 20 }).show(ui, |ui| {
                match person {
                    Some(p) => self.contact_content(ui, t, p, now),
                    None => { ui.label(RichText::new(tr("No profile information available")).size(15.0).color(t.text2)); }
                }
            });
        });
    }
}
