//! Edge WebView2 embedded as a child of the app's own HWND.
use std::sync::mpsc::{channel, Receiver};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2ContentLoadingEventArgs, ICoreWebView2Controller,
    ICoreWebView2Environment, ICoreWebView2NavigationCompletedEventArgs,
    ICoreWebView2NavigationStartingEventArgs, ICoreWebView2ProcessFailedEventArgs,
    ICoreWebView2WebMessageReceivedEventArgs, CreateCoreWebView2Environment,
};
use webview2_com::{
    take_pwstr, AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    ContentLoadingEventHandler, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DocumentTitleChangedEventHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler, ProcessFailedEventHandler,
    WebMessageReceivedEventHandler,
};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::WinRT::EventRegistrationToken;

/// Append one diagnostic line to the shared auth-debug log.
fn log(msg: String) { crate::cloud::auth::dbg_auth(format!("WEBVIEW {msg}")); }

/// A live embedded WebView2, owned on the UI thread (COM, single-threaded — never `Send`).
pub struct WebView {
    controller: ICoreWebView2Controller,
    _webview: ICoreWebView2,
    rx: Receiver<super::Captured>,
}

/// This platform paints the webview *inside* the app window, so the caller must give it bounds.
pub const EMBEDDED: bool = true;

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }

impl WebView {
    /// Creates the control as a child of `parent` (the app's HWND) and navigates to `url`.
    /// `user_data_dir` keeps cookies/session in our own profile.
    /// Environment + controller creation briefly pumps the Win32 message loop (webview2-com's
    /// `wait_for_async_operation`); everything after is callback-driven on the UI thread.
    pub fn start(parent: Option<isize>, url: &str, user_data_dir: &std::path::Path) -> Result<Self, String> {
        let hwnd = HWND(parent.ok_or("no window handle yet")? as *mut core::ffi::c_void);
        // WebView2 honours this for the profile location (avoids the non-writable exe folder default).
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", user_data_dir);

        let env: ICoreWebView2Environment = {
            let (tx, rx) = channel();
            CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
                Box::new(|h| unsafe { CreateCoreWebView2Environment(&h).map_err(webview2_com::Error::WindowsError) }),
                Box::new(move |code, env| { code?; let _ = tx.send(env); Ok(()) }),
            ).map_err(|e| format!("WebView2 runtime unavailable: {e}"))?;
            rx.recv().ok().flatten().ok_or("WebView2 environment creation failed")?
        };

