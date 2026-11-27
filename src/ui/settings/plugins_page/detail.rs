//! A plugin's own page under Settings > Plugins (opened from its card or from the settings search): header with name, version,
//! author, description and where it came from, the on / off switch, Update / Uninstall for repository plugins, then the settings
//! form generated from the plugin's schema (form.rs). Built-in and installed plugins get the same page.
use super::{badge, kit, meta_line, PUi};
use crate::*;

impl App {
    pub(super) fn plugin_detail(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut PUi, i: usize, now: f64) {
        if kit::link(ui, t, tr("‹ All plugins")).clicked() { s.detail = None; return; }
        ui.add_space(12.0);
        let design = self.plugins.is_design(i);
        let e = &self.plugins.list[i];
        let src = e.source.clone();
        let update = src.as_ref().and_then(|x| self.repos.update_for(&x.pkg, &x.version, &x.repo));
        let job = src.as_ref().and_then(|x| self.repos.job(&x.pkg)).map(|j| (j.version.clone(), j.fraction()));
        let version = src.as_ref().map_or_else(|| e.version.clone(), |x| x.version.clone());
        let origin = match &src {
            Some(x) => crate::trq!("From repository: %1", self.repos.states.iter().find(|r| r.url == x.repo).map_or_else(|| x.repo.clone(), |r| r.title())),
            None if e.builtin => tr("Built into Webox").to_string(),
            // never the file's path (it holds the account's user name)
            None if std::env::current_exe().ok().and_then(|x| x.parent().map(|d| e.file.starts_with(d))).unwrap_or(false) => "Mitgeliefert (Programmordner)".to_string(),
            None => tr("Installed manually (plugin folder)").to_string(),
        };
        let (name, meta, desc, err, builtin, mut on) = (e.name.clone(), meta_line(&version, &e.author), e.description.clone(), e.error.clone(), e.builtin, e.enabled);
        let cmds = e.commands.iter().map(|c| format!("/{c}")).collect::<Vec<_>>().join("  ");
        let (schema_err, has_settings) = (e.settings.error.clone(), e.settings.schema.is_some());
        let mut toggled = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                toggled = ui.add_enabled(err.is_none(), toggle(&mut on, t)).on_hover_text(if on { tr("Turn off") } else { tr("Turn on") }).changed();
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&name).font(F::Banner.semibold()).color(t.text1));
                        if builtin { badge(ui, t, "Built-in"); }
                    });
                    if !meta.is_empty() { ui.label(RichText::new(&meta).size(kit::SECOND).color(t.text2)); }
                });
            });
        });
        if toggled { self.plugins.set_enabled(i, on); self.applied = None; }
        ui.add_space(6.0);
        if !desc.is_empty() { kit::text_block(ui, &desc, kit::BODY, t.text1, 0.0); ui.add_space(4.0); }
        if !cmds.is_empty() { ui.label(RichText::new(&cmds).size(kit::SECOND).color(t.link)); }
        if design { kit::text_block(ui, tr("Design plugin: overrides the colors of the chosen theme (Appearance)."), kit::SECOND, t.text2, 0.0); }
        kit::text_block(ui, &origin, kit::SECOND, t.text2, 0.0);
        if let Some(x) = &err { kit::text_block(ui, &crate::trq!("Can't be loaded: %1", x), kit::SECOND, t.at(tok!("listitem-textmissed")), 0.0); }
        if src.is_some() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if let Some((v, f)) = &job {
                    ui.label(RichText::new(crate::trq!("Loading v%1 …", v)).size(kit::SECOND).color(t.text2));
                    crate::ui::theme::decor::progress(ui, *f, 140.0);
                    return;
                }
                if let Some(u) = &update {
                    if kit::pill(ui, t, &crate::trq!("Update to v%1", u.plugin.version), true, true).clicked() { self.request_install(s, u.clone(), now); }
                    ui.add_space(8.0);
                }
                if kit::pill(ui, t, "Deinstallieren", true, true).clicked() {
                    let pkg = src.as_ref().map(|x| x.pkg.clone()).unwrap_or_default();
                    match self.plugins.uninstall(&pkg) {
                        Ok(()) => { self.applied = None; s.detail = None; self.toast(crate::trq!("%1 uninstalled", name), now); }
                        Err(x) => self.toast(format!("{name}: {x}"), now),
                    }
                }
            });
            if s.detail.is_none() { return; }
        }
        if let Some(x) = &schema_err { kit::section(ui, t, tr("Settings"), "", false, |ui| { kit::text_block(ui, &crate::trq!("The settings of this plugin are broken: %1", x), kit::SECOND, t.at(tok!("listitem-textmissed")), 0.0); }); }
        else if !has_settings { kit::section(ui, t, tr("Settings"), "", false, |ui| { kit::text_block(ui, tr("This plugin has no settings."), kit::BODY, t.text2, 0.0); }); }
        else { self.plugin_form(ui, t, s, i, now); }
    }
}
