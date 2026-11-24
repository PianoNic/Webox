//! macOS: the toasts go to Notification Center (`platform::macos::notify`) instead of the app's own toast window, which macOS
//! would neither keep on top of full-screen apps nor place clear of the Dock. `WEBOX_NATIVE_TOASTS=0` keeps the app's own.
//! Buttons (join, snooze) and the click-to-open of a space need `UNUserNotificationCenter` in a signed bundle: not there yet.
use super::*;

impl App {
    pub(super) fn mac_toasts(&mut self) {
        if self.shot.is_some() || std::env::var("WEBOX_NATIVE_TOASTS").is_ok_and(|v| v == "0") { return; }
        for ts in std::mem::take(&mut self.notify.toasts) {
            if ts.dismissed { continue; }
            let lead = ts.lead.as_ref().map(|l| l.0.as_str()).unwrap_or("");
            let body = [ts.kicker.as_str(), lead, ts.body.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n");
            crate::platform::macos::notify(&ts.title, &body);
        }
    }
}
