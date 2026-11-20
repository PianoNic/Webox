//! The taskbar badge on Linux: com.canonical.Unity.LauncherEntry `Update` signal with the unread count (the Linux counterpart of
//! ITaskbarList3::SetOverlayIcon). Read by the KDE Plasma task manager, Ubuntu Dock / Dash to Dock, Plank and Latte; ignored
//! silently elsewhere. The flash itself goes through winit's RequestUserAttention (X11 urgency hint / Wayland xdg-activation).
use std::collections::HashMap;
use zbus::zvariant::Value;

/// Shows `count` on the launcher icon of `webox.desktop` (0 hides the badge).
pub(crate) fn set_count(count: usize) {
    super::rt().spawn(async move {
        let Some(bus) = super::bus().await else { return };
        let uri = format!("application://{}.desktop", super::DESKTOP_ID);
        let mut props: HashMap<&str, Value> = HashMap::new();
        props.insert("count", Value::from(count as i64));
        props.insert("count-visible", Value::from(count > 0));
        props.insert("urgent", Value::from(false));
        let _ = bus.emit_signal(None::<&str>, "/com/canonical/unity/launcherentry/webox", "com.canonical.Unity.LauncherEntry", "Update", &(uri, props)).await;
    });
}
