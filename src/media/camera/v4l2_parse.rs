//! The V4L2 structures the Linux back end exchanges with the kernel, read from / written to plain byte buffers (64-bit
//! layouts from `linux/videodev2.h`). Kept free of OS calls so the parsing is tested on every platform.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
use super::{Mode, Pix};

pub const CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
pub const CAP_STREAMING: u32 = 0x0400_0000;
pub const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
pub const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
pub const MEMORY_MMAP: u32 = 1;
pub const FRMSIZE_TYPE_DISCRETE: u32 = 1;
pub const FRMIVAL_TYPE_DISCRETE: u32 = 1;

/// `_IOC(dir, 'V', nr, size)`: dir 1 = write, 2 = read, 3 = both.
pub const fn ioc(dir: u64, nr: u64, size: u64) -> u64 { (dir << 30) | (size << 16) | ((b'V' as u64) << 8) | nr }
pub const QUERYCAP: u64 = ioc(2, 0, 104);
pub const ENUM_FMT: u64 = ioc(3, 2, 64);
pub const S_FMT: u64 = ioc(3, 5, 208);
pub const REQBUFS: u64 = ioc(3, 8, 20);
pub const QUERYBUF: u64 = ioc(3, 9, 88);
pub const QBUF: u64 = ioc(3, 15, 88);
pub const DQBUF: u64 = ioc(3, 17, 88);
pub const STREAMON: u64 = ioc(1, 18, 4);
pub const STREAMOFF: u64 = ioc(1, 19, 4);
pub const S_PARM: u64 = ioc(3, 22, 204);
pub const ENUM_FRAMESIZES: u64 = ioc(3, 74, 44);
pub const ENUM_FRAMEINTERVALS: u64 = ioc(3, 75, 52);

pub fn u32_at(b: &[u8], o: usize) -> u32 { u32::from_ne_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) }
pub fn put_u32(b: &mut [u8], o: usize, v: u32) { b[o..o + 4].copy_from_slice(&v.to_ne_bytes()); }
fn cstr(b: &[u8]) -> String { String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).trim().to_string() }

pub const fn fourcc(s: &[u8; 4]) -> u32 { (s[0] as u32) | (s[1] as u32) << 8 | (s[2] as u32) << 16 | (s[3] as u32) << 24 }
pub fn pix_of(fcc: u32) -> Option<Pix> {
    match fcc {
        x if x == fourcc(b"YUYV") => Some(Pix::Yuyv),
        x if x == fourcc(b"NV12") => Some(Pix::Nv12),
        x if x == fourcc(b"MJPG") => Some(Pix::Mjpg),
        x if x == fourcc(b"RGB3") => Some(Pix::Rgb24),
        x if x == fourcc(b"BGR4") || x == fourcc(b"XR24") || x == fourcc(b"AR24") => Some(Pix::Bgra),
        _ => None,
    }
}
pub fn fourcc_of(p: Pix) -> u32 {
    match p { Pix::Yuyv => fourcc(b"YUYV"), Pix::Nv12 => fourcc(b"NV12"), Pix::Mjpg => fourcc(b"MJPG"), Pix::Rgb24 => fourcc(b"RGB3"), Pix::Bgra => fourcc(b"XR24") }
}

/// `struct v4l2_capability` (104 bytes).
#[derive(Debug, PartialEq)]
pub struct Cap { pub driver: String, pub card: String, pub bus: String, pub caps: u32, pub device_caps: u32 }
pub fn parse_cap(b: &[u8; 104]) -> Cap {
    Cap { driver: cstr(&b[0..16]), card: cstr(&b[16..48]), bus: cstr(&b[48..80]), caps: u32_at(b, 84), device_caps: u32_at(b, 88) }
}
impl Cap {
    /// A node that streams video frames (UVC cameras also expose a metadata node per camera, which is skipped).
    pub fn is_camera(&self) -> bool {
        let c = if self.caps & CAP_DEVICE_CAPS != 0 { self.device_caps } else { self.caps };
        c & CAP_VIDEO_CAPTURE != 0 && c & CAP_STREAMING != 0
    }
}

/// `struct v4l2_fmtdesc` (64 bytes): request for entry `index`; the answer's pixel format is at offset 44.
pub fn fmtdesc(index: u32) -> [u8; 64] { let mut b = [0u8; 64]; put_u32(&mut b, 0, index); put_u32(&mut b, 4, BUF_TYPE_VIDEO_CAPTURE); b }
pub fn fmtdesc_fourcc(b: &[u8; 64]) -> u32 { u32_at(b, 44) }

