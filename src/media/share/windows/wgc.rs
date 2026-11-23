//! Windows.Graphics.Capture: a capture item for a monitor (HMONITOR) or a window (HWND) through IGraphicsCaptureItemInterop,
//! a free-threaded frame pool on our own D3D11 device, polled from the share thread (no dispatcher queue needed). Each new
//! frame is copied into a CPU-readable staging texture and read back as BGRA. Works for any user without elevation; the
//! system draws its capture border around the shared screen / window.
use super::{Capture, Grab};
use std::time::Duration;
use windows::core::Interface;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

fn err(e: windows::core::Error) -> String { format!("screen capture: {} (0x{:08X})", e.message(), e.code().0) }

pub(super) struct Wgc {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    winrt_dev: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    item: GraphicsCaptureItem,
    pool_size: SizeInt32,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    closed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    closed_token: Option<windows::Foundation::EventRegistrationToken>,
}

/// A hardware D3D11 device (the WARP software rasteriser where there is no GPU, e.g. a VM without one).
pub(super) unsafe fn d3d_device() -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
    let mut last = String::new();
    for driver in [D3D_DRIVER_TYPE_HARDWARE, windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP] {
        let (mut dev, mut ctx) = (None, None);
        match D3D11CreateDevice(None, driver, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut dev), None, Some(&mut ctx)) {
            Ok(()) => if let (Some(d), Some(c)) = (dev, ctx) { return Ok((d, c)); },
            Err(e) => last = err(e),
        }
    }
    Err(last)
}

/// WGC exists (Windows 10 1803 and later) and is allowed here.
pub(super) fn supported() -> bool { GraphicsCaptureSession::IsSupported().unwrap_or(false) }

impl Wgc {
    pub fn monitor(h: isize, cursor: bool) -> Result<Wgc, String> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(err)?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(HMONITOR(h as _)) }.map_err(err)?;
        Self::open(item, cursor)
    }

    pub fn window(h: isize, cursor: bool) -> Result<Wgc, String> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(err)?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(HWND(h as _)) }.map_err(err)?;
        Self::open(item, cursor)
    }

    fn open(item: GraphicsCaptureItem, cursor: bool) -> Result<Wgc, String> {
        unsafe {
            let (dev, ctx) = d3d_device()?;
            let dxgi: IDXGIDevice = dev.cast().map_err(err)?;
            let winrt_dev: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi).map_err(err)?.cast().map_err(err)?;
            let size = item.Size().map_err(err)?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt_dev, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size).map_err(err)?;
            let session = pool.CreateCaptureSession(&item).map_err(err)?;
            let _ = session.SetIsCursorCaptureEnabled(cursor);
            let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let c2 = closed.clone();
            // the window was closed / the monitor unplugged
            let closed_token = item.Closed(&windows::Foundation::TypedEventHandler::new(move |_, _| { c2.store(true, std::sync::atomic::Ordering::SeqCst); Ok(()) })).ok();
            session.StartCapture().map_err(err)?;
            Ok(Wgc { dev, ctx, winrt_dev, pool, session, item, pool_size: size, staging: None, closed, closed_token })
        }
    }

    fn read(&mut self, frame: &windows::Graphics::Capture::Direct3D11CaptureFrame, buf: &mut Vec<u8>) -> Result<Grab, String> {
        unsafe {
            let content = frame.ContentSize().map_err(err)?;
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface().map_err(err)?.cast().map_err(err)?;
            let tex: ID3D11Texture2D = access.GetInterface().map_err(err)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            tex.GetDesc(&mut desc);
            let w = (content.Width.max(0) as u32).min(desc.Width);
            let h = (content.Height.max(0) as u32).min(desc.Height);
            if w < 2 || h < 2 { return Ok(Grab::None); }
            if !self.staging.as_ref().is_some_and(|s| s.1 == desc.Width && s.2 == desc.Height) {
                let sd = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width, Height: desc.Height, MipLevels: 1, ArraySize: 1, Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, Usage: D3D11_USAGE_STAGING, BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, MiscFlags: 0,
                };
                let mut t = None;
                self.dev.CreateTexture2D(&sd, None, Some(&mut t)).map_err(err)?;
                self.staging = Some((t.ok_or("staging texture")?, desc.Width, desc.Height));
            }
            let staging = &self.staging.as_ref().ok_or("staging texture")?.0;
            self.ctx.CopyResource(staging, &tex);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut m)).map_err(err)?;
            let row = w as usize * 4;
            buf.resize(row * h as usize, 0);
            for y in 0..h as usize {
                let s = std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), row);
                buf[y * row..(y + 1) * row].copy_from_slice(s);
            }
            self.ctx.Unmap(staging, 0);
            // the source changed size (window resized): the pool follows so later frames are not cut off
            if content.Width != self.pool_size.Width || content.Height != self.pool_size.Height {
                self.pool_size = content;
                let _ = self.pool.Recreate(&self.winrt_dev, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, content);
            }
            Ok(Grab::Frame { w, h, bgra: true })
        }
    }
}

impl Capture for Wgc {
    fn grab(&mut self, buf: &mut Vec<u8>, _wait: Duration) -> Result<Grab, String> {
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) { return Ok(Grab::Closed); }
        // only the newest frame counts
        let mut last = None;
        while let Ok(f) = self.pool.TryGetNextFrame() { last = Some(f); }
        match last { Some(f) => { let r = self.read(&f, buf); let _ = f.Close(); r } None => Ok(Grab::None) }
    }
}

impl Drop for Wgc {
    fn drop(&mut self) {
        if let Some(t) = self.closed_token.take() { let _ = self.item.RemoveClosed(t); }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
