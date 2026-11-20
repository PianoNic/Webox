#![allow(clippy::too_many_arguments)]
use crate::*;

/// Narrowest / widest list card (pt).
const LIST_MIN_W: f32 = 280.0;
const LIST_MAX_W: f32 = 400.0;

/// Widest the list card may be: 400, and the chat keeps 560 pt so its tabs and icons never collide in a small window; never
/// below the minimum.
fn list_width_max(screen_w: f32, rail_w: f32) -> f32 { LIST_MAX_W.min(screen_w - rail_w - 4.0 - 560.0).max(LIST_MIN_W) }

impl App {
    /// Width of the list card (default 367 pt), stored as "list_w" in the settings JSON. Limits: see [`list_width_max`].
    pub(crate) fn list_card_width(&self, ctx: &egui::Context) -> f32 {
        const DEFAULT: f32 = 367.0;
        let rail_w = if self.rail_wide { RAIL_WIDE } else { super::chrome::RAIL_NARROW };
        let max = list_width_max(ctx.screen_rect().width(), rail_w);
        let shot = self.shot.is_some().then(|| std::env::var("WEBOX_SHOT_LISTW").ok().and_then(|v| v.parse::<f32>().ok())).flatten();
        let w = shot.or_else(|| ctx.data(|d| d.get_temp::<f32>(egui::Id::new("list_w")))).or_else(|| self.settings["list_w"].as_f64().map(|v| v as f32)).unwrap_or(DEFAULT);
        w.clamp(LIST_MIN_W, max)
    }

