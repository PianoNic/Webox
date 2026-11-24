//! AppKit pieces of the app: the menu-bar status item (the tray: icon, unread tooltip, the tray menu), the Dock badge, the
//! app's menu bar (Webox / Bearbeiten / Fenster) and "the app became active again" (a Dock click while hidden in the menu bar).
//! Everything here runs on the main thread (eframe's `update`); one `Target` object receives every menu action and the
//! notification, and queues what happened for the app (`take_status`, `take_menu`), waking the window.
#![allow(unused_unsafe)] // objc2 marks some of these calls safe, others not
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel, AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationDidBecomeActiveNotification, NSEventModifierFlags, NSEventType, NSImage, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSData, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSSize, NSString};
use std::sync::Mutex;

/// What the status item reports: "show the window" (its first row, or the app activated while hidden), or tray-menu row `i`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StatusEvent { Show, Item(u32) }

/// The app menu's own commands (the rest are AppKit's: hide, hide others, show all, bring all to front).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuCmd { About, Settings, Quit, Close, Minimize, Zoom, Undo, Redo, Cut, Copy, Paste, SelectAll }

/// One row of the tray menu (the same rows the other platforms show).
#[derive(Clone, Debug, PartialEq)]
pub struct MenuRow { pub label: String, pub enabled: bool, pub sep: bool }

static STATUS: Mutex<Vec<StatusEvent>> = Mutex::new(Vec::new());
static MENU: Mutex<Vec<MenuCmd>> = Mutex::new(Vec::new());
static EDIT: Mutex<Vec<MenuCmd>> = Mutex::new(Vec::new());
static CTX: Mutex<Option<eframe::egui::Context>> = Mutex::new(None);

fn wake() { if let Some(c) = CTX.lock().unwrap().as_ref() { c.request_repaint(); } }
fn push_status(e: StatusEvent) { STATUS.lock().unwrap().push(e); wake(); }
fn push_menu(c: MenuCmd) { MENU.lock().unwrap().push(c); wake(); }
pub fn take_status() -> Vec<StatusEvent> { std::mem::take(&mut *STATUS.lock().unwrap()) }
pub fn take_menu() -> Vec<MenuCmd> { std::mem::take(&mut *MENU.lock().unwrap()) }

/// Edit commands picked with the mouse, as the egui events the keys would have given (`App::raw_input_hook`, before egui
/// sees the frame's input).
pub fn inject(raw: &mut eframe::egui::RawInput) {
    use eframe::egui::{Event, Key, Modifiers};
    let cmds = std::mem::take(&mut *EDIT.lock().unwrap());
    let key = |k: Key, m: Modifiers| Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m };
    for c in cmds {
        match c {
            MenuCmd::Copy => raw.events.push(Event::Copy),
            MenuCmd::Cut => raw.events.push(Event::Cut),
            MenuCmd::Paste => if let Some(t) = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()) { raw.events.push(Event::Paste(t)); },
            MenuCmd::Undo => raw.events.push(key(Key::Z, Modifiers::COMMAND)),
            MenuCmd::Redo => raw.events.push(key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)),
            MenuCmd::SelectAll => raw.events.push(key(Key::A, Modifiers::COMMAND)),
            _ => {}
        }
    }
}

