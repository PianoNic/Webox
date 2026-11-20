//! Middle-click autoscroll as in a web browser (a Webox addition).
//!
//! Press the middle button over a list that scrolls: an anchor (a circle with up / down chevrons) marks the press point, and
//! moving the pointer away from it scrolls that list, faster the farther away (dead zone `DEAD`). A click (any button),
//! another middle click or Esc ends it; pressing, moving out of the dead zone and releasing (or holding for `HOLD_S`) ends
//! it on release (Chrome's "drag" mode). Speeds use the measured frame time, so they don't depend on the frame rate.
//!
//! Every list built with `widgets::vscroll()` gets it: `VScroll` wraps egui's `ScrollArea`, applies the movement before the
//! area is shown and starts autoscroll after it was shown (the innermost scrolling list under the press wins).
//! `begin_frame` runs once per frame in `App::raw_input_hook` (ends autoscroll on Esc / clicks and swallows those events,
//! measures the frame time); `paint` runs at the end of the frame (anchor icon, cursor).
use eframe::egui;
use egui::{Color32, CursorIcon, Event, Id, Key, LayerId, Order, PointerButton, Pos2, Rect, Stroke, Ui, Vec2};

/// No scrolling while the pointer is this close to the anchor (pt), as in browsers.
pub(crate) const DEAD: f32 = 10.0;
/// A held middle button released after this long ends autoscroll even without moving (s).
const HOLD_S: f64 = 0.5;
const K: &str = "autoscroll";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Auto {
    /// The scroll area being scrolled (egui's persistent id of the area).
    pub area: Id,
    /// Where the middle button went down: the anchor.
    pub origin: Pos2,
    pub pointer: Pos2,
    /// The starting press is still held.
    pub held: bool,
    /// The pointer left the dead zone while the starting press was held.
    pub moved: bool,
    pub pressed_at: std::time::Instant,
    pub last: std::time::Instant,
    /// Movement for this frame (pt, + = down), taken once by the area.
    pub pending: f32,
    /// Frames since the area was last shown (ends autoscroll when the list goes away).
    pub unseen: u32,
    /// A press that ended autoscroll: its release is swallowed too.
    pub swallow: Option<PointerButton>,
}

/// Scroll speed (pt/s, + = down) for the pointer `dy` pt below (+) / above (-) the anchor: 0 inside the dead zone, then
/// growing a little faster than linearly with the distance (about 160 pt/s at 30 pt, 640 pt/s at 100 pt, 2400 pt/s at 300 pt).
pub(crate) fn speed(dy: f32) -> f32 {
    let x = (dy.abs() - DEAD).max(0.0);
    dy.signum() * (5.0 * x + 0.02 * x * x).min(6000.0)
}

fn get(ctx: &egui::Context) -> Option<Auto> { ctx.data(|d| d.get_temp::<Option<Auto>>(Id::new(K))).flatten() }
fn put(ctx: &egui::Context, a: Option<Auto>) { ctx.data_mut(|d| d.insert_temp(Id::new(K), a)); }
pub(crate) fn active(ctx: &egui::Context) -> bool { get(ctx).is_some() }

/// Applies the frame's raw events to a running autoscroll: tracks the pointer and the starting press, drops the events that
/// belong to autoscroll (Esc, the clicks that end it), and returns whether it ends. `pressed_for` = seconds since the press.
pub(crate) fn route_events(a: &mut Auto, events: &mut Vec<Event>, pressed_for: f64) -> bool {
    let mut end = false;
    events.retain(|e| match e {
        Event::PointerMoved(p) => { a.pointer = *p; if a.held && (a.pointer - a.origin).length() > DEAD { a.moved = true; } true }
        Event::Key { key: Key::Escape, pressed: true, .. } => { end = true; false }
        Event::Key { key: Key::Escape, pressed: false, .. } => false,
        Event::PointerButton { button, pressed: true, .. } => { end = true; a.swallow = Some(*button); false }
        Event::PointerButton { button, pressed: false, .. } => {
            if a.swallow == Some(*button) { a.swallow = None; return false; }
            if *button == PointerButton::Middle && a.held {
                a.held = false;
                if a.moved || pressed_for >= HOLD_S { end = true; }
                return false;
            }
            true
        }
        Event::WindowFocused(false) => { end = true; true }
        _ => true,
    });
    end
}

