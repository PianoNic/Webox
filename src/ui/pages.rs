#![allow(clippy::too_many_arguments)]
use crate::*;

impl App {
    pub(crate) fn page(&mut self, ui: &mut egui::Ui, t: &Th, now: f64) {
        
        match self.tab {
            // The list pages live in page_kit.rs / calls_page.rs / meetings_page.rs / contacts_page.rs / teams_page.rs.
            Tab::Whiteboards => self.whiteboards_page(ui, t, now),
            Tab::Contacts | Tab::Calling | Tab::Meetings | Tab::Voicemail => {}
            Tab::Settings => self.settings_page(ui, t, now),
            Tab::Messaging | Tab::Teams => {}
        }
    }
}

