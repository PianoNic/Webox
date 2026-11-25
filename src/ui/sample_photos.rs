//! Offline-sample profile pictures and last-active times (generated; no network).

/// Offline sample profile pictures keyed by person id, so the avatar code can be checked without an account: generated
/// 192 px pictures (the size `cloud::people` downloads) - a gradient with a ring pattern, a "head" and fine stripes, so
/// sharpness and resampling are visible in screenshots (a smooth gradient hides blur).
pub(crate) fn sample_photos() -> Vec<(String, Vec<u8>)> {
    let mk = |key: &str, a: [u8; 3], b: [u8; 3]| {
        const S: u32 = 192;
        let img = image::RgbaImage::from_fn(S, S, |x, y| {
            let (fx, fy) = (x as f32 / S as f32, y as f32 / S as f32);
            let k = (fx + fy) / 2.0;
            let mut c = [0.0f32; 3];
            for i in 0..3 { c[i] = a[i] as f32 + (b[i] as f32 - a[i] as f32) * k; }
            let (dx, dy) = (fx - 0.5, fy - 0.42);
            let d = (dx * dx + dy * dy).sqrt();
            let shade = if d < 0.2 { 1.35 } else if (fy > 0.72) && (dx.abs() < 0.32) { 0.75 } else { 1.0 };
            let ring = if (d * 60.0).sin() > 0.6 && d >= 0.2 { 0.85 } else { 1.0 };
            let stripe = if fy > 0.86 && x % 4 < 2 { 0.7 } else { 1.0 };
            let f = shade * ring * stripe;
            image::Rgba([(c[0] * f).min(255.0) as u8, (c[1] * f).min(255.0) as u8, (c[2] * f).min(255.0) as u8, 255])
        });
        let mut v = Vec::new();
        let _ = img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png);
        (key.to_string(), v)
    };
    vec![mk("jonas@example.com", [30, 60, 140], [120, 80, 200]), mk("mara@example.com", [200, 150, 120], [90, 60, 50]), mk("you@example.com", [40, 90, 60], [140, 170, 90])]
}

/// Sample last-active times (Unix ms) for inactive contacts: "Active 40 minutes ago", "Active 1 hour ago", ...
pub(crate) fn sample_last_active(now_ms: i64) -> Vec<(String, i64)> {
    vec![("jonas@example.com".into(), now_ms - 40 * 60_000), ("sample-inactive@example.com".into(), now_ms - 60 * 60_000), ("nina@example.com".into(), now_ms - 45 * 60_000)]
}

/// Three distinct sample banners: the user's own (blue), the global default (pink/purple) and one person's (dark teal).
pub(crate) fn sample_banner(kind: u8) -> Vec<u8> {
    let (a, b): ([f32; 3], [f32; 3]) = match kind { 0 => ([20.0, 60.0, 140.0], [90.0, 170.0, 220.0]), 1 => ([155.0, 34.0, 104.0], [74.0, 26.0, 122.0]), _ => ([8.0, 40.0, 48.0], [30.0, 110.0, 100.0]) };
    let img = image::RgbaImage::from_fn(600, 120, |x, y| {
        let k = x as f32 / 600.0;
        let band = 1.0 + 0.12 * ((y as f32 / 9.0).sin());
        let c = |i: usize| ((a[i] + (b[i] - a[i]) * k) * band).clamp(0.0, 255.0) as u8;
        image::Rgba([c(0), c(1), c(2), 255])
    });
    let mut v = Vec::new();
    let _ = img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png);
    v
}

/// Offline sample message images (keys as used by `cloud/sample.rs`: "sample-img-1", "sample-img-2", "sample-img-tall"):
/// generated light "screenshot-like" pictures with a grid so cropping / scaling can be checked without a network. The
/// dimensions match the attachments in the sample messages.
pub(crate) fn sample_images() -> Vec<(String, Vec<u8>)> {
    let mk = |key: &str, w: u32, h: u32, tint: [u8; 3]| {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            let grid = x % 40 == 0 || y % 40 == 0;
            let k = y as f32 / h as f32;
            let base = |i: usize| (235.0 - 28.0 * k + (tint[i] as f32 - 235.0) * 0.25).clamp(0.0, 255.0) as u8;
            if grid { image::Rgba([205, 208, 214, 255]) } else { image::Rgba([base(0), base(1), base(2), 255]) }
        });
        let mut v = Vec::new();
        let _ = img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png);
        (key.to_string(), v)
    };
    vec![mk("sample-img-1", 800, 510, [225, 235, 250]), mk("sample-img-2", 800, 190, [250, 240, 225]), mk("sample-img-tall", 600, 1200, [235, 250, 235]), mk("sample-img-3", 600, 280, [60, 80, 140]), mk("sample-gif", 480, 270, [140, 90, 60])]
}
