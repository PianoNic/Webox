//! Settings > "Tastenkombinationen": search field, sections with one row per command (title, key label, Global checkbox,
//! "…" menu), key capture and the conflict dialogs. Colour tokens `shortcutinput-*`, `listitem-background`,
//! `label-secondary-text`, `separator-primary`; German strings from the pack. Key label 28 high, 4 px inner margin, 8 px radius.
use crate::shortcuts::{self, conflict_message, default_taken_message, find_conflict, registry, Combo, Conflict, Group, CONFLICT_TITLE, DEFAULT_TAKEN};
use crate::ui::menu::{light, menu_is_open, menu_open, webex_dialog, webex_menu, Align, Button, Item, Side};
use crate::*;
use std::collections::HashSet;

/// Per-page UI state (lives in `App::sc.page`).
#[derive(Default)]
pub(crate) struct Page {
    q: String,
    collapsed: HashSet<usize>,
    edit: Option<Edit>,
    dlg: Option<Dlg>,
    applied: Option<(usize, f64)>,
    menu: Option<(usize, Rect)>,
    last_pass: u64,
}
struct Edit { idx: usize, bad: Option<Combo> }
enum Dlg { Reassign { idx: usize, combo: Combo, c: Conflict }, Reset { idx: usize, combo: Combo, c: Conflict }, ResetAll }

enum Do { Edit(usize), Global(usize, bool), Menu(usize, Rect), Group(usize), Remove(usize), Reset(usize), ResetAll }

#[derive(Clone, Copy, PartialEq)]
enum Look { Inactive, Reassign, Available, Unavailable }

const ROW_H: f32 = 40.0; // title height + 10, pill 28 + margins
const HEAD_H: f32 = 36.0; // section header height
const ACTIONS_W: f32 = 64.0; // actions column 60 + right margin 4
const INFO: &str = concat!("Mit \"Global\" können Sie ", crate::app_name!(), "-Tastenkombinationen verwenden, wenn die App minimiert ist oder im Hintergrund ausgeführt wird. Deaktivierte Tastenkombinationen können nicht global festgelegt werden.");
const NO_GLOBAL: &str = "Diese Tastenkombination kann nicht als \"Global\" festgelegt werden.";
const HINT: &str = "Geben Sie mithilfe der Tastatur eine neue Kombination ein.";

