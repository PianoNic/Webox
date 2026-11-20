//! Toast layout and drawing.
use super::*;

/// Builds a message text job: emoji become pictures (placeholders, drawn over the text afterwards).
pub(super) fn text_job(ctx: &egui::Context, j: &mut LayoutJob, lead: Option<(&str, Color32, FontId)>, body: &str, font: FontId, col: Color32) -> Vec<(usize, String)> {
    let mut emoji = vec![];
    let n = std::cell::Cell::new(0usize);
    let put = |j: &mut LayoutJob, s: &str, f: FontId, c: Color32| { n.set(n.get() + s.chars().count()); j.append(s, 0.0, TextFormat { font_id: f, color: c, ..Default::default() }); };
    if let Some((l, lc, lf)) = lead { put(j, l, lf, lc); if !body.is_empty() { put(j, " ", font.clone(), col); } }
    let text = body.replace(['\r', '\n'], " ");
    let sys = ui::emoji_text::ensure_font(ctx);
    let mut at = 0;
    for (s, e) in ui::emoji_text::clusters(&text) {
        put(j, &text[at..s], font.clone(), col);
        let cluster = &text[s..e];
        match ui::emoji_text::resolve(cluster) {
            Some(name) => { emoji.push((n.get(), name)); put(j, ui::emoji_text::PLACEHOLDER, font.clone(), col); }
            None if sys => put(j, cluster, FontId::new(font.size, egui::FontFamily::Name(ui::emoji_text::FAMILY.into())), col),
            None => put(j, ui::emoji_text::PLACEHOLDER, font.clone(), col),
        }
        at = e;
    }
    put(j, &text[at..], font, col);
    emoji
}

/// The incoming-message toast layout for the message toasts.
pub(super) fn layout_message(ctx: &egui::Context, t: &Th, ts: &Toast) -> Lay {
    use msg_toast::*;
    let lay = |j: LayoutJob| ctx.fonts(|f| f.layout_job(j));
    let col_x = ML + AVATAR_COL + GAP;
    let mut y = MT + 8.0;
    let title = (!ts.title.is_empty()).then(|| { let mut j = job(COL_W - 32.0, 1); j.append(&ts.title, 0.0, TextFormat { font_id: semibold(PT11), color: toast_text(t), ..Default::default() }); lay(j) });
    let title_y = y;
    y += title.as_ref().map_or(0.0, |g| g.size().y.max(TITLE_MIN_H));
    let sender = ts.lead.as_ref().map(|(l, tone)| {
        // Sender line in text-secondary; file / mention lines take their highlight colour.
        let c = if *tone == Tone::Reply { t.text2 } else { lead_color(t, *tone) };
        let mut j = job(COL_W, 1); j.append(l, 0.0, TextFormat { font_id: FontId::proportional(SENDER_PX), color: c, ..Default::default() }); lay(j)
    });
    let sender_y = if sender.is_some() { y += 8.0; let sy = y; y += sender.as_ref().map_or(0.0, |g| g.size().y.max(SENDER_MIN_H)); sy } else { y };
    y += 2.0;
    let body_y = y;
    let mut emoji = vec![];
    let body = (!ts.body.is_empty()).then(|| {
        let mut j = job(COL_W, 2);
        emoji = text_job(ctx, &mut j, None, &ts.body, FontId::proportional(PT11), toast_text(t));
        lay(j)
    });
    y += body.as_ref().map_or(0.0, |g| g.size().y.min(BODY_MAX_H));
    let h = (y + MB).max(MT + AVATAR + MB).clamp(100.0, 160.0); // min / max height of the card
    Lay { h, kicker: None, title, body, emoji, ind_w: AVATAR_COL, col_x, buttons: vec![], w: W, sender, ys: Some((title_y, sender_y, body_y)) }
}