    /// The drag handle in the 4 px gap between the list card and the chat card: resize cursor, a thin line on hover, double-click resets.
    pub(crate) fn list_splitter(&mut self, ctx: &egui::Context, t: &Th, card_w: f32) {
        let rail_w = super::chrome::RAIL_NARROW + (RAIL_WIDE - super::chrome::RAIL_NARROW) * ctx.animate_bool_with_time(egui::Id::new("rail-open"), self.rail_wide, 0.18);
        let e = super::chrome::edge_inset(ctx); // the not-maximised margin around all content
        let x = e + rail_w + card_w + 2.0;
        let screen = ctx.screen_rect();
        let area = Rect::from_min_max(pos2(x - 5.0, e + super::chrome::header_h()), pos2(x + 5.0, screen.bottom() - e));
        egui::Area::new(egui::Id::new("list-splitter")).fixed_pos(area.min).order(egui::Order::Foreground).show(ctx, |ui| {
            let resp = ui.allocate_rect(area, Sense::click_and_drag());
            if resp.hovered() || resp.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                ui.painter().vline(x, (e + super::chrome::header_h() + 10.0)..=(screen.bottom() - e), Stroke::new(2.0, if resp.dragged() { t.focus } else { t.sep }));
            }
            if resp.dragged() {
                if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
                    let max = list_width_max(screen.width() - 2.0 * e, rail_w);
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new("list_w"), (p.x - e - rail_w - 2.0).clamp(LIST_MIN_W, max)));
                }
            }
            if resp.double_clicked() { ctx.data_mut(|d| d.insert_temp(egui::Id::new("list_w"), 367.0_f32)); self.save_list_w(367.0); }
            if resp.drag_stopped() { let w = ctx.data(|d| d.get_temp::<f32>(egui::Id::new("list_w"))).unwrap_or(367.0); self.save_list_w(w); }
        });
    }

    fn save_list_w(&mut self, w: f32) {
        if self.shot.is_some() { return; }
        self.settings["list_w"] = json!(w);
        system::save_settings(&self.cfg, &self.settings);
    }

    pub(crate) fn messaging(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let card_w = self.list_card_width(ctx);
        if !self.list_hidden {
            // Card with a 4 px gap to the conversation card.
            egui::SidePanel::left("spaces").exact_width(card_w + 4.0).resizable(false).show_separator_line(false)
                .frame(egui::Frame::default().inner_margin(egui::Margin { left: 0, right: 4, top: 0, bottom: 0 })).show(ctx, |ui| {
                // List card: theme-gradient-secondary, rounded top corners, flush bottom.
                let card = ui.available_rect_before_wrap();
                let _ = ui.allocate_rect(card, Sense::hover());
                let crect = card;
                vgrad(ui.painter(), crect, 10.0, t.card, t.card_b); crate::ui::theme::decor::card(ui.painter(), crect, 10.0, true);
                let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(
                    pos2(crect.left() + 8.0, crect.top() + crate::ui::theme::density::m().list_top), pos2(crect.right() - 6.0, crect.bottom()))));
                c.set_min_size(c.available_size());
                self.space_list(&mut c, t);
            });
        }

        if !self.list_hidden { self.list_splitter(ctx, t, card_w); }
        // No right margin: the chat card reaches the window's right edge; the 4 pt gap is only between the list card and the chat card.
        egui::CentralPanel::default().frame(egui::Frame::default().inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 0 })).show(ctx, |ui| {
                ui.set_min_size(ui.available_size());
                self.finish_create_space(ui.ctx());
                self.finish_direct(ui.ctx());
                // Nothing selected and no other pane open: the create-space pane is the empty view (as at startup).
                if self.selected.is_none() && self.new_space.is_none() && self.tab == Tab::Messaging && !super::new_message::is_open(ui.ctx()) && !super::invite::is_open(ui.ctx()) {
                    self.new_space = Some(String::new());
                }
                if self.new_space.is_some() { self.create_space_screen(ui, t, now) }
                else if self.selected.is_none() && super::new_message::is_open(ui.ctx()) { self.new_message_screen(ui, t, now) }
                else if self.selected.is_none() && super::invite::is_open(ui.ctx()) { self.invite_screen(ui, t, now) }
                else { self.conversation(ui, t, now) }
        });
        self.space_menus(ctx, t, now);
    }

    pub(crate) fn space_list(&mut self, ui: &mut egui::Ui, t: &Th) {
        let dm = crate::ui::theme::density::m();
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
        let mut filter_rect = Rect::NOTHING;
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let title = match (&self.team_sel, self.tab) {
                (Some(id), _) => self.teams.iter().find(|x| &x.id == id).map(|x| x.name.clone()).unwrap_or_default(),
                (None, Tab::Teams) => "Teams".into(),
                _ => tr("Messaging").into(),
            };
            ui.label(RichText::new(title).font(semibold(19.0)).color(t.text1));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                ui.painter().circle_filled(r.center(), 14.0, if resp.hovered() { t.white_btn.gamma_multiply(0.85) } else { t.white_btn });
                self.icons.paint(ui, "plus-regular", Rect::from_center_size(r.center(), Vec2::splat(16.0)), t.white_btn_text);
                if resp.on_hover_text(format!("{} (Strg+Umschalt+N)", tr("Create a space"))).clicked() { self.new_space = Some(String::new()) }
                // Filter glyph 10.4 x 8.8 pt centred 65.2 pt and the + centred 24 pt from the card's right edge (41.2 apart; here
                // 28 / 2 + 5.2 + 8 spacing + 28 / 2), glyph drawn at 16.
                ui.add_space(5.2);
                let tint = if self.filter == Filter::None { t.text2 } else { t.link };
                let fr = self.icons.button(ui, "filter-regular", 28.0, 16.0, tint, t.hover).on_hover_text(tr("Filter"));
                if fr.clicked() { super::menu::menu_toggle(ui.ctx(), "filters"); }
                filter_rect = fr.rect;
            });
        });
        {
            let mut items: Vec<super::menu::Entry> = FILTERS.iter().enumerate().map(|(i, (f, name))| super::menu::Item::new(&i.to_string(), tr(name)).checked(self.filter == *f).entry()).collect();
            items.extend(self.filter_menu_options()); // "Kompakte Ansicht anzeigen" (chrome/compact_list.rs)
            if let Some(a) = super::menu::webex_menu(ui.ctx(), &mut self.icons, t, "filters", filter_rect, super::menu::Side::Below, super::menu::Align::Right, true, &items) {
                if a == super::chrome::compact_list::MENU_ID { let on = !self.compact_view(); self.set_compact_view(on); }
                else if let Some((f, _)) = a.parse::<usize>().ok().and_then(|i| FILTERS.get(i)) { self.filter = *f; }
            }
        }
        if self.tab == Tab::Teams && self.team_sel.is_none() {
            ui.add_space(8.0);
            for tm in self.teams.clone() {
                let n = self.spaces.iter().filter(|s| s.team == tm.id).count();
                if self.row(ui, t, &tm.name, &format!("{n} spaces"), t.text2, false, false, true, Some(team_tones(t, tm.color, &tm.id).0), None, None).clicked() { self.team_sel = Some(tm.id.clone()); }
            }
            return;
        }
        ui.add_space(dm.list_title_gap);
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (l, name) in [(ListTab::All, "All"), (ListTab::Direct, "Direct"), (ListTab::Spaces, "Spaces"), (ListTab::Public, "Public")] {
                if text_tab(ui, tr(name), self.list_tab == l, t, t.tab_on).clicked() { self.list_tab = l }
            }
        });
        ui.add_space(3.0);
        // The scroll bar with arrow buttons (a 16 pt rounded track with a chevron at each end, as on the message list); the rows
        // leave its width free.
        crate::ui::widgets::vscroll().auto_shrink([false; 2]).arrow_bar(!t.light).show(ui, |ui| {
            ui.set_max_width(ui.available_width() - crate::ui::autoscroll::BAR_W);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.add_space(dm.list_scroll_top);
            // "Empfohlene Nachrichten" entry at the top of the list: mint disc with a blue bolt.
            let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), dm.recommended_h), Sense::hover());
            let r = Rect::from_min_max(full.min + Vec2::new(12.0, 0.0), full.max - Vec2::new(4.0, 0.0));
            let resp = ui.interact(r, ui.id().with("recommended"), Sense::click());
            if resp.hovered() { ui.painter().rect_filled(r, dm.recommended_h / 2.0, t.hover); }
            let c = r.left_center() + Vec2::new(dm.recommended_disc / 2.0 - 4.0, 0.0);
            // highlights_avatar_120.svg: #BEFADE disc with the blue bolt.
            self.icons.art(ui, "highlights_avatar_120", Rect::from_center_size(c, Vec2::splat(dm.recommended_disc)));
            ui.painter().text(r.left_center() + Vec2::new(dm.recommended_disc + 8.0, 0.0), egui::Align2::LEFT_CENTER, tr("Recommended messages"), FontId::proportional(16.4), t.text2);
            if resp.clicked() { self.filter = if self.filter == Filter::Notifications { Filter::None } else { Filter::Notifications } }
            ui.add_space(dm.recommended_gap);
            if self.skeleton_rows(ui, t) { return; }
            let list = self.visible_idx();
            let (fav, other): (Vec<usize>, Vec<usize>) = list.into_iter().partition(|&i| self.spaces[i].favorite);
            if !fav.is_empty() && self.section(ui, t, "fav", "Favorites", true) { ui.add_space(3.0); for &i in &fav { self.space_row_at(ui, t, i) } }
            if !other.is_empty() && (fav.is_empty() || self.section(ui, t, "other", "Other", false)) {
                ui.add_space(3.0);
                if super::new_message::is_open(ui.ctx()) { self.new_message_row(ui, t); } // the temporary "Neue Nachricht" row
                for &i in &other { self.space_row_at(ui, t, i) }
            }
        });
    }
}

#[cfg(test)]
mod list_width_tests {
    #[test]
    fn list_width_limits_are_the_originals() {
        // Wide window: at most 400.
        assert_eq!(super::list_width_max(1920.0, 58.0), 400.0);
        // Narrow window: the chat keeps 560, but never below the 280 minimum.
        assert_eq!(super::list_width_max(1000.0, 58.0), 1000.0 - 58.0 - 4.0 - 560.0);
        assert_eq!(super::list_width_max(700.0, 58.0), 280.0);
    }
}
