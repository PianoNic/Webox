//! Raising the main window on Wayland (xdg-activation-v1). winit 0.30 ignores Visible / Minimized(false) / Focus there; a
//! toplevel only comes to the front with an activation token the compositor handed out for a user action: the panel's
//! (StatusNotifierItem `ProvideXdgActivationToken`, sni.rs), the notification server's (`ActivationToken`, notifications.rs) or
//! the launching process's `XDG_ACTIVATION_TOKEN` (instance.rs). Without one the window asks for a token of its own (no input
//! serial, so KWin usually just marks the taskbar entry as demanding attention).
//!
//! A second wl_event_queue on winit's own wl_display (Backend::from_foreign_display), used only from the UI thread; the main
//! window's wl_surface is wrapped by pointer. Every failure is silent; waiting for the compositor is capped at a few ms, a late
//! token is picked up by `note` on a later frame.
use std::sync::Mutex;
use std::time::{Duration, Instant};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::{wl_callback::{self, WlCallback}, wl_registry::{self, WlRegistry}, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1::{self, XdgActivationTokenV1}, xdg_activation_v1::XdgActivationV1};

#[derive(Default)]
struct St { act: Option<XdgActivationV1>, synced: bool, token: Option<String> }

struct Act { conn: Connection, q: EventQueue<St>, st: St, surface: Option<(usize, WlSurface)>, pending: Option<XdgActivationTokenV1> }

/// None = no Wayland window seen yet. Never dropped, even without the global: winit owns the display, and the registry proxy
/// must keep a live queue for globals announced later.
static ACT: Mutex<Option<Act>> = Mutex::new(None);
/// A token offered by the panel / notification server / a second start, with its arrival (stale ones are not used).
static OFFER: Mutex<Option<(String, Instant)>> = Mutex::new(None);

impl Dispatch<WlRegistry, ()> for St {
    fn event(st: &mut Self, reg: &WlRegistry, e: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, .. } = e {
            if interface == XdgActivationV1::interface().name && st.act.is_none() { st.act = Some(reg.bind(name, 1, qh, ())); }
        }
    }
}
impl Dispatch<WlCallback, ()> for St {
    fn event(st: &mut Self, _: &WlCallback, e: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = e { st.synced = true; }
    }
}
impl Dispatch<XdgActivationV1, ()> for St {
    fn event(_: &mut Self, _: &XdgActivationV1, _: <XdgActivationV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<XdgActivationTokenV1, ()> for St {
    fn event(st: &mut Self, _: &XdgActivationTokenV1, e: xdg_activation_token_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_activation_token_v1::Event::Done { token } = e { st.token = Some(token); }
    }
}

/// Dispatches this queue until `done` or `ms` ran out (0 = only what is already there / readable now).
fn pump(q: &mut EventQueue<St>, st: &mut St, ms: u64, done: impl Fn(&St) -> bool) {
    use std::os::fd::AsRawFd;
    let end = Instant::now() + Duration::from_millis(ms);
    loop {
        if q.dispatch_pending(st).is_err() || done(st) { return; }
        let _ = q.flush();
        let Some(g) = q.prepare_read() else { continue }; // events queued meanwhile: dispatch them first
        let left = end.saturating_duration_since(Instant::now()).as_millis() as i32;
        let mut p = libc::pollfd { fd: g.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut p, 1, left) } <= 0 || g.read().is_err() { return; } // dropping the guard cancels the read
    }
}

fn bind(display: usize) -> Act {
    // Safety: `display` is winit's live wl_display (raw_window_handle of the main window); it outlives this process' use of it.
    let conn = Connection::from_backend(unsafe { Backend::from_foreign_display(display as *mut _) });
    let mut q = conn.new_event_queue();
    let qh = q.handle();
    let _reg = conn.display().get_registry(&qh, ());
    conn.display().sync(&qh, ());
    let mut st = St::default();
    pump(&mut q, &mut st, 100, |s| s.synced); // one roundtrip, well under 1 ms on a live compositor
    Act { conn, q, st, surface: None, pending: None }
}

/// Per frame from `note_window`: the main window's wl_display / wl_surface pointers. Binds once, collects a late own token.
pub(crate) fn note(display: usize, surface: usize) {
    let mut g = ACT.lock().unwrap();
    let a = g.get_or_insert_with(|| bind(display));
    let _ = a.q.dispatch_pending(&mut a.st); // cheap: what winit's reads queued for us (e.g. a global announced later)
    if a.surface.as_ref().map(|s| s.0) != Some(surface) {
        // Safety: the pointer is the main window's live wl_surface (winit's, rust-managed: its liveness is tracked).
        a.surface = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface as *mut _) }.ok()
            .and_then(|id| WlSurface::from_id(&a.conn, id).ok()).map(|s| (surface, s));
    }
    if a.pending.is_some() { finish(a, 0); }
}

/// True when the compositor offers xdg-activation (only known once the window was seen; false before that).
pub(crate) fn available() -> bool { ACT.lock().unwrap().as_ref().is_some_and(|a| a.st.act.is_some()) }

/// Keeps a token for the next `activate(None)` (panel, notification server, second start; any thread).
pub(crate) fn offer(token: String) {
    if !token.is_empty() { *OFFER.lock().unwrap() = Some((token, Instant::now())); }
}

fn finish(a: &mut Act, ms: u64) {
    pump(&mut a.q, &mut a.st, ms, |s| s.token.is_some());
    let Some(t) = a.st.token.take() else { return };
    if let Some(p) = a.pending.take() { p.destroy(); }
    let _ = use_token(a, t);
}

fn use_token(a: &mut Act, token: String) -> bool {
    let (Some(act), Some((_, s))) = (&a.st.act, &a.surface) else { return false };
    act.activate(token, s);
    let _ = a.conn.flush();
    true
}

/// Raises the main window: with `token`, else a fresh offered one, else a token requested for the window itself (KWin mostly
/// just marks the window as demanding attention then). True when a handed-in token was used.
pub(crate) fn activate(token: Option<String>) -> bool {
    let token = token.or_else(|| OFFER.lock().unwrap().take().filter(|(_, at)| at.elapsed() < Duration::from_secs(30)).map(|o| o.0));
    let mut g = ACT.lock().unwrap();
    let Some(a) = g.as_mut() else { return false };
    if let Some(t) = token { return use_token(a, t); }
    let (Some(act), Some((_, s))) = (&a.st.act, &a.surface) else { return false };
    if let Some(p) = a.pending.take() { p.destroy(); }
    let qh = a.q.handle();
    let p = act.get_activation_token(&qh, ());
    p.set_surface(s);
    p.set_app_id(super::DESKTOP_ID.to_string());
    p.commit();
    a.st.token = None;
    a.pending = Some(p);
    finish(a, 30);
    false
}
