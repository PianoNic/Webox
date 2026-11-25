//! Display density (Settings > Appearance): Standard or Compact. Compact tightens vertical spacing everywhere and keeps
//! every font size.
//!
//! Every spacing that depends on the density is a field of [`Metrics`]; `STANDARD` and `COMPACT` hold the two sets and the
//! views read `density::m().field`. The setting is stored as `density` ("standard" / "compact") and read every frame
//! ([`set_from`]), so a change applies at once.
//!
//! Each Kompakt value is the Standard value minus a multiple of 4 pt (whole pixels at 100 % and 125 %), so nothing moves off the
//! pixel grid (checked by the tests). The list's compact view is separate: see [`ListRow`].
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub(crate) enum Density { #[default] Standard, Compact }

impl Density {
    pub(crate) const ALL: [Density; 2] = [Density::Standard, Density::Compact];
    /// The value stored under `density` in the settings JSON.
    pub(crate) fn key(self) -> &'static str { match self { Density::Standard => "standard", Density::Compact => "compact" } }
    /// Unknown or missing values are Standard.
    pub(crate) fn from_key(s: &str) -> Density { if s == "compact" { Density::Compact } else { Density::Standard } }
    /// Labels.
    pub(crate) fn label(self) -> &'static str { match self { Density::Standard => "Standard", Density::Compact => "Kompakt" } }
    pub(crate) fn metrics(self) -> &'static Metrics { match self { Density::Standard => &STANDARD, Density::Compact => &COMPACT } }
}

/// Every spacing the density changes (pt).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Metrics {
    // ---- window header (ui/chrome)
    /// Height of the window header with the avatar, search pill and window controls; the controls' centre line stays.
    pub header_h: f32,
    // ---- chat list card (ui/messaging.rs, ui/chrome)
    /// Card top -> the "Messaging" title row.
    pub list_top: f32,
    /// Title row -> the All / Direct / Spaces tab row.
    pub list_title_gap: f32,
    /// Top of the scrolling list -> the "Recommended messages" entry.
    pub list_scroll_top: f32,
    /// The "Recommended messages" entry's height, its disc, and the space under it.
    pub recommended_h: f32,
    pub recommended_disc: f32,
    pub recommended_gap: f32,
    /// Section header row ("Favorites" / "Other").
    pub section_h: f32,
    /// Two-line list row: the hover / selection pill, the pitch (pill + 1 pt), the avatar, its left
    /// inset in the pill, and the text's x in the pill (avatar left + avatar + 12).
    pub row_h: f32,
    pub row_pitch: f32,
    pub row_avatar: f32,
    pub row_avatar_left: f32,
    pub row_text_x: f32,
    /// The pulsing placeholder rows while the list loads.
    pub row_skeleton_h: f32,
    // ---- conversation card banner (ui/conversation.rs)
    /// Banner height, the title / favourite star / action buttons' centre, the subtitle line's centre, the tab row's top.
    pub banner_h: f32,
    pub banner_title_y: f32,
    pub banner_actions_y: f32,
    pub banner_sub_y: f32,
    pub banner_tabs_y: f32,
    // ---- message list (ui/messages)
    /// List top -> first row.
    pub msg_list_top: f32,
    /// Row top -> first line, last line -> next row, row top -> avatar top, header line -> first content.
    pub msg_top_pad: f32,
    pub msg_bottom_pad: f32,
    pub msg_avatar_top: f32,
    pub msg_header_to_body: f32,
    /// Between two attachment blocks of one message.
    pub msg_attach_gap: f32,
    /// Date separators, "New messages" and info rows: text height + this.
    pub day_pad: f32,
    /// The "Seen by" row.
    pub seen_h: f32,
    /// Under the last list row.
    pub msg_list_bottom: f32,
    /// Collapsed thread row ("N replies" pill), and the space under a thread's end buttons.
    pub thread_collapsed_h: f32,
    pub thread_end_bottom: f32,
    // ---- composer (ui/composer)
    /// Composer line -> tool row top, -> reply banner top, -> text row top; under the text row.
    pub composer_tools_y: f32,
    pub composer_reply_y: f32,
    pub composer_text_top: f32,
    pub composer_bottom_pad: f32,
    // ---- settings (ui/settings)
    /// Page header, gap between sections, minimum checkbox / radio row height and the gap after a checkbox row.
    pub settings_header_h: f32,
    pub settings_section_gap: f32,
    pub settings_row_min: f32,
    pub settings_row_gap: f32,
    /// Navigation: first row's top, row height, row pitch.
    pub settings_nav_top: f32,
    pub settings_nav_row: f32,
    pub settings_nav_pitch: f32,
    // ---- menus / balloons (ui/menu)
    /// One-line item, two-line / avatar item.
    pub menu_item_h: f32,
    pub menu_item2_h: f32,
}

