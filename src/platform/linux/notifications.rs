//! Desktop notifications (org.freedesktop.Notifications) for the toasts on Wayland, where a client can't put its own window at
//! the bottom right of the screen. Same title, text and buttons as the app's toast windows; a click on the notification body is
//! the "default" action (open the space), a button is its id. `send` never blocks: the server's id (None without a notification
//! server) lands in the returned slot, and the caller falls back to the app's own toast window.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use zbus::zvariant::Value;

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

static INVOKED: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());
static CLOSED: Mutex<Vec<u32>> = Mutex::new(Vec::new());
static CTX: OnceLock<eframe::egui::Context> = OnceLock::new();

/// Where a sent notification's id arrives: unset while the server is asked, then Some(id) or None (failed).
pub(crate) type Sent = Arc<OnceLock<Option<u32>>>;

async fn listen(bus: zbus::Connection) {
    use futures_util::StreamExt;
    let Ok(p) = zbus::Proxy::new(&bus, DEST, PATH, DEST).await else { return };
    let (Ok(mut act), Ok(mut closed)) = (p.receive_signal("ActionInvoked").await, p.receive_signal("NotificationClosed").await) else { return };
    // Sent just before ActionInvoked by servers that support it (KDE, GNOME 46+): the token that lets the window come to the front.
    let mut token = p.receive_signal("ActivationToken").await.ok();
    loop {
        tokio::select! {
            Some(m) = act.next() => if let Ok((id, key)) = m.body().deserialize::<(u32, String)>() { INVOKED.lock().unwrap().push((id, key)); }
                else { continue },
            Some(m) = closed.next() => if let Ok((id, _reason)) = m.body().deserialize::<(u32, u32)>() { CLOSED.lock().unwrap().push(id); }
                else { continue },
            Some(m) = async { match &mut token { Some(t) => t.next().await, None => std::future::pending().await } } => {
                if let Ok((_id, t)) = m.body().deserialize::<(u32, String)>() { super::activation::offer(t); }
                continue
            }
            else => break,
        }
        if let Some(c) = CTX.get() { c.request_repaint(); }
    }
}

/// Shows a notification; `actions` are (key, label) pairs, "default" = click on the body; `urgency` 0 low, 1 normal, 2 critical.
pub(crate) fn send(ctx: &eframe::egui::Context, summary: &str, body: &str, actions: &[(&str, &str)], urgency: u8) -> Sent {
    let _ = CTX.set(ctx.clone());
    let (summary, body) = (summary.to_string(), body.to_string());
    let actions: Vec<String> = actions.iter().flat_map(|(k, l)| [k.to_string(), l.to_string()]).collect();
    let (slot, ctx) = (Sent::default(), ctx.clone());
    let s2 = slot.clone();
    super::rt().spawn(async move {
        let call = async move {
            static LISTENING: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
            let bus = super::bus().await?;
            let b2 = bus.clone();
            LISTENING.get_or_init(|| async move { tokio::spawn(listen(b2)); }).await;
            let mut hints: HashMap<&str, Value> = HashMap::new();
            hints.insert("desktop-entry", Value::from(super::DESKTOP_ID));
            hints.insert("urgency", Value::from(urgency.min(2)));
            // The app plays its own message sound / ringtone (sounds.rs); the server must not add one.
            hints.insert("suppress-sound", Value::from(true));
            let reply = bus.call_method(Some(DEST), PATH, Some(DEST), "Notify",
                &("Webox", 0u32, super::DESKTOP_ID, summary.as_str(), body.as_str(), actions, hints, -1i32)).await.ok()?;
            reply.body().deserialize::<u32>().ok()
        };
        let _ = s2.set(tokio::time::timeout(std::time::Duration::from_secs(3), call).await.ok().flatten());
        ctx.request_repaint();
    });
    slot
}

/// Removes a notification (e.g. the toast was handled in the app).
pub(crate) fn close(id: u32) {
    super::rt().spawn(async move {
        if let Some(bus) = super::bus().await { let _ = bus.call_method(Some(DEST), PATH, Some(DEST), "CloseNotification", &(id,)).await; }
    });
}

/// (notification id, action key) pairs clicked since the last call.
pub(crate) fn invoked() -> Vec<(u32, String)> { std::mem::take(&mut *INVOKED.lock().unwrap()) }
/// Notification ids the server closed (expired, dismissed, or after an action).
pub(crate) fn closed() -> Vec<u32> { std::mem::take(&mut *CLOSED.lock().unwrap()) }
