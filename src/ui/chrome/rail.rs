//! The navigation rail: its entries (narrow round highlight / wide labelled pill) and the toggle button on the rail / list edge.
#![allow(clippy::too_many_arguments)]
use super::*;

impl App {
    /// One rail entry: wide = rounded pill (172 x 42) with label; narrow = round highlight.
    fn rail_item(&mut self, ui: &mut egui::Ui, t: &Th, icon: &'static str, label: &str, on: bool, badge: bool, h: f32) -> egui::Response {
        let w = if self.rail_wide { ui.available_width() } else { 40.0_f32.min(ui.available_width()) }; // collapsed: the 40 pt circle itself
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
        // navigationtab-background: hovered white/black 7 %, checked 20 % (pressed = checked).
        let bg = if on || resp.is_pointer_button_down_on() { Some(t.rail_sel) } else if resp.hovered() { Some(t.tk("navigationtab-background:hovered")) } else { None };
        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        if let Some(c) = bg { if self.rail_wide { ui.painter().rect_filled(r, h / 2.0, c); } else { ui.painter().circle_filled(r.center(), 20.0, c); } }
        let ic = if self.rail_wide { pos2(r.left() + 20.0, r.center().y) } else { r.center() };
        // navigationtab-text: normal 70 %, checked 95 %.
        let icon_col = if on { t.tk("navigationtab-text:checked") } else { t.tk("navigationtab-text") };
        self.icons.paint(ui, icon, Rect::from_center_size(ic, Vec2::splat(24.0) /* 24 pt icon box */), icon_col);
        // The rail badge is appNav-badge (red-60), not the blue mainList-indicator.
        if badge { ui.painter().circle_filled(ic + Vec2::new(10.0, -10.0), 5.0, if crate::ui::theme::decor::on(collabhost_plugin_api::deco::BADGE_CYCLE) { crate::ui::theme::decor::cycle(0).unwrap_or(t.tk("appNav-badge")) } else { t.tk("appNav-badge") }); } // = t.badge unless an accent theme sets it
        if self.rail_wide {
            ui.painter().text(pos2(r.left() + 44.0, r.center().y), egui::Align2::LEFT_CENTER, label, if on { semibold(14.3) } else { FontId::proportional(13.7) }, icon_col);
        }
        if self.rail_wide { resp } else { resp.on_hover_text(label) }
    }

    pub(crate) fn rail(&mut self, ctx: &egui::Context, t: &Th) {
        self.about_dialog(ctx, t);
        // Expand / collapse slides the rail between RAIL_NARROW and RAIL_WIDE.
        let k = ctx.animate_bool_with_time(egui::Id::new("rail-open"), self.rail_wide, 0.18);
        let width = RAIL_NARROW + (RAIL_WIDE - RAIL_NARROW) * k;
        // Collapsed: left 5 puts the 40 pt circle at 5-45, right 15 leaves exactly its 40 pt (60.8 - 5 - 15).
        let (m, mr, h, gap) = if self.rail_wide { (12, 12, 40.0, 5.0) } else { (5, 15, 40.0, 5.0) }; // same 45 pt pitch collapsed and expanded
        egui::SidePanel::left("rail").exact_width(width).resizable(false).show_separator_line(false)
            .frame(egui::Frame::default().inner_margin(egui::Margin { left: m, right: mr, top: 8, bottom: 8 })).show(ctx, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let unread = self.spaces.iter().any(|s| s.unread);
            for (i, (tab, off, on_icon, label)) in RAIL.iter().enumerate() {
                // Settings > App-Seitenleiste: unticked entries leave the rail.
                let sb = match tab { Tab::Messaging => "sb_messaging", Tab::Meetings => "sb_meetings", Tab::Calling => "sb_calling", Tab::Whiteboards => "sb_whiteboards", Tab::Contacts => "sb_contacts", _ => "" };
                if !sb.is_empty() && self.settings[sb] == false { if i == 2 { let (sr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover()); ui.painter().hline((sr.left() + 6.0)..=(sr.right() - 4.0), sr.center().y - 1.0, Stroke::new(1.0, t.ink_a(0.06))); } continue; }
                let on = self.tab == *tab;
                if self.rail_item(ui, t, if on { on_icon } else { off }, tr(label), on, *tab == Tab::Messaging && unread, h).clicked() { self.set_tab(*tab) }
                ui.add_space(gap);
                // Thin divider between the call/meeting entries and the rest.
                if i == 2 {
                    let (sr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
                    ui.painter().hline((sr.left() + 6.0)..=(sr.right() - 4.0), sr.center().y - 1.0, Stroke::new(1.0, t.ink_a(0.06)));
                }
            }
            let more = self.rail_item(ui, t, "more-regular", tr("More"), false, false, h);
            if more.clicked() { super::super::menu::menu_toggle(ctx, "more"); }
            let more_items = vec![
                super::super::menu::Item::new("voicemail", "Voicemail").icon("voicemail-regular").entry(),
                super::super::menu::Item::new("plugins", "Plugins & Themes").icon("apps-regular").entry(),
                // The "Info" entry; the settings page can show `credits_ui` itself and drop this item.
                super::super::menu::Item::new("about", "Info über Webox").icon("info-circle-regular").entry(),
                super::super::menu::Entry::Sep,
                super::super::menu::Item::new("rail", if self.rail_wide { "Beschriftungen ausblenden" } else { "Beschriftungen anzeigen" }).icon("application-panel-regular").shortcut("Alt+Umschalt+Q").entry(),
            ];
            if let Some(a) = super::super::menu::webex_menu(ctx, &mut self.icons, t, "more", more.rect, super::super::menu::Side::Right, super::super::menu::Align::Left, false, &more_items) {
                match a.as_str() {
                    "voicemail" => self.set_tab(Tab::Voicemail),
                    // The plugin list lives in Settings > Plugins.
                    "plugins" => { self.set_tab(Tab::Settings); ctx.data_mut(|d| d.insert_temp(egui::Id::new("settings-goto"), super::super::settings::nav::Page::Plugins)); }
                    "about" => super::super::menu::menu_open(ctx, "about"),
                    _ => self.act(Act::ToggleRail, ctx),
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let bh = 40.0;
                if self.rail_item(ui, t, "help-circle-filled", tr("Help"), false, false, bh).clicked() { system::open(super::super::profile_menu::HELP_CENTER_URL) }
                let on = self.tab == Tab::Settings;
                if self.rail_item(ui, t, "settings-filled", tr("Settings"), on, false, bh).clicked() { self.set_tab(Tab::Settings) }
            });
        });
    }

