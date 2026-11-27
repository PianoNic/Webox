//! The hook interface between the standard GIF picker / message list and the built-in "GIF-Extras" plugin
//! (`src/extras/gif_extras/`). The core code only knows this trait: it asks whether the plugin is switched on (Settings > Plugins,
//! `Plugins::is_enabled(key)`, read every frame so the switch acts at once) and, if so, lets it draw its popup instead of the
//! standard one and decorate images in the chat and the viewer. With the plugin off (the default) none of these hooks run, so
//! the picker is the standard one and no star appears anywhere.
//!
//! In-tree on purpose: dynamic (DLL) plugins cannot draw egui UI safely across the C ABI, so this first-party plugin is compiled in
//! and only registered in the plugin list (with a "Built-in" badge).
use crate::extras::giphy::Gif;
use crate::*;
use std::cell::RefCell;
use std::rc::Rc;

/// What the plugin's popup did this frame.
#[derive(Default)]
pub(crate) struct PopupOut {
    /// The popup closed (Esc, click outside, a pick).
    pub close: bool,
    /// A GIF / sticker to stage in the composer, with its thumbnail bytes for the staging chip.
    pub pick: Option<(Gif, Option<Rc<Vec<u8>>>)>,
    /// An emoji for the composer (inserted at the caret).
    pub emoji: Option<String>,
}

pub(crate) trait GifExtras {
    /// The popup is being opened (reset to its start screen).
    fn open(&mut self, app: &mut App, ctx: &egui::Context);
    /// Draws the popup in `rect` (above `anchor`, the GIF button) while it is open, for the composer of `space`.
    fn popup(&mut self, app: &mut App, ui: &mut egui::Ui, t: &Th, rect: Rect, anchor: Rect, space: &str) -> PopupOut;
    /// Decorates an image attachment `a` drawn at `r` in the message list (`ready` = its picture is shown).
    fn image_overlay(&mut self, app: &mut App, ui: &mut egui::Ui, r: Rect, a: &cloud::Attachment, salt: (&str, usize), ready: bool);
    /// One extra button in the image viewer's top-right row, in `cell`; returns whether it took the cell.
    fn viewer_button(&mut self, app: &mut App, ui: &mut egui::Ui, cell: Rect, a: &cloud::Attachment) -> bool;
    /// The message menu is open on picture `a`: `Some(saved?)` adds "Add to favorites" / "Remove from favorites" to it.
    fn image_menu(&mut self, app: &mut App, ctx: &egui::Context, a: &cloud::Attachment) -> Option<bool>;
    /// That entry was picked.
    fn image_menu_pick(&mut self, app: &mut App, ctx: &egui::Context, a: &cloud::Attachment);
}

struct Reg { key: &'static str, ext: Option<Box<dyn GifExtras>> }
thread_local! { static EXT: RefCell<Option<Reg>> = const { RefCell::new(None) }; }

/// Registers the plugin under its plugin-list key (App::new; tests register their own).
pub(crate) fn register(key: &'static str, ext: Box<dyn GifExtras>) { EXT.with(|r| *r.borrow_mut() = Some(Reg { key, ext: Some(ext) })) }

/// The registered plugin's key.
pub(crate) fn key() -> Option<&'static str> { EXT.with(|r| r.borrow().as_ref().map(|r| r.key)) }

/// Runs `f` with the plugin taken out of the registry (so it may call back into the app; a nested call sees no plugin).
fn with<R>(f: impl FnOnce(&mut dyn GifExtras) -> R) -> Option<R> {
    let mut e = EXT.with(|r| r.borrow_mut().as_mut().and_then(|r| r.ext.take()))?;
    let out = f(&mut *e);
    EXT.with(|r| if let Some(r) = r.borrow_mut().as_mut() { r.ext = Some(e) });
    Some(out)
}

/// The gate: a plugin is registered and its switch is on; `shot_override` (`WEBOX_SHOT_GIF_EXTRAS` in screenshot runs) wins.
pub(crate) fn active(plugins: &crate::extras::plugins::Plugins, shot_override: Option<&str>) -> bool {
    let Some(k) = key() else { return false };
    match shot_override { Some(v) => v == "1", None => plugins.is_enabled(k) }
}

/// Runs a hook only while the gate is open (`None` otherwise: the plugin is not even touched).
pub(crate) fn dispatch<R>(on: bool, f: impl FnOnce(&mut dyn GifExtras) -> R) -> Option<R> { if on { with(f) } else { None } }

impl App {
    /// Whether the GIF-Extras plugin is on: its switch on the Plugins page (default off). Screenshot runs may force it with
    /// `WEBOX_SHOT_GIF_EXTRAS=0|1` (nothing is saved).
    pub(crate) fn gif_extras_on(&self) -> bool {
        let shot = if self.shot.is_some() { std::env::var("WEBOX_SHOT_GIF_EXTRAS").ok() } else { None };
        active(&self.plugins, shot.as_deref())
    }

    pub(super) fn gif_ext_open(&mut self, ctx: &egui::Context) -> bool { let on = self.gif_extras_on(); dispatch(on, |e| e.open(self, ctx)).is_some() }
    pub(super) fn gif_ext_popup(&mut self, ui: &mut egui::Ui, t: &Th, rect: Rect, anchor: Rect, space: &str) -> Option<PopupOut> { let on = self.gif_extras_on(); dispatch(on, |e| e.popup(self, ui, t, rect, anchor, space)) }

    /// Message list hook for one image attachment (only while the plugin is on).
    pub(crate) fn gif_ext_image_overlay(&mut self, ui: &mut egui::Ui, r: Rect, a: &cloud::Attachment, salt: (&str, usize), ready: bool) {
        let on = self.gif_extras_on();
        dispatch(on, |e| e.image_overlay(self, ui, r, a, salt, ready));
    }

    /// Image viewer hook: one extra button in `cell` (only while the plugin is on); whether it was drawn.
    pub(crate) fn gif_ext_viewer_button(&mut self, ui: &mut egui::Ui, cell: Rect, a: Option<&cloud::Attachment>) -> bool {
        let Some(a) = a else { return false };
        let on = self.gif_extras_on();
        dispatch(on, |e| e.viewer_button(self, ui, cell, a)).unwrap_or(false)
    }

    /// Message menu hook on a picture: `Some(saved?)` for the favourites entry (only while the plugin is on; None = no entry).
    pub(crate) fn gif_ext_image_menu(&mut self, ctx: &egui::Context, a: &cloud::Attachment) -> Option<bool> {
        let on = self.gif_extras_on();
        dispatch(on, |e| e.image_menu(self, ctx, a)).flatten()
    }

    /// The favourites entry of the message menu on a picture was picked.
    pub(crate) fn gif_ext_image_menu_pick(&mut self, ctx: &egui::Context, a: &cloud::Attachment) {
        let on = self.gif_extras_on();
        dispatch(on, |e| e.image_menu_pick(self, ctx, a));
    }
}
