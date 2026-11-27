//! macOS integration: the settings folder, opening links / files / folders with `open`, revealing a download in Finder
//! (`open -R`), and the submodules below. The sign-in window is `webview/platform_mac.rs`, global hotkeys
//! `shortcuts/global_macos.rs`, cameras / video / screen sharing live in `media/`, the self-update in `updater/macos.rs`.
pub mod appkit; // menu-bar status item, Dock badge, the app menu bar
pub mod keychain; // secret plugin settings
pub mod links; // webexteams:// / collabhost:// links (Apple Events)
pub mod login; // start at login
pub mod notifications; // UNUserNotificationCenter (in Webox.app), else osascript

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

/// A Notification Center banner without an app bundle (`notifications` uses `UNUserNotificationCenter` inside Webox.app). The
/// texts go in as arguments of the script, never into its source, so nothing in a message can run as AppleScript (`--` ends the
/// options: a title like "-e" would otherwise make the body a script line). macOS shows it as coming from "Script Editor", and a
/// click does nothing.
pub fn notify(title: &str, body: &str) {
    crate::system::spawn_detached(std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run", "--"])
        .args([title, body])
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()));
}