    /// The nav toggle on the rail / list boundary: while the pointer is near the edge a 1 px #444444 line (white 20 %,
    /// navigation-bar-divider-1) runs down it and a 20 pt round button (fill = the window gradient bottom #0F0F0F, ring white
    /// 70 %, thin chevron) sits centred in the area under the header. ">" opens the labelled rail, "<" closes it (same as
    /// Alt+Shift+Q, ToggleNavigationMenu).
    pub(crate) fn rail_edge_button(&mut self, ctx: &egui::Context, t: &Th) {
        let k = ctx.animate_bool_with_time(egui::Id::new("rail-open"), self.rail_wide, 0.18);
        let e = edge_inset(ctx);
        let bx = (e + RAIL_NARROW + (RAIL_WIDE - RAIL_NARROW) * k).round();
        let screen = ctx.screen_rect();
        let top = e + header_h();
        let cy = top + (screen.height() - e - top) / 2.0;
        let btn = Rect::from_center_size(pos2(bx - 0.5, cy), Vec2::splat(20.0));
        let near = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| (p.x - bx).abs() < 10.0 && p.y > top && ctx.layer_id_at(p).map_or(true, |l| l.order == egui::Order::Background || l.id == egui::Id::new("rail-edge"))); // not under an overlay
        if !near { return; }
        egui::Area::new(egui::Id::new("rail-edge")).fixed_pos(pos2(bx - 11.0, top)).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.set_clip_rect(Rect::from_min_size(pos2(bx - 11.0, top), Vec2::new(22.0, screen.height() - e - top)));
            ui.painter().vline(bx - 0.5, top..=(screen.bottom() - e), Stroke::new(1.0, t.sep));
            let resp = ui.interact(btn, egui::Id::new("rail-edge-btn"), Sense::click());
            ui.painter().circle_filled(btn.center(), 10.0, t.win_b);
            ui.painter().circle_stroke(btn.center(), 9.4, Stroke::new(1.2, if resp.hovered() { t.tk("navigationtab-text:checked") } else { t.tk("navigationtab-text") }));
            self.icons.paint(ui, if self.rail_wide { "arrow-left-optical_10" } else { "arrow-right-optical_10" }, Rect::from_center_size(btn.center(), Vec2::splat(11.0)), t.text1);
            // Tooltip: Webox's own wording (the pack has no German string for "Expand or collapse navigation menu").
            if resp.on_hover_text("Navigationsmenü ein-/ausklappen (Alt+Umschalt+Q)").clicked() { self.act(Act::ToggleRail, ctx) }
        });
    }
}
