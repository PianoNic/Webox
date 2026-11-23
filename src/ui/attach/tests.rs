use super::*;
#[test]
fn strip_text_and_progress() {
    assert_eq!(uploading_parts(&["a.pdf".into()]), ("a.pdf".into(), " wird hochgeladen".into()));
    assert_eq!(uploading_parts(&["a.pdf".into(), "b.png".into(), "c".into()]).0, "a.pdf (+2)");
    assert_eq!(percent(0, 0), 0);
    assert_eq!(percent(50, 200), 25);
    assert_eq!(percent(300, 200), 100);
    assert_eq!(sample_progress(10.0, 10.0), 0.0);
    assert_eq!(sample_progress(10.0, 20.0), 1.0);
}
#[test]
fn staging_area_flows_and_clamps() {
    // Flow layout: left margin 4, 208 wide bricklets, 20 apart; a new row 60 + 5 lower when the next one does not fit.
    let (pos, h) = flow_layout(3, 700.0);
    assert_eq!(pos, vec![Vec2::new(4.0, 0.0), Vec2::new(232.0, 0.0), Vec2::new(460.0, 0.0)]);
    assert_eq!(h, 64.0); // 60 + bottom margin 4
    let (pos, h) = flow_layout(3, 450.0);
    assert_eq!(pos[2], Vec2::new(4.0, 65.0));
    assert_eq!(h, 129.0);
    // One bricklet wider than the area still takes the first slot.
    assert_eq!(flow_layout(1, 100.0).0, vec![Vec2::new(4.0, 0.0)]);
    // Staging area: 68 .. 92 high, nothing without files.
    assert_eq!(staging_height(0, 700.0), 0.0);
    assert_eq!(staging_height(1, 700.0), 68.0);
    assert_eq!(staging_height(3, 450.0), 92.0);
}
#[test]
fn previews_fit_66_by_45_both_ways() {
    assert_eq!(preview_size(1320.0, 450.0), Vec2::new(66.0, 22.5));
    assert_eq!(preview_size(10.0, 10.0), Vec2::new(45.0, 45.0)); // small images are scaled up
    assert_eq!(preview_size(0.0, 5.0), Vec2::ZERO);
}
#[test]
fn icons_by_extension() {
    assert_eq!(file_icon("Q3.PDF"), "file-pdf-regular");
    assert_eq!(file_icon("x.docx"), "file-word-regular");
    assert_eq!(file_icon("noext"), "file-text-regular");
}
#[test]
fn image_paste_is_a_v_release_without_its_press() {
    let ctrl = egui::Modifiers::COMMAND;
    let v = |pressed, modifiers| egui::Event::Key { key: Key::V, physical_key: Some(Key::V), pressed, repeat: false, modifiers };
    // Ctrl+V with an image only: egui-winit drops the press, the release arrives.
    let mut s = PasteKeys::default();
    assert!(!s.frame(&[], true, false));
    assert!(s.frame(&[v(false, ctrl)], true, false));
    // Ctrl let go first: Ctrl was held since the last V release.
    let mut s = PasteKeys::default();
    assert!(!s.frame(&[], true, false));
    assert!(!s.frame(&[], false, false));
    assert!(s.frame(&[v(false, Default::default())], false, false));
    // A text paste: the press became Event::Paste.
    let mut s = PasteKeys::default();
    assert!(!s.frame(&[egui::Event::Paste("x".into())], true, false));
    assert!(!s.frame(&[v(false, ctrl)], true, false));
    // Typing v: press seen (same frame or the frame before), no Ctrl.
    let mut s = PasteKeys::default();
    assert!(!s.frame(&[v(true, Default::default())], false, true));
    assert!(!s.frame(&[v(false, Default::default())], false, false));
    assert!(!s.frame(&[v(true, Default::default()), v(false, Default::default())], false, false));
}