/// Once per frame, before egui sees the input (`App::raw_input_hook`).
pub(crate) fn begin_frame(ctx: &egui::Context, raw: &mut egui::RawInput) {
    // A swallowed release can arrive after autoscroll ended.
    let Some(mut a) = get(ctx) else {
        let ks = Id::new("autoscroll-swallow");
        if let Some(b) = ctx.data_mut(|d| { let b = d.get_temp::<PointerButton>(ks); d.remove::<PointerButton>(ks); b }) {
            let before = raw.events.len();
            raw.events.retain(|e| !matches!(e, Event::PointerButton { button, pressed: false, .. } if *button == b));
            if raw.events.len() == before { ctx.data_mut(|d| d.insert_temp(Id::new("autoscroll-swallow"), b)); }
        }
        return;
    };
    let now = std::time::Instant::now();
    let pressed_for = now.duration_since(a.pressed_at).as_secs_f64();
    let end = route_events(&mut a, &mut raw.events, pressed_for) || a.unseen > 2;
    if end {
        if let Some(b) = a.swallow { ctx.data_mut(|d| d.insert_temp(Id::new("autoscroll-swallow"), b)); }
        put(ctx, None);
        return;
    }
    let dt = now.duration_since(a.last).as_secs_f32().min(0.1);
    a.last = now;
    a.pending = speed(a.pointer.y - a.origin.y) * dt;
    a.unseen += 1;
    put(ctx, Some(a));
}

/// The movement this frame for the area `id` (once; 0 for any other area).
fn take(ctx: &egui::Context, id: Id) -> f32 {
    let Some(mut a) = get(ctx) else { return 0.0 };
    if a.area != id { return 0.0; }
    let dy = std::mem::take(&mut a.pending);
    a.unseen = 0;
    put(ctx, Some(a));
    ctx.request_repaint();
    dy
}

/// After an area was shown: a middle press inside it starts autoscroll, if it scrolls and nothing covers it (row overlays
/// and tooltips of the list don't count, see scroll_route.rs) and no inner list has started already.
fn maybe_start(ui: &Ui, id: Id, inner: Rect, content_h: f32) {
    let ctx = ui.ctx();
    if active(ctx) || content_h <= inner.height() + 0.5 { return; }
    // The press event's own position (egui forgets press_origin when the release comes in the same frame).
    let Some(p) = ctx.input(|i| i.events.iter().find_map(|e| match e { Event::PointerButton { pos, button: PointerButton::Middle, pressed: true, .. } => Some(*pos), _ => None })) else { return };
    if !ui.clip_rect().intersect(inner).contains(p) { return; }
    if !super::scroll_route::layer_reaches(ui, p) { return; }
    let now = std::time::Instant::now();
    let released = ctx.input(|i| i.pointer.button_released(PointerButton::Middle)); // a quick click in one frame
    put(ctx, Some(Auto { area: id, origin: p, pointer: ctx.input(|i| i.pointer.hover_pos()).unwrap_or(p), held: !released, moved: false, pressed_at: now, last: now, pending: 0.0, unseen: 0, swallow: None }));
    ctx.request_repaint();
}

/// End of the frame: the anchor icon at the press point and the scroll cursor.
pub(crate) fn paint(ctx: &egui::Context) {
    let Some(a) = get(ctx) else { return };
    let dy = a.pointer.y - a.origin.y;
    let dir = if dy.abs() <= DEAD { 0.0 } else { dy.signum() };
    ctx.set_cursor_icon(match dir as i32 { -1 => CursorIcon::ResizeNorth, 1 => CursorIcon::ResizeSouth, _ => CursorIcon::AllScroll });
    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("autoscroll-anchor")));
    draw_anchor(&p, a.origin, dir);
}

/// The anchor icon: a light disc with a dark rim, a centre dot and a chevron above and below; the chevron of the current
/// direction is darker.
fn draw_anchor(p: &egui::Painter, c: Pos2, dir: f32) {
    let r = 14.0;
    p.circle_filled(c + Vec2::new(0.0, 1.0), r + 1.0, Color32::from_black_alpha(60)); // soft shadow
    p.circle(c, r, Color32::from_rgba_unmultiplied(248, 248, 248, 240), Stroke::new(1.0, Color32::from_gray(70)));
    p.circle_filled(c, 2.0, Color32::from_gray(60));
    let chev = |sign: f32, on: bool| {
        let col = if on { Color32::from_gray(20) } else { Color32::from_gray(110) };
        let tip = c + Vec2::new(0.0, sign * 9.5);
        let base = c.y + sign * 5.0;
        p.add(egui::Shape::convex_polygon(vec![tip, Pos2::new(c.x + 4.5, base), Pos2::new(c.x - 4.5, base)], col, Stroke::NONE));
    };
    chev(-1.0, dir < 0.0);
    chev(1.0, dir > 0.0);
}

