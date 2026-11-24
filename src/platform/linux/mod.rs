//! Linux (X11 + Wayland) equivalents of the Win32 pieces in `tray.rs`, `notify.rs` and `shortcuts/global.rs`:
//!
//! * `sni.rs`: the tray icon as a StatusNotifierItem + com.canonical.dbusmenu over the session bus (KDE, XFCE, Cinnamon, MATE,
//!   LXQt, Sway/waybar, GNOME with the AppIndicator extension) with the same menu entries and the unread badge icon.
//! * `launcher.rs`: taskbar badge through the Unity LauncherEntry signal (KDE Plasma task manager, Ubuntu Dock, Dash to Dock, Plank).
//! * `notifications.rs`: on Wayland, where a client can't place its own windows, the toasts go to the desktop's notification
//!   server (org.freedesktop.Notifications) with the same text and buttons. On X11 the app's own toast windows are used, as on Windows.
//! * `x11.rs`: work area, window lookup / placement / capture and XGrabKey global hotkeys (pure-Rust x11rb, no libxcb).
//! * `activation.rs`: raising the main window on Wayland (xdg-activation-v1 with the panel's / notification's / launcher's token).
//! * `clipboard.rs`: image paste on Wayland through the data-control protocols (KDE, wlroots), XWayland as the fallback.
//! * global hotkeys on Wayland: the xdg-desktop-portal GlobalShortcuts interface (ashpd), see `shortcuts/global_linux.rs`.
//!
//! Everything talks D-Bus through zbus on one small tokio runtime of its own (zbus/ashpd are already in the tree via rfd's portal).
pub(crate) mod activation;
pub(crate) mod clipboard;
pub(crate) mod instance;
pub(crate) mod launcher;
pub(crate) mod notifications;
pub(crate) mod sni;
pub(crate) mod x11;

use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::OnceLock;

/// Which windowing system the main window ended up on (winit prefers Wayland when WAYLAND_DISPLAY is set).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Session { Unknown, X11, Wayland }

static SESSION: AtomicU8 = AtomicU8::new(0);
static SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32

pub(crate) fn session() -> Session {
    match SESSION.load(Ordering::Relaxed) { 1 => Session::X11, 2 => Session::Wayland, _ => Session::Unknown }
}
pub(crate) fn set_session(s: Session) { SESSION.store(match s { Session::X11 => 1, Session::Wayland => 2, Session::Unknown => 0 }, Ordering::Relaxed); }
pub(crate) fn is_x11() -> bool { session() == Session::X11 }

/// The main window's native pixels per point (winit's scale factor), kept current by the UI thread.
pub(crate) fn set_scale(s: f32) { SCALE.store(s.to_bits(), Ordering::Relaxed); }
pub(crate) fn scale() -> f32 { f32::from_bits(SCALE.load(Ordering::Relaxed)).max(0.5) }

/// Called once per frame by the main window with its raw handle: records X11 vs Wayland and returns the X11 window id
/// (0 on Wayland, where windows have no global id). On Wayland it hands the wl_display / wl_surface to `activation`.
pub(crate) fn note_window(frame: &eframe::Frame, ctx: &eframe::egui::Context) -> Option<isize> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    if let Some(p) = ctx.input(|i| i.viewport().native_pixels_per_point) { set_scale(p); }
    let h = frame.window_handle().ok()?;
    match h.as_raw() {
        RawWindowHandle::Xlib(w) => { set_session(Session::X11); Some(w.window as isize) }
        RawWindowHandle::Xcb(w) => { set_session(Session::X11); Some(w.window.get() as isize) }
        RawWindowHandle::Wayland(w) => {
            set_session(Session::Wayland);
            if let Ok(RawDisplayHandle::Wayland(d)) = frame.display_handle().map(|d| d.as_raw()) { activation::note(d.display.as_ptr() as usize, w.surface.as_ptr() as usize); }
            Some(0)
        }
        _ => None,
    }
}

/// The runtime all D-Bus work runs on (one worker thread; zbus spawns its socket reader there).
pub(crate) fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).thread_name("webox-dbus").enable_all().build()
        .expect("tokio runtime"))
}

/// The shared session-bus connection (None without a session bus, e.g. in a bare container).
pub(crate) async fn bus() -> Option<zbus::Connection> {
    static BUS: tokio::sync::OnceCell<Option<zbus::Connection>> = tokio::sync::OnceCell::const_new();
    BUS.get_or_init(|| async { zbus::Connection::session().await.ok() }).await.clone()
}

/// Runs `f` on the D-Bus runtime and waits for it (at most `ms` milliseconds).
pub(crate) fn block<T: Send + 'static>(ms: u64, f: impl std::future::Future<Output = T> + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    rt().spawn(async move { let _ = tx.send(f.await); });
    rx.recv_timeout(std::time::Duration::from_millis(ms)).ok()
}

/// The desktop file name the launcher / notification servers know us by (packaging installs `webox.desktop`).
pub(crate) const DESKTOP_ID: &str = "webox";
