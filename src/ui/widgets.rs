#![allow(clippy::too_many_arguments)]
use crate::*;

pub(crate) fn initials(name: &str, space: bool) -> String {
    let mut it = name.split_whitespace().filter_map(|w| w.chars().find(|c| c.is_alphanumeric()));
    let first = it.next();
    let s: String = if space { first.into_iter().collect() } else { first.into_iter().chain(it.last()).collect() };
    if s.is_empty() { "#".into() } else { s.to_uppercase() }
}

/// Round avatar with initials and, for people, the presence badge.

/// A round photo (texture from `App::photo(..)`) filling the circle, with an antialiased edge (`fill_convex_aa`).
fn circle_photo(p: &egui::Painter, c: egui::Pos2, radius: f32, tex: egui::TextureId) {
    const N: usize = 48;
    let pts: Vec<egui::Pos2> = (0..N).map(|i| c + Vec2::angled(i as f32 / N as f32 * std::f32::consts::TAU) * radius).collect();
    fill_convex_aa(p, &pts, tex, 1, |q| (Color32::WHITE, pos2(0.5 + (q.x - c.x) / (2.0 * radius), 0.5 + (q.y - c.y) / (2.0 * radius))));
}

/// Fills the convex polygon `pts` (its perimeter in order, either direction) with an antialiased edge, the way egui feathers
/// its own shapes: the fill ends half a device pixel inside the edge and fades to transparent half a pixel outside it.
/// Hand-built meshes (`Shape::mesh`) skip egui's feathering, so their curved edges showed hard pixel steps.
/// `at(point)` gives the colour (premultiplied; multiplied with the texture) and the texture coordinate there. `rings` > 1
/// subdivides the fan into concentric rings, for colours that are not linear across the shape (multi-stop gradients).
pub(crate) fn fill_convex_aa(p: &egui::Painter, pts: &[egui::Pos2], tex: egui::TextureId, rings: u32, at: impl Fn(egui::Pos2) -> (Color32, egui::Pos2)) {
    use egui::epaint::Vertex;
    let mut v: Vec<egui::Pos2> = Vec::with_capacity(pts.len());
    for &q in pts { if v.last().is_none_or(|l: &egui::Pos2| l.distance(q) > 1e-3) { v.push(q) } }
    while v.len() > 2 && v[0].distance(v[v.len() - 1]) <= 1e-3 { v.pop(); }
    let n = v.len();
    if n < 3 { return; }
    let area: f32 = (0..n).map(|i| { let (a, b) = (v[i], v[(i + 1) % n]); a.x * b.y - b.x * a.y }).sum();
    if area.abs() < 1e-6 { return; }
    let sign = area.signum();
    let edge_n = |i: usize| { let d = (v[(i + 1) % n] - v[i]).normalized(); Vec2::new(d.y, -d.x) * sign }; // outward
    let half = 0.5 / p.ctx().pixels_per_point();
    let c = v.iter().fold(Vec2::ZERO, |s, q| s + q.to_vec2()) / n as f32;
    let c = c.to_pos2();
    let rings = rings.max(1);
    let mut m = egui::Mesh::with_texture(tex);
    let vert = |m: &mut egui::Mesh, q: egui::Pos2, transparent: bool| {
        let (col, uv) = at(q);
        m.vertices.push(Vertex { pos: q, uv, color: if transparent { Color32::TRANSPARENT } else { col } });
    };
    vert(&mut m, c, false);
    // Per perimeter point: `rings` points from the centre outwards (the last one half a pixel inside the edge), then the
    // transparent one half a pixel outside it.
    let stride = rings + 1;
    for i in 0..n {
        let nv = edge_n((i + n - 1) % n) + edge_n(i);
        let nv = if nv.length_sq() > 1e-6 { nv.normalized() * (2.0 / nv.length()).min(2.0) } else { edge_n(i) }; // miter, capped
        let inner = v[i] - nv * half;
        for k in 1..=rings { vert(&mut m, c + (inner - c) * (k as f32 / rings as f32), false); }
        vert(&mut m, v[i] + nv * half, true);
    }
    let idx = |i: usize, k: u32| 1 + i as u32 * stride + k; // k = 0..rings-1 fill rings, k = rings: outer
    for i in 0..n {
        let j = (i + 1) % n;
        m.add_triangle(0, idx(i, 0), idx(j, 0));
        for k in 0..rings { m.add_triangle(idx(i, k), idx(i, k + 1), idx(j, k)); m.add_triangle(idx(j, k), idx(i, k + 1), idx(j, k + 1)); }
    }
    p.add(egui::Shape::mesh(m));
}