pub(super) fn layout(ctx: &egui::Context, t: &Th, ts: &Toast) -> Lay {
    if message_layout(ts.kind) { return layout_message(ctx, t, ts); }
    let ind_w = if ts.avatar.is_some() { 48.0 } else { 40.0 };
    let ind_h: f32 = if ts.avatar.is_some() { 64.0 } else { 56.0 };
    let col_x = ind_w + 12.0;
    let col_w = W - col_x - 12.0 - 32.0; // right: the close button (24 px + 8 px margin) minus the 12 px margin
    let lay = |j: LayoutJob| ctx.fonts(|f| f.layout_job(j));
    let mut col_h = 0.0;
    let mut add = |g: &Option<Arc<Galley>>| if let Some(g) = g { col_h += if col_h > 0.0 { 8.0 } else { 0.0 } + g.size().y; };
    let kicker = (!ts.kicker.is_empty()).then(|| {
        let mut j = job(col_w, 1);
        j.append(&ts.kicker, 0.0, TextFormat { font_id: FontId::proportional(14.0), color: t.text2, ..Default::default() });
        lay(j)
    });
    add(&kicker);
    let title = (!ts.title.is_empty()).then(|| {
        let mut j = job(col_w, 2);
        j.append(&ts.title, 0.0, TextFormat { font_id: semibold(16.0), color: t.text1, ..Default::default() });
        lay(j)
    });
    add(&title);
    // Message text: emoji become pictures (placeholders, drawn over the text afterwards).
    let mut emoji = vec![];
    let body = (!ts.body.is_empty() || ts.lead.is_some()).then(|| {
        let mut j = job(col_w, if matches!(ts.kind, Kind::Message | Kind::Mention | Kind::MessageReminder) { 3 } else { 5 });
        let (font, col) = (FontId::proportional(if title.is_none() { 14.0 } else { 16.0 }), t.text1);
        let n = std::cell::Cell::new(0usize);
        let put = |j: &mut LayoutJob, s: &str, f: FontId, c: Color32| { n.set(n.get() + s.chars().count()); j.append(s, 0.0, TextFormat { font_id: f, color: c, ..Default::default() }); };
        if let Some((l, tone)) = &ts.lead { put(&mut j, l, semibold(16.0), lead_color(t, *tone)); if !ts.body.is_empty() { put(&mut j, " ", font.clone(), col); } }
        let text = ts.body.replace(['\r', '\n'], " ");
        let sys = ui::emoji_text::ensure_font(ctx);
        let mut at = 0;
        for (s, e) in ui::emoji_text::clusters(&text) {
            put(&mut j, &text[at..s], font.clone(), col);
            let cluster = &text[s..e];
            match ui::emoji_text::resolve(cluster) {
                Some(name) => { emoji.push((n.get(), name)); put(&mut j, ui::emoji_text::PLACEHOLDER, font.clone(), col); }
                None if sys => put(&mut j, cluster, FontId::new(font.size, egui::FontFamily::Name(ui::emoji_text::FAMILY.into())), col),
                None => put(&mut j, ui::emoji_text::PLACEHOLDER, font.clone(), col),
            }
            at = e;
        }
        put(&mut j, &text[at..], font, col);
        lay(j)
    });
    add(&body);
    let buttons: Vec<(Arc<Galley>, f32)> = ts.buttons.iter().map(|(_, l, p)| {
        let mut j = job(f32::INFINITY, 1);
        j.append(l, 0.0, TextFormat { font_id: semibold(14.0), color: if *p { t.white_btn_text } else { t.text1 }, ..Default::default() });
        let g = lay(j);
        let w = g.size().x + 32.0; // pill padding
        (g, w)
    }).collect();
    let btn = if buttons.is_empty() { 0.0 } else { 28.0 + 12.0 };
    Lay { h: ind_h.max(32.0 + col_h + btn), kicker, title, body, emoji, ind_w, col_x, buttons, w: W, sender: None, ys: None }
}

// Drawing.

#[derive(Default)]
pub(super) struct CardOut { pub(super) close: bool, pub(super) clicked: bool, pub(super) button: Option<usize>, pub(super) hovered: bool }

/// Chat-message toasts (message, mention) use the incoming-message layout ([`msg_toast`]); every other kind the generic card.
pub(super) fn message_layout(k: Kind) -> bool { matches!(k, Kind::Message | Kind::Mention) }

/// notificationToast-text (MomentumDark #FFFFFF @ 95 % / MomentumDefault #000000 @ 95 %).
pub(super) fn toast_text(t: &Th) -> Color32 { if ui::menu::light(t) { alpha(Color32::BLACK, 0.95) } else { alpha(Color32::WHITE, 0.95) } }

