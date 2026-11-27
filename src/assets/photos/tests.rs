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

/// Unfocused with "Only play GIFs when Webox is the active window" (default on), a repaint (mouse move) keeps the
/// frame; back in focus the GIF goes on from there.
#[test]
fn gif_stands_still_while_unfocused() {
    let ctx = egui::Context::default();
    let focus = |f: bool| {
        let mut raw = egui::RawInput::default();
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(f);
        let _ = ctx.run(raw, |_| {});
    };
    let mut p = Photos::default();
    p.add_images(vec![("g".into(), gif(&[[255, 0, 0, 255], [0, 255, 0, 255]]))]);
    let first = p.image(&ctx, "g").unwrap().0;
    assert_eq!(p.image_frame(&ctx, "g", 10.0).unwrap().0, first);
    assert_eq!(p.image_frame(&ctx, "g", 10.09).unwrap().0, first); // last focused draw, 90 ms in
    focus(false);
    assert_eq!(p.image_frame(&ctx, "g", 10.11).unwrap().0, first); // focus lost after the 100 ms boundary: still frame 0
    assert_eq!(p.image_frame(&ctx, "g", 13.37).unwrap().0, first); // a mouse move later
    focus(true);
    assert_eq!(p.image_frame(&ctx, "g", 20.0).unwrap().0, first); // goes on at 90 ms
    assert_ne!(p.image_frame(&ctx, "g", 20.02).unwrap().0, first); // 110 ms → frame 1
}

#[test]
fn long_gif_keeps_its_loop_duration() {
    // 1000 frames × 10 ms = 10 s must stay 10 s with at most MAX_FRAMES frames.
    let mut v: Vec<(u32, f32)> = (0..1000).map(|i| (i, 0.01)).collect();
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

#[test]
fn gif_with_a_huge_canvas_is_refused_before_decoding() {
    // header + logical screen 65535 x 65535, no colour table, then the trailer: decoding it would allocate ~17 GB
    let bomb = [b"GIF89a".as_slice(), &[0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00], &[0x3b]].concat();
    assert!(!gif_canvas_ok(&bomb));
    assert!(gif_decoder(&bomb).is_err());
    let ctx = egui::Context::default();
    let mut p = Photos::default();
    p.add_images(vec![("bomb".into(), bomb)]);
    assert!(p.image(&ctx, "bomb").is_none());
    // a normal GIF still passes the check
    assert!(gif_canvas_ok(&gif(&[[1, 2, 3, 255]])) && gif_decoder(&gif(&[[1, 2, 3, 255]])).is_ok());
    assert!(!gif_canvas_ok(b"GIF89a")); // too short for a header
}

#[test]
fn gif_frame_budget_shrinks_with_the_canvas() {
    let gif = |w: u16, h: u16| { let mut b = b"GIF89a".to_vec(); b.extend(w.to_le_bytes()); b.extend(h.to_le_bytes()); b };
    assert_eq!(gif_frame_budget(&gif(500, 500)), 2000);
    assert_eq!(gif_frame_budget(&gif(4096, 4096)), 128);
    assert_eq!(gif_frame_budget(&gif(0, 0)), 2000);
    assert_eq!(gif_frame_budget(b"GIF8"), 0);
}

#[test]
fn viewer_image_is_dropped_once_the_viewer_no_longer_draws_it() {
    let ctx = egui::Context::default();
    let mut p = Photos::default();
    p.add_images(vec![("full:x".into(), gif(&[[1, 2, 3, 255]])), ("y".into(), gif(&[[4, 5, 6, 255]]))]);
    assert!(p.image(&ctx, "full:x").is_some() && p.image(&ctx, "y").is_some());
    for _ in 0..3 { let _ = ctx.run(Default::default(), |_| {}); }
    let _ = p.image(&ctx, "y"); // a later pass: the viewer is closed, the list image is still drawn
    assert!(!p.image_tex.contains_key("full:x"));
    assert!(p.image_tex.contains_key("y"));
}

#[test]
fn small_gif_keeps_every_frame() {
    // A typical Giphy GIF has 70-150 frames; a small one fits the memory budget and must play every frame (a fixed 60-frame
    // cap used to drop up to 40 % of them, which looked jerky and slow).
    let colours: Vec<[u8; 4]> = (0..100u8).map(|i| [i, 255 - i, i / 2, 255]).collect();
    let ctx = egui::Context::default();
    let mut p = Photos::default();
    p.add_images(vec![("g".into(), gif(&colours))]);
    let a = p.anim(&ctx, "g").unwrap();
    assert_eq!(a.frames.len(), 100);
    assert!((a.total - 10.0).abs() < 1e-3);
}

#[test]
fn deleted_images_get_a_grey_copy() {
    let ctx = egui::Context::default();
    let mut p = Photos::default();
    p.add_images(vec![("g".into(), gif(&[[255, 0, 0, 255]]))]);
    let (_, size) = p.image_gray(&ctx, "g").expect("grey copy");
    assert_eq!(size, [4, 4]);
    let (id2, _) = p.image_gray(&ctx, "g").unwrap();
    assert_eq!(p.image_gray(&ctx, "g").unwrap().0, id2); // made once, then cached
    assert!(p.image_gray(&ctx, "missing").is_none());
}

/// A PNG whose header claims `w` x `h` RGBA pixels (8 bit) with an empty image stream: a few dozen bytes.
fn png_header_only(w: u32, h: u32) -> Vec<u8> {
    fn crc32(b: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &x in b { c ^= x as u32; for _ in 0..8 { c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 }; } }
        !c
    }
    let chunk = |t: &[u8], d: &[u8]| { let mut v = (d.len() as u32).to_be_bytes().to_vec(); let td = [t, d].concat(); v.extend(&td); v.extend(crc32(&td).to_be_bytes()); v };
    let ihdr = [w.to_be_bytes().as_slice(), &h.to_be_bytes(), &[8, 6, 0, 0, 0]].concat();
    [b"\x89PNG\r\n\x1a\n".as_slice(), &chunk(b"IHDR", &ihdr), &chunk(b"IDAT", &[]), &chunk(b"IEND", &[])].concat()
}