impl App {
    pub(crate) fn shortcuts_page(&mut self, ui: &mut egui::Ui, t: &Th) {
        self.sc_load();
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let reg = registry();
        let lt = light(t);
        let base = if lt { Color32::BLACK } else { Color32::WHITE };
        let (text, text2, dim, sep) = (alpha(base, 0.95), alpha(base, 0.7), alpha(base, 0.4), alpha(base, if lt { 0.46 } else { 0.4 }));

        let pass = ctx.cumulative_pass_nr();
        if pass.saturating_sub(self.sc.page.last_pass) > 3 { self.sc.page.edit = None; self.sc.page.dlg = None; self.sc.page.menu = None; } // the page was away
        self.sc.page.last_pass = pass;
        self.sc_capture(&ctx, now);
        self.sc_page_dialogs(&ctx, t, now);

        let mut todo: Vec<Do> = Vec::new();
        let w = ui.available_width();
        // Search field with the search icon.
        let (fr, _) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::hover());
        ui.painter().rect_filled(fr, 8.0, alpha(base, 0.07));
        ui.painter().rect_stroke(fr, 8.0, Stroke::new(1.0_f32, alpha(base, 0.5)), egui::StrokeKind::Inside);
        self.icons.paint(ui, "search-regular", Rect::from_center_size(pos2(fr.left() + 20.0, fr.center().y), Vec2::splat(16.0)), text2);
        let mut fui = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(fr.left() + 36.0, fr.top()), pos2(fr.right() - 12.0, fr.bottom()))).layout(egui::Layout::left_to_right(egui::Align::Center)));
        fui.add(egui::TextEdit::singleline(&mut self.sc.page.q).id(egui::Id::new("sc-page-search")).frame(false).margin(Vec2::ZERO).desired_width(f32::INFINITY)
            .hint_text(RichText::new("Tastenkombinationen für die Suche").color(dim)).font(FontId::proportional(14.0)).text_color(text));
        ui.add_space(12.0);

        // Column headers.
        let (hr, _) = ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::hover());
        let hp = ui.painter();
        hp.text(pos2(hr.left() + 8.0, hr.center().y), egui::Align2::LEFT_CENTER, "Beschreibung", FontId::proportional(13.0), text2);
        let col_r = hr.right() - ACTIONS_W - 16.0;
        hp.text(pos2(col_r, hr.center().y), egui::Align2::RIGHT_CENTER, "Tastenkombination", FontId::proportional(13.0), text2);
        let gx = hr.right() - ACTIONS_W + 10.0;
        hp.text(pos2(gx, hr.center().y), egui::Align2::CENTER_CENTER, "Global", FontId::proportional(13.0), text2);
        let ir = Rect::from_center_size(pos2(gx + 38.0, hr.center().y), Vec2::splat(18.0));
        self.icons.paint(ui, "info_16", ir, text2);
        ui.interact(ir, egui::Id::new("sc-global-info"), Sense::hover()).on_hover_text(INFO);
        ui.painter().hline(hr.x_range(), hr.bottom() + 1.0, Stroke::new(1.0_f32, sep));
        ui.add_space(6.0);

        // List.
        let q = self.sc.page.q.trim().to_lowercase();
        let mut shown = 0;
        for (gi, g) in Group::SHOWN.iter().enumerate() {
            let rows: Vec<usize> = reg.cmds.iter().enumerate().filter(|(i, c)| c.group == *g && c.listed() && (q.is_empty() || {
                let key = self.sc.res.get(*i).and_then(|r| r.key).map(|k| k.display()).unwrap_or_default();
                c.label.to_lowercase().contains(&q) || key.to_lowercase().contains(&q) })).map(|(i, _)| i).collect();
            if rows.is_empty() { continue; }
            let open = !q.is_empty() || !self.sc.page.collapsed.contains(&gi);
            // Section header: chevron + title, click toggles.
            let (r, resp) = ui.allocate_exact_size(Vec2::new(w, HEAD_H), Sense::click());
            if resp.hovered() { ui.painter().rect_filled(r, 8.0, alpha(base, 0.07)); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            self.icons.paint(ui, if open { "arrow-down-optical_12" } else { "arrow-right-optical_10" }, Rect::from_center_size(pos2(r.left() + 20.0, r.center().y), Vec2::splat(12.0)), text);
            ui.painter().text(pos2(r.left() + 36.0, r.center().y), egui::Align2::LEFT_CENTER, g.title(), semibold(14.5), text);
            if resp.clicked() && q.is_empty() { todo.push(Do::Group(gi)); }
            if !open { continue; }
            for i in rows { shown += 1; self.sc_row(ui, t, i, w, [text, text2, dim, base], now, &mut todo); }
        }
        if shown == 0 {
            ui.add_space(24.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                let (ar, _) = ui.allocate_exact_size(Vec2::new(70.0, 70.0), Sense::hover());
                self.icons.art(ui, "desert-open-results-192-cobalt-lime", ar);
                ui.add_space(16.0);
                ui.add(egui::Label::new(RichText::new("Es wurden keine Tastenkombinationen gefunden, die Ihren Suchkriterien entsprechen.").color(text2).size(14.0)).wrap());
            });
        }
        #[cfg(target_os = "linux")]
        {
            ui.add_space(12.0);
            let mut on = self.settings[shortcuts::GLOBAL_KEY].as_bool().unwrap_or(false);
            if crate::ui::settings::kit::check(ui, &mut self.icons, t, "Globale Tastenkombinationen systemweit registrieren", &mut on, true, false) {
                self.settings[shortcuts::GLOBAL_KEY] = serde_json::json!(on);
                if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
            }
            ui.label(RichText::new("Belegt die Tasten auch in anderen Programmen").color(text2).size(12.5));
        }
        // Reset button.
        ui.add_space(16.0);
        let tw = ui.fonts(|f| f.layout_no_wrap("Auf Standard zurücksetzen".into(), semibold(14.0), text).size().x);
        let (br, bresp) = ui.allocate_exact_size(Vec2::new(tw + 40.0, 32.0), Sense::click());
        if bresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
        ui.painter().rect_filled(br, 16.0, if bresp.hovered() { alpha(base, 0.12) } else { Color32::TRANSPARENT });
        ui.painter().rect_stroke(br, 16.0, Stroke::new(1.0_f32, alpha(base, 0.3)), egui::StrokeKind::Inside);
        ui.painter().text(br.center(), egui::Align2::CENTER_CENTER, "Auf Standard zurücksetzen", semibold(14.0), text);
        if bresp.clicked() { todo.push(Do::ResetAll); }
        ui.add_space(16.0);

        // Row menu.
        if let Some((i, anchor)) = self.sc.page.menu {
            let c = &reg.cmds[i];
            let key = self.sc.res[i].key;
            let items = [
                Item::new("edit", "Bearbeiten").enabled(c.customizable).entry(),
                Item::new("remove", "Entfernen").enabled(c.customizable && key.is_some()).entry(),
                Item::new("reset", "Auf Standard zurücksetzen").enabled(c.customizable && self.sc.res[i].custom).entry(),
            ];
            let k = format!("sc-more-{i}");
            match webex_menu(&ctx, &mut self.icons, t, &k, anchor, Side::Below, Align::Right, false, &items).as_deref() {
                Some("edit") => todo.push(Do::Edit(i)), Some("remove") => todo.push(Do::Remove(i)), Some("reset") => todo.push(Do::Reset(i)), _ => {}
            }
            if !menu_is_open(&ctx, &k) { self.sc.page.menu = None; }
        }

        for d in todo {
            match d {
                Do::Edit(i) => { self.sc.page.edit = Some(Edit { idx: i, bad: None }); self.sc.page.applied = None; }
                Do::Global(i, on) => self.sc_set_global(i, on),
                Do::Menu(i, r) => { menu_open(&ctx, &format!("sc-more-{i}")); self.sc.page.menu = Some((i, r)); }
                Do::Group(g) => { if !self.sc.page.collapsed.remove(&g) { self.sc.page.collapsed.insert(g); } }
                Do::Remove(i) => self.sc_assign(i, None, None),
                Do::Reset(i) => match reg.cmds[i].default.and_then(|d| find_conflict(&reg.cmds, &self.sc.res, i, d).map(|c| (d, c))) {
                    Some((d, c)) => self.sc.page.dlg = Some(Dlg::Reset { idx: i, combo: d, c }),
                    None => self.sc_reset(i, None),
                },
                Do::ResetAll => self.sc.page.dlg = Some(Dlg::ResetAll),
            }
        }
        if self.sc.page.edit.is_some() || self.sc.page.applied.is_some() { ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
    }

    /// One shortcut row: title, [warning], key label or "+ Tastenkombination hinzufügen", Global checkbox, "…".
    fn sc_row(&mut self, ui: &mut egui::Ui, t: &Th, i: usize, w: f32, pal: [Color32; 4], now: f64, todo: &mut Vec<Do>) {
        let [text, text2, dim, base] = pal;
        let ctx = ui.ctx().clone();
        let lt = light(t);
        let reg = registry();
        let (c, r) = (&reg.cmds[i], self.sc.res[i]);
        let (row, rresp) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::click());
        if rresp.hovered() { ui.painter().rect_filled(row, 8.0, alpha(base, 0.07)); } // listitem-background hovered
        let editing = self.sc.page.edit.as_ref().is_some_and(|e| e.idx == i);
        if editing { ui.painter().rect_filled(row, 8.0, alpha(base, 0.11)); } // pressed (current item)
        let actions = Rect::from_min_max(pos2(row.right() - ACTIONS_W, row.top()), pos2(row.right() - 4.0, row.bottom()));
        // Right column: more button, global checkbox.
        if c.customizable {
            let mr = Rect::from_center_size(pos2(actions.right() - 14.0, row.center().y), Vec2::splat(28.0));
            let mresp = ui.interact(mr, egui::Id::new(("sc-more-btn", i)), Sense::click());
            if mresp.hovered() { ui.painter().circle_filled(mr.center(), 14.0, alpha(base, 0.11)); ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            self.icons.paint(ui, "more-bold", Rect::from_center_size(mr.center(), Vec2::splat(16.0)), text);
            if mresp.clicked() { todo.push(Do::Menu(i, mr)); }
            if rresp.secondary_clicked() { todo.push(Do::Menu(i, Rect::from_center_size(ctx.input(|i| i.pointer.interact_pos()).unwrap_or(mr.center()), Vec2::splat(2.0)))); }
            // Check box: 18 px box; enabled only for a command that can be global and has a key.
            let cb = Rect::from_center_size(pos2(actions.left() + 10.0, row.center().y), Vec2::splat(18.0));
            let can = c.can_be_global && r.key.is_some();
            let cresp = ui.interact(cb.expand(4.0), egui::Id::new(("sc-global", i)), if can { Sense::click() } else { Sense::hover() });
            let col = if can { text2 } else { dim };
            // checkbox-checked-background / -icon (see settings/kit.rs `ctl`).
            let kc = crate::ui::settings::kit::ctl(t);
            if r.global { ui.painter().rect_filled(cb, 4.0, if can { kc.checked_bg.pick(&cresp) } else { kc.checked_bg.normal }); self.icons.paint(ui, "check-bold", Rect::from_center_size(cb.center(), Vec2::splat(12.0)), kc.checked_icon); }
            else { ui.painter().rect_stroke(cb, 4.0, Stroke::new(1.5_f32, if can && cresp.hovered() { text } else { col }), egui::StrokeKind::Inside); }
            if can && cresp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            if !c.can_be_global { cresp.on_hover_text(NO_GLOBAL); } else if can && cresp.clicked() { todo.push(Do::Global(i, !r.global)); }
        }
        // Key label.
        let right = actions.left() - 16.0;
        let key = r.key;
        let applied = self.sc.page.applied.is_some_and(|(a, at)| a == i && now - at < 1.5);
        let (look, label) = match (&self.sc.page.edit, key) {
            (Some(e), _) if e.idx == i => match e.bad { Some(b) => (Look::Unavailable, b.display()), None => (Look::Reassign, String::new()) },
            (_, Some(k)) if applied => (Look::Available, k.display()),
            (_, Some(k)) => (Look::Inactive, k.display()),
            _ => (Look::Inactive, String::new()),
        };
        let mut left_limit = right;
        if look != Look::Inactive || key.is_some() || editing {
            let tw = if label.is_empty() { 0.0 } else { ui.fonts(|f| f.layout_no_wrap(label.clone(), FontId::proportional(14.0), text).size().x) };
            let badge = matches!(look, Look::Available | Look::Unavailable);
            let pw = if label.is_empty() { 112.0 } else { tw + 16.0 + if badge { 24.0 } else { 0.0 } }; // preferred width 112 when empty; padding 8 + 8
            let pr = Rect::from_min_size(pos2(right - pw, row.center().y - 14.0), Vec2::new(pw, 28.0));
            let presp = ui.interact(pr, egui::Id::new(("sc-pill", i)), if c.customizable && !editing { Sense::click() } else { Sense::hover() });
            if c.customizable && presp.hovered() && !editing { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); }
            let hov = presp.hovered() && c.customizable && !editing;
            paint_pill(ui, pr, look, lt, hov, &label, text, if badge { Some(if look == Look::Available { "check-circle-filled" } else { "error-legacy-filled" }) } else { None }, &mut self.icons);
            if presp.clicked() { todo.push(Do::Edit(i)); }
            // A press anywhere else leaves the edit mode (focus loss).
            if editing && self.sc.page.dlg.is_none() && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !pr.contains(p))) { self.sc.page.edit = None; }
            if editing { presp.on_hover_text(HINT); }
            left_limit = pr.left() - 8.0;
        } else if c.customizable {
            // Pill button, secondary XS, left icon plus-bold: "Tastenkombination hinzufügen".
            let label = "Tastenkombination hinzufügen";
            let tw = ui.fonts(|f| f.layout_no_wrap(label.into(), semibold(13.0), text).size().x);
            let pr = Rect::from_min_size(pos2(right - (tw + 16.0 + 18.0 + 8.0), row.center().y - 12.0), Vec2::new(tw + 42.0, 24.0));
            let presp = ui.interact(pr, egui::Id::new(("sc-add", i)), Sense::click());
            if presp.hovered() { ctx.set_cursor_icon(egui::CursorIcon::PointingHand); ui.painter().rect_filled(pr, 12.0, alpha(base, 0.11)); }
            ui.painter().rect_stroke(pr, 12.0, Stroke::new(1.0_f32, alpha(base, 0.3)), egui::StrokeKind::Inside);
            self.icons.paint(ui, "plus-bold", Rect::from_center_size(pos2(pr.left() + 16.0, pr.center().y), Vec2::splat(12.0)), text);
            ui.painter().text(pos2(pr.left() + 28.0, pr.center().y), egui::Align2::LEFT_CENTER, label, semibold(13.0), text);
            if presp.clicked() { todo.push(Do::Edit(i)); }
            left_limit = pr.left() - 8.0;
        }
        // Warning: the default combination is taken by a custom binding of another command.
        if let Some(by) = r.default_taken_by {
            let wr = Rect::from_center_size(pos2(left_limit - 12.0, row.center().y), Vec2::splat(20.0));
            self.icons.paint(ui, "warning-regular", Rect::from_center_size(wr.center(), Vec2::splat(16.0)), t.star);
            ui.interact(wr, egui::Id::new(("sc-warn", i)), Sense::hover()).on_hover_text(format!("{DEFAULT_TAKEN}\n{}", default_taken_message(&reg.cmds, i, by)));
            left_limit = wr.left() - 4.0;
        }
        // Title.
        let tcol = if c.customizable || r.key.is_none() { text } else { text };
        crate::ui::chrome::fit_text(ui.painter(), row.left() + 12.0, row.center().y, c.label, FontId::proportional(14.5), tcol, (left_limit - row.left() - 24.0).max(40.0));
        let _ = text2;
    }

    /// Records the next key combination while a row is in edit mode (Esc / Tab leave the mode).
    fn sc_capture(&mut self, ctx: &egui::Context, now: f64) {
        let reg = registry();
        if self.sc.page.edit.is_none() || self.sc.page.dlg.is_some() { if self.sc.page.edit.is_some() { Self::mark_capture(ctx); } return; }
        Self::mark_capture(ctx);
        // A digit key records the physical digit: Alt+Shift+1 is "!" on DE/CH.
        let evs: Vec<(Key, Modifiers)> = ctx.input(|i| i.events.iter().filter_map(|e| match e {
            egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } =>
                Some((physical_key.filter(|k| matches!(k, Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9)).unwrap_or(*key), *modifiers)),
            _ => None }).collect());
        let Some(&(key, mods)) = evs.first() else { return };
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. })));
        if matches!(key, Key::Escape | Key::Tab) { self.sc.page.edit = None; return; }
        let combo = Combo::from_event(key, &mods);
        if !combo.capturable() { return; }
        let idx = self.sc.page.edit.as_ref().map_or(0, |e| e.idx);
        match find_conflict(&reg.cmds, &self.sc.res, idx, combo) {
            None => { self.sc_assign(idx, Some(combo), None); self.sc.page.edit = None; self.sc.page.applied = Some((idx, now)); }
            Some(c) => { if let Some(e) = &mut self.sc.page.edit { e.bad = Some(combo); } self.sc.page.dlg = Some(Dlg::Reassign { idx, combo, c }); }
        }
    }
    fn mark_capture(ctx: &egui::Context) { ctx.data_mut(|d| d.insert_temp(egui::Id::new("sc-capture"), ctx.cumulative_pass_nr())); }

    fn sc_page_dialogs(&mut self, ctx: &egui::Context, t: &Th, now: f64) {
        let reg = registry();
        let Some(d) = self.sc.page.dlg.take() else { return };
        let mut keep = false;
        match &d {
            Dlg::Reassign { idx, combo, c } | Dlg::Reset { idx, combo, c } => {
                let (idx, combo, c) = (*idx, *combo, *c);
                let resetting = matches!(d, Dlg::Reset { .. });
                let msg = conflict_message(&reg.cmds, &c, combo, resetting);
                let buttons = if c.fate == shortcuts::Fate::Locked { vec![Button::primary("cancel", "OK")] } else { vec![Button::secondary("cancel", "Abbrechen"), Button::primary("ok", "Fortfahren")] };
                match webex_dialog(ctx, t, "sc-conflict", CONFLICT_TITLE, &msg, None, &buttons).as_deref() {
                    Some("ok") => {
                        if resetting { self.sc_reset(idx, Some(c)); } else { self.sc_assign(idx, Some(combo), Some(c)); self.sc.page.applied = Some((idx, now)); }
                        self.sc.page.edit = None;
                    }
                    Some(_) => self.sc.page.edit = None,
                    None => keep = true,
                }
            }
            Dlg::ResetAll => match webex_dialog(ctx, t, "sc-reset-all", "Alle Tastenkombinationen auf die standardmäßigen Werte zurücksetzen?",
                "Ersetzen Sie alle Ihre angepassten Tastenkombinationen durch die Standardtastenkombinationen.", None,
                &[Button::secondary("cancel", "Abbrechen"), Button::primary("ok", "Zurücksetzen")]).as_deref() {
                Some("ok") => { self.sc_reset_all(); self.toast("Tastenkombinationen werden auf die Standardeinstellungen zurückgesetzt.", now); }
                Some(_) => {}
                None => keep = true,
            },
        }
        if keep { self.sc.page.dlg = Some(d); }
    }
}

