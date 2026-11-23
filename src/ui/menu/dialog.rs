//! Dialog backgrounds, the toast frame and the combo box.
use super::*;

/// [`webex_dialog`] with the card fill overridden (`None` = modal-primary-background).
#[allow(clippy::too_many_arguments)]
pub(crate) fn webex_dialog_bg(ctx: &egui::Context, t: &Th, k: &str, title: &str, body: &str, content: Option<&mut dyn FnMut(&mut egui::Ui)>, buttons: &[Button], fill: Option<Color32>) -> Option<String> {
    let light = light(t);
    let (bg, border, text, text2) = if light { (Color32::WHITE, alpha(Color32::BLACK, 0.2), alpha(Color32::BLACK, 0.95), alpha(Color32::BLACK, 0.7)) }
        else { (hex(0x000000), alpha(Color32::WHITE, 0.2), alpha(Color32::WHITE, 0.95), alpha(Color32::WHITE, 0.7)) }; // modal-primary-* of MomentumDark
    let bg = fill.unwrap_or(bg);
    let screen = ctx.screen_rect();
    let mut result: Option<String> = None;
    // Backdrop: swallows clicks behind the dialog.
    egui::Area::new(egui::Id::new(("webex-dialog-dim", k))).fixed_pos(screen.min).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(if light { 90 } else { 150 }));
        ui.allocate_rect(screen, Sense::click_and_drag());
    });
    let w = 402.0_f32.max(buttons.iter().map(|b| b.label.chars().count() as f32 * 8.5 + 56.0).sum::<f32>() + 40.0);
    let w = ctx.data(|d| d.get_temp::<f32>(dialog_width_id(k))).unwrap_or(w); // a fixed width set by the caller
    egui::Area::new(egui::Id::new(("webex-dialog", k))).anchor(egui::Align2::CENTER_CENTER, [0.0, -20.0]).order(egui::Order::Tooltip).show(ctx, |ui| {
        let frame = egui::Frame::default().fill(bg).stroke(Stroke::new(1.0_f32, border)).corner_radius(12.0).inner_margin(egui::Margin { left: 20, right: 20, top: 16, bottom: 20 })
            .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if light { 50 } else { 170 }) });
        frame.show(ui, |ui| {
            ui.set_width(w - 40.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).font(semibold(17.0)).color(text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                    if resp.hovered() { ui.painter().circle_filled(r.center(), 12.0, if light { alpha(Color32::BLACK, 0.08) } else { alpha(Color32::WHITE, 0.15) }); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                    let c = r.center(); let d = 5.0;
                    ui.painter().line_segment([c - Vec2::splat(d), c + Vec2::splat(d)], Stroke::new(1.3_f32, text));
                    ui.painter().line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], Stroke::new(1.3_f32, text));
                    if resp.clicked() { result = Some("cancel".into()); }
                });
            });
            if !body.is_empty() { ui.add_space(8.0); ui.label(RichText::new(body).size(14.5).color(text2)); }
            if let Some(f) = content { ui.add_space(12.0); f(ui); }
            ui.add_space(20.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for b in buttons.iter().rev() {
                    let tw = ui.fonts(|f| f.layout_no_wrap(b.label.clone(), semibold(14.0), text).size().x);
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(tw + 40.0, 32.0), Sense::click());
                    let hov = resp.hovered();
                    if hov { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                    let (fill, fg, stroke) = if b.danger { (if hov { hex(0xA12512) } else { hex(0xD43519) }, Color32::WHITE, Stroke::NONE) }
                        else if b.primary { (if hov { if light { hex(0x2B2B2B) } else { hex(0xDCDDDB) } } else if light { hex(0x121212) } else { hex(0xF2F3F1) }, if light { Color32::WHITE } else { hex(0x121212) }, Stroke::NONE) }
                        else { (if hov { if light { alpha(Color32::BLACK, 0.07) } else { alpha(Color32::WHITE, 0.12) } } else { Color32::TRANSPARENT }, text, Stroke::new(1.0_f32, if light { alpha(Color32::BLACK, 0.3) } else { alpha(Color32::WHITE, 0.3) }) ) };
                    ui.painter().rect_filled(r, 16.0, fill);
                    if stroke.width > 0.0 { ui.painter().rect_stroke(r, 16.0, stroke, egui::StrokeKind::Inside); }
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, &b.label, semibold(14.0), fg);
                    if resp.clicked() { result = Some(b.id.clone()); }
                }
            });
        });
    });
    if result.is_none() && ctx.input(|i| i.key_pressed(Key::Escape)) { result = Some("cancel".into()); }
    result
}

