//! Loading / splash experience.
//!
//! * `webex-logo-spinner.gif` is the animated logo shown while the cached sign-in is being renewed: 240x240, 125 frames, 40 ms
//!   each (5 s loop), transparent background.
//! * `wallpaper-splash.png` / `wallpaper-splash-dark.png` (1440x1024 coloured glow band on white / black) are the splash
//!   background by theme.
//! * `Spinner_Symphony_color_darker.svg` (40 px) is the small list spinner.
//! * The list placeholder shape: thumbnail, 12 px bars radius 4, gap 8, 2/3 width second bar, colour = listitem-background
//!   focused.
//! * Texts are pack sources: "Signing In...", "Connecting...", "Loading messages",
//!   "We can't reach the {{APP_NAME}} service  •  Reconnecting...". The pack has no
//!   per-step wording (only one "Connecting..."), so Finding services / Registering / Key service map to it.
//! * The connectivity banner is styled with theme tokens.
use crate::*;
use std::sync::{Arc, Mutex, OnceLock};

// Webex client files (`assets/cisco/images`); without them the splash is the plain fill and egui's spinner.
fn asset(name: &str) -> Option<&'static [u8]> { crate::assets::cisco_assets::get(&format!("images/{name}")) }
/// Decoded frame side (px): 168 keeps the 125 frames at ~14 MB while staying sharp at the 120 pt display size.
const SIDE: u32 = 168;
const LOGO_PT: f32 = 120.0;

type Decoded = Vec<(egui::ColorImage, f64)>;
static DECODED: OnceLock<Mutex<Option<Decoded>>> = OnceLock::new();

#[derive(Clone)]
struct Frames(Arc<Vec<(egui::TextureHandle, f64)>>, f64);

fn decode() -> Decoded {
    use image::AnimationDecoder;
    let mut out = vec![];
    let Some(gif) = asset("webex-logo-spinner.gif") else { return out };
    let Ok(dec) = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(gif)) else { return out };
    for f in dec.into_frames().flatten() {
        let (n, d) = f.delay().numer_denom_ms();
        let secs = (n as f64 / d.max(1) as f64 / 1000.0).max(0.02);
        let img = image::imageops::resize(f.buffer(), SIDE, SIDE, image::imageops::FilterType::Triangle);
        out.push((egui::ColorImage::from_rgba_unmultiplied([SIDE as usize; 2], img.as_raw()), secs));
    }
    out
}

/// The logo frame for `now`, once decoded in the background (None while still decoding).
fn logo_frame(ctx: &egui::Context, now: f64) -> Option<egui::TextureId> {
    let id = egui::Id::new("loading-frames");
    let have: Option<Frames> = ctx.data(|d| d.get_temp(id));
    let fr = match have {
        Some(f) => f,
        None => {
            let cell = DECODED.get_or_init(|| { let c = Mutex::new(None); c });
            static STARTED: std::sync::Once = std::sync::Once::new();
            STARTED.call_once(|| { std::thread::spawn(|| { let v = decode(); *DECODED.get().unwrap().lock().unwrap() = Some(v); }); });
            let ready = cell.lock().unwrap().take();
            match ready {
                Some(v) if !v.is_empty() => {
                    let total: f64 = v.iter().map(|x| x.1).sum();
                    let tex: Vec<_> = v.into_iter().enumerate().map(|(i, (ci, s))| (ctx.load_texture(format!("splash-logo-{i}"), ci, egui::TextureOptions::LINEAR), s)).collect();
                    let f = Frames(Arc::new(tex), total);
                    ctx.data_mut(|d| d.insert_temp(id, f.clone()));
                    f
                }
                _ => { ctx.request_repaint_after(std::time::Duration::from_millis(50)); return None }
            }
        }
    };
    let mut t = now % fr.1;
    for (tex, s) in fr.0.iter() {
        if t < *s {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((*s - t).max(0.005)));
            return Some(tex.id());
        }
        t -= *s;
    }
    fr.0.last().map(|x| x.0.id())
}

pub(crate) struct Load { pub splash: Option<String>, pub skeleton: bool, pub banner: Option<String> }

/// Pack wording for the cloud thread's status lines (see module docs), in the active language.
#[allow(dead_code)] // the splash shows the raw status lines
pub(crate) fn status_text(s: &str) -> &'static str {
    if s.starts_with("Signing in") || s.starts_with("Renewing") { tr("Signing In...") }
    else if s.starts_with("Connected") { tr("Loading messages") }
    else { tr("Connecting...") }
}

/// The connectivity banner's text ({{APP_NAME}} here is the service, "Webex").
fn banner() -> String { crate::trq!("We can't reach the {{APP_NAME}} service  •  Reconnecting...").into_owned() }

