//! "Contacts": contact list (favourites + all contacts, presence, search), profile pane and the contact card popover
//! ("Contact card"). Strings from the language packs; illustration `Contacts_empty_space`.
//! Not shown yet: contact groups, phone numbers, company, department, job title, manager, personal meeting room link, and the
//! organisation directory as a "Directory" search.
#![allow(clippy::too_many_arguments)]
use crate::*;
use super::menu::{self, Entry, Item};
use super::page_kit::*;

impl App {
    /// A person is a favourite when its direct space is a favourite (the only favourite store the data layer has).
    fn is_favorite_person(&self, p: &Person) -> Option<(String, bool)> {
        self.spaces.iter().find(|s| s.direct && s.title == p.name).map(|s| (s.id.clone(), s.favorite))
    }

    pub(crate) fn contacts_list(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let mut st = self.pst(&ctx);
        // No "+" (Add contact): adding a contact writes to the contacts service, which is not implemented; a button that
        // cannot work is left out rather than kept as a no-op.
        let _ = self.list_header(ui, t, tr("Contacts"), None, None);
        ui.add_space(8.0);
        let mut q = std::mem::take(&mut st.contact_q);
        self.list_search(ui, t, &mut q, tr("Search for a name"));
        st.contact_q = q.clone();
        ui.add_space(6.0);
        let ql = q.to_lowercase();
        let mut people: Vec<Person> = self.people.iter().filter(|p| ql.is_empty() || p.name.to_lowercase().contains(&ql) || p.email.to_lowercase().contains(&ql)).cloned().collect();
        people.sort_by_cached_key(|p| p.name.to_lowercase()); // per frame: one lowercase copy per person, not per comparison
        let (fav, other): (Vec<Person>, Vec<Person>) = people.into_iter().partition(|p| self.is_favorite_person(p).is_some_and(|f| f.1));
        ui.spacing_mut().scroll = egui::style::ScrollStyle { bar_width: 10.0, bar_inner_margin: 2.0, bar_outer_margin: 0.0, ..egui::style::ScrollStyle::solid() };
        let mut menu_req: Option<(String, Rect)> = None;
        let mut chat: Option<Person> = None;
        let empty = fav.is_empty() && other.is_empty();
        crate::ui::widgets::vscroll().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            for (key, label, list, star) in [ // section() translates the label
                ("c-fav", "Favorites", &fav, true), ("c-all", "All contacts", &other, false)] {
                if list.is_empty() { continue; }
                if !self.section(ui, t, key, label, star) { continue; }
                ui.add_space(3.0);
                for p in list.iter() {
                    let photo = self.photo_for(ui.ctx(), &p.id, &p.email);
                    let sub = self.person_line(p, super::chrome::now_ms());
                    let mut spec = RowSpec::new(format!("ct-{}", p.id), &p.name);
                    spec.sub = &sub; spec.status = Some(&p.status); spec.photo = photo; spec.selected = st.contact_sel.as_deref() == Some(p.id.as_str());
                    spec.action = Some(("chat-regular", tr("Chat")));
                    let out = self.page_row(ui, t, &spec);
                    if out.clicked { st.contact_sel = Some(p.id.clone()); }
                    if out.double || out.action { chat = Some(p.clone()); }
                    if let Some(r) = out.menu { st.contact_sel = Some(p.id.clone()); menu_req = Some((p.id.clone(), r)); }
                }
                ui.add_space(6.0);
            }
            if empty {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| { ui.label(RichText::new(tr("No results")).font(semibold(15.0)).color(t.text1)); });
            }
        });
        self.set_pst(&ctx, st);
        if let Some(p) = chat { self.open_or_create_direct(&p, now); }
        if let Some((k, r)) = menu_req { self.open_page_menu(&ctx, "contact-row", &k, r); }
    }

    pub(crate) fn contacts_detail(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let st = self.pst(&ctx);
        let Some(p) = st.contact_sel.as_ref().and_then(|id| self.people.iter().find(|p| &p.id == id).cloned()) else {
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
            self.empty_state(&mut c, t, "Contacts_empty_space", 160.0, tr("Contacts"),
                tr("Add people to your contacts list and organize them here."), None);
            return;
        };
        let area = ui.max_rect().shrink2(Vec2::new(40.0, 36.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        self.person_header(&mut c, t, &p.name, "", Some(&p), 96.0);
        c.add_space(18.0);
        c.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if self.round_labeled(ui, t, "chat-filled", tr("Chat"), t.white_btn, t.white_btn_text, 72.0).clicked() { self.open_or_create_direct(&p, now); }
            if self.round_labeled(ui, t, "handset-filled", tr("Call"), t.green, hex(0x121212), 72.0).clicked() { self.start_call(&p.email, &p.name, now); }
            if self.round_labeled(ui, t, "contact-card-filled", tr("Contact card"), t.ink_a(0.14), t.text1, 84.0).clicked() {
                let at = Rect::from_center_size(ui.min_rect().center_top() + Vec2::new(0.0, 140.0), Vec2::splat(4.0));
                let mut s = self.pst(ui.ctx()); s.contact_card = Some((p.email.clone(), p.name.clone(), at)); self.set_pst(ui.ctx(), s);
            }
        });
        c.add_space(24.0);
        c.label(RichText::new(tr("Profile")).font(semibold(15.0)).color(t.text1));
        c.add_space(6.0);
        let w = c.available_width().min(520.0);
        let nowms = super::chrome::now_ms();
        for (label, value) in [(tr("Name"), p.name.clone()), (tr("Email address"), p.email.clone()), (tr("Status"), self.person_line(&p, nowms))] {
            if value.is_empty() { continue; }
            let (r, _) = c.allocate_exact_size(Vec2::new(w, 34.0), Sense::hover());
            c.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, label, FontId::proportional(13.5), t.text2);
            c.painter().text(r.left_center() + Vec2::new(150.0, 0.0), egui::Align2::LEFT_CENTER, value, FontId::proportional(14.5), t.text1);
            c.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0_f32, t.ink_a(0.08)));
        }
    }

    pub(crate) fn contacts_menu(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let key = self.pst(ctx).menu_target.as_ref().map(|m| m.0.clone()).unwrap_or_default();
        let person = self.people.iter().find(|p| p.id == key).cloned();
        let fav = person.as_ref().and_then(|p| self.is_favorite_person(p));
        let mut items = vec![
            Item::new("chat", tr("Chat")).icon("chat-regular").entry(), Item::new("call", tr("Call")).icon("handset-regular").entry(),
            Item::new("card", tr("Contact card")).icon("contact-card-regular").entry(), Entry::Sep,
        ];
        if let Some((_, on)) = &fav { items.push(Item::new("fav", if *on { tr("Remove from favorites") } else { tr("Add to favorites") }).icon(if *on { "favorite-filled" } else { "favorite-regular" }).entry()); }
        // "Remove contact": contacts-service write not implemented (see contacts_list) → disabled.
        items.push(Item::new("remove", tr("Remove contact")).icon("delete-regular").disabled().entry());
        if let Some((_, pick)) = self.page_menu(ctx, t, "contact-row", &items) {
            let Some(p) = person else { return };
            match pick.as_str() {
                "chat" => { self.open_or_create_direct(&p, now); }
                "call" => self.start_call(&p.email, &p.name, now),
                "card" => { let mut s = self.pst(ctx); let at = s.menu_target.as_ref().map(|m| m.1).unwrap_or(Rect::NOTHING); s.contact_card = Some((p.email.clone(), p.name.clone(), at)); self.set_pst(ctx, s); }
                "fav" => if let Some((id, on)) = fav { self.set_favorite(&id, !on) },
                _ => {}
            }
        }
    }

    /// The contact card popover ("Contact card"): photo, name, presence, Chat / Call, e-mail with copy. Black balloon like the other popovers.
    pub(crate) fn contact_card_popup(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let st = self.pst(ctx);
        let Some((email, name, anchor)) = st.contact_card.clone() else { return };
        let p = self.person_for(&name, &email);
        let w = 320.0;
        let screen = ctx.screen_rect();
        let pos = pos2((anchor.center().x - w / 2.0).clamp(8.0, (screen.right() - w - 8.0).max(8.0)), (anchor.bottom() + 6.0).clamp(60.0, (screen.bottom() - 330.0).max(60.0)));
        let light = menu::light(t);
        let (bg, border) = if light { (Color32::WHITE, alpha(Color32::BLACK, 0.2)) } else { (Color32::BLACK, t.ink_a(0.2)) };
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        let mut chat = false; let mut call = false; let mut copy = false;
        let resp = egui::Area::new(egui::Id::new("contact-card")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::default().fill(bg).stroke(Stroke::new(1.0_f32, border)).corner_radius(12.0).inner_margin(16)
                .shadow(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(if light { 40 } else { 140 }) }).show(ui, |ui| {
                ui.set_width(w - 32.0);
                self.person_header(ui, t, &name, "", p.as_ref(), 64.0);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    chat = self.round_labeled(ui, t, "chat-filled", tr("Chat"), t.white_btn, t.white_btn_text, 72.0).clicked();
                    call = !email.is_empty() && self.round_labeled(ui, t, "handset-filled", tr("Call"), t.green, hex(0x121212), 72.0).clicked();
                });
                if !email.is_empty() {
                    ui.add_space(12.0);
                    ui.label(RichText::new(tr("Email:")).size(12.5).color(t.text2));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&email).size(14.0).color(t.text1));
                        if self.icons.button(ui, "copy-regular", 26.0, 16.0, t.text2, t.ink_a(0.12)).on_hover_text(tr("Copy email")).clicked() { copy = true; }
                    });
                }
            });
        });
        // Click outside closes it.
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !resp.response.rect.contains(p)));
        if outside && ctx.input(|i| i.time) > 0.0 && !ctx.is_using_pointer() { close = true; }
        if copy { ctx.copy_text(email.clone()); self.toast(tr("Email copied"), now); }
        if chat { close = true; let who = p.clone().unwrap_or(Person { name: name.clone(), email: email.clone(), id: email.clone(), ..Default::default() }); self.open_or_create_direct(&who, now); }
        if call { close = true; self.start_call(&email, &name, now); }
        if close { let mut s = self.pst(ctx); s.contact_card = None; self.set_pst(ctx, s); }
    }
}