/// Tag of the status menu's "Webox anzeigen" row.
const SHOW_TAG: isize = -1;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WeboxMenuTarget"]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(statusRow:))]
        fn status_row(&self, item: &NSMenuItem) {
            let tag = unsafe { item.tag() };
            if tag == SHOW_TAG { activate(); push_status(StatusEvent::Show); } else if tag >= 0 { push_status(StatusEvent::Item(tag as u32)); }
        }
        #[unsafe(method(appCmd:))]
        fn app_cmd(&self, item: &NSMenuItem) {
            let cmd = match unsafe { item.tag() } {
                0 => MenuCmd::About, 1 => MenuCmd::Settings, 2 => MenuCmd::Quit, 3 => MenuCmd::Close, 4 => MenuCmd::Minimize, 5 => MenuCmd::Zoom,
                _ => return,
            };
            push_menu(cmd);
        }
        /// Edit commands: from the keyboard the key goes on to the window as a normal key press (egui does the editing, image
        /// paste included); from a mouse click the matching egui event is queued (`inject`).
        #[unsafe(method(editCmd:))]
        fn edit_cmd(&self, item: &NSMenuItem) {
            let mtm = MainThreadMarker::from(self);
            let app = NSApplication::sharedApplication(mtm);
            if let Some(ev) = unsafe { app.currentEvent() } {
                if unsafe { ev.r#type() } == NSEventType::KeyDown {
                    if let Some(w) = unsafe { app.keyWindow() } { unsafe { w.sendEvent(&ev) }; }
                    return;
                }
            }
            let cmd = match unsafe { item.tag() } { 10 => MenuCmd::Undo, 11 => MenuCmd::Redo, 12 => MenuCmd::Cut, 13 => MenuCmd::Copy, 14 => MenuCmd::Paste, 15 => MenuCmd::SelectAll, _ => return };
            EDIT.lock().unwrap().push(cmd);
            wake();
        }
        #[unsafe(method(didBecomeActive:))]
        fn did_become_active(&self, _n: &NSNotification) {
            if crate::platform::tray::HIDDEN.load(std::sync::atomic::Ordering::Relaxed) { push_status(StatusEvent::Show); }
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> { unsafe { msg_send![Self::alloc(mtm), init] } }
}

thread_local! {
    /// The one target (menus keep only weak references to their targets).
    static TARGET: std::cell::OnceCell<Retained<Target>> = const { std::cell::OnceCell::new() };
}
fn target(mtm: MainThreadMarker) -> Retained<Target> { TARGET.with(|t| t.get_or_init(|| Target::new(mtm)).clone()) }

/// Brings the app to the front (the window itself is shown by the app: `ViewportCommand::Visible` / `Focus`).
pub fn activate() {
    if let Some(mtm) = MainThreadMarker::new() {
        #[allow(deprecated)]
        unsafe { NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true) };
    }
}

fn item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str, tag: isize, target: Option<&Target>) -> Retained<NSMenuItem> {
    unsafe {
        let it = NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), action, &NSString::from_str(key));
        it.setTag(tag);
        if let Some(t) = target { it.setTarget(Some(t as &AnyObject)); }
        it
    }
}

fn submenu(mtm: MainThreadMarker, bar: &NSMenu, title: &str, items: &[Retained<NSMenuItem>]) -> Retained<NSMenu> {
    unsafe {
        let m = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        for i in items { m.addItem(i); }
        let holder = NSMenuItem::new(mtm);
        holder.setSubmenu(Some(&m));
        bar.addItem(&holder);
        m
    }
}

/// The app's menu bar, replacing winit's default one (whose Quit ends the process without the app's own quit path). Once.
pub fn install_main_menu(ctx: &eframe::egui::Context) {
    *CTX.lock().unwrap() = Some(ctx.clone());
    let Some(mtm) = MainThreadMarker::new() else { return };
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, std::sync::atomic::Ordering::Relaxed) { return; }
    let t = target(mtm);
    let t = Some(&*t);
    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        let bar = NSMenu::new(mtm);
        let hide_others = item(mtm, "Andere ausblenden", Some(sel!(hideOtherApplications:)), "h", 0, None);
        hide_others.setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Option);
        submenu(mtm, &bar, "Webox", &[
            item(mtm, "Über Webox", Some(sel!(appCmd:)), "", 0, t),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Einstellungen …", Some(sel!(appCmd:)), ",", 1, t),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Webox ausblenden", Some(sel!(hide:)), "h", 0, None),
            hide_others,
            item(mtm, "Alle einblenden", Some(sel!(unhideAllApplications:)), "", 0, None),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Webox beenden", Some(sel!(appCmd:)), "q", 2, t),
        ]);
        let redo = item(mtm, "Wiederholen", Some(sel!(editCmd:)), "z", 11, t);
        redo.setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
        submenu(mtm, &bar, "Bearbeiten", &[
            item(mtm, "Widerrufen", Some(sel!(editCmd:)), "z", 10, t), redo, NSMenuItem::separatorItem(mtm),
            item(mtm, "Ausschneiden", Some(sel!(editCmd:)), "x", 12, t), item(mtm, "Kopieren", Some(sel!(editCmd:)), "c", 13, t),
            item(mtm, "Einsetzen", Some(sel!(editCmd:)), "v", 14, t), item(mtm, "Alles auswählen", Some(sel!(editCmd:)), "a", 15, t),
        ]);
        let window = submenu(mtm, &bar, "Fenster", &[
            item(mtm, "Schließen", Some(sel!(appCmd:)), "w", 3, t),
            item(mtm, "Im Dock ablegen", Some(sel!(appCmd:)), "m", 4, t),
            item(mtm, "Zoomen", Some(sel!(appCmd:)), "", 5, t),
            NSMenuItem::separatorItem(mtm),
            item(mtm, "Alle nach vorne bringen", Some(sel!(arrangeInFront:)), "", 0, None),
        ]);
        app.setMainMenu(Some(&bar));
        app.setWindowsMenu(Some(&window));
        // A Dock click (or Cmd+Tab) while the window is hidden in the menu bar brings it back.
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(&*target(mtm), sel!(didBecomeActive:),
            Some(NSApplicationDidBecomeActiveNotification), None);
    }
}

