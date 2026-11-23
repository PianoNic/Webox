//! Horizontal layout of the header: avatar and status button on the left, back / forward, the search field and + in the
//! middle, the device button and the AI assistant on the right, window controls at the edge.
//!
//! Webex lays the header out in three columns. The middle one (search field) has a fixed width; the left (status button)
//! and right (device button) columns give up space equally when the window gets narrow: the status text elides, the
//! device button elides its label and finally shows only its icon. Nothing overlaps down to the minimum window width.
use crate::egui::{pos2, Rect, Vec2};

/// Search field: fixed 354 x 28 pt, centred at 45.5 % of the header width when there is room.
pub(crate) const SEARCH_W: f32 = 354.0;
const SEARCH_AT: f32 = 0.455;
/// Narrowest search field, below the minimum window width only (placeholder elided, shortcut hint hidden).
const SEARCH_MIN: f32 = 140.0;
/// Back cell left edge to the search field (two 30 pt cells, glyph centres 61 / 31 pt left of the field).
const NAV_W: f32 = 76.0;
/// Search field right edge to the + button's right edge (26 pt circle centred 32 pt right of the field).
const PLUS_W: f32 = 45.0;
/// Avatar: 32 pt centred 28 pt from the left; status button 16 pt after it.
const AVATAR_CX: f32 = 28.0;
const CHIP_X: f32 = AVATAR_CX + 16.0 + 16.0;
/// Status button padding: 1 pt border + 14 pt text padding per side.
const CHIP_PAD: f32 = 30.0;
/// Gap between the status button and the back button (2 pt layout spacing).
const LEFT_GAP: f32 = 2.0;
/// Gap between + and the device button (12 pt layout spacing).
const PLUS_GAP: f32 = 12.0;
/// Device button: right edge 165 pt from the header's right edge, icon + label padding 44 pt.
const DEVICE_R: f32 = 165.0;
const DEVICE_PAD: f32 = 44.0;
/// Icon-only device button (32 pt circle, the labelled button's height).
const DEVICE_ICON_W: f32 = 32.0;
/// Below this much label room a label is dropped (status button hidden, device button icon-only).
const MIN_LABEL: f32 = 24.0;
/// AI assistant: 30 pt cell centred 140 pt from the right edge.
const AI_R: f32 = 140.0;

/// Minimum window size (pt). The header needs this width to show all its items without overlap.
pub(crate) const MIN_WINDOW: [f32; 2] = [760.0, 540.0];

