//! The read-only notice of a closed conversation: "Diese Unterhaltung ist geschlossen." over "Dieser Bereich ist nicht mehr
//! aktiv, weil die Kommunikation zwischen bestimmten Gruppen gemäß der Richtlinie untersagt ist." with "Auf Updates prüfen" at
//! the right. It replaces the composer.
//!
//! Layout (pt): 96 high, background a vertical #121212 -> #0F0F0F, title (#F7F7F7) centred on the card, paragraph
//! (#B2B2B2, 22.4 pt line pitch), link "Auf Updates prüfen" semibold #64B4FA (= button-hyperlink-text) with a pop-out glyph,
//! 23 pt from the card's right edge, vertically centred on the paragraph.
//! "Auf Updates prüfen" is an app update check. Webox has no updater; the link opens the update check dialog, which answers
//! "Sie sind auf dem neuesten Stand" (ui/app_dialogs.rs).
use crate::*;

/// Height of the notice (pt).
pub(crate) const CLOSED_BAR_H: f32 = 96.0;

impl App {
    /// Whether the space is in the read-only "closed" state (see `Space::closed`).
    pub(crate) fn space_closed(&self, id: &str) -> bool { self.spaces.iter().any(|s| s.id == id && s.closed) }

    /// Paints the notice into `r` (the card's bottom strip, full card width).
    pub(crate) fn closed_notice(&mut self, ui: &mut egui::Ui, t: &Th, r: Rect) {
        let light = super::menu::light(t);
        // Dark: a vertical gradient. Light: the theme's own composer background.
        if light { ui.painter().rect_filled(r, 0.0, t.composer); } else { vgrad(ui.painter(), r, 0.0, hex(0x121212), hex(0x0F0F0F)); }
        // text-primary / text-secondary (dark #F7F7F7 / #B2B2B2; light #121212 / #545454).
        let (c_title, c_text, c_link) = (t.text1, t.text2, t.link);
        let body = FontId::proportional(16.0);
        // Title: one line, centred on the card.
        let g = ui.painter().layout_no_wrap(crate::trq!("This conversation is closed").into_owned(), body.clone(), c_title);
        ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, r.top() + 16.0), g, c_title);
        // Link at the right: text + launch glyph, 23 from the right edge.
        let lg = ui.painter().layout_no_wrap(crate::trq!("Check for updates").into_owned(), semibold(16.0), c_link);
        // Launch glyph 12 wide, its right edge 23.2 from the card edge, 11.2 after the text; the svg's glyph fills ~3/4 of its box,
        // hence a 16 box.
        let glyph_c = r.right() - 23.2 - 6.0;
        let link_left = glyph_c - 6.0 - 11.2 - lg.size().x;
        // Paragraph: centred in the room left of the link, wrapped.
        let (pl, pr) = (r.left() + 24.0, link_left - 33.0);
        let mut job = egui::text::LayoutJob::simple(crate::trq!("This space is no longer active because communication between certain groups is prohibited by policy.").into_owned(), body, c_text, (pr - pl).max(80.0));
        job.halign = egui::Align::Center;
        let pg = ui.painter().layout_job(job);
        let ptop = r.top() + 46.0;
        let pmid = ptop + pg.size().y / 2.0;
        ui.painter().galley(pos2((pl + pr) / 2.0, ptop), pg, c_text);
        // The link opens the update check (ui/app_dialogs.rs: "up to date"; Webox has no updater).
        let link_r = Rect::from_min_max(pos2(link_left, pmid - lg.size().y / 2.0), pos2(glyph_c + 8.0, pmid + lg.size().y / 2.0));
        let lresp = ui.interact(link_r, ui.id().with("closed-check-updates"), Sense::click());
        if lresp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        if lresp.clicked() { super::menu::menu_open(ui.ctx(), super::app_dialogs::UPDATES); }
        ui.painter().galley(pos2(link_left, pmid - lg.size().y / 2.0), lg, c_link);
        // Pop-out icon right of the link.
        self.icons.paint(ui, "pop-out-bold", Rect::from_center_size(pos2(glyph_c, pmid), Vec2::splat(16.0)), c_link);
    }

    /// The announcement notice: "Das ist ein Ankündigungsbereich" over "In diesem Bereich können nur Moderatoren ..." (both
    /// text-primary, 16 px, centred, the body word-wrapped). Placed like the closed notice.
    pub(crate) fn announcement_notice(&mut self, ui: &mut egui::Ui, t: &Th, r: Rect) {
        let light = super::menu::light(t);
        if light { ui.painter().rect_filled(r, 0.0, t.composer); } else { vgrad(ui.painter(), r, 0.0, hex(0x121212), hex(0x0F0F0F)); }
        let c = t.text1;
        let body = FontId::proportional(16.0);
        let g = ui.painter().layout_no_wrap(tr!("ConversationRenderer", "This is an announcement space").into_owned(), body.clone(), c);
        ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, r.top() + 16.0), g, c);
        let mut job = egui::text::LayoutJob::simple(tr!("ConversationRenderer", "Only moderators can post content, start calls, schedule meetings, or use whiteboards in this space.").into_owned(), body, c, (r.width() - 48.0).max(80.0));
        job.halign = egui::Align::Center;
        let pg = ui.painter().layout_job(job);
        ui.painter().galley(pos2(r.center().x, r.top() + 46.0), pg, c);
    }
}
