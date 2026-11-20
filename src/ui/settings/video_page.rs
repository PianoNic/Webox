//! Settings > Video.
//!
//! Layout: the camera preview is pinned above the scrolling page (424 x 240, radius 8, `background-tertiary`; a 28 px
//! "Vorschau" band at the bottom on `overlayFade-secondary-background-normal`, secondary semibold text; the round rotate
//! button (S, overlay style, `rotate-content-bold`) 8 px from the top-right corner; bottom margin 8). When the camera can't
//! run, the same box shows the error panel (warning-regular 40, header title, secondary text, optional secondary pill).
//! Sections: Kamera (camera combo, the option checkboxes, "Max. Kameraauflösung" row with more-info button and a combo of
//! min. width 180), Videoverbesserungen (touch up + level row, correct my lighting, customize brightness / contrast /
//! saturation with 120 px label rows of 32), Selbstansicht in Meetings (radio group + 110 x 75 illustration, separator,
//! checkbox), Erweiterte Videoeinstellungen.
//!
//! Real: camera list (worker thread, refreshed every 2 s while the page is shown = hot-plug), live preview (worker
//! thread, ≤ 30 fps, stops when the page is left), camera choice, mirror, zoom to fit, rotation, max. resolution (the
//! capture mode is chosen from it), hardware acceleration (Windows: the MF reader's hardware decoder for MJPG), touch up,
//! lighting and brightness / contrast / saturation as image adjustments of the preview. Stored only: the self-view
//! options, virtual-background acceleration and the capture API choice (Webox calls carry no video).
use super::kit::{self, section};
use crate::media::camera::{self, CamDev, Fail, Look, Preview, PreviewKey, State};
use crate::*;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// Camera preview / error panel size (pt).
pub(crate) const PREVIEW: Vec2 = Vec2::new(424.0, 240.0);
/// Gap below the pinned preview (pt).
pub(crate) const PINNED_GAP: f32 = 8.0;

const MIRROR_INFO: &str = "Diese Einstellung wirkt sich nur darauf aus, wie Sie Ihr eigenes Video sehen. Wenn der Hintergrund Ihnen spiegelverkehrt vorkommt, keine Sorge, alle anderen sehen ihn normal.";
const RES_INFO: &str = "Festen Sie die maximal bevorzugte Auflösung fest, die von der Kamera verwendet werden kann. Die tatsächlich übertragene Auflösung kann durch die Webex-Administrationseinstellungen, die verfügbare Bandbreite, die Leistungsfähigkeit der Kamera und andere Faktoren eingeschränkt sein.";
const TOUCH_INFO: &str = "Wenden Sie einen glättenden Effekt und einen weicheren Fokus auf Ihr Video an, um ein besseres Erscheinungsbild zu erhalten.";
const LIGHT_INFO: &str = "Achten Sie darauf, dass Sie bei jeder Beleuchtung, auch bei schwachem Licht oder hellem Hintergrund, gut sichtbar sind.";
/// Name of the synthetic camera of --sample runs (never a real device).
const SAMPLE_CAM: &str = "Testkamera";

/// Camera state of the page (kept in `CallUi`; the preview stops one frame after the page is no longer drawn).
#[derive(Default)]
pub(crate) struct CamUi {
    preview: Option<Preview>,
    tex: Option<egui::TextureHandle>,
    seq: u64,
    seen: f64,
    devs: Vec<CamDev>,
    listed: bool,
    listed_at: f64,
    job: Option<Receiver<Vec<CamDev>>>,
    failed_at: f64,
}
impl CamUi {
    /// Called every frame: releases the camera once the preview hasn't been drawn for half a second.
    pub(crate) fn idle(&mut self, now: f64) {
        if self.preview.is_some() && now - self.seen > 0.5 { self.preview = None; self.tex = None; self.seq = 0; }
    }
}

/// An enhancement value 0..100 (older Webox builds stored 0..1).
fn pct(v: &serde_json::Value) -> f32 { v.as_f64().map_or(50.0, |x| if x <= 1.0 { x * 100.0 } else { x }) as f32 }

