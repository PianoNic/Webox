//! Round avatars rendered for the screen they are shown on (round picture at size × device-pixel-ratio).
//!
//! Each (picture, physical pixel size, presence cut-out, background) gets its own texture, drawn 1:1 on the pixel grid:
//! * the photo is centre-cropped to a square and resampled with CatmullRom in linear light (premultiplied), so neither the GPU's
//!   bilinear minification (aliasing) nor an upscale from logical points (blur) touches it;
//! * the circle is an antialiased coverage mask in the texture itself (exact area coverage on the edge pixels), so the edge is
//!   smooth instead of the hard edge of an untextured-AA triangle fan;
//! * a presence badge cuts a transparent ring out of the picture (what lies behind shows through), instead
//!   of a ring painted in the card colour.
//!
//! The source pixels of a picture are registered with its (full-size) texture id when it is decoded (`register`); the cache
//! lives in the egui context's temp data and drops entries that were not drawn for a while.
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Largest side kept of a source picture (CPU copy for re-rendering at other sizes / scales). A 96 pt profile avatar at 200 %
/// needs 192 px.
pub const SOURCE_MAX: u32 = 256;
/// New round textures rendered per pass at most (a list full of new faces spreads over a few frames; the rest is drawn
/// with the fallback meanwhile).
const RENDERS_PER_PASS: usize = 12;
/// Entries kept when the cache is trimmed, and how long (passes) an entry must have been unused to be dropped.
const KEEP: usize = 384;
const IDLE_PASSES: u64 = 600;

/// A circle cut out of the avatar (centre and radius in texture pixels): the ring around the presence badge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut { pub cx: f32, pub cy: f32, pub r: f32 }

/// Cache key: source texture (None = no picture, only the background disc), size in physical px, cut (1/8 px), background.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key { pub src: Option<egui::TextureId>, pub px: u32, pub cut: Option<[i32; 3]>, pub bg: [u8; 4] }

impl Key {
    pub fn new(src: Option<egui::TextureId>, px: u32, cut: Option<Cut>, bg: egui::Color32) -> Self {
        let q = |v: f32| (v * 8.0).round() as i32;
        Key { src, px, cut: cut.map(|c| [q(c.cx), q(c.cy), q(c.r)]), bg: bg.to_array() }
    }
    fn cut(&self) -> Option<Cut> { self.cut.map(|[x, y, r]| Cut { cx: x as f32 / 8.0, cy: y as f32 / 8.0, r: r as f32 / 8.0 }) }
}

#[derive(Default)]
pub struct Store {
    sources: HashMap<egui::TextureId, Arc<image::RgbaImage>>,
    tex: HashMap<Key, (egui::TextureHandle, u64)>,
    pass: u64,
    rendered: usize,
    /// Textures created so far (tests: a second draw at the same size must not render again).
    pub renders: u64,
}

impl Store {
    fn begin(&mut self, pass: u64) {
        if self.pass == pass { return }
        self.pass = pass;
        self.rendered = 0;
        if self.tex.len() > KEEP {
            let mut idle: Vec<(u64, Key)> = self.tex.iter().filter(|(_, v)| v.1 + IDLE_PASSES < pass).map(|(k, v)| (v.1, *k)).collect();
            idle.sort_by_key(|e| e.0);
            for (_, k) in idle.into_iter().take(self.tex.len() - KEEP) { self.tex.remove(&k); }
        }
    }

    /// The round texture for `key`, rendered now if it is new and the pass's budget allows (`None`: draw the fallback and
    /// ask for another frame).
    pub fn get(&mut self, ctx: &egui::Context, key: Key) -> Option<egui::TextureId> {
        self.begin(ctx.cumulative_pass_nr());
        if let Some(e) = self.tex.get_mut(&key) { e.1 = self.pass; return Some(e.0.id()); }
        let src = match key.src { Some(id) => Some(self.sources.get(&id)?.clone()), None => None };
        if self.rendered >= RENDERS_PER_PASS { ctx.request_repaint(); return None; }
        self.rendered += 1;
        self.renders += 1;
        let img = render(src.as_deref(), key.px, key.cut(), egui::Color32::from_rgba_premultiplied(key.bg[0], key.bg[1], key.bg[2], key.bg[3]));
        let t = ctx.load_texture(format!("round:{key:?}"), img, egui::TextureOptions::LINEAR);
        let id = t.id();
        self.tex.insert(key, (t, self.pass));
        Some(id)
    }

    pub fn len(&self) -> usize { self.tex.len() }
}

fn store(ctx: &egui::Context) -> Arc<Mutex<Store>> {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Arc<Mutex<Store>>>(egui::Id::new("round-avatars")).clone())
}