/// The perimeter of `r` with per-corner radii (top-left, bottom-left, bottom-right, top-right), clockwise from the top-left
/// corner's start: arcs with enough points for a smooth curve at `ppp`, straight edges cut into pieces of at most `max_seg`.
pub(crate) fn rounded_rect_points(r: Rect, radii: [f32; 4], ppp: f32, max_seg: f32) -> Vec<egui::Pos2> {
    let max = r.width().min(r.height()) / 2.0;
    // (corner, direction from the arc centre to the corner, start angle in screen degrees), clockwise on screen.
    let corners = [(r.left_top(), Vec2::new(-1.0, -1.0), 180.0f32), (r.right_top(), Vec2::new(1.0, -1.0), 270.0), (r.right_bottom(), Vec2::new(1.0, 1.0), 0.0), (r.left_bottom(), Vec2::new(-1.0, 1.0), 90.0)];
    let rad = [radii[0], radii[3], radii[2], radii[1]];
    let mut pts: Vec<egui::Pos2> = Vec::new();
    let push_line = |pts: &mut Vec<egui::Pos2>, to: egui::Pos2| {
        if let Some(&from) = pts.last() {
            let k = ((from.distance(to) / max_seg.max(1.0)).ceil() as usize).max(1);
            for s in 1..k { pts.push(from + (to - from) * (s as f32 / k as f32)); }
        }
        pts.push(to);
    };
    for ((corner, dir, a0), rd) in corners.into_iter().zip(rad) {
        let rd = rd.clamp(0.0, max);
        let centre = corner - dir * rd;
        // Segments so the chord stays within ~0.1 device pixel of the arc.
        let steps = if rd * ppp < 0.5 { 0 } else { ((rd * ppp).sqrt() * 1.6).ceil().clamp(2.0, 48.0) as usize };
        for s in 0..=steps {
            let a = (a0 + 90.0 * s as f32 / steps.max(1) as f32).to_radians();
            let q = if steps == 0 { corner } else { centre + Vec2::new(a.cos(), a.sin()) * rd };
            push_line(&mut pts, q);
            if steps == 0 { break }
        }
    }
    if let Some(&first) = pts.first() { push_line(&mut pts, first); pts.pop(); }
    pts
}

/// The avatar's presence badge for a status (as `cloud::people` files it): the icon (empty = a plain filled dot) and its colour.
///
///   Active → unread-filled (a dot), green · Meeting → camera-presence-filled · CalendarMeeting → meetings-presence-badge-filled
///   · Call → handset-filled · Busy → busy-presence-regular (Meeting, CalendarMeeting, Call and Busy are orange)
///   · Dnd → dnd-presence-badge-filled · Sharing → share-screen-badge-filled (Dnd and Sharing are red)
///   · QuietHours → quiet-hours-presence-filled · Inactive → recents-presence-badge-filled · OOO → pto-presence-filled (grey group)
///   · Hold → pause-filled · JoinMeetingByMobile → phone-filled · JoinMeetingByPersonalDevice → generic-device-video-filled (grey)
///   · Away → away-calling-presence-filled (the presence services never report it here, so it has no entry).
/// The status names follow the presence model's names (Meeting, Presenting, CalendarItem, OutOfOffice, Quiet …).
pub(crate) fn presence_badge(status: &str) -> Option<(&'static str, Color32)> {
    let (green, orange, red, grey) = (hex(0x27A17A), hex(0xF2990A), hex(0xF55868), hex(0x949494));
    Some(match status {
        "active" => ("", green),
        "meeting" => ("camera-presence-filled", orange),
        "calendar" => ("meetings-presence-badge-filled", orange),
        "call" => ("handset-filled", orange),
        "busy" => ("busy-presence-regular", orange),
        "DoNotDisturb" => ("dnd-presence-badge-filled", red),
        "presenting" => ("share-screen-badge-filled", red),
        "quiet" => ("quiet-hours-presence-filled", grey),
        "inactive" => ("recents-presence-badge-filled", grey),
        "OutOfOffice" => ("pto-presence-filled", grey),
        "hold" => ("pause-filled", grey),
        "mobile" => ("phone-filled", grey),
        "device" => ("generic-device-video-filled", grey),
        _ => return None,
    })
}

