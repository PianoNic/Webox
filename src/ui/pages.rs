#![allow(clippy::too_many_arguments)]
use crate::*;

impl App {
    pub(crate) fn page(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        
        match self.tab {
            // The list pages live in page_kit.rs / calls_page.rs / meetings_page.rs / contacts_page.rs / teams_page.rs.
            Tab::Whiteboards => self.whiteboards_page(ui, t, now),
            Tab::Contacts | Tab::Calling | Tab::Meetings | Tab::Voicemail => {}
            Tab::Plugins => self.plugins_page(ui, t, now),
            Tab::Settings => self.settings_page(ui, t, now),
            Tab::Messaging | Tab::Teams => {}
        }
    }

    pub(crate) fn plugins_page(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Plugins & themes").size(21.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Reload").clicked() { self.plugins.rescan(); self.applied = None; self.toast("Plugins reloaded", now); }
                if ui.button("Open plugin folder").clicked() { system::open_folder(&self.plugins.dirs[0]) }
            });
        });
        ui.label(RichText::new(format!("Native Rust plugins (.{}) in {}", std::env::consts::DLL_EXTENSION, self.plugins.dirs[0].display())).color(t.text2).size(12.0));
        ui.add_space(12.0);
        if self.plugins.list.is_empty() { ui.label(RichText::new("No plugins found. Put a plugin file in the plugin folder and press Reload.").color(t.text2)); }
        crate::ui::widgets::vscroll().auto_shrink([false; 2]).show(ui, |ui| {
            for i in 0..self.plugins.list.len() {
                let e = &self.plugins.list[i];
                let mut on = e.enabled;
                let changed = egui::Frame::default().fill(t.selected).corner_radius(10.0).inner_margin(14).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&e.name).strong().size(15.0));
                                ui.label(RichText::new(format!("v{} by {}", e.version, e.author)).color(t.text2).size(12.0));
                            });
                            ui.label(RichText::new(&e.description).color(t.text2));
                            if !e.commands.is_empty() { ui.label(RichText::new(e.commands.iter().map(|c| format!("/{c}")).collect::<Vec<_>>().join("  ")).color(t.link).size(12.0)); }
                            if let Some(err) = &e.error { ui.label(RichText::new(format!("Can't load: {err}")).color(t.badge)); }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add_enabled(e.error.is_none(), toggle(&mut on, t)).changed())
                    }).inner.inner
                }).inner;
                if changed { self.plugins.set_enabled(i, on); self.applied = None; }
                ui.add_space(8.0);
            }
            if !self.plugin_log.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Plugin log").strong());
                for l in self.plugin_log.iter().rev().take(20) { ui.label(RichText::new(l).color(t.text2).monospace().size(11.0)); }
            }
        });
    }
}

