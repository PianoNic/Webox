//! Settings > Plugins > "Verfügbar": the plugins of every repository (official first), each with version, authors, description,
//! repository and compatibility (an incompatible one is drawn greyed with the reason), and Installieren / Aktualisieren / a download
//! progress bar. Below: the repository list (official + added, add / remove like Aniyomi's extension repos) and the daily check.
use super::{kit, PUi};
use crate::extras::plugin_repo::index;
use crate::*;

impl App {
    pub(super) fn available_tab(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut PUi, now: f64) {
        let blocked = crate::system::side_loading_protection();
        let loading = self.repos.states.iter().any(|r| r.loading);
        kit::section(ui, t, "Verfügbare Plugins", "Plugins aus den eingetragenen Repositories. Die Liste wird beim Öffnen dieser Seite aktualisiert.", true, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if kit::pill(ui, t, "Aktualisieren", true, !loading && !self.repos.sample).clicked() { self.repos.refresh(); }
                if loading { ui.add_space(8.0); ui.spinner(); ui.label(RichText::new("Wird geladen …").size(kit::SECOND).color(t.text2)); }
            });
            if blocked {
                ui.add_space(4.0);
                kit::text_block(ui, &format!("Installieren ist ausgeschaltet: {}.", crate::extras::plugins::policy_block_reason()), kit::SECOND, t.tk("listitem-textmissed"), 0.0);
            }
        });
        let mut list = self.repos.available();
        list.sort_by(|a, b| b.official.cmp(&a.official).then(a.compat.is_err().cmp(&b.compat.is_err())).then(a.plugin.name.to_lowercase().cmp(&b.plugin.name.to_lowercase())));
        if list.is_empty() {
            kit::section(ui, t, "", "", false, |ui| { kit::text_block(ui, if loading { "Repositories werden geladen …" } else { "Keine Plugins gefunden. Prüfen Sie die Repositories unten." }, kit::BODY, t.text2, 0.0); });
        }
        let k = kit::tok(t);
        let mut clicked = None;
        for a in &list {
            let p = &a.plugin;
            let have = self.plugins.installed(&p.pkg).map(|i| i.version.clone());
            let job = self.repos.job(&p.pkg).map(|j| (j.version.clone(), j.fraction()));
            ui.add_space(12.0);
            egui::Frame::default().stroke(Stroke::new(1.0_f32, k.border)).corner_radius(8.0).inner_margin(12).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add_enabled_ui(a.compat.is_ok(), |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if let Some((v, f)) = &job {
                            crate::ui::theme::decor::progress(ui, *f, 120.0);
                            ui.label(RichText::new(format!("v{v}")).size(kit::SECOND).color(t.text2));
                        } else if a.compat.is_ok() {
                            let (label, enabled) = match &have {
                                Some(h) if index::newer(&p.version, h) => ("Aktualisieren", true),
                                Some(_) => ("Installiert", false),
                                None => ("Installieren", true),
                            };
                            if kit::pill(ui, t, label, true, enabled && !blocked && !self.repos.sample).clicked() { clicked = Some(a.clone()); }
                        }
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&p.name).font(semibold(15.0)).color(t.text1));
                                let authors = if p.authors.is_empty() { String::new() } else { format!(" · {}", p.authors.join(", ")) };
                                ui.label(RichText::new(format!("v{}{authors}", p.version)).size(kit::SECOND).color(t.text2));
                                if let Some(h) = &have { ui.label(RichText::new(format!("(installiert: v{h})")).size(kit::SECOND).color(t.text2)); }
                            });
                            if !p.description.is_empty() { kit::text_block(ui, &p.description, kit::BODY, t.text2, 0.0); }
                            let origin = if a.official { format!("{} (offiziell)", a.repo_title) } else { format!("{} (nicht offiziell)", a.repo_title) };
                            ui.label(RichText::new(format!("{origin} · Schnittstelle {}", p.contract)).size(kit::SECOND).color(t.text2));
                            if let Err(why) = &a.compat { kit::text_block(ui, &format!("Nicht kompatibel: {why}"), kit::SECOND, t.text2, 0.0); }
                        });
                    });
                });
            });
        }
        if let Some(a) = clicked { self.request_install(s, a, now); }
        self.repos_section(ui, t, s);
    }

    fn repos_section(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut PUi) {
        let k = kit::tok(t);
        let mut remove = None;
        if self.shot.is_some() && std::env::var("WEBOX_SHOT_PLUGIN_TAB").is_ok_and(|v| v == "repos") { ui.scroll_to_cursor(Some(egui::Align::Min)); }
        kit::section(ui, t, "Repositories", "Ein Repository ist eine Index-Datei (index.min.json) mit Plugins und ihren Prüfsummen. Fremde Repositories können beliebigen nativen Code ausliefern: Fügen Sie nur Repositories hinzu, denen Sie vertrauen.", false, |ui| {
            for st in self.repos.states.clone() {
                ui.add_space(8.0);
                // Each row is as high as its three lines. The badge / remove button sits in a child at the row's top right: a
                // right-to-left layout with centred cross alignment would take the whole remaining page height.
                egui::Frame::default().stroke(Stroke::new(1.0_f32, k.border)).corner_radius(8.0).inner_margin(10).show(ui, |ui| {
                    let w = ui.available_width();
                    ui.set_width(w);
                    let top = ui.cursor().min;
                    ui.vertical(|ui| {
                        ui.set_width((w - 110.0).max(80.0));
                        ui.label(RichText::new(st.title()).font(semibold(14.0)).color(t.text1));
                        kit::text_block(ui, &st.url, kit::SECOND, t.text2, 0.0);
                        let col = if st.error.is_some() { t.tk("listitem-textmissed") } else { t.text2 };
                        ui.label(RichText::new(repo_status(&st)).size(kit::SECOND).color(col));
                    });
                    let slot = Rect::from_min_size(pos2(top.x + w - 104.0, top.y), Vec2::new(104.0, 24.0));
                    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(slot).layout(egui::Layout::right_to_left(egui::Align::Min)));
                    if self.repos.is_official(&st.url) { c.label(RichText::new("Offiziell").size(kit::SECOND).color(t.text2)); }
                    else if kit::pill(&mut c, t, "Entfernen", false, !self.repos.sample).clicked() { remove = Some(st.url.clone()); }
                });
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 140.0).max(160.0);
                let r = kit::text_field(ui, t, &mut s.add_url, "https://…/index.min.json", w);
                if r.changed() { s.add_error = None; }
                ui.add_space(8.0);
                let go = kit::pill(ui, t, "Hinzufügen", true, !s.add_url.trim().is_empty() && !self.repos.sample).clicked()
                    || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !s.add_url.trim().is_empty());
                if go && !self.repos.sample {
                    // checked first, then the trust warning (warn.rs), then added
                    match self.repos.check_repo_url(&s.add_url) { Ok(u) => { s.confirm_repo = Some(u); s.add_error = None; } Err(e) => s.add_error = Some(e) }
                }
            });
            if let Some(e) = &s.add_error { ui.add_space(4.0); kit::text_block(ui, e, kit::SECOND, t.tk("listitem-textmissed"), 0.0); }
        });
        if let Some(u) = remove { self.repos.remove_repo(&u); }
        kit::section(ui, t, "Updates", "", false, |ui| {
            let mut on = self.repos.cfg.auto_update;
            if kit::check(ui, &mut self.icons, t, "Automatisch nach Updates suchen", &mut on, true, false) { self.repos.set_auto_update(on); }
            kit::check_desc(ui, t, "Einmal am Tag, solange Plugins aus Repositories installiert sind. Updates werden angezeigt, nicht ohne Nachfrage installiert.");
        });
    }
}