/// Like `avatar`, but draws `photo` (the person's profile picture) instead of the initials when there is one.
///
/// One texture per (picture, size in physical pixels, badge cut-out, background), rendered by `photos::round` (CatmullRom
/// resample in linear light, antialiased circle in the texture) and placed 1:1 on the pixel grid, so at 125 % a 32 pt avatar
/// is a crisp 40 px picture rather than a stretched 32 px one. A presence badge cuts a transparent ring (`BADGE_RING` pt) out
/// of the picture, so whatever is behind the avatar shows through, and its dot is an antialiased disc of its own on the same
/// grid. Until a new texture is rendered (a few per frame) the scaled mesh is drawn.
pub(crate) fn avatar_photo(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, name: &str, size: f32, space: bool, bg: Option<Color32>, status: Option<&str>, photo: Option<egui::TextureId>) {
    avatar_photo_with(ui, ic, t, name, size, space, bg, status, photo, BadgeGeom::DEFAULT)
}

/// Where an avatar's presence badge sits: centre as a fraction of the avatar's box from its top-left, diameter as a fraction
/// of the size, the transparent ring cut around it (pt), and whether it ignores Settings "Verfügbarkeit und Status anzeigen"
/// (the header's own badge always shows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BadgeGeom { pub cx: f32, pub cy: f32, pub d: f32, pub ring: f32, pub always: bool }
impl BadgeGeom {
    /// Lists, messages, cards: centre 0.13 × size in from the bottom-right corner, diameter 0.34 × size, 2 pt ring.
    pub const DEFAULT: BadgeGeom = BadgeGeom { cx: 0.87, cy: 0.87, d: 0.34, ring: 2.0, always: false };
    /// The application header's own avatar: dot 11.6 pt centred 0.356 / 0.28 of the diameter right / below the centre,
    /// 1.2 pt ring (see chrome/mod.rs).
    pub const HEADER: BadgeGeom = BadgeGeom { cx: 0.856, cy: 0.78, d: 11.6 / 32.0, ring: 1.2, always: true };
}

