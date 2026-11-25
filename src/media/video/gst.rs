//! Linux: GStreamer. `playbin` picks the demuxer and decoders from the installed plugins (hardware ones such as VA-API too, when
//! the distribution ships them and ranks them first); its video sink is `videoconvert ! videoscale ! appsink` (RGBA, at most
//! `MAX_SIZE`, aspect kept), its audio sink `audioconvert ! appsink` (interleaved F32LE at the file's rate). Both app sinks run
//! unsynchronised with small queues: the pipeline decodes as far ahead as `sync.rs` lets it, and the player's own clock decides
//! when a picture is shown. Seeks are flushing and accurate.
use super::{Info, Packet, Source};
use super::sync::Frame;
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_audio as gst_audio;
use gstreamer_video as gst_video;
use std::path::Path;

struct Gst { pipeline: gst::Element, video: Option<gst_app::AppSink>, audio: Option<gst_app::AppSink>, v_eos: bool, a_eos: bool }

fn sink_bin(elements: &[&str], sink: &gst_app::AppSink) -> Result<gst::Bin, String> {
    let bin = gst::Bin::new();
    let mut chain: Vec<gst::Element> = vec![];
    for name in elements { chain.push(gst::ElementFactory::make(name).build().map_err(|_| format!("GStreamer element {name} is missing (gst-plugins-base)"))?); }
    chain.push(sink.clone().upcast());
    bin.add_many(&chain).map_err(|e| e.to_string())?;
    gst::Element::link_many(&chain).map_err(|e| e.to_string())?;
    let pad = chain[0].static_pad("sink").ok_or("no sink pad")?;
    bin.add_pad(&gst::GhostPad::with_target(&pad).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(bin)
}

fn bus_error(p: &gst::Element) -> Option<String> {
    let bus = p.bus()?;
    while let Some(m) = bus.pop() {
        if let gst::MessageView::Error(e) = m.view() {
            let mut s = e.error().to_string();
            if let Some(d) = e.debug() { if d.contains("missing") || d.contains("decoder") { s.push_str(" (a GStreamer plugin for this format is not installed)"); } }
            return Some(s);
        }
    }
    None
}

/// The file's bytes are a container `format::sniff` knows. playbin plays whatever its typefinders recognise, playlists (HLS,
/// DASH, SMIL, ...) that fetch other URLs included, so a received "video" of any other kind is refused before it gets there.
fn known_container(path: &Path) -> Result<(), String> {
    use std::io::Read;
    let mut head = vec![0u8; 4096];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).map_err(|e| e.to_string())?;
    head.truncate(n);
    super::format::sniff(&head).map(|_| ()).ok_or_else(|| "not a known video container".to_string())
}