/// Standard density.
pub(crate) const STANDARD: Metrics = Metrics {
    header_h: 44.8,
    list_top: 18.0, list_title_gap: 8.0, list_scroll_top: 8.0, recommended_h: 44.0, recommended_disc: 32.0, recommended_gap: 7.0,
    section_h: 26.0, row_h: 50.0, row_pitch: 51.0, row_avatar: 32.0, row_avatar_left: 8.0, row_text_x: 52.0, row_skeleton_h: 56.0,
    banner_h: 116.0, banner_title_y: 26.0, banner_actions_y: 30.0, banner_sub_y: 52.0, banner_tabs_y: 69.0,
    msg_list_top: 8.0, msg_top_pad: 6.0, msg_bottom_pad: 7.7, msg_avatar_top: 18.0, msg_header_to_body: 5.1, msg_attach_gap: 8.0,
    day_pad: 30.0, seen_h: 48.0, msg_list_bottom: 25.5, thread_collapsed_h: 48.0, thread_end_bottom: 8.0,
    composer_tools_y: 10.0, composer_reply_y: 40.0, composer_text_top: 60.6, composer_bottom_pad: 13.6,
    settings_header_h: 52.0, settings_section_gap: 16.0, settings_row_min: 24.0, settings_row_gap: 4.0,
    settings_nav_top: 56.0, settings_nav_row: 36.0, settings_nav_pitch: 38.0,
    menu_item_h: 40.0, menu_item2_h: 48.0,
};

/// Kompakt: Standard minus multiples of 4 pt; fonts unchanged.
pub(crate) const COMPACT: Metrics = Metrics {
    header_h: 40.8,
    list_top: 14.0, list_title_gap: 4.0, list_scroll_top: 4.0, recommended_h: 36.0, recommended_disc: 28.0, recommended_gap: 3.0,
    // 28 pt avatars: two lines still fit in a 42 pt row
    section_h: 22.0, row_h: 42.0, row_pitch: 43.0, row_avatar: 28.0, row_avatar_left: 8.0, row_text_x: 48.0, row_skeleton_h: 48.0,
    banner_h: 100.0, banner_title_y: 22.0, banner_actions_y: 26.0, banner_sub_y: 44.0, banner_tabs_y: 57.0,
    // the avatar moves with the first body line (top pad and header gap both -4), so it never reaches below a one-line row
    msg_list_top: 4.0, msg_top_pad: 2.0, msg_bottom_pad: 7.7, msg_avatar_top: 10.0, msg_header_to_body: 1.1, msg_attach_gap: 4.0,
    day_pad: 22.0, seen_h: 40.0, msg_list_bottom: 17.5, thread_collapsed_h: 40.0, thread_end_bottom: 4.0,
    composer_tools_y: 6.0, composer_reply_y: 36.0, composer_text_top: 56.6, composer_bottom_pad: 9.6,
    settings_header_h: 44.0, settings_section_gap: 12.0, settings_row_min: 20.0, settings_row_gap: 0.0,
    settings_nav_top: 52.0, settings_nav_row: 32.0, settings_nav_pitch: 34.0,
    menu_item_h: 32.0, menu_item2_h: 44.0,
};

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The active density (Standard until [`set`] runs).
pub(crate) fn get() -> Density { if CURRENT.load(Ordering::Relaxed) == 1 { Density::Compact } else { Density::Standard } }
pub(crate) fn set(d: Density) { CURRENT.store(d as u8, Ordering::Relaxed); }
/// The active table.
pub(crate) fn m() -> &'static Metrics { get().metrics() }
/// Reads `density` from the settings JSON (called every frame, so a change applies at once).
pub(crate) fn set_from(settings: &serde_json::Value) { set(of_settings(settings)); }
/// The density stored in the settings JSON.
pub(crate) fn of_settings(settings: &serde_json::Value) -> Density { Density::from_key(settings["density"].as_str().unwrap_or("")) }

/// Geometry of one chat list row: the two-line row of the density table, or the compact view (one line, 32 pt), which wins
/// over the density.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ListRow {
    /// Hover / selection pill height (radius = half of it) and the row pitch.
    pub h: f32,
    pub pitch: f32,
    /// Avatar size, its left inset in the pill, the text's x in the pill.
    pub avatar: f32,
    pub avatar_left: f32,
    pub text_x: f32,
    /// One line "title • subtitle" instead of two lines.
    pub one_line: bool,
    /// Title font size.
    pub title_pt: f32,
    /// Height of a loading placeholder row.
    pub skeleton_h: f32,
}

