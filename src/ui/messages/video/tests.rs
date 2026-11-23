use super::*;
use super::card::{card_size, contain};

fn att(w: u32, h: u32) -> Attachment { Attachment { name: "clip.mp4".into(), mime: "video/mp4".into(), key: "k".into(), width: w, height: h, ..Default::default() } }

#[test]
fn cards_are_sized_like_pictures_or_16_by_9() {
    // with the thumbnail's size: the 320 box of pictures
    assert_eq!(card_size(&att(1280, 720), 600.0, None), Vec2::new(320.0, 180.0));
    assert_eq!(card_size(&att(0, 0), 600.0, Some([720, 1280])), Vec2::new(180.0, 320.0));
    // nothing known yet: 16:9 at the picture width
    assert_eq!(card_size(&att(0, 0), 600.0, None), Vec2::new(320.0, 180.0));
    assert_eq!(card_size(&att(0, 0), 200.0, None), Vec2::new(200.0, 113.0));
}

#[test]
fn pictures_are_contain_fit() {
    let r = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(320.0, 180.0));
    assert_eq!(contain(r, 1920, 1080), r);
    let tall = contain(r, 720, 1280);
    assert!((tall.height() - 180.0).abs() < 0.01 && (tall.width() - 101.25).abs() < 0.01 && (tall.center().x - 160.0).abs() < 0.01);
    assert_eq!(contain(r, 0, 0), r);
}

#[test]
fn cache_paths_keep_the_name_and_split_by_key() {
    let a = cache_path("activity:1", "Urlaub 2026.mp4").unwrap();
    let b = cache_path("activity:2", "Urlaub 2026.mp4").unwrap();
    assert_ne!(a.parent(), b.parent());
    assert_eq!(a.file_name().unwrap(), "Urlaub 2026.mp4");
    assert!(a.starts_with(cache_dir().unwrap()));
    assert_eq!(cache_path("x", "..\\..\\evil.mp4").unwrap().file_name().unwrap(), "evil.mp4"); // no path parts from the share's name
}

#[test]
fn local_files_and_downloads_are_found() {
    let ctx = egui::Context::default();
    let a = Attachment { key: "video-src-test:1".into(), name: "x.mp4".into(), ..Default::default() };
    assert_eq!(source(&ctx, &a), Src::Remote);
    fs_::download_begin(&a.key, &a.name, &cache_path(&a.key, &a.name).unwrap(), Then::Play);
    fs_::download_progress(&a.key, 50, 100);
    assert_eq!(source(&ctx, &a), Src::Fetching(50));
    fs_::download_phase(&a.key, Phase::Failed("http 500".into()));
    assert_eq!(source(&ctx, &a), Src::Failed("http 500".into()));
    assert!(shown_file(&a).is_none()); // the playback cache is never shown in a folder
}