/// Runs `f` on the context's avatar store.
pub fn with<R>(ctx: &egui::Context, f: impl FnOnce(&mut Store) -> R) -> R {
    let s = store(ctx);
    let mut g = s.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

/// Remembers the pixels behind the picture texture `id` (centre-cropped to a square, at most `SOURCE_MAX` px).
pub fn register(ctx: &egui::Context, id: egui::TextureId, img: &image::RgbaImage) {
    let sq = square_source(img);
    with(ctx, |s| { s.tex.retain(|k, _| k.src != Some(id)); s.sources.insert(id, Arc::new(sq)); });
}

/// Drops a picture's source and its round textures (its texture was replaced or released).
pub fn forget(ctx: &egui::Context, id: egui::TextureId) {
    with(ctx, |s| { s.sources.remove(&id); s.tex.retain(|k, _| k.src != Some(id)); });
}

/// Centre square of `img`, shrunk to `SOURCE_MAX` (CatmullRom, linear light) when larger.
pub fn square_source(img: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    let s = w.min(h).max(1);
    let sq = image::imageops::crop_imm(img, (w - s.min(w)) / 2, (h - s.min(h)) / 2, s, s).to_image();
    if s <= SOURCE_MAX { return sq }
    let lin = resample_linear(&sq, SOURCE_MAX);
    let mut out = image::RgbaImage::new(SOURCE_MAX, SOURCE_MAX);
    for (o, p) in out.pixels_mut().zip(lin.pixels()) {
        let a = p[3];
        let un = |c: f32| if a > 0.0 { lin_to_srgb(c / a) } else { 0 };
        *o = image::Rgba([un(p[0]), un(p[1]), un(p[2]), (a * 255.0).round().clamp(0.0, 255.0) as u8]);
    }
    out
}

/// Resampling filter for the photos: CatmullRom comes closer to the ideal (area-averaged) avatar than Lanczos3 at every
/// size, without Lanczos' over-sharpening / ringing (round_measure.rs).
const FILTER: image::imageops::FilterType = image::imageops::FilterType::CatmullRom;

// Colour.

fn srgb_to_lin_lut() -> &'static [f32; 256] {
    static LUT: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| { let c = i as f32 / 255.0; if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } }))
}
fn lin_to_srgb(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let c = if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// `img` resampled to `px`×`px` in linear light with premultiplied alpha; the result is linear premultiplied.
fn resample_linear(img: &image::RgbaImage, px: u32) -> image::Rgba32FImage { resample_linear_with(img, px, FILTER) }
pub(crate) fn resample_linear_with(img: &image::RgbaImage, px: u32, filter: image::imageops::FilterType) -> image::Rgba32FImage {
    let lut = srgb_to_lin_lut();
    let lin = image::Rgba32FImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        let a = p[3] as f32 / 255.0;
        image::Rgba([lut[p[0] as usize] * a, lut[p[1] as usize] * a, lut[p[2] as usize] * a, a])
    });
    if img.width() == px && img.height() == px { return lin }
    image::imageops::resize(&lin, px, px, filter)
}

// Mask.

/// Area of the pixel (x, y) (unit square at x..x+1, y..y+1) inside the circle (cx, cy, r): exact inside / outside, 8×8
/// supersampled on the edge band.
pub fn circle_coverage(cx: f32, cy: f32, r: f32, x: u32, y: u32) -> f32 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    if d <= r - 0.75 { return 1.0 }
    if d >= r + 0.75 { return 0.0 }
    const N: u32 = 8;
    let mut n = 0;
    for j in 0..N {
        for i in 0..N {
            let sx = x as f32 + (i as f32 + 0.5) / N as f32;
            let sy = y as f32 + (j as f32 + 0.5) / N as f32;
            if (sx - cx).powi(2) + (sy - cy).powi(2) <= r * r { n += 1 }
        }
    }
    n as f32 / (N * N) as f32
}

/// Coverage of the round avatar of `px`×`px` pixels (disc filling the square, minus the optional cut), row-major.
pub fn mask(px: u32, cut: Option<Cut>) -> Vec<f32> {
    let c = px as f32 / 2.0;
    let mut out = Vec::with_capacity((px * px) as usize);
    for y in 0..px {
        for x in 0..px {
            let mut a = circle_coverage(c, c, c, x, y);
            if let (Some(k), true) = (cut, a > 0.0) { a *= 1.0 - circle_coverage(k.cx, k.cy, k.r, x, y); }
            out.push(a);
        }
    }
    out
}