/// The error panel's title, text and whether it offers "Systemeinstellungen öffnen" (pack strings, QObject).
fn fail_text(f: &Fail) -> (String, String, bool) {
    match f {
        Fail::NoCamera => ("Keine Kamera gefunden".into(), "Stellen Sie sicher, dass Ihre Kamera an Ihren Computer angeschlossen ist.".into(), false),
        Fail::InUse(n) => ("Ihre Kamera kann nicht gestartet werden".into(), format!("Stellen Sie sicher, dass Ihr Kameragerät {n} derzeit nicht von einer anderen Anwendung verwendet wird."), false),
        Fail::Denied => ("Webex den Zugriff auf Ihre Kamera erlauben".into(), "Erlauben Sie in Ihren Systemeinstellungen den Zugriff auf Ihre Kamera, damit andere Sie in Meetings sehen können.".into(), cfg!(windows)),
        Fail::Unplugged(n) => ("Ihre Kamera ist nicht angeschlossen".into(), format!("Stellen Sie sicher, dass Ihr Kameragerät {n} verbunden ist und funktioniert."), false),
        Fail::Other(_) => ("Ihre Kamera kann nicht gestartet werden".into(), "Stellen Sie sicher, dass Ihr Kameragerät funktioniert.".into(), false),
    }
}

impl App {
    fn cam_sample(&self) -> bool { self.call.sample || self.shot.is_some() }

    /// The look of the preview from the page's settings.
    pub(crate) fn cam_look(&self) -> Look {
        let s = &self.settings;
        Look {
            mirror: self.sb("camera_mirror", true),
            rotation: s["camera_rotation"].as_u64().unwrap_or(0) as u8 % 4,
            fill: self.sb("camera_zoom_fit", true),
            lighting: self.sb("video_lighting", false),
            bcs: self.sb("video_bcs", false).then(|| [pct(&s["video_brightness"]), pct(&s["video_contrast"]), pct(&s["video_saturation"])]),
            touch_up: self.sb("video_touch_up", false).then(|| pct(&s["video_touch_up_level"])),
        }
    }

    /// Camera list on a worker thread, every 2 s while the page is shown.
    fn cam_refresh(&mut self, now: f64) {
        if self.cam_sample() {
            let c = &mut self.call.cam;
            if !c.listed { c.devs = vec![CamDev { id: "sample".into(), name: SAMPLE_CAM.into() }]; c.listed = true; }
            return;
        }
        let c = &mut self.call.cam;
        if let Some(rx) = &c.job {
            match rx.try_recv() {
                Ok(v) => { c.devs = v; c.job = None; c.listed = true; }
                Err(TryRecvError::Disconnected) => { c.job = None; c.listed = true; }
                Err(TryRecvError::Empty) => {}
            }
        }
        if c.job.is_none() && (c.listed_at == 0.0 || now - c.listed_at > 2.0) {
            c.listed_at = now;
            let (tx, rx) = channel();
            let _ = std::thread::Builder::new().name("camera-list".into()).spawn(move || { let _ = tx.send(camera::list()); crate::ui::wake(); });
            c.job = Some(rx);
        }
    }

    /// The chosen camera: the saved name if it is plugged in, else the first one.
    fn cam_selected(&self) -> Option<CamDev> {
        let want = self.ss("camera", "");
        let d = &self.call.cam.devs;
        d.iter().find(|x| x.name == want).or(d.first()).cloned()
    }

    fn cam_max_h(&self) -> u32 { self.ss("camera_resolution", "auto").parse().unwrap_or(1080) }

