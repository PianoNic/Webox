//! H.264 Annex B helpers for the share stream and its RTP packetizer: splitting an access unit into NAL units, key frame
//! detection, the SPS fields the SDP needs (`profile-level-id`), marking a Baseline stream as Constrained Baseline, and keeping
//! SPS + PPS in front of every key frame.

/// NAL unit types used here.
pub const IDR: u8 = 5;
pub const SEI: u8 = 6;
pub const SPS: u8 = 7;
pub const PPS: u8 = 8;
pub const AUD: u8 = 9;

pub fn nal_type(nal: &[u8]) -> u8 { nal.first().map_or(0, |b| b & 0x1f) }

/// The NAL units of an Annex B buffer (without their start codes; 3- and 4-byte start codes).
pub fn nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = vec![];
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 { starts.push(i + 3); i += 3; } else { i += 1; }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut e = if k + 1 < starts.len() { starts[k + 1] - 3 } else { data.len() };
        while e > s && data[e - 1] == 0 { e -= 1; } // trailing zeros / the 4th start-code byte
        if e > s { out.push(&data[s..e]); }
    }
    out
}

/// Joins NAL units back into Annex B with 4-byte start codes.
pub fn annex_b<'a>(nals: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
    let mut v = vec![];
    for n in nals { v.extend_from_slice(&[0, 0, 0, 1]); v.extend_from_slice(n); }
    v
}

pub fn is_keyframe(data: &[u8]) -> bool { nals(data).iter().any(|n| nal_type(n) == IDR) }

/// (profile_idc, constraint flags byte, level_idc) of an SPS NAL unit.
pub fn sps_profile(sps: &[u8]) -> Option<(u8, u8, u8)> {
    (nal_type(sps) == SPS && sps.len() >= 4).then(|| (sps[1], sps[2], sps[3]))
}

/// The SDP `profile-level-id` (six hex digits, e.g. "42e01f") of the first SPS in `data`.
pub fn profile_level_id(data: &[u8]) -> Option<String> {
    let sps = nals(data).into_iter().find(|n| nal_type(n) == SPS)?;
    let (p, c, l) = sps_profile(sps)?;
    Some(format!("{p:02x}{c:02x}{l:02x}"))
}

/// Picture size from an SPS (luma pixels after cropping); for checks and logs.
pub fn sps_size(sps: &[u8]) -> Option<(u32, u32)> {
    if nal_type(sps) != SPS { return None; }
    let rbsp: Vec<u8> = { let mut v = Vec::with_capacity(sps.len()); let mut z = 0; for &b in &sps[1..] { if z >= 2 && b == 3 { z = 0; continue; } z = if b == 0 { z + 1 } else { 0 }; v.push(b); } v };
    let mut r = Bits { d: &rbsp, p: 0 };
    let profile = r.u(8)?; r.u(16)?; // constraint flags + level
    r.ue()?; // seq_parameter_set_id
    let mut chroma = 1;
    if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&profile) {
        chroma = r.ue()?;
        if chroma == 3 { r.u(1)?; }
        r.ue()?; r.ue()?; r.u(1)?;
        if r.u(1)? == 1 { for i in 0..if chroma == 3 { 12 } else { 8 } { if r.u(1)? == 1 { skip_scaling(&mut r, if i < 6 { 16 } else { 64 })?; } } }
    }
    r.ue()?; // log2_max_frame_num
    match r.ue()? { 0 => { r.ue()?; } 1 => { r.u(1)?; r.se()?; r.se()?; for _ in 0..r.ue()? { r.se()?; } } _ => {} }
    r.ue()?; r.u(1)?; // max_num_ref_frames, gaps
    let (wm, hm) = (r.ue()? + 1, r.ue()? + 1);
    let frame_mbs_only = r.u(1)?;
    if frame_mbs_only == 0 { r.u(1)?; }
    r.u(1)?; // direct_8x8
    let (mut w, mut h) = (wm * 16, hm * 16 * (2 - frame_mbs_only));
    if r.u(1)? == 1 {
        let (l, rr, t, b) = (r.ue()?, r.ue()?, r.ue()?, r.ue()?);
        let (cx, cy) = match chroma { 0 => (1, 2 - frame_mbs_only), 1 => (2, 2 * (2 - frame_mbs_only)), 2 => (2, 2 - frame_mbs_only), _ => (1, 2 - frame_mbs_only) };
        w = w.saturating_sub((l + rr) * cx);
        h = h.saturating_sub((t + b) * cy);
    }
    Some((w, h))
}

fn skip_scaling(r: &mut Bits, n: usize) -> Option<()> {
    let (mut last, mut next) = (8i32, 8i32);
    for _ in 0..n { if next != 0 { next = (last + r.se()? + 256) % 256; } if next != 0 { last = next; } }
    Some(())
}

struct Bits<'a> { d: &'a [u8], p: usize }
impl Bits<'_> {
    fn u(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n { let b = *self.d.get(self.p / 8)?; v = (v << 1) | ((b >> (7 - self.p % 8)) & 1) as u32; self.p += 1; }
        Some(v)
    }
    fn ue(&mut self) -> Option<u32> { let mut z = 0; while self.u(1)? == 0 { z += 1; if z > 31 { return None; } } Some((1u32 << z) - 1 + self.u(z)?) }
    fn se(&mut self) -> Option<i32> { let k = self.ue()? as i64; Some(if k % 2 == 1 { ((k + 1) / 2) as i32 } else { -(k / 2) as i32 }) }
}

/// Sets constraint_set0 and constraint_set1 on a Baseline SPS: the encoders used here write Baseline without FMO, ASO or
/// redundant slices, which is Constrained Baseline (42e0xx), the profile WebRTC endpoints negotiate.
pub fn constrain_baseline(sps: &mut [u8]) {
    if nal_type(sps) == SPS && sps.len() >= 4 && sps[1] == 66 { sps[2] |= 0xc0; }
}

/// Keeps the last SPS / PPS and puts them in front of key frames that come without them. Also drops access unit
/// delimiters and marks the SPS as Constrained Baseline. Returns the cleaned access unit.
#[derive(Default)]
pub struct ParamSets { pub sps: Option<Vec<u8>>, pub pps: Option<Vec<u8>> }

impl ParamSets {
    pub fn fix(&mut self, data: &[u8]) -> Vec<u8> {
        let units = nals(data);
        let mut rest: Vec<&[u8]> = Vec::with_capacity(units.len());
        let mut params = false;
        for n in &units {
            match nal_type(n) {
                AUD => {}
                SPS => { let mut s = n.to_vec(); constrain_baseline(&mut s); self.sps = Some(s); params = true; }
                PPS => { self.pps = Some(n.to_vec()); params = true; }
                _ => rest.push(n),
            }
        }
        // parameter sets go first, SPS before PPS
        let key = rest.iter().any(|n| nal_type(n) == IDR);
        let mut out: Vec<&[u8]> = vec![];
        if key || params { out.extend(self.sps.as_deref()); out.extend(self.pps.as_deref()); }
        out.extend(rest);
        annex_b(out)
    }
}
