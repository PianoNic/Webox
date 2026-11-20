//! Decorations of design plugins (plugin-api 1.4 `Decor`): background washes for the window, the list card and the content
//! card, and the host-drawn decorations a plugin switches on (`deco` bits: a flag band on the rail, rings around avatars,
//! gradient outlines, scrollbar thumbs, toggle tracks, ...). The plugin only declares them; everything is painted here, with
//! the colours it sends. Set once per frame by apply_theme; every drawing site asks `on(bit)` and does nothing when no design
//! plugin is active.
use collabhost_plugin_api::{deco, Decor, Wash as ApiWash};
use eframe::egui::{epaint::{Mesh, PathShape, PathStroke, Vertex, WHITE_UV}, pos2, vec2, Color32, Painter, Pos2, Rect, Shape};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct Wash { pub stops: Vec<Color32>, pub angle: f32, pub opacity: f32, pub border: f32 }
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct D { pub window: Option<Wash>, pub list: Option<Wash>, pub content: Option<Wash>, pub stripes: Vec<Color32>, pub flags: u64 }

thread_local! {
    static CUR: RefCell<Option<Rc<D>>> = const { RefCell::new(None) };
    static SELF_NAME: RefCell<String> = const { RefCell::new(String::new()) };
}

fn wash(w: &ApiWash) -> Option<Wash> {
    let n = (w.stops_len as usize).min(w.stops.len());
    let stops: Vec<Color32> = w.stops[..n].iter().filter_map(|v| crate::rgba(*v)).collect();
    let (opacity, border) = (if w.opacity.is_finite() { w.opacity.clamp(0.0, 1.0) } else { 0.0 }, if w.border.is_finite() { w.border.clamp(0.0, 6.0) } else { 0.0 });
    (!stops.is_empty() && (opacity > 0.0 || border > 0.0)).then(|| Wash { stops, angle: if w.angle.is_finite() { w.angle } else { 0.0 }, opacity, border })
}

/// The host's form of a plugin's `Decor`.
pub(crate) fn from_api(d: &Decor) -> D {
    let n = (d.stripes_len as usize).min(d.stripes.len());
    D { window: wash(&d.window), list: wash(&d.list_card), content: wash(&d.content_card),
        stripes: d.stripes[..n].iter().filter_map(|v| crate::rgba(*v)).collect(), flags: d.flags }
}

/// The active decorations (None = no design plugin with decorations). `self_name`: the signed-in person, for "own avatar".
pub(crate) fn set(d: Option<D>, self_name: &str) {
    CUR.with(|c| { let mut c = c.borrow_mut(); if c.as_deref() != d.as_ref() { *c = d.map(Rc::new) } });
    SELF_NAME.with(|s| if *s.borrow() != self_name { *s.borrow_mut() = self_name.to_string() });
}
pub(crate) fn get() -> Option<Rc<D>> { CUR.with(|c| c.borrow().clone()) }
/// Whether a decoration is switched on (and there are colours to draw it in).
pub(crate) fn on(bit: u64) -> bool { CUR.with(|c| c.borrow().as_ref().is_some_and(|d| d.flags & bit != 0 && !d.stripes.is_empty())) }
pub(crate) fn stripes() -> Vec<Color32> { CUR.with(|c| c.borrow().as_ref().map(|d| d.stripes.clone()).unwrap_or_default()) }
pub(crate) fn is_self(name: &str) -> bool { SELF_NAME.with(|s| !name.is_empty() && *s.borrow() == name) }

/// The colour at `k` (0..1) of evenly spread `stops`.
pub(crate) fn at(stops: &[Color32], k: f32) -> Color32 {
    match stops.len() { 0 => Color32::TRANSPARENT, 1 => stops[0], n => {
        let x = k.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (x.floor() as usize).min(n - 2);
        stops[i].lerp_to_gamma(stops[i + 1], x - i as f32)
    } }
}

/// Where `p` lies (0..1) along `angle` across `r`.
fn along(r: Rect, angle: f32, p: Pos2) -> f32 {
    let d = vec2(angle.to_radians().cos(), angle.to_radians().sin());
    let proj = |q: Pos2| (q - r.min).dot(d);
    let ks = [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()].map(proj);
    let (lo, hi) = ks.iter().fold((f32::MAX, f32::MIN), |(a, b), k| (a.min(*k), b.max(*k)));
    ((proj(p) - lo) / (hi - lo).max(1e-3)).clamp(0.0, 1.0)
}