/// `avatar_photo` with its presence badge at `geom`.
pub(crate) fn avatar_photo_with(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, name: &str, size: f32, space: bool, bg: Option<Color32>, status: Option<&str>, photo: Option<egui::TextureId>, geom: BadgeGeom) {
    use crate::assets::photos::round;
    let (r, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let ctx = ui.ctx().clone();
    let g = AvatarGrid::new(r.min, size, ctx.pixels_per_point());
    let bg = bg.unwrap_or(t.avatar);
    // Settings "Verfügbarkeit und Status anzeigen" off: no presence badges (App::presence_shown sets the flag each frame).
    let status = if geom.always || PRESENCE_SHOWN.load(std::sync::atomic::Ordering::Relaxed) { status } else { None };
    let badge = presence_badge(status.unwrap_or(""));
    let dot = badge.map(|_| g.badge(size, geom));
    let cut = dot.map(|(x, y, d)| round::Cut { cx: x, cy: y, r: d / 2.0 + geom.ring * g.ppp });
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let tex = round::with(&ctx, |s| s.get(&ctx, round::Key::new(photo, g.px, cut, bg)));
    match tex {
        Some(id) => { ui.painter().image(id, g.rect, uv, Color32::WHITE); }
        None => {
            ui.painter().circle_filled(g.rect.center(), g.rect.width() / 2.0, bg);
            if let Some(p) = photo { circle_photo(ui.painter(), g.rect.center(), g.rect.width() / 2.0, p) }
        }
    }
    if photo.is_none() { ui.painter().text(g.rect.center(), egui::Align2::CENTER_CENTER, initials(name, space), FontId::proportional(size * if space { 0.5 } else { 0.42 }), t.avatar_text); }
    // a design plugin's ring around your own avatar (and everybody's, if it says so)
    if !space && (if crate::ui::theme::decor::is_self(name) { crate::ui::theme::decor::on(collabhost_plugin_api::deco::AVATAR_RING_SELF) } else { crate::ui::theme::decor::on(collabhost_plugin_api::deco::AVATAR_RING_ALL) }) {
        crate::ui::theme::decor::ring(ui.painter(), g.rect.center(), g.rect.width() / 2.0 + 1.5, (size / 16.0).clamp(1.5, 3.0), &crate::ui::theme::decor::stripes());
    }
    if let (Some((icon, col)), Some((x, y, d))) = (badge, dot) {
        let dr = g.texel_rect(x - d / 2.0, y - d / 2.0, d);
        if tex.is_none() { ui.painter().circle_filled(dr.center(), dr.width() / 2.0 + geom.ring, t.card); }
        if icon.is_empty() {
            match round::with(&ctx, |s| s.get(&ctx, round::Key::new(None, d as u32, None, col))) {
                Some(id) => { ui.painter().image(id, dr, uv, Color32::WHITE); }
                None => { ui.painter().circle_filled(dr.center(), dr.width() / 2.0, col); }
            }
        } else { ic.paint_exact(ui, icon, dr, col) }
        if crate::ui::theme::decor::on(collabhost_plugin_api::deco::PRESENCE_RING) { crate::ui::theme::decor::ring(ui.painter(), dr.center(), dr.width() / 2.0 + 0.8, 1.4, &crate::ui::theme::decor::stripes()); }
    }
}

/// Where an avatar of `size` pt lands on the physical pixel grid: its rect (top-left snapped to a device pixel, side
/// `px` device pixels) and the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AvatarGrid { pub rect: Rect, pub px: u32, pub ppp: f32 }
impl AvatarGrid {
    pub fn new(min: egui::Pos2, size: f32, ppp: f32) -> Self {
        let px = (size * ppp).round().max(1.0) as u32;
        let snap = |v: f32| (v * ppp).round() / ppp;
        AvatarGrid { rect: Rect::from_min_size(pos2(snap(min.x), snap(min.y)), Vec2::splat(px as f32 / ppp)), px, ppp }
    }
    /// The presence dot in texture pixels: (centre x, centre y, diameter) for `geom`, placed so the dot's own texture sits on
    /// whole pixels.
    pub fn badge(&self, size: f32, geom: BadgeGeom) -> (f32, f32, f32) {
        let d = (size * geom.d * self.ppp).round().max(1.0);
        let at = |f: f32| (size * f * self.ppp - d / 2.0).round() + d / 2.0;
        (at(geom.cx), at(geom.cy), d)
    }
    /// A square of `side` texture pixels whose top-left is (`x`, `y`) texture pixels from the avatar's corner, in points.
    pub fn texel_rect(&self, x: f32, y: f32, side: f32) -> Rect {
        Rect::from_min_size(self.rect.min + Vec2::new(x, y) / self.ppp, Vec2::splat(side / self.ppp))
    }
}


/// The vertical card gradients (theme-gradient-primary/-secondary), as a rounded mesh: perimeter points (corner arcs plus
/// straight edges) fanned from the centre.
pub(crate) fn vgrad(p: &egui::Painter, r: Rect, radius: f32, top: Color32, bottom: Color32) { vgrad_corners(p, r, [radius; 4], top, bottom) }

/// `vgrad` with one radius per corner: top-left, bottom-left, bottom-right, top-right.
pub(crate) fn vgrad_corners(p: &egui::Painter, r: Rect, radii: [f32; 4], top: Color32, bottom: Color32) {
    let lerp = |y: f32| {
        let k = ((y - r.top()) / r.height().max(1.0)).clamp(0.0, 1.0);
        Color32::from_rgb(
            (top.r() as f32 + (bottom.r() as f32 - top.r() as f32) * k) as u8,
            (top.g() as f32 + (bottom.g() as f32 - top.g() as f32) * k) as u8,
            (top.b() as f32 + (bottom.b() as f32 - top.b() as f32) * k) as u8,
        )
    };
    // On the pixel grid like egui's own rects (round_rects_to_pixels), with an antialiased rim: the plain fan had stair-stepped
    // corners.
    use egui::emath::GuiRounding;
    let ppp = p.ctx().pixels_per_point();
    let r = r.round_to_pixels(ppp);
    let pts = rounded_rect_points(r, radii, ppp, f32::INFINITY);
    fill_convex_aa(p, &pts, egui::TextureId::default(), 1, |q| (lerp(q.y), egui::epaint::WHITE_UV));
}

pub(crate) fn panel(fill: Color32) -> egui::Frame { egui::Frame::default().fill(fill) }

/// A round button filled with a colour, an icon in white (call, video, share).
pub(crate) fn round_fill(ui: &mut egui::Ui, ic: &mut icons::Icons, icon: &'static str, fill: Color32, size: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    ui.painter().circle_filled(r.center(), size / 2.0, if resp.hovered() { fill.gamma_multiply(0.85) } else { fill });
    ic.paint(ui, icon, Rect::from_center_size(r.center(), Vec2::splat(size * 0.5)), Color32::WHITE);
    resp
}

/// Pill tab (list filters, conversation tabs): selected = semibold on a rounded fill, unselected = dimmer regular text
/// (#BEBEBE on dark).
pub(crate) fn text_tab(ui: &mut egui::Ui, label: &str, on: bool, t: &Th, on_bg: Color32) -> egui::Response {
    let font = if on { semibold(14.0) } else { FontId::proportional(14.0) };
    let col = if on { t.text1 } else { t.tab_off };
    let w = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font.clone(), col).size().x) + 22.0;
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
    if on { ui.painter().rect_filled(r, 14.0, on_bg); } else if resp.hovered() { ui.painter().rect_filled(r, 14.0, alpha(on_bg, 0.5)); }
    if on && crate::ui::theme::decor::on(collabhost_plugin_api::deco::TAB_PILLS) { crate::ui::theme::decor::outline(ui.painter(), r, 14.0, 1.5, &crate::ui::theme::decor::stripes(), 0.0); }
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, font, col);
    resp
}