impl App {
    pub(crate) fn load_state(&self, ctx: &egui::Context, now: f64) -> Load {
        let none = Load { splash: None, skeleton: false, banner: None };
        // Test hooks, read once (this runs twice per frame).
        static SHOT_LOADING: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        static CONNECT_DELAY: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
        if let Some(v) = SHOT_LOADING.get_or_init(|| std::env::var("WEBOX_SHOT_LOADING").ok()) {
            return match v.as_str() {
                "1" => Load { splash: Some("Finding services…".into()), ..none },
                "2" => Load { skeleton: true, ..none },
                "3" => Load { banner: Some(banner()), ..none },
                _ => none,
            };
        }
        // WEBOX_SAMPLE_CONNECT_DELAY_MS=<n>: the sample client emulates a slow connect over n ms.
        if self.client.is_none() && !self.need_login {
            if let Some(n) = *CONNECT_DELAY.get_or_init(|| std::env::var("WEBOX_SAMPLE_CONNECT_DELAY_MS").ok().and_then(|v| v.parse::<f64>().ok())) {
                let id = egui::Id::new("sim-t0");
                let t0: f64 = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
                let f = (now - t0) * 1000.0 / n.max(1.0);
                ctx.request_repaint_after(std::time::Duration::from_millis(33));
                const STEPS: [&str; 5] = ["Signing in…", "Finding services…", "Registering this device…", "Opening the key service…", "Connected"];
                return if f < 0.85 { Load { splash: Some(STEPS[((f / 0.17) as usize).min(4)].into()), ..none } }
                    else if f < 1.0 { Load { skeleton: true, ..none } }
                    else if f < 1.3 { Load { banner: Some(banner()), ..none } }
                    else { none };
            }
        }
        // Saved sign-in being renewed at app start (the logo animation).
        if !self.need_login && self.pending.is_some() && self.client.is_none() {
            return Load { splash: Some("Renewing your sign-in…".into()), ..none };
        }
        if self.client.is_some() {
            let st = self.status.as_str();
            // The connectivity banner waits 60 s before it shows: a short reconnect is silent.
            let rid = egui::Id::new("reconnect-since");
            if st.starts_with("Reconnecting") {
                let t0: f64 = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(rid, || now));
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
                if now - t0 >= 60.0 { return Load { banner: Some(banner()), ..none } }
            } else {
                ctx.data_mut(|d| d.remove::<f64>(rid));
            }
            // Hold the splash until the chats and everything are loaded. Ready = the space list is here and the opened chat's
            // messages are here, then a short settle time for photos / presence / covers (no "all done" signal exists in the data
            // layer). Never longer than 30 s (an account may have no spaces), and only once per run.
            let done_id = egui::Id::new("splash-done");
            if !ctx.data(|d| d.get_temp::<bool>(done_id)).unwrap_or(false) {
                let t0: f64 = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(egui::Id::new("splash-t0"), || now));
                let loaded = !self.spaces.is_empty() && self.selected.as_ref().map_or(true, |id| self.messages.contains_key(id));
                let ready_t: Option<f64> = if loaded { Some(ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(egui::Id::new("splash-ready"), || now))) } else { None };
                let settled = ready_t.map_or(false, |t| now - t >= 1.5);
                let nothing_to_wait_for = self.spaces.is_empty() && st.starts_with("Connected") && now - t0 > 10.0; // account without spaces
                if settled || nothing_to_wait_for || now - t0 > 30.0 {
                    ctx.data_mut(|d| d.insert_temp(done_id, true));
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    // Tell the user what is being loaded (same raw English style as the connection status lines): spaces, then the
                    // opened chat's messages, then the short settle time for photos / presence.
                    let known = ["Signing in", "Finding services", "Registering", "Opening the key"].iter().any(|p| st.starts_with(p));
                    let line: String = if known { st.into() }
                        else if self.spaces.is_empty() { "Loading your spaces…".into() }
                        else if !loaded { "Loading messages…".into() }
                        else { "Loading photos and presence…".into() };
                    return Load { splash: Some(line), ..none };
                }
            }
            if self.spaces.is_empty() {
                let known = ["Signing in", "Finding services", "Registering", "Opening the key"].iter().any(|p| st.starts_with(p));
                if known { return Load { splash: Some(st.into()), ..none } }
                // Connected, spaces not here yet: skeleton list, but only for a while (an account may have no spaces).
                let t0: f64 = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(egui::Id::new("skel-t0"), || now));
                if now - t0 < 20.0 { return Load { skeleton: true, ..none } }
            }
        }
        none
    }

    /// Full-area splash (fades in/out): wallpaper by theme, the animated logo, status line below it.
    pub(crate) fn splash_overlay(&mut self, ctx: &egui::Context, t: &Th, area: Rect, text: Option<&str>) {
        let tid = egui::Id::new("splash-text");
        if let Some(s) = text { ctx.data_mut(|d| d.insert_temp(tid, s.to_string())); }
        let a = ctx.animate_bool_with_time(egui::Id::new("splash-fade"), text.is_some(), 0.25);
        if a <= 0.0 { return; }
        let shown: String = ctx.data(|d| d.get_temp(tid)).unwrap_or_default();
        let light = t.light; // the splash wallpaper has a light and a dark variant
        let now = ctx.input(|i| i.time);
        let logo = logo_frame(ctx, now);
        egui::Area::new(egui::Id::new("splash-area")).order(egui::Order::Foreground).fixed_pos(area.min).interactable(true).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
            // No header while loading: the window can still be moved by dragging the splash.
            if resp.drag_started() { crate::ui::chrome::note_os_grab(ctx); ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
            let p = ui.painter();
            p.rect_filled(r, 0.0, if light { Color32::WHITE } else { Color32::BLACK }.gamma_multiply(a));
            if let Some((id, [w, h])) = asset(if light { "wallpaper-splash.png" } else { "wallpaper-splash-dark.png" })
                .and_then(|b| self.icons.png(ui, if light { "splash_light" } else { "splash_dark" }, b)) {
                let (ia, ra) = (w as f32 / h.max(1) as f32, r.width() / r.height());
                let uv = if ia > ra { let k = ra / ia; Rect::from_min_max(pos2(0.5 - k / 2.0, 0.0), pos2(0.5 + k / 2.0, 1.0)) }
                         else { let k = ia / ra; Rect::from_min_max(pos2(0.0, 0.5 - k / 2.0), pos2(1.0, 0.5 + k / 2.0)) };
                p.image(id, r, uv, Color32::WHITE.gamma_multiply(a));
            }
            let c = r.center() - Vec2::new(0.0, 14.0);
            if let Some(id) = logo {
                p.image(id, Rect::from_center_size(c, Vec2::splat(LOGO_PT)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(a));
            }
            p.text(c + Vec2::new(0.0, LOGO_PT / 2.0 + 22.0), egui::Align2::CENTER_CENTER, &shown, F::BodyLarge.font(), t.text2.gamma_multiply(a));
        });
        // Fades: one frame after another; while shown, the logo schedules its own frames (logo_frame) and status changes wake the window.
        if a < 1.0 { ctx.request_repaint(); }
    }

    /// Placeholder rows while the spaces load (pulsing, like a shimmer). Returns true when drawn.
    pub(crate) fn skeleton_rows(&self, ui: &mut egui::Ui, t: &Th) -> bool {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if !self.load_state(&ctx, now).skeleton { return false }
        let pulse = 0.55 + 0.45 * (0.5 + 0.5 * (now * 2.4).sin() as f32);
        let col = t.selected.gamma_multiply(pulse);
        let w = ui.available_width();
        // 56 pt in Standard; Kompakt and the compact view make it shorter
        let g = crate::ui::theme::density::ListRow::current(self.compact_view());
        let k = g.avatar / 32.0; // circle and bars scale with the row's avatar
        for i in 0..9 {
            let (r, _) = ui.allocate_exact_size(Vec2::new(w, g.skeleton_h), Sense::hover());
            let p = ui.painter();
            let c = pos2(r.left() + 28.0, r.center().y);
            p.circle_filled(c, 16.0 * k, col);
            let x = r.left() + 56.0;
            let bw = (r.right() - 16.0 - x).max(40.0);
            p.rect_filled(Rect::from_min_size(pos2(x, r.center().y - 14.0 * k), Vec2::new(bw * [0.7, 0.55, 0.8][i % 3], 12.0 * k)), 4.0, col);
            p.rect_filled(Rect::from_min_size(pos2(x, r.center().y + 4.0 * k), Vec2::new(bw * 2.0 / 3.0 * [1.0, 0.8, 0.9][i % 3], 12.0 * k)), 4.0, col);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
        true
    }

    /// Top banner under the header while the services are unreachable ("Reconnecting").
    pub(crate) fn connectivity_banner(&mut self, ctx: &egui::Context, t: &Th, text: Option<&str>) {
        let a = ctx.animate_bool_with_time(egui::Id::new("conn-banner"), text.is_some(), 0.2);
        if a <= 0.0 { return; }
        let now = ctx.input(|i| i.time);
        let logo = logo_frame(ctx, now);
        egui::TopBottomPanel::top("conn-banner-panel").exact_height(34.0 * a).show_separator_line(false)
            .frame(egui::Frame::default().fill(t.selected)).show(ctx, |ui| {
            let r = ui.max_rect();
            if r.height() < 20.0 { return }
            let p = ui.painter();
            let mut x = r.center().x;
            let fallback = banner();
            let txt = text.unwrap_or(&fallback);
            let galley = p.layout_no_wrap(txt.to_string(), F::Label.font(), t.text1);
            x -= (galley.size().x + 28.0) / 2.0;
            if let Some(id) = logo { p.image(id, Rect::from_center_size(pos2(x + 10.0, r.center().y), Vec2::splat(22.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
            p.galley(pos2(x + 28.0, r.center().y - galley.size().y / 2.0), galley, t.text1);
        });
        // The slide in / out animates itself (animate_bool), the logo schedules its own frames (logo_frame).
        if a < 1.0 { ctx.request_repaint(); }
    }
}