/// The status line of a repository row.
fn repo_status(st: &crate::extras::plugin_repo::RepoState) -> String {
    if st.loading { return "Wird geladen …".into(); }
    if let Some(e) = &st.error {
        // "Server antwortet mit HTTP 404" (net.rs) reads better as what it means for the person
        let e = match e.strip_prefix("Server antwortet mit HTTP ") { Some(code) => format!("Repository nicht erreichbar (HTTP {code})"), None => format!("Fehler: {e}") };
        return format!("{e}{}", if st.index.is_some() { " - zeige die zuletzt geladene Liste" } else { "" });
    }
    match &st.index {
        Some(i) => format!("{} Plugin{}{}", i.plugins.len(), if i.plugins.len() == 1 { "" } else { "s" },
            if i.skipped > 0 { format!(", {} ungültige Einträge übersprungen", i.skipped) } else { String::new() }),
        None => "Noch nicht geladen".into(),
    }
}

#[cfg(test)]
mod tests {
    use crate::extras::plugin_repo::RepoState;
    #[test]
    fn friendly_repo_status() {
        let st = RepoState { url: "https://example.com/i.json".into(), error: Some("Server antwortet mit HTTP 404".into()), ..Default::default() };
        assert_eq!(super::repo_status(&st), "Repository nicht erreichbar (HTTP 404)");
        let st = RepoState { error: Some("Verbindung fehlgeschlagen: Dns".into()), ..Default::default() };
        assert_eq!(super::repo_status(&st), "Fehler: Verbindung fehlgeschlagen: Dns");
    }
}