/// The frame of a toast: black card, 1 px border (white @ 20 %), radius 8.
pub(crate) fn toast_frame(t: &Th) -> egui::Frame {
    let p = pal(t);
    egui::Frame::default().fill(p.bg).stroke(Stroke::new(1.0_f32, p.border)).corner_radius(8.0).inner_margin(12)
        .shadow(egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(if light(t) { 40 } else { 120 }) })
}

/// (border, text) opacity of the combo field: combobox-border normal / hovered / pressed 50 %, checked / focused 90 %,
/// disabled 20 %; combobox-text 95 %, disabled 40 %.
pub(super) fn combo_alphas(enabled: bool, open: bool) -> (f32, f32) { if !enabled { (0.2, 0.4) } else if open { (0.9, 0.95) } else { (0.5, 0.95) } }

/// The combo box: 32 high, radius 8, text padding 12, chevron at the right, list = the black balloon menu 4 px below, as wide
/// as the field, current entry checked. Colours (`combobox-background` / `-border` / `-text` / `-icon`): fill #000000 (hovered
/// white 7 %, pressed / open white 11 %, disabled none), border white 50 % (open 90 %, disabled 20 %), text and chevron white
/// 95 % (disabled 40 %); light theme the same with black on #FFFFFF. The list's balloon uses the menu palette
/// (`menulistbackground-primary-*`: #000000, white 20 %).
/// Returns the field's response; `changed()` is true on the frame an entry was picked (`*selected` updated).
pub(crate) fn dropdown(ui: &mut egui::Ui, icons: &mut icons::Icons, t: &Th, id: &str, selected: &mut usize, options: &[String], width: f32) -> egui::Response {
    let ctx = ui.ctx().clone();
    let enabled = ui.is_enabled();
    let key = format!("dropdown-{id}");
    let open = menu_is_open(&ctx, &key);
    let (r, mut resp) = ui.allocate_exact_size(Vec2::new(width.min(ui.available_width()).max(60.0), 32.0), if enabled { Sense::click() } else { Sense::hover() });
    let lt = light(t);
    let base = if lt { Color32::BLACK } else { Color32::WHITE };
    let (border_a, text_a) = combo_alphas(enabled, open);
    if enabled && resp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
    let p = ui.painter();
    // Hovered wins over open (white 7 %); normal #000000.
    let fill = if !enabled { Color32::TRANSPARENT } else if resp.is_pointer_button_down_on() { alpha(base, 0.11) } else if resp.hovered() { alpha(base, 0.07) } else if open { alpha(base, 0.11) } else if lt { Color32::WHITE } else { Color32::BLACK };
    p.rect_filled(r, 8.0, fill);
    p.rect_stroke(r, 8.0, Stroke::new(1.0_f32, alpha(base, border_a)), egui::StrokeKind::Inside);
    let col = alpha(base, text_a);
    super::super::chrome::fit_text(p, r.left() + 12.0, r.center().y, options.get(*selected).map_or("", |s| s.as_str()), FontId::proportional(15.0), col, r.width() - 12.0 - 34.0);
    // The chevron points up while the list is open.
    icons.paint(ui, if open { "arrow-up-regular" } else { "arrow-down-regular" }, Rect::from_center_size(pos2(r.right() - 17.0, r.center().y), Vec2::splat(16.0)), col);
    if resp.clicked() { menu_toggle(&ctx, &key); }
    let items: Vec<Entry> = options.iter().enumerate().map(|(i, o)| Item::new(&i.to_string(), o.as_str()).checked(i == *selected).entry()).collect();
    if let Some(a) = menu_impl(&ctx, icons, t, &key, r, Side::Below, Align::Left, false, &items, Some(r.width())) {
        if let Ok(i) = a.parse::<usize>() { if i != *selected { *selected = i; resp.mark_changed(); } }
    }
    resp
}
