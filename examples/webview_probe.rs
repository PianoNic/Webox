//! Windows-only probe for the embedded WebView2 sign-in plumbing.
//!
//! Not part of the app — it mirrors what `src/platform/webview/` does (environment → controller parented to
//! a real HWND → inject script → `add_WebMessageReceived`) to check, without a credentialed sign-in, that:
//!   1. the WebView2 control instantiates and parents to our own window,
//!   2. the inject-script → `postMessage` → `WebMessageReceived` → code-extract IPC path works
//!      (it posts a synthetic `webox-code:...` and reads it back), and
//!   3. the control renders the page in-window (navigation completes, screenshot saved).
//!
//! A code appearing after a real password needs credentials and is not covered; this exercises
//! everything up to and including the capture mechanism with a stand-in code.
//!
//!     cargo run --example webview_probe -- "<authorize-url>"   # url optional; defaults to about:blank

#[cfg(windows)]
mod win {

use std::sync::mpsc::channel;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2Environment, ICoreWebView2Controller, ICoreWebView2Environment,
    ICoreWebView2NavigationStartingEventArgs, ICoreWebView2WebMessageReceivedEventArgs,
};
use webview2_com::{
    take_pwstr, AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler, WebMessageReceivedEventHandler,
};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, GetDC, GetDIBits, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Mirror of `webview::capture` for messages: our marker, a `code=` URL param, or an idbroker JSON key
/// (`authorizationCode`/`authCode`/`code`). Used to verify the object-postMessage path end-to-end.
fn capture_msg(s: &str) -> Option<String> {
    if let Some(i) = s.find("webox-code:") { return Some(s[i + 11..].split(['&', ' ', '#', '"']).next().unwrap_or("").to_string()); }
    if let Some(c) = capture_url(s) { return Some(c); }
    for key in ["authorizationCode", "authCode", "\"code\""] {
        if let Some(idx) = s.find(key) {
            let rest = s[idx + key.len()..].trim_start_matches(['"', '\'', ':', '=', ' ']);
            let tok: String = rest.chars().take_while(|&c| matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '_' | '~' | '-')).collect();
            if tok.len() >= 12 { return Some(tok); }
        }
    }
    None
}

/// Mirror of `webview::capture`'s URL path: `code=<token>` at a word boundary, ≥20 chars. Used to
/// verify that NavigationStarting captures the code from a (true-mode) redirect URL.
fn capture_url(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut from = 0;
    while let Some(rel) = s[from..].find("code=") {
        let pos = from + rel;
        let prev_ident = pos > 0 && matches!(b[pos - 1], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_');
        if !prev_ident {
            let tok: String = s[pos + 5..].chars().take_while(|&c| matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '_' | '~' | '-')).collect();
            if tok.len() >= 20 { return Some(tok); }
        }
        from = pos + 5;
    }
    None
}

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }

extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if m == WM_DESTROY { PostQuitMessage(0); return LRESULT(0); }
        DefWindowProcW(h, m, w, l)
    }
}

