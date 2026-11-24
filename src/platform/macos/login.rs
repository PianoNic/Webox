//! Start at login. From `Webox.app` on macOS 13+: `SMAppService.mainApp` (the app shows up under System Settings > General >
//! Login Items, where the user can switch it off too). Otherwise (older macOS, a bare binary): a LaunchAgent in
//! `~/Library/LaunchAgents` that starts this program with `--minimized --autostarted`.
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::msg_send;
use std::path::PathBuf;

#[link(name = "ServiceManagement", kind = "framework")]
extern "C" {}

const LABEL: &str = "io.github.pianonic.webox";
/// SMAppServiceStatus
const ENABLED: isize = 1;
const REQUIRES_APPROVAL: isize = 2;

/// `SMAppService.mainAppService`, when the class exists (macOS 13+) and Webox runs from its bundle.
fn main_app() -> Option<*mut AnyObject> {
    if !super::notifications::bundled() { return None; }
    let cls = AnyClass::get(c"SMAppService")?;
    let svc: *mut AnyObject = unsafe { msg_send![cls, mainAppService] };
    (!svc.is_null()).then_some(svc)
}

fn agent_file() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
}

fn xml_escape(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

/// The LaunchAgent's text: this program, minimized, once at login.
pub(crate) fn agent_plist(exe: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>{LABEL}</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\
\t\t<string>--minimized</string>\n\t\t<string>--autostarted</string>\n\t</array>\n\t<key>RunAtLoad</key>\n\t<true/>\n\t<key>ProcessType</key>\n\t<string>Interactive</string>\n</dict>\n</plist>\n",
        xml_escape(exe))
}

pub fn registered() -> bool {
    if let Some(svc) = main_app() {
        let st: isize = unsafe { msg_send![svc, status] };
        if st == ENABLED || st == REQUIRES_APPROVAL { return true; }
    }
    agent_file().is_some_and(|f| f.is_file())
}

pub fn set(on: bool) -> Result<(), String> {
    if let Some(svc) = main_app() {
        let mut err: *mut AnyObject = std::ptr::null_mut();
        let ok: Bool = if on { unsafe { msg_send![svc, registerAndReturnError: &mut err] } } else { unsafe { msg_send![svc, unregisterAndReturnError: &mut err] } };
        if ok.as_bool() {
            if let Some(f) = agent_file() { let _ = std::fs::remove_file(f); } // one mechanism at a time
            return Ok(());
        }
        if !on { // not registered that way: maybe the LaunchAgent is
            if let Some(f) = agent_file() { let _ = std::fs::remove_file(f); }
            return Ok(());
        }
        // fall through to the LaunchAgent
    }
    let f = agent_file().ok_or("kein Benutzerordner")?;
    if !on { let _ = std::fs::remove_file(&f); return Ok(()); }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(d) = f.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
    std::fs::write(&f, agent_plist(&exe.to_string_lossy())).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn launch_agent_starts_this_program_minimized() {
        let p = super::agent_plist("/Applications/A & B.app/Contents/MacOS/webox");
        assert!(p.contains("<string>/Applications/A &amp; B.app/Contents/MacOS/webox</string>"));
        assert!(p.contains("<string>--minimized</string>") && p.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(p.contains("<string>io.github.pianonic.webox</string>"));
    }
}