#[test]
fn picture_decode_bomb_is_refused_from_its_header() {
    // 11000 x 11000 RGBA = ~484 MB: under the image crate's default 512 MB, so without our limits the decoder would allocate it.
    let bomb = png_header_only(11000, 11000);
    assert!(matches!(decode_net(&bomb), Err(image::ImageError::Limits(_))));
    // a side over the limit is refused too, even when its memory would fit
    assert!(matches!(decode_net(&png_header_only(NET_IMAGE_SIDE + 1, 1)), Err(image::ImageError::Limits(_))));
    // as a message picture and as a profile picture: nothing to show, no crash
    let ctx = egui::Context::default();
    let mut p = Photos::default();
    p.add_images(vec![("bomb".into(), bomb.clone())]);
    assert!(p.image(&ctx, "bomb").is_none());
    assert!(p.image_gray(&ctx, "bomb").is_none());
    p.add(vec![("person".into(), bomb)]);
    assert!(p.texture(&ctx, "person").is_none());
    // profile pictures have the tighter limit: 5000 x 5000 is a message picture's size, not an avatar's
    assert!(matches!(decode_limited(&png_header_only(5000, 5000), AVATAR_SIDE, AVATAR_ALLOC), Err(image::ImageError::Limits(_))));
    // the GIF canvas check applies to the still decode as well
    let gif_bomb = [b"GIF89a".as_slice(), &[0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00], &[0x3b]].concat();
    assert!(matches!(decode_net(&gif_bomb), Err(image::ImageError::Limits(_))));
}

#[test]
fn normal_pictures_still_decode_within_the_limits() {
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(300, 200, image::Rgba([1, 2, 3, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let png = png.into_inner();
    assert_eq!(decode_net(&png).map(|i| (i.width(), i.height())).ok(), Some((300, 200)));
    let ctx = egui::Context::default();
    let mut p = Photos::default();
    p.add_images(vec![("ok".into(), png.clone())]);
    assert_eq!(p.image(&ctx, "ok").map(|i| i.1), Some([300, 200]));
    p.add(vec![("person".into(), png)]);
    assert!(p.texture(&ctx, "person").is_some());
}
