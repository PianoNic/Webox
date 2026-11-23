//! A minimal MPEG transport stream writer (tests only): wraps the share stream's access units so the playback decoder
//! (`media::video`, which opens container files) can decode them. One program, one H.264 stream on PID 0x100, PAT + PMT in
//! front of every key frame, PCR on the video PID.
const PMT_PID: u16 = 0x1000;
const PID: u16 = 0x100;

#[derive(Default)]
pub struct Ts { pub out: Vec<u8>, cc: [u8; 3] }

fn crc32(d: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in d { c ^= (b as u32) << 24; for _ in 0..8 { c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04c1_1db7 } else { c << 1 }; } }
    c
}

impl Ts {
    fn section(&mut self, pid: u16, k: usize, body: &[u8]) {
        let mut p = vec![0x47, 0x40 | (pid >> 8) as u8, pid as u8, 0x10 | (self.cc[k] & 0xf), 0];
        self.cc[k] = self.cc[k].wrapping_add(1);
        p.extend_from_slice(body);
        p.extend_from_slice(&crc32(body).to_be_bytes());
        p.resize(188, 0xff);
        self.out.extend_from_slice(&p);
    }

    fn tables(&mut self) {
        // PAT: program 1 -> PMT
        self.section(0, 0, &[0x00, 0xb0, 13, 0x00, 0x01, 0xc1, 0, 0, 0x00, 0x01, 0xe0 | (PMT_PID >> 8) as u8, PMT_PID as u8]);
        // PMT: PCR on the video PID, stream type 0x1b (H.264)
        self.section(PMT_PID, 1, &[0x02, 0xb0, 18, 0x00, 0x01, 0xc1, 0, 0, 0xe0 | (PID >> 8) as u8, PID as u8, 0xf0, 0, 0x1b, 0xe0 | (PID >> 8) as u8, PID as u8, 0xf0, 0]);
    }

    /// Adds one access unit (Annex B) at `pts_us`.
    pub fn push(&mut self, au: &[u8], pts_us: u64, key: bool) {
        if key || self.out.is_empty() { self.tables(); }
        let pts = pts_us * 9 / 100 + 90_000; // 90 kHz, starting at 1 s
        let mut pes = vec![0, 0, 1, 0xe0, 0, 0, 0x80, 0x80, 5,
            0x21 | ((pts >> 29) & 0x0e) as u8, (pts >> 22) as u8, 0x01 | ((pts >> 14) & 0xfe) as u8, (pts >> 7) as u8, 0x01 | ((pts << 1) & 0xfe) as u8];
        pes.extend_from_slice(&[0, 0, 0, 1, 9, 0xf0]); // access unit delimiter
        pes.extend_from_slice(au);
        let mut first = true;
        let mut rest = &pes[..];
        while !rest.is_empty() {
            let mut p = vec![0x47, (if first { 0x40 } else { 0 }) | (PID >> 8) as u8, PID as u8];
            // adaptation field: PCR (+ random access on key frames) in the first packet, stuffing in the last
            let mut af: Option<Vec<u8>> = None;
            if first {
                let pcr = pts.saturating_sub(9000); // 100 ms before the picture is due
                af = Some(vec![if key { 0x50 } else { 0x10 }, (pcr >> 25) as u8, (pcr >> 17) as u8, (pcr >> 9) as u8, (pcr >> 1) as u8, ((pcr & 1) << 7) as u8 | 0x7e, 0]);
            }
            let room = 184 - af.as_ref().map_or(0, |a| a.len() + 1);
            let n = rest.len().min(room);
            if n < room {
                let s = room - n;
                match &mut af {
                    Some(a) => a.extend(std::iter::repeat_n(0xff, s)),
                    None if s == 1 => af = Some(vec![]),
                    None => { let mut a = vec![0]; a.extend(std::iter::repeat_n(0xff, s - 2)); af = Some(a); }
                }
            }
            let cc = self.cc[2] & 0xf;
            self.cc[2] = self.cc[2].wrapping_add(1);
            p.push(if af.is_none() { 0x10 } else { 0x30 } | cc);
            if let Some(a) = &af { p.push(a.len() as u8); p.extend_from_slice(a); }
            p.extend_from_slice(&rest[..n]);
            debug_assert_eq!(p.len(), 188);
            self.out.extend_from_slice(&p);
            rest = &rest[n..];
            first = false;
        }
    }
}