/// `struct v4l2_frmsizeenum` (44 bytes).
pub fn frmsize(index: u32, fcc: u32) -> [u8; 44] { let mut b = [0u8; 44]; put_u32(&mut b, 0, index); put_u32(&mut b, 4, fcc); b }
/// Discrete sizes as (w, h); a stepwise / continuous range offers its common 16:9 / 4:3 sizes inside the range.
pub fn frmsize_sizes(b: &[u8; 44]) -> Vec<(u32, u32)> {
    if u32_at(b, 8) == FRMSIZE_TYPE_DISCRETE { return vec![(u32_at(b, 12), u32_at(b, 16))]; }
    let (min_w, max_w, min_h, max_h) = (u32_at(b, 12), u32_at(b, 16), u32_at(b, 24), u32_at(b, 28));
    [(1920, 1080), (1280, 720), (960, 540), (640, 480), (640, 360)].into_iter().filter(|&(w, h)| w >= min_w && w <= max_w && h >= min_h && h <= max_h).collect()
}
/// `struct v4l2_frmivalenum` (52 bytes).
pub fn frmival(index: u32, fcc: u32, w: u32, h: u32) -> [u8; 52] {
    let mut b = [0u8; 52];
    put_u32(&mut b, 0, index); put_u32(&mut b, 4, fcc); put_u32(&mut b, 8, w); put_u32(&mut b, 12, h);
    b
}
/// Frames per second of a frame-interval answer (for a range: its fastest end).
pub fn frmival_fps(b: &[u8; 52]) -> Option<f32> {
    let (num, den) = (u32_at(b, 20), u32_at(b, 24)); // discrete: the interval; stepwise: its minimum
    (num > 0).then(|| den as f32 / num as f32)
}

/// `struct v4l2_format` (208 bytes, the union at 8): request a pixel format and size.
pub fn format(fcc: u32, w: u32, h: u32) -> [u8; 208] {
    let mut b = [0u8; 208];
    put_u32(&mut b, 0, BUF_TYPE_VIDEO_CAPTURE);
    put_u32(&mut b, 8, w); put_u32(&mut b, 12, h); put_u32(&mut b, 16, fcc); put_u32(&mut b, 20, 1 /* V4L2_FIELD_NONE */);
    b
}
/// What the driver set: (fourcc, width, height, bytes per line, image size).
pub fn format_result(b: &[u8; 208]) -> (u32, u32, u32, u32, u32) { (u32_at(b, 16), u32_at(b, 8), u32_at(b, 12), u32_at(b, 24), u32_at(b, 28)) }

/// `struct v4l2_streamparm` (204 bytes): ask for `fps` frames per second (timeperframe = 1 / fps).
pub fn streamparm(fps: u32) -> [u8; 204] {
    let mut b = [0u8; 204];
    put_u32(&mut b, 0, BUF_TYPE_VIDEO_CAPTURE); put_u32(&mut b, 12, 1); put_u32(&mut b, 16, fps.max(1));
    b
}
/// `struct v4l2_requestbuffers` (20 bytes).
pub fn reqbufs(count: u32) -> [u8; 20] { let mut b = [0u8; 20]; put_u32(&mut b, 0, count); put_u32(&mut b, 4, BUF_TYPE_VIDEO_CAPTURE); put_u32(&mut b, 8, MEMORY_MMAP); b }
/// `struct v4l2_buffer` (88 bytes on 64-bit): index 0, type 4, bytesused 8, memory 60, m.offset 64, length 72.
pub fn buffer(index: u32) -> [u8; 88] { let mut b = [0u8; 88]; put_u32(&mut b, 0, index); put_u32(&mut b, 4, BUF_TYPE_VIDEO_CAPTURE); put_u32(&mut b, 60, MEMORY_MMAP); b }
pub fn buffer_fields(b: &[u8; 88]) -> (u32, u32, u32, u32) { (u32_at(b, 0), u32_at(b, 8), u32_at(b, 64), u32_at(b, 72)) } // index, bytesused, offset, length

