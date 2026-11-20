//! The voicemail list, detail and menu.
use super::*;

impl App {
    // Voicemail.
    pub(crate) fn voicemail_list(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let mut st = self.pst(&ctx);
        let mut play_req: Option<(usize, u32)> = None;
        self.list_header(ui, t, "Voicemail", None, None);
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (i, n) in ["Posteingang", "Gesendet", "Papierkorb"].iter().enumerate() {
                if text_tab(ui, n, st.vm_tab as usize == i, t, t.tab_on).clicked() { st.vm_tab = i as u8; st.vm_sel = None; }
            }
        });
        ui.add_space(6.0);
        let all = self.voicemails();
        let nowms = super::super::chrome::now_ms();
        let mut menu_req: Option<(String, Rect)> = None;
        let mut visible = 0;
        crate::ui::widgets::vscroll().auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            for (i, v) in all.iter().enumerate() {
                let trash = st.vm_deleted.contains(&i);
                let in_tab = match st.vm_tab { 0 => !v.sent && !trash, 1 => v.sent && !trash, _ => trash };
                if !in_tab { continue; }
                visible += 1;
                let unread = v.unread && !st.vm_read.contains(&i);
                let p = self.person_for(&v.name, &v.email);
                let photo = p.as_ref().and_then(|p| self.photo_for(ui.ctx(), &p.id, &p.email));
                let sub = format!("{}{}", duration_text(v.secs), if v.urgent { "  ·  Dringend" } else { "" });
                let right = when_text(v.at_ms, nowms);
                let mut spec = RowSpec::new(format!("vm{i}"), &v.name);
                spec.sub = &sub; spec.right = &right; spec.bold = unread; spec.dot = unread; spec.selected = st.vm_sel == Some(i); spec.photo = photo;
                spec.status = p.as_ref().map(|p| p.status.as_str()); spec.action = Some(("play-filled", "Voicemail wiedergeben"));
                if v.urgent { spec.sub_col = Some(missed_color(t)); }
                let out = self.page_row(ui, t, &spec);
                if out.clicked || out.action { st.vm_sel = Some(i); st.vm_read.insert(i); }
                if out.action { st.vm_playing = Some(i); play_req = Some((i, v.secs)); }
                if let Some(r) = out.menu { st.vm_sel = Some(i); menu_req = Some((format!("{i}"), r)); }
            }
        });
        let empty = visible == 0;
        self.set_pst(&ctx, st.clone());
        if empty { /* the detail card shows the empty state */ }
        if let Some((i, secs)) = play_req { self.vm_toggle(i, secs, now); }
        if let Some((k, r)) = menu_req { self.open_page_menu(&ctx, "vm-row", &k, r); }
    }

    pub(crate) fn voicemail_detail(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let ctx = ui.ctx().clone();
        let mut st = self.pst(&ctx);
        let all = self.voicemails();
        let sel = st.vm_sel.and_then(|i| all.get(i).cloned().map(|v| (i, v)));
        let Some((i, v)) = sel else {
            let (title, body) = match st.vm_tab {
                1 => ("Keine Sprachnachrichten", "Hier finden Sie alle Sprachnachrichten, die Sie gesendet haben. Ihre gesendeten Elemente sind gerade leer."),
                2 => ("Keine Sprachnachrichten", "Hallo Recycling-Star! Heute ist nichts im Papierkorb. Wenn Sie Sprachnachrichten löschen, finden Sie sie hier."),
                _ => ("Keine Sprachnachrichten", "Hier finden Sie alle Ihre Sprachnachrichten. Derzeit sind keine Nachrichten vorhanden. Genießen Sie die Stille."),
            };
            // A signed-in account without a voicemail service gets the "not set up" text.
            let body = if self.client.is_some() { "Offenbar ist Ihre Voicemail-Funktion noch nicht eingerichtet. Wenden Sie sich an Ihren Administrator, um zu beginnen." } else { body };
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
            self.empty_state(&mut c, t, "empty_voicemail", 160.0, title, if all.is_empty() || st.vm_tab != 0 { body } else { "Wählen Sie eine Sprachnachricht aus der Liste aus." }, None);
            return;
        };
        let p = self.person_for(&v.name, &v.email);
        let area = ui.max_rect().shrink2(Vec2::new(40.0, 36.0));
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        self.person_header(&mut c, t, &v.name, &v.email, p.as_ref(), 72.0);
        if !v.number.is_empty() { c.add_space(4.0); c.label(RichText::new(&v.number).size(13.5).color(t.text2)); }
        c.add_space(18.0);
        // Player: play / pause, slider, time ("Voicemail wiedergeben"). Voicemails exist only in `--sample` (made up, no audio
        // file): the player runs its position over the message's duration; a click on the slider seeks.
        let w = c.available_width().min(560.0);
        let (r, _) = c.allocate_exact_size(Vec2::new(w, 48.0), Sense::hover());
        c.painter().rect_filled(r, 24.0, t.ink_a(0.07));
        let pb = Rect::from_center_size(r.left_center() + Vec2::new(26.0, 0.0), Vec2::splat(32.0));
        let presp = c.interact(pb, c.id().with("vm-play"), Sense::click());
        let (pos, playing) = self.vm_position(i, v.secs, now);
        c.painter().circle_filled(pb.center(), 16.0, if presp.hovered() { t.white_btn.gamma_multiply(0.85) } else { t.white_btn });
        self.icons.paint(&c, if playing { "pause-filled" } else { "play-filled" }, Rect::from_center_size(pb.center(), Vec2::splat(16.0)), t.white_btn_text);
        if presp.on_hover_text("Voicemail wiedergeben").clicked() { self.vm_toggle(i, v.secs, now); st.vm_read.insert(i); }
        let tr_ = Rect::from_min_max(pos2(pb.right() + 14.0, r.center().y - 2.0), pos2(r.right() - 100.0, r.center().y + 2.0));
        let frac = if v.secs > 0 { (pos / v.secs as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
        c.painter().rect_filled(tr_, 2.0, t.ink_a(0.25));
        c.painter().rect_filled(Rect::from_min_max(tr_.min, pos2(tr_.left() + tr_.width() * frac, tr_.bottom())), 2.0, t.text1);
        c.painter().circle_filled(pos2(tr_.left() + tr_.width() * frac, tr_.center().y), 6.0, t.text1);
        let sresp = c.interact(tr_.expand2(Vec2::new(0.0, 8.0)), c.id().with("vm-seek"), Sense::click());
        if let Some(p) = sresp.interact_pointer_pos().filter(|_| sresp.clicked()) {
            let f = ((p.x - tr_.left()) / tr_.width()).clamp(0.0, 1.0) as f64 * v.secs as f64;
            self.call.mc.vm_pos = Some((i, f, if playing { Some(now) } else { None }));
        }
        if playing { ui.ctx().request_repaint_after(std::time::Duration::from_millis(200)); }
        c.painter().text(r.right_center() - Vec2::new(16.0, 0.0), egui::Align2::RIGHT_CENTER, format!("{} / {}", duration_text(pos as u32), duration_text(v.secs)), FontId::proportional(12.5), t.text2);
        c.add_space(14.0);
        c.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if !v.email.is_empty() && self.pill(ui, t, "Anrufen", Pill::Join, Some("handset-filled")).clicked() { self.start_call(&v.email, &v.name, now); }
            if self.pill(ui, t, if st.vm_deleted.contains(&i) { "Wiederherstellen" } else { "Löschen" }, Pill::Secondary, None).clicked() {
                if !st.vm_deleted.remove(&i) { st.vm_deleted.insert(i); }
                st.vm_sel = None;
            }
        });
        c.add_space(18.0);
        c.label(RichText::new("Abschrift").font(semibold(15.0)).color(t.text1));
        c.add_space(4.0);
        let text = if v.transcript.is_empty() { "Keine Abschrift verfügbar." } else { v.transcript.as_str() }; // Webox's own wording
        let mut tc = c.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(c.cursor().min, Vec2::new(w, 120.0))));
        tc.add(egui::Label::new(RichText::new(text).size(14.5).color(t.text2)).wrap());
        self.set_pst(&ctx, st);
    }

    pub(crate) fn voicemail_menu(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let items = vec![
            Item::new("play", "Voicemail wiedergeben").icon("play-filled").entry(), Item::new("call", "Anrufen").icon("handset-regular").entry(),
            Item::new("card", "Visitenkarte").icon("contact-card-regular").entry(), Entry::Sep, Item::new("delete", "Löschen").icon("delete-regular").danger().entry(),
        ];
        if let Some((key, pick)) = self.page_menu(ctx, t, "vm-row", &items) {
            let all = self.voicemails();
            let Some(i) = key.parse::<usize>().ok() else { return };
            let Some(v) = all.get(i).cloned() else { return };
            match pick.as_str() {
                "play" => { let mut s = self.pst(ctx); s.vm_sel = Some(i); s.vm_read.insert(i); self.set_pst(ctx, s); self.vm_toggle(i, v.secs, now); }
                "call" => if !v.email.is_empty() { self.start_call(&v.email, &v.name, now) } else { self.no_phone_calling(now) },
                "card" => { let mut s = self.pst(ctx); let at = s.menu_target.as_ref().map(|m| m.1).unwrap_or(Rect::NOTHING); s.contact_card = Some((v.email.clone(), v.name.clone(), at)); self.set_pst(ctx, s); }
                _ => { let mut s = self.pst(ctx); s.vm_deleted.insert(i); s.vm_sel = None; self.set_pst(ctx, s); }
            }
        }
    }
}