pub(super) fn draw_card(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, ts: &Toast, lay: &Lay, rect: Rect, photo: Option<egui::TextureId>, op: f32) -> CardOut {
    let mut out = CardOut::default();
    let light = ui::menu::light(t);
    // notificationToast-background / -border (MomentumDark #000000 + white 20 % / MomentumDefault #FFFFFF + black 20 %; the same
    // values as modal-primary-background / -border).
    let (bg, border) = if light { (Color32::WHITE, alpha(Color32::BLACK, 0.2)) } else { (hex(0x000000), alpha(Color32::WHITE, 0.2)) };
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_opacity(op);
        let p = ui.painter().clone();
        // Fill and 1 px border only, no shadow: on a dark desktop a light shadow reads as a white glow.
        p.rect_filled(rect, 0.0, bg); // square corners
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
        let card = ui.interact(rect, egui::Id::new(("toast-card", ts.id)), Sense::click());
        out.hovered = card.hovered();
        if card.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        out.clicked = card.clicked();
        // Message toast rows (avatar at (15, 11), title / sender / message at their own y).
        if let Some((ty, sy, by)) = lay.ys {
            if let Some((name, _)) = &ts.avatar {
                let r = Rect::from_min_size(rect.min + Vec2::new(msg_toast::ML, msg_toast::MT), Vec2::splat(msg_toast::AVATAR));
                ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| avatar_photo(ui, ic, t, name, msg_toast::AVATAR, false, None, None, photo));
            }
            let x = rect.left() + lay.col_x;
            if let Some(g) = &lay.title { p.galley(pos2(x, rect.top() + ty), g.clone(), toast_text(t)); }
            if let Some(g) = &lay.sender { p.galley(pos2(x, rect.top() + sy), g.clone(), toast_text(t)); }
            if let Some(g) = &lay.body {
                let origin = pos2(x, rect.top() + by);
                p.galley(origin, g.clone(), toast_text(t));
                let idx: Vec<usize> = lay.emoji.iter().map(|e| e.0).collect();
                for (r, (_, name)) in ui::emoji_text::placeholder_rects(g, &idx).into_iter().zip(&lay.emoji) {
                    if let Some(r) = r { let r = r.translate(origin.to_vec2()); ic.emoji(ui, name, Rect::from_center_size(r.center(), Vec2::splat(18.0))); }
                }
            }
        }
        // Indicator: 32 px avatar or 24 px icon at (16, 16).
        if lay.ys.is_some() {} else if let Some((name, _)) = &ts.avatar {
            let r = Rect::from_min_size(rect.min + Vec2::new(16.0, 16.0), Vec2::splat(32.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| avatar_photo(ui, ic, t, name, 32.0, false, None, None, photo));
        } else {
            ic.paint(ui, ts.icon(), Rect::from_min_size(rect.min + Vec2::new(16.0, 16.0), Vec2::splat(24.0)), if ts.kind == Kind::Info { t.text2 } else { t.accent });
        }
        // Text column: top 16, spacing 8.
        let x = rect.left() + lay.col_x;
        let mut y = rect.top() + 16.0;
        for g in [&lay.kicker, &lay.title].into_iter().flatten().filter(|_| lay.ys.is_none()) { p.galley(pos2(x, y), g.clone(), t.text1); y += g.size().y + 8.0; }
        if let Some(g) = lay.body.as_ref().filter(|_| lay.ys.is_none()) {
            let origin = pos2(x, y);
            p.galley(origin, g.clone(), t.text1);
            let idx: Vec<usize> = lay.emoji.iter().map(|e| e.0).collect();
            for (r, (_, name)) in ui::emoji_text::placeholder_rects(g, &idx).into_iter().zip(&lay.emoji) {
                if let Some(r) = r { let r = r.translate(origin.to_vec2()); ic.emoji(ui, name, Rect::from_center_size(r.center(), Vec2::splat(19.0))); }
            }
            y += g.size().y;
        }
        let _ = y;
        // Pill buttons: right = 12, 12 below the text, spacing 8; the first (primary) is leftmost.
        if !lay.buttons.is_empty() {
            let total: f32 = lay.buttons.iter().map(|b| b.1).sum::<f32>() + 8.0 * (lay.buttons.len() as f32 - 1.0);
            let mut bx = rect.right() - 12.0 - total;
            let by = rect.bottom() - 16.0 - 28.0 + 4.0; // the card's bottom padding is 16, the row is 28 high
            for (i, (g, w)) in lay.buttons.iter().enumerate() {
                let r = Rect::from_min_size(pos2(bx, by), Vec2::new(*w, 28.0));
                let resp = ui.interact(r, egui::Id::new(("toast-btn", ts.id, i)), Sense::click());
                let primary = ts.buttons[i].2;
                let hov = resp.hovered();
                if primary { p.rect_filled(r, 14.0, if hov { alpha(t.white_btn, 0.85) } else { t.white_btn }); }
                else {
                    p.rect_filled(r, 14.0, if hov { t.pill } else { Color32::TRANSPARENT });
                    p.rect_stroke(r, 14.0, Stroke::new(1.0, t.ink_a(0.3)), egui::StrokeKind::Inside); // button-secondary-border
                }
                p.galley(r.center() - g.size() / 2.0, g.clone(), t.text1);
                if hov { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
                if resp.clicked() { out.button = Some(i); }
                bx += w + 8.0;
            }
        }
        // Ghost XS round close button: generic cards 8 px from the top right; the message toast's close button ends the
        // button row at the content's right edge (margin 15), centred in its 32 px row below the top margin 11.
        let cr = if lay.ys.is_some() { Rect::from_min_size(pos2(rect.right() - msg_toast::MR - 24.0, rect.top() + msg_toast::MT + 4.0), Vec2::splat(24.0)) }
            else { Rect::from_min_size(pos2(rect.right() - 8.0 - 24.0, rect.top() + 8.0), Vec2::splat(24.0)) };
        let cresp = ui.interact(cr, egui::Id::new(("toast-close", ts.id)), Sense::click());
        if cresp.hovered() { p.circle_filled(cr.center(), 12.0, t.pill); ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        let icon = if lay.ys.is_some() { "cancel-regular" } else { "cancel-bold" };
        ic.paint(ui, icon, Rect::from_center_size(cr.center(), Vec2::splat(12.0)), t.text2);
        if cresp.clicked() { out.close = true; }
    });
    out
}