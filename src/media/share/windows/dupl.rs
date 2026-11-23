//! DXGI Desktop Duplication: the monitor fallback where Windows.Graphics.Capture is missing (older Windows 10, some
//! virtual machines). The desktop image arrives as a GPU texture on the monitor's adapter and is read back like WGC frames.
//! The pointer is not part of the image here; after a mode change or a secure desktop (UAC) the duplication is reopened.
use super::{Capture, Grab};
use std::time::Duration;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Foundation::HMODULE;

fn err(e: windows::core::Error) -> String { format!("desktop duplication: {} (0x{:08X})", e.message(), e.code().0) }

pub(crate) struct Dupl {
    monitor: isize,
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    output: IDXGIOutput1,
    dup: Option<IDXGIOutputDuplication>,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    /// Just (re)opened: the next frame carries the whole desktop even when only the pointer moved.
    fresh: bool,
}

impl Dupl {
    /// Duplicates the monitor `h` (HMONITOR).
    pub fn open(h: isize) -> Result<Dupl, String> {
        unsafe {
            let f: IDXGIFactory1 = CreateDXGIFactory1().map_err(err)?;
            let mut i = 0;
            while let Ok(a) = f.EnumAdapters1(i) {
                i += 1;
                let mut j = 0;
                while let Ok(o) = a.EnumOutputs(j) {
                    j += 1;
                    let Ok(d) = o.GetDesc() else { continue };
                    if d.Monitor.0 as isize != h { continue; }
                    let (mut dev, mut ctx) = (None, None);
                    D3D11CreateDevice(&a, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut dev), None, Some(&mut ctx)).map_err(err)?;
                    let (dev, ctx) = (dev.ok_or("no device")?, ctx.ok_or("no context")?);
                    let output: IDXGIOutput1 = o.cast().map_err(err)?;
                    let mut me = Dupl { monitor: h, dev, ctx, output, dup: None, staging: None, fresh: true };
                    me.reopen()?;
                    return Ok(me);
                }
            }
            Err("desktop duplication: the monitor is not attached to an adapter".into())
        }
    }

    fn reopen(&mut self) -> Result<(), String> {
        self.dup = None;
        self.dup = Some(unsafe { self.output.DuplicateOutput(&self.dev) }.map_err(err)?);
        self.fresh = true;
        Ok(())
    }

    unsafe fn read(&mut self, res: &IDXGIResource, buf: &mut Vec<u8>) -> Result<Grab, String> {
        let tex: ID3D11Texture2D = res.cast().map_err(err)?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        tex.GetDesc(&mut desc);
        let (w, h) = (desc.Width, desc.Height);
        if !self.staging.as_ref().is_some_and(|s| s.1 == w && s.2 == h) {
            let sd = D3D11_TEXTURE2D_DESC {
                Width: w, Height: h, MipLevels: 1, ArraySize: 1, Format: desc.Format, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, MiscFlags: 0,
            };
            let mut t = None;
            self.dev.CreateTexture2D(&sd, None, Some(&mut t)).map_err(err)?;
            self.staging = Some((t.ok_or("staging texture")?, w, h));
        }
        let staging = &self.staging.as_ref().ok_or("staging texture")?.0;
        self.ctx.CopyResource(staging, &tex);
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        self.ctx.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut m)).map_err(err)?;
        let row = w as usize * 4;
        buf.resize(row * h as usize, 0);
        for y in 0..h as usize {
            buf[y * row..(y + 1) * row].copy_from_slice(std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), row));
        }
        self.ctx.Unmap(staging, 0);
        Ok(Grab::Frame { w, h, bgra: true })
    }
}

impl Capture for Dupl {
    fn grab(&mut self, buf: &mut Vec<u8>, wait: Duration) -> Result<Grab, String> {
        unsafe {
            if self.dup.is_none() && self.reopen().is_err() { return Ok(Grab::None); }
            let Some(dup) = self.dup.clone() else { return Ok(Grab::None) };
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res = None;
            match dup.AcquireNextFrame(wait.as_millis() as u32, &mut info, &mut res) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(Grab::None),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => { self.dup = None; return Ok(Grab::None); }
                // the secure desktop (lock screen, UAC) is up: no picture until it is gone
                Err(e) if e.code() == windows::Win32::Foundation::E_ACCESSDENIED => return Ok(Grab::None),
                Err(e) => return Err(err(e)),
            }
            // a frame with only a pointer update has no new picture (except the first one, which holds the whole desktop)
            let r = match (&res, info.LastPresentTime) { (Some(r), t) if t != 0 || self.fresh => self.read(r, buf), _ => Ok(Grab::None) };
            if matches!(r, Ok(Grab::Frame { .. })) { self.fresh = false; }
            let _ = dup.ReleaseFrame();
            if windows::Win32::Graphics::Gdi::GetMonitorInfoW(windows::Win32::Graphics::Gdi::HMONITOR(self.monitor as _), &mut windows::Win32::Graphics::Gdi::MONITORINFO { cbSize: 40, ..Default::default() }).as_bool() { r } else { Ok(Grab::Closed) }
        }
    }
}
