//! OS integration: config dir / arguments / links (`system`), tray + taskbar (`tray`), toast windows (`notify`),
//! the embedded sign-in webview (`webview`), the browser's native messaging host (`nmhost`) and the Linux pieces (`linux`).
pub mod system;
pub mod tray;
pub mod notify; // toast windows + tray
pub mod webview;
pub mod nmhost;
#[cfg(target_os = "linux")]
pub mod linux; // tray (SNI), notifications, launcher badge, X11 helpers
