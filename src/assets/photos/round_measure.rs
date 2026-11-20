//! Quality comparison of the round avatars (run: `cargo test --bin webox round_measure -- --ignored --nocapture`;
//! `WEBOX_MEASURE_DIR=<dir>` also writes 8× enlarged PNGs of the 32 pt case).
//!
//! An avatar with a synthetic picture (the --sample photo's pattern) at 125 %, over a dark header colour:
//! * reference: 8×8 supersampled ideal render (exact circle; picture area-averaged in linear light from the 192 px source);
//! * before: a CPU model of a plain GPU path - an 80 px picture, a 48-segment triangle fan (egui gives meshes no
//!   antialiasing, so a pixel is in or out by its centre), bilinear sampling without mipmaps, over the feathered background
//!   disc, at the layout's fractional device-pixel position (32 pt avatars at x = n.3 pt sit 0.375 px off the grid at 125 %);
//! * after: `round::render` from the 192 px picture, on the pixel grid.
//! Scores against the reference at the same position: RMS error (0-255) on the edge band and inside, and the high-frequency
//! (Laplacian) energy inside relative to the reference (1.0 = as sharp as ideal, < 1 blurred, > 1 aliased / ringing).
#![cfg(test)]
use super::round;
use eframe::egui::Color32;

const CARD: [f32; 3] = [0.12, 0.12, 0.12]; // dark header, sRGB 0..1
const BG: [f32; 3] = [60.0 / 255.0; 3];

fn picture(side: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(side, side, |x, y| {
        let (a, b) = ([30.0f32, 60.0, 140.0], [120.0f32, 80.0, 200.0]);
        let (fx, fy) = ((x as f32 + 0.5) / side as f32, (y as f32 + 0.5) / side as f32);
        let k = (fx + fy) / 2.0;
        let (dx, dy) = (fx - 0.5, fy - 0.42);
        let d = (dx * dx + dy * dy).sqrt();
        let shade = if d < 0.2 { 1.35 } else if fy > 0.72 && dx.abs() < 0.32 { 0.75 } else { 1.0 };
        let ring = if (d * 60.0).sin() > 0.6 && d >= 0.2 { 0.85 } else { 1.0 };
        let stripe = if fy > 0.86 && ((fx * 48.0) as u32) % 2 == 0 { 0.7 } else { 1.0 };
        let f = shade * ring * stripe;
        let c = |i: usize| ((a[i] + (b[i] - a[i]) * k) * f).min(255.0) as u8;
        image::Rgba([c(0), c(1), c(2), 255])
    })
}

fn lin(c: f32) -> f32 { if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } }
fn srgb(l: f32) -> f32 { let l = l.clamp(0.0, 1.0); if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 } }

fn bilinear(img: &image::RgbaImage, u: f32, v: f32) -> [f32; 3] {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (x, y) = ((u * w - 0.5).clamp(0.0, w - 1.0), (v * h - 0.5).clamp(0.0, h - 1.0));
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(img.width() - 1), (y0 + 1).min(img.height() - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let p = |x, y, i: usize| img.get_pixel(x, y)[i] as f32 / 255.0;
    std::array::from_fn(|i| (p(x0, y0, i) * (1.0 - fx) + p(x1, y0, i) * fx) * (1.0 - fy) + (p(x0, y1, i) * (1.0 - fx) + p(x1, y1, i) * fx) * fy)
}

type Img = Vec<[f32; 3]>;

/// Ideal avatar of `px` device pixels whose circle box starts `off` px right/down of the image's pixel grid.
fn reference(src: &image::RgbaImage, px: u32, off: f32) -> Img {
    let r = px as f32 / 2.0;
    let c = r + off;
    let n = 8;
    (0..px * px).map(|i| {
        let (x, y) = (i % px, i / px);
        // The picture area-averaged in linear light over the whole pixel; the circle's coverage composited in gamma space,
        // as egui and Qt blend.
        let (mut acc, mut inside) = ([0.0f32; 3], 0);
        for j in 0..n { for k in 0..n {
            let (sx, sy) = (x as f32 + (k as f32 + 0.5) / n as f32, y as f32 + (j as f32 + 0.5) / n as f32);
            let col = bilinear(src, (sx - off) / px as f32, (sy - off) / px as f32);
            for q in 0..3 { acc[q] += lin(col[q]) / (n * n) as f32; }
            if (sx - c).powi(2) + (sy - c).powi(2) <= r * r { inside += 1 }
        } }
        let cov = inside as f32 / (n * n) as f32;
        std::array::from_fn(|q| srgb(acc[q]) * cov + CARD[q] * (1.0 - cov))
    }).collect()
}

/// The plain drawing: background disc (egui-feathered: coverage r - d + 0.5) then the textured 48-gon, in or out by pixel centre.
fn before(src80: &image::RgbaImage, px: u32, off: f32) -> Img {
    let r = px as f32 / 2.0;
    let c = r + off;
    let poly: Vec<(f32, f32)> = (0..48).map(|i| { let a = i as f32 / 48.0 * std::f32::consts::TAU; (c + a.cos() * r, c + a.sin() * r) }).collect();
    let inside = |x: f32, y: f32| {
        let mut ins = false;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) { ins = !ins }
        }
        ins
    };
    (0..px * px).map(|i| {
        let (x, y) = ((i % px) as f32 + 0.5, (i / px) as f32 + 0.5);
        if inside(x, y) { return bilinear(src80, (x - off) / px as f32, (y - off) / px as f32) }
        let d = ((x - c).powi(2) + (y - c).powi(2)).sqrt();
        let cov = (r - d + 0.5).clamp(0.0, 1.0);
        std::array::from_fn(|q| BG[q] * cov + CARD[q] * (1.0 - cov))
    }).collect()
}