/// Dispatch pending messages so WebView2's COM callbacks get delivered on this thread.
fn pump_briefly() {
    unsafe {
        for _ in 0..200 {
            let mut msg = MSG::default();
            if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

pub fn main() -> Result<(), String> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "about:blank".into());

    unsafe {
        let hmod = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let hinstance: HINSTANCE = hmod.into();
        let cls = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hinstance, lpszClassName: w!("WeboxProbe"), ..Default::default() };
        RegisterClassW(&cls);
        let hwnd = CreateWindowExW(
            Default::default(), w!("WeboxProbe"), w!("WebOx WebView2 probe"),
            WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, 920, 700,
            None, None, hinstance, None,
        ).map_err(|e| e.to_string())?;
        let _ = ShowWindow(hwnd, SW_SHOW);

        // Environment with its own profile folder, like src/platform/webview/.
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", std::env::temp_dir().join("webox_probe_profile"));
        let env: ICoreWebView2Environment = {
            let (tx, rx) = channel();
            CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
                Box::new(|h| { CreateCoreWebView2Environment(&h).map_err(webview2_com::Error::WindowsError) }),
                Box::new(move |c, e| { c?; let _ = tx.send(e); Ok(()) }),
            ).map_err(|e| format!("env: {e}"))?;
            rx.recv().ok().flatten().ok_or("no environment")?
        };
        println!("[1/4] WebView2 environment created (runtime present).");

        // Controller parented to our own HWND.
        let controller: ICoreWebView2Controller = {
            let (tx, rx) = channel();
            let env = env.clone();
            CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
                Box::new(move |h| { env.CreateCoreWebView2Controller(hwnd, &h).map_err(webview2_com::Error::WindowsError) }),
                Box::new(move |c, ctl| { c?; let _ = tx.send(ctl); Ok(()) }),
            ).map_err(|e| format!("controller: {e}"))?;
            rx.recv().ok().flatten().ok_or("no controller")?
        };
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        controller.SetBounds(rc).map_err(|e| e.to_string())?;
        controller.SetIsVisible(true).map_err(|e| e.to_string())?;
        println!("[2/4] Controller created as a child of HWND {:?} and sized to the window.", hwnd.0);

        let webview = controller.CoreWebView2().map_err(|e| e.to_string())?;

        // Inject a synthetic code post. `jsontest` posts an idbroker-shaped OBJECT (not a string), so we
        // verify WebMessageAsJson carries it and capture() reads the `authorizationCode` key; otherwise a
        // plain string post.
        let inject = if url == "jsontest" {
            r#"try{window.chrome.webview.postMessage({messageType:'idbroker.authorize.response',authorizationCode:'TESTCODE0123456789abcdef',AuthResult:0});}catch(e){window.chrome.webview.postMessage('webox-log:inject-threw:'+e);}"#
        } else {
            r#"try{window.chrome.webview.postMessage('webox-code:TEST-ABCDEF0123456789abcdef');}catch(e){}"#
        };
        let js = wide(inject);
        let _ = AddScriptToExecuteOnDocumentCreatedCompletedHandler::wait_for_async_operation(
            Box::new(move |h| { webview.AddScriptToExecuteOnDocumentCreated(PCWSTR(js.as_ptr()), &h).map_err(webview2_com::Error::WindowsError) }),
            Box::new(|c, _| { c?; Ok(()) }),
        );
        let webview = controller.CoreWebView2().map_err(|e| e.to_string())?;

        // Capture posted messages exactly like the app does: log BOTH string and JSON forms.
        let (msg_tx, msg_rx) = channel::<String>();
        let mut tok = Default::default();
        webview.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_wv, args: Option<ICoreWebView2WebMessageReceivedEventArgs>| {
                if let Some(a) = args {
                    let mut sp = PWSTR::null();
                    let as_str = if a.TryGetWebMessageAsString(&mut sp).is_ok() { take_pwstr(sp) } else { String::new() };
                    let mut jp = PWSTR::null();
                    let as_json = if a.WebMessageAsJson(&mut jp).is_ok() { take_pwstr(jp) } else { String::new() };
                    println!("    WEBMSG str={as_str:?} json={as_json:?}");
                    let _ = msg_tx.send(if !as_str.is_empty() { as_str } else { as_json });
                }
                Ok(())
            })),
            &mut tok,
        ).map_err(|e| e.to_string())?;

        // Watch navigation completion.
        let (nav_tx, nav_rx) = channel();
        let mut ntok = Default::default();
        webview.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_wv, _args| { let _ = nav_tx.send(()); Ok(()) })),
            &mut ntok,
        ).map_err(|e| e.to_string())?;

        // Capture the code from a navigation URL exactly like the app's NavigationStarting hook does.
        let (ns_tx, ns_rx) = channel::<String>();
        let mut nstok = Default::default();
        webview.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_wv, args: Option<ICoreWebView2NavigationStartingEventArgs>| {
                if let Some(a) = args {
                    let mut p = PWSTR::null();
                    if a.Uri(&mut p).is_ok() { if let Some(c) = capture_url(&take_pwstr(p)) { let _ = ns_tx.send(c); } }
                }
                Ok(())
            })),
            &mut nstok,
        ).map_err(|e| e.to_string())?;

        // `navtest`: stand in for idbroker's true-mode redirect — a page that navigates to a
        // `code=`-carrying callback URL, which NavigationStarting must catch (even though the host
        // won't resolve). Otherwise navigate to the given URL (real authorize URL / about:blank).
        let nav_url = match url.as_str() {
            "navtest" => "data:text/html,<script>setTimeout(function(){location.href='https://example.invalid/cb?code=NAVTEST-0123456789abcdefABCDEF0'},80)</script>".to_string(),
            "jsontest" => "about:blank".to_string(),
            _ => url.clone(),
        };
        let u = wide(&nav_url);
        webview.Navigate(PCWSTR(u.as_ptr())).map_err(|e| e.to_string())?;
        println!("[3/4] Navigating to: {}", match url.as_str() { "navtest" => "navtest (JS redirect to a code= callback)", "jsontest" => "jsontest (object postMessage via WebMessageAsJson)", _ => &url });

        let (mut got_code, mut navigated) = (false, false);
        for _ in 0..60 {
            pump_briefly();
            if !got_code { if let Ok(m) = msg_rx.try_recv() {
                match capture_msg(&m) {
                    Some(c) => println!("    IPC OK  postMessage -> WebMessageReceived -> capture() = {c:?}"),
                    None => println!("    IPC message (not a code): {m:?}"),
                }
                got_code = true;
            }}
            if let Ok(c) = ns_rx.try_recv() { println!("    NAV CAPTURE OK  NavigationStarting URL -> capture() = {c:?}"); }
            if !navigated && nav_rx.try_recv().is_ok() { navigated = true; println!("    NavigationCompleted fired (page rendered in-window)."); }
            if got_code && navigated { break; }
        }
        let mut title = PWSTR::null();
        if webview.DocumentTitle(&mut title).is_ok() { println!("    DocumentTitle: {:?}", take_pwstr(title)); }

        // Screenshot the whole window (DWM-composited, includes the WebView2 child).
        let shot = std::env::temp_dir().join("webox_probe_shot.bmp");
        match snapshot(hwnd) {
            Ok(bmp) => { let _ = std::fs::write(&shot, bmp); println!("[4/4] Screenshot saved: {}", shot.display()); }
            Err(e) => println!("[4/4] (screenshot skipped: {e})"),
        }

        let _ = controller.Close();
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

