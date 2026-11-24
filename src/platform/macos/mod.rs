//! macOS integration that has a simple native answer: the settings folder, opening links / files / folders with `open`, revealing a
//! download in Finder (`open -R`) and Notification Center banners (`osascript`). The sign-in window is `webview/platform_mac.rs`.
//!
//! Not built yet (each has a no-op stub where Windows / Linux have code):
//! * menu-bar icon (tray): `NSStatusBar` / `NSStatusItem`; the close button quits instead of hiding to the tray;
//! * Dock badge with the unread count: `NSApp.dockTile.badgeLabel`;
//! * global hotkeys: `RegisterEventHotKey` (Carbon) or a `CGEventTap`;
//! * screen sharing: ScreenCaptureKit (`SCStream`) + VideoToolbox (`VTCompressionSession`) for H.264;
//! * cameras: AVFoundation (`AVCaptureSession`); video playback: AVFoundation (`AVAssetReader`);
//! * start at login: `SMAppService.mainApp` (macOS 13+);
//! * secret plugin settings in the Keychain: `SecItemAdd` / `SecItemCopyMatching`;
//! * in-app updates: the app only reports a new release and links to the GitHub download (no self-replacing of a .app bundle).
use std::path::{Path, PathBuf};

/// `~/Library/Application Support` (the base of the settings folder `WebOx`).
pub fn app_support() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join("Library/Application Support"))
}

/// Opens a link, a file or a folder with its default app (Launch Services).
pub fn open(target: impl AsRef<std::ffi::OsStr>) {
    crate::system::spawn_detached(std::process::Command::new("/usr/bin/open").arg(target));
}

/// Shows `p` selected in a Finder window.
pub fn reveal(p: &Path) {
    crate::system::spawn_detached(std::process::Command::new("/usr/bin/open").arg("-R").arg(p));
}

/// A Notification Center banner. The texts go in as arguments of the script, never into its source, so nothing in a message can
/// run as AppleScript. macOS shows it as coming from "Script Editor"; a click does not open the space (that needs
/// `UNUserNotificationCenter` in a signed bundle).
pub fn notify(title: &str, body: &str) {
    crate::system::spawn_detached(std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run"])
        .args([title, body])
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()));
}