    /// The pinned preview box (drawn by `settings_content` above the scrolling page).
    pub(crate) fn video_preview(&mut self, ui: &mut egui::Ui, t: &Th, now: f64, r: Rect) {
        self.call.cam.seen = now;
        self.cam_refresh(now);
        let sample = self.cam_sample();
        let key = PreviewKey { dev: self.cam_selected(), max_h: self.cam_max_h(), hw: self.sb("camera_hw_accel", true), sample };
        let look = self.cam_look();
        let ppp = ui.ctx().pixels_per_point();
        let px = ((r.width() * ppp).round() as usize, (r.height() * ppp).round() as usize);
        let c = &mut self.call.cam;
        if c.listed {
            let failed = c.preview.as_ref().is_some_and(|p| matches!(p.state(), State::Failed(_)));
            let stale = c.preview.as_ref().is_some_and(|p| p.key != key);
            // a failed camera is retried every 3 s (in use by another app, unplugged and back, permission granted)
            if c.preview.is_none() || stale || (failed && now - c.failed_at > 3.0) {
                c.preview = Some(Preview::start(key, look, px, crate::ui::wake));
                c.failed_at = now;
                if stale { c.tex = None; }
                c.seq = 0;
            }
        }
        let state = c.preview.as_ref().map_or(State::Opening, |p| p.state());
        if let Some(p) = &c.preview {
            p.set_look(look);
            p.set_size(px.0, px.1);
            if let Some(f) = p.take(c.seq) {
                c.seq = f.seq;
                let img = egui::ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba);
                match &mut c.tex { Some(tx) => tx.set(img, egui::TextureOptions::LINEAR), None => c.tex = Some(ui.ctx().load_texture("camera-preview", img, egui::TextureOptions::LINEAR)) }
            }
        }
        if matches!(state, State::Failed(_)) { c.tex = None; } else { c.failed_at = now; }
        let tex = c.tex.as_ref().map(|x| x.id());
        // Near-black box with a black 50 % band (background-tertiary / overlayFade-secondary are not in the token table).
        let bg = if t.light { Color32::from_gray(232) } else { Color32::from_gray(38) };
        let resp = ui.allocate_rect(r, Sense::hover());
        match &state {
            State::Failed(f) => self.video_error(ui, t, r, f),
            _ => {
                ui.painter().rect_filled(r, 8.0, if tex.is_some() { Color32::BLACK } else { bg });
                if let Some(id) = tex { egui::Image::from_texture(egui::load::SizedTexture::new(id, r.size())).corner_radius(8).paint_at(ui, r); }
                let band = Rect::from_min_max(pos2(r.left(), r.bottom() - 28.0), r.max);
                ui.painter().rect_filled(band, egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }, Color32::from_black_alpha(128));
                ui.painter().text(band.center(), egui::Align2::CENTER_CENTER, "Vorschau", semibold(kit::SECOND), Color32::WHITE);
                // rotate button (rotateButtonVisible: shown while the camera runs)
                if matches!(state, State::Running { .. }) {
                    let br = Rect::from_center_size(pos2(r.right() - 8.0 - 14.0, r.top() + 8.0 + 14.0), Vec2::splat(28.0));
                    let b = ui.interact(br, egui::Id::new("camera-rotate"), Sense::click()).on_hover_text("Einmal klicken, um die Videoansicht um 90 Grad zu drehen");
                    ui.painter().circle_filled(br.center(), 14.0, Color32::from_black_alpha(if b.is_pointer_button_down_on() { 200 } else if b.hovered() { 166 } else { 128 }));
                    self.icons.paint(ui, "rotate-content-bold", Rect::from_center_size(br.center(), Vec2::splat(16.0)), Color32::WHITE);
                    if b.clicked() { let r = (look.rotation + 1) % 4; self.sset("camera_rotation", json!(r)); }
                }
                if let State::Running { mode, name, .. } = &state {
                    // Webox addition: what the capture runs at, on hover only.
                    resp.on_hover_text(format!("{name} · {}×{} {} · {:.0} fps", mode.w, mode.h, mode.pix.name(), mode.fps));
                }
            }
        }
    }

    fn video_error(&mut self, ui: &mut egui::Ui, t: &Th, r: Rect, f: &Fail) {
        let bg = if t.light { Color32::from_gray(232) } else { Color32::from_gray(38) };
        ui.painter().rect_filled(r, 8.0, bg);
        let (title, text, action) = fail_text(f);
        let w = r.width() - 48.0;
        let tg = ui.painter().layout(title, semibold(16.0), t.text1, w);
        let xg = ui.painter().layout(text, FontId::proportional(kit::SECOND), t.text1, w);
        let h = 40.0 + 4.0 + tg.size().y.max(24.0) + 4.0 + xg.size().y.max(20.0) + if action { 12.0 + 32.0 } else { 0.0 };
        let mut y = r.center().y - h / 2.0;
        self.icons.paint(ui, "warning-regular", Rect::from_min_size(pos2(r.center().x - 20.0, y), Vec2::splat(40.0)), t.text1);
        y += 44.0;
        let th = tg.size().y.max(24.0);
        ui.painter().galley(pos2(r.center().x - tg.size().x / 2.0, y + (th - tg.size().y) / 2.0), tg, t.text1);
        y += th + 4.0;
        let xw = xg.size().x;
        ui.painter().galley(pos2(r.center().x - xw / 2.0, y), xg.clone(), t.text1);
        y += xg.size().y.max(20.0) + 12.0;
        if action {
            let label = "Systemeinstellungen öffnen";
            let bw = ui.fonts(|fo| fo.layout_no_wrap(label.into(), semibold(kit::BODY), t.text1).size().x) + 32.0;
            let br = Rect::from_min_size(pos2(r.center().x - bw / 2.0, y), Vec2::new(bw, 32.0));
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(br));
            if kit::pill(&mut c, t, label, true, true).clicked() { system::open("ms-settings:privacy-webcam"); }
        }
        if let Fail::Other(e) = f { ui.interact(r, egui::Id::new("camera-error"), Sense::hover()).on_hover_text(e.as_str()); }
    }

    /// A 32 px row: 120 px label, then a 0..100 slider over the rest (brightnessRow / touchUpLevelRow).
    fn adjust_row(&mut self, ui: &mut egui::Ui, t: &Th, key: &str, label: &str, indent: f32) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(indent);
            let (lr, _) = ui.allocate_exact_size(Vec2::new(120.0, 32.0), Sense::hover());
            crate::ui::chrome::fit_text(ui.painter(), lr.left(), lr.center().y, label, FontId::proportional(kit::BODY), t.text1, 116.0);
            ui.add_space(8.0);
            let mut v = pct(&self.settings[key]);
            ui.allocate_ui(Vec2::new(ui.available_width(), 32.0), |ui| {
                ui.add_space(6.0);
                if kit::slider(ui, t, &mut v, 0.0, 100.0, 101, ui.available_width()) { self.sset(key, json!(v.round() as i64)); }
            });
        });
    }

    pub(super) fn page_video(&mut self, ui: &mut egui::Ui, t: &Th) {
        section(ui, t, "Kamera", "", true, |ui| {
            let devs = self.call.cam.devs.clone();
            let sel = self.cam_selected();
            let names: Vec<String> = if devs.is_empty() { vec!["Keine Kameras gefunden".into()] } else { devs.iter().map(|d| d.name.clone()).collect() };
            let cur = sel.and_then(|s| devs.iter().position(|d| d.id == s.id)).unwrap_or(0);
            let w = ui.available_width();
            ui.add_enabled_ui(!devs.is_empty(), |ui| {
                if let Some(i) = kit::combo(ui, &mut self.icons, t, "camera", w, &names, cur) { if let Some(d) = devs.get(i) { self.sset("camera", json!(d.name)); } }
            });
            ui.add_space(8.0);
            let mut m = self.sb("camera_mirror", true);
            if kit::check_more(ui, &mut self.icons, t, "Meine Videovorschau spiegeln", &mut m, MIRROR_INFO) { self.sset("camera_mirror", json!(m)); }
            ui.add_space(4.0);
            self.pcheck(ui, t, "camera_zoom_fit", true, "Auf optimale Darstellung zoomen");
            self.pcheck(ui, t, "camera_hw_accel", true, "Hardware-Beschleunigung aktivieren");
            ui.add_space(4.0);
            // Max. Kameraauflösung: Auto shows the camera's own best height ("Auto ({{0}}P)"); the list holds what it reaches
            let modes = match self.call.cam.preview.as_ref().map(|p| p.state()) { Some(State::Running { modes, .. }) => modes, _ => vec![] };
            let heights = if modes.is_empty() { camera::HEIGHTS.to_vec() } else { camera::offered_heights(&modes) };
            let auto = if modes.is_empty() { 1080 } else { camera::auto_height(&modes) };
            let mut opts: Vec<(String, String)> = vec![("auto".into(), format!("Auto ({auto}P)"))];
            opts.extend(heights.iter().map(|h| (h.to_string(), format!("{h}P")))); // entry labels
            let cur_v = self.ss("camera_resolution", "auto");
            let cur = opts.iter().position(|o| o.0 == cur_v).unwrap_or(0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("Max. Kameraauflösung").size(kit::BODY).color(t.text1));
                kit::more_info(ui, &mut self.icons, t, RES_INFO);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let labels: Vec<String> = opts.iter().map(|o| o.1.clone()).collect();
                    if let Some(i) = kit::combo(ui, &mut self.icons, t, "camera_resolution", 180.0, &labels, cur) { let v = opts[i].0.clone(); self.sset("camera_resolution", json!(v)); }
                });
            });
        });
        section(ui, t, "Videoverbesserungen", "", false, |ui| {
            let mut tu = self.sb("video_touch_up", false);
            if kit::check_more(ui, &mut self.icons, t, "Aussehen auffrischen", &mut tu, TOUCH_INFO) { self.sset("video_touch_up", json!(tu)); }
            if tu { ui.add_space(4.0); self.adjust_row(ui, t, "video_touch_up_level", "Ebene", 32.0); }
            ui.add_space(4.0);
            let mut li = self.sb("video_lighting", false);
            if kit::check_more(ui, &mut self.icons, t, "Belichtung anpassen", &mut li, LIGHT_INFO) { self.sset("video_lighting", json!(li)); }
            ui.add_space(4.0);
            if self.pcheck(ui, t, "video_bcs", false, "Helligkeit, Kontrast und Sättigung anpassen") || self.sb("video_bcs", false) {
                if self.sb("video_bcs", false) {
                    self.adjust_row(ui, t, "video_brightness", "Helligkeit", 0.0);
                    self.adjust_row(ui, t, "video_contrast", "Kontrast", 0.0);
                    self.adjust_row(ui, t, "video_saturation", "Farbsättigung", 0.0);
                }
            }
        });
        section(ui, t, "Selbstansicht in Meetings", "Ihre Selbstansicht ist Ihre eigene Videoansicht. Wenn Ihr Video ausgeschaltet ist, ist Ihre Selbstansicht die Ansicht, die andere anstelle Ihres Videos sehen, z. B. Ihr Profilbild oder Ihren Anzeigenamen. Diese Optionen steuern nur Ihre Anzeige Ihrer Selbstansicht und nicht, was andere sehen.", false, |ui| {
            let cur = self.ss("selfview", "others");
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 15.0;
                let w = ui.available_width() - 110.0 - 15.0;
                ui.allocate_ui(Vec2::new(w, 80.0), |ui| {
                    ui.vertical(|ui| {
                        for (v, l) in [("others", "Mit anderen anzeigen"), ("floating", "In einem unverankerten Fenster anzeigen"), ("hidden", "Selbstansicht ausblenden")] {
                            if kit::radio(ui, t, l, cur == v, "") { self.sset("selfview", json!(v)); }
                            ui.add_space(4.0);
                        }
                    });
                });
                let art = match (cur.as_str(), t.light) {
                    ("floating", false) => "show-in-floating-window-dark", ("floating", true) => "show-in-floating-window-light",
                    ("hidden", false) => "hide-video-dark", ("hidden", true) => "hide-video-light",
                    (_, false) => "show-with-others-dark", (_, true) => "show-with-others-light",
                };
                let (ar, _) = ui.allocate_exact_size(Vec2::new(110.0, 75.0), Sense::hover());
                self.icons.art(ui, art, ar);
            });
            ui.add_space(8.0);
            let (sr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
            ui.painter().hline(sr.x_range(), sr.center().y, Stroke::new(1.0_f32, t.sep));
            ui.add_space(8.0);
            self.pcheck(ui, t, "selfview_only_video_on", false, "Nur anzeigen, wenn mein Video aktiviert ist");
        });
        section(ui, t, "Erweiterte Videoeinstellungen", "", false, |ui| {
            self.pcheck(ui, t, "video_gpu_bg", true, "GPU-Beschleunigung für virtuellen Hintergrund aktivieren");
            if cfg!(windows) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Videoaufnahme").size(kit::BODY).color(t.text1));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.pcombo(ui, t, "video_capture", "mf", &[("mf", "Microsoft Media Foundation"), ("ds", "Microsoft DirectShow")], 220.0);
                    });
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhancement_values_accept_old_fractions() {
        assert_eq!(pct(&json!(0.5)), 50.0);
        assert_eq!(pct(&json!(80)), 80.0);
        assert_eq!(pct(&serde_json::Value::Null), 50.0);
    }

    #[test]
    fn every_failure_has_a_message() {
        for f in [Fail::NoCamera, Fail::InUse("Kamera A".into()), Fail::Denied, Fail::Unplugged("Kamera B".into()), Fail::Other("x".into())] {
            let (title, text, _) = fail_text(&f);
            assert!(!title.is_empty() && !text.is_empty());
        }
        assert!(fail_text(&Fail::InUse("Kamera A".into())).1.contains("Kamera A"));
    }
}
