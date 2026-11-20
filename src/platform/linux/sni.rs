//! The tray icon on Linux: a StatusNotifierItem (org.kde.StatusNotifierItem) with its menu exported as com.canonical.dbusmenu,
//! registered with the desktop's org.kde.StatusNotifierWatcher. The panel draws the icon and the menu itself (native look and
//! placement, X11 and Wayland alike). Left click = `Activate` → show the window; the menu has the same entries as the Windows
//! tray menu; hosts without dbusmenu call `ContextMenu(x, y)` and get the app's own menu window instead.
//! No watcher on the bus (plain GNOME without the AppIndicator extension, bare WMs): `Tray::new` returns None, as on Windows
//! when Shell_NotifyIcon fails, so "close to tray" never hides the window where it couldn't be brought back.
use crate::platform::tray::{app_icon_rgba, tooltip, unread_icon_rgba, TrayEvent};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Type, Value};
use zbus::interface;

type Pixmaps = Vec<(i32, i32, Vec<u8>)>;
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// One row of the tray menu as the panel shows it.
#[derive(Clone, PartialEq, Debug)]
pub struct MenuRow { pub label: String, pub enabled: bool, pub sep: bool }

#[derive(Default)]
struct St { icon: Pixmaps, tip: String, rows: Vec<MenuRow>, revision: u32, events: Vec<TrayEvent>, ctx: Option<eframe::egui::Context> }

fn push(st: &Arc<Mutex<St>>, e: TrayEvent) {
    let ctx = { let mut s = st.lock().unwrap(); s.events.push(e); s.ctx.clone() };
    if let Some(c) = ctx { c.request_repaint(); }
}

/// SNI pixmaps are ARGB32 in network byte order, several sizes so the panel can pick a sharp one.
fn pixmaps(unread: bool) -> Pixmaps {
    [16u32, 22, 24, 32, 48, 64].iter().map(|&s| {
        let rgba = if unread { unread_icon_rgba(s) } else { app_icon_rgba(s) };
        (s as i32, s as i32, rgba.chunks(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect())
    }).collect()
}

fn ov<'a>(v: impl Into<Value<'a>>) -> OwnedValue { v.into().try_to_owned().expect("plain value") }

struct Item { st: Arc<Mutex<St>> }

#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)] fn category(&self) -> &str { "Communications" }
    #[zbus(property)] fn id(&self) -> &str { "webox" }
    #[zbus(property)] fn title(&self) -> &str { "Webox" }
    #[zbus(property)] fn status(&self) -> &str { "Active" }
    #[zbus(property)] fn window_id(&self) -> i32 { 0 }
    #[zbus(property)] fn icon_name(&self) -> &str { "" }
    #[zbus(property)] fn icon_theme_path(&self) -> &str { "" }
    #[zbus(property)] fn icon_pixmap(&self) -> Pixmaps { self.st.lock().unwrap().icon.clone() }
    #[zbus(property)] fn attention_icon_name(&self) -> &str { "" }
    #[zbus(property)] fn overlay_icon_name(&self) -> &str { "" }
    #[zbus(property)] fn tool_tip(&self) -> (String, Pixmaps, String, String) { ("".into(), vec![], "Webox".into(), self.st.lock().unwrap().tip.clone()) }
    #[zbus(property)] fn item_is_menu(&self) -> bool { false }
    #[zbus(property)] fn menu(&self) -> OwnedObjectPath { OwnedObjectPath::try_from(MENU_PATH).unwrap() }

    fn activate(&self, _x: i32, _y: i32) { push(&self.st, TrayEvent::Left); }
    fn secondary_activate(&self, _x: i32, _y: i32) { push(&self.st, TrayEvent::Left); }
    fn context_menu(&self, x: i32, y: i32) { push(&self.st, TrayEvent::Menu(x, y)); }
    fn scroll(&self, _delta: i32, _orientation: &str) {}

    #[zbus(signal)] async fn new_icon(e: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)] async fn new_tool_tip(e: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)] async fn new_title(e: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// One dbusmenu node `(ia{sv}av)`; the tray menu is flat, so only the root has children.
#[derive(serde::Serialize, Type, Value)]
struct Node { id: i32, props: HashMap<String, OwnedValue>, children: Vec<OwnedValue> }

struct Menu { st: Arc<Mutex<St>> }

impl Menu {
    /// Menu item ids: 0 = root, row i = i + 1.
    fn props(&self, id: i32) -> HashMap<String, OwnedValue> {
        let mut p = HashMap::new();
        if id == 0 { p.insert("children-display".into(), ov("submenu")); return p; }
        let s = self.st.lock().unwrap();
        let Some(r) = s.rows.get(id as usize - 1) else { return p };
        if r.sep { p.insert("type".into(), ov("separator")); return p; }
        p.insert("label".into(), ov(r.label.replace('_', "__"))); // '_' marks a mnemonic in dbusmenu
        p.insert("enabled".into(), ov(r.enabled));
        p.insert("visible".into(), ov(true));
        p
    }
    fn count(&self) -> i32 { self.st.lock().unwrap().rows.len() as i32 }
}