/// Key (under the area's id) of the area's last maximum offset (content height - view height).
const K_MAX: &str = "vscroll-max";

/// The offset autoscroll moves to: `base + dy`, kept within [0, the area's maximum] (unknown maximum: only >= 0).
pub(crate) fn autoscroll_target(base: f32, dy: f32, max: Option<f32>) -> f32 {
    let y = (base + dy).max(0.0);
    match max { Some(m) => y.min(m.max(0.0)), None => y }
}

fn remember_max<R>(ctx: &egui::Context, id: Id, out: &egui::scroll_area::ScrollAreaOutput<R>) {
    let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
    ctx.data_mut(|d| d.insert_temp(id.with(K_MAX), max));
}

/// The scroll bar with arrow buttons (the message list, the space list, the GIF picker): a 16 pt track (ink @ 0.11) with an
/// 18 pt chevron button at each end and a 7.6 pt pill thumb.
pub(crate) const BAR_W: f32 = 16.0;
const BAR_THUMB_W: f32 = 7.6;
const BAR_ARROW: f32 = 18.0;
const BAR_MIN_THUMB: f32 = 24.0;
/// One click on an arrow button scrolls this far (pt).
pub(crate) const BAR_STEP: f32 = 48.0;

/// Draws the arrow scroll bar in `track` (the full-height strip at the list's right edge) for a list scrolled to `off` of `max`
/// with a `view_h` high viewport, and handles it: thumb drag, a track click (one page), the arrow buttons (`BAR_STEP`).
/// `dark` = drawn on a dark background (white ink); `round_top` also rounds the track's top (the message list's track runs
/// under the header and is rounded only at the bottom). Nothing is drawn while the list does not scroll.
/// Returns the new offset when the user moved it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn arrow_bar(ui: &mut Ui, id: Id, track: Rect, off: f32, max: f32, view_h: f32, dark: bool, round_top: bool) -> Option<f32> {
    if max < 0.5 { return None; }
    let (up, down) = (Rect::from_min_size(track.min, Vec2::new(track.width(), BAR_ARROW)), Rect::from_min_max(Pos2::new(track.left(), track.bottom() - BAR_ARROW), track.max));
    let inner = Rect::from_min_max(Pos2::new(track.left(), up.bottom()), Pos2::new(track.right(), down.top()));
    let len = (inner.height() * view_h / (view_h + max)).clamp(BAR_MIN_THUMB.min(inner.height()), inner.height());
    let ty = inner.top() + (inner.height() - len) * (off / max).clamp(0.0, 1.0);
    let thumb = Rect::from_min_size(Pos2::new(track.center().x - BAR_THUMB_W / 2.0, ty), Vec2::new(BAR_THUMB_W, len));
    let mut to = None;
    let t_resp = ui.interact(track, id.with("sb-track"), egui::Sense::click());
    let th_resp = ui.interact(thumb.expand2(Vec2::new(4.0, 0.0)), id.with("sb-thumb"), egui::Sense::drag());
    let (u_resp, d_resp) = (ui.interact(up, id.with("sb-up"), egui::Sense::click()), ui.interact(down, id.with("sb-down"), egui::Sense::click()));
    if th_resp.dragged() {
        let k = max / (inner.height() - len).max(1.0);
        to = Some((off + th_resp.drag_delta().y * k).clamp(0.0, max));
    } else if u_resp.clicked() { to = Some((off - BAR_STEP).max(0.0)); }
    else if d_resp.clicked() { to = Some((off + BAR_STEP).min(max)); }
    else if t_resp.clicked() && !th_resp.hovered() {
        if let Some(p) = t_resp.interact_pointer_pos() { to = Some((off + if p.y < thumb.top() { -view_h } else { view_h } * 0.9).clamp(0.0, max)); }
    }
    let fg = if dark { Color32::WHITE } else { Color32::BLACK };
    let p = ui.painter();
    let r = (track.width() / 2.0) as u8;
    p.rect_filled(track, egui::CornerRadius { nw: if round_top { r } else { 0 }, ne: if round_top { r } else { 0 }, sw: r, se: r }, crate::alpha(fg, 0.11));
    let hot = th_resp.hovered() || th_resp.dragged();
    // Thumb under the pointer: #808080 dark, #797979 light; an accent-theme plugin may colour it (ui/theme/accents.rs).
    let hot_c = crate::ui::theme::over("scrollbar-thumb-background:hovered").unwrap_or(if dark { crate::hex(0xA0A0A0) } else { crate::hex(0x5A5A5A) });
    p.rect_filled(thumb, BAR_THUMB_W / 2.0, if hot { hot_c } else if dark { crate::hex(0x808080) } else { crate::hex(0x797979) });
    let chev = |r: Rect, dir: f32, on: bool| {
        let (cx, cy) = (r.center().x, r.center().y);
        let s = Stroke::new(1.3, crate::alpha(fg, if on { 0.95 } else { 0.55 }));
        p.line_segment([Pos2::new(cx - 3.5, cy + dir * 1.8), Pos2::new(cx, cy - dir * 1.8)], s);
        p.line_segment([Pos2::new(cx, cy - dir * 1.8), Pos2::new(cx + 3.5, cy + dir * 1.8)], s);
    };
    chev(up, 1.0, u_resp.hovered());
    chev(down, -1.0, d_resp.hovered());
    if to.is_some() { ui.ctx().request_repaint(); }
    to
}

