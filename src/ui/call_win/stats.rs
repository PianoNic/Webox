//! "Show Call Statistics": the call statistics panel of the call window (the official `MediaStatistics` dialog: a title
//! band, "Transmit" and "Receive" side by side, each with Audio and Video (main / content) columns, rows of 12 px figures,
//! a "Close" button). Opened from the title bar's signal button; drawn over the call window, not a window of its own.
//!
//! The figures come from `cloud::media_stats` (the call's thread reads them every 2 s while the panel is open, and only
//! then); this file turns two readings into rates. The call window already repaints every 200 ms, so the panel adds no
//! redraws of its own.
use super::*;
use crate::cloud::media_stats::{Flow, Snapshot, AUDIO, SHARE, VIDEO};

/// Row height (the official labels: 12 px, line-height 2.33).
const ROW_H: f32 = 28.0;
const TITLE_H: f32 = 36.0;
const BAND_H: f32 = 32.0;
const LABEL_W: f32 = 96.0;
/// Side by side from this panel width on, else "Receive" below "Transmit".
const TWO_COLUMNS: f32 = 640.0;
/// "Poor network connection" from this loss (%) or round trip (ms) on (calling-gap.md C26).
const POOR_LOSS: f32 = 5.0;
const POOR_RTT: f32 = 400.0;

pub(crate) const ROWS: [&str; 7] = ["Packet loss", "Jitter", "Latency", "Bit rate", "Codec", "Resolution", "Frame rate"]; // shown through tr()
const COLS: [&str; 3] = ["Audio", "Video", "Content"];
const DASH: &str = "–";

/// The panel's state in the call window.
#[derive(Default)]
pub(crate) struct StatsUi {
    pub open: bool,
    /// The two newest readings (rates are their differences), each with the received share's picture count then.
    prev: Option<(Snapshot, u64)>,
    cur: Option<(Snapshot, u64)>,
}

impl StatsUi {
    pub(crate) fn close(&mut self) { *self = StatsUi::default(); }
    /// Takes a new reading when there is one.
    fn take(&mut self, s: Snapshot, rx_pictures: u64) {
        if self.cur.as_ref().is_some_and(|(c, _)| c.at == s.at) { return; }
        self.prev = self.cur.take();
        self.cur = Some((s, rx_pictures));
    }
}

/// The figures of one table: `[direction][row][column]`, direction 0 = sent, 1 = received.
pub(crate) type Table = [[[String; 3]; 7]; 2];

fn secs(prev: &Snapshot, cur: &Snapshot) -> Option<f64> {
    let d = cur.at?.checked_duration_since(prev.at?)?.as_secs_f64();
    (d > 0.2).then_some(d)
}

/// "1,5 %" / "12 ms" / "48 kbit/s": German decimal comma.
fn num(v: f64, digits: usize, unit: &str) -> String { format!("{:.*} {unit}", digits, v).replace('.', ",") }

/// The panel's figures from the newest reading and the one before (`None` = only one so far: no rates yet). `rx_size`:
/// the received share's picture size; `rx_pictures`: its decoded pictures at each reading.
pub(crate) fn table(prev: Option<&(Snapshot, u64)>, cur: &(Snapshot, u64), rx_size: Option<[usize; 2]>) -> Table {
    let mut out: Table = Default::default();
    let (c, cp) = (&cur.0, cur.1);
    let dt = prev.and_then(|(p, _)| secs(p, c));
    for dir in 0..2 {
        for col in 0..3 {
            let i = [AUDIO, VIDEO, SHARE][col];
            let f: Flow = if dir == 0 { c.tx[i] } else { c.rx[i] };
            let pf: Option<Flow> = prev.map(|(p, _)| if dir == 0 { p.tx[i] } else { p.rx[i] });
            let mut o: [String; 7] = std::array::from_fn(|_| DASH.to_string());
            if f.packets == 0 { for (r, v) in o.into_iter().enumerate() { out[dir][r][col] = v; } continue; } // this stream doesn't run
            // loss: what the far end reported about ours; ours since the previous reading (since the start before that)
            let loss = if dir == 0 { f.fraction_lost.map(|x| x as f64 * 100.0) } else {
                let (e, l) = match pf { Some(p) if f.expected > p.expected => (f.expected - p.expected, f.lost.saturating_sub(p.lost)), _ => (f.expected, f.lost) };
                (e > 0).then(|| l as f64 / e as f64 * 100.0)
            };
            if let Some(x) = loss { o[0] = num(x, 1, "%"); }
            if let Some(j) = f.jitter_ms { o[1] = num(j as f64, 0, "ms"); }
            if let Some(r) = c.rtt_ms { o[2] = num(r as f64, 0, "ms"); }
            if let (Some(d), Some(p)) = (dt, pf) { o[3] = num(f.bytes.saturating_sub(p.bytes) as f64 * 8.0 / d / 1000.0, 0, "kbit/s"); }
            o[4] = if i == AUDIO { "Opus" } else { "H.264" }.into();
            if i == SHARE {
                let size = if dir == 0 { f.size.map(|(w, h)| [w as usize, h as usize]) } else { rx_size };
                if let Some([w, h]) = size { o[5] = format!("{w} × {h}"); }
                let fps = match (dt, prev) {
                    (Some(d), Some((p, _))) if dir == 0 => Some(f.frames.saturating_sub(p.tx[SHARE].frames) as f64 / d),
                    (Some(d), Some((_, pp))) => Some(cp.saturating_sub(*pp) as f64 / d),
                    _ => None,
                };
                if let Some(x) = fps { o[6] = num(x, 0, "fps"); }
            }
            for (r, v) in o.into_iter().enumerate() { out[dir][r][col] = v; }
        }
    }
    out
}

