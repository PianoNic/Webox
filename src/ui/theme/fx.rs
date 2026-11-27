//! Short effects of design plugins (plugin-api 1.4 `deco` effect bits): a burst of confetti and hearts from the send button,
//! a ripple on a reaction you add, a "Happy Pride!" sparkle once in June. Painted on a layer above the window that takes no
//! input, so they never block anything. The window repaints only while one is running (request_repaint_after), and nothing
//! starts while the OS asks for reduced motion.
use eframe::egui::{self, pos2, vec2, Color32, Id, LayerId, Order, Pos2, Rect, Stroke};
use std::cell::RefCell;

const BURST_S: f64 = 0.9;
const RIPPLE_S: f64 = 0.6;
const SPARKLE_S: f64 = 2.6;

enum Kind { Burst(Pos2), Ripple(Rect), Sparkle }
struct Fx { kind: Kind, t0: Option<f64>, seed: u32, colours: Vec<Color32> }

thread_local! {
    static FX: RefCell<Vec<Fx>> = const { RefCell::new(Vec::new()) };
    static SEND_AT: RefCell<Option<Rect>> = const { RefCell::new(None) };
}

/// Whether the OS asks for less motion (Windows: "Show animations in Windows" off).
pub(crate) fn reduce_motion() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
        let mut on = windows::Win32::Foundation::BOOL(1);
        let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }.is_ok();
        ok && !on.as_bool()
    }
    #[cfg(not(windows))]
    { std::env::var_os("WEBOX_REDUCE_MOTION").is_some() }
}

fn start(kind: Kind) {
    if reduce_motion() { return; }
    let colours = super::decor::stripes();
    if colours.is_empty() { return; }
    let seed = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos())) | 1;
    FX.with(|f| { let mut f = f.borrow_mut(); f.retain(|x| !matches!((&x.kind, &kind), (Kind::Burst(_), Kind::Burst(_)))); f.push(Fx { kind, t0: None, seed, colours }) });
    super::super::wake();
}

/// The send button's rect this frame (the burst starts there).
pub(crate) fn send_button(r: Rect) { SEND_AT.with(|s| *s.borrow_mut() = Some(r)); }
/// A message was sent: a burst from the send button (with SEND_BURST on).
pub(crate) fn sent() {
    if !super::decor::on(collabhost_plugin_api::deco::SEND_BURST) { return; }
    if let Some(r) = SEND_AT.with(|s| *s.borrow()) { start(Kind::Burst(r.center())) }
}
/// A reaction was added on `r` (with REACTION_RIPPLE on).
pub(crate) fn reacted(r: Rect) { if super::decor::on(collabhost_plugin_api::deco::REACTION_RIPPLE) { start(Kind::Ripple(r)) } }
/// The June sparkle (the caller checks the date and that it has not run this year).
pub(crate) fn sparkle() { start(Kind::Sparkle) }
pub(crate) fn running() -> bool { FX.with(|f| !f.borrow().is_empty()) }

/// xorshift: the n-th pseudo-random number of a burst, 0..1.
fn rnd(seed: u32, n: u32) -> f32 { let mut x = seed ^ n.wrapping_mul(0x9E37_79B9); x ^= x << 13; x ^= x >> 17; x ^= x << 5; (x % 10_000) as f32 / 10_000.0 }

fn heart(p: &egui::Painter, c: Pos2, s: f32, col: Color32) {
    let pts: Vec<Pos2> = (0..24).map(|i| {
        let t = i as f32 / 24.0 * std::f32::consts::TAU;
        let (x, y) = (16.0 * t.sin().powi(3), 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos());
        c + vec2(x, -y) * (s / 32.0)
    }).collect();
    p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
}

