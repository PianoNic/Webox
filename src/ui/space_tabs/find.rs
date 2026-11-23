//! The pinned messages popover and the in-space find view.
use super::*;

impl App {
    // Angeheftete Nachrichten (pin icon).
    pub(crate) fn pins_popover(&mut self, ctx: &egui::Context, t: &Th, anchor: Rect, space_id: &str) {
        let s = st(ctx);
        if !s.pins || s.space != space_id { return; }
        let light = menu::light(t);
        // Balloon: background-primary (#121212), border wx-default-border (#3B3B3B), radius 5, width 400, arrow at the top right.
        let (bg, border) = if light { (Color32::WHITE, hex(0xEDEDED)) } else { (hex(0x121212), hex(0x3B3B3B)) };
        let screen = ctx.screen_rect();
        let (w, h, arrow) = (400.0, 330.0, 10.0);
        let x = (anchor.center().x + 28.0 - w).clamp(8.0, screen.right() - w - 8.0);
        let y = anchor.bottom() + 4.0;
        let outer = Rect::from_min_size(pos2(x, y), Vec2::new(w, h));
        let body = Rect::from_min_max(pos2(x, y + arrow), outer.max);
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        egui::Area::new(egui::Id::new("pins-popover")).fixed_pos(outer.min).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.allocate_exact_size(outer.size(), Sense::hover());
            let p = ui.painter().clone();
            p.add(egui::Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(if light { 40 } else { 140 }) }.as_shape(body, 5.0));
            p.rect_filled(body, 5.0, bg);
            p.rect_stroke(body, 5.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
            let cx = (anchor.center().x).clamp(body.left() + 20.0, body.right() - 20.0);
            p.add(egui::Shape::convex_polygon(vec![pos2(cx - 9.0, body.top() + 1.0), pos2(cx + 9.0, body.top() + 1.0), pos2(cx, body.top() - arrow)], bg, Stroke::NONE));
            p.line_segment([pos2(cx - 9.0, body.top()), pos2(cx, body.top() - arrow)], Stroke::new(1.0_f32, border));
            p.line_segment([pos2(cx + 9.0, body.top()), pos2(cx, body.top() - arrow)], Stroke::new(1.0_f32, border));
            // Header 46 pt, bold title centred; empty state: illustration, header, subtitle.
            p.text(pos2(body.center().x, body.top() + 23.0 + 5.0), egui::Align2::CENTER_CENTER, "Angeheftete Nachrichten", semibold(15.0), t.text1);
            self.icons.art(ui, "message-activity-192-cobalt-lime", Rect::from_center_size(pos2(body.center().x, body.top() + 140.0), Vec2::splat(150.0)));
            p.text(pos2(body.center().x, body.top() + 238.0), egui::Align2::CENTER_CENTER, "Angeheftete Nachrichten", semibold(15.0), t.text1);
            p.text(pos2(body.center().x, body.top() + 262.0), egui::Align2::CENTER_CENTER, "Die diesem Bereich angehefteten Nachrichten werden hier angezeigt.", FontId::proportional(12.5), t.text2);
        });
        if let Some(pos) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None }) {
            if !outer.expand(2.0).contains(pos) && !anchor.contains(pos) { close = true; }
        }
        if close { let mut s = st(ctx); s.pins = false; put(ctx, s); }
    }

    // In Unterhaltung suchen.
    /// While the search bar is open this replaces the message list. Returns true when it drew (the caller skips the list).
    pub(crate) fn find_view(&mut self, b: &mut egui::Ui, t: &Th, space: &Space, msgs: &[Message]) -> bool {
        let ctx = b.ctx().clone();
        let mut s = st(&ctx);
        if !s.find || s.space != space.id { return false; }
        let area = b.max_rect();
        let q = s.find_q.trim().to_string();
        let hits: Vec<&Message> = if q.is_empty() { vec![] } else { msgs.iter().rev().filter(|m| m.notice.is_empty() && m.text.to_lowercase().contains(&q.to_lowercase())).collect() };
        let n = hits.len();
        if s.find_i >= n { s.find_i = 0; }
        // Keyboard: Enter / Down = next result, Shift+Enter / Up = previous, Esc closes.
        let mut moved = false;
        let focused = ctx.memory(|m| m.has_focus(egui::Id::new("find-in-space"))); // never steal keys from the composer
        if n > 0 && focused {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown) || i.consume_key(Modifiers::NONE, Key::Enter)) { s.find_i = (s.find_i + 1) % n; moved = true; }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp) || i.consume_key(Modifiers::SHIFT, Key::Enter)) { s.find_i = (s.find_i + n - 1) % n; moved = true; }
        }
        let mut close = focused && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        // The bar: field pill (32 pt), "n von m", up, down, close.
        let bar = Rect::from_min_size(area.min + Vec2::new(24.0, 10.0), Vec2::new(area.width() - 48.0, 32.0));
        b.painter().rect_filled(bar, 16.0, t.search);
        self.icons.paint(b, "search-regular", Rect::from_center_size(bar.left_center() + Vec2::new(18.0, 0.0), Vec2::splat(16.0)), hex(0xEDEDED));
        let right_w = 150.0;
        let mut c = b.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(bar.min + Vec2::new(34.0, 0.0), pos2(bar.right() - right_w, bar.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let r = c.add(egui::TextEdit::singleline(&mut s.find_q).id(egui::Id::new("find-in-space")).hint_text(RichText::new("In Unterhaltung suchen").color(t.text2).size(14.5)).font(FontId::proportional(14.5)).frame(false).desired_width(bar.width() - right_w - 40.0));
        if std::mem::take(&mut s.focus_find) { r.request_focus(); }
        let cnt = if q.is_empty() { String::new() } else if n == 0 { "Keine Ergebnisse".into() } else { format!("{} von {}", s.find_i + 1, n) }; // pack: "%1 of %2"
        b.painter().text(pos2(bar.right() - 96.0, bar.center().y), egui::Align2::RIGHT_CENTER, cnt, FontId::proportional(13.0), t.text2);
        for (k, (icon, tip)) in [("arrow-up-regular", "Vorheriger"), ("arrow-down-regular", "Weiter"), ("cancel-regular", "Schließen")].into_iter().enumerate() {
            let br = Rect::from_center_size(pos2(bar.right() - 76.0 + 26.0 * k as f32 + 13.0, bar.center().y), Vec2::splat(24.0));
            let resp = b.interact(br, egui::Id::new(("find-btn", k)), if k < 2 && n == 0 { Sense::hover() } else { Sense::click() });
            if resp.hovered() { b.painter().circle_filled(br.center(), 12.0, t.ink_a(0.15)); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            self.icons.paint(b, icon, Rect::from_center_size(br.center(), Vec2::splat(14.0)), if k < 2 && n == 0 { t.text2 } else { t.text1 });
            if resp.on_hover_text(tip).clicked() {
                match k { 0 if n > 0 => { s.find_i = (s.find_i + n - 1) % n; moved = true; } 1 if n > 0 => { s.find_i = (s.find_i + 1) % n; moved = true; } 2 => close = true, _ => {} }
            }
        }
        // Results ("Suchergebnisse").
        let head_y = bar.bottom() + 14.0;
        if !q.is_empty() { b.painter().text(pos2(area.left() + 24.0, head_y + 8.0), egui::Align2::LEFT_CENTER, "Suchergebnisse", FontId::proportional(13.0), t.text2); }
        let body = Rect::from_min_max(pos2(area.left(), head_y + 22.0), area.max);
        let mut lst = b.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        lst.set_clip_rect(body.intersect(b.clip_rect()));
        let mut click: Option<usize> = None;
        crate::ui::widgets::vscroll().id_salt(("find-scroll", &space.id)).auto_shrink([false; 2]).show(&mut lst, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            if !q.is_empty() && n == 0 { empty_state(ui, &mut self.icons, t, Some("desert-open-results-192-cobalt-lime"), "Keine Ergebnisse gefunden"); }
            for (i, m) in hits.iter().enumerate() {
                let (full, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 66.0), Sense::hover());
                let r = Rect::from_min_max(full.min + Vec2::new(16.0, 0.0), pos2(full.max.x - 16.0, full.max.y - 2.0));
                let resp = ui.interact(r, ui.id().with(("find-hit", i)), Sense::click());
                if i == s.find_i { ui.painter().rect_filled(r, 12.0, t.selected); } else if resp.hovered() { ui.painter().rect_filled(r, 12.0, t.ink_a(0.05)); }
                if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                if moved && i == s.find_i { ui.scroll_to_rect(r, None); }
                let ph = self.photo_for(&ctx, "", &m.email);
                let mut av = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(r.min + Vec2::new(12.0, 10.0), Vec2::splat(32.0))));
                avatar_photo(&mut av, &mut self.icons, t, &m.author, 32.0, false, None, None, ph);
                let tx = r.left() + 56.0;
                let g = ui.painter().text(pos2(tx, r.top() + 16.0), egui::Align2::LEFT_CENTER, &m.author, semibold(14.0), t.text1);
                ui.painter().text(pos2(g.right() + 10.0, r.top() + 16.0), egui::Align2::LEFT_CENTER, when(m), FontId::proportional(12.5), t.text2);
                let job = hl_job(m.text.lines().next().unwrap_or(""), &q, 14.0, t.text2, t.text1, 1, r.width() - 56.0 - 16.0);
                let gal = ui.painter().layout_job(job);
                ui.painter().galley(pos2(tx, r.top() + 31.0), gal, t.text2);
                if resp.clicked() { click = Some(i); }
            }
        });
        // A click on a result shows it in the conversation (search bar closes, the list scrolls to it, it flashes).
        let jump = click.and_then(|i| hits.get(i).map(|m| m.id.clone()));
        if let Some(i) = click { s.find_i = i; }
        if close || jump.is_some() { s.find = false; }
        put(&ctx, s);
        if let Some(mid) = jump { self.goto_message(&ctx, &space.id, &mid); }
        true
    }
}