/// Key (under the area's id) of an offset set through the arrow bar, applied on the next frame.
const K_BAR_SET: &str = "vscroll-bar-set";

/// `widgets::vscroll()`: egui's vertical `ScrollArea` (no drag-to-scroll) with middle-click autoscroll. Forwards the builder
/// calls the app uses. `.arrow_bar(dark)` replaces egui's bar by [`arrow_bar`] (the caller leaves `BAR_W` free at the right
/// of the content).
pub(crate) struct VScroll { area: egui::ScrollArea, salt: Option<Id>, offset: Option<f32>, bar: Option<bool> }

impl VScroll {
    pub(crate) fn new() -> Self { Self { area: egui::ScrollArea::vertical().drag_to_scroll(false), salt: None, offset: None, bar: None } }
    /// The scroll bar with arrow buttons (`dark` = white ink on a dark background), drawn over the right `BAR_W`.
    pub(crate) fn arrow_bar(mut self, dark: bool) -> Self { self.bar = Some(dark); self.area = self.area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden); self }
    pub(crate) fn id_salt(mut self, salt: impl std::hash::Hash) -> Self { self.salt = Some(Id::new(&salt)); self.area = self.area.id_salt(salt); self }
    pub(crate) fn auto_shrink(mut self, a: impl Into<egui::Vec2b>) -> Self { self.area = self.area.auto_shrink(a); self }
    pub(crate) fn max_height(mut self, h: f32) -> Self { self.area = self.area.max_height(h); self }
    pub(crate) fn scroll_bar_visibility(mut self, v: egui::scroll_area::ScrollBarVisibility) -> Self { self.area = self.area.scroll_bar_visibility(v); self }
    pub(crate) fn vertical_scroll_offset(mut self, y: f32) -> Self { self.offset = Some(y); self }

    /// The id egui gives this area in `ui` (ScrollArea::begin: `ui.make_persistent_id(id_salt or "scroll_area")`).
    fn id(&self, ui: &Ui) -> Id { ui.make_persistent_id(self.salt.unwrap_or_else(|| Id::new("scroll_area"))) }

    fn prepare(self, ui: &Ui) -> (egui::ScrollArea, Id, Option<bool>) {
        let id = self.id(ui);
        let dy = take(ui.ctx(), id);
        let bar_set = if self.bar.is_some() { ui.ctx().data_mut(|d| d.remove_temp::<f32>(id.with(K_BAR_SET))) } else { None };
        let offset = if let Some(y) = bar_set { Some(y) } else if dy != 0.0 {
            let base = self.offset.unwrap_or_else(|| egui::scroll_area::State::load(ui.ctx(), id).map_or(0.0, |s| s.offset.y));
            // Clamped here: egui paints with the offset it is given and clamps it to the content only after painting, so an offset past the end would show the content pushed up with a gap below, snapping back every frame.
            Some(autoscroll_target(base, dy, ui.ctx().data(|d| d.get_temp::<f32>(id.with(K_MAX)))))
        } else { self.offset };
        (match offset { Some(y) => self.area.vertical_scroll_offset(y), None => self.area }, id, self.bar)
    }

    pub(crate) fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
        let (area, id, bar) = self.prepare(ui);
        let out = area.show(ui, add);
        finish(ui, id, bar, &out);
        out
    }

    pub(crate) fn show_viewport<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui, Rect) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
        let (area, id, bar) = self.prepare(ui);
        let out = area.show_viewport(ui, add);
        finish(ui, id, bar, &out);
        out
    }
}