/// A vertical scroll area that scrolls with the wheel / touch pad and its scroll bar but not by dragging the content (a drag
/// selects text): egui's `drag_to_scroll(false)`. With middle-click autoscroll (ui/autoscroll.rs).
pub(crate) fn vscroll() -> super::autoscroll::VScroll { super::autoscroll::VScroll::new() }

/// The one rule for "may this widget react to the pointer right now": the pointer is over `rect` in `ui`'s own layer, i.e. no
/// other window / overlay (image viewer, dialog, menu, popup, picker ...) lies on top of that point. egui's own
/// `Response::hovered()` already works that way; this is for the manual checks (hover toolbars, custom hover fills). Also
/// clip-aware (a scrolled-away row isn't hovered).
pub(crate) fn hover_ok(ui: &egui::Ui, rect: Rect) -> bool { ui.rect_contains_pointer(rect) }

#[cfg(test)]
mod presence_badge_tests {
    use super::*;
    const GREEN: u32 = 0x27A17A;
    const ORANGE: u32 = 0xF2990A;
    const RED: u32 = 0xF55868;
    const GREY: u32 = 0x949494;
    fn badge(s: &str) -> Option<(&'static str, Color32)> { presence_badge(s) }

    #[test] fn active_is_a_plain_green_dot() { assert_eq!(badge("active"), Some(("", hex(GREEN)))); }
    #[test] fn in_a_call_is_a_handset_not_a_calendar() { assert_eq!(badge("call"), Some(("handset-filled", hex(ORANGE)))); }
    #[test] fn in_a_meeting_is_the_camera() { assert_eq!(badge("meeting"), Some(("camera-presence-filled", hex(ORANGE)))); }
    #[test] fn calendar_meeting_is_the_calendar() { assert_eq!(badge("calendar"), Some(("meetings-presence-badge-filled", hex(ORANGE)))); }
    #[test] fn busy_is_the_busy_glyph() { assert_eq!(badge("busy"), Some(("busy-presence-regular", hex(ORANGE)))); }
    #[test] fn presenting_is_the_red_screen_share() { assert_eq!(badge("presenting"), Some(("share-screen-badge-filled", hex(RED)))); }
    #[test] fn do_not_disturb_is_the_red_minus() { assert_eq!(badge("DoNotDisturb"), Some(("dnd-presence-badge-filled", hex(RED)))); }
    #[test] fn inactive_is_the_grey_clock() { assert_eq!(badge("inactive"), Some(("recents-presence-badge-filled", hex(GREY)))); }
    #[test] fn out_of_office_is_the_grey_plane() { assert_eq!(badge("OutOfOffice"), Some(("pto-presence-filled", hex(GREY)))); }
    #[test] fn quiet_hours_is_the_grey_moon() { assert_eq!(badge("quiet"), Some(("quiet-hours-presence-filled", hex(GREY)))); }
    #[test] fn on_hold_is_grey_pause() { assert_eq!(badge("hold"), Some(("pause-filled", hex(GREY)))); }
    #[test] fn joined_by_mobile_is_a_grey_phone() { assert_eq!(badge("mobile"), Some(("phone-filled", hex(GREY)))); }
    #[test] fn joined_by_device_is_a_grey_video_device() { assert_eq!(badge("device"), Some(("generic-device-video-filled", hex(GREY)))); }
    #[test] fn unknown_has_no_badge() { assert_eq!(badge(""), None); assert_eq!(badge("something"), None); }

    /// Every icon the mapping names ships in assets/icons.
    #[test]
    fn every_badge_icon_exists() {
        for s in ["meeting", "calendar", "call", "busy", "DoNotDisturb", "presenting", "quiet", "inactive", "OutOfOffice", "hold", "mobile", "device"] {
            let (icon, _) = badge(s).unwrap();
            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons").join(format!("{icon}.svg"));
            assert!(p.exists(), "{s}: {icon}.svg missing");
        }
    }
}

#[cfg(test)]
mod avatar_grid_tests {
    use super::*;

