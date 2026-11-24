//! macOS: WKWebView (WebKit, part of the OS) in a window of its own, like WebKitGTK on Linux: eframe owns its NSWindow's content
//! view, so the sign-in page gets a small titled window next to it. Everything runs on the main thread, where eframe's `update`
//! (and so `start`, `poll` and `Drop`) already runs; WebKit calls the delegate methods below on the same thread.
//!
//! The SCRAPER posts to `window.webkit.messageHandlers.webox` (the same name WebKitGTK uses); committed / redirected URLs are
//! checked for a `code=` as well. Cookies live in WebKit's default data store for this app, so idbroker can remember the session.
#![allow(unused_unsafe)] // objc2 marks some of these calls safe, others not; one style for all of them
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowDelegate, NSWindowStyleMask};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURLRequest, NSURL};
use objc2_web_kit::{
    WKNavigation, WKNavigationDelegate, WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
};
use std::sync::mpsc::{channel, Receiver, Sender};

/// A separate window: no in-app bounds.
pub const EMBEDDED: bool = false;

fn log(msg: String) { crate::cloud::auth::dbg_auth(format!("WKWEBVIEW {msg}")); }

pub struct Ivars { tx: Sender<super::Captured> }

define_class!(
    // The script message handler, navigation delegate and window delegate of one sign-in window.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WeboxSigninDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl WKScriptMessageHandler for Delegate {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _ucc: &WKUserContentController, msg: &WKScriptMessage) {
            let body: Retained<AnyObject> = unsafe { msg.body() };
            let Some(s) = body.downcast_ref::<NSString>().map(|s| s.to_string()) else { return };
            if let Some(rest) = s.strip_prefix("webox-log:") { log(super::redact(rest).chars().take(2000).collect()); }
            if let Some(c) = super::classify(&s) { let _ = self.ivars().tx.send(c); }
        }
    }

    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start(&self, wv: &WKWebView, _nav: Option<&WKNavigation>) { self.check_url(wv); }

        #[unsafe(method(webView:didReceiveServerRedirectForProvisionalNavigation:))]
        fn did_redirect(&self, wv: &WKWebView, _nav: Option<&WKNavigation>) { self.check_url(wv); }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, wv: &WKWebView, _nav: Option<&WKNavigation>) { self.check_url(wv); }
    }

    unsafe impl NSWindowDelegate for Delegate {
        // The user closed the window: the sign-in page goes back to the email step.
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) { let _ = self.ivars().tx.send(super::Captured::Closed); }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, tx: Sender<super::Captured>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { tx });
        unsafe { msg_send![super(this), init] }
    }

    /// The SCRAPER's `location.hash` fallback and any `code=`-carrying navigation.
    fn check_url(&self, wv: &WKWebView) {
        let Some(u) = (unsafe { wv.URL() }).and_then(|u| unsafe { u.absoluteString() }) else { return };
        let dec = super::percent_decode(&u.to_string());
        if let Some(c) = super::classify(&dec).or_else(|| super::capture(&dec).map(super::Captured::Code)) { let _ = self.ivars().tx.send(c); }
    }
}

pub struct WebView {
    rx: Receiver<super::Captured>,
    window: Retained<NSWindow>,
    ucc: Retained<WKUserContentController>,
    _delegate: Retained<Delegate>,
}

impl WebView {
    pub fn start(_parent: Option<isize>, url: &str, _user_data_dir: &std::path::Path) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("WKWebView must be created on the main thread")?;
        let (tx, rx) = channel();
        let delegate = Delegate::new(mtm, tx);
        unsafe {
            let config = WKWebViewConfiguration::new(mtm);
            let ucc = config.userContentController();
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                WKUserScript::alloc(mtm), &NSString::from_str(super::SCRAPER), WKUserScriptInjectionTime::AtDocumentStart, false);
            ucc.addUserScript(&script);
            ucc.addScriptMessageHandler_name(ProtocolObject::from_ref(&*delegate), &NSString::from_str("webox"));

            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(820.0, 640.0));
            let webview = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config);
            webview.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));

            let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable;
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false);
            window.setReleasedWhenClosed(false); // the Retained below owns it
            window.setTitle(&NSString::from_str("Webex"));
            window.setContentView(Some(&webview));
            window.center();
            window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            window.makeKeyAndOrderFront(None);

            let nsurl = NSURL::URLWithString(&NSString::from_str(url)).ok_or("invalid sign-in URL")?;
            webview.loadRequest(&NSURLRequest::requestWithURL(&nsurl));
            log(format!("opened {}", super::log_url(url)));
            Ok(WebView { rx, window, ucc, _delegate: delegate })
        }
    }

    pub fn poll(&self) -> Option<super::Captured> { self.rx.try_recv().ok() }
    pub fn set_bounds(&self, _x: i32, _y: i32, _w: i32, _h: i32) {}
}

impl Drop for WebView {
    /// The app closes the window (Back, sign-in done): no `Closed` for that, and the handler is let go (WebKit retains it).
    fn drop(&mut self) {
        unsafe {
            self.window.setDelegate(None);
            self.ucc.removeScriptMessageHandlerForName(&NSString::from_str("webox"));
            self.window.close();
        }
    }
}
