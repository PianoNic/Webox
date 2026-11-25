//! Linux cameras through V4L2: `/dev/video*` nodes that capture and stream, formats / sizes / rates from the ENUM ioctls,
//! frames by mmap streaming (4 buffers). Raw ioctls with the byte layouts of `v4l2_parse.rs` (64-bit only), no libv4l.
//!
//! Not covered: the PipeWire camera portal (sandboxed Flatpak builds see no `/dev/video*`); consuming its stream needs a
//! PipeWire client library, which this build does not link.
use super::v4l2_parse as p;
use super::{pick_mode, CamDev, Fail, Mode, Raw, Source};
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

fn ioctl<const N: usize>(fd: i32, req: u64, buf: &mut [u8; N]) -> Result<(), i32> {
    loop {
        // SAFETY: `buf` is a correctly sized byte image of the structure `req` encodes (sizes are part of the request code).
        let r = unsafe { libc::ioctl(fd, req as libc::Ioctl, buf.as_mut_ptr()) };
        if r != -1 { return Ok(()); }
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if e != libc::EINTR { return Err(e); }
    }
}

fn open_node(path: &str) -> Result<OwnedFd, i32> {
    let c = CString::new(path).map_err(|_| libc::EINVAL)?;
    // SAFETY: plain open(2) of a NUL-terminated path.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 { return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0)); }
    // SAFETY: `fd` was just returned by open and is owned by nobody else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn cap(fd: i32) -> Option<p::Cap> { let mut b = [0u8; 104]; ioctl(fd, p::QUERYCAP, &mut b).ok()?; Some(p::parse_cap(&b)) }

/// Every (format, size, rate) the node offers.
fn enum_modes(fd: i32) -> Vec<Mode> {
    let mut found = vec![];
    for fi in 0..64 {
        let mut d = p::fmtdesc(fi);
        if ioctl(fd, p::ENUM_FMT, &mut d).is_err() { break; }
        let fcc = p::fmtdesc_fourcc(&d);
        if p::pix_of(fcc).is_none() { continue; }
        for si in 0..64 {
            let mut s = p::frmsize(si, fcc);
            if ioctl(fd, p::ENUM_FRAMESIZES, &mut s).is_err() { break; }
            let sizes = p::frmsize_sizes(&s);
            let stepwise = p::u32_at(&s, 8) != p::FRMSIZE_TYPE_DISCRETE;
            for (w, h) in sizes {
                let mut best: Option<f32> = None;
                for ii in 0..32 {
                    let mut iv = p::frmival(ii, fcc, w, h);
                    if ioctl(fd, p::ENUM_FRAMEINTERVALS, &mut iv).is_err() { break; }
                    if let Some(f) = p::frmival_fps(&iv) { best = Some(best.map_or(f, |b: f32| b.max(f))); }
                    if p::u32_at(&iv, 16) != p::FRMIVAL_TYPE_DISCRETE { break; }
                }
                found.push((fcc, w, h, best.unwrap_or(30.0)));
            }
            if stepwise { break; }
        }
    }
    p::modes_of(&found)
}

pub fn list() -> Vec<CamDev> {
    let mut v = vec![];
    let Ok(rd) = std::fs::read_dir("/dev") else { return v };
    let mut nodes: Vec<(u32, String)> = rd.filter_map(|e| e.ok()).filter_map(|e| {
        let n = e.file_name().to_string_lossy().to_string();
        n.strip_prefix("video").and_then(|k| k.parse::<u32>().ok()).map(|k| (k, format!("/dev/{n}")))
    }).collect();
    nodes.sort();
    let mut seen_bus = std::collections::HashSet::new();
    for (k, path) in nodes {
        let fd = match open_node(&path) {
            Ok(fd) => fd,
            // No access (not in the `video` group, no uaccess ACL): listed anyway, so the picker says why instead of showing
            // no camera; picking it fails with `Fail::Denied`. Only a device's first node (`index` 0; the others are
            // metadata nodes), named from sysfs (readable without access to the node).
            Err(libc::EACCES | libc::EPERM) => {
                let sys = |f: &str| std::fs::read_to_string(format!("/sys/class/video4linux/video{k}/{f}")).map(|s| s.trim().to_string()).unwrap_or_default();
                if sys("index").parse::<u32>().unwrap_or(0) == 0 {
                    let name = sys("name");
                    v.push(CamDev { name: if name.is_empty() { denied_name(&path) } else { format!("{name} ({})", denied_name(&path)) }, id: path });
                }
                continue;
            }
            Err(_) => continue,
        };
        let Some(c) = cap(fd.as_raw_fd()) else { continue };
        if !c.is_camera() || enum_modes(fd.as_raw_fd()).is_empty() { continue; }
        if !c.bus.is_empty() && !seen_bus.insert(c.bus.clone()) { continue; } // one entry per physical camera
        v.push(CamDev { name: if c.card.is_empty() { path.clone() } else { c.card }, id: path });
    }
    v
}

/// The device list's note for a node Webox may not open.
fn denied_name(path: &str) -> String { crate::trq!("No access to {{0}}", path) }

struct Map { ptr:*mut libc::c_void, len: usize }
impl Drop for Map {
    // SAFETY: unmaps exactly what mmap returned.
    fn drop(&mut self) { unsafe { libc::munmap(self.ptr, self.len); } }
}

pub struct V4l2Source { fd: OwnedFd, maps: Vec<Map>, mode: Mode, modes: Vec<Mode>, name: String, w: u32, h: u32, stride: u32, on: bool }

fn fail(e: i32, name: &str) -> Fail {
    match e {
        libc::EBUSY => Fail::InUse(name.into()),
        libc::EACCES | libc::EPERM => Fail::Denied,
        libc::ENODEV | libc::ENOENT | libc::ENXIO => Fail::Unplugged(name.into()),
        _ => Fail::Other(format!("{name}: {}", std::io::Error::from_raw_os_error(e))),
    }
}

pub fn open(dev: &CamDev, max_h: u32) -> Result<V4l2Source, Fail> {
    // the saved camera by path, else by name (the node number can change between boots)
    let path = if std::path::Path::new(&dev.id).exists() { dev.id.clone() }
        else { list().into_iter().find(|d| d.name == dev.name).map(|d| d.id).ok_or_else(|| Fail::Unplugged(dev.name.clone()))? };
    let fd = open_node(&path).map_err(|e| fail(e, &dev.name))?;
    let raw = fd.as_raw_fd();
    let modes = enum_modes(raw);
    let mode = pick_mode(&modes, max_h).ok_or_else(|| Fail::Other(format!("{}: {}", dev.name, crate::trq!("no supported video format"))))?;
    let mut f = p::format(p::fourcc_of(mode.pix), mode.w, mode.h);
    if mode.pix == super::Pix::Bgra { p::put_u32(&mut f, 16, p::fourcc(b"BGR4")); } // whichever 32-bit fourcc it listed
    ioctl(raw, p::S_FMT, &mut f).map_err(|e| fail(e, &dev.name))?;
    let (_, w, h, bpl, _) = p::format_result(&f);
    let mut sp = p::streamparm(mode.fps.round() as u32);
    let _ = ioctl(raw, p::S_PARM, &mut sp); // best effort: not every driver lets the rate be set
    let mut rb = p::reqbufs(4);
    ioctl(raw, p::REQBUFS, &mut rb).map_err(|e| fail(e, &dev.name))?;
    let count = p::u32_at(&rb, 0).min(8);
    if count == 0 { return Err(Fail::Other(format!("{}: {}", dev.name, crate::trq!("no buffers")))); }
    let mut maps = vec![];
    for i in 0..count {
        let mut b = p::buffer(i);
        ioctl(raw, p::QUERYBUF, &mut b).map_err(|e| fail(e, &dev.name))?;
        let (_, _, off, len) = p::buffer_fields(&b);
        // SAFETY: maps the driver buffer described by QUERYBUF; unmapped in `Map::drop`.
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, raw, off as libc::off_t) };
        if ptr == libc::MAP_FAILED { return Err(fail(std::io::Error::last_os_error().raw_os_error().unwrap_or(0), &dev.name)); }
        maps.push(Map { ptr, len: len as usize });
        ioctl(raw, p::QBUF, &mut b).map_err(|e| fail(e, &dev.name))?;
    }
    let mut t = [0u8; 4];
    p::put_u32(&mut t, 0, p::BUF_TYPE_VIDEO_CAPTURE);
    ioctl(raw, p::STREAMON, &mut t).map_err(|e| fail(e, &dev.name))?;
    Ok(V4l2Source { fd, maps, mode: super::Mode { w, h, ..mode }, modes, name: dev.name.clone(), w, h, stride: bpl, on: true })
}

