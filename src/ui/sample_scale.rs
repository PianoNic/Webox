//! A bigger offline sample for memory / CPU checks (`--sample` only): `WEBOX_SAMPLE_SCALE=<n>` adds n spaces (every other one a
//! direct chat with its own 192 px profile picture), 40 messages each with a picture in every fifth, so the caches see an account
//! of a realistic size. `WEBOX_SAMPLE_TOUR=1` then opens every space once (one per 150 ms), as browsing through the list does,
//! and returns to the first one, after which the window is left idle.
use crate::cloud::{Attachment, Message, Person, Space};
use crate::App;
use eframe::egui;

/// Distinct pictures generated; the messages cycle through them under keys of their own (decoded and cached per key, as real
/// pictures are).
const POOL: usize = 12;

pub(crate) fn scale(app: &mut App) {
    let Some(n) = std::env::var("WEBOX_SAMPLE_SCALE").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0) else { return };
    let pool: Vec<Vec<u8>> = (0..POOL).map(picture).collect();
    let faces: Vec<Vec<u8>> = (0..POOL).map(face).collect();
    let today = crate::system::today();
    let (mut images, mut photos) = (Vec::new(), Vec::new());
    for k in 0..n {
        let id = format!("x{k}");
        let direct = k % 2 == 0;
        let who = Person { id: format!("p{k}@example.com"), name: format!("Person {k}"), email: format!("p{k}@example.com"), status: if k % 3 == 0 { "active" } else { "inactive" }.into() };
        if direct { photos.push((who.email.clone(), faces[k % POOL].clone())); }
        let title = if direct { who.name.clone() } else { format!("Project space {k}") };
        app.spaces.push(Space { id: id.clone(), title, direct, last: "Mo".into(), participants: vec![app.me.clone(), who.clone()], ..Default::default() });
        app.people.push(who.clone());
        let msgs: Vec<Message> = (0..40).map(|j| {
            let mut m = Message { id: format!("{id}-{j}"), author: who.name.clone(), email: who.email.clone(), date: today.clone(), time: format!("{:02}:{:02}", 8 + j / 6, (j * 7) % 60),
                text: format!("Message {j} in space {k}: the quick brown fox jumps over the lazy dog, again and again."), ..Default::default() };
            if j % 5 == 4 {
                let key = format!("scale-img-{k}-{j}");
                images.push((key.clone(), pool[(k + j) % POOL].clone()));
                m.images = vec![Attachment { name: "photo.jpg".into(), mime: "image/jpeg".into(), key, width: 512, height: 384, ..Default::default() }];
            }
            m
        }).collect();
        app.messages.insert(id, msgs);
    }
    app.photos.add(photos);
    app.photos.add_images(images);
}

/// `WEBOX_SAMPLE_TOUR=1`: opens the next space every 150 ms until each was shown once, then the first one again.
pub(crate) fn tour(app: &mut App, ctx: &egui::Context) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("WEBOX_SAMPLE_TOUR").is_some()) || app.tour.0 > app.spaces.len() { return }
    let now = ctx.input(|i| i.time);
    if now < app.tour.1 { ctx.request_repaint_after(std::time::Duration::from_secs_f64(app.tour.1 - now)); return }
    app.tour.1 = now + 0.15;
    let id = app.spaces.get(app.tour.0).or(app.spaces.first()).map(|s| s.id.clone());
    app.tour.0 += 1;
    if let Some(id) = id { app.open_space(id); }
    ctx.request_repaint_after(std::time::Duration::from_millis(150));
}

/// A 512x384 photo-like JPEG (the size of the thumbnails Webex delivers for pictures) (smooth gradients and noise, so it neither compresses to nothing nor decodes instantly).
fn picture(seed: usize) -> Vec<u8> {
    let s = seed as f32;
    let img = image::RgbImage::from_fn(512, 384, |x, y| {
        let (fx, fy) = (x as f32 / 512.0, y as f32 / 384.0);
        let n = ((x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ (seed as u32).wrapping_mul(97)) % 23) as f32;
        let c = |i: f32| ((128.0 + 100.0 * ((fx * (3.0 + i) + fy * (2.0 + s * 0.3) + i).sin())) + n).clamp(0.0, 255.0) as u8;
        image::Rgb([c(0.0), c(1.0), c(2.0)])
    });
    let mut v = Vec::new();
    let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut v, 80).encode_image(&img);
    v
}

/// A 192 px profile picture (the size `cloud::people` downloads).
fn face(seed: usize) -> Vec<u8> {
    let base = [(seed * 37 % 200) as u8 + 30, (seed * 71 % 200) as u8 + 30, (seed * 13 % 200) as u8 + 30, 255];
    let img = image::RgbaImage::from_fn(192, 192, |x, y| {
        let d = ((x as f32 - 96.0).powi(2) + (y as f32 - 80.0).powi(2)).sqrt();
        if d < 40.0 { image::Rgba([230, 200, 170, 255]) } else { image::Rgba(base) }
    });
    let mut v = Vec::new();
    let _ = img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png);
    v
}