#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)] fn version(&self) -> u32 { 3 }
    #[zbus(property)] fn text_direction(&self) -> &str { "ltr" }
    #[zbus(property)] fn status(&self) -> &str { "normal" }
    #[zbus(property)] fn icon_theme_path(&self) -> Vec<String> { vec![] }

    fn get_layout(&self, parent_id: i32, recursion_depth: i32, _property_names: Vec<String>) -> (u32, Node) {
        let rev = self.st.lock().unwrap().revision;
        let leaf = |id: i32| Node { id, props: self.props(id), children: vec![] };
        if parent_id != 0 { return (rev, leaf(parent_id)); }
        let children = if recursion_depth == 0 { vec![] } else {
            (1..=self.count()).filter_map(|i| Value::from(leaf(i)).try_to_owned().ok()).collect()
        };
        (rev, Node { id: 0, props: self.props(0), children })
    }
    fn get_group_properties(&self, ids: Vec<i32>, _property_names: Vec<String>) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let ids = if ids.is_empty() { (0..=self.count()).collect() } else { ids };
        ids.into_iter().map(|i| (i, self.props(i))).collect()
    }
    fn get_property(&self, id: i32, name: String) -> OwnedValue { self.props(id).remove(&name).unwrap_or_else(|| ov("")) }
    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if event_id == "clicked" && id > 0 { push(&self.st, TrayEvent::Item(id as u32 - 1)); }
    }
    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, ev, _, _) in events { if ev == "clicked" && id > 0 { push(&self.st, TrayEvent::Item(id as u32 - 1)); } }
        vec![]
    }
    fn about_to_show(&self, _id: i32) -> bool { false }
    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) { (vec![], vec![]) }

    #[zbus(signal)] async fn layout_updated(e: &SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;
}

async fn register(conn: &zbus::Connection, name: &str) -> bool {
    conn.call_method(Some(WATCHER), "/StatusNotifierWatcher", Some(WATCHER), "RegisterStatusNotifierItem", &(name,)).await.is_ok()
}

pub struct Tray { st: Arc<Mutex<St>>, conn: zbus::Connection, shown: Option<usize> }

impl Tray {
    pub fn new(ctx: &eframe::egui::Context) -> Option<Tray> {
        let st = Arc::new(Mutex::new(St { icon: pixmaps(false), tip: tooltip(0), ctx: Some(ctx.clone()), ..Default::default() }));
        let st2 = st.clone();
        let conn = super::block(3000, async move {
            let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
            let conn = zbus::connection::Builder::session().ok()?.name(name.clone()).ok()?
                .serve_at(ITEM_PATH, Item { st: st2.clone() }).ok()?
                .serve_at(MENU_PATH, Menu { st: st2 }).ok()?
                .build().await.ok()?;
            if !register(&conn, &name).await { return None; }
            // A panel restart brings a new watcher: register again with it.
            let c2 = conn.clone();
            tokio::spawn(async move {
                use futures_util::StreamExt;
                let Ok(dbus) = zbus::fdo::DBusProxy::new(&c2).await else { return };
                let Ok(mut s) = dbus.receive_name_owner_changed_with_args(&[(0, WATCHER)]).await else { return };
                while let Some(sig) = s.next().await {
                    if sig.args().is_ok_and(|a| a.new_owner().is_some()) { register(&c2, &name).await; }
                }
            });
            Some(conn)
        }).flatten()?;
        Some(Tray { st, conn, shown: Some(0) })
    }

    /// Icon with the red badge while something is unread, and the tooltip with the count.
    pub fn set_unread(&mut self, unread: usize) {
        if self.shown == Some(unread) { return; }
        let badge_changed = self.shown.map(|s| s > 0) != Some(unread > 0);
        self.shown = Some(unread);
        { let mut s = self.st.lock().unwrap(); s.tip = tooltip(unread); if badge_changed { s.icon = pixmaps(unread > 0); } }
        let conn = self.conn.clone();
        super::rt().spawn(async move {
            let Ok(i) = conn.object_server().interface::<_, Item>(ITEM_PATH).await else { return };
            if badge_changed { let _ = Item::new_icon(i.signal_emitter()).await; }
            let _ = Item::new_tool_tip(i.signal_emitter()).await;
        });
    }

    /// The menu rows (labels follow the app's language and sign-in state); cheap when unchanged.
    pub fn set_menu(&mut self, rows: Vec<MenuRow>) {
        let rev = {
            let mut s = self.st.lock().unwrap();
            if s.rows == rows { return; }
            s.rows = rows; s.revision += 1; s.revision
        };
        let conn = self.conn.clone();
        super::rt().spawn(async move {
            let Ok(i) = conn.object_server().interface::<_, Menu>(MENU_PATH).await else { return };
            let _ = Menu::layout_updated(i.signal_emitter(), rev, 0).await;
        });
    }

    pub fn poll(&mut self) -> Vec<TrayEvent> { std::mem::take(&mut self.st.lock().unwrap().events) }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let conn = self.conn.clone();
        super::block(500, async move { let _ = conn.close().await; });
    }
}
