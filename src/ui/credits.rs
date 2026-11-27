//! "About Webox": third-party credits. Standalone so the settings page can embed `credits_ui` in its own "About" page;
//! until then it is reachable from the rail's "More" menu (chrome/rail.rs).
use crate::*;
use super::menu::{self, Button};

/// Third-party material shipped in the app: (what, credit, licence), English sources (translated where shown).
pub(crate) const CREDITS: [(&str, &str, &str); 2] = [
    ("Emoji graphics", "Twemoji © Twitter, Inc. and other contributors (jdecked/twemoji)", "CC-BY 4.0 – https://creativecommons.org/licenses/by/4.0/"),
    ("Icons and illustrations", "Momentum icon set of the original client", "as in the original"),
];

impl App {
    /// The credits section (use inside any page or dialog).
    pub(crate) fn credits_ui(&self, ui: &mut egui::Ui, t: &Th) {
        ui.label(RichText::new(tr("Open source licenses and legal notices")).font(F::BodyLarge.semibold()).color(t.text1));
        ui.add_space(6.0);
        for (what, who, lic) in CREDITS {
            ui.label(RichText::new(tr(what)).font(F::Label.semibold()).color(t.text1));
            ui.label(RichText::new(tr(who)).size(13.0).color(t.text2));
            ui.label(RichText::new(tr(lic)).size(12.5).color(t.text2));
            ui.add_space(8.0);
        }
        ui.label(RichText::new(tr("Emoji graphics: Twemoji © Twitter, Inc. and other contributors, licensed CC-BY 4.0")).size(12.5).color(t.text2));
    }

    /// The "About Webox" dialog (open it with `menu::menu_open(ctx, "about")`); call every frame.
    pub(crate) fn about_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, "about") { return; }
        let this: &App = self;
        let mut body = |ui: &mut egui::Ui| this.credits_ui(ui, t);
        if menu::webex_dialog(ctx, t, "about-dlg", tr("About Webox"), "", Some(&mut body), &[Button::primary("close", tr("Close"))]).is_some() { menu::menu_close(ctx, "about"); }
    }
}
