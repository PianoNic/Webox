//! WebKitGTK (webkit2gtk-4.1) in a top-level window of its own: eframe runs on winit, not GTK, so a GTK widget can't live
//! inside the app's window. GTK may only ever be initialised on one thread, so a single long-lived GTK thread runs the GTK
//! main loop for the whole process and every sign-in window is created on it (`MainContext::invoke`) - a second sign-in
//! after Back / a failed attempt works too. The profile (cookies, local storage) lives in our own folder (`EBWebView` in
//! the config dir), like the WebView2 profile on Windows, so idbroker can remember the session.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};

pub struct WebView {
    rx: Receiver<super::Captured>,
    close: Arc<AtomicBool>,
}

/// We can't embed GTK inside the winit window, so this is a separate window: no in-app bounds.
pub const EMBEDDED: bool = false;

fn log(msg: String) { crate::cloud::auth::dbg_auth(format!("WEBKITGTK {msg}")); }

/// Starts the GTK thread once; false when GTK can't initialise (no display, libraries missing).
fn gtk_thread() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("webox-gtk".into()).spawn(move || {
            gtk::glib::set_prgname(Some("webox")); // window class / app id = the desktop file, so the panel groups it with the app
            gtk::glib::set_application_name("Webox");
            if gtk::init().is_err() { let _ = tx.send(false); return; }
            let _ = tx.send(true);
            gtk::main();
        });
        spawned.is_ok() && rx.recv().unwrap_or(false)
    })
}

fn open(url: String, profile: std::path::PathBuf, tx: Sender<super::Captured>, close: Arc<AtomicBool>) {
    use gtk::prelude::*;
    use webkit2gtk::{CookieManagerExt, SettingsExt, UserContentManagerExt, WebContextExt, WebViewExt, WebsiteDataManagerExt};
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Webex");
    window.set_default_size(820, 640);
    window.set_position(gtk::WindowPosition::Center);
    if let Ok(img) = image::load_from_memory(crate::icons::APP_ICON_256) {
        let img = img.to_rgba8();
        let (w, h) = (img.width() as i32, img.height() as i32);
        let pb = gtk::gdk_pixbuf::Pixbuf::from_bytes(&gtk::glib::Bytes::from_owned(img.into_raw()), gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, w, h, w * 4);
        window.set_icon(Some(&pb));
    }

    let _ = std::fs::create_dir_all(&profile);
    let dm = webkit2gtk::WebsiteDataManager::builder()
        .base_data_directory(profile.join("data").to_string_lossy().to_string())
        .base_cache_directory(profile.join("cache").to_string_lossy().to_string())
        .build();
    if let Some(cm) = dm.cookie_manager() {
        cm.set_persistent_storage(&profile.join("cookies.sqlite").to_string_lossy(), webkit2gtk::CookiePersistentStorage::Sqlite);
    }
    let context = webkit2gtk::WebContext::with_website_data_manager(&dm);
    context.set_cache_model(webkit2gtk::CacheModel::DocumentViewer);

    // The SCRAPER (shared with WebView2) posts to window.webkit.messageHandlers.webox: the code and diagnostics.
    let ucm = webkit2gtk::UserContentManager::new();
    ucm.add_script(&webkit2gtk::UserScript::new(super::SCRAPER, webkit2gtk::UserContentInjectedFrames::AllFrames,
        webkit2gtk::UserScriptInjectionTime::Start, &[], &[]));
    ucm.register_script_message_handler("webox");

    let webview = webkit2gtk::WebView::builder().web_context(&context).user_content_manager(&ucm).build();
    if let Some(st) = WebViewExt::settings(&webview) { st.set_enable_developer_extras(false); st.set_javascript_can_open_windows_automatically(false); }
    window.add(&webview);

    // A message carries no sender, so the page shown decides: a code counts only while idbroker's own page is loaded.
    let (tx_msg, wv_msg, auth_msg) = (tx.clone(), webview.downgrade(), url.clone());
    ucm.connect_script_message_received(Some("webox"), move |_, r| {
        let Some(s) = r.js_value().map(|v| v.to_string()) else { return };
        if let Some(rest) = s.strip_prefix("webox-log:") { log(super::redact(rest).chars().take(2000).collect()); }
        if let Some(c) = super::classify(&s) {
            let page = wv_msg.upgrade().and_then(|w| w.uri()).map(|u| u.to_string()).unwrap_or_default();
            if super::trusted_source(&page, &auth_msg) { let _ = tx_msg.send(c); } else { log(format!("IGNORED code from untrusted page {:?}", super::log_url(&page))); }
        }
    });

    // Every committed URI: the SCRAPER's location.hash fallback and any `code=`-carrying navigation (idbroker's pages only).
    let (tx_uri, auth_uri) = (tx, url.clone());
    webview.connect_uri_notify(move |wv| {
        if let Some(u) = wv.uri() {
            if !super::trusted_source(u.as_str(), &auth_uri) { return; }
            let dec = super::percent_decode(u.as_str());
            if let Some(c) = super::classify(&dec).or_else(|| super::capture(&dec).map(super::Captured::Code)) { let _ = tx_uri.send(c); }
        }
    });
    webview.connect_load_failed(|_, _, uri, err| { log(format!("load failed {}: {err}", super::log_url(uri))); false });
    webview.load_uri(&url);
    window.show_all();

    // The app closes this window (Back, sign-in done) by dropping its handle.
    let w2 = window.clone();
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
        if close.load(Ordering::Relaxed) { unsafe { w2.destroy(); } gtk::glib::ControlFlow::Break } else { gtk::glib::ControlFlow::Continue }
    });
}

impl WebView {
    pub fn start(_parent: Option<isize>, url: &str, user_data_dir: &std::path::Path) -> Result<Self, String> {
        if !gtk_thread() { return Err("WebKitGTK unavailable (GTK could not be initialised)".into()); }
        let (tx, rx) = channel();
        let close = Arc::new(AtomicBool::new(false));
        let (url, profile, c2) = (url.to_string(), user_data_dir.to_path_buf(), close.clone());
        gtk::glib::MainContext::default().invoke(move || open(url, profile, tx, c2));
        Ok(WebView { rx, close })
    }

    pub fn poll(&self) -> Option<super::Captured> { self.rx.try_recv().ok() }
    pub fn set_bounds(&self, _x: i32, _y: i32, _w: i32, _h: i32) {}
}

impl Drop for WebView {
    fn drop(&mut self) { self.close.store(true, Ordering::Relaxed); }
}