pub(super) fn open(path: &Path, max: (u32, u32), with_audio: bool) -> Result<(Box<dyn Source>, Info), String> {
    known_container(path)?;
    crate::media::gst_init()?;
    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let uri = crate::app::files::file_uri(&abs);
    let pipeline = gst::ElementFactory::make("playbin").property("uri", &uri).build().map_err(|_| "GStreamer: playbin is missing (gstreamer1.0-plugins-base)".to_string())?;
    let vcaps = gst::Caps::builder("video/x-raw").field("format", "RGBA")
        .field("width", gst::IntRange::new(1, max.0 as i32)).field("height", gst::IntRange::new(1, max.1 as i32))
        .field("pixel-aspect-ratio", gst::Fraction::new(1, 1)).build();
    let vsink = gst_app::AppSink::builder().caps(&vcaps).max_buffers(3).drop(false).sync(false).build();
    pipeline.set_property("video-sink", sink_bin(&["videoconvert", "videoscale"], &vsink)?.to_value());
    let asink = if with_audio {
        let acaps = gst::Caps::builder("audio/x-raw").field("format", "F32LE").field("layout", "interleaved").build();
        let s = gst_app::AppSink::builder().caps(&acaps).max_buffers(8).drop(false).sync(false).build();
        pipeline.set_property("audio-sink", sink_bin(&["audioconvert"], &s)?.to_value());
        pipeline.set_property_from_str("flags", "video+audio");
        Some(s)
    } else {
        pipeline.set_property_from_str("flags", "video");
        None
    };
    pipeline.set_state(gst::State::Paused).map_err(|_| bus_error(&pipeline).unwrap_or_else(|| "GStreamer cannot open the file".into()))?;
    // wait for the preroll (streams known, first picture decoded)
    let bus = pipeline.bus().ok_or("bus")?;
    match bus.timed_pop_filtered(gst::ClockTime::from_seconds(15), &[gst::MessageType::AsyncDone, gst::MessageType::Error]) {
        Some(m) => if let gst::MessageView::Error(e) = m.view() {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(format!("GStreamer: {}", e.error()));
        },
        None => { let _ = pipeline.set_state(gst::State::Null); return Err("GStreamer: the file did not open in time".into()); }
    }
    let n_video: i32 = pipeline.property("n-video");
    let n_audio: i32 = pipeline.property("n-audio");
    let duration = pipeline.query_duration::<gst::ClockTime>().map_or(0.0, |d| d.nseconds() as f64 / 1e9);
    let (mut w, mut h) = (0, 0);
    if let Some(c) = vsink.static_pad("sink").and_then(|p| p.current_caps()) {
        if let Ok(vi) = gst_video::VideoInfo::from_caps(&c) { (w, h) = (vi.width(), vi.height()); }
    }
    pipeline.set_state(gst::State::Playing).map_err(|e| e.to_string())?;
    let has_audio = with_audio && n_audio > 0;
    let info = Info { duration, width: w, height: h, has_video: n_video > 0, has_audio, hardware: false, ready: true, error: None };
    if n_video <= 0 && !has_audio { let _ = pipeline.set_state(gst::State::Null); return Err("GStreamer: no video in the file".into()); }
    Ok((Box::new(Gst { pipeline, video: (n_video > 0).then_some(vsink), audio: asink.filter(|_| has_audio), v_eos: n_video <= 0, a_eos: !has_audio }), info))
}

fn picture(s: &gst::Sample) -> Option<Frame> {
    let caps = s.caps()?;
    let vi = gst_video::VideoInfo::from_caps(caps).ok()?;
    let buf = s.buffer()?;
    let pts = buf.pts().map_or(0.0, |t| t.nseconds() as f64 / 1e9);
    let map = buf.map_readable().ok()?;
    let (w, h, stride) = (vi.width() as usize, vi.height() as usize, vi.stride()[0] as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h { rgba.extend_from_slice(map.get(y * stride..y * stride + w * 4)?); }
    Some(Frame { pts, w, h, rgba })
}

fn sound(s: &gst::Sample) -> Option<Packet> {
    let ai = gst_audio::AudioInfo::from_caps(s.caps()?).ok()?;
    let buf = s.buffer()?;
    let pts = buf.pts().map_or(0.0, |t| t.nseconds() as f64 / 1e9);
    let map = buf.map_readable().ok()?;
    let data = map.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    Some(Packet::Audio { pts, rate: ai.rate(), ch: ai.channels() as u16, data })
}

impl Source for Gst {
    fn read(&mut self) -> Result<Option<Packet>, String> {
        let tick = gst::ClockTime::from_mseconds(5);
        loop {
            if let Some(e) = bus_error(&self.pipeline) { return Err(format!("GStreamer: {e}")); }
            if self.v_eos && self.a_eos { return Ok(None); }
            if let (false, Some(v)) = (self.v_eos, &self.video) {
                if let Some(s) = v.try_pull_sample(tick) { if let Some(f) = picture(&s) { return Ok(Some(Packet::Video(f))); } }
                else if v.is_eos() { self.v_eos = true; }
            }
            if let (false, Some(a)) = (self.a_eos, &self.audio) {
                if let Some(s) = a.try_pull_sample(tick) { if let Some(p) = sound(&s) { return Ok(Some(p)); } }
                else if a.is_eos() { self.a_eos = true; }
            }
        }
    }

    fn seek(&mut self, secs: f64) -> Result<(), String> {
        let t = gst::ClockTime::from_nseconds((secs.max(0.0) * 1e9) as u64);
        self.pipeline.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, t).map_err(|e| format!("GStreamer: seek failed: {e}"))?;
        self.v_eos = self.video.is_none();
        self.a_eos = self.audio.is_none();
        Ok(())
    }
}

impl Drop for Gst {
    fn drop(&mut self) { let _ = self.pipeline.set_state(gst::State::Null); }
}