/// Builds the mode list from (fourcc, w, h, fps) answers, keeping the formats the converters know.
pub fn modes_of(found: &[(u32, u32, u32, f32)]) -> Vec<Mode> {
    let mut v: Vec<Mode> = found.iter().filter_map(|&(f, w, h, fps)| pix_of(f).map(|pix| Mode { pix, w, h, fps })).collect();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_videodev2() {
        // values as printed by a C program including <linux/videodev2.h> on x86_64
        assert_eq!(QUERYCAP, 0x8068_5600);
        assert_eq!(ENUM_FMT, 0xc040_5602);
        assert_eq!(S_FMT, 0xc0d0_5605);
        assert_eq!(REQBUFS, 0xc014_5608);
        assert_eq!(QUERYBUF, 0xc058_5609);
        assert_eq!(QBUF, 0xc058_560f);
        assert_eq!(DQBUF, 0xc058_5611);
        assert_eq!(STREAMON, 0x4004_5612);
        assert_eq!(S_PARM, 0xc0cc_5616);
        assert_eq!(ENUM_FRAMESIZES, 0xc02c_564a);
        assert_eq!(ENUM_FRAMEINTERVALS, 0xc034_564b);
    }

    #[test]
    fn capability_parsing() {
        let mut b = [0u8; 104];
        b[..9].copy_from_slice(b"uvcvideo\0");
        b[16..16 + 18].copy_from_slice(b"Example Camera 01\0");
        b[48..48 + 21].copy_from_slice(b"usb-0000:00:14.0-8\0\0\0");
        put_u32(&mut b, 84, CAP_DEVICE_CAPS | CAP_VIDEO_CAPTURE | CAP_STREAMING | 0x0080_0000);
        put_u32(&mut b, 88, CAP_VIDEO_CAPTURE | CAP_STREAMING);
        let c = parse_cap(&b);
        assert_eq!((c.driver.as_str(), c.card.as_str(), c.bus.as_str()), ("uvcvideo", "Example Camera 01", "usb-0000:00:14.0-8"));
        assert!(c.is_camera());
        // the same camera's metadata node: device caps without VIDEO_CAPTURE
        put_u32(&mut b, 88, 0x0080_0000 | CAP_STREAMING);
        assert!(!parse_cap(&b).is_camera());
    }

    #[test]
    fn format_enumeration_parsing() {
        assert_eq!(pix_of(fourcc(b"YUYV")), Some(Pix::Yuyv));
        assert_eq!(pix_of(fourcc(b"MJPG")), Some(Pix::Mjpg));
        assert_eq!(pix_of(fourcc(b"H264")), None);
        for p in [Pix::Yuyv, Pix::Nv12, Pix::Mjpg, Pix::Rgb24, Pix::Bgra] { assert_eq!(pix_of(fourcc_of(p)), Some(p)); }
        let mut d = fmtdesc(2);
        assert_eq!((u32_at(&d, 0), u32_at(&d, 4)), (2, 1));
        put_u32(&mut d, 44, fourcc(b"MJPG"));
        assert_eq!(fmtdesc_fourcc(&d), fourcc(b"MJPG"));
        let mut s = frmsize(0, fourcc(b"YUYV"));
        put_u32(&mut s, 8, FRMSIZE_TYPE_DISCRETE); put_u32(&mut s, 12, 1280); put_u32(&mut s, 16, 720);
        assert_eq!(frmsize_sizes(&s), vec![(1280, 720)]);
        put_u32(&mut s, 8, 3); put_u32(&mut s, 12, 320); put_u32(&mut s, 16, 1280); put_u32(&mut s, 24, 240); put_u32(&mut s, 28, 720);
        assert_eq!(frmsize_sizes(&s), vec![(1280, 720), (960, 540), (640, 480), (640, 360)]);
        let mut i = frmival(0, fourcc(b"YUYV"), 1280, 720);
        put_u32(&mut i, 16, FRMIVAL_TYPE_DISCRETE); put_u32(&mut i, 20, 1); put_u32(&mut i, 24, 30);
        assert_eq!(frmival_fps(&i), Some(30.0));
        put_u32(&mut i, 20, 1001); put_u32(&mut i, 24, 30_000);
        assert!((frmival_fps(&i).unwrap() - 29.97).abs() < 0.01);
        let m = modes_of(&[(fourcc(b"YUYV"), 640, 480, 30.0), (fourcc(b"YUYV"), 640, 480, 30.0), (fourcc(b"H264"), 1920, 1080, 30.0)]);
        assert_eq!(m, vec![Mode { pix: Pix::Yuyv, w: 640, h: 480, fps: 30.0 }]);
    }

    #[test]
    fn format_and_buffer_layouts() {
        let f = format(fourcc(b"MJPG"), 1280, 720);
        assert_eq!((u32_at(&f, 0), u32_at(&f, 8), u32_at(&f, 12)), (1, 1280, 720));
        let mut r = f;
        put_u32(&mut r, 24, 2560); put_u32(&mut r, 28, 1_843_200);
        assert_eq!(format_result(&r), (fourcc(b"MJPG"), 1280, 720, 2560, 1_843_200));
        let p = streamparm(30);
        assert_eq!((u32_at(&p, 12), u32_at(&p, 16)), (1, 30));
        let rb = reqbufs(4);
        assert_eq!((u32_at(&rb, 0), u32_at(&rb, 4), u32_at(&rb, 8)), (4, 1, 1));
        let mut b = buffer(3);
        put_u32(&mut b, 8, 4096); put_u32(&mut b, 64, 0x10_000); put_u32(&mut b, 72, 614_400);
        assert_eq!(buffer_fields(&b), (3, 4096, 0x10_000, 614_400));
        assert_eq!(u32_at(&b, 60), MEMORY_MMAP);
    }
}