/// The round avatar texture: `src` (square) resampled to `px`, over `bg`, masked to the antialiased disc minus `cut`.
pub fn render(src: Option<&image::RgbaImage>, px: u32, cut: Option<Cut>, bg: egui::Color32) -> egui::ColorImage { render_with(src, px, cut, bg, FILTER) }
pub(crate) fn render_with(src: Option<&image::RgbaImage>, px: u32, cut: Option<Cut>, bg: egui::Color32, filter: image::imageops::FilterType) -> egui::ColorImage {
    let px = px.max(1);
    let m = mask(px, cut);
    let lut = srgb_to_lin_lut();
    // Background, unmultiplied → linear premultiplied.
    let bga = bg.a() as f32 / 255.0;
    let un = |c: u8| if bg.a() > 0 { ((c as f32 / bga).round().clamp(0.0, 255.0)) as usize } else { 0 };
    let bgl = [lut[un(bg.r())] * bga, lut[un(bg.g())] * bga, lut[un(bg.b())] * bga, bga];
    let photo = src.map(|s| resample_linear_with(s, px, filter));
    let pixels = m.iter().enumerate().map(|(i, &cov)| {
        if cov <= 0.0 { return egui::Color32::TRANSPARENT }
        let p = photo.as_ref().map_or([0.0; 4], |ph| { let q = ph.get_pixel(i as u32 % px, i as u32 / px); [q[0], q[1], q[2], q[3].clamp(0.0, 1.0)] });
        // Photo over background (premultiplied, linear).
        let o: [f32; 4] = std::array::from_fn(|c| p[c] + bgl[c] * (1.0 - p[3]));
        let a = o[3];
        if a <= 0.0 { return egui::Color32::TRANSPARENT }
        // egui blends premultiplied colours in gamma space: store sRGB(colour) × coverage-alpha.
        let fa = a * cov;
        let g = |c: f32| (lin_to_srgb(c / a) as f32 * fa).round().clamp(0.0, 255.0) as u8;
        egui::Color32::from_rgba_premultiplied(g(o[0]), g(o[1]), g(o[2]), (fa * 255.0).round().clamp(0.0, 255.0) as u8)
    }).collect();
    egui::ColorImage { size: [px as usize, px as usize], pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_is_antialiased_and_area_true() {
        for px in [24u32, 32, 40, 50, 120] {
            let m = mask(px, None);
            let area: f32 = m.iter().sum();
            let want = std::f32::consts::PI * (px as f32 / 2.0).powi(2);
            assert!((area - want).abs() / want < 0.005, "{px}: area {area} vs {want}");
            // Centre opaque, corners empty, and the edge has partial pixels (not a hard 0/1 mask).
            assert_eq!(m[(px / 2 * px + px / 2) as usize], 1.0);
            assert_eq!(m[0], 0.0);
            let partial = m.iter().filter(|&&a| a > 0.02 && a < 0.98).count();
            assert!(partial as f32 > px as f32 * 2.0, "{px}: only {partial} edge pixels are partial");
        }
    }

    #[test]
    fn mask_edge_falls_off_within_about_one_pixel() {
        // Along the middle row, coverage goes from 1 to 0 within two pixels at each side.
        let px = 40;
        let m = mask(px, None);
        let row: Vec<f32> = (0..px).map(|x| m[(px / 2 * px + x) as usize]).collect();
        let ramp = row.iter().filter(|&&a| a > 0.0 && a < 1.0).count();
        assert!(ramp <= 4, "{row:?}");
    }

    #[test]
    fn mask_is_symmetric() {
        let px = 33;
        let m = mask(px, None);
        for y in 0..px { for x in 0..px {
            let a = m[(y * px + x) as usize];
            assert!((a - m[(y * px + (px - 1 - x)) as usize]).abs() < 1e-6);
            assert!((a - m[(x * px + y) as usize]).abs() < 1e-6);
        } }
    }

    #[test]
    fn cut_removes_the_ring_area() {
        let px = 40;
        let cut = Cut { cx: 34.8, cy: 34.8, r: 9.3 };
        let m = mask(px, Some(cut));
        // The cut's centre is transparent, the opposite side untouched.
        assert_eq!(m[(35 * px + 35) as usize], 0.0);
        assert_eq!(m[(10 * px + 10) as usize], 1.0);
        let full: f32 = mask(px, None).iter().sum();
        let with: f32 = m.iter().sum();
        assert!(with < full - 50.0);
        // The cut's edge is antialiased too.
        assert!((0..px).map(|x| m[(28 * px + x) as usize]).any(|a| a > 0.05 && a < 0.95));
    }

    #[test]
    fn render_keeps_a_flat_colour_exact_and_alpha_follows_the_mask() {
        let src = image::RgbaImage::from_pixel(80, 80, image::Rgba([200, 100, 50, 255]));
        let img = render(Some(&src), 40, None, egui::Color32::BLACK);
        let c = img.pixels[20 * 40 + 20];
        assert_eq!(c, egui::Color32::from_rgb(200, 100, 50));
        assert_eq!(img.pixels[0], egui::Color32::TRANSPARENT);
        let m = mask(40, None);
        for (p, a) in img.pixels.iter().zip(&m) { assert!((p.a() as f32 - a * 255.0).abs() <= 1.0); }
    }

    #[test]
    fn render_without_photo_is_the_background_disc() {
        let bg = egui::Color32::from_rgb(10, 120, 200);
        let img = render(None, 32, None, bg);
        assert_eq!(img.pixels[16 * 32 + 16], bg);
    }

    #[test]
    fn transparent_logo_shows_the_background() {
        let src = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 0]));
        let bg = egui::Color32::from_rgb(30, 60, 90);
        let img = render(Some(&src), 32, None, bg);
        assert_eq!(img.pixels[16 * 32 + 16], bg);
    }

    #[test]
    fn downscale_keeps_fine_detail_without_aliasing_to_a_wrong_mean() {
        // 1-px black/white stripes shrunk 4:1: a good filter gives mid grey in linear light (sRGB ~188), not black or white.
        let src = image::RgbaImage::from_fn(160, 160, |x, _| if x % 2 == 0 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255, 255, 255, 255]) });
        let img = render(Some(&src), 40, None, egui::Color32::BLACK);
        let c = img.pixels[20 * 40 + 20];
        assert!((c.r() as i32 - 188).abs() <= 3, "{c:?}");
    }

    #[test]
    fn square_source_crops_the_centre_and_caps_the_size() {
        let wide = image::RgbaImage::from_fn(300, 100, |x, _| if (100..200).contains(&x) { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) });
        let s = square_source(&wide);
        assert_eq!(s.dimensions(), (100, 100));
        assert_eq!(s.get_pixel(0, 50)[0], 255);
        let big = image::RgbaImage::from_pixel(1000, 1000, image::Rgba([10, 20, 30, 255]));
        let s = square_source(&big);
        assert_eq!(s.dimensions(), (SOURCE_MAX, SOURCE_MAX));
        let p = s.get_pixel(128, 128);
        assert!((p[0] as i32 - 10).abs() <= 1 && (p[2] as i32 - 30).abs() <= 1, "{p:?}");
    }

    #[test]
    fn cache_renders_once_per_size_and_again_for_a_new_scale() {
        let ctx = egui::Context::default();
        let tex = ctx.load_texture("t", egui::ColorImage::new([4, 4], egui::Color32::WHITE), egui::TextureOptions::LINEAR);
        register(&ctx, tex.id(), &image::RgbaImage::from_pixel(64, 64, image::Rgba([1, 2, 3, 255])));
        let bg = egui::Color32::GRAY;
        let a = with(&ctx, |s| s.get(&ctx, Key::new(Some(tex.id()), 40, None, bg))).unwrap();
        let b = with(&ctx, |s| s.get(&ctx, Key::new(Some(tex.id()), 40, None, bg))).unwrap();
        assert_eq!(a, b);
        assert_eq!(with(&ctx, |s| s.renders), 1);
        // 32 pt at 150 % instead of 125 %: a new texture.
        let c = with(&ctx, |s| s.get(&ctx, Key::new(Some(tex.id()), 48, None, bg))).unwrap();
        assert_ne!(a, c);
        // With a presence cut: another one.
        let d = with(&ctx, |s| s.get(&ctx, Key::new(Some(tex.id()), 40, Some(Cut { cx: 34.0, cy: 34.0, r: 9.0 }), bg))).unwrap();
        assert_ne!(a, d);
        assert_eq!(with(&ctx, |s| (s.renders, s.len())), (3, 3));
        // An unknown picture has no round texture (fallback drawing), and forgetting drops its entries.
        assert!(with(&ctx, |s| s.get(&ctx, Key::new(Some(egui::TextureId::User(9)), 40, None, bg))).is_none());
        forget(&ctx, tex.id());
        assert_eq!(with(&ctx, |s| s.len()), 0);
        assert!(with(&ctx, |s| s.get(&ctx, Key::new(Some(tex.id()), 40, None, bg))).is_none());
    }

    #[test]
    fn render_budget_spreads_new_textures_over_passes() {
        let ctx = egui::Context::default();
        let bg = egui::Color32::GRAY;
        let got = (1..=RENDERS_PER_PASS as u32 + 3).filter(|px| with(&ctx, |s| s.get(&ctx, Key::new(None, 10 + px, None, bg))).is_some()).count();
        assert_eq!(got, RENDERS_PER_PASS);
    }
}