/// The line on top: "Network & CPU is good", or "Poor network connection" (loss over 5 % or a round trip over 400 ms).
pub(crate) fn verdict(t: &Table, rtt_ms: Option<f32>) -> (bool, &'static str) {
    let worst = t.iter().flat_map(|d| d[0].iter()).filter_map(|s| s.trim_end_matches(" %").replace(',', ".").parse::<f32>().ok()).fold(0.0, f32::max);
    if worst > POOR_LOSS || rtt_ms.is_some_and(|r| r > POOR_RTT) { (false, tr("Poor network connection")) } else { (true, tr("Network & CPU is good")) }
}

/// A 40 × `h` icon button of the call window's title bar.
pub(super) fn title_button(ui: &mut egui::Ui, ic: &mut icons::Icons, t: &Th, r: Rect, id: &str, icon: &'static str, on: bool, tip: &str) -> egui::Response {
    let resp = ui.interact(r, ui.id().with(("title-btn", id)), Sense::click());
    if resp.hovered() || on { ui.painter().rect_filled(r, 0.0, t.selected); }
    if resp.has_focus() { ui.painter().rect_stroke(r.shrink(2.0), 2.0, Stroke::new(2.0_f32, t.focus), egui::StrokeKind::Inside); }
    ic.paint(ui, icon, Rect::from_center_size(r.center(), Vec2::splat(16.0)), t.text1);
    resp.on_hover_text(tip)
}

impl App {
    /// Each frame (from `call_windows`): the call's thread reads figures only while the panel is open on a live call.
    pub(super) fn call_stats_frame(&mut self, live: bool) {
        if !live && self.call.win.stats.open { self.call.win.stats.close(); }
        self.call.win.full_toggle &= live;
        crate::cloud::media_stats::want(self.call.win.stats.open);
    }

    /// The title bar's own buttons left of the caption buttons: call statistics (connected calls) and full screen.
    pub(super) fn call_title_buttons(&mut self, ui: &mut egui::Ui, t: &Th, bar: Rect, connected: bool, full: bool) {
        let at = |n: f32| Rect::from_min_size(pos2(bar.right() - WIN_BTN_W * (3.0 + n), bar.top()), Vec2::new(WIN_BTN_W, bar.height()));
        let (icon, tip) = if full { ("fullscreen-exit-regular", tr("Exit full screen")) } else { ("fullscreen-regular", tr("Full screen")) };
        if title_button(ui, &mut self.icons, t, at(1.0), "full", icon, false, tip).clicked() { self.call.win.full_toggle = true; }
        if connected {
            let open = self.call.win.stats.open;
            if title_button(ui, &mut self.icons, t, at(2.0), "stats", "cellular-regular", open, tr("Show Call Statistics")).clicked() {
                if open { self.call.win.stats.close() } else { self.call.win.stats.open = true }
            }
        }
    }