/// Paints the running effects (once per frame, last); asks for the next frame only while one runs.
pub(crate) fn paint(ctx: &egui::Context) {
    if !running() { return; }
    let now = ctx.input(|i| i.time);
    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("decor-fx")));
    let screen = ctx.screen_rect();
    FX.with(|f| {
        let mut f = f.borrow_mut();
        for fx in f.iter_mut() {
            let t = now - *fx.t0.get_or_insert(now);
            let c = |i: usize| fx.colours[i % fx.colours.len()];
            match fx.kind {
                Kind::Burst(o) => {
                    let k = (t / BURST_S) as f32;
                    let fade = if k > 0.6 { 1.0 - (k - 0.6) / 0.4 } else { 1.0 }.clamp(0.0, 1.0);
                    for i in 0..22u32 {
                        let a = (-150.0 + 120.0 * rnd(fx.seed, i)).to_radians();
                        let v = 140.0 + 160.0 * rnd(fx.seed, i + 100);
                        let tt = t as f32;
                        let q = o + vec2(a.cos() * v * tt, a.sin() * v * tt + 420.0 * tt * tt);
                        let col = crate::alpha(c(i as usize), fade);
                        if i % 3 == 0 { heart(&p, q, 12.0, col) } else {
                            let s = 3.0 + 2.0 * rnd(fx.seed, i + 200);
                            let rot = tt * 8.0 * (rnd(fx.seed, i + 300) - 0.5);
                            let (dx, dy) = (vec2(rot.cos(), rot.sin()) * s, vec2(-rot.sin(), rot.cos()) * s * 0.6);
                            p.add(egui::Shape::convex_polygon(vec![q - dx - dy, q + dx - dy, q + dx + dy, q - dx + dy], col, Stroke::NONE));
                        }
                    }
                }
                Kind::Ripple(r) => {
                    let k = (t / RIPPLE_S) as f32;
                    let rad = r.height() / 2.0 + k * r.width().max(24.0);
                    let cols: Vec<Color32> = fx.colours.iter().map(|x| crate::alpha(*x, (1.0 - k).clamp(0.0, 1.0))).collect();
                    super::decor::ring(&p, r.center(), rad, 2.5, &cols);
                }
                Kind::Sparkle => {
                    let k = (t / SPARKLE_S) as f32;
                    let a = if k < 0.15 { k / 0.15 } else if k > 0.75 { (1.0 - k) / 0.25 } else { 1.0 }.clamp(0.0, 1.0);
                    let box_r = Rect::from_center_size(pos2(screen.center().x, screen.top() + screen.height() * 0.3), vec2(300.0, 64.0));
                    // dark pill under white text: readable on any theme
                    p.rect_filled(box_r, 32.0, crate::alpha(Color32::from_rgb(18, 18, 18), 0.85 * a));
                    let cols: Vec<Color32> = fx.colours.iter().map(|x| crate::alpha(*x, a)).collect();
                    super::decor::outline(&p, box_r, 32.0, 3.0, &cols, 0.0);
                    p.text(box_r.center(), egui::Align2::CENTER_CENTER, "Happy Pride!", crate::F::Hero.font(), crate::alpha(Color32::WHITE, a));
                    for i in 0..14u32 {
                        let q = pos2(box_r.left() - 40.0 + (box_r.width() + 80.0) * rnd(fx.seed, i), box_r.top() - 30.0 + (box_r.height() + 60.0) * rnd(fx.seed, i + 50));
                        let tw = ((t as f32 * 6.0 + i as f32).sin() * 0.5 + 0.5) * a;
                        let s = 4.0 + 5.0 * rnd(fx.seed, i + 90);
                        let col = crate::alpha(c(i as usize), tw);
                        p.line_segment([q - vec2(s, 0.0), q + vec2(s, 0.0)], Stroke::new(2.0_f32, col));
                        p.line_segment([q - vec2(0.0, s), q + vec2(0.0, s)], Stroke::new(2.0_f32, col));
                    }
                }
            }
        }
        f.retain(|fx| { let d = match fx.kind { Kind::Burst(_) => BURST_S, Kind::Ripple(_) => RIPPLE_S, Kind::Sparkle => SPARKLE_S }; fx.t0.is_none_or(|t0| now - t0 < d) });
    });
    if running() { ctx.request_repaint_after(std::time::Duration::from_millis(16)); }
}

impl crate::App {
    /// Per frame: the June sparkle once a year (first start in June with JUNE_SPARKLE on; `WEBOX_SHOT_PRIDE_SPARKLE` in
    /// screenshot runs), then the running effects.
    pub(crate) fn decor_effects(&mut self, ctx: &egui::Context) {
        use collabhost_plugin_api::deco::JUNE_SPARKLE;
        if self.frames == 2 && super::decor::on(JUNE_SPARKLE) {
            let today = crate::system::today(); // YYYY-MM-DD
            let forced = self.shot.is_some() && std::env::var_os("WEBOX_SHOT_PRIDE_SPARKLE").is_some();
            let year = today.get(..4).unwrap_or_default().to_string();
            if forced || sparkle_due(&today, self.settings["pride_sparkle_year"].as_str()) {
                sparkle();
                if self.shot.is_none() { self.settings["pride_sparkle_year"] = serde_json::json!(year); crate::system::save_settings(&self.cfg, &self.settings); }
            }
        }
        paint(ctx);
    }
}

/// Pure check used by `decor_effects`: whether the sparkle is due on `today` (YYYY-MM-DD) after the one of `last_year`.
pub(crate) fn sparkle_due(today: &str, last_year: Option<&str>) -> bool { today.get(5..7) == Some("06") && today.get(..4) != last_year }

#[cfg(test)]
mod tests {
    #[test]
    fn sparkle_once_a_year_in_june() {
        assert!(super::sparkle_due("2027-06-01", None));
        assert!(super::sparkle_due("2027-06-30", Some("2026")));
        assert!(!super::sparkle_due("2027-06-02", Some("2027")));
        assert!(!super::sparkle_due("2027-07-01", None));
    }
    #[test]
    fn random_is_in_range() { for n in 0..500 { let x = super::rnd(12345, n); assert!((0.0..1.0).contains(&x)); } }
}