/// The compact view: 32 pt rows on a 33 pt pitch, 24 pt avatars 4 pt in, the text at 40 in the secondary font (13.6), name and
/// subtitle on one line separated by a "•".
pub(crate) const COMPACT_VIEW: ListRow = ListRow { h: 32.0, pitch: 33.0, avatar: 24.0, avatar_left: 4.0, text_x: 40.0, one_line: true, title_pt: 13.6, skeleton_h: 33.0 };

impl ListRow {
    /// The row for the current density, or the compact view.
    pub(crate) fn current(compact_view: bool) -> ListRow {
        if compact_view { return COMPACT_VIEW; }
        let d = m();
        ListRow { h: d.row_h, pitch: d.row_pitch, avatar: d.row_avatar, avatar_left: d.row_avatar_left, text_x: d.row_text_x, one_line: false, title_pt: 15.6, skeleton_h: d.row_skeleton_h }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field as (name, value), so the tests cover new fields too (a field missing here fails `all_fields_listed`).
    fn fields(m: &Metrics) -> Vec<(&'static str, f32)> {
        vec![
            ("header_h", m.header_h), ("list_top", m.list_top), ("list_title_gap", m.list_title_gap), ("list_scroll_top", m.list_scroll_top),
            ("recommended_h", m.recommended_h), ("recommended_disc", m.recommended_disc), ("recommended_gap", m.recommended_gap),
            ("section_h", m.section_h), ("row_h", m.row_h), ("row_pitch", m.row_pitch), ("row_avatar", m.row_avatar),
            ("row_avatar_left", m.row_avatar_left), ("row_text_x", m.row_text_x), ("row_skeleton_h", m.row_skeleton_h), ("banner_h", m.banner_h), ("banner_title_y", m.banner_title_y),
            ("banner_actions_y", m.banner_actions_y), ("banner_sub_y", m.banner_sub_y), ("banner_tabs_y", m.banner_tabs_y),
            ("msg_list_top", m.msg_list_top), ("msg_top_pad", m.msg_top_pad), ("msg_bottom_pad", m.msg_bottom_pad),
            ("msg_avatar_top", m.msg_avatar_top), ("msg_header_to_body", m.msg_header_to_body), ("msg_attach_gap", m.msg_attach_gap),
            ("day_pad", m.day_pad), ("seen_h", m.seen_h), ("msg_list_bottom", m.msg_list_bottom), ("thread_collapsed_h", m.thread_collapsed_h),
            ("thread_end_bottom", m.thread_end_bottom), ("composer_tools_y", m.composer_tools_y), ("composer_reply_y", m.composer_reply_y),
            ("composer_text_top", m.composer_text_top), ("composer_bottom_pad", m.composer_bottom_pad), ("settings_header_h", m.settings_header_h),
            ("settings_section_gap", m.settings_section_gap), ("settings_row_min", m.settings_row_min), ("settings_row_gap", m.settings_row_gap),
            ("settings_nav_top", m.settings_nav_top), ("settings_nav_row", m.settings_nav_row), ("settings_nav_pitch", m.settings_nav_pitch),
            ("menu_item_h", m.menu_item_h), ("menu_item2_h", m.menu_item2_h),
        ]
    }

    #[test]
    fn all_fields_listed() {
        // Metrics is all f32: its size tells how many fields there are
        assert_eq!(fields(&STANDARD).len() * 4, std::mem::size_of::<Metrics>());
    }

    #[test]
    fn standard_is_the_previous_layout() {
        // the values the views had before the option
        let s = &STANDARD;
        assert_eq!((s.header_h, s.row_h, s.row_pitch, s.row_avatar, s.row_text_x), (44.8, 50.0, 51.0, 32.0, 52.0));
        assert_eq!((s.banner_h, s.banner_tabs_y, s.menu_item_h, s.menu_item2_h), (116.0, 69.0, 40.0, 48.0));
        assert_eq!((s.msg_top_pad, s.msg_bottom_pad, s.msg_avatar_top, s.msg_header_to_body), (6.0, 7.7, 18.0, 5.1));
        assert_eq!((s.day_pad, s.seen_h, s.msg_list_bottom, s.composer_text_top, s.composer_bottom_pad), (30.0, 48.0, 25.5, 60.6, 13.6));
        assert_eq!(Density::default(), Density::Standard);
        assert_eq!(Density::from_key(""), Density::Standard);
        assert_eq!(Density::from_key("bogus"), Density::Standard);
    }

    #[test]
    fn compact_is_never_larger_and_moves_on_the_pixel_grid() {
        for ((name, s), (_, c)) in fields(&STANDARD).into_iter().zip(fields(&COMPACT)) {
            assert!(c <= s, "{name}: Kompakt {c} > Standard {s}");
            assert!(c >= 0.0, "{name}");
            let d = s - c;
            // a multiple of 4 pt = whole pixels at 100 % (x1) and 125 % (x1.25)
            assert!((d / 4.0 - (d / 4.0).round()).abs() < 1e-4, "{name}: delta {d} is not a multiple of 4 pt");
            for zoom in [1.0_f32, 1.25] {
                let px = d * zoom;
                assert!((px - px.round()).abs() < 1e-3, "{name}: delta {px} px at {zoom}");
            }
        }
    }

    #[test]
    fn compact_rows_fit_their_content() {
        let c = &COMPACT;
        // two-line list row: title (15.6, about 18.7 high) centred 9.5 above and subtitle (13, about 15.6) 10 below the centre
        assert!(9.5 + 18.7 / 2.0 <= c.row_h / 2.0 && 10.0 + 15.6 / 2.0 <= c.row_h / 2.0);
        assert!(c.row_avatar <= c.row_h && c.row_pitch - c.row_h == STANDARD.row_pitch - STANDARD.row_h);
        assert_eq!(c.row_text_x, c.row_avatar_left + c.row_avatar + 12.0); // text 12 after the avatar, as in Standard
        assert_eq!(STANDARD.row_text_x, STANDARD.row_avatar_left + STANDARD.row_avatar + 12.0);
        // a one-line message with a header: the 32 pt avatar ends inside the row (header >= 16, body line >= 19)
        for m in [&STANDARD, &COMPACT] {
            assert!(m.msg_avatar_top + 32.0 <= m.msg_top_pad + 16.0 + m.msg_header_to_body + 19.0 + m.msg_bottom_pad);
            // the avatar keeps its distance to the first body line
            assert!(((m.msg_top_pad + m.msg_header_to_body - m.msg_avatar_top) - (STANDARD.msg_top_pad + STANDARD.msg_header_to_body - STANDARD.msg_avatar_top)).abs() < 1e-4);
            // collapsed thread: 28 pt avatars and pill
            assert!(m.thread_collapsed_h >= 28.0 + 8.0);
            // "Seen by": 24 pt avatars
            assert!(m.seen_h >= 24.0 + 8.0);
            // composer: tool row (28) ends before the reply banner, which ends (18) before the text row
            assert!(m.composer_tools_y + 28.0 <= m.composer_reply_y && m.composer_reply_y + 18.0 <= m.composer_text_top);
            // banner: title (20 pt) above the subtitle above the 28 pt tab row, all inside the banner
            assert!(m.banner_title_y + 12.0 <= m.banner_sub_y - 8.0 && m.banner_sub_y + 8.0 <= m.banner_tabs_y && m.banner_tabs_y + 28.0 <= m.banner_h);
            // the header's controls (centre 18.4, 32 pt avatar) stay inside it
            assert!(18.4 + 16.0 <= m.header_h);
            // menu rows: 16 pt text / two lines 16 + 14
            assert!(m.menu_item_h >= 24.0 && m.menu_item2_h >= 40.0 && m.menu_item2_h >= m.menu_item_h);
            // settings: the 16 pt checkbox
            assert!(m.settings_row_min >= 16.0 + 4.0 && m.settings_nav_row < m.settings_nav_pitch);
        }
    }

    #[test]
    fn compact_view_values() {
        let v = ListRow::current(true);
        assert_eq!((v.h, v.pitch, v.avatar, v.avatar_left, v.text_x), (32.0, 33.0, 24.0, 4.0, 40.0));
        assert!(v.one_line);
        let n = ListRow { h: 50.0, pitch: 51.0, avatar: 32.0, avatar_left: 8.0, text_x: 52.0, one_line: false, title_pt: 15.6, skeleton_h: 56.0 };
        assert_eq!(STANDARD.row_h, n.h);
        assert_eq!(STANDARD.row_pitch, n.pitch);
    }

    #[test]
    fn settings_value() {
        // (the global is not switched here: other tests run in parallel and expect Standard)
        assert_eq!(of_settings(&serde_json::json!({ "density": "compact" })), Density::Compact);
        assert_eq!(of_settings(&serde_json::json!({ "density": "standard" })), Density::Standard);
        assert_eq!(of_settings(&serde_json::json!({})), Density::Standard);
        assert_eq!(Density::Compact.metrics().menu_item_h, 32.0);
        for d in Density::ALL { assert_eq!(Density::from_key(d.key()), d); }
        assert_eq!(get(), Density::Standard);
    }
}
