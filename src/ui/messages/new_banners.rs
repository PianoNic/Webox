//! "New messages" / "New replies" buttons over the message list: pill buttons (L, icon on the right). The "above"
//! ones (arrow-up_12) sit at the top of the list, 4 pt below it, the replies one 4 pt below the messages one when both show;
//! the "below" ones (arrow-down_12) are stacked 4 pt above the scroll-to-latest button (jump_button.rs), the replies one
//! above the messages one.
//! Rules: a target message; while it lies below the visible rows the "below" button shows, above them the "above" button;
//! once it is (partly) visible it is marked seen and both stay hidden until a new target comes. Reply targets work the same
//! over a list (each seen on its own; above / below if any unseen one is). A click centres the target row; for replies the
//! first unseen target in that direction.
//! Here: the message target is the first unread message when the chat opens (the "New messages" line) and, while the
//! list is scrolled up, the first message from someone else that arrives; reply targets likewise. Pill metrics: height 32,
//! 14 pt semibold, 16 pt side padding. Colours: button-message-fill (#1170CF / hover #0353A8 / pressed #063A75, text white
//! @ 0.95).
use super::*;

const GAP: f32 = 4.0;
const PILL_H: f32 = 32.0;

/// The targets of a space: (message id, seen) for "New messages", and the reply targets.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NewTargets { pub msg: Option<(String, bool)>, pub replies: Vec<(String, bool)> }

/// Where a target row is relative to the view (content-space top / bottom of the row, view = offset .. offset + height).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place { Above, Below, Visible, Unknown }

pub(crate) fn place(row: Option<(f32, f32)>, off: f32, view_h: f32) -> Place {
    match row {
        None => Place::Unknown,
        Some((_, b)) if b <= off + 0.5 => Place::Above,
        Some((t, _)) if t >= off + view_h - 0.5 => Place::Below,
        Some(_) => Place::Visible,
    }
}

/// Which buttons show: (messages above, messages below, replies above, replies below). Targets that are visible get marked
/// seen (in `t`); seen targets never show again.
pub(crate) fn banners(t: &mut NewTargets, row: impl Fn(&str) -> Option<(f32, f32)>, off: f32, view_h: f32) -> (bool, bool, bool, bool) {
    let (mut ma, mut mb, mut ra, mut rb) = (false, false, false, false);
    if let Some((id, seen)) = &mut t.msg {
        if !*seen {
            match place(row(id), off, view_h) { Place::Above => ma = true, Place::Below => mb = true, Place::Visible => *seen = true, Place::Unknown => {} }
        }
    }
    for (id, seen) in t.replies.iter_mut().filter(|r| !r.1) {
        match place(row(id), off, view_h) { Place::Above => ra = true, Place::Below => rb = true, Place::Visible => *seen = true, Place::Unknown => {} }
    }
    (ma, mb, ra, rb)
}

/// The offset that centres a row, within 0 ..= max.
pub(crate) fn centre_offset(row: (f32, f32), view_h: f32, max: f32) -> f32 { ((row.0 + row.1) / 2.0 - view_h / 2.0).clamp(0.0, max.max(0.0)) }

fn key(space: &str) -> Id { Id::new(("msg-new-targets", space)) }
pub(super) fn targets(ctx: &egui::Context, space: &str) -> NewTargets { ctx.data(|d| d.get_temp::<NewTargets>(key(space))).unwrap_or_default() }
pub(super) fn set_targets(ctx: &egui::Context, space: &str, t: NewTargets) { ctx.data_mut(|d| d.insert_temp(key(space), t)); }

/// Opening a space: what is there now counts as known (messages that came while it was closed are no arrivals).
pub(super) fn forget_arrivals(ctx: &egui::Context, space: &str) {
    ctx.data_mut(|d| { d.remove::<std::collections::HashSet<String>>(Id::new(("msg-known-ids", space))); d.remove::<(usize, String)>(Id::new(("msg-known-sig", space))); });
}

/// Messages that arrived since the last frame (ids not seen before in this space); the first call per space only records.
pub(super) fn arrived(ctx: &egui::Context, space: &str, msgs: &[Message]) -> Vec<String> {
    let k = Id::new(("msg-known-ids", space));
    let sig = (msgs.len(), msgs.last().map(|m| m.id.clone()).unwrap_or_default());
    let ks = Id::new(("msg-known-sig", space));
    if ctx.data(|d| d.get_temp::<(usize, String)>(ks)).as_ref() == Some(&sig) { return Vec::new(); }
    ctx.data_mut(|d| d.insert_temp(ks, sig));
    let known: Option<std::collections::HashSet<String>> = ctx.data(|d| d.get_temp(k));
    let now: std::collections::HashSet<String> = msgs.iter().map(|m| m.id.clone()).collect();
    let new = match &known { Some(k) => msgs.iter().filter(|m| !k.contains(&m.id)).map(|m| m.id.clone()).collect(), None => Vec::new() };
    ctx.data_mut(|d| d.insert_temp(k, now));
    new
}