impl Drop for V4l2Source {
    fn drop(&mut self) {
        if self.on { let mut t = [0u8; 4]; p::put_u32(&mut t, 0, p::BUF_TYPE_VIDEO_CAPTURE); let _ = ioctl(self.fd.as_raw_fd(), p::STREAMOFF, &mut t); }
        self.maps.clear(); // unmap before the fd closes
    }
}

impl Source for V4l2Source {
    fn mode(&self) -> Mode { self.mode }
    fn modes(&self) -> Vec<Mode> { self.modes.clone() }
    fn name(&self) -> String { self.name.clone() }
    fn next(&mut self) -> Result<Option<Raw>, Fail> {
        let fd = self.fd.as_raw_fd();
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd, 200 ms timeout.
        let r = unsafe { libc::poll(&mut pfd, 1, 200) };
        if r == 0 { return Ok(None); }
        if r < 0 { let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0); return if e == libc::EINTR { Ok(None) } else { Err(fail(e, &self.name)) }; }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP) != 0 { return Err(Fail::Unplugged(self.name.clone())); }
        let mut b = p::buffer(0);
        match ioctl(fd, p::DQBUF, &mut b) { Ok(()) => {}, Err(libc::EAGAIN) => return Ok(None), Err(e) => return Err(fail(e, &self.name)) }
        let (i, used, _, _) = p::buffer_fields(&b);
        let raw = self.maps.get(i as usize).map(|m| {
            // SAFETY: the dequeued buffer belongs to us until it is queued again below.
            let s = unsafe { std::slice::from_raw_parts(m.ptr as *const u8, (used as usize).min(m.len)) };
            let (w, h) = (self.w as usize, self.h as usize);
            let stride = (self.stride as usize).max(match self.mode.pix { super::Pix::Yuyv => w * 2, super::Pix::Bgra => w * 4, super::Pix::Rgb24 => w * 3, _ => w });
            let uv_off = stride * h;
            Raw { pix: self.mode.pix, w: self.w, h: self.h, stride, uv_off, data: s.to_vec() }
        });
        ioctl(fd, p::QBUF, &mut b).map_err(|e| fail(e, &self.name))?;
        Ok(raw)
    }
}
