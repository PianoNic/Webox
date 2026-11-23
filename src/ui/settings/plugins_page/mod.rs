//! Settings > Plugins (Webox only). Two tabs:
//! - "Installiert": the loaded native plugins (src/extras/plugins.rs, plugin-api/) with name, version, author, description, their
//!   slash commands and an on / off switch that is saved (plugins.json) and takes effect at once. Plugins installed from a repository
//!   also get "Aktualisieren" (a newer compatible version in a repository) and "Deinstallieren". Design plugins (a plugin that
//!   supplies a palette) paint over the selected theme's colours; the page says so next to them.
//! - "Verfügbar" (available.rs): the plugins of all repositories with an install button, and the repository list.
//! Installing shows the native-code warning before the first install and before every install from a repository that is not the
//! official one (warn.rs). Drawn with the settings kit like the other pages.
//! Screenshot env: `WEBOX_SHOT_SETTINGS=plugins`, `WEBOX_SHOT_PLUGIN_TAB=available` (`repos`: scrolled to the repository list).
mod available;
mod detail;
mod form;
mod warn;

use super::kit;
use crate::extras::plugin_repo::{Avail, Event};
use crate::*;

#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum PTab { #[default] Installed, Available }

/// Page state between frames (egui temp data).
#[derive(Clone, Default)]
pub(super) struct PUi {
    pub tab: PTab, pub add_url: String, pub add_error: Option<String>, pub confirm: Option<Avail>, pub shot_done: bool,
    /// A checked repository URL waiting for the trust warning (warn.rs) before it is added.
    pub confirm_repo: Option<String>,
    /// The plugin (key) whose own page is open.
    pub detail: Option<String>,
    /// Text being typed into a setting ("<plugin key>|<setting id>" -> text), and why a value was refused.
    pub edits: std::collections::HashMap<String, String>, pub errors: std::collections::HashMap<String, String>,
}

/// Temp-data id the settings search uses to open a plugin's page (value: the plugin key).
pub(crate) const GOTO: &str = "plugins-goto";

fn pui(ctx: &egui::Context) -> PUi { ctx.data(|d| d.get_temp(egui::Id::new("plugins-ui"))).unwrap_or_default() }
fn set_pui(ctx: &egui::Context, s: PUi) { ctx.data_mut(|d| d.insert_temp(egui::Id::new("plugins-ui"), s)) }

impl App {
    /// Per frame (settings_window): finished index fetches and downloads, the daily update check.
    pub(crate) fn repo_tick(&mut self, ctx: &egui::Context, now: f64) {
        let installed: Vec<(String, String, String)> = self.plugins.list.iter().filter_map(|e| e.source.as_ref())
            .map(|s| (s.pkg.clone(), s.version.clone(), s.repo.clone())).collect();
        for ev in self.repos.tick(&installed) {
            match ev {
                Event::Downloaded(inst, staged) => {
                    let (name, version) = (inst.name.clone(), inst.version.clone());
                    match self.plugins.install(inst, &staged) {
                        Ok(()) => { self.applied = None; self.toast(format!("{name} {version} installiert"), now); }
                        Err(e) => self.toast(format!("{name}: {e}"), now),
                    }
                }
                Event::Failed(pkg, e) => self.toast(format!("Download von {pkg} fehlgeschlagen: {e}"), now),
                Event::UpdatesFound(n) => self.toast(if n == 1 { "1 Plugin-Update verfügbar (Einstellungen > Plugins)".to_string() }
                    else { format!("{n} Plugin-Updates verfügbar (Einstellungen > Plugins)") }, now),
            }
        }
        if self.repos.busy() { ctx.request_repaint_after(std::time::Duration::from_millis(100)); }
    }

    /// Starts an install or update; the warning dialog comes first when it is due.
    pub(super) fn request_install(&mut self, s: &mut PUi, a: Avail, now: f64) {
        let from_other = self.plugins.installed(&a.plugin.pkg).is_some_and(|i| i.repo != a.repo);
        let is_update = self.plugins.installed(&a.plugin.pkg).is_some();
        if !self.repos.cfg.warned || (!a.official && (!is_update || from_other)) { s.confirm = Some(a); return; }
        self.start_install(a, now);
    }
    pub(super) fn start_install(&mut self, a: Avail, now: f64) {
        if let Err(e) = self.repos.start_install(&a) { self.toast(format!("{}: {e}", a.plugin.name), now); }
    }