impl App {
    /// Draws the buttons over `list` and handles their clicks (sets the scroll offset `k_set`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_banners(&mut self, ui: &mut egui::Ui, list: Rect, space: &str, msgs: &[Message], off: f32, view_h: f32, max: f32, k_set: Id) {
        let ctx = ui.ctx().clone();
        let spans = ctx.data(|d| d.get_temp::<Vec<(String, f32, f32)>>(Id::new("msg-rowspans"))).unwrap_or_default();
        // A reply inside a collapsed thread has no row of its own: its thread root stands for it.
        let row = |id: &str| -> Option<(f32, f32)> {
            let find = |x: &str| spans.iter().find(|s| s.0 == x).map(|s| (s.1, s.2));
            find(id).or_else(|| msgs.iter().find(|m| m.id == id).and_then(|m| find(&m.parent)))
        };
        let mut t = targets(&ctx, space);
        let before = t.clone();
        let (ma, mb, ra, rb) = banners(&mut t, row, off, view_h);
        let mut go: Option<(f32, f32)> = None;
        let msgs_txt = crate::tr!("MessagesView", "New messages").into_owned();
        let replies_txt = crate::tr!("MessagesView", "New replies").into_owned();
        // Below: stacked on the scroll-to-latest button (bottom 20, 24 high) with 4 pt gaps.
        let mut bottom = list.bottom() - 20.0 - 24.0 - GAP;
        if mb && self.banner_pill(ui, list, &msgs_txt, "arrow-down_12", bottom, false, "nb-msg-below") { go = t.msg.as_ref().and_then(|m| row(&m.0)); }
        if mb { bottom -= PILL_H + GAP; }
        if rb && self.banner_pill(ui, list, &replies_txt, "arrow-down_12", bottom, false, "nb-rep-below") {
            go = t.replies.iter().filter(|r| !r.1).find_map(|r| row(&r.0).filter(|p| place(Some(*p), off, view_h) == Place::Below));
        }
        // Above: from the top of the list down.
        let mut top = list.top() + GAP;
        if ma && self.banner_pill(ui, list, &msgs_txt, "arrow-up_12", top, true, "nb-msg-above") { go = t.msg.as_ref().and_then(|m| row(&m.0)); }
        if ma { top += PILL_H + GAP; }
        if ra && self.banner_pill(ui, list, &replies_txt, "arrow-up_12", top, true, "nb-rep-above") {
            go = t.replies.iter().filter(|r| !r.1).find_map(|r| row(&r.0).filter(|p| place(Some(*p), off, view_h) == Place::Above));
        }
        if let Some(r) = go { ctx.data_mut(|d| d.insert_temp(k_set, centre_offset(r, view_h, max))); ctx.request_repaint(); }
        if t != before { set_targets(&ctx, space, t); }
    }

    /// One banner button, centred over `list`; `y` is its top (`from_top`) or its bottom. True when clicked.
    #[allow(clippy::too_many_arguments)]
    fn banner_pill(&mut self, ui: &mut egui::Ui, list: Rect, text: &str, icon: &'static str, y: f32, from_top: bool, id: &str) -> bool {
        let g = ui.painter().layout_no_wrap(text.to_string(), FontId::new(14.0, egui::FontFamily::Name("semibold".into())), wa(Color32::WHITE, 0.95));
        let w = 16.0 + g.size().x + 8.0 + 12.0 + 16.0;
        let top = if from_top { y } else { y - PILL_H };
        let r = Rect::from_min_size(pos2(list.center().x - w / 2.0, top), Vec2::new(w, PILL_H));
        let resp = ui.interact(r, Id::new(id), Sense::click());
        let bg = if resp.is_pointer_button_down_on() { hex(0x063A75) } else if resp.hovered() { hex(0x0353A8) } else { hex(0x1170CF) };
        ui.painter().rect_filled(r, PILL_H / 2.0, bg);
        ui.painter().galley(pos2(r.left() + 16.0, r.center().y - g.size().y / 2.0), g, wa(Color32::WHITE, 0.95));
        self.icons.paint(ui, icon, Rect::from_center_size(pos2(r.right() - 16.0 - 6.0, r.center().y), Vec2::splat(12.0)), wa(Color32::WHITE, 0.95));
        if resp.hovered() { ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand); }
        resp.clicked()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_of_a_row() {
        assert_eq!(place(Some((0.0, 40.0)), 100.0, 400.0), Place::Above); // ends above the view
        assert_eq!(place(Some((90.0, 140.0)), 100.0, 400.0), Place::Visible); // partly visible counts
        assert_eq!(place(Some((500.0, 540.0)), 100.0, 400.0), Place::Below); // starts below the view
        assert_eq!(place(Some((480.0, 540.0)), 100.0, 400.0), Place::Visible);
        assert_eq!(place(None, 100.0, 400.0), Place::Unknown);
    }

    #[test]
    fn buttons_until_seen_then_never_again() {
        let rows = |id: &str| match id { "m" => Some((1000.0, 1040.0)), "r1" => Some((10.0, 50.0)), "r2" => Some((2000.0, 2040.0)), _ => None };
        let mut t = NewTargets { msg: Some(("m".into(), false)), replies: vec![("r1".into(), false), ("r2".into(), false)] };
        // View 300..700: the message and r2 are below, r1 above.
        assert_eq!(banners(&mut t, rows, 300.0, 400.0), (false, true, true, true));
        // Scrolled to the message: it is seen, its button goes; reply buttons stay.
        assert_eq!(banners(&mut t, rows, 900.0, 400.0), (false, false, true, true));
        assert!(t.msg.as_ref().unwrap().1);
        // Scrolled back up: the seen message no longer shows a button.
        assert_eq!(banners(&mut t, rows, 0.0, 400.0), (false, false, false, true)); // r1 now visible -> seen
        assert_eq!(banners(&mut t, rows, 1900.0, 400.0), (false, false, false, false)); // r2 seen
        assert_eq!(banners(&mut t, rows, 300.0, 400.0), (false, false, false, false));
        // No target: nothing.
        assert_eq!(banners(&mut NewTargets::default(), rows, 0.0, 400.0), (false, false, false, false));
    }

    #[test]
    fn centring() {
        assert_eq!(centre_offset((1000.0, 1040.0), 400.0, 5000.0), 820.0);
        assert_eq!(centre_offset((10.0, 50.0), 400.0, 5000.0), 0.0);
        assert_eq!(centre_offset((4990.0, 5030.0), 400.0, 4700.0), 4700.0);
    }
}
