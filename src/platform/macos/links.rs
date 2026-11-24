//! `webexteams://` / `collabhost://` links (declared as CFBundleURLTypes in Webox.app's Info.plist). macOS hands them over as a
//! kAEGetURL Apple Event, both when the link starts the app and while it runs (Launch Services keeps one copy, so there is no
//! second start to hand over as on Linux). winit owns the app delegate, so the handler sits on NSAppleEventManager. It is
//! registered once more when the app is about to finish launching: AppKit installs its own handlers before that point, and the
//! event of a link that launched the app arrives after it. Links queue here; the frame takes them (notify/hub.rs, the same
//! path as Linux's hand-over: show the window, open the space or keep it as `pending_space`).
#![allow(unused_unsafe)] // objc2 marks some of these calls safe, others not
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSApplicationWillFinishLaunchingNotification;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSString};
use std::sync::Mutex;

/// 'GURL': both kInternetEventClass and kAEGetURL.
const GURL: u32 = u32::from_be_bytes(*b"GURL");
/// '----': keyDirectObject, the parameter that holds the link.
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CTX: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WeboxLinkHandler"]
    struct Handler;

    unsafe impl NSObjectProtocol for Handler {}

    impl Handler {
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _n: &NSNotification) { register(self); }

        #[unsafe(method(handleGetURL:withReplyEvent:))]
        fn handle_get_url(&self, event: &AnyObject, _reply: Option<&AnyObject>) {
            let Some(uri) = url_of(event) else { return };
            eprintln!("webox: link {uri}");
            PENDING.lock().unwrap().push(uri);
            if let Some(c) = CTX.get() { c.request_repaint(); }
        }
    }
);

thread_local! {
    /// The one handler (neither the notification center nor the Apple Event manager keeps it alive).
    static HANDLER: std::cell::OnceCell<Retained<Handler>> = const { std::cell::OnceCell::new() };
}

fn register(h: &Handler) {
    unsafe {
        let mgr: Retained<AnyObject> = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
        let _: () = msg_send![&*mgr, setEventHandler: h, andSelector: sel!(handleGetURL:withReplyEvent:), forEventClass: GURL, andEventID: GURL];
    }
}

/// The link of a kAEGetURL event (its direct parameter).
fn url_of(event: &AnyObject) -> Option<String> {
    let param: Option<Retained<AnyObject>> = unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
    let s: Option<Retained<NSString>> = unsafe { msg_send![&*param?, stringValue] };
    s.map(|s| s.to_string()).filter(|s| !s.is_empty())
}

/// Before `eframe::run_native` (which starts NSApplication), on the main thread.
pub fn install() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let h = HANDLER.with(|c| c.get_or_init(|| unsafe { msg_send![Handler::alloc(mtm), init] }).clone());
    register(&h);
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(&*h, sel!(willFinishLaunching:),
            Some(NSApplicationWillFinishLaunchingNotification), None);
    }
}

/// Links that arrived since the last frame.
pub(crate) fn take(ctx: &eframe::egui::Context) -> Vec<String> {
    let _ = CTX.set(ctx.clone());
    std::mem::take(&mut *PENDING.lock().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kAEGetURL event as Launch Services sends it, with `url` as its direct parameter (none for `None`).
    fn event(url: Option<&str>) -> Retained<AnyObject> {
        unsafe {
            let none: Option<&AnyObject> = None;
            // kAutoGenerateReturnID = -1, kAnyTransactionID = 0
            let ev: Retained<AnyObject> = msg_send![class!(NSAppleEventDescriptor), appleEventWithEventClass: GURL, eventID: GURL,
                targetDescriptor: none, returnID: -1i16, transactionID: 0i32];
            if let Some(u) = url {
                let d: Retained<AnyObject> = msg_send![class!(NSAppleEventDescriptor), descriptorWithString: &*NSString::from_str(u)];
                let _: () = msg_send![&*ev, setParamDescriptor: &*d, forKeyword: DIRECT_OBJECT];
            }
            ev
        }
    }

    #[test]
    fn reads_the_link_of_a_get_url_event() {
        let url = url_of(&event(Some("webexteams://im?space=s2"))).expect("link");
        assert_eq!(url, "webexteams://im?space=s2");
        assert_eq!(crate::system::space_from_uri(&url).as_deref(), Some("s2"));
        assert_eq!(url_of(&event(Some("collabhost://space/abc"))).as_deref(), Some("collabhost://space/abc"));
        assert_eq!(url_of(&event(None)), None);
    }
}
