use super::*;

fn gif(frames: &[[u8; 4]]) -> Vec<u8> {
    let mut out = vec![];
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        for px in frames {
            let img = image::RgbaImage::from_pixel(4, 4, image::Rgba(*px));
            let delay = image::Delay::from_numer_denom_ms(100, 1);
            enc.encode_frame(image::Frame::from_parts(img, 0, 0, delay)).unwrap();
        }
    }
    out
}

#[test]
fn animated_gif_loops_through_frames() {
    let ctx = egui::Context::default();
    let mut p = Photos::default();
    p.add_images(vec![("g".into(), gif(&[[255, 0, 0, 255], [0, 255, 0, 255]]))]);
    let first = p.image(&ctx, "g").unwrap().0;
    assert_eq!(p.image_frame(&ctx, "g", 10.0).unwrap().0, first); // t = 0 → frame 0
    let second = p.image_frame(&ctx, "g", 10.15).unwrap().0; // 150 ms in → frame 1 (delays 100 ms)
    assert_ne!(second, first);
    assert_eq!(p.image_frame(&ctx, "g", 10.25).unwrap().0, first); // loops
}

#[test]
fn long_gif_keeps_its_loop_duration() {
    // 100 frames × 100 ms = 10 s must stay 10 s with at most MAX_FRAMES frames.
    let mut v: Vec<(u32, f32)> = (0..100).map(|i| (i, 0.1)).collect();
    while v.len() > MAX_FRAMES { v = halve_frames(v); }
    assert!(v.len() <= MAX_FRAMES && v.len() > MAX_FRAMES / 2);
    assert!((v.iter().map(|f| f.1).sum::<f32>() - 10.0).abs() < 1e-3);
    assert_eq!(v[0].0, 0); // the first frame is kept
    let odd = halve_frames(vec![(0, 1.0), (1, 1.0), (2, 1.0)]);
    assert_eq!(odd.iter().map(|f| f.1).sum::<f32>(), 3.0);
}

#[test]
fn broken_bytes_are_not_an_image() {
    let ctx = egui::Context::default();
    let mut p = Photos::default();
    p.add_images(vec![("x".into(), b"not an image".to_vec())]);
    assert!(p.image(&ctx, "x").is_none());
}
