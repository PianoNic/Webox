//! Picture plumbing on the CPU: fitting a source into the stream's size, BGRA / RGBA -> NV12 with box-filter scaling (BT.601
//! limited range, what decoders assume for an H.264 stream without colour information), and the test picture.

/// Fits (w, h) into `max` keeping the ratio, never enlarging; even numbers (4:2:0 needs them), at least 2 x 2.
pub fn fit(w: u32, h: u32, max: (u32, u32)) -> (u32, u32) {
    if w == 0 || h == 0 { return (max.0.max(2) & !1, max.1.max(2) & !1); }
    let k = (max.0 as f64 / w as f64).min(max.1 as f64 / h as f64).min(1.0);
    (((w as f64 * k).round() as u32 & !1).max(2), ((h as f64 * k).round() as u32 & !1).max(2))
}

/// A source picture: 4 bytes per pixel, rows `stride` bytes apart.
#[derive(Clone, Copy)]
pub struct Picture<'a> { pub px: &'a [u8], pub w: u32, pub h: u32, pub stride: usize, pub bgra: bool }

/// Scales `src` to `dw` x `dh` (each output pixel averages the source pixels it covers) and writes NV12 into `out`
/// (Y plane, then interleaved U V at half resolution). Rows are shared out to a few threads.
pub fn to_nv12(src: Picture, dw: u32, dh: u32, out: &mut Vec<u8>) {
    let (dw, dh) = (dw as usize & !1, dh as usize & !1);
    out.resize(dw * dh * 3 / 2, 0);
    if dw == 0 || dh == 0 || src.w == 0 || src.h == 0 { return; }
    // source column / row span of every output column / row
    let span = |n: usize, s: u32| -> Vec<(usize, usize)> {
        (0..n).map(|i| { let a = i * s as usize / n; let b = ((i + 1) * s as usize / n).max(a + 1).min(s as usize); (a, b) }).collect()
    };
    let (xs, ys) = (span(dw, src.w), span(dh, src.h));
    let (y_plane, uv_plane) = out.split_at_mut(dw * dh);
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 8);
    let pairs = dh / 2;
    let per = pairs.div_ceil(threads).max(1);
    std::thread::scope(|sc| {
        for (k, (yc, uvc)) in y_plane.chunks_mut(per * 2 * dw).zip(uv_plane.chunks_mut(per * dw)).enumerate() {
            let (xs, ys) = (&xs, &ys);
            sc.spawn(move || {
                let mut rgb = vec![[0u32; 3]; dw * 2];
                for p in 0..yc.len() / (2 * dw) {
                    let row0 = (k * per + p) * 2;
                    for r in 0..2 { sample_row(src, xs, ys[row0 + r], &mut rgb[r * dw..(r + 1) * dw]); }
                    for r in 0..2 {
                        for x in 0..dw {
                            let [cr, cg, cb] = rgb[r * dw + x];
                            yc[(p * 2 + r) * dw + x] = (((66 * cr + 129 * cg + 25 * cb + 128) >> 8) + 16) as u8;
                        }
                    }
                    for x in 0..dw / 2 {
                        let mut s = [0i32; 3];
                        for i in [x * 2, x * 2 + 1, dw + x * 2, dw + x * 2 + 1] { for c in 0..3 { s[c] += rgb[i][c] as i32; } }
                        let (cr, cg, cb) = (s[0] / 4, s[1] / 4, s[2] / 4);
                        uvc[p * dw + x * 2] = (((-38 * cr - 74 * cg + 112 * cb + 128) >> 8) + 128).clamp(0, 255) as u8;
                        uvc[p * dw + x * 2 + 1] = (((112 * cr - 94 * cg - 18 * cb + 128) >> 8) + 128).clamp(0, 255) as u8;
                    }
                }
            });
        }
    });
}

/// One output row: the average R G B of each output pixel's source box.
fn sample_row(src: Picture, xs: &[(usize, usize)], (y0, y1): (usize, usize), out: &mut [[u32; 3]]) {
    let (ri, bi) = if src.bgra { (2, 0) } else { (0, 2) };
    for (o, &(x0, x1)) in out.iter_mut().zip(xs) {
        let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
        // large boxes are sampled on a grid of at most 4 x 4 (a 4K screen into 720p still reads every third pixel)
        let (sx, sy) = (((x1 - x0) / 4).max(1), ((y1 - y0) / 4).max(1));
        let mut n = 0u32;
        let mut y = y0;
        while y < y1 {
            let row = &src.px[y * src.stride..];
            let mut x = x0;
            while x < x1 { let p = &row[x * 4..x * 4 + 4]; r += p[ri] as u32; g += p[1] as u32; b += p[bi] as u32; n += 1; x += sx; }
            y += sy;
        }
        *o = [r / n, g / n, b / n];
    }
}

/// The test picture `i` (RGBA, `w` x `h`): eight colour bars sliding one bar width per second at 30 fps, a white square
/// moving across, and a strip of blocks counting the pictures.
pub fn test_picture(i: u32, w: u32, h: u32, rgba: &mut [u8]) {
    const BARS: [[u8; 3]; 8] = [[235, 235, 235], [235, 235, 16], [16, 235, 235], [16, 235, 16], [235, 16, 235], [235, 16, 16], [16, 16, 235], [16, 16, 16]];
    let (w, h) = (w as usize, h as usize);
    let bar = (w / 8).max(1);
    let shift = (i as usize * bar / 30) % w.max(1);
    let sq = (h / 6).max(2);
    let (qx, qy) = ((i as usize * 7) % w.saturating_sub(sq).max(1), h / 2 - sq / 2);
    for y in 0..h {
        for x in 0..w {
            let mut c = BARS[((x + shift) / bar) % 8];
            if (qx..qx + sq).contains(&x) && (qy..qy + sq).contains(&y) { c = [255, 255, 255]; }
            if y >= h - h / 16 { let k = x * 16 / w; c = if (i >> (15 - k.min(15))) & 1 == 1 { [255, 255, 255] } else { [0, 0, 0] }; }
            rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
}

/// Scales an RGBA / BGRA picture down into `max` (box filter) and returns RGBA, for thumbnails.
pub fn thumb(src: Picture, max: (u32, u32)) -> (u32, u32, Vec<u8>) {
    let (dw, dh) = fit(src.w, src.h, max);
    let (dw, dh) = (dw as usize, dh as usize);
    let xs: Vec<(usize, usize)> = (0..dw).map(|i| { let a = i * src.w as usize / dw; (a, ((i + 1) * src.w as usize / dw).max(a + 1)) }).collect();
    let mut out = vec![0u8; dw * dh * 4];
    let mut row = vec![[0u32; 3]; dw];
    for y in 0..dh {
        let a = y * src.h as usize / dh;
        sample_row(src, &xs, (a, ((y + 1) * src.h as usize / dh).max(a + 1)), &mut row);
        for (x, c) in row.iter().enumerate() { out[(y * dw + x) * 4..(y * dw + x) * 4 + 4].copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, 255]); }
    }
    (dw as u32, dh as u32, out)
}
