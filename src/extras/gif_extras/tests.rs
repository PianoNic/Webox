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