/// Moves a point of `r` that lies outside its rounded corners onto the corner's arc.
fn round_in(r: Rect, radius: f32, p: Pos2) -> Pos2 {
    if radius <= 0.0 { return p; }
    let cx = if p.x < r.left() + radius { r.left() + radius } else if p.x > r.right() - radius { r.right() - radius } else { return p };
    let cy = if p.y < r.top() + radius { r.top() + radius } else if p.y > r.bottom() - radius { r.bottom() - radius } else { return p };
    let v = p - pos2(cx, cy);
    if v.length() <= radius { p } else { pos2(cx, cy) + v.normalized() * radius }
}

/// Fills `r` (rounded by `radius`) with the gradient of `stops` along `angle`, at `opacity`.
pub(crate) fn fill(p: &Painter, r: Rect, radius: f32, stops: &[Color32], angle: f32, opacity: f32) {
    if stops.is_empty() || opacity <= 0.0 || r.width() < 1.0 || r.height() < 1.0 { return; }
    let (cols, rows) = (((r.width() / 24.0).ceil() as usize).clamp(4, 48), ((r.height() / 24.0).ceil() as usize).clamp(2, 24));
    let mut m = Mesh::default();
    for j in 0..=rows {
        for i in 0..=cols {
            let q = round_in(r, radius, pos2(r.left() + r.width() * i as f32 / cols as f32, r.top() + r.height() * j as f32 / rows as f32));
            m.vertices.push(Vertex { pos: q, uv: WHITE_UV, color: crate::alpha(at(stops, along(r, angle, q)), opacity) });
        }
    }
    let w = (cols + 1) as u32;
    for j in 0..rows as u32 { for i in 0..cols as u32 { let a = j * w + i; m.add_triangle(a, a + 1, a + w); m.add_triangle(a + 1, a + w + 1, a + w); } }
    p.add(Shape::mesh(m));
}

fn rounded_path(r: Rect, radius: f32) -> Vec<Pos2> {
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0);
    let mut pts = vec![];
    for (c, a0) in [(pos2(r.right() - rad, r.top() + rad), -90.0f32), (pos2(r.right() - rad, r.bottom() - rad), 0.0), (pos2(r.left() + rad, r.bottom() - rad), 90.0), (pos2(r.left() + rad, r.top() + rad), 180.0)] {
        for s in 0..=6 { let a = (a0 + 90.0 * s as f32 / 6.0).to_radians(); pts.push(c + vec2(a.cos(), a.sin()) * rad); }
    }
    pts
}

/// A gradient outline of the rounded rect `r`.
pub(crate) fn outline(p: &Painter, r: Rect, radius: f32, width: f32, stops: &[Color32], angle: f32) {
    if stops.is_empty() || width <= 0.0 { return; }
    let st = stops.to_vec();
    p.add(PathShape::closed_line(rounded_path(r.shrink(width / 2.0), radius), PathStroke::new_uv(width, move |b, q| at(&st, along(b, angle, q)))));
}

/// A ring in the stripe colours around a circle (`c`, radius to the ring's middle).
pub(crate) fn ring(p: &Painter, c: Pos2, radius: f32, width: f32, stops: &[Color32]) {
    if stops.is_empty() { return; }
    let pts: Vec<Pos2> = (0..48).map(|i| { let a = (i as f32 / 48.0 * 360.0 - 90.0).to_radians(); c + vec2(a.cos(), a.sin()) * radius }).collect();
    let st = stops.to_vec();
    p.add(PathShape::closed_line(pts, PathStroke::new_uv(width, move |b, q| {
        let a = (q - b.center()).angle().to_degrees() + 90.0; // 0 at the top, clockwise
        at(&st, a.rem_euclid(360.0) / 360.0)
    })));
}

/// The stripes stacked from top to bottom, like a flag.
pub(crate) fn flag(p: &Painter, r: Rect, stops: &[Color32]) {
    let n = stops.len().max(1) as f32;
    for (i, c) in stops.iter().enumerate() {
        let y0 = r.top() + r.height() * i as f32 / n;
        p.rect_filled(Rect::from_min_max(pos2(r.left(), y0), pos2(r.right(), r.top() + r.height() * (i + 1) as f32 / n)), 0.0, *c);
    }
}

