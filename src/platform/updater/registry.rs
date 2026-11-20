//! The installer's entry under "Apps" (HKCU uninstall key, packaging/windows/webox.iss): after a self-update its
//! `DisplayVersion` is set to the running version, when the entry belongs to this program's folder.
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::System::Registry::{RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

/// `AppId` of webox.iss plus Inno Setup's `_is1` suffix.
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{6F3A2D1C-8B4E-4C7A-9E15-2B7D0C4F8A61}_is1";

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain([0]).collect() }

fn get(name: &str) -> Option<String> {
    let (k, n) = (wide(UNINSTALL_KEY), wide(name));
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    if r.is_err() { return None; }
    buf.truncate((len as usize / 2).saturating_sub(1));
    Some(String::from_utf16_lossy(&buf))
}

/// True when the entry's install folder is the folder of `exe`.
pub fn same_folder(install_location: &str, exe: &Path) -> bool {
    let a = install_location.trim().trim_end_matches(['\\', '/']).to_lowercase();
    exe.parent().is_some_and(|d| d.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase() == a)
}

pub fn sync_display_version(exe: &Path, version: &str) {
    let Some(loc) = get("InstallLocation") else { return };
    if !same_folder(&loc, exe) || get("DisplayVersion").as_deref() == Some(version) { return; }
    let (k, n, v) = (wide(UNINSTALL_KEY), wide("DisplayVersion"), wide(version));
    let _ = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), REG_SZ.0, Some(v.as_ptr() as *const _), (v.len() * 2) as u32) };
}
