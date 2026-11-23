//! What can be shared on Windows: the monitors (EnumDisplayMonitors, primary first) and the top-level application windows in
//! Z order (visible, titled, not cloaked, not tool windows, not owned by another window, not Webox's own), plus the picker's
//! thumbnails through GDI (BitBlt of the monitor, PrintWindow for windows, which also draws covered windows).
use super::super::{convert, Kind, ShareSource};
use std::ffi::c_void;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::*;

pub(super) fn monitors() -> Vec<ShareSource> {
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, l: LPARAM) -> BOOL {
        (*(l.0 as *mut Vec<isize>)).push(m.0 as isize);
        true.into()
    }
    let mut hs: Vec<isize> = vec![];
    unsafe { let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut hs as *mut _ as isize)); }
    let mut v: Vec<ShareSource> = hs.iter().filter_map(|&h| unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(HMONITOR(h as _), &mut mi).as_bool().then(|| {
            let r = mi.rcMonitor;
            ShareSource { kind: Kind::Screen, handle: h, index: 0, name: String::new(), app: String::new(), size: ((r.right - r.left) as u32, (r.bottom - r.top) as u32),
                primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0, minimized: false }
        })
    }).collect();
    v.sort_by_key(|s| !s.primary);
    for (i, s) in v.iter_mut().enumerate() { s.index = i as u32; s.name = format!("{} {}", crate::trq!("Screen"), i + 1); }
    v
}

pub(super) fn windows() -> Vec<ShareSource> {
    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
        (*(l.0 as *mut Vec<HWND>)).push(h);
        true.into()
    }
    let mut hs: Vec<HWND> = vec![];
    unsafe { let _ = EnumWindows(Some(each), LPARAM(&mut hs as *mut _ as isize)); }
    let me = unsafe { GetCurrentProcessId() };
    hs.into_iter().filter_map(|h| unsafe { window(h, me) }).collect()
}

unsafe fn window(h: HWND, me: u32) -> Option<ShareSource> {
    if !IsWindowVisible(h).as_bool() { return None; }
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    if pid == me { return None; }
    if !GetWindow(h, GW_OWNER).unwrap_or_default().is_invalid() { return None; }
    let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 || ex & WS_EX_NOACTIVATE.0 != 0 { return None; }
    let mut cloaked = 0u32;
    if DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut c_void, 4).is_ok() && cloaked != 0 { return None; }
    let mut t = [0u16; 512];
    let n = GetWindowTextW(h, &mut t);
    if n <= 0 { return None; }
    let name = String::from_utf16_lossy(&t[..n as usize]);
    let mut cls = [0u16; 64];
    let cn = GetClassNameW(h, &mut cls);
    if matches!(String::from_utf16_lossy(&cls[..cn.max(0) as usize]).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") { return None; }
    let mut r = RECT::default();
    let _ = GetWindowRect(h, &mut r);
    let minimized = IsIconic(h).as_bool();
    if !minimized && (r.right - r.left < 40 || r.bottom - r.top < 40) { return None; }
    Some(ShareSource { kind: Kind::Window, handle: h.0 as isize, index: 0, name, app: app_name(pid), size: ((r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32), primary: false, minimized })
}

/// The program's file name without ".exe".
unsafe fn app_name(pid: u32) -> String {
    let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
    let mut buf = [0u16; 520];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
    let _ = windows::Win32::Foundation::CloseHandle(p);
    if !ok { return String::new(); }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The monitor's or window's current picture, scaled into `max`.
pub(super) fn thumbnail(s: &ShareSource, max: (u32, u32)) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let (x, y, w, h, hwnd) = match s.kind {
            Kind::Screen => {
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                if !GetMonitorInfoW(HMONITOR(s.handle as _), &mut mi).as_bool() { return None; }
                let r = mi.rcMonitor;
                (r.left, r.top, r.right - r.left, r.bottom - r.top, None)
            }
            Kind::Window => {
                let hw = HWND(s.handle as _);
                if IsIconic(hw).as_bool() || !IsWindow(hw).as_bool() { return None; }
                let mut r = RECT::default();
                GetWindowRect(hw, &mut r).ok()?;
                (0, 0, r.right - r.left, r.bottom - r.top, Some(hw))
            }
            _ => return None,
        };
        if w <= 0 || h <= 0 { return None; }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp);
        let ok = match hwnd {
            Some(hw) => PrintWindow(hw, mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool(),
            None => BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT).is_ok(),
        };
        let mut bmi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h,
            biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut px = vec![0u8; (w * h * 4) as usize];
        GetDIBits(mem, bmp, 0, h as u32, Some(px.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if !ok { return None; }
        Some(convert::thumb(convert::Picture { px: &px, w: w as u32, h: h as u32, stride: w as usize * 4, bgra: true }, max))
    }
}

/// Brings a minimised window back before it is shared (a minimised window has no picture to capture).
pub(super) fn restore(h: isize) {
    unsafe { if IsIconic(HWND(h as _)).as_bool() { let _ = ShowWindow(HWND(h as _), SW_RESTORE); } }
}
