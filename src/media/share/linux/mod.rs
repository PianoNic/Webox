//! Linux share pipeline, all in GStreamer:
//! `pipewiresrc` (portal) | `ximagesrc` (X11 without the portal) | `appsrc` (test picture)
//! -> videoconvert -> videoscale -> videorate -> I420 at the stream size and rate -> H.264 encoder -> [h264parse] -> appsink.
//! The encoder is the first one installed of x264enc (gst-plugins-ugly), openh264enc (gst-plugins-bad), vah264enc /
//! vaapih264enc (VA-API); `WEBOX_SHARE_ENCODER=<element>` picks one. Key frames on request go upstream as a force-key-unit
//! event; bitrate and frame rate changes are applied to the running pipeline. A resized window keeps the stream size
//! (scaled with borders).
mod portal;
mod x11;

use super::{convert, h264, AccessUnit, Kind, Opts, Out, ShareSource, Status};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use std::time::Instant;

/// Encoders in order of preference: (element, bitrate unit divisor from bit/s, key frame interval property).
const ENCODERS: [(&str, u32, &str); 4] = [("x264enc", 1000, "key-int-max"), ("openh264enc", 1, "gop-size"), ("vah264enc", 1000, "key-int-max"), ("vaapih264enc", 1000, "keyframe-period")];

fn use_portal() -> bool {
    let wayland = match crate::platform::linux::session() {
        crate::platform::linux::Session::Wayland => true,
        crate::platform::linux::Session::X11 => false,
        crate::platform::linux::Session::Unknown => std::env::var_os("WAYLAND_DISPLAY").is_some(),
    };
    (wayland || std::env::var_os("WEBOX_SHARE_X11").is_none()) && portal::available()
}

pub(super) fn sources() -> Vec<ShareSource> {
    if use_portal() {
        let p = |handle, name: String| ShareSource { kind: Kind::Portal, handle, index: 0, name, app: String::new(), size: (0, 0), primary: handle == 1, minimized: false };
        return vec![p(1, crate::trq!("Screen").into()), p(2, crate::tr!("ScreenAndAppTab", "Application").into())];
    }
    if x11::present() { x11::sources() } else { vec![] }
}

pub(super) fn thumbnail(s: &ShareSource, max: (u32, u32)) -> Option<(u32, u32, Vec<u8>)> {
    matches!(s.kind, Kind::Screen | Kind::Window).then(|| x11::thumbnail(s, max)).flatten()
}

fn make(name: &str) -> Result<gst::Element, String> {
    gst::ElementFactory::make(name).build().map_err(|_| format!("GStreamer element {name} is missing (see docs/LINUX.md)"))
}

/// Sets a property where the element has it (versions differ), from its text form.
fn prop(e: &gst::Element, name: &str, value: &str) { if e.find_property(name).is_some() { e.set_property_from_str(name, value); } }

fn raw_caps(size: Option<(u32, u32)>, fps: u32) -> gst::Caps {
    let mut b = gst::Caps::builder("video/x-raw").field("format", "I420").field("framerate", gst::Fraction::new(fps as i32, 1)).field("pixel-aspect-ratio", gst::Fraction::new(1, 1));
    if let Some((w, h)) = size { b = b.field("width", w as i32).field("height", h as i32); }
    b.build()
}

pub(super) fn run(src: &ShareSource, opts: Opts, out: &Out) -> Result<(), String> {
    gst::init().map_err(|e| format!("GStreamer: {e}"))?;
    let ctl = &out.ctl;
    // the source element, and the source size where known
    let mut cast = None;
    let (source, size) = match src.kind {
        Kind::Test => {
            let s = make("appsrc")?;
            let caps = gst::Caps::builder("video/x-raw").field("format", "RGBA").field("width", src.size.0 as i32).field("height", src.size.1 as i32)
                .field("framerate", gst::Fraction::new(opts.fps as i32, 1)).build();
            s.set_property("caps", &caps);
            prop(&s, "format", "time");
            prop(&s, "is-live", "true");
            (s, Some(src.size))
        }
        Kind::Screen | Kind::Window => {
            let s = make("ximagesrc")?;
            prop(&s, "use-damage", "false");
            prop(&s, "show-pointer", if opts.cursor { "true" } else { "false" });
            if src.kind == Kind::Window { prop(&s, "xid", &src.handle.to_string()); }
            (s, Some(src.size))
        }
        Kind::Portal => {
            let Some(c) = portal::open(src.handle == 2, opts.cursor, &|| ctl.stopped())? else { return Ok(()) };
            let s = make("pipewiresrc")?;
            prop(&s, "fd", &std::os::fd::AsRawFd::as_raw_fd(&c.fd).to_string());
            prop(&s, "path", &c.node.to_string());
            prop(&s, "do-timestamp", "true");
            prop(&s, "keepalive-time", "1000"); // a still screen still gives a picture a second
            prop(&s, "always-copy", "true");
            let size = c.size.filter(|s| s.0 > 0 && s.1 > 0);
            cast = Some(c);
            (s, size)
        }
    };
    let size = size.filter(|s| s.0 > 0 && s.1 > 0).map(|(w, h)| convert::fit(w, h, opts.max_size));
    let r = encode(src, source, size, opts, out);
    drop(cast);
    r
}

