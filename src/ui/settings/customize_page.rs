//! Settings > Oberfläche anpassen: one section per area of the window (ui/customize.rs). Each element is a row with a check
//! box, its icon and label; in the rail and the composer toolbar two arrows move it up / down. "Zurücksetzen" under each
//! section brings back the default look of that area.
use super::kit::{self, section};
use crate::ui::customize::{self as cz, El, Group};
use crate::*;

/// What a row asked for (applied after drawing).
enum Do { Show(&'static str, bool), Move(usize, bool), Reset }

const ROW_H: f32 = 32.0;
const ARROW: f32 = 24.0;

impl App {
    pub(super) fn page_customize(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        kit::text_block(ui, "Blenden Sie Elemente aus, die Sie nicht brauchen, und ordnen Sie die App-Seitenleiste und die Symbolleiste des Nachrichtenfelds neu an. Ausgeblendete Funktionen bleiben über ihre Tastenkombinationen und Menüs erreichbar.", kit::SECOND, t.text2, 0.0);
        for (i, g) in cz::GROUPS.into_iter().enumerate() {
            let mut todo = None;
            section(ui, t, g.title, g.desc, false, |ui| {
                if i == 0 { kit::text_block(ui, "„Einstellungen“ und „Weitere“ können nicht beide ausgeblendet werden, damit die Einstellungen erreichbar bleiben.", kit::SECOND, t.text2, 0.0); ui.add_space(4.0); }
                let (o, n) = cz::order(&self.settings, g);
                for pos in 0..n { if let Some(d) = self.customize_row(ui, t, g, &g.els[o[pos] as usize], g.orderable.then_some((pos, n))) { todo = Some(d); } }
                for e in g.els.iter().filter(|e| e.fixed) { if let Some(d) = self.customize_row(ui, t, g, e, None) { todo = Some(d); } }
                ui.add_space(8.0);
                if kit::pill(ui, t, "Zurücksetzen", false, !cz::is_default(&self.settings, g)).clicked() { todo = Some(Do::Reset); }
            });
            if let Some(d) = todo { self.customize_apply(g, d, now); }
        }
    }

    /// One element row: check box, icon, label, and (`order` = position, count) the up / down arrows.
    fn customize_row(&mut self, ui: &mut egui::Ui, t: &Th, g: &'static Group, e: &'static El, order: Option<(usize, usize)>) -> Option<Do> {
        let mut out = None;
        let mut on = cz::shown(&self.settings, g, e.id);
        let enabled = !on || cz::can_hide(&self.settings, g, e.id);
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
        let arrows_w = if g.orderable { 2.0 * ARROW + 8.0 } else { 0.0 };
        // Check box (no label of its own), then the element's icon and label; a click on the label toggles too.
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(row.min + Vec2::new(0.0, (ROW_H - 24.0) / 2.0), Vec2::new(24.0, 24.0))));
        if kit::check(&mut c, &mut self.icons, t, "", &mut on, enabled, false) { out = Some(Do::Show(e.id, on)); }
        let ic = Rect::from_center_size(pos2(row.left() + 36.0, row.center().y), Vec2::splat(16.0));
        let ink = if enabled { t.text1 } else { t.text2 };
        if !e.icon.is_empty() { self.icons.paint(ui, e.icon, ic, ink); }
        let lr = Rect::from_min_max(pos2(row.left() + 28.0, row.top()), pos2(row.right() - arrows_w, row.bottom()));
        let lresp = ui.interact(lr, egui::Id::new(("cz-label", g.key, e.id)), if enabled { Sense::click() } else { Sense::hover() });
        crate::ui::chrome::fit_text(ui.painter(), row.left() + 52.0, row.center().y, e.label, FontId::proportional(kit::BODY), ink, lr.right() - row.left() - 56.0);
        if enabled && lresp.clicked() { out = Some(Do::Show(e.id, !on)); }
        if !enabled { lresp.on_hover_text("Blenden Sie zuerst „Weitere“ bzw. „Einstellungen“ wieder ein."); }
        if let Some((pos, n)) = order {
            for (k, (icon, up, tip)) in [("arrow-up-regular", true, "Nach oben"), ("arrow-down-regular", false, "Nach unten")].into_iter().enumerate() {
                let can = if up { pos > 0 } else { pos + 1 < n };
                let r = Rect::from_min_size(pos2(row.right() - 2.0 * ARROW - 4.0 + k as f32 * (ARROW + 4.0), row.center().y - ARROW / 2.0), Vec2::splat(ARROW));
                let resp = ui.interact(r, egui::Id::new(("cz-move", g.key, e.id, up)), if can { Sense::click() } else { Sense::hover() });
                if can && resp.hovered() { ui.painter().circle_filled(r.center(), ARROW / 2.0, kit::tok(t).btn_hover); }
                self.icons.paint(ui, icon, Rect::from_center_size(r.center(), Vec2::splat(14.0)), if can { t.text1 } else { alpha(t.text2, 0.4) });
                if can && resp.on_hover_text(format!("{tip}: {}", e.label)).clicked() { out = Some(Do::Move(pos, up)); }
            }
        }
        out
    }

    fn customize_apply(&mut self, g: &'static Group, d: Do, now: f64) {
        match d {
            Do::Show(id, show) => { if !cz::set_shown(&mut self.settings, g, id, show) { self.toast("„Einstellungen“ und „Weitere“ können nicht beide ausgeblendet werden.", now); return; } }
            Do::Move(pos, up) => cz::move_el(&mut self.settings, g, pos, up),
            Do::Reset => cz::reset(&mut self.settings, g),
        }
        if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); } // --screenshot runs never write the user's file
    }
}