    pub(super) fn page_plugins(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        let mut s = pui(ui.ctx());
        if self.shot.is_some() && !s.shot_done {
            s.shot_done = true;
            if std::env::var("WEBOX_SHOT_PLUGIN_TAB").is_ok_and(|v| v == "available" || v == "repos") { s.tab = PTab::Available; }
            if let Ok(k) = std::env::var("WEBOX_SHOT_PLUGIN_DETAIL") { s.detail = Some(k); }
            if std::env::var_os("WEBOX_SHOT_PLUGIN_WARN").is_some() {
                s.tab = PTab::Available;
                s.confirm = self.repos.available().into_iter().find(|a| !a.official && a.compat.is_ok());
            }
        }
        if let Some(k) = ui.ctx().data_mut(|d| d.remove_temp::<String>(egui::Id::new(GOTO))) { s.detail = Some(k); }
        if let Some(i) = s.detail.as_ref().and_then(|k| self.plugins.list.iter().position(|e| &e.key == k)) {
            self.plugin_detail(ui, t, &mut s, i, now);
            self.install_warning(ui.ctx(), t, &mut s, now);
        self.repo_warning(ui.ctx(), t, &mut s);
            set_pui(ui.ctx(), s);
            return;
        }
        s.detail = None;
        self.repos.page_shown(now);
        let updates = self.plugins.list.iter().filter_map(|e| e.source.as_ref()).filter(|i| self.repos.update_for(&i.pkg, &i.version, &i.repo).is_some()).count();
        let n_avail = self.repos.available().len();
        let inst_label = if updates > 0 { format!("Installiert ({}) · {updates} Update{}", self.plugins.list.len(), if updates == 1 { "" } else { "s" }) }
            else { format!("Installiert ({})", self.plugins.list.len()) };
        tabs(ui, t, &mut s.tab, &[(PTab::Installed, inst_label), (PTab::Available, format!("Verfügbar ({n_avail})"))]);
        ui.add_space(8.0);
        match s.tab {
            PTab::Installed => self.installed_tab(ui, t, &mut s, now),
            PTab::Available => self.available_tab(ui, t, &mut s, now),
        }
        self.install_warning(ui.ctx(), t, &mut s, now);
        self.repo_warning(ui.ctx(), t, &mut s);
        set_pui(ui.ctx(), s);
    }

    fn installed_tab(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut PUi, now: f64) {
        let dir = self.plugins.dirs[0].clone();
        kit::section(ui, t, "Installierte Plugins", &format!("Native Plugins (.{}) aus Repositories und aus dem Plugin-Ordner. Aktivierte Plugins laufen sofort; die Auswahl wird gespeichert.", std::env::consts::DLL_EXTENSION), true, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if kit::pill(ui, t, "Neu laden", true, true).clicked() { self.plugins.rescan(); self.applied = None; self.toast("Plugins neu geladen", now); }
                ui.add_space(8.0);
                if kit::pill(ui, t, "Plugin-Ordner öffnen", true, true).clicked() { system::open_folder(&dir); }
            });
            // No folder path on this page (it holds the account's user name): the button opens the folder.
        });
        if self.plugins.list.is_empty() {
            kit::section(ui, t, "", "", false, |ui| { kit::text_block(ui, "Keine Plugins installiert. Unter „Verfügbar“ finden Sie Plugins aus den Repositories, oder legen Sie eine Plugin-Datei in den Plugin-Ordner und laden Sie neu.", kit::BODY, t.text2, 0.0); });
        }
        let k = kit::tok(t);
        let mut action: Option<(usize, Act2)> = None;
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(std::path::Path::to_path_buf));
        for i in 0..self.plugins.list.len() {
            let design = self.plugins.is_design(i);
            let e = &self.plugins.list[i];
            let src = e.source.clone();
            let shipped = exe_dir.as_ref().is_some_and(|x| e.file.starts_with(x));
            let update = src.as_ref().and_then(|i| self.repos.update_for(&i.pkg, &i.version, &i.repo));
            let job = src.as_ref().and_then(|i| self.repos.job(&i.pkg)).map(|j| (j.version.clone(), j.fraction()));
            let repo_title = src.as_ref().map(|i| self.repos.states.iter().find(|s| s.url == i.repo).map_or_else(|| i.repo.clone(), |s| s.title()));
            let version = src.as_ref().map_or_else(|| e.version.clone(), |i| i.version.clone());
            let (name, meta, desc, cmds, err, mut on, builtin) = (e.name.clone(), meta_line(&version, &e.author), e.description.clone(),
                e.commands.iter().map(|c| format!("/{c}")).collect::<Vec<_>>().join("  "), e.error.clone(), e.enabled, e.builtin);
            let has_settings = e.settings.schema.is_some();
            ui.add_space(12.0);
            egui::Frame::default().stroke(Stroke::new(1.0_f32, k.border)).corner_radius(8.0).inner_margin(12).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        let r = ui.add_enabled(err.is_none(), toggle(&mut on, t)).on_hover_text(if on { "Deaktivieren" } else { "Aktivieren" });
                        if r.changed() { action = Some((i, Act2::Enable(on))); }
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&name).font(semibold(15.0)).color(t.text1));
                                if builtin { badge(ui, t, "Built-in"); }
                                if !meta.is_empty() { ui.label(RichText::new(&meta).size(kit::SECOND).color(t.text2)); }
                            });
                            if !desc.is_empty() { kit::text_block(ui, &desc, kit::BODY, t.text2, 0.0); }
                            if !cmds.is_empty() { ui.label(RichText::new(&cmds).size(kit::SECOND).color(t.link)); }
                            if design { ui.add_space(2.0); kit::text_block(ui, "Design-Plugin: überschreibt die Farben des ausgewählten Themas (Erscheinungsbild).", kit::SECOND, t.text2, 0.0); }
                            let origin = match &repo_title {
                                Some(r) => format!("Aus Repository: {r}"),
                                None if builtin => "In Webox integriert".to_string(),
                                None if shipped => "Mitgeliefert (Programmordner)".to_string(),
                                None => "Manuell installiert (Plugin-Ordner)".to_string(),
                            };
                            ui.label(RichText::new(origin).size(kit::SECOND).color(t.text2));
                            if let Some(x) = &err { kit::text_block(ui, &format!("Kann nicht geladen werden: {x}"), kit::SECOND, t.tk("listitem-textmissed"), 0.0); }
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if kit::pill(ui, t, if has_settings { "Einstellungen" } else { "Details" }, true, true).clicked() { action = Some((i, Act2::Open)); }
                                ui.add_space(8.0);
                                if src.is_some() {
                                    if let Some((v, f)) = &job {
                                        ui.label(RichText::new(format!("Lädt v{v} …")).size(kit::SECOND).color(t.text2));
                                        crate::ui::theme::decor::progress(ui, *f, 140.0);
                                    } else if let Some(u) = &update {
                                        if kit::pill(ui, t, &format!("Aktualisieren auf v{}", u.plugin.version), true, true).clicked() { action = Some((i, Act2::Update(u.clone()))); }
                                        ui.add_space(8.0);
                                    }
                                    if job.is_none() && kit::pill(ui, t, "Deinstallieren", true, true).clicked() { action = Some((i, Act2::Uninstall)); }
                                }
                            });
                        });
                    });
                });
            });
        }
        match action {
            Some((i, Act2::Enable(on))) => { self.plugins.set_enabled(i, on); self.applied = None; }
            Some((_, Act2::Update(a))) => self.request_install(s, a, now),
            Some((i, Act2::Open)) => s.detail = Some(self.plugins.list[i].key.clone()),
            Some((i, Act2::Uninstall)) => {
                let (pkg, name) = (self.plugins.list[i].source.as_ref().map(|s| s.pkg.clone()).unwrap_or_default(), self.plugins.list[i].name.clone());
                match self.plugins.uninstall(&pkg) { Ok(()) => { self.applied = None; self.toast(format!("{name} deinstalliert"), now) } Err(e) => self.toast(format!("{name}: {e}"), now) }
            }
            None => {}
        }
        if !self.plugin_log.is_empty() {
            kit::section(ui, t, "Plugin-Protokoll", "", false, |ui| {
                for l in self.plugin_log.iter().rev().take(20) { ui.label(RichText::new(l).color(t.text2).monospace().size(11.0)); }
            });
        }
    }
}