    #[test]
    fn avatar_is_rendered_at_physical_pixels_on_the_pixel_grid() {
        // 32 pt at 125 % = 40 device pixels, the corner on a whole device pixel even from a fractional layout position.
        let g = AvatarGrid::new(pos2(10.3, 7.0), 32.0, 1.25);
        assert_eq!(g.px, 40);
        for v in [g.rect.min.x * 1.25, g.rect.min.y * 1.25, g.rect.width() * 1.25] { assert!((v - v.round()).abs() < 1e-4, "{g:?}"); }
        assert_eq!(AvatarGrid::new(pos2(0.0, 0.0), 24.0, 1.25).px, 30);
        assert_eq!(AvatarGrid::new(pos2(0.0, 0.0), 96.0, 1.5).px, 144);
        assert_eq!(AvatarGrid::new(pos2(0.0, 0.0), 32.0, 1.0).px, 32);
    }

    #[test]
    fn badge_dot_sits_on_whole_pixels_inside_the_avatar() {
        for geom in [BadgeGeom::DEFAULT, BadgeGeom::HEADER] {
            for (size, ppp) in [(24.0, 1.25), (32.0, 1.25), (40.0, 1.25), (96.0, 1.5), (32.0, 2.0)] {
                let g = AvatarGrid::new(pos2(0.0, 0.0), size, ppp);
                let (x, y, d) = g.badge(size, geom);
                for left in [x - d / 2.0, y - d / 2.0] {
                    assert!((left - left.round()).abs() < 1e-4 && (d - d.round()).abs() < 1e-4, "{size} @ {ppp}: {x} {y} {d}");
                    // In the bottom-right quarter; it may overhang the circle's box (DEFAULT: 0.13 in, radius 0.17).
                    assert!(left + d <= g.px as f32 + 0.04 * size * ppp + 1.0 && left > g.px as f32 / 2.0, "{size} @ {ppp}");
                }
                // Where the geometry puts it, within half a device pixel.
                assert!((x - size * geom.cx * ppp).abs() <= 0.5 + 1e-4 && (y - size * geom.cy * ppp).abs() <= 0.5 + 1e-4);
            }
        }
    }

    /// A whole frame at 125 %: the photo avatar is one image of its own 40 px texture, not a scaled mesh of the 80+ px photo.
    #[test]
    fn photo_avatar_draws_a_screen_resolution_texture() {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(1.25);
        let photo = ctx.load_texture("p", egui::ColorImage::new([4, 4], Color32::WHITE), egui::TextureOptions::LINEAR);
        crate::assets::photos::round::register(&ctx, photo.id(), &image::RgbaImage::from_pixel(192, 192, image::Rgba([90, 120, 150, 255])));
        let t = Th::of(crate::ui::theme::ThemeId::Dark);
        let mut ic = icons::Icons::default();
        let mut sizes = vec![];
        for _ in 0..2 {
            let out = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| avatar_photo(ui, &mut ic, &t, "A B", 32.0, false, None, Some("active"), Some(photo.id())));
            });
            sizes = out.textures_delta.set.iter().filter(|(_, d)| d.pos.is_none()).map(|(_, d)| d.image.size()).collect();
            if !sizes.is_empty() { break }
        }
        assert!(sizes.contains(&[40, 40]), "{sizes:?}"); // the avatar
        assert!(sizes.contains(&[14, 14]), "{sizes:?}"); // its presence dot: 0.34 × 32 pt × 1.25
    }
}

