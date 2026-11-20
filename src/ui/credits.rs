//! "Info über Webox": third-party credits. Standalone so the settings page can embed `credits_ui` in its own "Info" page;
//! until then it is reachable from the rail's "Weitere" menu (chrome/rail.rs).
use crate::*;
use super::menu::{self, Button};

/// Third-party material shipped in the app: (what, credit, licence).
pub(crate) const CREDITS: [(&str, &str, &str); 2] = [
    ("Emoji-Grafiken", "Twemoji © Twitter, Inc. und weitere Mitwirkende (jdecked/twemoji)", "CC-BY 4.0 – https://creativecommons.org/licenses/by/4.0/"),
    ("Symbole und Illustrationen", "Momentum-Symbolsatz des ursprünglichen Clients", "wie im Original"),
];

impl App {
    /// The credits section (use inside any page or dialog).
    pub(crate) fn credits_ui(&self, ui: &mut egui::Ui, t: &Th) {
        ui.label(RichText::new("Open-Source-Lizenzen und Rechtliche Hinweise").font(semibold(14.5)).color(t.text1));
        ui.add_space(6.0);
        for (what, who, lic) in CREDITS {
            ui.label(RichText::new(what).font(semibold(13.5)).color(t.text1));
            ui.label(RichText::new(who).size(13.0).color(t.text2));
            ui.label(RichText::new(lic).size(12.5).color(t.text2));
            ui.add_space(8.0);
        }
        ui.label(RichText::new("Emoji graphics: Twemoji © Twitter, Inc. and other contributors, licensed CC-BY 4.0").size(12.5).color(t.text2));
    }

    /// The "Info über Webox" dialog (open it with `menu::menu_open(ctx, "about")`); call every frame.
    pub(crate) fn about_dialog(&mut self, ctx: &egui::Context, t: &Th) {
        if !menu::menu_is_open(ctx, "about") { return; }
        let this: &App = self;
        let mut body = |ui: &mut egui::Ui| this.credits_ui(ui, t);
        if menu::webex_dialog(ctx, t, "about-dlg", "Info über Webox", "", Some(&mut body), &[Button::primary("close", "Schließen")]).is_some() { menu::menu_close(ctx, "about"); }
    }
}