/// Key label: 28 px high, radius 8, 1 px border, inner area inset by 4 (radius 4), colours = `shortcutinput-*` tokens.
fn paint_pill(ui: &egui::Ui, r: Rect, look: Look, light: bool, hovered: bool, label: &str, text: Color32, badge: Option<&'static str>, icons: &mut icons::Icons) {
    let base = if light { Color32::BLACK } else { Color32::WHITE };
    let (border, bg, inner, fg) = match look {
        Look::Inactive => (alpha(base, 0.5), if hovered { alpha(base, 0.07) } else { Color32::TRANSPARENT }, if hovered { Color32::TRANSPARENT } else { alpha(base, 0.11) }, alpha(base, 0.95)),
        Look::Reassign => (alpha(base, 0.9), alpha(base, 0.11), if light { Color32::WHITE } else { Color32::BLACK }, alpha(base, 0.4)),
        Look::Available => (if light { hex(0x185E46) } else { hex(0x3CC29A) }, alpha(base, 0.11), alpha(base, 0.3), alpha(base, 0.95)),
        Look::Unavailable => (if light { hex(0xAB0A15) } else { hex(0xFC8B98) }, alpha(base, 0.11), alpha(base, 0.3), alpha(base, 0.95)),
    };
    let p = ui.painter();
    p.rect_filled(r, 8.0, bg);
    p.rect_stroke(r, 8.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
    p.rect_filled(r.shrink(4.0), 4.0, inner);
    let _ = text;
    let mut x = r.left() + 8.0;
    if let Some(ic) = badge { icons.paint(ui, ic, Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), Vec2::splat(14.0)), border); x += 20.0; }
    if !label.is_empty() { p.text(pos2(x, r.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(14.0), fg); }
}