#[cfg(test)]
mod aa_fill_tests {
    use super::*;

    #[test]
    fn rounded_rect_points_follow_the_arcs() {
        let r = Rect::from_min_size(pos2(10.0, 20.0), Vec2::new(100.0, 60.0));
        for ppp in [1.0, 1.25, 1.5, 2.0] {
            let pts = rounded_rect_points(r, [10.0, 0.0, 10.0, 4.0], ppp, 24.0);
            assert!(pts.iter().all(|q| r.expand(1e-3).contains(*q)), "{ppp}");
            // Top-left arc: every point of it is 10 pt from its centre; the square corner is a point itself.
            let c = pos2(20.0, 30.0);
            assert!(pts.iter().filter(|q| q.x < 20.0 && q.y < 30.0).all(|q| (q.distance(c) - 10.0).abs() < 1e-3));
            assert!(pts.iter().any(|q| q.distance(r.left_bottom()) < 1e-3));
            // Straight edges are cut into pieces of at most 24 pt, arcs into chords within ~0.1 device pixel.
            for (a, b) in pts.iter().zip(pts.iter().cycle().skip(1)) { assert!(a.distance(*b) <= 24.0 + 1e-3); }
            let arc: Vec<_> = pts.iter().filter(|q| q.x < 20.0 && q.y < 30.0).collect();
            for w in arc.windows(2) { let mid = pos2((w[0].x + w[1].x) / 2.0, (w[0].y + w[1].y) / 2.0); assert!((10.0 - mid.distance(c)) * ppp < 0.15, "{ppp}"); }
        }
    }

    /// The fill has egui's antialiased rim: the outermost ring is transparent and lies half a device pixel outside the edge,
    /// the coloured one half a pixel inside it.
    #[test]
    fn convex_fill_has_a_feathered_rim() {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(1.5);
        let r = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(60.0, 40.0));
        let mut meshes = vec![];
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            let pts = rounded_rect_points(r, [10.0; 4], 1.5, 24.0);
            fill_convex_aa(&p, &pts, egui::TextureId::default(), 2, |_| (Color32::RED, egui::epaint::WHITE_UV));
            ctx.graphics(|g| for s in g.get(egui::LayerId::background()).unwrap().all_entries() { if let egui::Shape::Mesh(m) = &s.shape { meshes.push(m.clone()) } });
        });
        let m = &meshes[0];
        let (clear, red): (Vec<&egui::epaint::Vertex>, Vec<_>) = m.vertices.iter().partition(|v| v.color == Color32::TRANSPARENT);
        assert!(!clear.is_empty() && red.len() > clear.len());
        let half = 0.5 / 1.5;
        // The straight top edge: transparent vertices at y = -half, coloured outermost ones at y = +half.
        assert!(clear.iter().any(|v| (v.pos.y + half).abs() < 1e-3 && v.pos.x > 20.0 && v.pos.x < 40.0));
        assert!(red.iter().any(|v| (v.pos.y - half).abs() < 1e-3 && v.pos.x > 20.0 && v.pos.x < 40.0));
        assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()) && m.indices.len() % 3 == 0);
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use egui::{Id, Order};

    #[test]
    fn hover_checks_follow_the_layer_under_the_pointer() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(pos2(10.0, 10.0), Vec2::new(100.0, 40.0));
        let run = |cover: bool| {
            let mut hovered = false;
            let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(300.0, 200.0))), ..Default::default() };
            raw.events.push(egui::Event::PointerMoved(pos2(40.0, 30.0)));
            for _ in 0..3 {
                let r = raw.clone();
                let _ = ctx.run(r, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| { hovered = hover_ok(ui, rect); });
                    if cover {
                        egui::Area::new(Id::new("cover")).fixed_pos(pos2(0.0, 0.0)).order(Order::Foreground).show(ctx, |ui| { ui.allocate_exact_size(Vec2::new(300.0, 200.0), Sense::click_and_drag()); });
                    }
                });
                raw.events.clear();
            }
            hovered
        };
        assert!(run(false));
        assert!(!run(true)); // an overlay on top: no hover for the page behind it
    }
}

