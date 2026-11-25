//! The warning before an install: shown before the first install at all, and before every install from a repository that is not
//! the official one. Plugins are native code that runs inside Webox with the signed-in session.
use super::{kit, PUi};
use crate::*;

pub(super) const NATIVE_CODE: &str = "Plugins are native code. They run inside Webox with the same rights as Webox itself and have full access to your account and your signed-in session: messages, spaces, contacts and credentials. Only install plugins from sources you trust.";

impl App {
    /// The warning before a repository is added: it can ship any native code (the official one is built in and never asks).
    pub(super) fn repo_warning(&mut self, ctx: &egui::Context, t: &Th, s: &mut PUi) {
        let Some(url) = s.confirm_repo.clone() else { return };
        let mut answer = None;
        let m = egui::Modal::new(egui::Id::new("plugin-repo-warning")).frame(egui::Frame::popup(&ctx.style()).fill(t.card).corner_radius(12.0).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new(tr("Add repository?")).font(semibold(18.0)).color(t.text1));
            ui.add_space(8.0);
            kit::text_block(ui, &url, kit::SECOND, t.text2, 0.0);
            ui.add_space(8.0);
            kit::text_block(ui, tr("This repository is not the official Webox repository. Whoever runs it can offer any plugins."), kit::BODY, t.tk("listitem-textmissed"), 0.0);
            ui.add_space(8.0);
            kit::text_block(ui, tr(NATIVE_CODE), kit::BODY, t.text1, 0.0);
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::pill(ui, t, tr("Add"), true, true).clicked() { answer = Some(true); }
                ui.add_space(8.0);
                if kit::pill(ui, t, tr("Cancel"), true, true).clicked() { answer = Some(false); }
            });
        });
        if m.should_close() && answer.is_none() { answer = Some(false); }
        match answer {
            Some(true) => {
                s.confirm_repo = None;
                match self.repos.add_repo(&url) { Ok(()) => { s.add_url.clear(); s.add_error = None; } Err(e) => s.add_error = Some(e) }
            }
            Some(false) => s.confirm_repo = None,
            None => {}
        }
    }

    pub(super) fn install_warning(&mut self, ctx: &egui::Context, t: &Th, s: &mut PUi, now: f64) {
        let Some(a) = s.confirm.clone() else { return };
        let mut answer = None;
        let m = egui::Modal::new(egui::Id::new("plugin-install-warning")).frame(egui::Frame::popup(&ctx.style()).fill(t.card).corner_radius(12.0).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new(crate::trq!("Install %1 v%2?", a.plugin.name, a.plugin.version)).font(semibold(18.0)).color(t.text1));
            ui.add_space(8.0);
            kit::text_block(ui, tr(NATIVE_CODE), kit::BODY, t.text1, 0.0);
            if !a.official {
                ui.add_space(8.0);
                kit::text_block(ui, &crate::trq!("This plugin is not from the official Webox repository but from \u{201c}%1\u{201d}:", a.repo_title), kit::BODY, t.tk("listitem-textmissed"), 0.0);
                kit::text_block(ui, &a.repo, kit::SECOND, t.text2, 0.0);
            }
            ui.add_space(8.0);
            let authors = if a.plugin.authors.is_empty() { tr("unknown").to_string() } else { a.plugin.authors.join(", ") };
            kit::text_block(ui, &crate::trq!("Author: %1 · Package: %2", authors, a.plugin.pkg), kit::SECOND, t.text2, 0.0);
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::pill(ui, t, tr("Install"), true, true).clicked() { answer = Some(true); }
                ui.add_space(8.0);
                if kit::pill(ui, t, tr("Cancel"), true, true).clicked() { answer = Some(false); }
            });
        });
        if m.should_close() && answer.is_none() { answer = Some(false); }
        match answer {
            Some(true) => { s.confirm = None; self.repos.confirm_warning(); self.start_install(a, now); }
            Some(false) => s.confirm = None,
            None => {}
        }
    }
}
