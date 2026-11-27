//! Settings > Plugins > "Available": the plugins of every repository (official first), each with version, authors, description,
//! repository and compatibility (an incompatible one is drawn greyed with the reason), and Install / Update / a download
//! progress bar. Below: the repository list (official + added, add / remove like Aniyomi's extension repos) and the daily check.
use super::{kit, PUi};
use crate::extras::plugin_repo::{index, sign::Trust};
use crate::*;

impl App {
    pub(super) fn available_tab(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut PUi, now: f64) {
        let blocked = crate::system::side_loading_protection();
        let loading = self.repos.states.iter().any(|r| r.loading);
        kit::section(ui, t, tr("Available plugins"), "", true, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if kit::pill(ui, t, tr("Refresh"), true, !loading && !self.repos.sample).clicked() { self.repos.refresh(); }
                if loading { ui.add_space(8.0); ui.spinner(); ui.label(RichText::new(tr("Loading...")).size(kit::SECOND).color(t.text2)); }
            });
            if blocked {
                ui.add_space(4.0);
                kit::text_block(ui, &crate::trq!("Installing is turned off: %1.", crate::extras::plugins::policy_block_reason()), kit::SECOND, t.tk("listitem-textmissed"), 0.0);
            }
        });
        let mut list = self.repos.available();
        list.sort_by(|a, b| b.official.cmp(&a.official).then(a.compat.is_err().cmp(&b.compat.is_err())).then(a.plugin.name.to_lowercase().cmp(&b.plugin.name.to_lowercase())));
        if list.is_empty() {
            kit::section(ui, t, "", "", false, |ui| { kit::text_block(ui, if loading { tr("Loading repositories …") } else { tr("No plugins found. Check the repositories below.") }, kit::BODY, t.text2, 0.0); });
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
                                Some(h) if index::newer(&p.version, h) => (tr("Update"), true),
                                Some(_) => (tr("Installed"), false),
                                None => (tr("Install"), true),
                            };
                            if kit::pill(ui, t, label, true, enabled && a.installable && !blocked && !self.repos.sample).clicked() { clicked = Some(a.clone()); }
                        }
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&p.name).font(semibold(15.0)).color(t.text1));
                                let authors = if p.authors.is_empty() { String::new() } else { format!(" · {}", p.authors.join(", ")) };
                                ui.label(RichText::new(format!("v{}{authors}", p.version)).size(kit::SECOND).color(t.text2));
                                // the button already says "Installiert"; the installed version only matters next to an update
                                if let Some(h) = have.as_ref().filter(|h| index::newer(&p.version, h)) { ui.label(RichText::new(crate::trq!("(installed: v%1)", h)).size(kit::SECOND).color(t.text2)); }
                            });
                            if !p.description.is_empty() { kit::text_block(ui, &p.description, kit::BODY, t.text2, 0.0); }
                            // the source line is the trust signal, so it only shows for plugins from unofficial repositories
                            if !a.official { ui.label(RichText::new(crate::trq!("%1 (not official)", a.repo_title)).size(kit::SECOND).color(t.tk("listitem-textmissed"))); }
                            if let Err(why) = &a.compat { kit::text_block(ui, &crate::trq!("Not compatible: %1", why), kit::SECOND, t.text2, 0.0); }
                            if let Some(w) = trust_warning(&a.trust, a.installable) { kit::text_block(ui, &w, kit::SECOND, t.tk("listitem-textmissed"), 0.0); }
                        });
                    });
                });
            });
        }
        if let Some(a) = clicked { self.request_install(s, a, now); }
        // Split the page into two zones: the plugin catalogue above, repository sources and the daily check below. The divider
        // keeps the two lower section headers from reading as peers of "Available plugins".
        ui.add_space(24.0);
        let sep = ui.available_rect_before_wrap();
        ui.painter().hline(sep.x_range(), sep.top(), Stroke::new(1.0_f32, k.border));
        self.repos_section(ui, t, s);
    }

    fn repos_section(&mut self, ui: &mut egui::Ui, t: &Th, s: &mut PUi) {
        let k = kit::tok(t);
        let mut remove = None;
        if self.shot.is_some() && std::env::var("WEBOX_SHOT_PLUGIN_TAB").is_ok_and(|v| v == "repos") { ui.scroll_to_cursor(Some(egui::Align::Min)); }
        kit::section(ui, t, "Repositories", tr("Only add repositories you trust: they can deliver any code."), false, |ui| {
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
                    else if kit::pill(&mut c, t, tr("Remove"), false, !self.repos.sample).clicked() { remove = Some(st.url.clone()); }
                });
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 140.0).max(160.0);
                let r = kit::text_field(ui, t, &mut s.add_url, "https://…/index.min.json", w);
                if r.changed() { s.add_error = None; }
                ui.add_space(8.0);
                let go = kit::pill(ui, t, tr("Add"), true, !s.add_url.trim().is_empty() && !self.repos.sample).clicked()
                    || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !s.add_url.trim().is_empty());
                if go && !self.repos.sample {
                    // checked first, then the trust warning (warn.rs), then added
                    match self.repos.check_repo_url(&s.add_url) { Ok(u) => { s.confirm_repo = Some(u); s.add_error = None; } Err(e) => s.add_error = Some(e) }
                }
            });
            if let Some(e) = &s.add_error { ui.add_space(4.0); kit::text_block(ui, e, kit::SECOND, t.tk("listitem-textmissed"), 0.0); }
        });
        if let Some(u) = remove { self.repos.remove_repo(&u); }
        kit::section(ui, t, tr("Updates"), "", false, |ui| {
            let mut on = self.repos.cfg.auto_update;
            if kit::check(ui, &mut self.icons, t, tr("Check for updates automatically"), &mut on, true, false) { self.repos.set_auto_update(on); }
            kit::check_desc(ui, t, tr("Once a day. Updates are only shown, not installed automatically."));
        });
        // Only once Webox checks signatures (a key is built in, sign.rs): before that nothing is blocked.
        if self.repos.signatures_checked() {
            kit::section(ui, t, tr("Signatures"), "", false, |ui| {
                let mut on = self.repos.cfg.allow_unsigned;
                if kit::check(ui, &mut self.icons, t, tr("Allow unsigned repositories"), &mut on, true, false) { self.repos.set_allow_unsigned(on); }
                kit::check_desc(ui, t, tr("Installs plugins from repositories whose list is not signed with the Webox key, or whose signature is wrong. Anyone who can change such a list can deliver any code."));
            });
        }
    }
}