enum Act2 { Enable(bool), Update(Avail), Uninstall, Open }

/// Tab headers: semibold label, the selected one underlined in the accent colour.
fn tabs(ui: &mut egui::Ui, t: &Th, sel: &mut PTab, items: &[(PTab, String)]) {
    let k = kit::tok(t);
    ui.horizontal(|ui| {
        for (tab, label) in items {
            let on = *sel == *tab;
            let g = ui.painter().layout_no_wrap(label.clone(), semibold(kit::BODY), if on { t.text1 } else { t.text2 });
            let (r, resp) = ui.allocate_exact_size(Vec2::new(g.size().x + 24.0, 36.0), Sense::click());
            if resp.hovered() && !on { ui.painter().rect_filled(r, 6.0, k.item_hover); }
            ui.painter().galley(pos2(r.left() + 12.0, r.center().y - g.size().y / 2.0), g, t.text1);
            if on { ui.painter().rect_filled(Rect::from_min_max(pos2(r.left() + 8.0, r.bottom() - 3.0), pos2(r.right() - 8.0, r.bottom())), 1.5, t.link); }
            if resp.clicked() { *sel = *tab; }
        }
    });
    let r = ui.min_rect();
    ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0_f32, k.border));
}

/// A small rounded label after a plugin's name (e.g. "Built-in" for the plugins compiled into the app).
fn badge(ui: &mut egui::Ui, t: &Th, text: &str) {
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(11.0), t.text2);
    let (r, _) = ui.allocate_exact_size(g.size() + Vec2::new(12.0, 4.0), Sense::hover());
    ui.painter().rect_stroke(r, r.height() / 2.0, Stroke::new(1.0_f32, t.text2.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    ui.painter().galley(r.center() - g.size() / 2.0, g, t.text2);
}

fn meta_line(version: &str, author: &str) -> String {
    match (version.is_empty(), author.is_empty()) {
        (false, false) => format!("v{version} · {author}"),
        (false, true) => format!("v{version}"),
        (true, false) => author.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn meta_line_skips_missing_parts() {
        assert_eq!(super::meta_line("1.2", "Webox"), "v1.2 · Webox");
        assert_eq!(super::meta_line("", "Webox"), "Webox");
        assert_eq!(super::meta_line("", ""), "");
    }
}