pub(crate) fn semibold(size: f32) -> FontId { FontId::new(size, egui::FontFamily::Name("semibold".into())) }

/// A flat vertical gradient quad (the window's own background gradient).
pub(crate) fn vquad(p: &egui::Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut m = egui::Mesh::default();
    for (pt, c) in [(r.left_top(), top), (r.right_top(), top), (r.right_bottom(), bottom), (r.left_bottom(), bottom)] { m.colored_vertex(pt, c); }
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(m);
}

/// The UI's German strings (from the language packs). Unknown keys pass through unchanged.
pub(crate) fn tr(s: &str) -> &str {
    // Delegate to the language packs (any context); English = identity.
    if crate::i18n::is_source_language() { return s; }
    if let Some(t) = crate::i18n::lookup_any(s) { return t; }
    // The hand-written map below is the fallback.
    match s {
        "Messaging" => "Nachrichten", "Meetings" => "Meetings", "Calling" => "Anrufe", "Whiteboards" => "Whiteboards",
        "Contacts" => "Kontakte", "Teams" => "Teams", "More" => "Weitere", "Settings" => "Einstellungen", "Help" => "Hilfe",
        "All" => "Alle", "Direct" => "Direkt", "Spaces" => "Bereiche", "Public" => "Öffentlich",
        "Recommended messages" => "Empfohlene Nachrichten", "Favorites" => "Favoriten", "Other" => "Andere",
        "Messages" => "Nachrichten", "People" => "Personen", "Profile" => "Profil", "Content" => "Inhalt", "Apps" => "+Apps",
        "Search, meet, and call" => "Suchen, treffen und anrufen", "Ctrl+F" => "Strg+F", "Connect to a device" => "Mit einem Gerät verbinden",
        "Set a status" => "Status festlegen", "Meet" => "Treffen",
        "Shift+Enter for a new line" => "Umschalt+Eingabe für eine neue Zeile", "Today" => "Heute", "Yesterday" => "Gestern", "You" => "Sie",
        "No filter" => "Kein Filter", "Notifications" => "Benachrichtigungen", "Drafts" => "Entwürfe", "Unread" => "Ungelesen",
        "@Mentions to me" => "@Erwähnungen an mich", "@Mentions to all" => "@Erwähnungen an alle", "Flags" => "Markierungen",
        "Filter" => "Filtern", "Create a space" => "Bereich erstellen", "Reply to thread" => "Im Thread antworten",
        "Edited" => "Bearbeitet", "Back" => "Zurück", "Forward" => "Weiter", "Seen by" => "Gesehen von",
        _ => s,
    }
}

pub(crate) fn toggle<'a>(on: &'a mut bool, t: &'a Th) -> impl egui::Widget + 'a {
    move |ui: &mut egui::Ui| {
        let (r, mut resp) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
        if resp.clicked() { *on = !*on; resp.mark_changed(); }
        let k = ui.ctx().animate_bool(resp.id, *on);
        if *on && crate::ui::theme::decor::on(collabhost_plugin_api::deco::TOGGLE_TRACK) { crate::ui::theme::decor::fill(ui.painter(), r, 11.0, &crate::ui::theme::decor::stripes(), 0.0, 1.0); }
        else { ui.painter().rect_filled(r, 11.0, if *on { t.accent } else { t.pill }); }
        let knob = pos2(egui::lerp(r.left() + 11.0..=r.right() - 11.0, k), r.center().y);
        ui.painter().circle_filled(knob, 8.0, Color32::WHITE);
        if *on && crate::ui::theme::decor::on(collabhost_plugin_api::deco::TOGGLE_TRACK) { ui.painter().circle_stroke(knob, 8.0, Stroke::new(1.0_f32, Color32::from_black_alpha(90))); } // the knob stays visible on light stripes
        resp
    }
}

// See avatar_photo: false while Settings > "Verfügbarkeit und Status anzeigen" is off.
pub(crate) static PRESENCE_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// Message text scale of Barrierefreiheit > Schriftgröße (f32 bits; 1.0 = Regulär), set each frame by App::appwide_frame.
pub(crate) static MSG_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000);
pub(crate) fn msg_scale() -> f32 { f32::from_bits(MSG_SCALE.load(std::sync::atomic::Ordering::Relaxed)) }