fn after(src: &image::RgbaImage, px: u32) -> Img { after_with(src, px, image::imageops::FilterType::CatmullRom) }
fn after_with(src: &image::RgbaImage, px: u32, f: image::imageops::FilterType) -> Img {
    let bg = Color32::from_rgb(60, 60, 60);
    let img = round::render_with(Some(&round::square_source(src)), px, None, bg, f);
    img.pixels.iter().map(|p| { let a = p.a() as f32 / 255.0; let c = [p.r(), p.g(), p.b()]; std::array::from_fn(|q| c[q] as f32 / 255.0 + CARD[q] * (1.0 - a)) }).collect()
}

fn scores(img: &Img, refi: &Img, px: u32, off: f32) -> (f32, f32, f32) {
    let c = px as f32 / 2.0;
    let (mut e_edge, mut n_edge, mut e_in, mut n_in, mut hf, mut hf_ref) = (0.0, 0, 0.0, 0, 0.0, 0.0);
    let lap = |im: &Img, x: u32, y: u32| (0..3).map(|q| { let v = |x: u32, y: u32| im[(y * px + x) as usize][q]; 4.0 * v(x, y) - v(x - 1, y) - v(x + 1, y) - v(x, y - 1) - v(x, y + 1) }).map(|l| l * l).sum::<f32>();
    for y in 1..px - 1 { for x in 1..px - 1 {
        let d = (((x as f32 + 0.5) - c - off).powi(2) + ((y as f32 + 0.5) - c - off).powi(2)).sqrt();
        let i = (y * px + x) as usize;
        let e: f32 = (0..3).map(|q| (img[i][q] - refi[i][q]).powi(2)).sum::<f32>() / 3.0;
        if (d - c).abs() < 1.5 { e_edge += e; n_edge += 1 } else if d < c - 3.0 { e_in += e; n_in += 1; hf += lap(img, x, y); hf_ref += lap(refi, x, y); }
    } }
    ((e_edge / n_edge as f32).sqrt() * 255.0, (e_in / n_in as f32).sqrt() * 255.0, hf / hf_ref)
}

#[test]
#[ignore]
fn avatars_before_after_at_125_percent() {
    let src192 = picture(192);
    let src80 = image::imageops::resize(&src192, 80, 80, image::imageops::FilterType::Lanczos3);
    println!("avatar @125 %           | edge RMS before → after | interior RMS before → after | HF energy vs ideal before → after");
    let mut worse = vec![];
    for (pt, off) in [(24.0f32, 0.375f32), (32.0, 0.0), (32.0, 0.375), (40.0, 0.375), (96.0, 0.375)] {
        let px = (pt * 1.25).round() as u32;
        let b = scores(&before(&src80, px, off), &reference(&src192, px, off), px, off);
        let a = scores(&after(&src192, px), &reference(&src192, px, 0.0), px, 0.0);
        println!("{pt:>3} pt = {px:>3} px, off {off:.3} | {:6.2} → {:5.2} | {:6.2} → {:5.2} | {:5.2} → {:5.2}", b.0, a.0, b.1, a.1, b.2, a.2);
        let cr = scores(&after_with(&src192, px, image::imageops::FilterType::Lanczos3), &reference(&src192, px, 0.0), px, 0.0);
        println!("                  (Lanczos3 instead: edge {:5.2}, interior {:5.2}, HF {:5.2})", cr.0, cr.1, cr.2);
        if a.0 >= b.0 { worse.push(pt) }
        if pt == 32.0 && off > 0.0 {
            if let Ok(dir) = std::env::var("WEBOX_MEASURE_DIR") {
                let save = |img: &Img, name: &str| {
                    let out = image::RgbImage::from_fn(px, px, |x, y| { let p = img[(y * px + x) as usize]; image::Rgb(p.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8)) });
                    let _ = image::imageops::resize(&out, px * 8, px * 8, image::imageops::FilterType::Nearest).save(std::path::Path::new(&dir).join(name));
                };
                save(&reference(&src192, px, 0.0), "avatar-ideal.png");
                save(&before(&src80, px, off), "avatar-before.png");
                save(&after(&src192, px), "avatar-after.png");
            }
        }
    }
    assert!(worse.is_empty(), "edge got worse at {worse:?}");
}