        let controller: ICoreWebView2Controller = {
            let (tx, rx) = channel();
            let env = env.clone();
            CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
                Box::new(move |h| unsafe { env.CreateCoreWebView2Controller(hwnd, &h).map_err(webview2_com::Error::WindowsError) }),
                Box::new(move |code, c| { code?; let _ = tx.send(c); Ok(()) }),
            ).map_err(|e| format!("WebView2 controller creation failed: {e}"))?;
            rx.recv().ok().flatten().ok_or("WebView2 controller creation failed")?
        };

        let webview = unsafe { controller.CoreWebView2().map_err(|e| e.to_string())? };

        // Inject the SCRAPER into every document before it loads (AddScriptToExecuteOnDocumentCreated).
        for src in [super::SCRAPER.to_string()] {
            let js = wide(&src);
            let wv_script = webview.clone();
            let _ = AddScriptToExecuteOnDocumentCreatedCompletedHandler::wait_for_async_operation(
                Box::new(move |h| unsafe { wv_script.AddScriptToExecuteOnDocumentCreated(PCWSTR(js.as_ptr()), &h).map_err(webview2_com::Error::WindowsError) }),
                Box::new(|code, _id| { code?; Ok(()) }),
            );
        }

        let (tx, rx) = channel();
        let mut toks = [EventRegistrationToken::default(); 5];
        unsafe {
            // 1) Host-bridge / postMessage delivery (idbroker.authorize.response, or the SCRAPER's
            //    webox-code:/webox-log: posts). Log BOTH forms: idbroker may post a JSON object,
            //    for which the string form is empty but WebMessageAsJson carries it. Codes are redacted.
            let tx_msg = tx.clone();
            webview.add_WebMessageReceived(
                &WebMessageReceivedEventHandler::create(Box::new(move |_wv, args: Option<ICoreWebView2WebMessageReceivedEventArgs>| {
                    if let Some(args) = args {
                        let mut sp = PWSTR::null();
                        let as_str = if args.TryGetWebMessageAsString(&mut sp).is_ok() { take_pwstr(sp) } else { String::new() };
                        let mut jp = PWSTR::null();
                        let as_json = if args.WebMessageAsJson(&mut jp).is_ok() { take_pwstr(jp) } else { String::new() };
                        log(format!("webmsg str={:?} json={:?}", super::redact(&as_str), super::redact(&as_json)));
                        // Act only on the SCRAPER's explicit webox-code:<code>.
                        let msg = if !as_str.is_empty() { &as_str } else { &as_json };
                        if let Some(c) = super::classify(msg) {
                            let super::Captured::Code(code) = &c;
                            log(format!("RECV code len={}", code.len()));
                            let _ = tx_msg.send(c);
                        }
                    }
                    Ok(())
                })),
                &mut toks[0],
            ).map_err(|e| e.to_string())?;
            // 2) Log every navigation URL; intercept an explicit OOB redirect (`urn:ietf:wg:oauth:2.0:oob?code=…`)
            //    so non-SSO flows that redirect rather than rendering the hidden-code HTML page also work.
            let tx_nav = tx.clone();
            webview.add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(move |_wv, args: Option<ICoreWebView2NavigationStartingEventArgs>| {
                    if let Some(a) = args {
                        let mut pw = PWSTR::null();
                        if a.Uri(&mut pw).is_ok() {
                            let uri = take_pwstr(pw);
                            let cap = super::capture(&uri);
                            log(format!("nav-start hasCode={} uri={:?}", cap.is_some(), super::log_url(&uri)));
                            if uri.starts_with("urn:ietf:wg:oauth:2.0:oob") {
                                if let Some(code) = cap {
                                    let _ = a.SetCancel(true);
                                    let _ = tx_nav.send(super::Captured::Code(code));
                                }
                            }
                        }
                    }
                    Ok(())
                })),
                &mut toks[1],
            ).map_err(|e| e.to_string())?;
            // 3) Navigation result: success + the WebView2 error status (tells nav-failed from blank-page).
            let wv_nc = webview.clone();
            webview.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(Box::new(move |_wv, args: Option<ICoreWebView2NavigationCompletedEventArgs>| {
                    if let Some(a) = args {
                        let mut ok = windows::Win32::Foundation::BOOL(0);
                        let _ = a.IsSuccess(&mut ok);
                        let mut err = Default::default();
                        let _ = a.WebErrorStatus(&mut err);
                        let mut sp = PWSTR::null();
                        let src = if wv_nc.Source(&mut sp).is_ok() { take_pwstr(sp) } else { String::new() };
                        log(format!("nav-done success={} webErrorStatus={} source={:?}", ok.as_bool(), err.0, super::log_url(&src)));
                    }
                    Ok(())
                })),
                &mut toks[2],
            ).map_err(|e| e.to_string())?;
            // 4) Content loading (is an error page? still loading?) — helps explain a blank render.
            webview.add_ContentLoading(
                &ContentLoadingEventHandler::create(Box::new(move |_wv, args: Option<ICoreWebView2ContentLoadingEventArgs>| {
                    if let Some(a) = args {
                        let mut ep = windows::Win32::Foundation::BOOL(0);
                        let _ = a.IsErrorPage(&mut ep);
                        log(format!("content-loading isErrorPage={}", ep.as_bool()));
                    }
                    Ok(())
                })),
                &mut toks[3],
            ).map_err(|e| e.to_string())?;
            // 5) Renderer process failure (crash) — the other reason a page blanks.
            webview.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(move |_wv, args: Option<ICoreWebView2ProcessFailedEventArgs>| {
                    if let Some(a) = args {
                        let mut kind = Default::default();
                        let _ = a.ProcessFailedKind(&mut kind);
                        log(format!("PROCESS-FAILED kind={}", kind.0));
                    }
                    Ok(())
                })),
                &mut toks[4],
            ).map_err(|e| e.to_string())?;
            // Title changes (also a cheap liveness signal).
            let wv_dt = webview.clone();
            let mut dt_tok = EventRegistrationToken::default();
            let _ = webview.add_DocumentTitleChanged(
                &DocumentTitleChangedEventHandler::create(Box::new(move |_wv, _args: Option<windows::core::IUnknown>| {
                    let mut sp = PWSTR::null();
                    if wv_dt.DocumentTitle(&mut sp).is_ok() { log(format!("title={:?}", take_pwstr(sp))); }
                    Ok(())
                })),
                &mut dt_tok,
            );
            let _ = controller.SetIsVisible(true);
            let u = wide(url);
            webview.Navigate(PCWSTR(u.as_ptr())).map_err(|e| e.to_string())?;
        }
        Ok(WebView { controller, _webview: webview, rx })
    }

    /// A token (redeemed in-webview) or a code (ureq fallback), once the page has handed one back.
    pub fn poll(&self) -> Option<super::Captured> { self.rx.try_recv().ok() }

    /// Position the control over the given client-area rectangle, in physical pixels.
    pub fn set_bounds(&self, x: i32, y: i32, w: i32, h: i32) {
        let _ = unsafe { self.controller.SetBounds(RECT { left: x, top: y, right: x + w.max(0), bottom: y + h.max(0) }) };
    }
}

impl Drop for WebView {
    fn drop(&mut self) { let _ = unsafe { self.controller.Close() }; }
}
