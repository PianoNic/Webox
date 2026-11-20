//! Compact view of the space list ("Kompakte Ansicht anzeigen"): 32 pt rows on a 33 pt pitch, 24 pt avatars 4 pt from the
//! row's left edge, the text 12 pt after them in the secondary font, name and subtitle on one line ("Name • Untertitel").
//! Toggled in the list's filter menu and in Settings > Messaging > Bereichsliste; stored locally as compact_view, off by
//! default.
use crate::*;

/// Settings JSON key.
pub(crate) const KEY: &str = "compact_view";
/// The filter menu item's id.
pub(crate) const MENU_ID: &str = "compact-view";

/// (name width, subtitle width) out of `avail`. `header_w` / `sub_w` are the texts' natural widths (rounded up + 2);
/// `has_sub` = there is a subtitle. The name has 6 pt right padding, the "•" is 6 wide, the subtitle has 6 pt left padding,
/// each keeps at least 30.
pub(crate) fn compact_widths(avail: f32, header_w: f32, sub_w: f32, has_sub: bool) -> (f32, f32) {
    const PAD: f32 = 6.0;
    const DOT: f32 = 6.0;
    const MIN: f32 = 30.0;
    if !has_sub { return (avail, 0.0); }
    let he = header_w + PAD;
    let se = sub_w + PAD;
    let hw = if avail >= he + DOT + se || avail >= he + DOT + MIN { he }
        else if avail >= MIN + DOT + MIN { avail - DOT - MIN }
        else { MIN.max(avail - DOT - MIN) };
    let sw = MIN.max(se.min(avail - hw - DOT));
    (hw, sw)
}

/// Draws "title • subtitle" on one line, vertically centred on `cy`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn one_line(p: &egui::Painter, x: f32, cy: f32, avail: f32, title: &str, tf: FontId, title_col: Color32, sub: &str, sub_col: Color32, dot_col: Color32) {
    let title = title.trim_end();
    let sub = sub.replace('\n', " ");
    let sub_font = FontId::proportional(13.0); // as in the two-line rows
    let nat = |s: &str, f: FontId| p.layout_no_wrap(s.to_string(), f, Color32::WHITE).size().x.ceil() + 2.0;
    let has_sub = !sub.trim().is_empty();
    let (hw, sw) = compact_widths(avail, nat(title, tf.clone()), if has_sub { nat(&sub, sub_font.clone()) } else { 0.0 }, has_sub);
    super::fit_text(p, x, cy, title, tf.clone(), title_col, (hw - 6.0).max(10.0));
    if has_sub {
        p.text(pos2(x + hw, cy), egui::Align2::LEFT_CENTER, "•", tf, dot_col);
        super::fit_text(p, x + hw + 6.0 + 6.0, cy, &sub, sub_font, sub_col, (sw - 6.0).max(10.0));
    }
}

impl App {
    /// The compact view is on.
    pub(crate) fn compact_view(&self) -> bool { self.settings[KEY].as_bool().unwrap_or(false) }

    /// Switches the compact view and saves it (not in --screenshot runs, which never touch the user's file).
    pub(crate) fn set_compact_view(&mut self, on: bool) {
        self.settings[KEY] = json!(on);
        if self.shot.is_none() { system::save_settings(&self.cfg, &self.settings); }
    }

    /// The filter menu's option entries (after the filters).
    pub(crate) fn filter_menu_options(&self) -> Vec<super::super::menu::Entry> {
        vec![super::super::menu::Entry::Sep, super::super::menu::Item::new(MENU_ID, crate::trq!("Show compact view")).checked(self.compact_view()).entry()]
    }
}

#[cfg(test)]
mod tests {
    use super::compact_widths;

    #[test]
    fn widths_share_the_line() {
        // no subheader: the header takes everything
        assert_eq!(compact_widths(200.0, 50.0, 0.0, false), (200.0, 0.0));
        // room for both: natural widths (+ paddings)
        assert_eq!(compact_widths(300.0, 100.0, 80.0, true), (106.0, 86.0));
        // header fits with at least 30 for the subheader: the header keeps its width, the subheader gets the rest
        assert_eq!(compact_widths(160.0, 100.0, 80.0, true), (106.0, 48.0));
        // too narrow: the subheader keeps its 30, the header shrinks
        assert_eq!(compact_widths(120.0, 100.0, 80.0, true), (84.0, 30.0));
        // tiny: both minimums
        assert_eq!(compact_widths(40.0, 100.0, 80.0, true), (30.0, 30.0));
    }
}
