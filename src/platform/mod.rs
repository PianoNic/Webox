//! OS integration: config dir / arguments / links (`system`), tray + taskbar (`tray`), toast windows (`notify`),
//! the embedded sign-in webview (`webview`), the browser's native messaging host (`nmhost`) and the Linux pieces (`linux`).
pub mod system;
pub mod tray;
pub mod notify; // toast windows + tray
pub mod webview;
pub mod nmhost;
pub mod updater; // app updates from the GitHub releases
#[cfg(target_os = "linux")]
pub mod linux; // tray (SNI), notifications, launcher badge, X11 helpers
#[cfg(target_os = "macos")]
pub mod macos; // open / reveal, Notification Center, the settings folder