fn finish<R>(ui: &mut Ui, id: Id, bar: Option<bool>, out: &egui::scroll_area::ScrollAreaOutput<R>) {
    remember_max(ui.ctx(), id, out);
    maybe_start(ui, id, out.inner_rect, out.content_size.y);
    if let Some(dark) = bar {
        let r = out.inner_rect;
        let track = Rect::from_min_max(Pos2::new(r.right() - BAR_W, r.top()), r.max);
        let max = (out.content_size.y - r.height()).max(0.0);
        if let Some(y) = arrow_bar(ui, id, track, out.state.offset.y, max, r.height(), dark, true) {
            ui.ctx().data_mut(|d| d.insert_temp(id.with(K_BAR_SET), y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(held: bool) -> Auto {
        let t = std::time::Instant::now();
        Auto { area: Id::new("a"), origin: Pos2::new(100.0, 100.0), pointer: Pos2::new(100.0, 100.0), held, moved: false, pressed_at: t, last: t, pending: 0.0, unseen: 0, swallow: None }
    }
    fn btn(b: PointerButton, pressed: bool) -> Event { Event::PointerButton { pos: Pos2::new(1.0, 1.0), button: b, pressed, modifiers: Default::default() } }

    #[test]
    fn autoscroll_stops_dead_at_both_ends() {
        assert_eq!(autoscroll_target(1828.0, 21.0, Some(1828.0)), 1828.0); // at the bottom, pointer below: stays
        assert_eq!(autoscroll_target(1820.0, 21.0, Some(1828.0)), 1828.0); // reaches the bottom exactly
        assert_eq!(autoscroll_target(5.0, -21.0, Some(1828.0)), 0.0); // and the top
        assert_eq!(autoscroll_target(100.0, 12.0, Some(1828.0)), 112.0);
        assert_eq!(autoscroll_target(100.0, 12.0, None), 112.0); // first frame, maximum not known yet
        assert_eq!(autoscroll_target(0.0, 30.0, Some(-5.0)), 0.0); // content shorter than the view
    }

    #[test]
    fn speed_has_a_dead_zone_and_grows_with_distance() {
        assert_eq!(speed(0.0), 0.0);
        assert_eq!(speed(9.9), 0.0);
        assert_eq!(speed(-10.0), 0.0);
        assert!(speed(30.0) > 0.0 && speed(-30.0) < 0.0);
        assert_eq!(speed(30.0), -speed(-30.0));
        assert!(speed(100.0) > 3.0 * speed(30.0)); // faster than linear
        assert!(speed(1.0e6) <= 6000.0);
        // Frame-rate independent: 60 frames of 1/60 s move as far as 30 frames of 1/30 s.
        let d60: f32 = (0..60).map(|_| speed(80.0) / 60.0).sum();
        let d30: f32 = (0..30).map(|_| speed(80.0) / 30.0).sum();
        assert!((d60 - d30).abs() < 0.01);
    }

    #[test]
    fn a_quick_middle_click_stays_and_a_click_ends_it() {
        let mut a = auto(true);
        let mut ev = vec![btn(PointerButton::Middle, false)];
        assert!(!route_events(&mut a, &mut ev, 0.1)); // released quickly without moving: sticky mode
        assert!(!a.held && ev.is_empty());
        let mut ev = vec![Event::PointerMoved(Pos2::new(100.0, 180.0))];
        assert!(!route_events(&mut a, &mut ev, 1.0));
        assert_eq!(ev.len(), 1); // moves stay visible to egui
        let mut ev = vec![btn(PointerButton::Primary, true)];
        assert!(route_events(&mut a, &mut ev, 2.0)); // a click ends it and is swallowed
        assert!(ev.is_empty());
        assert_eq!(a.swallow, Some(PointerButton::Primary));
    }

    #[test]
    fn press_move_release_ends_it_and_esc_ends_it() {
        let mut a = auto(true);
        let mut ev = vec![Event::PointerMoved(Pos2::new(100.0, 150.0)), btn(PointerButton::Middle, false)];
        assert!(route_events(&mut a, &mut ev, 0.2)); // Chrome's drag mode
        let mut a = auto(true);
        assert!(route_events(&mut a, &mut vec![btn(PointerButton::Middle, false)], 0.8)); // held long
        let mut a = auto(false);
        let mut ev = vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }];
        assert!(route_events(&mut a, &mut ev, 3.0));
        assert!(ev.is_empty()); // Esc doesn't also close something else
        let mut a = auto(false);
        assert!(route_events(&mut a, &mut vec![btn(PointerButton::Middle, true)], 3.0)); // a second middle click
    }
}