/// Keeps the minimum window size in content points when the UI is zoomed (Ctrl++), so the header keeps its room.
pub(crate) fn keep_min_size(ctx: &crate::egui::Context) {
    let (id, z) = (crate::egui::Id::new("min-size-zoom"), ctx.zoom_factor());
    if ctx.data(|d| d.get_temp::<f32>(id)) == Some(z) { return; }
    ctx.data_mut(|d| d.insert_temp(id, z));
    ctx.send_viewport_cmd(crate::egui::ViewportCommand::MinInnerSize(Vec2::from(MIN_WINDOW)));
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HeaderLayout {
    pub avatar: Rect,
    /// Status button; `None` when there is no room for its text.
    pub status: Option<Rect>,
    pub back: Rect,
    pub fwd: Rect,
    pub search: Rect,
    pub plus: Rect,
    pub device: Rect,
    /// The device button shows its label (else only the icon, with the label as tooltip).
    pub device_label: bool,
    pub ai: Rect,
}

/// Lays out the header `full` (cy = vertical centre line) for a status text `status_w` and a device label `device_w` wide.
pub(crate) fn header_layout(full: Rect, cy: f32, status_w: f32, device_w: f32) -> HeaderLayout {
    let (x0, w) = (full.left(), full.width());
    let mid_w = NAV_W + SEARCH_W + PLUS_W;
    // Where the middle group may start (left column end) and must end (right column start), natural and minimal.
    let chip_nat = status_w + CHIP_PAD;
    let left_nat = x0 + CHIP_X + chip_nat + LEFT_GAP;
    let left_min = x0 + AVATAR_CX + 16.0 + LEFT_GAP;
    let dev_right = x0 + w - DEVICE_R;
    let dev_nat = device_w + DEVICE_PAD;
    let right_nat = dev_right - dev_nat - PLUS_GAP;
    let right_min = dev_right - DEVICE_ICON_W - PLUS_GAP;
    let want = x0 + w * SEARCH_AT - SEARCH_W / 2.0 - NAV_W;
    let (mut take_l, mut take_r, mut search_w) = (0.0, 0.0, SEARCH_W);
    let start = if right_nat - left_nat >= mid_w {
        want.clamp(left_nat, right_nat - mid_w)
    } else {
        // Both side columns give up the shortfall equally, each down to its minimum.
        let over = mid_w - (right_nat - left_nat);
        let (room_l, room_r) = ((left_nat - left_min).max(0.0), (right_min - right_nat).max(0.0));
        take_l = (over / 2.0).min(room_l);
        take_r = (over - take_l).min(room_r);
        take_l = (over - take_r).min(room_l);
        // Only a zoomed UI gets narrower than the minimum window: then the search field gives up the rest.
        let rest = over - take_l - take_r;
        if rest > 0.01 { search_w = (SEARCH_W - rest).max(SEARCH_MIN); }
        left_nat - take_l
    };
    let sr = Rect::from_center_size(pos2(start + NAV_W + search_w / 2.0, cy), Vec2::new(search_w, 28.0));
    let chip_w = chip_nat - take_l;
    let status = (chip_w - CHIP_PAD >= MIN_LABEL).then(|| Rect::from_min_max(pos2(x0 + CHIP_X, cy - 14.0), pos2(x0 + CHIP_X + chip_w, cy + 14.0)));
    let dev_w = dev_nat - take_r;
    let device_label = dev_w - DEVICE_PAD >= MIN_LABEL;
    let dev_w = if device_label { dev_w } else { DEVICE_ICON_W };
    HeaderLayout {
        avatar: Rect::from_center_size(pos2(x0 + AVATAR_CX, cy), Vec2::splat(32.0)),
        status,
        back: Rect::from_center_size(pos2(sr.left() - 61.0, cy), Vec2::splat(30.0)),
        fwd: Rect::from_center_size(pos2(sr.left() - 31.0, cy), Vec2::splat(30.0)),
        search: sr,
        plus: Rect::from_center_size(pos2(sr.right() + 32.0, cy), Vec2::splat(26.0)),
        device: Rect::from_min_max(pos2(dev_right - dev_w, cy - 16.0), pos2(dev_right, cy + 16.0)),
        device_label,
        ai: Rect::from_center_size(pos2(x0 + w - AI_R, cy), Vec2::splat(30.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Left edge of the window controls (minimise cell), from the header's right edge.
    const CONTROLS_W: f32 = 14.0 + 40.2 * 2.0 + 20.0;

    fn lay(w: f32, status_w: f32, device_w: f32) -> (HeaderLayout, Rect) {
        let full = Rect::from_min_size(pos2(6.0, 6.0), Vec2::new(w, 44.8));
        (header_layout(full, full.top() + 18.4, status_w, device_w), full)
    }

    /// Left-to-right order with no overlaps, all inside the header and left of the window controls.
    fn assert_no_overlap(l: &HeaderLayout, full: Rect, what: &str) {
        let mut seq = vec![("avatar", l.avatar)];
        if let Some(s) = l.status { seq.push(("status", s)); }
        seq.extend([("back", l.back), ("fwd", l.fwd), ("search", l.search), ("plus", l.plus), ("device", l.device), ("ai", l.ai)]);
        seq.push(("controls", Rect::from_min_max(pos2(full.right() - CONTROLS_W, full.top()), full.max)));
        assert!(seq[0].1.left() >= full.left(), "{what}: avatar outside");
        for p in seq.windows(2) {
            assert!(p[0].1.right() <= p[1].1.left() + 0.01, "{what}: {} ({:?}) overlaps {} ({:?})", p[0].0, p[0].1, p[1].0, p[1].1);
        }
    }

    #[test]
    fn nothing_overlaps_from_the_minimum_width_up() {
        // 748 pt = the 760 pt minimum window minus the 6 pt resize strips; German / English / long texts.
        for (status_w, device_w) in [(115.0, 172.0), (106.0, 133.0), (320.0, 172.0), (0.0, 0.0)] {
            let mut w = 748.0;
            while w <= 2600.0 {
                let (l, full) = lay(w, status_w, device_w);
                assert_no_overlap(&l, full, &format!("w {w}, texts {status_w} / {device_w}"));
                assert!((l.search.width() - SEARCH_W).abs() < 0.01, "w {w}: search {}", l.search.width()); // the search field never shrinks
                w += 0.5;
            }
        }
    }

    #[test]
    fn zoomed_below_the_minimum_only_the_search_field_shrinks() {
        // Ctrl++ on a minimum-size window: down to 520 pt of content nothing overlaps.
        let mut w = 520.0;
        while w < 748.0 {
            let (l, full) = lay(w, 115.0, 172.0);
            assert_no_overlap(&l, full, &format!("w {w}"));
            assert!(l.search.width() >= SEARCH_MIN);
            w += 0.5;
        }
    }

    #[test]
    fn wide_window_keeps_the_natural_layout() {
        // 1550 pt window: search 544-898 pt, + centred at 930 pt, full labels.
        let (l, _) = lay(1538.0, 115.0, 172.0);
        assert!((l.search.left() - (6.0 + 1538.0 * 0.455 - 177.0)).abs() < 0.01);
        assert!(l.status.is_some_and(|s| (s.width() - 145.0).abs() < 0.01));
        assert!(l.device_label && (l.device.width() - 216.0).abs() < 0.01);
    }

    #[test]
    fn narrow_window_elides_both_sides_then_drops_the_device_label() {
        // 960 pt (1200 px at 125 %): both labels shorter by the same amount.
        let (l, _) = lay(948.0, 115.0, 172.0);
        let (dl, dr) = (145.0 - l.status.unwrap().width(), 216.0 - l.device.width());
        assert!(dl > 0.0 && (dl - dr).abs() < 0.01, "status -{dl}, device -{dr}");
        // Minimum width: device icon-only, status button gone (the avatar stays).
        let (l, _) = lay(748.0, 115.0, 172.0);
        assert!(!l.device_label && l.device.width() == DEVICE_ICON_W);
        assert!(l.status.is_none());
    }
}