/// PrintWindow the whole window (PW_RENDERFULLCONTENT grabs the composited webview child), returned
/// as a 24-bit BMP byte vector.
unsafe fn snapshot(hwnd: HWND) -> Result<Vec<u8>, String> {
    let mut rc = RECT::default();
    GetWindowRect(hwnd, &mut rc).map_err(|e| e.to_string())?;
    let (w, h) = ((rc.right - rc.left).max(1), (rc.bottom - rc.top).max(1));
    let screen = GetDC(hwnd);
    let mem = CreateCompatibleDC(screen);
    let bmp = CreateCompatibleBitmap(screen, w, h);
    let old = SelectObject(mem, bmp);
    if !PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool() {
        let _ = BitBlt(mem, 0, 0, w, h, screen, 0, 0, SRCCOPY);
    }
    // Bottom-up 24-bit DIB (positive biHeight), matching a positive-height BMP.
    let mut bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: h,
        biPlanes: 1, biBitCount: 24, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
    let row = ((w * 3 + 3) & !3) as usize;
    let mut px = vec![0u8; row * h as usize];
    let got = GetDIBits(mem, bmp, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
    SelectObject(mem, old);
    let _ = DeleteDC(mem);
    ReleaseDC(hwnd, screen);
    if got == 0 { return Err("GetDIBits failed".into()); }
    let data_len = px.len() as u32;
    let mut out = Vec::with_capacity(54 + px.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + data_len).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&px);
    Ok(out)
}
}

#[cfg(windows)]
fn main() -> Result<(), String> { win::main() }

/// Other platforms: nothing to probe (Linux uses WebKitGTK, see src/platform/webview/).
#[cfg(not(windows))]
fn main() {}