/// The status line of a repository row.
fn repo_status(st: &crate::extras::plugin_repo::RepoState) -> String {
    if st.loading { return tr("Loading...").into(); }
    if let Some(e) = &st.error {
        // "The server answered with HTTP 404" (the error text of net.rs, translated) reads better as what it means for the person
        let tpl = crate::trq!("The server answered with HTTP {{0}}");
        let code = tpl.split_once("{{0}}").and_then(|(pre, post)| e.strip_prefix(pre)?.strip_suffix(post));
        let e = match code { Some(code) => crate::trq!("Repository not reachable (HTTP %1)", code), None => crate::trq!("Error: {{0}}", e) };
        return format!("{e}{}", if st.index.is_some() { tr(" - showing the last loaded list") } else { "" });
    }
    match &st.index {
        Some(i) => format!("{}{} · {}", if i.plugins.len() == 1 { tr("1 plugin").to_string() } else { crate::trq!("%1 plugins", i.plugins.len()) },
            if i.skipped > 0 { crate::trq!(", %1 invalid entries skipped", i.skipped) } else { String::new() }, trust_label(&st.trust)),
        None => tr("Not loaded yet").into(),
    }
}

/// How a repository's index signature reads in its status line.
fn trust_label(t: &Trust) -> &'static str {
    match t {
        Trust::Signed => tr("signed"),
        Trust::Unsigned => tr("not signed"),
        Trust::BadSignature => tr("signature invalid"),
        Trust::NotChecked => tr("signature not checked"),
    }
}

/// The warning under a plugin whose repository index is not signed with Webox's key (none while signatures aren't checked).
fn trust_warning(t: &Trust, installable: bool) -> Option<String> {
    let what = match t {
        Trust::Signed | Trust::NotChecked => return None,
        Trust::Unsigned => tr("Unsigned repository: Webox can't check that this list comes from its publisher."),
        Trust::BadSignature => tr("The repository's signature is invalid: the list may have been changed by someone else."),
    };
    Some(if installable { what.to_string() } else { format!("{what} {}", tr("Allow unsigned repositories below to install it anyway.")) })
}

#[cfg(test)]
mod tests {
    use crate::extras::plugin_repo::RepoState;
    #[test]
    fn friendly_repo_status() {
        let st = RepoState { url: "https://example.com/i.json".into(), error: Some(crate::trq!("The server answered with HTTP {{0}}", 404)), ..Default::default() };
        assert_eq!(super::repo_status(&st), crate::trq!("Repository not reachable (HTTP %1)", 404));
        let st = RepoState { error: Some("Connection failed: Dns".into()), ..Default::default() };
        assert_eq!(super::repo_status(&st), crate::trq!("Error: {{0}}", "Connection failed: Dns"));
    }

    #[test]
    fn unsigned_repositories_are_flagged() {
        use crate::extras::plugin_repo::{index, sign::Trust};
        let st = RepoState { index: index::parse("[]").ok(), trust: Trust::BadSignature, ..Default::default() };
        assert!(super::repo_status(&st).ends_with(&*crate::tr("signature invalid")));
        assert!(super::trust_warning(&Trust::Signed, true).is_none() && super::trust_warning(&Trust::NotChecked, true).is_none());
        let blocked = super::trust_warning(&Trust::Unsigned, false).unwrap();
        assert!(blocked.ends_with(&*crate::tr("Allow unsigned repositories below to install it anyway.")));
        assert!(!super::trust_warning(&Trust::BadSignature, true).unwrap().contains(&*crate::tr("Allow unsigned repositories below to install it anyway.")));
    }
}
