//! "Open keyboard shortcut search" (Ctrl+/): a search popup listing every command with its key; picking one runs it.
//! Drawn with the dialog look of `ui/menu/`. Strings: "Search shortcuts" (placeholder), "No results".
use super::table::Kind;
use super::registry;
use crate::ui::menu::{light, webex_dialog, Button};
use crate::*;

#[derive(Default)]
pub(crate) struct Popup { q: String, sel: usize, was_open: bool, scroll_to: bool }

const W: f32 = 560.0;
const ROW: f32 = 40.0;

/// The commands the search offers, in the page's order.
fn candidates(q: &str, res: &[super::Resolved]) -> Vec<usize> {
    let reg = registry();
    let q = q.trim().to_lowercase();
    let mut out = Vec::new();
    for g in super::Group::SHOWN {
        for (i, c) in reg.cmds.iter().enumerate() {
            if c.group != g || !c.listed() || matches!(c.kind, Kind::Native) { continue; }
            let key = res.get(i).and_then(|r| r.key).map(|k| k.display()).unwrap_or_default();
            if q.is_empty() || c.label.to_lowercase().contains(&q) || key.to_lowercase().contains(&q) { out.push(i); }
        }
    }
    out
}

impl App {
    pub(crate) fn shortcut_search(&mut self, ctx: &egui::Context, t: &Th) {
        if !self.show_shortcuts { self.sc.popup.was_open = false; return; }
        let first = !self.sc.popup.was_open;
        if first { self.sc.popup = Popup { was_open: true, ..Default::default() }; }
        let reg = registry();
        let light = light(t);
        let (bg, border, text, text2, dim) = if light { (Color32::WHITE, alpha(Color32::BLACK, 0.2), alpha(Color32::BLACK, 0.95), alpha(Color32::BLACK, 0.7), alpha(Color32::BLACK, 0.4)) }
            else { (hex(0x000000), alpha(Color32::WHITE, 0.2), alpha(Color32::WHITE, 0.95), alpha(Color32::WHITE, 0.7), alpha(Color32::WHITE, 0.4)) }; // modal-primary-* of MomentumDark
        let hover = if light { alpha(Color32::BLACK, 0.07) } else { alpha(Color32::WHITE, 0.07) };
        let items = candidates(&self.sc.popup.q, &self.sc.res);
        let n = items.len();
        if self.sc.popup.sel >= n { self.sc.popup.sel = n.saturating_sub(1); }
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        let mut picked: Option<usize> = None;
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) && n > 0 { self.sc.popup.sel = (self.sc.popup.sel + 1).min(n - 1); self.sc.popup.scroll_to = true; }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) { self.sc.popup.sel = self.sc.popup.sel.saturating_sub(1); self.sc.popup.scroll_to = true; }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) { picked = items.get(self.sc.popup.sel).copied(); }

        let mut outer = Rect::NOTHING;
        let area = egui::Area::new(egui::Id::new("sc-search")).anchor(egui::Align2::CENTER_TOP, [0.0, 64.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::default().fill(bg).stroke(Stroke::new(1.0_f32, border)).corner_radius(12.0).inner_margin(12)
                .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if light { 50 } else { 170 }) })
                .show(ui, |ui| {
                    ui.set_width(W - 24.0);
                    // Search field with the search icon.
                    let (fr, _) = ui.allocate_exact_size(Vec2::new(W - 24.0, 36.0), Sense::hover());
                    ui.painter().rect_filled(fr, 18.0, if light { alpha(Color32::BLACK, 0.07) } else { alpha(Color32::WHITE, 0.11) });
                    self.icons.paint(ui, "search-regular", Rect::from_center_size(pos2(fr.left() + 20.0, fr.center().y), Vec2::splat(16.0)), text2);
                    let mut fui = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(fr.left() + 36.0, fr.top()), pos2(fr.right() - 12.0, fr.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                    let r = fui.add(egui::TextEdit::singleline(&mut self.sc.popup.q).id(egui::Id::new("sc-search-field")).frame(false).margin(Vec2::ZERO).desired_width(f32::INFINITY)
                        .hint_text(RichText::new(crate::trq!("Search shortcuts")).color(dim)).font(FontId::proportional(15.0)).text_color(text));
                    if first { r.request_focus(); }
                    if r.changed() { self.sc.popup.sel = 0; self.sc.popup.scroll_to = true; }
                    ui.add_space(8.0);
                    if items.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new(crate::trq!("No results")).size(14.0).color(text2));
                        ui.add_space(8.0);
                        return;
                    }
                    egui::ScrollArea::vertical().max_height(ROW * 9.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (pos, &i) in items.iter().enumerate() {
                            let c = &reg.cmds[i];
                            let (rr, resp) = ui.allocate_exact_size(Vec2::new(W - 24.0, ROW), Sense::click());
                            let on = pos == self.sc.popup.sel;
                            if resp.hovered() { if ctx.input(|i| i.pointer.delta() != Vec2::ZERO) { self.sc.popup.sel = pos; } ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
                            if on { ui.painter().rect_filled(rr, 8.0, hover); if self.sc.popup.scroll_to { ui.scroll_to_rect(rr, None); } }
                            let stub = matches!(c.kind, Kind::Stub);
                            let key = self.sc.res.get(i).and_then(|r| r.key).map(|k| k.display()).unwrap_or_default();
                            let kw = if key.is_empty() { 0.0 } else { ui.fonts(|f| f.layout_no_wrap(key.clone(), FontId::proportional(13.0), text2).size().x) + 16.0 };
                            crate::ui::chrome::fit_text(ui.painter(), rr.left() + 12.0, rr.center().y, c.label, FontId::proportional(14.5), if stub { dim } else { text }, rr.width() - 24.0 - kw - 8.0);
                            if !key.is_empty() {
                                let chip = Rect::from_min_max(pos2(rr.right() - 12.0 - kw, rr.center().y - 12.0), pos2(rr.right() - 12.0, rr.center().y + 12.0));
                                ui.painter().rect_filled(chip, 6.0, if light { alpha(Color32::BLACK, 0.11) } else { alpha(Color32::WHITE, 0.11) });
                                ui.painter().text(chip.center(), egui::Align2::CENTER_CENTER, &key, FontId::proportional(13.0), if stub { dim } else { text2 });
                            }
                            if resp.clicked() { picked = Some(i); }
                        }
                    });
                    self.sc.popup.scroll_to = false;
                });
        });
        outer = outer.union(area.response.rect);
        if let Some(pos) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None }) { if !outer.contains(pos) { close = true; } }
        if let Some(i) = picked {
            self.show_shortcuts = false;
            if reg.cmds[i].id != "ShortcutMenu" { self.run_command(i, ctx); }
        } else if close { self.show_shortcuts = false; }
    }

    /// Confirmation of the sign-out shortcut: "Are you sure you want to sign out?"; buttons "Cancel" / "Sign out".
    pub(crate) fn shortcut_dialogs(&mut self, ctx: &egui::Context, t: &Th) {
        if !self.sc.sign_out_ask { return; }
        let (title, cancel) = (crate::trq!("Sign out"), crate::trq!("Cancel"));
        match webex_dialog(ctx, t, "sc-sign-out", &title, &crate::trq!("Are you sure you want to sign out?"), None, &[Button::secondary("cancel", &cancel), Button::primary("ok", &title)]).as_deref() {
            Some("ok") => { self.sc.sign_out_ask = false; self.sign_out(); }
            Some(_) => self.sc.sign_out_ask = false,
            None => {}
        }
    }
}