    /// The statistics panel over `area` of the call window, while open.
    pub(super) fn call_stats_panel(&mut self, ui: &mut egui::Ui, t: &Th, area: Rect) {
        if !self.call.win.stats.open { return; }
        let rx_pic = crate::cloud::share_rx::latest();
        if let Some(s) = crate::cloud::media_stats::latest() { self.call.win.stats.take(s, rx_pic.as_ref().map_or(0, |p| p.seq)); }
        let two = area.width() - 24.0 >= TWO_COLUMNS;
        let w = if two { (area.width() - 24.0).min(880.0) } else { (area.width() - 24.0).min(520.0) };
        let table_h = BAND_H + ROW_H * (ROWS.len() + 1) as f32;
        let content_h = ROW_H + if two { table_h } else { 2.0 * table_h + 8.0 } + 48.0;
        let h = (TITLE_H + content_h).min(area.height() - 24.0);
        let r = Rect::from_center_size(area.center(), Vec2::new(w, h));
        // the panel takes the pointer: nothing behind it reacts
        let _ = ui.interact(r, ui.id().with("stats-panel"), Sense::click_and_drag());
        let p = ui.painter();
        p.rect_filled(r.expand(1.0), 6.0, t.sep);
        p.rect_filled(r, 6.0, t.card);
        let title = Rect::from_min_size(r.min, Vec2::new(w, TITLE_H));
        p.rect_filled(title, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, t.selected);
        p.text(pos2(title.left() + 16.0, title.center().y), egui::Align2::LEFT_CENTER, tr("Statistics"), F::Body.semibold(), t.text1);
        let x = Rect::from_min_size(pos2(title.right() - TITLE_H, title.top()), Vec2::splat(TITLE_H));
        if title_button(ui, &mut self.icons, t, x, "stats-x", "cancel-regular", false, tr("Close")).clicked() { self.call.win.stats.close(); return; }
        let st = &self.call.win.stats;
        let tab = st.cur.as_ref().map(|c| (table(st.prev.as_ref(), c, rx_pic.as_ref().map(|p| p.image.size)), c.0.rtt_ms));
        let body = Rect::from_min_max(pos2(r.left(), title.bottom()), r.max);
        let mut close = false;
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(body));
        egui::ScrollArea::vertical().id_salt("call-stats").auto_shrink([false, false]).show(&mut c, |ui| {
            let (room, _) = ui.allocate_exact_size(Vec2::new(w, content_h), Sense::hover());
            let p = ui.painter();
            let line = Rect::from_min_size(room.min, Vec2::new(w, ROW_H));
            let (good, says) = match &tab { Some((tb, rtt)) => verdict(tb, *rtt), None => (true, tr("Loading statistics…")) };
            p.circle_filled(pos2(line.left() + 20.0, line.center().y), 4.0, if tab.is_none() { t.text2 } else if good { t.green } else { t.badge });
            p.text(pos2(line.left() + 32.0, line.center().y), egui::Align2::LEFT_CENTER, says, F::Caption.font(), t.text1);
            let half = if two { (w - 8.0) / 2.0 } else { w };
            for (d, name) in [tr("Transmit"), tr("Receive")].into_iter().enumerate() {
                let o = if two { pos2(room.left() + d as f32 * (half + 8.0), line.bottom()) } else { pos2(room.left(), line.bottom() + d as f32 * (table_h + 8.0)) };
                let band = Rect::from_min_size(o, Vec2::new(half, BAND_H));
                p.rect_filled(band, 0.0, t.win_b);
                p.text(pos2(band.left() + 16.0, band.center().y), egui::Align2::LEFT_CENTER, name, F::Title.font(), t.text1);
                let col_w = (half - 16.0 - LABEL_W) / 3.0;
                let cx = |k: usize| band.left() + 16.0 + LABEL_W + col_w * (k as f32 + 0.5);
                for (k, h) in COLS.iter().enumerate() { p.text(pos2(cx(k), band.bottom() + ROW_H / 2.0), egui::Align2::CENTER_CENTER, tr(h), F::Caption.semibold(), t.text2); }
                for (row, label) in ROWS.iter().enumerate() {
                    let y = band.bottom() + ROW_H * (row as f32 + 1.5);
                    p.text(pos2(band.left() + 16.0, y), egui::Align2::LEFT_CENTER, tr(label), F::Caption.font(), t.text2);
                    for k in 0..3 {
                        let v = tab.as_ref().map_or(DASH, |(tb, _)| tb[d][row][k].as_str());
                        p.text(pos2(cx(k), y), egui::Align2::CENTER_CENTER, v, F::Caption.font(), t.text1);
                    }
                }
            }
            // "Close" (the official closeButton: primary, radius 2, 14 px)
            let b = Rect::from_min_size(pos2(room.right() - 16.0 - 96.0, room.bottom() - 40.0), Vec2::new(96.0, 32.0));
            let br = ui.interact(b, ui.id().with("stats-close"), Sense::click());
            ui.painter().rect_filled(b, 16.0, if br.hovered() { t.accent.gamma_multiply(0.85) } else { t.accent });
            ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, tr("Close"), F::Body.semibold(), Color32::WHITE);
            if br.has_focus() { ui.painter().rect_stroke(b.expand(1.5), 17.5, Stroke::new(2.0_f32, t.focus), egui::StrokeKind::Inside); }
            close = br.clicked();
        });
        if close { self.call.win.stats.close(); }
    }

    /// `WEBOX_SAMPLE_CALL=stats`: the panel open with made-up readings (two, 2 s apart, so rates show).
    pub(super) fn sample_stats(&mut self) {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let mk = |k: u64, at: Instant| {
            let mut s = Snapshot { at: Some(at), rtt_ms: Some(38.0), ..Default::default() };
            s.tx[AUDIO] = Flow { bytes: 8_000 * k, packets: 50 * k, fraction_lost: Some(0.004), jitter_ms: Some(3.0), ..Default::default() };
            s.rx[AUDIO] = Flow { bytes: 7_600 * k, packets: 50 * k - k / 2, expected: 50 * k, lost: k / 2, jitter_ms: Some(6.0), ..Default::default() };
            s.tx[SHARE] = Flow { bytes: 150_000 * k, packets: 130 * k, fraction_lost: Some(0.0), jitter_ms: Some(9.0), size: Some((1920, 1080)), frames: 10 * k, ..Default::default() };
            s
        };
        let st = &mut self.call.win.stats;
        st.open = true;
        st.prev = Some((mk(10, t0), 0));
        st.cur = Some((mk(12, t0 + Duration::from_secs(2)), 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn reading(at: Instant, k: u64) -> Snapshot {
        let mut s = Snapshot { at: Some(at), rtt_ms: Some(42.0), ..Default::default() };
        s.tx[AUDIO] = Flow { bytes: 10_000 * k, packets: 50 * k, fraction_lost: Some(0.02), jitter_ms: Some(4.4), ..Default::default() };
        s.rx[AUDIO] = Flow { bytes: 8_000 * k, packets: 49 * k, expected: 50 * k, lost: k, jitter_ms: Some(12.0), ..Default::default() };
        s.tx[SHARE] = Flow { bytes: 1, packets: 1, size: Some((1280, 720)), frames: 16 * k, ..Default::default() };
        s
    }

    #[test]
    fn rates_from_two_readings() {
        let t0 = Instant::now();
        let (a, b) = ((reading(t0, 1), 10u64), (reading(t0 + Duration::from_secs(2), 2), 70u64));
        let tb = table(Some(&a), &b, Some([1920, 1080]));
        // sent audio: the far end's report
        assert_eq!(tb[0][0][0], "2,0 %");
        assert_eq!(tb[0][1][0], "4 ms");
        assert_eq!(tb[0][2][0], "42 ms");
        assert_eq!(tb[0][3][0], "40 kbit/s", "10 000 bytes in 2 s");
        assert_eq!(tb[0][4][0], "Opus");
        assert_eq!(tb[0][5][0], DASH, "audio has no picture");
        // received audio: 1 of 50 lost in the last 2 s
        assert_eq!(tb[1][0][0], "2,0 %");
        assert_eq!(tb[1][1][0], "12 ms");
        assert_eq!(tb[1][3][0], "32 kbit/s");
        // no main video in either direction
        assert!(tb.iter().all(|d| d.iter().all(|r| r[1] == DASH)));
        // the share: our picture size and rate; nothing received (no packets) shows dashes
        assert_eq!((tb[0][4][2].as_str(), tb[0][5][2].as_str(), tb[0][6][2].as_str()), ("H.264", "1280 × 720", "8 fps"));
        assert!(tb[1].iter().all(|r| r[2] == DASH), "{:?}", tb[1]);
        assert_eq!(verdict(&tb, Some(42.0)), (true, tr("Network & CPU is good")));
        assert_eq!(verdict(&tb, Some(450.0)).0, false, "a round trip over 400 ms");
    }

    #[test]
    fn first_reading_has_no_rates_and_loss_is_cumulative() {
        let a = (reading(Instant::now(), 3), 0u64);
        let tb = table(None, &a, None);
        assert_eq!(tb[1][0][0], "2,0 %", "3 of 150 since the start");
        assert_eq!(tb[0][3][0], DASH, "no bit rate from one reading");
        let mut bad = a.clone();
        bad.0.rx[AUDIO].lost = 30;
        assert_eq!(verdict(&table(None, &bad, None), Some(20.0)), (false, tr("Poor network connection")), "20 % loss");
    }

    #[test]
    fn received_share_rate_from_decoded_pictures() {
        let t0 = Instant::now();
        let mut a = reading(t0, 1);
        a.rx[SHARE] = Flow { bytes: 100_000, packets: 100, expected: 100, ..Default::default() };
        let mut b = reading(t0 + Duration::from_secs(2), 2);
        b.rx[SHARE] = Flow { bytes: 600_000, packets: 600, expected: 600, ..Default::default() };
        let tb = table(Some(&(a, 100)), &(b, 160), Some([1920, 1080]));
        assert_eq!((tb[1][5][2].as_str(), tb[1][6][2].as_str(), tb[1][3][2].as_str()), ("1920 × 1080", "30 fps", "2000 kbit/s"));
        assert_eq!(tb[1][0][2], "0,0 %");
    }
}