/// The app's icon as an NSImage of `pt` points (PNG data, `px` pixels for Retina).
fn image(rgba: Vec<u8>, px: u32, pt: f64) -> Option<Retained<NSImage>> {
    let img = image::RgbaImage::from_raw(px, px, rgba)?;
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).ok()?;
    let data = NSData::with_bytes(&png.into_inner());
    let ns = unsafe { NSImage::initWithData(NSImage::alloc(), &data) }?;
    unsafe { ns.setSize(NSSize::new(pt, pt)) };
    Some(ns)
}

/// The menu-bar status item: the app icon (with the red dot while something is unread), the unread count as tooltip and the
/// tray menu, "Webox anzeigen" first.
pub struct StatusItem { item: Retained<NSStatusItem>, rows: Option<Vec<MenuRow>>, unread: Option<usize> }

impl StatusItem {
    pub fn new(ctx: &eframe::egui::Context) -> Option<StatusItem> {
        *CTX.lock().unwrap() = Some(ctx.clone());
        let mtm = MainThreadMarker::new()?;
        let item = unsafe { NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength) };
        let mut s = StatusItem { item, rows: None, unread: None };
        s.set_unread(0);
        s.set_menu(vec![]);
        let _ = mtm;
        Some(s)
    }

    pub fn set_unread(&mut self, n: usize) {
        if self.unread == Some(n) { return; }
        self.unread = Some(n);
        let Some(mtm) = MainThreadMarker::new() else { return };
        let rgba = if n > 0 { crate::platform::tray::unread_icon_rgba(36) } else { crate::platform::tray::app_icon_rgba(36) };
        unsafe {
            if let Some(b) = self.item.button(mtm) {
                if let Some(img) = image(rgba, 36, 18.0) { b.setImage(Some(&img)); }
                b.setToolTip(Some(&NSString::from_str(&crate::platform::tray::tooltip(n))));
            }
        }
    }

    pub fn set_menu(&mut self, rows: Vec<MenuRow>) {
        if self.rows.as_ref() == Some(&rows) { return; }
        let Some(mtm) = MainThreadMarker::new() else { return };
        let t = target(mtm);
        unsafe {
            let m = NSMenu::new(mtm);
            m.setAutoenablesItems(false);
            m.addItem(&item(mtm, "Webox anzeigen", Some(sel!(statusRow:)), "", SHOW_TAG, Some(&t)));
            m.addItem(&NSMenuItem::separatorItem(mtm));
            for (i, r) in rows.iter().enumerate() {
                if r.sep { m.addItem(&NSMenuItem::separatorItem(mtm)); continue; }
                let it = item(mtm, &r.label, Some(sel!(statusRow:)), "", i as isize, Some(&t));
                it.setEnabled(r.enabled);
                m.addItem(&it);
            }
            self.item.setMenu(Some(&m));
        }
        self.rows = Some(rows);
    }
}

impl Drop for StatusItem {
    fn drop(&mut self) { unsafe { NSStatusBar::systemStatusBar().removeStatusItem(&self.item) }; }
}

/// The unread count on the Dock icon (no badge at 0).
pub fn dock_badge(n: usize) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let label = match n { 0 => None, n if n > 99 => Some(NSString::from_str("99+")), n => Some(NSString::from_str(&n.to_string())) };
    unsafe { NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref()) };
}
