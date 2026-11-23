//! The selected UI language: global state, system locale, switching and persisting.
use super::*;

// Selected language (global state).

pub(super) struct State {
    pub(super) setting: String,
    pub(super) code: &'static str,
    pub(super) catalog: Option<&'static Catalog>,
}

pub(super) static GENERATION: AtomicU64 = AtomicU64::new(1);

#[cfg(not(test))]
pub(super) fn state() -> &'static RwLock<State> {
    static S: OnceLock<RwLock<State>> = OnceLock::new();
    S.get_or_init(|| {
        let code = resolve_code(&system_locale());
        RwLock::new(State { setting: "system".into(), code, catalog: catalog_for(code) })
    })
}

/// Test builds: one language state per test thread, so a test that switches the language (i18n tests) can't
/// change the strings another test running in parallel sees.
#[cfg(test)]
pub(super) fn state() -> &'static RwLock<State> {
    thread_local! {
        static S: &'static RwLock<State> = Box::leak(Box::new({
            let code = resolve_code(&system_locale());
            RwLock::new(State { setting: "system".into(), code, catalog: catalog_for(code) })
        }));
    }
    S.with(|s| *s)
}

/// OS UI language tag (`de-CH`). Env `WEBOX_LANG` overrides. Falls back to `en-US`.
pub fn system_locale() -> String {
    if let Ok(l) = std::env::var("WEBOX_LANG") {
        if !l.is_empty() {
            return l;
        }
    }
    #[cfg(windows)]
    {
        if let Some(l) = win_ui_language() {
            return l;
        }
    }
    #[cfg(not(windows))]
    {
        for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(l) = std::env::var(k) {
                if !l.is_empty() && l != "C" && l != "POSIX" {
                    return l;
                }
            }
        }
    }
    "en-US".into()
}

#[cfg(windows)]
pub(super) fn win_ui_language() -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Globalization::{GetUserDefaultLocaleName, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
    unsafe {
        let mut num = 0u32;
        let mut len = 0u32;
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut num, PWSTR::null(), &mut len).is_ok() && len > 1 {
            let mut buf = vec![0u16; len as usize];
            if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut num, PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
                // Double-NUL separated list; first entry is the preferred language.
                let first: Vec<u16> = buf.iter().copied().take_while(|&c| c != 0).collect();
                if !first.is_empty() {
                    return Some(String::from_utf16_lossy(&first));
                }
            }
        }
        let mut buf = [0u16; 85];
        let n = GetUserDefaultLocaleName(&mut buf);
        if n > 1 {
            return Some(String::from_utf16_lossy(&buf[..(n as usize - 1)]));
        }
    }
    None
}

/// Select the UI language: `"system"` or a pack code / locale tag (`de`, `en_GB`, `fr-CA`). Switching is
/// immediate (no restart); egui callers should use [`set_language_ctx`] / [`sync_from_settings`].
pub fn set_language(setting: &str) {
    let setting = if setting.is_empty() { "system" } else { setting };
    let code = if setting == "system" { resolve_code(&system_locale()) } else { resolve_code(setting) };
    let mut s = state().write().unwrap();
    if s.setting == setting && s.code == code {
        return;
    }
    s.setting = setting.to_string();
    s.code = code;
    s.catalog = catalog_for(code);
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// The pack code actually in effect (`en` when English / no pack).
pub fn active_code() -> &'static str {
    state().read().unwrap().code
}
/// True while the source language (English) is active, i.e. `tr` is the identity.
pub fn is_source_language() -> bool {
    state().read().unwrap().catalog.is_none()
}
/// Incremented whenever the language changes; cache derived strings against it.
pub fn generation() -> u64 {
    let _ = state();
    GENERATION.load(Ordering::Relaxed)
}

/// [`set_language`] + egui repaint.
pub fn set_language_ctx(ctx: &eframe::egui::Context, setting: &str) {
    let g = generation();
    set_language(setting);
    if generation() != g {
        ctx.request_repaint();
    }
}

/// Call once per frame: applies settings key `language` (live switching, repaint on change).
pub fn sync_from_settings(ctx: &eframe::egui::Context, settings: &serde_json::Value) {
    let want = settings.get("language").and_then(|v| v.as_str()).unwrap_or("system");
    let cur = state().read().unwrap().setting.clone();
    if want != cur && !(want.is_empty() && cur == "system") {
        set_language_ctx(ctx, want);
    }
}