/// A surface's wash: the gradient over it at its opacity and, if it has one, the gradient outline.
pub(crate) fn paint_wash(p: &Painter, r: Rect, radius: f32, w: &Wash) {
    fill(p, r, radius, &w.stops, w.angle, w.opacity);
    if w.border > 0.0 { outline(p, r, radius, w.border, &w.stops, w.angle); }
}

/// After a card's own gradient: the list card's (`list`) or content card's wash, if a design plugin gives one.
pub(crate) fn card(p: &Painter, r: Rect, radius: f32, list: bool) {
    if let Some(d) = get() { if let Some(w) = if list { &d.list } else { &d.content } { paint_wash(p, r, radius, w) } }
}

/// A progress bar (`frac` 0..1, None = waiting) `width` wide: in the stripe colours with PROGRESS on, else egui's own.
pub(crate) fn progress(ui: &mut eframe::egui::Ui, frac: Option<f32>, width: f32) {
    if !on(deco::PROGRESS) { ui.add(eframe::egui::ProgressBar::new(frac.unwrap_or(0.0)).desired_width(width).show_percentage()); return; }
    let (r, _) = ui.allocate_exact_size(vec2(width, 10.0), eframe::egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 5.0, ui.visuals().extreme_bg_color);
    let f = frac.unwrap_or(0.0).clamp(0.0, 1.0);
    if f > 0.0 { fill(p, Rect::from_min_size(r.min, vec2((r.width() * f).max(10.0), r.height())), 5.0, &stripes(), 0.0, 1.0); }
}

/// After the rail and the header: the flag band along the rail's left edge (RAIL_STRIPE) and the gradient under the header
/// (HEADER_STRIPE), on the background layer under everything else.
pub(crate) fn chrome(ctx: &eframe::egui::Context) {
    use eframe::egui::{containers::panel::PanelState, Id, LayerId};
    let (rail, header) = (on(deco::RAIL_STRIPE), on(deco::HEADER_STRIPE));
    if !rail && !header { return; }
    let (p, s) = (ctx.layer_painter(LayerId::background()), stripes());
    if rail { if let Some(st) = PanelState::load(ctx, Id::new("rail")) { flag(&p, Rect::from_min_size(st.rect.left_top(), vec2(4.0, st.rect.height())), &s) } }
    if header { if let Some(st) = PanelState::load(ctx, Id::new("top")) { fill(&p, Rect::from_min_max(pos2(st.rect.left(), st.rect.bottom() - 2.0), st.rect.right_bottom()), 0.0, &s, 0.0, 1.0) } }
}

/// The colour of the `i`-th item of a cycling decoration (BADGE_CYCLE).
pub(crate) fn cycle(i: usize) -> Option<Color32> { let s = stripes(); (!s.is_empty()).then(|| s[i % s.len()]) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gradient_maths() {
        let s = [Color32::RED, Color32::BLUE];
        assert_eq!(at(&s, 0.0), Color32::RED);
        assert_eq!(at(&s, 1.0), Color32::BLUE);
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 50.0));
        assert!((along(r, 0.0, pos2(50.0, 10.0)) - 0.5).abs() < 1e-4);
        assert!((along(r, 90.0, pos2(0.0, 50.0)) - 1.0).abs() < 1e-4);
        assert_eq!(round_in(r, 10.0, pos2(0.0, 0.0)).distance(pos2(10.0, 10.0)).round(), 10.0);
        assert_eq!(round_in(r, 10.0, pos2(50.0, 25.0)), pos2(50.0, 25.0));
    }
    #[test]
    fn api_decor_is_sanitised() {
        let mut d = Decor { stripes_len: 99, flags: deco::RAIL_STRIPE, ..Default::default() };
        d.stripes[0] = 0xFF0000FF;
        d.window = ApiWash { stops_len: 1, opacity: f32::NAN, border: 0.0, ..Default::default() };
        let h = from_api(&d);
        assert_eq!(h.stripes.len(), 1, "0 entries (= keep) are dropped, the length is clamped");
        assert!(h.window.is_none(), "a wash with no opacity and no border draws nothing");
        set(Some(h), "Alex Example");
        assert!(on(deco::RAIL_STRIPE) && !on(deco::TAB_PILLS) && is_self("Alex Example"));
        set(None, "");
        assert!(!on(deco::RAIL_STRIPE));
    }
}