fn encode(src: &ShareSource, source: gst::Element, size: Option<(u32, u32)>, opts: Opts, out: &Out) -> Result<(), String> {
    let ctl = &out.ctl;
    let wanted = std::env::var("WEBOX_SHARE_ENCODER").ok();
    let (enc_name, unit, gop_prop) = ENCODERS.iter().copied()
        .find(|(n, ..)| wanted.as_deref().map_or(true, |w| w == *n) && gst::ElementFactory::find(n).is_some())
        .ok_or("no H.264 encoder for GStreamer is installed (x264enc, openh264enc or VA-API; see docs/LINUX.md)")?;
    let enc = make(enc_name)?;
    let fps = ctl.fps();
    match enc_name {
        "x264enc" => { prop(&enc, "tune", "zerolatency"); prop(&enc, "speed-preset", "veryfast"); prop(&enc, "bframes", "0"); prop(&enc, "byte-stream", "true"); }
        "openh264enc" => { prop(&enc, "rate-control", "bitrate"); prop(&enc, "complexity", "low"); }
        _ => { prop(&enc, "rate-control", "cbr"); prop(&enc, "b-frames", "0"); prop(&enc, "max-bframes", "0"); }
    }
    prop(&enc, "bitrate", &(ctl.bitrate() / unit).to_string());
    if opts.keyframe_secs > 0 { prop(&enc, gop_prop, &(opts.keyframe_secs * fps).to_string()); }
    let raw = make("capsfilter")?;
    raw.set_property("caps", raw_caps(size, fps));
    let h264caps = make("capsfilter")?;
    h264caps.set_property("caps", gst::Caps::builder("video/x-h264").field("stream-format", "byte-stream").field("alignment", "au").field("profile", "constrained-baseline").build());
    let queue = make("queue")?;
    prop(&queue, "leaky", "downstream");
    prop(&queue, "max-size-buffers", "2");
    let sink = gst_app::AppSink::builder().sync(false).max_buffers(8).drop(false).build();
    let mut chain = vec![source.clone(), make("videoconvert")?, make("videoscale")?, make("videorate")?, raw.clone(), queue, enc.clone(), h264caps];
    // SPS + PPS before every key frame, one access unit per buffer
    if let Ok(p) = make("h264parse") { prop(&p, "config-interval", "-1"); chain.push(p); }
    chain.push(sink.clone().upcast());
    let pipeline = gst::Pipeline::new();
    pipeline.add_many(&chain).map_err(|e| e.to_string())?;
    gst::Element::link_many(&chain).map_err(|e| format!("GStreamer: {e}"))?;
    pipeline.set_state(gst::State::Playing).map_err(|_| "GStreamer: the share pipeline does not start".to_string())?;
    let r = pull(src, &pipeline, &source, &sink, &enc, &raw, enc_name, unit, size, out);
    let _ = pipeline.set_state(gst::State::Null);
    r
}

#[allow(clippy::too_many_arguments)]
fn pull(src: &ShareSource, pipeline: &gst::Pipeline, source: &gst::Element, sink: &gst_app::AppSink, enc: &gst::Element, raw: &gst::Element,
        enc_name: &str, unit: u32, size: Option<(u32, u32)>, out: &Out) -> Result<(), String> {
    let ctl = &out.ctl;
    let bus = pipeline.bus().ok_or("bus")?;
    let (mut bitrate, mut fps) = (ctl.bitrate(), ctl.fps());
    let mut params = h264::ParamSets::default();
    let (mut first_pts, mut running) = (None, false);
    let (t0, mut pushed) = (Instant::now(), 0u32);
    let appsrc = source.clone().dynamic_cast::<gst_app::AppSrc>().ok();
    let mut picture = vec![0u8; (src.size.0 * src.size.1 * 4) as usize];
    while !ctl.stopped() {
        while let Some(m) = bus.pop() {
            match m.view() {
                gst::MessageView::Error(e) => return Err(format!("GStreamer: {}", e.error())),
                gst::MessageView::Eos(_) => return Ok(()),
                _ => {}
            }
        }
        if let Some(a) = &appsrc {
            // the test picture, paced like a live source
            while (pushed as f64) < t0.elapsed().as_secs_f64() * fps as f64 + 1.0 {
                convert::test_picture(pushed, src.size.0, src.size.1, &mut picture);
                let mut b = gst::Buffer::from_slice(picture.clone());
                if let Some(b) = b.get_mut() { b.set_pts(gst::ClockTime::from_nseconds(pushed as u64 * 1_000_000_000 / fps as u64)); }
                let _ = a.push_buffer(b);
                pushed += 1;
            }
        }
        if ctl.take_keyframe() {
            sink.send_event(gstreamer_video::UpstreamForceKeyUnitEvent::builder().all_headers(true).build());
        }
        if ctl.bitrate() != bitrate { bitrate = ctl.bitrate(); prop(enc, "bitrate", &(bitrate / unit).to_string()); }
        if ctl.fps() != fps { fps = ctl.fps(); raw.set_property("caps", raw_caps(size, fps)); }
        let Some(sample) = sink.try_pull_sample(gst::ClockTime::from_mseconds(50)) else { continue };
        let Some(buf) = sample.buffer() else { continue };
        let Ok(map) = buf.map_readable() else { continue };
        let (w, h) = sample.caps().and_then(|c| c.structure(0).map(|s| (s.get::<i32>("width").unwrap_or(0) as u32, s.get::<i32>("height").unwrap_or(0) as u32))).unwrap_or((0, 0));
        if !running { running = true; ctl.set_status(Status::Running { width: w, height: h, encoder: format!("GStreamer {enc_name}") }); }
        let pts = buf.pts().map_or(0, |t| t.nseconds());
        let base = *first_pts.get_or_insert(pts);
        let data = params.fix(map.as_slice());
        let keyframe = h264::is_keyframe(&data);
        if !out.send(AccessUnit { data, pts_us: pts.saturating_sub(base) / 1000, keyframe, width: w, height: h }) { return Ok(()); }
    }
    Ok(())
}
