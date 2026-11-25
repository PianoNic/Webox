//! Reads one file out of a zip archive: stored or deflated entries, no zip64 (what PowerShell's Compress-Archive writes for the
//! release zip). Sizes come from the central directory; the CRC-32 is checked.
use std::io::Read;

fn u16_at(b: &[u8], i: usize) -> Option<usize> { b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize) }
fn u32_at(b: &[u8], i: usize) -> Option<u32> { b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])) }

/// The bytes of the first entry whose file name (last path part) equals `name`, ignoring case. `cap` limits the unpacked size.
pub fn extract(zip: &[u8], name: &str, cap: u64) -> Result<Vec<u8>, String> {
    let bad = || crate::trq!("Archive damaged").into_owned();
    // End of central directory: the last "PK\x05\x06" within the final 64 KiB + 22 bytes.
    let from = zip.len().saturating_sub(65_557);
    let eocd = (from..zip.len().saturating_sub(21)).rev().find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or_else(bad)?;
    let count = u16_at(zip, eocd + 10).ok_or_else(bad)?;
    let mut p = u32_at(zip, eocd + 16).ok_or_else(bad)? as usize;
    for _ in 0..count {
        if u32_at(zip, p) != Some(0x0201_4b50) { return Err(bad()); }
        let method = u16_at(zip, p + 10).ok_or_else(bad)?;
        let crc = u32_at(zip, p + 16).ok_or_else(bad)?;
        let csize = u32_at(zip, p + 20).ok_or_else(bad)? as usize;
        let usize_ = u32_at(zip, p + 24).ok_or_else(bad)? as u64;
        let (n, e, c) = (u16_at(zip, p + 28).ok_or_else(bad)?, u16_at(zip, p + 30).ok_or_else(bad)?, u16_at(zip, p + 32).ok_or_else(bad)?);
        let local = u32_at(zip, p + 42).ok_or_else(bad)? as usize;
        let entry = String::from_utf8_lossy(zip.get(p + 46..p + 46 + n).ok_or_else(bad)?).replace('\\', "/");
        p += 46 + n + e + c;
        if !entry.rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case(name)) { continue; }
        if usize_ > cap { return Err(crate::trq!("{{0}} is larger than allowed", name)); }
        if u32_at(zip, local) != Some(0x0403_4b50) { return Err(bad()); }
        let data_at = local + 30 + u16_at(zip, local + 26).ok_or_else(bad)? + u16_at(zip, local + 28).ok_or_else(bad)?;
        let raw = zip.get(data_at..data_at + csize).ok_or_else(bad)?;
        let out = match method {
            0 => raw.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(usize_ as usize);
                flate2::read::DeflateDecoder::new(raw).take(cap + 1).read_to_end(&mut out).map_err(|_| bad())?;
                out
            }
            m => return Err(crate::trq!("Compression method {{0}} is not supported", m)),
        };
        let mut h = flate2::Crc::new();
        h.update(&out);
        if out.len() as u64 != usize_ || h.sum() != crc { return Err(bad()); }
        return Ok(out);
    }
    Err(format!("{name} fehlt im Archiv"))
}
