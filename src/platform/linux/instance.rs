//! One running copy per user session (Linux): the first instance owns the bus name `io.github.webox.Webox`; a second start
//! (launcher click while the window sits in the tray, a `collabhost://` link, autostart racing a manual start) hands its
//! link to the first one via `ActivateWithToken(uri, token)` and exits. The first one then shows its window and opens the
//! linked space; the token is the second start's `XDG_ACTIVATION_TOKEN` (the launcher's, so Wayland lets the window come to the
//! front, see activation.rs). Without a session bus every start runs on its own.
use std::sync::Mutex;

const NAME: &str = "io.github.webox.Webox";
const PATH: &str = "/io/github/webox/Webox";

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CTX: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();

struct Instance;

#[zbus::interface(name = "io.github.webox.Webox")]
impl Instance {
    fn activate(&self, uri: String) {
        PENDING.lock().unwrap().push(uri);
        if let Some(c) = CTX.get() { c.request_repaint(); }
    }
    fn activate_with_token(&self, uri: String, token: String) {
        super::activation::offer(token);
        self.activate(uri);
    }
}

/// True when another instance took over (the caller exits). Otherwise this process now owns the name.
pub(crate) fn hand_off(uri: Option<&str>) -> bool {
    let uri = uri.unwrap_or("").to_string();
    let token = std::env::var("XDG_ACTIVATION_TOKEN").unwrap_or_default();
    super::block(2000, async move {
        let bus = super::bus().await?;
        let _ = bus.object_server().at(PATH, Instance).await;
        use zbus::fdo::{RequestNameFlags, RequestNameReply};
        match bus.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into()).await {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Some(false),
            // An older running build only knows Activate(uri).
            _ => Some(bus.call_method(Some(NAME), PATH, Some(NAME), "ActivateWithToken", &(uri.as_str(), token.as_str())).await.is_ok()
                || bus.call_method(Some(NAME), PATH, Some(NAME), "Activate", &(uri.as_str(),)).await.is_ok()),
        }
    }).flatten().unwrap_or(false)
}

/// Links (possibly empty) another start handed over since the last frame.
pub(crate) fn take(ctx: &eframe::egui::Context) -> Vec<String> {
    let _ = CTX.set(ctx.clone());
    std::mem::take(&mut *PENDING.lock().unwrap())
}
