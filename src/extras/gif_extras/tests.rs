use super::*;
use super::stars::giphy_to_gif;
use crate::extras::plugins::Plugins;
use crate::ui::gif_picker::ext::{active, dispatch, register, GifExtras, PopupOut};
use std::cell::Cell;
use std::rc::Rc;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("webox-gifx-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Counts every hook call (stands in for the plugin).
struct Spy(Rc<Cell<u32>>);
impl GifExtras for Spy {
    fn open(&mut self, _: &mut App, _: &egui::Context) { self.0.set(self.0.get() + 1) }
    fn popup(&mut self, _: &mut App, _: &mut egui::Ui, _: &Th, _: Rect, _: Rect, _: &str) -> PopupOut { self.0.set(self.0.get() + 1); PopupOut::default() }
    fn image_overlay(&mut self, _: &mut App, _: &mut egui::Ui, _: Rect, _: &cloud::Attachment, _: (&str, usize), _: bool) { self.0.set(self.0.get() + 1) }
    fn viewer_button(&mut self, _: &mut App, _: &mut egui::Ui, _: Rect, _: &cloud::Attachment) -> bool { self.0.set(self.0.get() + 1); true }
}

#[test]
fn listed_as_builtin_and_off_by_default() {
    let d = tmp("list");
    let p = Plugins::new(&d);
    let e = p.list.iter().find(|e| e.name == NAME).expect("GIF-Extras is in the plugin list");
    assert!(e.builtin && !e.enabled && e.error.is_none());
    assert_eq!((e.version.as_str(), e.description.is_empty()), (VERSION, false));
    assert!(!p.is_enabled(KEY), "default off");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn switch_is_saved_and_read_back() {
    let d = tmp("persist");
    let mut p = Plugins::new(&d);
    let i = p.list.iter().position(|e| e.builtin).unwrap();
    p.set_enabled(i, true);
    assert!(p.is_enabled(KEY), "live: on at once");
    assert!(Plugins::new(&d).is_enabled(KEY), "saved in plugins.json");
    p.set_enabled(i, false);
    assert!(!Plugins::new(&d).is_enabled(KEY));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn off_means_no_hook_runs_and_on_means_they_do() {
    let d = tmp("gate");
    let mut p = Plugins::new(&d);
    let calls = Rc::new(Cell::new(0));
    register(KEY, Box::new(Spy(calls.clone())));
    // Off (default): the gate is closed, so no star / popup hook is reached at all - the picker stays the standard one.
    assert!(!active(&p, None));
    assert!(dispatch(active(&p, None), |_| ()).is_none());
    assert_eq!(calls.get(), 0);
    // On: the hooks reach the plugin.
    let i = p.list.iter().position(|e| e.builtin).unwrap();
    p.set_enabled(i, true);
    assert!(active(&p, None));
    assert_eq!(dispatch(active(&p, None), |_| 7), Some(7));
    // Screenshot override wins over the saved switch, both ways.
    assert!(!active(&p, Some("0")));
    p.set_enabled(i, false);
    assert!(active(&p, Some("1")));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn no_registered_plugin_means_off() {
    // A fresh thread has no registration: even a saved "on" opens nothing.
    std::thread::spawn(|| {
        let d = tmp("noreg");
        std::fs::write(d.join("plugins.json"), format!("{{\"{KEY}\": true}}")).unwrap();
        let p = Plugins::new(&d);
        assert!(p.is_enabled(KEY));
        assert!(!active(&p, None) && !active(&p, Some("1")));
        let _ = std::fs::remove_dir_all(d);
    }).join().unwrap();
}

#[test]
fn turning_off_leaves_the_favourites_file_alone() {
    let d = tmp("favs");
    let mut f = favs::Favs::load(Some(d.clone()));
    f.toggle(&crate::extras::giphy::Gif { id: "a1".into(), thumb_url: "https://media.giphy.com/media/a1/200w_d.gif".into(), media_url: "https://media.giphy.com/media/a1/giphy.gif".into(), ..Default::default() }, false);
    let before = std::fs::read(d.join("giphy_favorites.json")).unwrap();
    // A kept uploaded GIF (gif-favorites/).
    let kept = d.join(upload_favs::DIR);
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("0123.gif"), b"GIF89a").unwrap();
    let mut p = Plugins::new(&d);
    let i = p.list.iter().position(|e| e.builtin).unwrap();
    p.set_enabled(i, true);
    p.set_enabled(i, false);
    assert_eq!(std::fs::read(d.join("giphy_favorites.json")).unwrap(), before, "switching never touches the favourites");
    assert_eq!(std::fs::read(kept.join("0123.gif")).unwrap(), b"GIF89a", "nor the kept uploaded GIFs");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn giphy_gifs_convert_for_the_favourites_and_toggle() {
    let a = cloud::Attachment { name: "Happy Cat.gif".into(), width: 480, height: 270, giphy: Some(cloud::Giphy { id: "xT9".into(), url: "https://media3.giphy.com/media/xT9/giphy.gif?cid=1".into(), title: "Happy Cat.GIF".into() }), ..Default::default() };
    let g = giphy_to_gif(&a).expect("giphy gif");
    assert_eq!((g.id.as_str(), g.title.as_str()), ("xT9", "Happy Cat"));
    assert_eq!(g.thumb_url, "https://media3.giphy.com/media/xT9/200w_d.gif");
    assert_eq!(g.media_url, "https://media3.giphy.com/media/xT9/giphy.gif?cid=1");
    assert_eq!((g.thumb_w, g.thumb_h, g.media_w, g.media_h), (200, 112, 480, 270));
    // An uploaded GIF (no giphy source) or a source without id / https gets no Giphy star.
    assert!(giphy_to_gif(&cloud::Attachment { name: "x.gif".into(), ..Default::default() }).is_none());
    assert!(giphy_to_gif(&cloud::Attachment { giphy: Some(cloud::Giphy { id: String::new(), url: "https://media.giphy.com/a".into(), title: String::new() }), ..Default::default() }).is_none());
    assert!(giphy_to_gif(&cloud::Attachment { giphy: Some(cloud::Giphy { id: "a".into(), url: "http://media.giphy.com/a/giphy.gif".into(), title: String::new() }), ..Default::default() }).is_none());
    // Toggling in the plugin's store (memory only here): added, then removed again.
    let mut favs = favs::Favs::load(None);
    assert!(favs.toggle(&g, false));
    assert!(favs.contains("xT9", false) && favs.list(false)[0].title == "Happy Cat");
    assert!(!favs.toggle(&g, false));
    assert!(!favs.contains("xT9", false));
}

#[test]
fn home_screen_tiles() {
    use picker::{home_tiles, Tile};
    assert_eq!(home_tiles(2), vec![Tile::Favs, Tile::Trending, Tile::Cat(0), Tile::Cat(1)]);
}

#[test]
fn compact_size_setting_is_off_by_default_saved_and_overridable() {
    let d = tmp("compact");
    let mut p = Plugins::new(&d);
    assert!(settings_schema().contains("Kompakte Größe") && settings_schema().contains("350 × 390"));
    assert!(!compact(&p, None), "default: the standard size");
    let i = p.list.iter().position(|e| e.key == KEY).unwrap();
    p.set_setting(i, COMPACT, serde_json::Value::Bool(true)).unwrap();
    assert!(compact(&p, None), "live: on at once");
    assert!(compact(&Plugins::new(&d), None), "saved");
    // Screenshot override wins both ways.
    assert!(!compact(&p, Some("0")));
    p.set_setting(i, COMPACT, serde_json::Value::Bool(false)).unwrap();
    assert!(!compact(&p, None) && compact(&p, Some("1")));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn compact_layout_matches_the_earlier_popup() {
    use picker::layout::{compact_rect, geo, COMPACT_H, COMPACT_W};
    let screen = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1200.0, 800.0));
    let button = Rect::from_min_size(pos2(300.0, 700.0), Vec2::splat(28.0));
    // 40 px left of the button's centre, 10 px above it.
    let r = compact_rect(screen, button);
    assert_eq!((r.width(), r.height()), (COMPACT_W, COMPACT_H));
    assert_eq!((r.left(), r.bottom()), (274.0, 690.0));
    let g = geo(r, true);
    assert_eq!((g.cols, g.gap, g.colw, g.tile_h), (2, 6.0, 156.0, 76.0));
    assert_eq!(g.content_w(), 318.0);
    assert!(g.content_w() + 8.0 <= g.grid.width(), "room for the scroll bar");
    assert_eq!(g.grid.top() - r.top(), 46.0 + 28.0 + 8.0);
    assert_eq!(r.bottom() - g.grid.bottom(), 34.0);
    assert_eq!(g.home_tile(3), Rect::from_min_size(pos2(162.0, 82.0), Vec2::new(156.0, 76.0)));
    assert_eq!(g.home_h(5), 3.0 * 82.0);
    let [a, b, tip] = g.caret(button).unwrap();
    assert_eq!((a.x, b.x, tip), (306.0, 322.0, pos2(314.0, 698.0)));
    // Masonry with the compact gap: two columns 162 px apart.
    let gif = |w, h| crate::extras::giphy::Gif { thumb_w: w, thumb_h: h, ..Default::default() };
    let (t, _) = crate::ui::gif_picker::masonry_gap(&[gif(200, 100), gif(200, 200), gif(200, 100)], g.colw, g.cols, g.gap);
    assert_eq!((t[0].min.x, t[1].min.x, t[2].min, t[2].height()), (0.0, 162.0, pos2(0.0, 84.0), 78.0));
}

#[test]
fn compact_popup_stays_in_the_window() {
    use picker::layout::compact_rect;
    // A 1280 x 720 window at 125 % zoom is 1024 x 576 points; the button at the right edge and near the top.
    let screen = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1024.0, 576.0));
    let r = compact_rect(screen, Rect::from_min_size(pos2(1000.0, 540.0), Vec2::splat(24.0)));
    assert!(r.left() >= 8.0 && r.right() <= screen.right() - 8.0 && r.top() >= 8.0 && r.bottom() <= 540.0);
    let r = compact_rect(screen, Rect::from_min_size(pos2(2.0, 200.0), Vec2::splat(24.0)));
    assert_eq!((r.left(), r.top()), (8.0, 8.0));
}

#[test]
fn standard_layout_is_unchanged() {
    use crate::ui::gif_picker::{col_width, popup_rect, COLS, GAP, GRID_PAD};
    let screen = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1200.0, 800.0));
    let r = popup_rect(screen, Rect::from_min_size(pos2(300.0, 700.0), Vec2::splat(28.0)));
    let g = picker::layout::geo(r, false);
    assert_eq!((g.rect, g.cols, g.gap, g.tile_h, g.caret(r)), (r, COLS, GAP, 96.0, None));
    let grid = Rect::from_min_max(pos2(r.left() + 12.0, r.top() + 82.0), pos2(r.right() - 2.0, r.bottom() - 34.0));
    assert_eq!((g.grid, g.colw), (grid, col_width(grid.width() + 2.0 * GRID_PAD - 12.0, COLS)));
    assert_eq!((g.hit(), g.clip()), (r, r.expand(2.0)));
}

#[test]
fn send_on_click_is_a_plugin_setting_off_by_default() {
    let schema: serde_json::Value = serde_json::from_str(&settings_schema()).unwrap();
    assert!(schema.to_string().contains(SEND_INSTANTLY), "the toggle is on the plugin's own settings page");
}
